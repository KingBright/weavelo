//! Directed delivery owns a control-only WorkspaceConnection. The target uses
//! the normal pull connection after proving the one-time sending intention.
use protocol::session::SessionCommand;
use remote_core::workspace_session::WorkspaceConnection;
use std::time::Duration;
type Error = Box<dyn std::error::Error + Send + Sync>;

pub(crate) use remote_core::directed_delivery::claim;

#[cfg(any(unix, windows))]
mod sender {
    use super::*;
    use host::directed_delivery::LocalIntent;
    use std::{
        net::SocketAddr,
        path::PathBuf,
        sync::{Arc, Mutex},
        time::Instant,
    };
    static NEXT_REGISTRATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    pub(crate) struct SendHandle {
        pub target: String,
        stop: tokio::sync::watch::Sender<bool>,
        heartbeat: Arc<Mutex<Instant>>,
        status: Arc<Mutex<String>>,
        task: tokio::task::JoinHandle<()>,
    }
    impl SendHandle {
        pub fn start(
            profile: PathBuf,
            generation: String,
            source: String,
            target: String,
            routes: Vec<(String, SocketAddr)>,
            sources: Vec<protocol::session::CaptureSource>,
            upgrade_operation: super::super::upgrade_quit::Operation,
        ) -> Self {
            let (stop, mut cancelled) = tokio::sync::watch::channel(false);
            let heartbeat = Arc::new(Mutex::new(Instant::now()));
            let beat = heartbeat.clone();
            let status = Arc::new(Mutex::new("Contacting target".into()));
            let progress = status.clone();
            let peer = target.clone();
            let task = tokio::spawn(async move {
                let _upgrade_operation=upgrade_operation;
                let result = run(
                    profile,
                    generation,
                    source,
                    peer,
                    routes,
                    sources,
                    &mut cancelled,
                    beat,
                    progress.clone(),
                )
                .await;
                *progress.lock().unwrap() = match result {
                    Ok(()) => "Sending stopped".into(),
                    Err(error) => format!("Delivery stopped: {error}"),
                };
            });
            Self {
                target,
                stop,
                heartbeat,
                status,
                task,
            }
        }
        pub fn poll(&self) -> String {
            *self.heartbeat.lock().unwrap() = Instant::now();
            self.status.lock().unwrap().clone()
        }
        pub fn is_finished(&self) -> bool {
            self.task.is_finished()
        }
    }
    impl Drop for SendHandle {
        fn drop(&mut self) {
            self.stop.send_replace(true);
        }
    }
    async fn local(
        client: &crate::background_service::Client,
        intent: LocalIntent,
    ) -> Result<(), Error> {
        let client = client.clone();
        tokio::task::spawn_blocking(move || client.delivery(intent)).await?
    }
    async fn run(
        profile: PathBuf,
        generation: String,
        source: String,
        target: String,
        routes: Vec<(String, SocketAddr)>,
        sources: Vec<protocol::session::CaptureSource>,
        cancelled: &mut tokio::sync::watch::Receiver<bool>,
        heartbeat: Arc<Mutex<Instant>>,
        status: Arc<Mutex<String>>,
    ) -> Result<(), Error> {
        protocol::session::validate_delivery_sources(&sources)?;
        let scope=remote_core::shared_files::current_share_scope();
        let sequence = NEXT_REGISTRATION
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |n| n.checked_add(1),
            )
            .map_err(|_| "Sending sequence exhausted")?;
        let client = tokio::task::spawn_blocking(move || {
            crate::background_service::Client::connect(&profile)
        })
        .await??;
        let listing=client.clone();
        let available=tokio::task::spawn_blocking(move||listing.capture_sources()).await??;
        if sources.iter().any(|source|!available.iter().any(|info|info.source==*source)) {
            return Err("A selected source ended or is no longer shared".into());
        }
        let mut connected = None;
        for (_, endpoint) in routes.into_iter().take(3) {
            let result = tokio::select! {biased;_=cancelled.changed()=>return Ok(()),result=tokio::time::timeout(Duration::from_secs(5),WorkspaceConnection::connect_media(endpoint))=>result};
            if let Ok(Ok(connection)) = result {
                connected = Some(connection);
                break;
            }
        }
        let (conn, mut events) = connected.ok_or("No authenticated route to target")?;
        let mut ticket = [0; 32];
        ticket[..16].copy_from_slice(&remote_core::session_crypto::random_bytes_16());
        ticket[16..].copy_from_slice(&remote_core::session_crypto::random_bytes_16());
        let request = u64::from_le_bytes(ticket[..8].try_into().unwrap()).max(1);
        if scope!=remote_core::shared_files::current_share_scope() {return Err("The device network changed".into());}
        local(
            &client,
            LocalIntent::Register {
                ticket,
                target: target.clone(),
                generation: generation.clone(),
                connection: conn.id,
                sequence,
                sources: sources.clone(),
            },
        )
        .await?;
        let renewing = client.clone();
        let renewing_generation = generation.clone();
        let result = remote_core::directed_delivery::maintain_offer(&conn, &mut events,
            remote_core::directed_delivery::OutgoingOffer {source, target, ticket, request, sources, scope}, cancelled,
            || heartbeat.lock().unwrap().elapsed() <= Duration::from_secs(10),
            move |revision| {
                let client = renewing.clone(); let generation = renewing_generation.clone();
                async move {local(&client, LocalIntent::Renew {ticket, generation, revision}).await}
            },
            |state| *status.lock().unwrap() = remote_core::directed_delivery::sending_status(state).into(),
        ).await;
        // Revoke locally first even when the target's cancellation reply is lost.
        let revoked = local(&client, LocalIntent::Revoke { ticket, generation }).await;
        let _ = conn
            .control(SessionCommand::DeliveryCancel {
                request_id: request,
            })
            .await;
        conn.close().await;
        revoked?;
        result
    }
}
#[cfg(any(unix, windows))]
pub(crate) use sender::SendHandle;
