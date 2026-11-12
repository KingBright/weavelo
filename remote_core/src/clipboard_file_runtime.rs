use crate::data_plane::checksum_crc32;
use crate::file_transfer_runtime::{FileTransferCommand, FileTransferEvent, FileTransferGroupFile};
use crate::traits::{ClipboardFileReference, ClipboardFileReferenceProvider};
use std::error::Error;
use std::fmt;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

// Native clipboard operations are shared by the text/image and file planes.
// A write admitted before revocation may finish, but later owners cannot begin
// a native operation until it finishes. Reads are discarded after revocation.
static CLIPBOARD_OWNER: std::sync::Mutex<Option<std::sync::Weak<std::sync::atomic::AtomicBool>>> =
    std::sync::Mutex::new(None);
static CLIPBOARD_OPERATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn claim_clipboard_owner(gate: &std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    let mut owner = CLIPBOARD_OWNER.lock().unwrap();
    if let Some(previous) = owner.take().and_then(|old| old.upgrade()) {
        previous.store(false, Ordering::Release);
    }
    gate.store(true, Ordering::Release);
    *owner = Some(std::sync::Arc::downgrade(gate));
}

#[derive(Debug, Clone)]
pub struct ClipboardOwnership(pub std::sync::Arc<std::sync::atomic::AtomicBool>);
impl PartialEq for ClipboardOwnership {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ClipboardOwnership {}
impl ClipboardOwnership {
    pub fn current(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}
#[derive(Debug, Clone)]
pub struct ClipboardFileAdmission {
    pub ownership: ClipboardOwnership,
    status: std::sync::Arc<std::sync::atomic::AtomicU8>,
    observation: Option<(std::sync::Arc<ClipboardObservation>, u64)>,
}
impl ClipboardFileAdmission {
    pub fn new(ownership: ClipboardOwnership) -> Self {
        Self {
            ownership,
            status: Default::default(),
            observation: None,
        }
    }
    fn with_observation(
        mut self,
        tracker: std::sync::Arc<ClipboardObservation>,
        epoch: u64,
    ) -> Self {
        self.observation = Some((tracker, epoch));
        self
    }
    pub fn accept(&self) -> bool {
        // Coordinate the admission point with observations from the other plane.
        let observed = self
            .observation
            .as_ref()
            .map(|(tracker, _)| tracker.0.lock().unwrap());
        if !self.ownership.current()
            || observed
                .as_ref()
                .zip(self.observation.as_ref())
                .is_some_and(|(current, (_, epoch))| current.1 != *epoch)
        {
            return false;
        }
        self.status
            .compare_exchange(
                0,
                1,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
    }
    pub fn reject(&self) {
        let _ = self.status.compare_exchange(
            0,
            2,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        );
    }
    fn status(&self) -> u8 {
        self.status.load(std::sync::atomic::Ordering::Acquire)
    }
}
impl PartialEq for ClipboardFileAdmission {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.status, &other.status)
    }
}
impl Eq for ClipboardFileAdmission {}
pub type ClipboardOwnerSlot = std::sync::Arc<std::sync::Mutex<Option<ClipboardOwnership>>>;

#[derive(Debug)]
pub struct ClipboardOwnershipRevoked;
impl fmt::Display for ClipboardOwnershipRevoked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "clipboard ownership revoked")
    }
}
impl Error for ClipboardOwnershipRevoked {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardContentKind {
    TextImage,
    Files,
}

/// Process-local observation shared by the two native clipboard planes.
#[derive(Debug, Default)]
pub struct ClipboardObservation(std::sync::Mutex<(Option<ClipboardContentKind>, u64)>);
impl ClipboardObservation {
    fn observe(&self, kind: ClipboardContentKind) -> u64 {
        let mut current = self.0.lock().unwrap();
        if current.0 != Some(kind) {
            current.0 = Some(kind);
            current.1 = current.1.saturating_add(1);
        }
        current.1
    }
    pub fn epoch(&self) -> u64 {
        self.0.lock().unwrap().1
    }
}
pub fn process_clipboard_observation() -> std::sync::Arc<ClipboardObservation> {
    static TRACKER: std::sync::OnceLock<std::sync::Arc<ClipboardObservation>> =
        std::sync::OnceLock::new();
    TRACKER.get_or_init(Default::default).clone()
}

fn observable_text_image(bundle: &protocol::ClipboardBundle) -> bool {
    bundle.items.iter().any(|item| match item {
        protocol::ClipboardItem::Text(text) => !text.text.is_empty(),
        protocol::ClipboardItem::Image(image) => !image.bytes.is_empty(),
        protocol::ClipboardItem::File(_) => false,
    })
}

pub struct OwnedClipboardProvider<P> {
    provider: P,
    gate: std::sync::Arc<std::sync::atomic::AtomicBool>,
    observation: Option<std::sync::Arc<ClipboardObservation>>,
    observed_epoch: Option<u64>,
}
impl<P> OwnedClipboardProvider<P> {
    pub fn new(provider: P, gate: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self {
            provider,
            gate,
            observation: None,
            observed_epoch: None,
        }
    }
    pub fn with_observation(mut self, observation: std::sync::Arc<ClipboardObservation>) -> Self {
        self.observation = Some(observation);
        self
    }
    fn observe(&mut self, kind: ClipboardContentKind) {
        self.observed_epoch = self
            .observation
            .as_ref()
            .map(|tracker| tracker.observe(kind));
    }
    fn current(&self) -> bool {
        self.gate.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[async_trait::async_trait]
impl<P: crate::traits::ClipboardProvider + Send> crate::traits::ClipboardProvider
    for OwnedClipboardProvider<P>
{
    fn platform(&self) -> crate::traits::PlatformKind {
        self.provider.platform()
    }
    fn capabilities(&self) -> crate::traits::ClipboardBackendCapabilities {
        self.provider.capabilities()
    }
    fn clipboard_revision(&self) -> Option<u64> {
        self.provider.clipboard_revision()
    }
    fn clipboard_observation_epoch(&self) -> Option<u64> {
        self.observed_epoch
    }
    async fn read_clipboard(
        &mut self,
        policy: crate::clipboard_plane::ClipboardSyncPolicy,
    ) -> Result<Option<protocol::ClipboardBundle>, Box<dyn Error + Send + Sync>> {
        let _operation = CLIPBOARD_OPERATION.lock().await;
        if !self.current() {
            return Err(Box::new(ClipboardOwnershipRevoked));
        }
        let result = self.provider.read_clipboard(policy).await?;
        if !self.current() {
            return Err(Box::new(ClipboardOwnershipRevoked));
        }
        if result.as_ref().is_some_and(observable_text_image) {
            self.observe(ClipboardContentKind::TextImage);
            Ok(result)
        } else {
            self.observed_epoch = None;
            Ok(None)
        }
    }
    async fn write_clipboard(
        &mut self,
        bundle: &protocol::ClipboardBundle,
        policy: crate::clipboard_plane::ClipboardSyncPolicy,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let _operation = CLIPBOARD_OPERATION.lock().await;
        if !self.current() {
            return Err(Box::new(ClipboardOwnershipRevoked));
        }
        self.provider.write_clipboard(bundle, policy).await?;
        if observable_text_image(bundle) {
            self.observe(ClipboardContentKind::TextImage);
        } else {
            self.observed_epoch = None;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl<P: ClipboardFileReferenceProvider + Send> ClipboardFileReferenceProvider
    for OwnedClipboardProvider<P>
{
    fn clipboard_observation_epoch(&self) -> Option<u64> {
        self.observed_epoch
    }
    fn platform(&self) -> crate::traits::PlatformKind {
        self.provider.platform()
    }
    async fn read_clipboard_file_references(
        &mut self,
    ) -> Result<Vec<ClipboardFileReference>, Box<dyn Error + Send + Sync>> {
        let _operation = CLIPBOARD_OPERATION.lock().await;
        if !self.current() {
            return Ok(Vec::new());
        }
        let result = self.provider.read_clipboard_file_references().await?;
        if !self.current() {
            return Ok(Vec::new());
        }
        if !result.is_empty() {
            self.observe(ClipboardContentKind::Files);
        }
        Ok(result)
    }
    async fn write_clipboard_file_references(
        &mut self,
        references: &[ClipboardFileReference],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let _operation = CLIPBOARD_OPERATION.lock().await;
        if !self.current() {
            return Err(Box::new(ClipboardOwnershipRevoked));
        }
        self.provider
            .write_clipboard_file_references(references)
            .await?;
        if !references.is_empty() {
            self.observe(ClipboardContentKind::Files);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ClipboardFileSyncConfig {
    pub enabled: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub poll_interval: Duration,
    pub max_files_per_poll: usize,
    pub state: Option<std::sync::Arc<std::sync::Mutex<ClipboardFileSyncState>>>,
    pub observation: Option<std::sync::Arc<ClipboardObservation>>,
}

#[derive(Debug, Default)]
pub struct ClipboardFileSyncState {
    observed_epoch: u64,
    last_sent_refs_crc32: Option<u32>,
    pending: Option<(u32, ClipboardFileAdmission)>,
    last_applied_refs_crc32: Option<u32>,
}

impl Default for ClipboardFileSyncConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            poll_interval: Duration::from_millis(500),
            max_files_per_poll: 16,
            state: None,
            observation: None,
        }
    }
}

#[derive(Debug)]
pub enum ClipboardFileSyncError {
    Provider(Box<dyn Error + Send + Sync>),
    CommandClosed,
}

impl fmt::Display for ClipboardFileSyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClipboardFileSyncError::Provider(err) => {
                write!(f, "clipboard file reference provider failed: {err}")
            }
            ClipboardFileSyncError::CommandClosed => {
                write!(f, "file transfer command channel is closed")
            }
        }
    }
}

impl Error for ClipboardFileSyncError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ClipboardFileSyncError::Provider(err) => Some(err.as_ref()),
            ClipboardFileSyncError::CommandClosed => None,
        }
    }
}

pub async fn run_clipboard_file_sync<P>(
    mut provider: P,
    file_command_tx: mpsc::Sender<FileTransferCommand>,
    mut file_event_rx: mpsc::UnboundedReceiver<FileTransferEvent>,
    forward_event_tx: Option<mpsc::UnboundedSender<FileTransferEvent>>,
    mut cancel_rx: broadcast::Receiver<()>,
    config: ClipboardFileSyncConfig,
) -> Result<(), ClipboardFileSyncError>
where
    P: ClipboardFileReferenceProvider + Send + 'static,
{
    let mut poll_interval = tokio::time::interval(config.poll_interval);
    let state = config.state.clone().unwrap_or_default();

    poll_interval.tick().await;

    loop {
        tokio::select! {
            _ = cancel_rx.recv() => {
                return Ok(());
            }
            _ = poll_interval.tick() => {
                if config.enabled.as_ref().is_some_and(|enabled| !enabled.load(std::sync::atomic::Ordering::Relaxed)) { continue; }
                let references = provider
                    .read_clipboard_file_references()
                    .await
                    .map_err(ClipboardFileSyncError::Provider)?;
                if config.enabled.as_ref().is_some_and(|enabled| !enabled.load(std::sync::atomic::Ordering::Acquire)) { continue; }
                let references = bounded_references(references, config.max_files_per_poll);
                if references.is_empty() {
                    let mut state = state.lock().unwrap();
                    if config.enabled.as_ref().is_some_and(|gate| !gate.load(std::sync::atomic::Ordering::Acquire)) { continue; }
                    if let Some((_, admission)) = state.pending.take() { admission.reject(); }
                    state.last_sent_refs_crc32 = None;
                    state.last_applied_refs_crc32 = None;
                    continue;
                }

                let refs_crc32 = file_references_crc32(&references);
                let observed = if let Some(tracker) = &config.observation {
                    let Some(epoch) = provider.clipboard_observation_epoch() else { continue };
                    if tracker.epoch() != epoch { continue; }
                    Some(epoch)
                } else { None };
                let duplicate = { let mut state = state.lock().unwrap();
                    if let Some(epoch) = observed {
                        if state.observed_epoch != epoch {
                            if let Some((_, admission)) = state.pending.take() { admission.reject(); }
                            state.last_sent_refs_crc32 = None;
                            state.last_applied_refs_crc32 = None;
                            state.observed_epoch = epoch;
                        }
                    }
                    if let Some((crc, admission)) = state.pending.take() {
                        if admission.status() == 1 { state.last_sent_refs_crc32 = Some(crc); state.last_applied_refs_crc32 = None; }
                        else if admission.status() == 0 && admission.ownership.current() { state.pending = Some((crc, admission)); }
                    }
                    Some(refs_crc32) == state.last_sent_refs_crc32 || Some(refs_crc32) == state.last_applied_refs_crc32
                        || state.pending.as_ref().is_some_and(|(crc, _)| *crc == refs_crc32) };
                if duplicate {
                    continue;
                }

                let files = references
                    .iter()
                    .map(|reference| FileTransferGroupFile {
                        path: reference.path.clone(),
                        mime_type: None,
                    })
                    .collect();
                let permit = tokio::select! {
                    _ = cancel_rx.recv() => return Ok(()),
                    permit = file_command_tx.reserve() => permit.map_err(|_| ClipboardFileSyncError::CommandClosed)?,
                };
                if config.enabled.as_ref().is_some_and(|gate| !gate.load(std::sync::atomic::Ordering::Acquire)) { continue; }
                if observed.is_some_and(|epoch| config.observation.as_ref().is_some_and(|tracker| tracker.epoch() != epoch)) { continue; }
                let mut state = state.lock().unwrap();
                let command = match &config.enabled {
                    Some(gate) => {
                        let mut admission = ClipboardFileAdmission::new(ClipboardOwnership(gate.clone()));
                        if let (Some(tracker), Some(epoch)) = (&config.observation, observed) { admission = admission.with_observation(tracker.clone(), epoch); }
                        state.pending = Some((refs_crc32, admission.clone()));
                        FileTransferCommand::SendOwnedClipboardFiles { files, admission }
                    }
                    None => { state.last_sent_refs_crc32 = Some(refs_crc32); state.last_applied_refs_crc32 = None; FileTransferCommand::SendClipboardFiles { files } }
                };
                permit.send(command);
            }
            maybe_event = file_event_rx.recv() => {
                let Some(event) = maybe_event else {
                    return Ok(());
                };

                if let Some(tx) = &forward_event_tx {
                    let _ = tx.send(event.clone());
                }

                if config.enabled.as_ref().is_some_and(|enabled| !enabled.load(std::sync::atomic::Ordering::Relaxed)) { continue; }

                if let FileTransferEvent::IncomingClipboardReady { paths, ownership, .. } = event {
                        if let Some(ownership) = ownership {
                            if !ownership.current() || config.enabled.as_ref().is_none_or(|gate| !std::sync::Arc::ptr_eq(gate, &ownership.0)) { continue; }
                        }
                        let references = paths
                            .into_iter()
                            .map(ClipboardFileReference::new)
                            .collect::<Vec<_>>();
                        // Revoke queued older sends before beginning the native write.
                        // An already admitted transfer may finish independently.
                        {
                            let mut state = state.lock().unwrap();
                            if let Some((crc, admission)) = state.pending.take() {
                                admission.reject();
                                if admission.status() == 1 {
                                    state.last_sent_refs_crc32 = Some(crc);
                                    state.last_applied_refs_crc32 = None;
                                }
                            }
                        }
                        if let Err(error) = provider.write_clipboard_file_references(&references).await {
                            if error.is::<ClipboardOwnershipRevoked>() { continue; }
                            return Err(ClipboardFileSyncError::Provider(error));
                        }
                        let observed = if let Some(tracker) = &config.observation {
                            let Some(epoch) = provider.clipboard_observation_epoch() else { continue };
                            if tracker.epoch() != epoch { continue; }
                            Some(epoch)
                        } else { None };
                        let mut state = state.lock().unwrap();
                        if let Some(epoch) = observed { state.observed_epoch = epoch; }
                        state.last_sent_refs_crc32 = None;
                        state.last_applied_refs_crc32 = Some(file_references_crc32(&references));
                }
            }
        }
    }
}

fn bounded_references(
    mut references: Vec<ClipboardFileReference>,
    max_files_per_poll: usize,
) -> Vec<ClipboardFileReference> {
    if max_files_per_poll == 0 {
        references.clear();
    } else {
        references.truncate(max_files_per_poll);
    }
    references
}

fn file_references_crc32(references: &[ClipboardFileReference]) -> u32 {
    let mut bytes = Vec::new();
    for reference in references {
        bytes.extend_from_slice(reference.path.to_string_lossy().as_bytes());
        bytes.push(0);
    }
    checksum_crc32(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_provider::MemoryClipboardFileReferenceProvider;
    use std::path::PathBuf;
    use tokio::time::timeout;

    struct DelayedFiles {
        shared: std::sync::Arc<std::sync::Mutex<Vec<ClipboardFileReference>>>,
        entered: std::sync::Arc<tokio::sync::Notify>,
        release: std::sync::Arc<tokio::sync::Semaphore>,
        delay_read: bool,
        delay_write: bool,
    }
    #[async_trait::async_trait]
    impl ClipboardFileReferenceProvider for DelayedFiles {
        async fn read_clipboard_file_references(
            &mut self,
        ) -> Result<Vec<ClipboardFileReference>, Box<dyn Error + Send + Sync>> {
            if self.delay_read {
                self.entered.notify_one();
                self.release.acquire().await.unwrap().forget();
            }
            Ok(self.shared.lock().unwrap().clone())
        }
        async fn write_clipboard_file_references(
            &mut self,
            refs: &[ClipboardFileReference],
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            if self.delay_write {
                self.entered.notify_one();
                self.release.acquire().await.unwrap().forget();
            }
            *self.shared.lock().unwrap() = refs.to_vec();
            Ok(())
        }
    }

    #[tokio::test]
    async fn ownership_discards_stale_reads_and_serializes_admitted_writes() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let shared = Arc::new(std::sync::Mutex::new(vec![ClipboardFileReference::new(
            "/from-a",
        )]));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let first = Arc::new(AtomicBool::new(false));
        claim_clipboard_owner(&first);
        let slow = DelayedFiles {
            shared: shared.clone(),
            entered: entered.clone(),
            release: release.clone(),
            delay_read: true,
            delay_write: false,
        };
        let mut old = OwnedClipboardProvider::new(slow, first.clone());
        let read = tokio::spawn(async move { old.read_clipboard_file_references().await.unwrap() });
        entered.notified().await;
        let second = Arc::new(AtomicBool::new(false));
        claim_clipboard_owner(&second);
        assert!(!first.load(Ordering::Acquire));
        release.add_permits(1);
        assert!(
            read.await.unwrap().is_empty(),
            "a read completing after focus loss must be discarded"
        );
        let mut current = OwnedClipboardProvider::new(
            DelayedFiles {
                shared: shared.clone(),
                entered: entered.clone(),
                release: release.clone(),
                delay_read: false,
                delay_write: true,
            },
            second.clone(),
        );
        let write = tokio::spawn(async move {
            current
                .write_clipboard_file_references(&[ClipboardFileReference::new("/received-from-b")])
                .await
                .unwrap();
        });
        entered.notified().await;
        let third = Arc::new(AtomicBool::new(false));
        claim_clipboard_owner(&third);
        let mut next = OwnedClipboardProvider::new(
            DelayedFiles {
                shared: shared.clone(),
                entered: entered.clone(),
                release: release.clone(),
                delay_read: false,
                delay_write: false,
            },
            third.clone(),
        );
        let read_next =
            tokio::spawn(async move { next.read_clipboard_file_references().await.unwrap() });
        tokio::task::yield_now().await;
        assert!(
            !read_next.is_finished(),
            "new native operations wait for an admitted old write"
        );
        release.add_permits(1);
        write.await.unwrap();
        assert_eq!(
            read_next.await.unwrap(),
            vec![ClipboardFileReference::new("/received-from-b")],
            "the new session can copy the previous session's local clipboard"
        );
        let mut stale = OwnedClipboardProvider::new(
            DelayedFiles {
                shared: shared.clone(),
                entered: entered.clone(),
                release: release.clone(),
                delay_read: false,
                delay_write: false,
            },
            second,
        );
        assert!(
            stale
                .write_clipboard_file_references(&[ClipboardFileReference::new("/stale")])
                .await
                .unwrap_err()
                .is::<ClipboardOwnershipRevoked>()
        );
        assert_eq!(
            *shared.lock().unwrap(),
            vec![ClipboardFileReference::new("/received-from-b")]
        );
        let (commands, mut sent) = mpsc::channel(4);
        let (events, incoming) = mpsc::unbounded_channel();
        let (cancel, cancel_rx) = broadcast::channel(1);
        let saved = std::sync::Arc::new(std::sync::Mutex::new(ClipboardFileSyncState::default()));
        let bridge = tokio::spawn(run_clipboard_file_sync(
            OwnedClipboardProvider::new(
                DelayedFiles {
                    shared: shared.clone(),
                    entered: entered.clone(),
                    release: release.clone(),
                    delay_read: false,
                    delay_write: false,
                },
                third.clone(),
            ),
            commands,
            incoming,
            None,
            cancel_rx,
            ClipboardFileSyncConfig {
                enabled: Some(third.clone()),
                state: Some(saved.clone()),
                poll_interval: Duration::from_millis(10),
                ..Default::default()
            },
        ));
        events
            .send(FileTransferEvent::IncomingClipboardReady {
                group_id: 10,
                ownership: None,
                paths: vec![PathBuf::from("/incoming")],
            })
            .unwrap();
        timeout(Duration::from_secs(1), async {
            while *shared.lock().unwrap() != vec![ClipboardFileReference::new("/incoming")] {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            timeout(Duration::from_millis(40), sent.recv())
                .await
                .is_err(),
            "the applied file must not echo to its source"
        );
        let fourth = Arc::new(AtomicBool::new(false));
        claim_clipboard_owner(&fourth);
        events
            .send(FileTransferEvent::IncomingClipboardReady {
                group_id: 11,
                ownership: None,
                paths: vec![PathBuf::from("/late")],
            })
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            *shared.lock().unwrap(),
            vec![ClipboardFileReference::new("/incoming")],
            "late completion from the old owner must not replace the clipboard"
        );
        let _ = cancel.send(());
        bridge.await.unwrap().unwrap();
        let (commands, mut sent) = mpsc::channel(4);
        let (_events, incoming) = mpsc::unbounded_channel();
        let (cancel, cancel_rx) = broadcast::channel(1);
        let bridge = tokio::spawn(run_clipboard_file_sync(
            OwnedClipboardProvider::new(
                DelayedFiles {
                    shared: shared.clone(),
                    entered: entered.clone(),
                    release: release.clone(),
                    delay_read: false,
                    delay_write: false,
                },
                fourth.clone(),
            ),
            commands,
            incoming,
            None,
            cancel_rx,
            ClipboardFileSyncConfig {
                enabled: Some(fourth.clone()),
                state: Some(saved),
                poll_interval: Duration::from_millis(10),
                ..Default::default()
            },
        ));
        assert!(
            timeout(Duration::from_millis(40), sent.recv())
                .await
                .is_err(),
            "returning to the source must not replay its own received file"
        );
        let _ = cancel.send(());
        bridge.await.unwrap().unwrap();
        let fifth = Arc::new(AtomicBool::new(false));
        claim_clipboard_owner(&fifth);
        let (commands, mut sent) = mpsc::channel(4);
        let (_events, incoming) = mpsc::unbounded_channel();
        let (cancel, cancel_rx) = broadcast::channel(1);
        let bridge = tokio::spawn(run_clipboard_file_sync(
            OwnedClipboardProvider::new(
                DelayedFiles {
                    shared,
                    entered,
                    release,
                    delay_read: false,
                    delay_write: false,
                },
                fifth.clone(),
            ),
            commands,
            incoming,
            None,
            cancel_rx,
            ClipboardFileSyncConfig {
                enabled: Some(fifth),
                poll_interval: Duration::from_millis(10),
                ..Default::default()
            },
        ));
        assert!(
            matches!(
                timeout(Duration::from_secs(1), sent.recv()).await.unwrap(),
                Some(FileTransferCommand::SendOwnedClipboardFiles { .. })
            ),
            "a different selected host must receive the copied file"
        );
        let _ = cancel.send(());
        bridge.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn revoked_queued_send_and_old_generation_completion_are_ignored() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let old = Arc::new(AtomicBool::new(true));
        let shared = Arc::new(std::sync::Mutex::new(vec![ClipboardFileReference::new(
            "/old",
        )]));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let (commands, mut receiver) = mpsc::channel(1);
        commands.send(FileTransferCommand::Drain).await.unwrap();
        let (_events, incoming) = mpsc::unbounded_channel();
        let (cancel, cancel_rx) = broadcast::channel(1);
        let bridge = tokio::spawn(run_clipboard_file_sync(
            OwnedClipboardProvider::new(
                DelayedFiles {
                    shared: shared.clone(),
                    entered: entered.clone(),
                    release: release.clone(),
                    delay_read: true,
                    delay_write: false,
                },
                old.clone(),
            ),
            commands,
            incoming,
            None,
            cancel_rx,
            ClipboardFileSyncConfig {
                enabled: Some(old.clone()),
                poll_interval: Duration::from_millis(1),
                ..Default::default()
            },
        ));
        entered.notified().await;
        release.add_permits(1);
        // The full channel keeps the send pending until ownership is revoked.
        tokio::task::yield_now().await;
        old.store(false, Ordering::Release);
        assert!(matches!(
            receiver.recv().await,
            Some(FileTransferCommand::Drain)
        ));
        tokio::task::yield_now().await;
        assert!(receiver.try_recv().is_err());
        cancel.send(()).unwrap();
        bridge.await.unwrap().unwrap();

        // An A-to-B-to-A transition creates a new gate, so an old A event
        // cannot be applied by the new A bridge.
        let current = Arc::new(AtomicBool::new(true));
        let (commands, _receiver) = mpsc::channel(1);
        let (events, incoming) = mpsc::unbounded_channel();
        let (forward, mut forwarded) = mpsc::unbounded_channel();
        let (cancel, cancel_rx) = broadcast::channel(1);
        let bridge = tokio::spawn(run_clipboard_file_sync(
            OwnedClipboardProvider::new(
                DelayedFiles {
                    shared: shared.clone(),
                    entered,
                    release,
                    delay_read: false,
                    delay_write: false,
                },
                current.clone(),
            ),
            commands,
            incoming,
            Some(forward),
            cancel_rx,
            ClipboardFileSyncConfig {
                enabled: Some(current),
                poll_interval: Duration::from_secs(60),
                ..Default::default()
            },
        ));
        events
            .send(FileTransferEvent::IncomingClipboardReady {
                group_id: 1,
                paths: vec![PathBuf::from("/stale")],
                ownership: Some(ClipboardOwnership(old)),
            })
            .unwrap();
        // FIFO barrier: the second forwarded event follows processing the first.
        events.send(FileTransferEvent::Drained).unwrap();
        forwarded.recv().await.unwrap();
        forwarded.recv().await.unwrap();
        assert_eq!(
            *shared.lock().unwrap(),
            vec![ClipboardFileReference::new("/old")]
        );
        cancel.send(()).unwrap();
        bridge.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn dropped_queued_send_is_retried_by_next_owner() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let state = Arc::new(std::sync::Mutex::new(ClipboardFileSyncState::default()));
        let old = Arc::new(AtomicBool::new(true));
        let (commands, mut receiver) = mpsc::channel(1);
        let (_events, incoming) = mpsc::unbounded_channel();
        let (cancel, cancel_rx) = broadcast::channel(1);
        let bridge = tokio::spawn(run_clipboard_file_sync(
            MemoryClipboardFileReferenceProvider::with_references(vec![
                ClipboardFileReference::new("/copy"),
            ]),
            commands,
            incoming,
            None,
            cancel_rx,
            ClipboardFileSyncConfig {
                enabled: Some(old.clone()),
                state: Some(state.clone()),
                poll_interval: Duration::from_millis(1),
                ..Default::default()
            },
        ));
        let command = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        old.store(false, Ordering::Release);
        assert!(
            matches!(command, FileTransferCommand::SendOwnedClipboardFiles { admission, .. } if !admission.ownership.current())
        );
        cancel.send(()).unwrap();
        bridge.await.unwrap().unwrap();
        let (commands, mut receiver) = mpsc::channel(1);
        let (_events, incoming) = mpsc::unbounded_channel();
        let (cancel, cancel_rx) = broadcast::channel(1);
        let bridge = tokio::spawn(run_clipboard_file_sync(
            MemoryClipboardFileReferenceProvider::with_references(vec![
                ClipboardFileReference::new("/copy"),
            ]),
            commands,
            incoming,
            None,
            cancel_rx,
            ClipboardFileSyncConfig {
                enabled: Some(Arc::new(AtomicBool::new(true))),
                state: Some(state),
                poll_interval: Duration::from_millis(1),
                ..Default::default()
            },
        ));
        assert!(matches!(
            timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap(),
            Some(FileTransferCommand::SendOwnedClipboardFiles { .. })
        ));
        cancel.send(()).unwrap();
        bridge.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn file_content_can_return_after_opposite_direction_content() {
        use std::sync::{Arc, atomic::AtomicBool};
        let shared = Arc::new(std::sync::Mutex::new(vec![ClipboardFileReference::new(
            "/X",
        )]));
        let state = Arc::new(std::sync::Mutex::new(ClipboardFileSyncState::default()));
        let gate = Arc::new(AtomicBool::new(true));
        let (commands, mut receiver) = mpsc::channel(4);
        let (events, incoming) = mpsc::unbounded_channel();
        let (cancel, cancel_rx) = broadcast::channel(1);
        let bridge = tokio::spawn(run_clipboard_file_sync(
            DelayedFiles {
                shared: shared.clone(),
                entered: Arc::new(tokio::sync::Notify::new()),
                release: Arc::new(tokio::sync::Semaphore::new(0)),
                delay_read: false,
                delay_write: false,
            },
            commands,
            incoming,
            None,
            cancel_rx,
            ClipboardFileSyncConfig {
                enabled: Some(gate.clone()),
                state: Some(state.clone()),
                poll_interval: Duration::from_millis(2),
                ..Default::default()
            },
        ));
        let next_admission = |command| match command {
            FileTransferCommand::SendOwnedClipboardFiles { admission, .. } => admission,
            _ => panic!("expected clipboard send"),
        };
        assert!(
            next_admission(
                timeout(Duration::from_secs(1), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap()
            )
            .accept()
        );
        timeout(Duration::from_secs(1), async {
            while state.lock().unwrap().last_sent_refs_crc32.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        events
            .send(FileTransferEvent::IncomingClipboardReady {
                group_id: 1,
                paths: vec![PathBuf::from("/Y")],
                ownership: Some(ClipboardOwnership(gate.clone())),
            })
            .unwrap();
        let y_crc = file_references_crc32(&[ClipboardFileReference::new("/Y")]);
        timeout(Duration::from_secs(1), async {
            while state.lock().unwrap().last_applied_refs_crc32 != Some(y_crc) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        *shared.lock().unwrap() = vec![ClipboardFileReference::new("/X")];
        assert!(
            next_admission(
                timeout(Duration::from_secs(1), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap()
            )
            .accept(),
            "sent X must be reusable after received Y"
        );
        timeout(Duration::from_secs(1), async {
            while state.lock().unwrap().last_sent_refs_crc32.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        events
            .send(FileTransferEvent::IncomingClipboardReady {
                group_id: 2,
                paths: vec![PathBuf::from("/Y")],
                ownership: Some(ClipboardOwnership(gate)),
            })
            .unwrap();
        timeout(Duration::from_secs(1), async {
            while *shared.lock().unwrap() != vec![ClipboardFileReference::new("/Y")] {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            timeout(Duration::from_millis(25), receiver.recv())
                .await
                .is_err(),
            "received Y must not echo"
        );
        cancel.send(()).unwrap();
        bridge.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn incoming_file_cancels_a_queued_older_admission() {
        use std::sync::{Arc, atomic::AtomicBool};
        let shared = Arc::new(std::sync::Mutex::new(vec![ClipboardFileReference::new(
            "/old",
        )]));
        let state = Arc::new(std::sync::Mutex::new(ClipboardFileSyncState::default()));
        let gate = Arc::new(AtomicBool::new(true));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let (commands, mut receiver) = mpsc::channel(4);
        let (events, incoming) = mpsc::unbounded_channel();
        let (forward, mut forwarded) = mpsc::unbounded_channel();
        let (cancel, cancel_rx) = broadcast::channel(1);
        let bridge = tokio::spawn(run_clipboard_file_sync(
            DelayedFiles {
                shared: shared.clone(),
                entered: entered.clone(),
                release: release.clone(),
                delay_read: false,
                delay_write: true,
            },
            commands,
            incoming,
            Some(forward),
            cancel_rx,
            ClipboardFileSyncConfig {
                enabled: Some(gate.clone()),
                state: Some(state.clone()),
                poll_interval: Duration::from_millis(2),
                ..Default::default()
            },
        ));
        let FileTransferCommand::SendOwnedClipboardFiles { admission, .. } =
            timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap()
                .unwrap()
        else {
            panic!()
        };
        events
            .send(FileTransferEvent::IncomingClipboardReady {
                group_id: 1,
                paths: vec![PathBuf::from("/new")],
                ownership: Some(ClipboardOwnership(gate)),
            })
            .unwrap();
        entered.notified().await;
        assert!(
            !admission.accept(),
            "an older send must not be admitted while the newer clipboard write is in progress"
        );
        release.add_permits(1);
        events.send(FileTransferEvent::Drained).unwrap();
        forwarded.recv().await.unwrap();
        forwarded.recv().await.unwrap();
        assert_eq!(
            *shared.lock().unwrap(),
            vec![ClipboardFileReference::new("/new")]
        );
        assert_eq!(state.lock().unwrap().last_sent_refs_crc32, None);
        assert!(
            timeout(Duration::from_millis(25), receiver.recv())
                .await
                .is_err()
        );
        cancel.send(()).unwrap();
        bridge.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn file_copy_after_non_file_clipboard_is_resent_and_old_pending_is_cancelled() {
        use std::sync::{Arc, atomic::AtomicBool};
        let shared = Arc::new(std::sync::Mutex::new(vec![ClipboardFileReference::new(
            "/X",
        )]));
        let state = Arc::new(std::sync::Mutex::new(ClipboardFileSyncState::default()));
        let gate = Arc::new(AtomicBool::new(true));
        let (commands, mut receiver) = mpsc::channel(4);
        let (_events, incoming) = mpsc::unbounded_channel();
        let (cancel, cancel_rx) = broadcast::channel(1);
        let bridge = tokio::spawn(run_clipboard_file_sync(
            DelayedFiles {
                shared: shared.clone(),
                entered: Arc::new(tokio::sync::Notify::new()),
                release: Arc::new(tokio::sync::Semaphore::new(0)),
                delay_read: false,
                delay_write: false,
            },
            commands,
            incoming,
            None,
            cancel_rx,
            ClipboardFileSyncConfig {
                enabled: Some(gate),
                state: Some(state.clone()),
                poll_interval: Duration::from_millis(2),
                ..Default::default()
            },
        ));
        let receive_admission = |command| match command {
            FileTransferCommand::SendOwnedClipboardFiles { admission, .. } => admission,
            _ => panic!("expected clipboard send"),
        };
        assert!(
            receive_admission(
                timeout(Duration::from_secs(1), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap()
            )
            .accept()
        );
        timeout(Duration::from_secs(1), async {
            while state.lock().unwrap().last_sent_refs_crc32.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        shared.lock().unwrap().clear(); // Text or image clipboard: no file references.
        timeout(Duration::from_secs(1), async {
            while state.lock().unwrap().last_sent_refs_crc32.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        *shared.lock().unwrap() = vec![ClipboardFileReference::new("/X")];
        let stale = receive_admission(
            timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap()
                .unwrap(),
        );
        shared.lock().unwrap().clear();
        timeout(Duration::from_secs(1), async {
            while state.lock().unwrap().pending.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            !stale.accept(),
            "a pending old file copy must not survive the non-file clipboard"
        );
        *shared.lock().unwrap() = vec![ClipboardFileReference::new("/X")];
        assert!(matches!(
            timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap(),
            Some(FileTransferCommand::SendOwnedClipboardFiles { .. })
        ));
        cancel.send(()).unwrap();
        bridge.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn interleaved_planes_invalidate_old_content_even_when_a_poll_misses_the_switch() {
        use crate::clipboard_sync::{ClipboardSyncConfig, ClipboardSyncEndpoint};
        use crate::traits::ClipboardProvider;
        use protocol::ClipboardBundle;
        use std::sync::{Arc, atomic::AtomicBool};
        #[derive(Clone)]
        enum Native {
            Text(&'static str),
            File(&'static str),
        }
        #[derive(Clone)]
        struct Mixed(
            Arc<std::sync::Mutex<Native>>,
            Arc<std::sync::atomic::AtomicUsize>,
        );
        #[async_trait::async_trait]
        impl ClipboardProvider for Mixed {
            fn capabilities(&self) -> crate::traits::ClipboardBackendCapabilities {
                Default::default()
            }
            async fn read_clipboard(
                &mut self,
                _: crate::clipboard_plane::ClipboardSyncPolicy,
            ) -> Result<Option<ClipboardBundle>, Box<dyn Error + Send + Sync>> {
                Ok(match &*self.0.lock().unwrap() {
                    Native::Text(text) => Some(ClipboardBundle::text(1, *text)),
                    Native::File(_) => None,
                })
            }
            async fn write_clipboard(
                &mut self,
                _: &ClipboardBundle,
                _: crate::clipboard_plane::ClipboardSyncPolicy,
            ) -> Result<(), Box<dyn Error + Send + Sync>> {
                Ok(())
            }
        }
        #[async_trait::async_trait]
        impl ClipboardFileReferenceProvider for Mixed {
            async fn read_clipboard_file_references(
                &mut self,
            ) -> Result<Vec<ClipboardFileReference>, Box<dyn Error + Send + Sync>> {
                self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(match &*self.0.lock().unwrap() {
                    Native::File(path) => vec![ClipboardFileReference::new(*path)],
                    Native::Text(_) => vec![],
                })
            }
            async fn write_clipboard_file_references(
                &mut self,
                _: &[ClipboardFileReference],
            ) -> Result<(), Box<dyn Error + Send + Sync>> {
                Ok(())
            }
        }
        for start_with_file in [true, false] {
            let shared = Arc::new(std::sync::Mutex::new(if start_with_file {
                Native::File("/X")
            } else {
                Native::Text("X")
            }));
            let file_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let observation = Arc::new(ClipboardObservation::default());
            let gate = Arc::new(AtomicBool::new(true));
            let mut text = ClipboardSyncEndpoint::new(
                OwnedClipboardProvider::new(
                    Mixed(shared.clone(), file_reads.clone()),
                    gate.clone(),
                )
                .with_observation(observation.clone()),
                ClipboardSyncConfig::default(),
            );
            text.set_observation(Some(observation.clone()));
            if !start_with_file {
                assert!(text.poll_outgoing(1).await.unwrap().is_some());
                *shared.lock().unwrap() = Native::File("/Y");
            }
            let (commands, mut receiver) = mpsc::channel(4);
            let (_events, incoming) = mpsc::unbounded_channel();
            let (cancel, cancel_rx) = broadcast::channel(1);
            let bridge = tokio::spawn(run_clipboard_file_sync(
                OwnedClipboardProvider::new(
                    Mixed(shared.clone(), file_reads.clone()),
                    gate.clone(),
                )
                .with_observation(observation.clone()),
                commands,
                incoming,
                None,
                cancel_rx,
                ClipboardFileSyncConfig {
                    enabled: Some(gate),
                    observation: Some(observation.clone()),
                    poll_interval: Duration::from_millis(500),
                    ..Default::default()
                },
            ));
            let FileTransferCommand::SendOwnedClipboardFiles { admission, .. } =
                timeout(Duration::from_secs(2), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap()
            else {
                panic!()
            };
            assert!(admission.accept());
            if start_with_file {
                let file_epoch = observation.epoch();
                for _ in 0..3 {
                    assert!(text.poll_outgoing(0).await.unwrap().is_none());
                }
                assert_eq!(
                    observation.epoch(),
                    file_epoch,
                    "auxiliary non-text reads must not alternate the content type"
                );
                let before_text = file_reads.load(std::sync::atomic::Ordering::SeqCst);
                *shared.lock().unwrap() = Native::Text("Y");
                assert!(text.poll_outgoing(1).await.unwrap().is_some());
                *shared.lock().unwrap() = Native::File("/X");
                assert_eq!(
                    file_reads.load(std::sync::atomic::Ordering::SeqCst),
                    before_text,
                    "the file plane must not observe the intermediate text"
                );
                let Some(FileTransferCommand::SendOwnedClipboardFiles {
                    admission: pending, ..
                }) = timeout(Duration::from_secs(2), receiver.recv())
                    .await
                    .unwrap()
                else {
                    panic!("file X must send again even though this plane missed text Y")
                };
                *shared.lock().unwrap() = Native::Text("Z");
                assert!(text.poll_outgoing(2).await.unwrap().is_some());
                assert!(
                    !pending.accept(),
                    "a queued older file copy cannot be admitted after text is observed"
                );
            } else {
                *shared.lock().unwrap() = Native::Text("X");
                assert!(
                    text.poll_outgoing(2).await.unwrap().is_some(),
                    "text X must send again even though this plane missed file Y"
                );
            }
            cancel.send(()).unwrap();
            bridge.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn successful_native_writes_change_observation_but_revoked_writes_do_not() {
        use crate::traits::ClipboardProvider;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let observation = Arc::new(ClipboardObservation::default());
        let gate = Arc::new(AtomicBool::new(true));
        let mut text = OwnedClipboardProvider::new(
            crate::clipboard_provider::MemoryClipboardProvider::new(),
            gate.clone(),
        )
        .with_observation(observation.clone());
        let mut files =
            OwnedClipboardProvider::new(MemoryClipboardFileReferenceProvider::new(), gate.clone())
                .with_observation(observation.clone());
        text.write_clipboard(
            &protocol::ClipboardBundle::text(1, "X"),
            crate::clipboard_plane::ClipboardSyncPolicy::default(),
        )
        .await
        .unwrap();
        let text_epoch = observation.epoch();
        files
            .write_clipboard_file_references(&[ClipboardFileReference::new("/Y")])
            .await
            .unwrap();
        let file_epoch = observation.epoch();
        assert!(file_epoch > text_epoch);
        text.write_clipboard(
            &protocol::ClipboardBundle::text(3, ""),
            crate::clipboard_plane::ClipboardSyncPolicy::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            observation.epoch(),
            file_epoch,
            "empty text is not a valid content observation"
        );
        gate.store(false, Ordering::Release);
        assert!(
            text.write_clipboard(
                &protocol::ClipboardBundle::text(2, "X"),
                crate::clipboard_plane::ClipboardSyncPolicy::default()
            )
            .await
            .unwrap_err()
            .is::<ClipboardOwnershipRevoked>()
        );
        assert_eq!(observation.epoch(), file_epoch);
    }

    #[tokio::test]
    async fn clipboard_file_sync_turns_local_file_references_into_send_commands() {
        let provider = MemoryClipboardFileReferenceProvider::with_references(vec![
            ClipboardFileReference::new("/tmp/a.txt"),
            ClipboardFileReference::new("/tmp/b.txt"),
        ]);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = broadcast::channel(1);

        let task = tokio::spawn(run_clipboard_file_sync(
            provider,
            command_tx,
            event_rx,
            None,
            cancel_rx,
            ClipboardFileSyncConfig {
                poll_interval: Duration::from_millis(1),
                max_files_per_poll: 1,
                ..Default::default()
            },
        ));

        let command = timeout(Duration::from_secs(1), command_rx.recv())
            .await
            .expect("file command should arrive")
            .expect("file command channel should remain open");
        assert_eq!(
            command,
            FileTransferCommand::SendClipboardFiles {
                files: vec![FileTransferGroupFile {
                    path: PathBuf::from("/tmp/a.txt"),
                    mime_type: None
                }]
            }
        );
        assert!(
            timeout(Duration::from_millis(20), command_rx.recv())
                .await
                .is_err(),
            "same clipboard file reference should not be resent"
        );

        let _ = cancel_tx.send(());
        task.await
            .expect("sync task should join")
            .expect("sync task should stop cleanly");
    }

    #[tokio::test]
    async fn clipboard_file_sync_writes_received_file_references_and_forwards_events() {
        let provider = MemoryClipboardFileReferenceProvider::new();
        let (command_tx, _command_rx) = mpsc::channel(8);
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (forward_tx, mut forward_rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = broadcast::channel(1);

        let task = tokio::spawn(run_clipboard_file_sync(
            provider,
            command_tx,
            event_rx,
            Some(forward_tx),
            cancel_rx,
            ClipboardFileSyncConfig {
                poll_interval: Duration::from_secs(60),
                ..ClipboardFileSyncConfig::default()
            },
        ));

        let event = FileTransferEvent::IncomingCompleted {
            transfer_id: 1,
            file_object_id: 2,
            group: None,
            path: PathBuf::from("/tmp/received.txt"),
            size_bytes: 5,
        };
        event_tx
            .send(event.clone())
            .expect("incoming event should send");

        assert_eq!(
            timeout(Duration::from_secs(1), forward_rx.recv())
                .await
                .expect("forwarded event should arrive"),
            Some(event)
        );

        let _ = cancel_tx.send(());
        task.await
            .expect("sync task should join")
            .expect("sync task should stop cleanly");
    }

    #[tokio::test]
    async fn clipboard_file_sync_publishes_group_paths_atomically() {
        let provider = MemoryClipboardFileReferenceProvider::new();
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = broadcast::channel(1);

        let task = tokio::spawn(run_clipboard_file_sync(
            provider,
            command_tx,
            event_rx,
            None,
            cancel_rx,
            ClipboardFileSyncConfig {
                poll_interval: Duration::from_millis(1),
                ..ClipboardFileSyncConfig::default()
            },
        ));

        event_tx
            .send(FileTransferEvent::IncomingClipboardReady {
                ownership: None,
                group_id: 9,
                paths: vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")],
            })
            .expect("group event should send");

        assert!(
            timeout(Duration::from_millis(50), command_rx.recv())
                .await
                .is_err(),
            "publishing a received group should not echo it back as a new send"
        );

        let _ = cancel_tx.send(());
        task.await
            .expect("sync task should join")
            .expect("sync task should stop cleanly");
    }
}
