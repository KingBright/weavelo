//! Use only with LivePublisherTouchTest's currently focused input-only fixture.
//! Receiver-side MotionEvents, not these enqueue results, determine acceptance.
use protocol::{
    InputEvent, TouchAction,
    session::{CaptureSource, SessionCommand, SubscriptionRequest},
    touch::Contact,
};
use remote_core::{SharedHostStats, Statistics, workspace_session::WorkspaceConnection};
use std::{sync::Arc, time::Duration};

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("private-phone-ip:port current-fixture-ready.json required".into());
    }
    let target: std::net::SocketAddr = args[0].parse().map_err(|_| "Invalid fixture endpoint")?;
    if !matches!(target.ip(),std::net::IpAddr::V4(ip) if ip.is_private()) {
        return Err("Owned LAN fixture only".into());
    }
    let raw = std::fs::read(&args[1]).map_err(|e| e.to_string())?;
    let ready: serde_json::Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    let nonce = ready["nonce"]
        .as_str()
        .filter(|s| s.len() == 36)
        .ok_or("Missing fixture nonce")?;
    if ready["capture"] != false {
        return Err("Expected explicit input-only fixture".into());
    }
    let width = ready["width"]
        .as_u64()
        .filter(|x| (200..=8192).contains(x))
        .ok_or("Invalid width")? as u32;
    let height = ready["height"]
        .as_u64()
        .filter(|x| (200..=8192).contains(x))
        .ok_or("Invalid height")? as u32;
    let identity = remote_core::mesh::AppPrivateMeshConfigStore::new(
        remote_core::mesh::default_app_private_mesh_dir(),
    ).load().map_err(|_| "Existing paired profile unavailable")?
        .ok_or("Existing paired profile required")?;
    remote_core::init_crypto_provider();
    remote_core::session_crypto::use_paired_session_secret(identity.network_secret.expose_secret());
    let (connection, mut events) = WorkspaceConnection::connect_media(target).await?;
    let outcome = tokio::time::timeout(Duration::from_secs(12), async {
        let (video, _sink) = remote_core::video_queue::channel(2);
        connection
            .subscribe(
                SubscriptionRequest {
                    id: 83,
                    source: CaptureSource::MainDisplay,
                    width,
                    height,
                    fps: 10,
                    bitrate_kbps: 1000,
                    audio: false,
                },
                video,
                Arc::new(Statistics::default()),
                Arc::new(SharedHostStats::default()),
            )
            .await?;
        loop {
            match events.control.recv().await.ok_or("Connection ended")? {
                SessionCommand::Subscribed {
                    id: 83,
                    supports_input: true,
                    max_touch_points,
                    ..
                } if max_touch_points >= 2 => break,
                SessionCommand::Error { reason, .. } | SessionCommand::Closed { reason, .. } => {
                    return Err(reason);
                }
                SessionCommand::Subscribed { .. } => return Err("Native touch is not ready".into()),
                _ => {}
            }
        }
        let binding = loop {
            if let Some(binding) = connection.input_binding(83, 0) {
                break binding;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let point = |pointer_id, action, x, y| Contact {
            pointer_id,
            action,
            normalized_x: x,
            normalized_y: y,
            pressure: if matches!(action, TouchAction::Up) {
                0.
            } else {
                1.
            },
        };
        use TouchAction::{Down, Move, Up};
        let samples = [
            vec![point(7, Down, 0.3, 0.4)],
            vec![point(7, Move, 0.35, 0.45), point(9, Down, 0.7, 0.6)],
            vec![point(7, Move, 0.4, 0.5), point(9, Move, 0.65, 0.55)],
            vec![point(9, Up, 0.65, 0.55)],
            vec![point(7, Move, 0.45, 0.55)],
            vec![point(7, Up, 0.45, 0.55)],
        ];
        for contacts in samples {
            if !connection.refresh_bound_input(binding)
                || !connection.try_bound_input(binding, Some(InputEvent::TouchFrame(contacts)))
            {
                return Err("Fixture input binding ended".into());
            }
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
        connection.try_bound_input(binding, None);
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "Fixture operation timed out".to_owned())
    .and_then(|result| result);
    let cleanup = connection.close_verified().await;
    println!(
        "{}",
        serde_json::json!({"fixture_nonce":nonce,"enqueued":outcome.is_ok(),
        "cleanup_confirmed":cleanup.is_ok(),"native_acceptance":"read receiver MotionEvent report"})
    );
    outcome?;
    cleanup
}
