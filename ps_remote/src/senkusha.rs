use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::time::timeout;

/// Default UDP port used by Remote Play Senkusha path/RTT probing.
pub const SENKUSHA_PORT: u16 = 9297;

const DEFAULT_MTU: u32 = 1454;
const DEFAULT_RTT_US: u64 = 2_000;
const PROBE_MAGIC: &[u8; 8] = b"SNKSPING";

/// Measured or fallback network parameters for a PS5 streaming path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SenkushaResult {
    pub mtu_in: u32,
    pub mtu_out: u32,
    pub rtt_us: u64,
    pub probed: bool,
}

impl Default for SenkushaResult {
    fn default() -> Self {
        Self {
            mtu_in: DEFAULT_MTU,
            mtu_out: DEFAULT_MTU,
            rtt_us: DEFAULT_RTT_US,
            probed: false,
        }
    }
}

/// Run a bounded UDP 9297 RTT probe against the PS5 console.
///
/// If the console does not answer within `per_probe_timeout` (up to `attempts` probes),
/// this gracefully falls back to `SenkushaResult::default()` (`mtu = 1454`, `rtt = 2000 us`),
/// matching Chiaki-ng behavior on PS5 LAN connections.
pub async fn probe_senkusha(
    host: IpAddr,
    port: u16,
    per_probe_timeout: Duration,
) -> SenkushaResult {
    let bind_addr: SocketAddr = match host {
        IpAddr::V4(_) => "0.0.0.0:0".parse().unwrap(),
        IpAddr::V6(_) => "[::]:0".parse().unwrap(),
    };
    let Ok(sock) = UdpSocket::bind(bind_addr).await else {
        return SenkushaResult::default();
    };
    if sock.connect(SocketAddr::new(host, port)).await.is_err() {
        return SenkushaResult::default();
    }

    let mut samples_us = Vec::with_capacity(3);
    let mut recv_buf = [0u8; 1500];

    for seq in 0..3u32 {
        let mut packet = [0u8; 16];
        packet[..8].copy_from_slice(PROBE_MAGIC);
        packet[8..12].copy_from_slice(&seq.to_be_bytes());
        let sent_at = Instant::now();
        if sock.send(&packet).await.is_err() {
            break;
        }
        match timeout(per_probe_timeout, sock.recv(&mut recv_buf)).await {
            Ok(Ok(n)) if n >= 4 => {
                let elapsed = sent_at.elapsed().as_micros() as u64;
                samples_us.push(elapsed.max(100));
            }
            _ => continue,
        }
    }

    if samples_us.is_empty() {
        return SenkushaResult::default();
    }

    samples_us.sort_unstable();
    let median_rtt = samples_us[samples_us.len() / 2];
    SenkushaResult {
        mtu_in: DEFAULT_MTU,
        mtu_out: DEFAULT_MTU,
        rtt_us: median_rtt,
        probed: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn senkusha_falls_back_on_timeout_and_measures_echo() {
        // Closed/unanswered loopback port -> bounded fallback
        let fallback_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let dead_port = fallback_sock.local_addr().unwrap().port();
        drop(fallback_sock);

        let res = probe_senkusha(
            "127.0.0.1".parse().unwrap(),
            dead_port,
            Duration::from_millis(25),
        )
        .await;
        assert_eq!(res, SenkushaResult::default());
        assert!(!res.probed);
        assert_eq!(res.mtu_in, 1454);
        assert_eq!(res.rtt_us, 2_000);

        // Responding loopback peer -> probed = true
        let echo = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let echo_port = echo.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            for _ in 0..3 {
                let (n, peer) = echo.recv_from(&mut buf).await.unwrap();
                let _ = echo.send_to(&buf[..n], peer).await.unwrap();
            }
        });

        let measured = probe_senkusha(
            "127.0.0.1".parse().unwrap(),
            echo_port,
            Duration::from_millis(200),
        )
        .await;
        assert!(measured.probed);
        assert_eq!(measured.mtu_out, 1454);
        assert!(measured.rtt_us >= 100);
        task.await.unwrap();
    }
}
