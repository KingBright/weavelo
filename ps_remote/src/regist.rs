use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use rand::Rng;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::{Instant, sleep, timeout};

use crate::rpcrypt::{RPCRYPT_KEY_SIZE, Rpcrypt, RpcryptError, aeropause_ps5};
use crate::store::{PairedConsole, console_id_from_host_id};

pub const REGIST_PORT: u16 = 9295;
pub const PS5_RP_VERSION: &str = "1.0";
pub const PS5_CLIENT_TYPE: &str = "dabfa2ec873de5839bee8d3f4c0239c4282c07c25c6077a2931afcf0adc0d34f";

const INNER_HEADER_OFF: usize = 0x1e0;
const SEARCH_REQUEST_SLEEP: Duration = Duration::from_millis(100);
const REGIST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Error)]
pub enum RegistError {
    #[error("network I/O error during PS5 pairing: {0}")]
    Io(#[from] io::Error),
    #[error("PS5 pairing crypto error: {0}")]
    Crypto(#[from] RpcryptError),
    #[error("timed out waiting for PS5 pairing discovery response (SRC3/RES3) on port {0}")]
    SearchTimeout(u16),
    #[error("PS5 refused pairing discovery; keep Settings -> System -> Remote Play -> Link Device open on screen")]
    SearchRefused,
    #[error("timed out waiting for PS5 pairing TCP response")]
    ResponseTimeout,
    #[error("PS5 pairing rejected with HTTP {status} (reason={reason:?}: {reason_desc})")]
    HttpError {
        status: u16,
        reason: Option<u32>,
        reason_desc: &'static str,
    },
    #[error("invalid PS5 pairing response: {0}")]
    InvalidResponse(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistResult {
    pub nickname: String,
    pub ap_ssid: String,
    pub ap_bssid: String,
    pub ap_key: String,
    pub ap_name: String,
    pub rp_regist_key: [u8; 16],
    pub rp_key: [u8; 16],
    pub rp_key_type: u32,
    pub ps5_mac: [u8; 6],
    pub host_addr: IpAddr,
}

impl RegistResult {
    /// Converts a successful PS5 pairing response into a persistable `PairedConsole`.
    pub fn into_paired_console(self, host_id: String, account_id: [u8; 8]) -> PairedConsole {
        let console_id = console_id_from_host_id(&host_id);
        PairedConsole {
            console_id,
            host_id,
            nickname: if self.nickname.is_empty() {
                "PlayStation 5".to_string()
            } else {
                self.nickname
            },
            host_addr: Some(self.host_addr),
            account_id,
            rp_regist_key: self.rp_regist_key,
            rp_key: self.rp_key,
            rp_key_type: self.rp_key_type,
            ps5_mac: self.ps5_mac,
            paired_at_unix: PairedConsole::now_unix_secs(),
        }
    }
}

/// Formats the obfuscated + AES-128-CFB128 encrypted `/sie/ps5/rp/sess/rgst` request body.
pub fn format_regist_payload(
    ambassador: &[u8; RPCRYPT_KEY_SIZE],
    account_id: &[u8; 8],
    pin: u32,
    prefix_fill: u8,
) -> Result<(Vec<u8>, Rpcrypt), RegistError> {
    let mut payload = vec![prefix_fill; INNER_HEADER_OFF];
    let key_0_off = usize::from(payload[0x18d] & 0x1f);
    let key_1_off = usize::from(payload[0] >> 3);

    let crypt = Rpcrypt::init_regist_ps5(*ambassador, key_0_off, pin)?;
    let aeropause = aeropause_ps5(key_1_off, &crypt.ambassador)?;
    payload[0xc7..0xcf].copy_from_slice(&aeropause[8..16]);
    payload[0x191..0x199].copy_from_slice(&aeropause[0..8]);

    let account_b64 = BASE64_STANDARD.encode(account_id);
    let inner = format!(
        "Client-Type: {PS5_CLIENT_TYPE}\r\n\
         Np-AccountId: {account_b64}\r\n"
    );
    let encrypted = crypt.encrypt(0, inner.as_bytes());
    payload.extend_from_slice(&encrypted);
    Ok((payload, crypt))
}

/// Formats the HTTP/1.1 request header required by PS5 `/sie/ps5/rp/sess/rgst`.
pub fn format_regist_request_header(payload_len: usize, local_host: &str) -> Vec<u8> {
    format!(
        "POST /sie/ps5/rp/sess/rgst HTTP/1.1\r\n\
         HOST: {local_host}\r\n\
         User-Agent: remoteplay Windows\r\n\
         Connection: close\r\n\
         Content-Length: {payload_len}\r\n\
         RP-Version: {PS5_RP_VERSION}\r\n\r\n"
    )
    .into_bytes()
}

/// Parses decrypted key/value response headers returned by the PS5 pairing endpoint.
pub fn parse_regist_response_headers(
    decrypted: &[u8],
    host_addr: IpAddr,
) -> Result<RegistResult, RegistError> {
    let text = std::str::from_utf8(decrypted)
        .map_err(|_| RegistError::InvalidResponse("decrypted body is not UTF-8"))?;

    let mut out = RegistResult {
        nickname: String::new(),
        ap_ssid: String::new(),
        ap_bssid: String::new(),
        ap_key: String::new(),
        ap_name: String::new(),
        rp_regist_key: [0u8; 16],
        rp_key: [0u8; 16],
        rp_key_type: 0,
        ps5_mac: [0u8; 6],
        host_addr,
    };

    let mut regist_key_found = false;
    let mut key_found = false;
    let mut mac_found = false;

    for raw_line in text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        match key {
            "PS5-Nickname" => out.nickname = val.to_string(),
            "AP-Ssid" => out.ap_ssid = val.to_string(),
            "AP-Bssid" => out.ap_bssid = val.to_string(),
            "AP-Key" => out.ap_key = val.to_string(),
            "AP-Name" => out.ap_name = val.to_string(),
            "PS5-RegistKey" => {
                let bytes = hex::decode(val)
                    .map_err(|_| RegistError::InvalidResponse("invalid PS5-RegistKey hex"))?;
                if bytes.is_empty() || bytes.len() > 16 {
                    return Err(RegistError::InvalidResponse("invalid PS5-RegistKey length"));
                }
                out.rp_regist_key = [0u8; 16];
                out.rp_regist_key[..bytes.len()].copy_from_slice(&bytes);
                regist_key_found = true;
            }
            "RP-KeyType" => {
                out.rp_key_type = parse_u32_auto(val)
                    .map_err(|_| RegistError::InvalidResponse("invalid RP-KeyType"))?;
            }
            "RP-Key" => {
                let bytes = hex::decode(val)
                    .map_err(|_| RegistError::InvalidResponse("invalid RP-Key hex"))?;
                if bytes.len() != 16 {
                    return Err(RegistError::InvalidResponse("RP-Key must be 16 bytes"));
                }
                out.rp_key.copy_from_slice(&bytes);
                key_found = true;
            }
            "PS5-Mac" => {
                let bytes = hex::decode(val)
                    .map_err(|_| RegistError::InvalidResponse("invalid PS5-Mac hex"))?;
                if bytes.len() != 6 {
                    return Err(RegistError::InvalidResponse("PS5-Mac must be 6 bytes"));
                }
                out.ps5_mac.copy_from_slice(&bytes);
                mac_found = true;
            }
            _ => {}
        }
    }

    if !regist_key_found {
        return Err(RegistError::InvalidResponse("missing PS5-RegistKey"));
    }
    if !key_found {
        return Err(RegistError::InvalidResponse("missing RP-Key"));
    }
    if !mac_found {
        return Err(RegistError::InvalidResponse("missing PS5-Mac"));
    }

    Ok(out)
}

/// Pairs with a LAN PS5 on the standard Remote Play registration port (`9295`).
pub async fn pair_with_ps5(
    host: IpAddr,
    account_id: [u8; 8],
    pin: u32,
) -> Result<RegistResult, RegistError> {
    pair_with_ps5_ports(host, REGIST_PORT, REGIST_PORT, account_id, pin).await
}

/// Pairs with a PS5 using explicit UDP (`SRC3`/`RES3`) and TCP (`/sie/ps5/rp/sess/rgst`) ports.
pub async fn pair_with_ps5_ports(
    host: IpAddr,
    udp_port: u16,
    tcp_port: u16,
    account_id: [u8; 8],
    pin: u32,
) -> Result<RegistResult, RegistError> {
    let bind_addr: SocketAddr = match host {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let udp = UdpSocket::bind(bind_addr).await?;
    udp.connect(SocketAddr::new(host, udp_port)).await?;
    udp.send(b"SRC3\0").await?;

    let deadline = Instant::now() + REGIST_TIMEOUT;
    let mut search_buf = [0u8; 256];
    let peer_ip = loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(RegistError::SearchTimeout(udp_port));
        };
        match timeout(remaining, udp.recv(&mut search_buf)).await {
            Ok(Ok(n)) if n >= 4 && &search_buf[..4] == b"RES3" => break host,
            Ok(Ok(_)) => continue,
            Ok(Err(e)) if e.kind() == io::ErrorKind::ConnectionRefused || e.kind() == io::ErrorKind::ConnectionReset => {
                return Err(RegistError::SearchRefused);
            }
            Ok(Err(e)) => return Err(RegistError::Io(e)),
            Err(_) => return Err(RegistError::SearchTimeout(udp_port)),
        }
    };
    drop(udp);

    sleep(SEARCH_REQUEST_SLEEP).await;

    let mut stream = timeout(REGIST_TIMEOUT, TcpStream::connect(SocketAddr::new(peer_ip, tcp_port)))
        .await
        .map_err(|_| RegistError::ResponseTimeout)??;
    let local_host = stream
        .local_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "10.0.2.15".to_string());

    let mut ambassador = [0u8; RPCRYPT_KEY_SIZE];
    rand::rng().fill_bytes(&mut ambassador);
    let (payload, crypt) = format_regist_payload(&ambassador, &account_id, pin, b'A')?;
    let header = format_regist_request_header(payload.len(), &local_host);

    timeout(REGIST_TIMEOUT, async {
        stream.write_all(&header).await?;
        stream.write_all(&payload).await?;
        stream.flush().await
    })
    .await
    .map_err(|_| RegistError::ResponseTimeout)??;

    let mut raw = Vec::with_capacity(1500);
    let header_end = timeout(REGIST_TIMEOUT, read_until_double_crlf(&mut stream, &mut raw))
        .await
        .map_err(|_| RegistError::ResponseTimeout)??;

    let header_text = std::str::from_utf8(&raw[..header_end])
        .map_err(|_| RegistError::InvalidResponse("HTTP header is not UTF-8"))?;
    let (status, reason, content_length) = parse_http_response_head(header_text)?;
    if status != 200 {
        return Err(RegistError::HttpError {
            status,
            reason,
            reason_desc: reason.map(rp_application_reason_str).unwrap_or("unknown"),
        });
    }
    let content_len = content_length
        .filter(|&len| len > 0 && len <= 16 * 1024)
        .ok_or(RegistError::InvalidResponse("missing or invalid Content-Length"))?;

    let total_needed = header_end + content_len;
    if raw.len() < total_needed {
        timeout(REGIST_TIMEOUT, async {
            while raw.len() < total_needed {
                let mut chunk = [0u8; 1024];
                let to_read = (total_needed - raw.len()).min(chunk.len());
                let n = stream.read(&mut chunk[..to_read]).await?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "truncated PS5 pairing response body",
                    ));
                }
                raw.extend_from_slice(&chunk[..n]);
            }
            Ok::<(), io::Error>(())
        })
        .await
        .map_err(|_| RegistError::ResponseTimeout)??;
    }

    let decrypted = crypt.decrypt(0, &raw[header_end..total_needed]);
    parse_regist_response_headers(&decrypted, peer_ip)
}

async fn read_until_double_crlf(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
) -> Result<usize, RegistError> {
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(pos) = find_double_crlf(buf) {
            return Ok(pos);
        }
        if buf.len() >= 8192 {
            return Err(RegistError::InvalidResponse("HTTP header too large"));
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(RegistError::InvalidResponse("EOF before HTTP header end"));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
}

fn parse_http_response_head(head: &str) -> Result<(u16, Option<u32>, Option<usize>), RegistError> {
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or(RegistError::InvalidResponse("empty HTTP response"))?;
    let mut parts = status_line.split_whitespace();
    let _proto = parts
        .next()
        .ok_or(RegistError::InvalidResponse("missing HTTP version"))?;
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or(RegistError::InvalidResponse("invalid HTTP status code"))?;

    let mut reason = None;
    let mut content_length = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let val = v.trim();
        if k.trim().eq_ignore_ascii_case("RP-Application-Reason") {
            reason = parse_u32_auto(val).ok();
        } else if k.trim().eq_ignore_ascii_case("Content-Length") {
            content_length = val.parse::<usize>().ok();
        }
    }

    Ok((status, reason, content_length))
}

fn parse_u32_auto(s: &str) -> Result<u32, ()> {
    let s = s.trim();
    if let Some(hex_str) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex_str, 16).map_err(|_| ())
    } else {
        s.parse::<u32>().map_err(|_| ())
    }
}

pub fn rp_application_reason_str(reason: u32) -> &'static str {
    match reason {
        0x8010_8b09 => "invalid PIN or pairing session expired",
        0x8010_8b02 => "console already registered or busy",
        0x8010_8b10 => "invalid account ID or PSN mismatch",
        0x8010_8b11 => "registration rejected by console",
        0x8010_8b15 => "remote play disabled in console settings",
        _ => "PS5 application error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpcrypt::ambassador_from_aeropause_ps5;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn ps5_regist_loopback_roundtrip() {
        let udp_srv = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let udp_port = udp_srv.local_addr().unwrap().port();
        let tcp_srv = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_port = tcp_srv.local_addr().unwrap().port();

        let expected_account = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let pin = 1234_5678u32;

        let server = tokio::spawn(async move {
            let mut ubuf = [0u8; 64];
            let (n, peer) = udp_srv.recv_from(&mut ubuf).await.unwrap();
            assert_eq!(&ubuf[..n], b"SRC3\0");
            udp_srv.send_to(b"RES3", peer).await.unwrap();

            let (mut sock, _) = tcp_srv.accept().await.unwrap();
            let mut req = Vec::new();
            let head_end = read_until_double_crlf(&mut sock, &mut req).await.unwrap();
            let head = std::str::from_utf8(&req[..head_end]).unwrap().to_string();
            assert!(head.starts_with("POST /sie/ps5/rp/sess/rgst HTTP/1.1\r\n"));
            let (_, _, content_len) = parse_http_response_head(&format!(
                "HTTP/1.1 200 OK\r\n{}",
                head.split_once("\r\n\r\n")
                    .map(|_| head.split("\r\n").skip(1).collect::<Vec<_>>().join("\r\n"))
                    .unwrap_or_default()
            ))
            .unwrap();
            let body_len = content_len.unwrap();
            while req.len() < head_end + body_len {
                let mut tmp = [0u8; 1024];
                let r = sock.read(&mut tmp).await.unwrap();
                assert!(r > 0);
                req.extend_from_slice(&tmp[..r]);
            }
            let body = &req[head_end..head_end + body_len];
            assert!(body.len() > INNER_HEADER_OFF);

            let key_0_off = usize::from(body[0x18d] & 0x1f);
            let key_1_off = usize::from(body[0] >> 3);
            let mut aeropause = [0u8; 16];
            aeropause[0..8].copy_from_slice(&body[0x191..0x199]);
            aeropause[8..16].copy_from_slice(&body[0xc7..0xcf]);
            let ambassador = ambassador_from_aeropause_ps5(key_1_off, &aeropause).unwrap();
            let srv_crypt = Rpcrypt::init_regist_ps5(ambassador, key_0_off, pin).unwrap();
            let dec = srv_crypt.decrypt(0, &body[INNER_HEADER_OFF..]);
            let dec_str = std::str::from_utf8(&dec).unwrap();
            assert!(dec_str.contains(PS5_CLIENT_TYPE));
            let expected_b64 = BASE64_STANDARD.encode(expected_account);
            assert!(dec_str.contains(&format!("Np-AccountId: {expected_b64}\r\n")));

            let resp_plain = "\
                PS5-Nickname: LivingRoom-PS5\r\n\
                PS5-RegistKey: 3031323334353637\r\n\
                RP-KeyType: 2\r\n\
                RP-Key: 00112233445566778899aabbccddeeff\r\n\
                PS5-Mac: aabbccddeeff\r\n";
            let resp_enc = srv_crypt.encrypt(0, resp_plain.as_bytes());
            let resp_head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                resp_enc.len()
            );
            sock.write_all(resp_head.as_bytes()).await.unwrap();
            sock.write_all(&resp_enc).await.unwrap();
            sock.flush().await.unwrap();
        });

        let result = pair_with_ps5_ports(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            udp_port,
            tcp_port,
            expected_account,
            pin,
        )
        .await
        .unwrap();

        server.await.unwrap();
        assert_eq!(result.nickname, "LivingRoom-PS5");
        assert_eq!(&result.rp_regist_key[..8], b"01234567");
        assert_eq!(&result.rp_regist_key[8..], &[0u8; 8]);
        assert_eq!(result.rp_key_type, 2);
        assert_eq!(
            result.rp_key,
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
                0xdd, 0xee, 0xff
            ]
        );
        assert_eq!(result.ps5_mac, [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
    }
}
