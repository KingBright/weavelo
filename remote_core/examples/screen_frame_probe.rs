//! Bounded, paired screen sample for distinguishing capture/encode from presentation.
//! No input, permissions, profile changes, or background recording.
use protocol::session::{CaptureSource, SessionCommand, SubscriptionRequest};
use remote_core::{SharedHostStats, Statistics, workspace_session::WorkspaceConnection};
use std::{io::Write, sync::Arc, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        return Err("screen_frame_probe LAN_ENDPOINT NEW_OUTPUT.hevc [WINDOW_ID]".into());
    }
    let source = args.get(2).map(|id| id.parse().map(CaptureSource::Window)).transpose()?.unwrap_or(CaptureSource::MainDisplay);
    let target: std::net::SocketAddr = args[0].parse()?;
    if !matches!(target.ip(), std::net::IpAddr::V4(ip) if ip.is_private()) {
        return Err("Use the owned device's LAN endpoint".into());
    }
    let identity = remote_core::mesh::AppPrivateMeshConfigStore::new(
        remote_core::mesh::default_app_private_mesh_dir(),
    )
    .load()?
    .ok_or("Existing paired profile required")?;
    remote_core::init_crypto_provider();
    remote_core::session_crypto::use_paired_session_secret(identity.network_secret.expose_secret());
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&args[1])?;
    let (conn, mut events) = tokio::time::timeout(
        Duration::from_secs(10),
        WorkspaceConnection::connect_media(target),
    )
    .await??;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        conn.control(SessionCommand::ListSources { request_id: 1, known_icons: vec![] }).await?;
        loop {
            match events.control.recv().await.ok_or("Connection ended before source catalog")? {
                SessionCommand::Sources { sources, .. } => {
                    eprintln!("{}", serde_json::json!({"catalog": sources}));
                    if !sources.iter().any(|entry| entry.source == source) { return Err("Requested source absent from catalog".into()); }
                    break;
                }
                SessionCommand::Error { reason, .. } | SessionCommand::Closed { reason, .. } => return Err(reason),
                _ => {}
            }
        }
        let (tx, mut video) = remote_core::video_queue::channel(16);
        conn.subscribe(SubscriptionRequest { id: 85, source,
            width: 1280, height: 720, fps: 30, bitrate_kbps: 4000, audio: false },
            tx, Arc::new(Statistics::default()), Arc::new(SharedHostStats::default())).await?;
        let mut frames = 0;
        let mut bytes = 0;
        let mut first_geometry = None;
        while frames < 8 {
            tokio::select! {
                event = events.control.recv() => match event {
                    Some(SessionCommand::Error { reason, .. } | SessionCommand::Closed { reason, .. }) => return Err(reason),
                    None => return Err("Connection ended".into()),
                    _ => {}
                },
                packet = video.recv() => {
                    let (packet, _, _) = packet.ok_or("Video route ended")?;
                    let (geometry, hevc) = protocol::viewport::split_frame(&packet.payload)?;
                    if packet.header.ssrc != 85 || bytes + hevc.len() > 32 * 1024 * 1024 {
                        return Err("Unexpected subscription or excessive sample".into());
                    }
                    if frames == 0 { first_geometry = geometry; }
                    file.write_all(hevc).map_err(|e| e.to_string())?;
                    bytes += hevc.len(); frames += 1;
                }
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        Ok::<_, String>(serde_json::json!({ "frames": frames, "bytes": bytes, "geometry": first_geometry }))
    }).await;
    let closed = conn.close_verified().await;
    eprintln!("cleanup_confirmed={}", closed.is_ok());
    let mut report = result??;
    report["cleanup_confirmed"] = closed.is_ok().into();
    println!("{report}");
    closed?;
    Ok(())
}
