//! The background service owns the tray. A viewer window owns no tray or host.
use crate::{UnifiedRuntimeHandle, background_service, desktop, private_ipc};
use std::{future::Future, path::{Path, PathBuf}, sync::Arc};
use tokio::sync::{Notify, mpsc};

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

type Error = Box<dyn std::error::Error + Send + Sync>;
#[derive(Clone, Copy, Debug)]
pub(super) enum Action { Open, Quit }

/// Close the current frontend through its authenticated same-user endpoint.
/// A startup race or changed owner is an error, never a process signal.
fn close_gui(profile: &Path, expected_hash: &str) -> Result<(), Error> {
    let directory = private_ipc::gui_directory(profile)?;
    if private_ipc::absent(&directory, "gui-receive.sock")? {
        let path = directory.join("desktop-instance.lock");
        if path.exists() {
            private_ipc::private_file(&path)?;
            let lock = std::fs::OpenOptions::new().read(true).open(path)?;
            lock.try_lock().map_err(|_| "The Weavelo window is starting; wait before quitting")?;
        }
        return Ok(());
    }
    let status = desktop::directed_receive::exchange(&directory, &desktop::directed_receive::Request::Status)?;
    if status["ok"] != true || status["executable_sha256"] != expected_hash
        || status["renderer"] != crate::gui_backend::ORIGINAL_RENDERER {
        return Err("The current Weavelo window has a different executable identity".into());
    }
    let generation = status["generation"].as_str().ok_or("GUI generation unavailable")?;
    desktop::upgrade_quit::command(&directory, &[
        "--gui-close-json".into(), generation.into(), expected_hash.into(),
    ])?;
    Ok(())
}

async fn open_gui(profile: &Path) -> Result<(), Error> {
    let mut command = tokio::process::Command::new(std::env::current_exe()?.canonicalize()?);
    command.arg("--gui").env("WEAVELO_DEVICE_GROUP_DIR", profile)
        .env_remove("WEAVELO_HEADLESS")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0800_0000); // Native GUI launch has no console window.
    let mut child = command.spawn()?;
    // Keep reaping activation-only children while an existing window is open.
    tokio::spawn(async move { let _ = child.wait().await; });
    Ok(())
}

async fn actions(profile: PathBuf, runtime: Arc<UnifiedRuntimeHandle>,
    stop: impl Future<Output = ()> + Send + 'static, mut receiver: mpsc::UnboundedReceiver<Action>) -> Result<(), Error> {
    let hash = background_service::executable_hash()?;
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = &mut stop => return Ok(()),
            result = tokio::signal::ctrl_c() => { result?;runtime.owner.shutdown_host().await?;return Ok(()); },
            action = receiver.recv() => match action {
                Some(Action::Open) => if let Err(error) = open_gui(&profile).await { eprintln!("Weavelo tray: {error}"); },
                Some(Action::Quit) => {
                    let selected = profile.clone();let expected = hash.clone();
                    let closed = tokio::task::spawn_blocking(move || close_gui(&selected, &expected)).await?;
                    if let Err(error) = closed {eprintln!("Weavelo quit was not completed: {error}");continue;}
                    runtime.owner.shutdown_host().await?;
                    return Ok(());
                }
                None => return Err("Weavelo tray action channel closed".into()),
            }
        }
    }
}

pub(crate) async fn run(profile: PathBuf, runtime: Arc<UnifiedRuntimeHandle>, stopped: Arc<Notify>) -> Result<(), Error> {
    let (sender,receiver) = mpsc::unbounded_channel();
    // Register before creating native UI: a concurrent verified shutdown must
    // also be observed when Notify::notify_waiters runs during initialization.
    let mut stop = Box::pin(stopped.notified_owned());
    stop.as_mut().enable();
    let future = actions(profile, runtime, stop, receiver);
    #[cfg(target_os = "macos")]
    { mac::run(sender, future).await }
    #[cfg(target_os = "linux")]
    { let _tray = linux::Tray::start(sender).await?;future.await }
    #[cfg(target_os = "windows")]
    { let _tray = windows::Tray::start(sender)?;future.await }
}
