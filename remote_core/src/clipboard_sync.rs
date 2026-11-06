use crate::clipboard_plane::{
    ClipboardBundleAssembler, ClipboardEnvelopeSpec, ClipboardPlaneError, ClipboardSyncPolicy,
    clipboard_bundle_to_envelopes_with_policy,
};
use crate::data_plane::checksum_crc32;
use crate::traits::ClipboardProvider;
use protocol::{ClipboardBundle, ContentKind, DataEnvelope};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardSyncConfig {
    pub policy: ClipboardSyncPolicy,
    pub stream_id: u32,
    pub next_object_id: u64,
    pub next_sequence_number: u64,
    pub chunk_payload_len: usize,
}

impl Default for ClipboardSyncConfig {
    fn default() -> Self {
        Self {
            policy: ClipboardSyncPolicy::default(),
            stream_id: 1,
            next_object_id: 1,
            next_sequence_number: 1,
            chunk_payload_len: crate::clipboard_plane::DEFAULT_CLIPBOARD_CHUNK_PAYLOAD_LEN,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardOutgoingTransfer {
    pub bundle_id: u64,
    pub content_crc32: u32,
    pub envelopes: Vec<DataEnvelope>,
}

/// Per-peer content history survives foreground ownership changes.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClipboardSyncHistory {
    sent: Option<u32>,
    applied: Option<u32>,
    observed_epoch: u64,
}

#[derive(Debug)]
pub enum ClipboardSyncError {
    Provider(Box<dyn Error + Send + Sync>),
    Plane(ClipboardPlaneError),
    Encode(bincode::Error),
    MissingChunkMetadata,
    UnexpectedContentKind(ContentKind),
}

impl fmt::Display for ClipboardSyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClipboardSyncError::Provider(err) => write!(f, "clipboard provider failed: {err}"),
            ClipboardSyncError::Plane(err) => write!(f, "clipboard data plane failed: {err}"),
            ClipboardSyncError::Encode(err) => write!(f, "clipboard encoding failed: {err}"),
            ClipboardSyncError::MissingChunkMetadata => {
                write!(f, "clipboard data envelope is missing chunk metadata")
            }
            ClipboardSyncError::UnexpectedContentKind(kind) => {
                write!(f, "unexpected clipboard sync content kind: {kind:?}")
            }
        }
    }
}

impl Error for ClipboardSyncError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ClipboardSyncError::Provider(err) => Some(err.as_ref()),
            ClipboardSyncError::Plane(err) => Some(err),
            ClipboardSyncError::Encode(err) => Some(err),
            ClipboardSyncError::MissingChunkMetadata
            | ClipboardSyncError::UnexpectedContentKind(_) => None,
        }
    }
}

impl From<ClipboardPlaneError> for ClipboardSyncError {
    fn from(value: ClipboardPlaneError) -> Self {
        Self::Plane(value)
    }
}

impl From<bincode::Error> for ClipboardSyncError {
    fn from(value: bincode::Error) -> Self {
        Self::Encode(value)
    }
}

pub struct ClipboardSyncEndpoint<P> {
    provider: P,
    config: ClipboardSyncConfig,
    inbound: HashMap<u64, ClipboardBundleAssembler>,
    last_sent_content_crc32: Option<u32>,
    last_applied_content_crc32: Option<u32>,
    last_read_revision: Option<u64>,
    observation: Option<std::sync::Arc<crate::clipboard_file_runtime::ClipboardObservation>>,
    observed_epoch: u64,
    completed: std::collections::VecDeque<(u64, u32, u64, Option<u32>)>,
}

impl<P: ClipboardProvider> ClipboardSyncEndpoint<P> {
    pub fn history(&self) -> ClipboardSyncHistory {
        ClipboardSyncHistory { sent: self.last_sent_content_crc32, applied: self.last_applied_content_crc32, observed_epoch: self.observed_epoch }
    }
    pub fn restore_history(&mut self, history: ClipboardSyncHistory) {
        if self.last_sent_content_crc32 != history.sent || self.last_applied_content_crc32 != history.applied || self.observed_epoch != history.observed_epoch {
            self.last_read_revision = None;
        }
        self.last_sent_content_crc32 = history.sent;
        self.last_applied_content_crc32 = history.applied;
        self.observed_epoch = history.observed_epoch;
    }
    pub fn set_observation(&mut self, observation: Option<std::sync::Arc<crate::clipboard_file_runtime::ClipboardObservation>>) { self.observation = observation; }
    fn adopt_epoch(&mut self, epoch: u64) {
        if self.observed_epoch != epoch {
            self.last_sent_content_crc32 = None;
            self.last_applied_content_crc32 = None;
            self.last_read_revision = None;
            self.observed_epoch = epoch;
        }
    }
    pub fn new(provider: P, config: ClipboardSyncConfig) -> Self {
        Self {
            provider,
            config,
            inbound: HashMap::new(),
            last_sent_content_crc32: None,
            last_applied_content_crc32: None,
            last_read_revision: None,
            observation: None,
            observed_epoch: 0,
            completed: std::collections::VecDeque::new(),
        }
    }

    pub fn provider(&self) -> &P {
        &self.provider
    }

    pub fn provider_mut(&mut self) -> &mut P {
        &mut self.provider
    }

    pub fn config(&self) -> ClipboardSyncConfig {
        self.config
    }

    pub fn last_sent_content_crc32(&self) -> Option<u32> {
        self.last_sent_content_crc32
    }
    pub fn retry_outgoing(&mut self) {
        self.last_sent_content_crc32 = None;
        self.last_read_revision = None;
    }
    pub fn retry_outgoing_if(&mut self, content_crc32: u32) {
        if self.last_sent_content_crc32 == Some(content_crc32) { self.retry_outgoing(); }
    }
    pub fn outgoing_succeeded(&mut self, content_crc32: u32) {
        if self.last_sent_content_crc32 == Some(content_crc32) { self.last_applied_content_crc32 = None; }
    }

    pub fn last_applied_content_crc32(&self) -> Option<u32> {
        self.last_applied_content_crc32
    }

    pub async fn poll_outgoing(
        &mut self,
        timestamp_ms: u64,
    ) -> Result<Option<ClipboardOutgoingTransfer>, ClipboardSyncError> {
        let revision = self.provider.clipboard_revision();
        if self.observation.as_ref().is_some_and(|tracker| tracker.epoch() != self.observed_epoch) { self.last_read_revision = None; }
        if revision.is_some() && revision == self.last_read_revision {
            return Ok(None);
        }
        let Some(bundle) = self
            .provider
            .read_clipboard(self.config.policy)
            .await
            .map_err(ClipboardSyncError::Provider)?
        else {
            // A real non-text/image clipboard invalidates the prior content
            // history so the same item can be copied again later.
            self.last_sent_content_crc32 = None;
            self.last_applied_content_crc32 = None;
            self.last_read_revision = None;
            return Ok(None);
        };

        if let Some(tracker) = &self.observation {
            let Some(epoch) = self.provider.clipboard_observation_epoch() else { return Ok(None) };
            if tracker.epoch() != epoch { return Ok(None); }
            self.adopt_epoch(epoch);
        }
        let content_crc32 = clipboard_content_crc32(&bundle)?;
        // Delayed rendering may itself change the counter during a read.
        self.last_read_revision = revision.filter(|revision| Some(*revision) == self.provider.clipboard_revision());
        if Some(content_crc32) == self.last_sent_content_crc32.or(self.last_applied_content_crc32) {
            return Ok(None);
        }

        let spec = ClipboardEnvelopeSpec {
            object_id: self.config.next_object_id,
            stream_id: self.config.stream_id,
            first_sequence_number: self.config.next_sequence_number,
            timestamp_ms,
            max_chunk_payload_len: self.config.chunk_payload_len,
        };
        let envelopes =
            clipboard_bundle_to_envelopes_with_policy(&bundle, spec, self.config.policy)?;

        self.config.next_object_id = self.config.next_object_id.saturating_add(1);
        self.config.next_sequence_number = self
            .config
            .next_sequence_number
            .saturating_add(envelopes.len() as u64);
        self.last_sent_content_crc32 = Some(content_crc32);

        Ok(Some(ClipboardOutgoingTransfer {
            bundle_id: bundle.bundle_id,
            content_crc32,
            envelopes,
        }))
    }

    pub async fn receive_envelope(
        &mut self,
        envelope: DataEnvelope,
    ) -> Result<Option<ClipboardBundle>, ClipboardSyncError> {
        if envelope.header.kind != ContentKind::ClipboardBundle {
            return Err(ClipboardSyncError::UnexpectedContentKind(
                envelope.header.kind,
            ));
        }

        let object_id = envelope
            .header
            .chunk
            .as_ref()
            .map(|chunk| chunk.object_id)
            .ok_or(ClipboardSyncError::MissingChunkMetadata)?;

        let chunk = envelope.header.chunk.as_ref().unwrap();
        let identity = (
            object_id,
            chunk.total_chunks,
            chunk.total_size,
            envelope
                .header
                .reliability_info
                .as_ref()
                .and_then(|r| r.checksum_crc32),
        );
        if self.completed.contains(&identity) {
            return Ok(None);
        }
        if chunk.total_size > self.config.policy.max_bundle_bytes as u64
            || chunk.total_chunks > 131_072
            || (self.inbound.len() >= 2 && !self.inbound.contains_key(&object_id))
        {
            return Err(ClipboardSyncError::Provider(
                "clipboard receive resource limit exceeded".into(),
            ));
        }
        let assembler = match self.inbound.entry(object_id) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(ClipboardBundleAssembler::from_first_chunk(&envelope)?)
            }
        };
        let progress = assembler.push_chunk(envelope)?;
        if !progress.is_complete {
            return Ok(None);
        }

        let assembler = self
            .inbound
            .remove(&object_id)
            .expect("complete clipboard assembler should still be present");
        let bundle = assembler.finish()?;
        let content_crc32 = clipboard_content_crc32(&bundle)?;
        if let Some(tracker) = &self.observation { self.adopt_epoch(tracker.epoch()); }

        if Some(content_crc32) == self.last_sent_content_crc32.or(self.last_applied_content_crc32) {
            self.remember_completed(identity);
            return Ok(None);
        }

        self.provider
            .write_clipboard(&bundle, self.config.policy)
            .await
            .map_err(ClipboardSyncError::Provider)?;
        if let Some(tracker) = &self.observation {
            let Some(epoch) = self.provider.clipboard_observation_epoch() else { self.remember_completed(identity); return Ok(Some(bundle)); };
            if tracker.epoch() != epoch { self.remember_completed(identity); return Ok(Some(bundle)); }
            self.adopt_epoch(epoch);
        }
        self.last_applied_content_crc32 = Some(content_crc32);
        self.last_sent_content_crc32 = None;
        self.remember_completed(identity);
        Ok(Some(bundle))
    }
    fn remember_completed(&mut self, identity: (u64, u32, u64, Option<u32>)) {
        self.completed.push_back(identity);
        if self.completed.len() > 64 {
            self.completed.pop_front();
        }
    }
}

pub fn clipboard_content_crc32(bundle: &ClipboardBundle) -> Result<u32, ClipboardSyncError> {
    let normalized = ClipboardBundle::new(0, bundle.items.clone());
    let encoded = normalized.encode()?;
    Ok(checksum_crc32(&encoded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_provider::MemoryClipboardProvider;
    use crate::traits::ClipboardProvider;
    use protocol::{ClipboardImage, ClipboardItem, ClipboardText};

    #[tokio::test]
    async fn revoked_write_does_not_mark_content_applied_and_can_retry() {
        use std::sync::{Arc, atomic::AtomicBool};
        use crate::clipboard_file_runtime::{ClipboardOwnershipRevoked, OwnedClipboardProvider};
        let bundle = text_bundle(1, "retry");
        let mut source = ClipboardSyncEndpoint::new(MemoryClipboardProvider::with_bundle(bundle), ClipboardSyncConfig::default());
        let transfer = source.poll_outgoing(1).await.unwrap().unwrap();
        let gate = Arc::new(AtomicBool::new(false));
        let mut revoked = ClipboardSyncEndpoint::new(OwnedClipboardProvider::new(MemoryClipboardProvider::new(), gate), ClipboardSyncConfig::default());
        for envelope in &transfer.envelopes {
            let result = revoked.receive_envelope(envelope.clone()).await;
            if let Err(error) = result {
                assert!(matches!(error, ClipboardSyncError::Provider(error) if error.is::<ClipboardOwnershipRevoked>()));
            }
        }
        let mut retry = ClipboardSyncEndpoint::new(MemoryClipboardProvider::new(), ClipboardSyncConfig::default());
        retry.restore_history(revoked.history());
        let mut applied = false;
        for envelope in transfer.envelopes { applied |= retry.receive_envelope(envelope).await.unwrap().is_some(); }
        assert!(applied);
    }

    #[tokio::test]
    async fn peer_history_suppresses_refocus_echo_but_allows_copy_to_another_peer() {
        for bundle in [text_bundle(1, "from A"), image_bundle(2)] {
            let mut source = ClipboardSyncEndpoint::new(MemoryClipboardProvider::with_bundle(bundle.clone()), ClipboardSyncConfig::default());
            let transfer = source.poll_outgoing(1).await.unwrap().unwrap();
            let mut receiver = ClipboardSyncEndpoint::new(MemoryClipboardProvider::new(), ClipboardSyncConfig::default());
            for envelope in transfer.envelopes { receiver.receive_envelope(envelope).await.unwrap(); }
            let mut refocused = ClipboardSyncEndpoint::new(MemoryClipboardProvider::with_bundle(bundle.clone()), ClipboardSyncConfig::default());
            refocused.restore_history(receiver.history());
            assert!(refocused.poll_outgoing(2).await.unwrap().is_none());
            let mut another_peer = ClipboardSyncEndpoint::new(MemoryClipboardProvider::with_bundle(bundle), ClipboardSyncConfig::default());
            assert!(another_peer.poll_outgoing(2).await.unwrap().is_some());
        }
    }

    #[tokio::test]
    async fn previous_content_can_return_after_opposite_direction_content() {
        for x in [text_bundle(1, "X"), image_bundle(1)] {
            let y = text_bundle(2, "Y");
            let mut local = ClipboardSyncEndpoint::new(MemoryClipboardProvider::with_bundle(x.clone()), ClipboardSyncConfig::default());
            let mut remote = ClipboardSyncEndpoint::new(MemoryClipboardProvider::with_bundle(y.clone()), ClipboardSyncConfig::default());
            assert!(local.poll_outgoing(1).await.unwrap().is_some());
            let incoming_y = remote.poll_outgoing(1).await.unwrap().unwrap();
            for envelope in incoming_y.envelopes { local.receive_envelope(envelope).await.unwrap(); }
            local.provider_mut().write_clipboard(&x, ClipboardSyncPolicy::default()).await.unwrap();
            assert!(local.poll_outgoing(2).await.unwrap().is_some(), "sent X must be reusable after received Y");
            // Increment the remote transfer identity before deliberately copying Y again.
            remote.provider_mut().write_clipboard(&x, ClipboardSyncPolicy::default()).await.unwrap();
            remote.poll_outgoing(2).await.unwrap().unwrap();
            remote.provider_mut().write_clipboard(&y, ClipboardSyncPolicy::default()).await.unwrap();
            let incoming_y = remote.poll_outgoing(3).await.unwrap().unwrap();
            let mut applied = false;
            for envelope in incoming_y.envelopes { applied |= local.receive_envelope(envelope).await.unwrap().is_some(); }
            assert!(applied, "received Y must be reusable after sent X");
            assert!(local.poll_outgoing(3).await.unwrap().is_none(), "the current received content must not echo");
        }
    }

    #[tokio::test]
    async fn failed_send_retains_previous_echo_history_and_retries_new_content() {
        let x = text_bundle(1, "X");
        let y = text_bundle(2, "Y");
        let mut source = ClipboardSyncEndpoint::new(MemoryClipboardProvider::with_bundle(x.clone()), ClipboardSyncConfig::default());
        let mut local = ClipboardSyncEndpoint::new(MemoryClipboardProvider::new(), ClipboardSyncConfig::default());
        for envelope in source.poll_outgoing(1).await.unwrap().unwrap().envelopes { local.receive_envelope(envelope).await.unwrap(); }
        local.provider_mut().write_clipboard(&y, ClipboardSyncPolicy::default()).await.unwrap();
        let pending = local.poll_outgoing(2).await.unwrap().unwrap();
        local.retry_outgoing_if(pending.content_crc32);
        local.provider_mut().write_clipboard(&x, ClipboardSyncPolicy::default()).await.unwrap();
        assert!(local.poll_outgoing(3).await.unwrap().is_none(), "failure must retain the last applied content");
        local.provider_mut().write_clipboard(&y, ClipboardSyncPolicy::default()).await.unwrap();
        assert!(local.poll_outgoing(4).await.unwrap().is_some(), "failed content must remain retryable");
    }

    #[tokio::test]
    async fn text_copy_after_unsupported_clipboard_is_sent_again() {
        for original in [text_bundle(1, "X"), image_bundle(2)] {
            let mut endpoint = ClipboardSyncEndpoint::new(MemoryClipboardProvider::with_bundle(original.clone()), ClipboardSyncConfig::default());
            assert!(endpoint.poll_outgoing(1).await.unwrap().is_some());
            endpoint.provider_mut().clear();
            assert!(endpoint.poll_outgoing(2).await.unwrap().is_none());
            endpoint.provider_mut().write_clipboard(&original, ClipboardSyncPolicy::default()).await.unwrap();
            assert!(endpoint.poll_outgoing(3).await.unwrap().is_some(), "the same content is a new copy after an unsupported clipboard");
        }
    }

    #[tokio::test]
    async fn revoked_read_preserves_prior_clipboard_history() {
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
        use crate::clipboard_file_runtime::{ClipboardOwnershipRevoked, OwnedClipboardProvider};
        let gate = Arc::new(AtomicBool::new(true));
        let provider = OwnedClipboardProvider::new(MemoryClipboardProvider::with_bundle(text_bundle(1, "X")), gate.clone());
        let mut endpoint = ClipboardSyncEndpoint::new(provider, ClipboardSyncConfig::default());
        assert!(endpoint.poll_outgoing(1).await.unwrap().is_some());
        let old = endpoint.last_sent_content_crc32();
        gate.store(false, Ordering::Release);
        assert!(matches!(endpoint.poll_outgoing(2).await, Err(ClipboardSyncError::Provider(error)) if error.is::<ClipboardOwnershipRevoked>()));
        assert_eq!(endpoint.last_sent_content_crc32(), old);
    }

    #[tokio::test]
    async fn ownership_revoked_during_empty_read_does_not_clear_history() {
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
        use crate::clipboard_file_runtime::{ClipboardOwnershipRevoked, OwnedClipboardProvider};
        struct DelayedNone { entered: Arc<tokio::sync::Notify>, release: Arc<tokio::sync::Semaphore> }
        #[async_trait::async_trait]
        impl ClipboardProvider for DelayedNone {
            fn capabilities(&self) -> crate::traits::ClipboardBackendCapabilities { Default::default() }
            async fn read_clipboard(&mut self, _: ClipboardSyncPolicy) -> Result<Option<ClipboardBundle>, Box<dyn Error + Send + Sync>> {
                self.entered.notify_one();
                self.release.acquire().await.unwrap().forget();
                Ok(None)
            }
            async fn write_clipboard(&mut self, _: &ClipboardBundle, _: ClipboardSyncPolicy) -> Result<(), Box<dyn Error + Send + Sync>> { Ok(()) }
        }
        let observation = Arc::new(crate::clipboard_file_runtime::ClipboardObservation::default());
        let gate = Arc::new(AtomicBool::new(true));
        let mut seed = ClipboardSyncEndpoint::new(OwnedClipboardProvider::new(MemoryClipboardProvider::with_bundle(text_bundle(1, "X")), gate.clone()).with_observation(observation.clone()), ClipboardSyncConfig::default());
        seed.set_observation(Some(observation.clone()));
        assert!(seed.poll_outgoing(1).await.unwrap().is_some());
        let previous = seed.history();
        let previous_epoch = observation.epoch();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let mut endpoint = ClipboardSyncEndpoint::new(OwnedClipboardProvider::new(DelayedNone { entered: entered.clone(), release: release.clone() }, gate.clone()).with_observation(observation.clone()), ClipboardSyncConfig::default());
        endpoint.set_observation(Some(observation.clone()));
        endpoint.restore_history(previous);
        let task = tokio::spawn(async move { let result = endpoint.poll_outgoing(2).await; (endpoint, result) });
        entered.notified().await;
        gate.store(false, Ordering::Release);
        release.add_permits(1);
        let (endpoint, result) = task.await.unwrap();
        assert!(matches!(result, Err(ClipboardSyncError::Provider(error)) if error.is::<ClipboardOwnershipRevoked>()));
        assert_eq!(endpoint.history().sent, previous.sent);
        assert_eq!(endpoint.history().applied, previous.applied);
        assert_eq!(observation.epoch(), previous_epoch);
    }

    struct RevisionProvider {
        inner: MemoryClipboardProvider,
        revision: Option<u64>,
        reads: usize,
        change_during_read: bool,
    }
    #[async_trait::async_trait]
    impl ClipboardProvider for RevisionProvider {
        fn capabilities(&self) -> crate::traits::ClipboardBackendCapabilities { self.inner.capabilities() }
        fn clipboard_revision(&self) -> Option<u64> { self.revision }
        async fn read_clipboard(&mut self, policy: ClipboardSyncPolicy) -> Result<Option<ClipboardBundle>, Box<dyn Error + Send + Sync>> {
            self.reads += 1;
            if self.change_during_read { self.revision = self.revision.map(|revision| revision + 1); self.change_during_read = false; }
            self.inner.read_clipboard(policy).await
        }
        async fn write_clipboard(&mut self, bundle: &ClipboardBundle, policy: ClipboardSyncPolicy) -> Result<(), Box<dyn Error + Send + Sync>> {
            self.inner.write_clipboard(bundle, policy).await
        }
    }
    #[tokio::test]
    async fn unchanged_native_revision_skips_reads_but_failed_delivery_can_retry() {
        let provider = RevisionProvider { inner: MemoryClipboardProvider::with_bundle(text_bundle(1, "first")), revision: Some(1), reads: 0, change_during_read: false };
        let mut endpoint = ClipboardSyncEndpoint::new(provider, ClipboardSyncConfig::default());
        assert!(endpoint.poll_outgoing(1).await.unwrap().is_some());
        assert!(endpoint.poll_outgoing(2).await.unwrap().is_none());
        assert_eq!(endpoint.provider().reads, 1);
        endpoint.retry_outgoing();
        assert!(endpoint.poll_outgoing(3).await.unwrap().is_some());
        assert_eq!(endpoint.provider().reads, 2);
        endpoint.provider_mut().inner.write_clipboard(&text_bundle(2, "second"), ClipboardSyncPolicy::default()).await.unwrap();
        endpoint.provider_mut().revision = Some(2);
        assert!(endpoint.poll_outgoing(4).await.unwrap().is_some());
        assert_eq!(endpoint.provider().reads, 3);
    }
    #[tokio::test]
    async fn delayed_rendering_revision_is_observed_again_before_skipping_reads() {
        let provider = RevisionProvider { inner: MemoryClipboardProvider::with_bundle(text_bundle(1, "text")), revision: Some(1), reads: 0, change_during_read: true };
        let mut endpoint = ClipboardSyncEndpoint::new(provider, ClipboardSyncConfig::default());
        assert!(endpoint.poll_outgoing(1).await.unwrap().is_some());
        assert!(endpoint.poll_outgoing(2).await.unwrap().is_none());
        assert!(endpoint.poll_outgoing(3).await.unwrap().is_none());
        assert_eq!(endpoint.provider().reads, 2);
    }
    #[tokio::test]
    async fn empty_or_unknown_revision_does_not_suppress_later_available_data() {
        let provider = RevisionProvider { inner: MemoryClipboardProvider::new(), revision: Some(7), reads: 0, change_during_read: false };
        let mut endpoint = ClipboardSyncEndpoint::new(provider, ClipboardSyncConfig::default());
        assert!(endpoint.poll_outgoing(1).await.unwrap().is_none());
        endpoint.provider_mut().inner.write_clipboard(&text_bundle(1, "ready"), ClipboardSyncPolicy::default()).await.unwrap();
        assert!(endpoint.poll_outgoing(2).await.unwrap().is_some());
        endpoint.provider_mut().revision = None;
        assert!(endpoint.poll_outgoing(3).await.unwrap().is_none());
        assert!(endpoint.poll_outgoing(4).await.unwrap().is_none());
        assert_eq!(endpoint.provider().reads, 4);
    }

    fn text_bundle(bundle_id: u64, text: &str) -> ClipboardBundle {
        ClipboardBundle::new(
            bundle_id,
            vec![ClipboardItem::Text(ClipboardText {
                text: text.to_string(),
            })],
        )
    }

    fn image_bundle(bundle_id: u64) -> ClipboardBundle {
        ClipboardBundle::new(
            bundle_id,
            vec![ClipboardItem::Image(ClipboardImage {
                mime_type: "image/png".to_string(),
                width: Some(1),
                height: Some(1),
                bytes: vec![0x89, b'P', b'N', b'G'],
            })],
        )
    }

    #[tokio::test]
    async fn endpoint_polls_provider_into_reliable_clipboard_envelopes() {
        let provider = MemoryClipboardProvider::with_bundle(text_bundle(7, "hello"));
        let mut endpoint = ClipboardSyncEndpoint::new(
            provider,
            ClipboardSyncConfig {
                chunk_payload_len: 8,
                ..ClipboardSyncConfig::default()
            },
        );

        let transfer = endpoint
            .poll_outgoing(1_000)
            .await
            .expect("poll should succeed")
            .expect("clipboard should produce outgoing transfer");

        assert_eq!(transfer.bundle_id, 7);
        assert!(transfer.envelopes.len() > 1);
        assert!(
            transfer
                .envelopes
                .iter()
                .all(|envelope| envelope.header.kind == ContentKind::ClipboardBundle)
        );
        assert_eq!(
            endpoint.config().next_sequence_number,
            1 + transfer.envelopes.len() as u64
        );
        assert_eq!(
            endpoint.last_sent_content_crc32(),
            Some(transfer.content_crc32)
        );
    }

    #[tokio::test]
    async fn endpoint_applies_received_transfer_and_suppresses_echo() {
        let source_provider = MemoryClipboardProvider::with_bundle(text_bundle(1, "remote text"));
        let sink_provider = MemoryClipboardProvider::new();
        let mut source =
            ClipboardSyncEndpoint::new(source_provider, ClipboardSyncConfig::default());
        let mut sink = ClipboardSyncEndpoint::new(sink_provider, ClipboardSyncConfig::default());
        let transfer = source
            .poll_outgoing(2_000)
            .await
            .expect("poll should succeed")
            .expect("source should send clipboard");

        let mut applied = None;
        for envelope in transfer.envelopes {
            applied = sink
                .receive_envelope(envelope)
                .await
                .expect("receive should succeed")
                .or(applied);
        }

        assert_eq!(applied, Some(text_bundle(1, "remote text")));
        let read_back = sink
            .provider_mut()
            .read_clipboard(ClipboardSyncPolicy::default())
            .await
            .expect("sink provider should read");
        assert_eq!(read_back, Some(text_bundle(1, "remote text")));
        assert_eq!(
            sink.poll_outgoing(3_000)
                .await
                .expect("echo poll should succeed"),
            None
        );
    }

    #[tokio::test]
    async fn endpoint_sends_new_local_change_after_remote_apply() {
        let source_provider = MemoryClipboardProvider::with_bundle(text_bundle(1, "first"));
        let mut endpoint =
            ClipboardSyncEndpoint::new(source_provider, ClipboardSyncConfig::default());

        let first = endpoint
            .poll_outgoing(1_000)
            .await
            .expect("first poll should succeed")
            .expect("first clipboard should send");
        endpoint.last_applied_content_crc32 = Some(first.content_crc32);

        endpoint
            .provider_mut()
            .write_clipboard(&text_bundle(2, "second"), ClipboardSyncPolicy::default())
            .await
            .expect("local provider update should succeed");
        let second = endpoint
            .poll_outgoing(2_000)
            .await
            .expect("second poll should succeed")
            .expect("new local clipboard should send");

        assert_ne!(first.content_crc32, second.content_crc32);
    }

    #[tokio::test]
    async fn content_crc_ignores_bundle_id() {
        assert_eq!(
            clipboard_content_crc32(&text_bundle(1, "same")).expect("crc should compute"),
            clipboard_content_crc32(&text_bundle(2, "same")).expect("crc should compute")
        );
        assert_ne!(
            clipboard_content_crc32(&text_bundle(1, "left")).expect("crc should compute"),
            clipboard_content_crc32(&text_bundle(1, "right")).expect("crc should compute")
        );
    }

    #[tokio::test]
    async fn endpoint_roundtrips_image_clipboard() {
        let source_provider = MemoryClipboardProvider::with_bundle(image_bundle(9));
        let sink_provider = MemoryClipboardProvider::new();
        let mut source =
            ClipboardSyncEndpoint::new(source_provider, ClipboardSyncConfig::default());
        let mut sink = ClipboardSyncEndpoint::new(sink_provider, ClipboardSyncConfig::default());
        let transfer = source
            .poll_outgoing(4_000)
            .await
            .expect("image poll should succeed")
            .expect("image clipboard should send");

        let mut applied = None;
        for envelope in transfer.envelopes {
            applied = sink
                .receive_envelope(envelope)
                .await
                .expect("image receive should succeed")
                .or(applied);
        }

        assert_eq!(applied, Some(image_bundle(9)));
    }

    #[tokio::test]
    async fn endpoint_rejects_non_clipboard_envelope() {
        let provider = MemoryClipboardProvider::new();
        let mut endpoint = ClipboardSyncEndpoint::new(provider, ClipboardSyncConfig::default());
        let envelope = DataEnvelope::realtime_video(1, 1, 1_000, 1_016, vec![1]);

        let err = endpoint
            .receive_envelope(envelope)
            .await
            .expect_err("non clipboard envelope should be rejected");

        assert!(matches!(
            err,
            ClipboardSyncError::UnexpectedContentKind(ContentKind::VideoH265)
        ));
    }
}
