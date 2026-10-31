//! The canonical background owner holds portal grants; frontends send only
//! bounded commands and receive scalar status over the existing private IPC.
mod store;
use crate::sharing_permissions::{ShareKind, SharedSource, SharingCall, SharingSnapshot};
use host::{PersistentPortalRestore, PreparedPortalCapture, service::LocalPortalShareCommand};
use protocol::session::CaptureSource;
use remote_core::shared_files::{ShareScope, current_share_scope};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};

struct Pending {
    id: u64,
    scope: ShareScope,
    cancel: watch::Sender<bool>,
}
struct Source {
    id: u64,
    kind: ShareKind,
    scope: ShareScope,
    capture: PreparedPortalCapture,
    _cancel: watch::Sender<bool>,
    remembered: bool,
}
#[derive(Default)]
struct State {
    serial: u64,
    pending: Option<Pending>,
    sources: Vec<Source>,
    display: Option<CaptureSource>,
    stopping: bool,
    message: Option<String>,
}
pub(crate) struct Manager {
    control: Option<mpsc::Sender<LocalPortalShareCommand>>,
    state: Mutex<State>,
    store: Mutex<Option<store::Store>>,
    publication: tokio::sync::Mutex<()>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    closing: AtomicBool,
    cleanup_failed: AtomicBool,
}
impl Manager {
    pub fn new(
        profile: &Path,
        control: Option<mpsc::Sender<LocalPortalShareCommand>>,
    ) -> Arc<Self> {
        let mut state = State::default();
        let store = match store::Store::load(profile) {
            Ok(store) => Some(store),
            Err(_) => {
                state.message = Some(
                    "Saved sharing selection could not be opened; existing data was retained"
                        .into(),
                );
                None
            }
        };
        if let Some(store) = &store {
            state.serial = store
                .keys(current_share_scope())
                .iter()
                .map(|(id, _)| *id)
                .max()
                .unwrap_or(0);
        }
        Arc::new(Self {
            control,
            state: Mutex::new(state),
            store: Mutex::new(store),
            publication: tokio::sync::Mutex::new(()),
            tasks: Mutex::new(Vec::new()),
            closing: AtomicBool::new(false),
            cleanup_failed: AtomicBool::new(false),
        })
    }
    fn spawn(self: &Arc<Self>, work: impl std::future::Future<Output = ()> + Send + 'static) {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.retain(|task| !task.is_finished());
        tasks.push(tokio::spawn(work));
    }
    pub fn snapshot(&self) -> SharingSnapshot {
        let state = self.state.lock().unwrap();
        let scope = current_share_scope();
        SharingSnapshot {
            selecting: state.pending.as_ref().map(|pending| pending.id),
            stopping: state.stopping || self.closing.load(Ordering::Acquire),
            message: state.message.clone(),
            sources: state
                .sources
                .iter()
                .filter(|source| source.scope == scope && !source.capture.is_revoked())
                .map(|source| {
                    let info = source.capture.source_info();
                    SharedSource {
                        source: info.source,
                        label: format!(
                            "{} {}",
                            if source.kind == ShareKind::Window {
                                "Window"
                            } else {
                                "Display"
                            },
                            source.id
                        ),
                        kind: source.kind,
                        supports_input: info.supports_input,
                        remembered: source.remembered,
                    }
                })
                .collect(),
        }
    }
    pub fn catalog(&self) -> Vec<protocol::session::CaptureSourceInfo> {
        let scope=current_share_scope();
        self.state.lock().unwrap().sources.iter()
            .filter(|source|source.scope==scope && !source.capture.is_revoked())
            .map(|source|source.capture.source_info()).collect()
    }
    pub fn call(self: &Arc<Self>, call: SharingCall) -> Result<SharingSnapshot, String> {
        if self.closing.load(Ordering::Acquire) && !matches!(&call, SharingCall::Status) {
            return Err("Sharing service is stopping".into());
        }
        match call {
            SharingCall::Status => {}
            SharingCall::Select { kind, remember } => {
                self.select(kind, remember, None)?;
            }
            SharingCall::Cancel { attempt } => {
                let state = self.state.lock().unwrap();
                if let Some(pending) = &state.pending
                    && pending.id == attempt
                {
                    pending.cancel.send_replace(true);
                }
            }
            SharingCall::Stop { source } => {
                let should_stop = {
                    let state = self.state.lock().unwrap();
                    if let Some(item) = state.sources.iter().find(|item| {
                        item.capture.source_info().source == source && !item.capture.is_revoked()
                    }) {
                        item.capture.revoke();
                        true
                    } else {
                        false
                    }
                };
                if !should_stop {
                    return Ok(self.snapshot());
                }
                let owner = self.clone();
                self.spawn(async move {
                    owner.stop(Some(source), true).await;
                });
            }
            SharingCall::StopAll => {
                {
                    let mut state = self.state.lock().unwrap();
                    if state.stopping {
                        drop(state);
                        return Ok(self.snapshot());
                    }
                    state.stopping = true;
                    if let Some(pending) = &state.pending {
                        pending.cancel.send_replace(true);
                    }
                    for source in &state.sources {
                        source.capture.revoke();
                    }
                }
                let owner = self.clone();
                self.spawn(async move {
                    owner.stop(None, true).await;
                });
            }
        }
        Ok(self.snapshot())
    }
    pub fn restore_saved(self: &Arc<Self>) {
        let scope = current_share_scope();
        let keys = self
            .store
            .lock()
            .unwrap()
            .as_ref()
            .map(|store| store.keys(scope))
            .unwrap_or_default();
        if keys.is_empty() {
            return;
        }
        let owner = self.clone();
        self.spawn(async move {
            for (id, kind) in keys {
                if owner.closing.load(Ordering::Acquire) || scope != current_share_scope() {
                    break;
                }
                let attempt = match owner.select(kind, true, Some(id)) {
                    Ok(attempt) => attempt,
                    Err(_) => break,
                };
                loop {
                    if owner
                        .state
                        .lock()
                        .unwrap()
                        .pending
                        .as_ref()
                        .is_none_or(|pending| pending.id != attempt)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        });
    }
    fn select(
        self: &Arc<Self>,
        kind: ShareKind,
        remember: bool,
        saved: Option<u64>,
    ) -> Result<u64, String> {
        let scope = current_share_scope();
        if scope.is_none() {
            return Err("Pair devices before sharing this computer".into());
        }
        if self.control.is_none() {
            return Err("Sharing service is unavailable".into());
        }
        if self
            .tasks
            .lock()
            .unwrap()
            .iter()
            .filter(|task| !task.is_finished())
            .count()
            >= 16
        {
            return Err("Sharing cleanup is still running".into());
        }
        let (id, rx) = {
            let mut state = self.state.lock().unwrap();
            state
                .sources
                .retain(|source| source.scope == scope && !source.capture.is_revoked());
            if self.closing.load(Ordering::Acquire) {
                return Err("Sharing service is stopping".into());
            }
            if state.pending.is_some() || state.stopping {
                return Err("Finish the current system selection first".into());
            }
            if state.sources.len() >= 8 {
                return Err("Up to eight sources can be shared at once".into());
            }
            let id = match saved {
                Some(id) => id,
                None => state
                    .serial
                    .checked_add(1)
                    .ok_or("Source identifier exhausted")?,
            };
            state.serial = state.serial.max(id);
            let (cancel, rx) = watch::channel(false);
            state.pending = Some(Pending { id, scope, cancel });
            state.message = None;
            (id, rx)
        };
        let owner = self.clone();
        self.spawn(async move {
            let selection=owner.select_owned(id,scope,kind,remember,saved,rx);
            tokio::pin!(selection);
            let result=loop {tokio::select! {biased;
                result=&mut selection=>break result,
                _=tokio::time::sleep(Duration::from_millis(250))=>{
                    if scope!=current_share_scope() || owner.closing.load(Ordering::Acquire) {
                        if let Some(pending)=&owner.state.lock().unwrap().pending {pending.cancel.send_replace(true);}
                    }
                }
            }};
            let mut state=owner.state.lock().unwrap();
            if state.pending.as_ref().is_some_and(|pending|pending.id==id) {
                state.pending=None;if !owner.closing.load(Ordering::Acquire) && let Err(message)=result {state.message=Some(message);}
            }
        });
        Ok(id)
    }
    async fn select_owned(
        self: &Arc<Self>,
        id: u64,
        scope: ShareScope,
        kind: ShareKind,
        remember: bool,
        saved: Option<u64>,
        cancel: watch::Receiver<bool>,
    ) -> Result<(), String> {
        let restoration = if let Some(saved) = saved {
            let record = self
                .store
                .lock()
                .unwrap()
                .as_mut()
                .ok_or("Saved sharing data is unavailable")?
                .take(scope, saved)
                .map_err(|_| "Saved selection could not be consumed; no restore was attempted")?
                .ok_or("Saved selection changed")?;
            if record.kind != kind {
                return Err("Saved selection changed".into());
            }
            Some(record.grant)
        } else {
            None
        };
        let (picker, control) = store::selection(kind);
        let mut lease = if remember {
            host::request_local_portal_capture_persistent("", picker, cancel, control, restoration)
                .await
        } else if control {
            host::request_local_portal_control_capture("", picker, cancel).await
        } else {
            host::request_local_portal_capture("", picker, cancel).await
        }
        .map_err(portal_message)?;
        let token = lease.take_persistent_restoration();
        let capture = match host::prepare_pipewire_capture(lease).await {
            Ok(capture) => capture,
            Err(error) => {
                // A failed frame negotiation does not undo the user's real OS
                // grant. Keep its rotated token for the next normal restoration.
                let _serial = self.publication.lock().await;
                let current = scope == current_share_scope()
                    && !self.closing.load(Ordering::Acquire)
                    && !matches!(
                        error,
                        host::PortalCallError::Cancelled | host::PortalCallError::Revoked
                    )
                    && self.state.lock().unwrap().pending.as_ref().is_some_and(|pending| {
                        pending.id == id && pending.scope == scope && !*pending.cancel.borrow()
                    });
                if current && let Some(grant) = token
                    && let Some(store) = self.store.lock().unwrap().as_mut()
                    && store.put(scope, store::Record { id, kind, grant }).is_ok()
                {
                    return Err(format!("{}; permission remembered", portal_message(error)));
                }
                return Err(portal_message(error));
            }
        };
        if control && let Err(reason) = capture.input_status() {
            self.state.lock().unwrap().message = Some(format!("Video only: {reason}"));
        }
        capture
            .set_label(format!(
                "{} {}",
                if kind == ShareKind::Window {
                    "Window"
                } else {
                    "Display"
                },
                id
            ))
            .map_err(portal_message)?;
        let _serial = self.publication.lock().await;
        let sender = {
            let state = self.state.lock().unwrap();
            state
                .pending
                .as_ref()
                .filter(|pending| {
                    pending.id == id && pending.scope == scope && !*pending.cancel.borrow()
                })
                .map(|pending| pending.cancel.clone())
        };
        let Some(sender) = sender
            .filter(|_| scope == current_share_scope() && !self.closing.load(Ordering::Acquire))
        else {
            let _ = capture.close_owned().await;
            return Err("Selection cancelled or device network changed".into());
        };
        if let Err(error) = self
            .activate(id, scope, kind, capture.clone(), sender, token)
            .await
        {
            capture.revoke();
            let _ = capture.close_owned().await;
            return Err(error);
        }
        Ok(())
    }
    async fn activate(
        &self,
        id: u64,
        scope: ShareScope,
        kind: ShareKind,
        capture: PreparedPortalCapture,
        cancel: watch::Sender<bool>,
        token: Option<PersistentPortalRestore>,
    ) -> Result<(), String> {
        {
            let mut state = self.state.lock().unwrap();
            if scope != current_share_scope()
                || *cancel.borrow()
                || self.closing.load(Ordering::Acquire)
            {
                return Err("Selection changed".into());
            }
            if kind != ShareKind::Window {
                state.display = Some(capture.source_info().source);
            }
            state.sources.push(Source {
                id,
                scope,
                kind,
                capture,
                _cancel: cancel.clone(),
                remembered: false,
            });
        }
        if let Err(error) = self.publish().await {
            self.state
                .lock()
                .unwrap()
                .sources
                .retain(|source| source.id != id);
            if let Some(store) = self.store.lock().unwrap().as_mut() {
                let _ = store.forget(scope, Some(id));
            }
            return Err(error);
        }
        if *cancel.borrow()
            || scope != current_share_scope()
            || self.closing.load(Ordering::Acquire)
        {
            self.state
                .lock()
                .unwrap()
                .sources
                .retain(|source| source.id != id);
            let _ = self.publish().await;
            return Err("Selection cancelled or device network changed".into());
        }
        let remembered = if let Some(grant) = token {
            let mut store = self.store.lock().unwrap();
            match store
                .as_mut()
                .ok_or_else(|| std::io::Error::other("store unavailable"))
                .and_then(|store| store.put(scope, store::Record { id, kind, grant }))
            {
                Ok(()) => true,
                Err(_) => {
                    self.state.lock().unwrap().message =
                        Some("Sharing started, but this selection could not be remembered".into());
                    false
                }
            }
        } else {
            false
        };
        if let Some(source) = self
            .state
            .lock()
            .unwrap()
            .sources
            .iter_mut()
            .find(|source| source.id == id)
        {
            source.remembered = remembered;
        }
        if *cancel.borrow() && !self.closing.load(Ordering::Acquire) {
            if let Some(store) = self.store.lock().unwrap().as_mut() {
                let _ = store.forget(scope, Some(id));
            }
            self.state
                .lock()
                .unwrap()
                .sources
                .retain(|source| source.id != id);
            let _ = self.publish().await;
            return Err("Selection cancelled".into());
        }
        Ok(())
    }
    async fn publish(&self) -> Result<(), String> {
        let scope = current_share_scope();
        let (captures, display) = {
            let state = self.state.lock().unwrap();
            let captures: Vec<_> = state
                .sources
                .iter()
                .filter(|source| source.scope == scope && !source.capture.is_revoked())
                .map(|source| source.capture.clone())
                .collect();
            let display = state.display.filter(|id| {
                captures
                    .iter()
                    .any(|capture| capture.source_info().source == *id)
            });
            (captures, display)
        };
        if scope.is_none() {
            return if captures.is_empty() {
                Ok(())
            } else {
                Err("Device network changed".into())
            };
        }
        let (reply, result) = oneshot::channel();
        let control = self
            .control
            .as_ref()
            .ok_or("Sharing service is unavailable")?;
        tokio::time::timeout(Duration::from_secs(2), async {
            control
                .send(LocalPortalShareCommand::Publish {
                    scope,
                    captures,
                    default_display: display,
                    reply,
                })
                .await
                .map_err(|_| "Sharing service stopped")?;
            result.await.map_err(|_| "Sharing service stopped")?
        })
        .await
        .map_err(|_| "Sharing service did not acknowledge the sources")?
    }
    async fn stop(&self, source: Option<CaptureSource>, forget: bool) {
        let _serial = self.publication.lock().await;
        let removed = {
            let mut state = self.state.lock().unwrap();
            let mut removed = Vec::new();
            let mut kept = Vec::new();
            for item in state.sources.drain(..) {
                if source.is_none_or(|source| item.capture.source_info().source == source) {
                    item.capture.revoke();
                    removed.push(item);
                } else {
                    kept.push(item);
                }
            }
            state.sources = kept;
            if source.is_none() {
                state.display = None;
            }
            removed
        };
        let mut errors = Vec::new();
        if forget {
            let mut store = self.store.lock().unwrap();
            if let Some(store) = store.as_mut() {
                if source.is_none() {
                    if store.forget_all().is_err() {
                        errors.push("Saved selections could not be forgotten");
                    }
                } else {
                    for item in &removed {
                        if store.forget(item.scope, Some(item.id)).is_err() {
                            errors.push("Saved selection could not be forgotten");
                        }
                    }
                }
            }
        }
        if !self.closing.load(Ordering::Acquire) && self.publish().await.is_err() {
            errors.push("Source list update could not be confirmed");
        }
        let mut closing = tokio::task::JoinSet::new();
        for item in removed {
            closing.spawn(async move { item.capture.close_owned().await });
        }
        while let Some(result) = closing.join_next().await {
            if !matches!(result, Ok(Ok(()))) {
                self.cleanup_failed.store(true, Ordering::Release);
                errors.push("System sharing cleanup could not be confirmed");
            }
        }
        let mut state = self.state.lock().unwrap();
        if source.is_none() {
            state.stopping = false;
        }
        state.message = if errors.is_empty() {
            None
        } else {
            Some(errors.join("; "))
        };
    }
    pub fn cancel_for_exit(&self) {
        self.closing.store(true, Ordering::Release);
        let state = self.state.lock().unwrap();
        if let Some(pending) = &state.pending {
            pending.cancel.send_replace(true);
        }
        for source in &state.sources {
            source.capture.revoke();
        }
    }
    pub async fn shutdown(&self) -> Result<(), String> {
        self.cancel_for_exit();
        self.stop(None, false).await;
        let mut jobs = std::mem::take(&mut *self.tasks.lock().unwrap());
        while let Some(mut job) = jobs.pop() {
            match tokio::time::timeout(Duration::from_secs(5), &mut job).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {
                    self.cleanup_failed.store(true, Ordering::Release);
                }
                Err(_) => {
                    let mut retained = self.tasks.lock().unwrap();
                    retained.push(job);
                    retained.extend(jobs);
                    return Err("Sharing selection cleanup is still pending".into());
                }
            }
        }
        if self.state.lock().unwrap().pending.is_some() {
            return Err("Sharing selection cleanup is still pending".into());
        }
        if self.cleanup_failed.load(Ordering::Acquire) {
            return Err("System sharing cleanup could not be confirmed".into());
        }
        match self.state.lock().unwrap().message.clone() {
            Some(message) => Err(message),
            None => Ok(()),
        }
    }
}
fn portal_message(error: host::PortalCallError) -> String {
    use host::PortalCallError::*;
    match error {
        Cancelled => "Selection cancelled",
        Revoked => "System sharing ended; choose a source again",
        Capabilities => "This system cannot share the requested source or control mode",
        Timeout => "System sharing did not become ready in time",
        InvalidSource => "The selected source has an unsupported format",
        _ => "The system sharing service is unavailable",
    }
    .into()
}
