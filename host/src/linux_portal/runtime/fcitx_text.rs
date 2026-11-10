//! Native committed text over a private, peer-verified connection to Fcitx.
//! The caller retains the RemoteDesktop grant and revalidates it after prepare.

use super::PortalInputError;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use zbus::Connection;

const SERVICE: &str = "org.fcitx.Fcitx5";
const SOCKET_PREFIX: &str = "weavelo-fcitx-text-v1-";
const FCITX_EXECUTABLE: &str = "/usr/bin/fcitx5";

type Result<T> = std::result::Result<T, PortalInputError>;

pub(super) struct PreparedCommit(UnixStream);

pub(super) async fn prepare(
    connection: &Connection,
    deadline: Instant,
) -> Result<Option<PreparedCommit>> {
    tokio::time::timeout_at(deadline.into(), async {
        let bus = zbus::fdo::DBusProxy::new(connection)
            .await
            .map_err(|_| PortalInputError::TextUnavailable)?;
        let service = zbus::names::BusName::try_from(SERVICE).expect("static D-Bus name");
        let pid = match bus.get_connection_unix_process_id(service).await {
            Ok(pid) => pid,
            Err(zbus::fdo::Error::NameHasNoOwner(_) | zbus::fdo::Error::ServiceUnknown(_)) => {
                return Ok(None);
            }
            Err(_) => return Err(PortalInputError::TextUnavailable),
        };

        let uid = unsafe { libc::geteuid() };
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or(PortalInputError::TextUnavailable)?;
        let metadata =
            fs::symlink_metadata(&runtime).map_err(|_| PortalInputError::TextUnavailable)?;
        if !runtime.is_absolute()
            || !metadata.is_dir()
            || metadata.uid() != uid
            || metadata.mode() & 0o077 != 0
        {
            return Err(PortalInputError::TextUnavailable);
        }

        let mut stream = UnixStream::connect(runtime.join(format!("{SOCKET_PREFIX}{pid}.sock")))
            .await
            .map_err(|_| PortalInputError::TextUnavailable)?;
        let peer = stream
            .peer_cred()
            .map_err(|_| PortalInputError::TextUnavailable)?;
        if peer.uid() != uid
            || peer.pid().and_then(|p| u32::try_from(p).ok()) != Some(pid)
            || !system_fcitx(pid)
        {
            return Err(PortalInputError::TextUnavailable);
        }
        // Until this point no text has been sent. The peer acknowledges that
        // it has pinned a focused context on this particular connection.
        match stream.read_u8().await {
            Ok(1) => Ok(Some(PreparedCommit(stream))),
            Ok(_) => Err(PortalInputError::TextFocusChanged),
            Err(_) => Err(PortalInputError::TextUnavailable),
        }
    })
    .await
    .map_err(|_| PortalInputError::Expired)?
}

fn system_fcitx(pid: u32) -> bool {
    let Ok(expected) = fs::metadata(FCITX_EXECUTABLE) else {
        return false;
    };
    let Ok(actual) = fs::metadata(format!("/proc/{pid}/exe")) else {
        return false;
    };
    expected.is_file()
        && expected.uid() == 0
        && expected.mode() & 0o022 == 0
        && (expected.dev(), expected.ino()) == (actual.dev(), actual.ino())
}

impl PreparedCommit {
    pub(super) async fn send(mut self, text: &str, deadline: Instant) -> Result<()> {
        let expires = monotonic_deadline(deadline)?;
        tokio::time::timeout_at(deadline.into(), async {
            let len = u16::try_from(text.len()).map_err(|_| PortalInputError::InvalidEvent)?;
            let mut header = [0; 10];
            header[..8].copy_from_slice(&expires.to_le_bytes());
            header[8..].copy_from_slice(&len.to_le_bytes());
            self.0
                .write_all(&header)
                .await
                .map_err(|_| PortalInputError::TextUnavailable)?;
            self.0
                .write_all(text.as_bytes())
                .await
                .map_err(|_| PortalInputError::TextUnavailable)?;
            // Only a complete, half-closed frame can commit. Dropping an
            // incomplete frame on expiry is never a request to retry.
            self.0
                .shutdown()
                .await
                .map_err(|_| PortalInputError::TextUnavailable)?;
            match self.0.read_u8().await {
                Ok(1) => Ok(()),
                _ => Err(PortalInputError::TextFocusChanged),
            }
        })
        .await
        .map_err(|_| PortalInputError::Expired)?
    }
}

fn monotonic_deadline(deadline: Instant) -> Result<u64> {
    let mut clock = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut clock) } != 0 {
        return Err(PortalInputError::TextUnavailable);
    }
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or(PortalInputError::Expired)?;
    let now = (clock.tv_sec as u64) * 1_000_000_000 + clock.tv_nsec as u64;
    Ok(now.saturating_add(remaining.as_nanos().min(u128::from(u64::MAX)) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn sends_exact_utf8_length_and_waits_for_native_ack() {
        let (client, mut addon) = UnixStream::pair().unwrap();
        let text = "中文".repeat(169) + "🙂AB1234";
        let expected = text.clone();
        assert_eq!(text.len(), protocol::interaction::MAX_COMMITTED_TEXT_BYTES);
        let server = tokio::spawn(async move {
            let mut frame = Vec::new();
            addon.read_to_end(&mut frame).await.unwrap();
            assert_eq!(
                u16::from_le_bytes(frame[8..10].try_into().unwrap()) as usize,
                expected.len()
            );
            assert_eq!(&frame[10..], expected.as_bytes());
            addon.write_u8(1).await.unwrap();
        });
        PreparedCommit(client)
            .send(&text, Instant::now() + Duration::from_millis(250))
            .await
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn expired_text_does_not_reach_the_addon() {
        let (client, mut addon) = UnixStream::pair().unwrap();
        let deadline = Instant::now() - Duration::from_millis(1);
        assert_eq!(
            PreparedCommit(client).send("中文", deadline).await,
            Err(PortalInputError::Expired)
        );
        let mut frame = Vec::new();
        addon.read_to_end(&mut frame).await.unwrap();
        assert!(frame.is_empty());
    }
}
