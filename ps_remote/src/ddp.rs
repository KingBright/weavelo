use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::net::UdpSocket;

/// PlayStation 5 Device Discovery Protocol (DDP) UDP port.
pub const DDP_PORT_PS5: u16 = 9302;

/// PlayStation 5 DDP protocol version string (`"00030010"`). PS4 (`"00020020"`) is rejected.
pub const DDP_VERSION_PS5: &str = "00030010";

#[derive(Debug, Error)]
pub enum DdpError {
    #[error("network I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid PS5 regist key for DDP user credential")]
    InvalidRegistKey,
    #[error("timed out waiting for PS5 {0} to become ready")]
    WakeTimeout(IpAddr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsolePowerState {
    Ready,
    Standby,
    Unknown(u16),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredConsole {
    pub ip: IpAddr,
    pub port: u16,
    pub host_id: String,
    pub host_name: String,
    pub host_type: String,
    pub system_version: String,
    pub protocol_version: String,
    pub state: ConsolePowerState,
    pub host_request_port: u16,
}

impl DiscoveredConsole {
    /// Returns true strictly for PlayStation 5 consoles (`00030010` / `PS5`).
    pub fn is_ps5(&self) -> bool {
        if self.protocol_version != DDP_VERSION_PS5 {
            return false;
        }
        if !self.host_type.is_empty() && !self.host_type.eq_ignore_ascii_case("PS5") {
            return false;
        }
        true
    }
}

/// Builds the PS5 DDP `SRCH` discovery packet (null-terminated to match Sony DDP framing).
pub fn build_srch_packet() -> Vec<u8> {
    format!(
        "SRCH * HTTP/1.1\ndevice-discovery-protocol-version:{}\n\0",
        DDP_VERSION_PS5
    )
    .into_bytes()
}

/// Computes the numeric DDP `user-credential` from the ASCII-hex prefix stored in `rp_regist_key`.
pub fn user_credential_from_regist_key(rp_regist_key: &[u8; 16]) -> Result<u64, DdpError> {
    let end = rp_regist_key
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(rp_regist_key.len());
    let ascii = std::str::from_utf8(&rp_regist_key[..end])
        .map_err(|_| DdpError::InvalidRegistKey)?
        .trim();
    if ascii.is_empty() {
        return Err(DdpError::InvalidRegistKey);
    }
    u64::from_str_radix(ascii, 16).map_err(|_| DdpError::InvalidRegistKey)
}

/// Builds the PS5 DDP `WAKEUP` packet (`auth-type:R` as required by PS5).
pub fn build_wakeup_packet(user_credential: u64) -> Vec<u8> {
    format!(
        "WAKEUP * HTTP/1.1\nclient-type:vr\nauth-type:R\nmodel:w\napp-type:r\nuser-credential:{}\ndevice-discovery-protocol-version:{}\n\0",
        user_credential, DDP_VERSION_PS5
    )
    .into_bytes()
}

/// Parses a DDP `SRCH` response packet, returning `None` if malformed or not a PS5.
pub fn parse_srch_response(
    ip: IpAddr,
    port: u16,
    payload: &[u8],
) -> Option<DiscoveredConsole> {
    let trimmed = match payload.iter().position(|&b| b == 0) {
        Some(pos) => &payload[..pos],
        None => payload,
    };
    let text = std::str::from_utf8(trimmed).ok()?;
    let mut lines = text.lines();
    let status_line = lines.next()?.trim();
    if !status_line.starts_with("HTTP/1.1 ") {
        return None;
    }
    let code_str = status_line.split_whitespace().nth(1)?;
    let status_code: u16 = code_str.parse().ok()?;
    let state = match status_code {
        200 => ConsolePowerState::Ready,
        620 => ConsolePowerState::Standby,
        other => ConsolePowerState::Unknown(other),
    };

    let mut host_id = String::new();
    let mut host_name = String::new();
    let mut host_type = String::new();
    let mut system_version = String::new();
    let mut protocol_version = String::new();
    let mut host_request_port = 9295u16;

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        if key.eq_ignore_ascii_case("host-id") {
            host_id = val.to_string();
        } else if key.eq_ignore_ascii_case("host-name") {
            host_name = val.to_string();
        } else if key.eq_ignore_ascii_case("host-type") {
            host_type = val.to_string();
        } else if key.eq_ignore_ascii_case("system-version") {
            system_version = val.to_string();
        } else if key.eq_ignore_ascii_case("device-discovery-protocol-version") {
            protocol_version = val.to_string();
        } else if key.eq_ignore_ascii_case("host-request-port") {
            if let Ok(parsed) = val.parse::<u16>() {
                host_request_port = parsed;
            }
        }
    }

    if host_id.is_empty() {
        return None;
    }

    let console = DiscoveredConsole {
        ip,
        port,
        host_id,
        host_name,
        host_type,
        system_version,
        protocol_version,
        state,
        host_request_port,
    };
    if !console.is_ps5() {
        return None;
    }
    Some(console)
}

/// Discovers PS5 consoles on the LAN (via broadcast + optional explicit socket targets).
pub async fn discover_ps5(
    timeout: Duration,
    extra_targets: &[SocketAddr],
) -> Result<Vec<DiscoveredConsole>, DdpError> {
    let sock = UdpSocket::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))).await?;
    sock.set_broadcast(true)?;

    let srch = build_srch_packet();
    let mut destinations: Vec<SocketAddr> = vec![SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::BROADCAST,
        DDP_PORT_PS5,
    ))];
    for &target in extra_targets {
        if target.is_ipv4() && !destinations.contains(&target) {
            destinations.push(target);
        }
    }

    for &dest in &destinations {
        let _ = sock.send_to(&srch, dest).await;
    }

    let mut found: BTreeMap<(String, IpAddr), DiscoveredConsole> = BTreeMap::new();
    let deadline = Instant::now() + timeout;
    let resend_at = Instant::now() + Duration::from_millis(180);
    let mut resent = false;
    let mut buf = [0u8; 2048];

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        if !resent && now >= resend_at {
            resent = true;
            for &dest in &destinations {
                let _ = sock.send_to(&srch, dest).await;
            }
        }
        let step_wait = if !resent {
            (deadline - now).min(resend_at.saturating_duration_since(now))
        } else {
            deadline - now
        };
        match tokio::time::timeout(step_wait, sock.recv_from(&mut buf)).await {
            Ok(Ok((n, src))) => {
                if let Some(console) = parse_srch_response(src.ip(), src.port(), &buf[..n]) {
                    found.insert((console.host_id.clone(), console.ip), console);
                }
            }
            Ok(Err(e)) => return Err(DdpError::Io(e)),
            Err(_) => {
                if resent {
                    break;
                }
            }
        }
    }

    Ok(found.into_values().collect())
}

/// Probes a specific PS5 IP on port `9302` (`DDP_PORT_PS5`).
pub async fn probe_ps5(
    target: IpAddr,
    timeout: Duration,
) -> Result<Option<DiscoveredConsole>, DdpError> {
    let dest = SocketAddr::new(target, DDP_PORT_PS5);
    let bind_addr = if target.is_ipv4() {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
    } else {
        "[::]:0".parse().expect("valid IPv6 unspecified addr")
    };
    let sock = UdpSocket::bind(bind_addr).await?;
    let srch = build_srch_packet();
    sock.send_to(&srch, dest).await?;

    let mut buf = [0u8; 2048];
    let deadline = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        match tokio::time::timeout(deadline - now, sock.recv_from(&mut buf)).await {
            Ok(Ok((n, src))) => {
                if let Some(console) = parse_srch_response(src.ip(), src.port(), &buf[..n]) {
                    return Ok(Some(console));
                }
            }
            Ok(Err(e)) => return Err(DdpError::Io(e)),
            Err(_) => return Ok(None),
        }
    }
}

/// Sends a DDP `WAKEUP` packet (`auth-type:R`) to wake a Standby PS5.
pub async fn send_wakeup(target: IpAddr, rp_regist_key: &[u8; 16]) -> Result<(), DdpError> {
    send_wakeup_to_port(target, DDP_PORT_PS5, rp_regist_key).await
}

/// Sends a DDP `WAKEUP` packet to an explicit UDP port (useful in tests and NAT overrides).
pub async fn send_wakeup_to_port(
    target: IpAddr,
    port: u16,
    rp_regist_key: &[u8; 16],
) -> Result<(), DdpError> {
    let cred = user_credential_from_regist_key(rp_regist_key)?;
    let pkt = build_wakeup_packet(cred);
    let bind_addr = if target.is_ipv4() {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
    } else {
        "[::]:0".parse().expect("valid IPv6 unspecified addr")
    };
    let sock = UdpSocket::bind(bind_addr).await?;
    if target.is_ipv4() {
        sock.set_broadcast(true)?;
    }
    sock.send_to(&pkt, SocketAddr::new(target, port)).await?;
    Ok(())
}

/// Wakes a PS5 from Standby and polls DDP until it reports `Ready` or `timeout` elapses.
pub async fn wake_and_wait_ready(
    target: IpAddr,
    rp_regist_key: &[u8; 16],
    timeout: Duration,
) -> Result<DiscoveredConsole, DdpError> {
    let deadline = Instant::now() + timeout;
    send_wakeup(target, rp_regist_key).await?;

    let mut next_wakeup = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(console) = probe_ps5(target, Duration::from_millis(600)).await? {
            if console.state == ConsolePowerState::Ready {
                return Ok(console);
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(DdpError::WakeTimeout(target));
        }
        if now >= next_wakeup {
            let _ = send_wakeup(target, rp_regist_key).await;
            next_wakeup = now + Duration::from_secs(2);
        }
        tokio::time::sleep(Duration::from_millis(250).min(deadline - now)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ps5_ready_and_standby_and_rejects_ps4() {
        let ready_raw = b"HTTP/1.1 200 Ok\r\nhost-id:AABBCCDDEEFF\r\nhost-type:PS5\r\nhost-name:LivingRoom-PS5\r\nhost-request-port:9295\r\ndevice-discovery-protocol-version:00030010\r\nsystem-version:09600001\r\n\0";
        let console =
            parse_srch_response(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)), 9302, ready_raw)
                .expect("PS5 should parse");
        assert_eq!(console.state, ConsolePowerState::Ready);
        assert_eq!(console.host_id, "AABBCCDDEEFF");
        assert_eq!(console.host_name, "LivingRoom-PS5");
        assert_eq!(console.protocol_version, "00030010");
        assert_eq!(console.host_request_port, 9295);

        let standby_raw = b"HTTP/1.1 620 Server Standby\nhost-id:AABBCCDDEEFF\nhost-type:PS5\nhost-name:PS5\ndevice-discovery-protocol-version:00030010\n";
        let standby = parse_srch_response(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
            9302,
            standby_raw,
        )
        .expect("PS5 standby should parse");
        assert_eq!(standby.state, ConsolePowerState::Standby);

        let ps4_raw = b"HTTP/1.1 200 Ok\nhost-id:112233445566\nhost-type:PS4\nhost-name:PS4\ndevice-discovery-protocol-version:00020020\n";
        assert!(parse_srch_response(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 51)),
            987,
            ps4_raw
        )
        .is_none());
    }

    #[test]
    fn wakeup_packet_uses_auth_type_r_and_ps5_version() {
        let mut regist_key = [0u8; 16];
        regist_key[..8].copy_from_slice(b"1a2b3c4d");
        let cred = user_credential_from_regist_key(&regist_key).unwrap();
        assert_eq!(cred, 0x1a2b_3c4d);
        let pkt = build_wakeup_packet(cred);
        let s = std::str::from_utf8(&pkt[..pkt.len() - 1]).unwrap();
        assert!(s.starts_with("WAKEUP * HTTP/1.1\n"));
        assert!(s.contains("auth-type:R\n"));
        assert!(s.contains("device-discovery-protocol-version:00030010\n"));
        assert!(s.contains(&format!("user-credential:{}\n", 0x1a2b_3c4d_u64)));
        assert_eq!(pkt.last().copied(), Some(0));
    }

    #[tokio::test]
    async fn loopback_discover_and_wakeup() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();

        let srv_task = tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                let (n, peer) = server.recv_from(&mut buf).await.unwrap();
                let msg = std::str::from_utf8(&buf[..n - 1]).unwrap();
                if msg.contains("SRCH * HTTP/1.1") {
                    assert!(msg.contains("00030010"));
                    let resp = b"HTTP/1.1 620 Server Standby\nhost-id:DEAD1234BEEF\nhost-type:PS5\nhost-name:LabPS5\ndevice-discovery-protocol-version:00030010\n\0";
                    server.send_to(resp, peer).await.unwrap();
                } else {
                    assert!(msg.contains("WAKEUP * HTTP/1.1"));
                    assert!(msg.contains("auth-type:R"));
                    assert!(msg.contains("user-credential:305419896"));
                    break;
                }
            }
        });

        let found = discover_ps5(Duration::from_millis(120), &[addr])
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].host_id, "DEAD1234BEEF");
        assert_eq!(found[0].state, ConsolePowerState::Standby);

        let mut key = [0u8; 16];
        key[..8].copy_from_slice(b"12345678");
        send_wakeup_to_port(addr.ip(), addr.port(), &key)
            .await
            .unwrap();
        srv_task.await.unwrap();
    }
}
