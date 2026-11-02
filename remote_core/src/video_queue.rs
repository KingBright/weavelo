//! Bounded HEVC ingress: replace old GOPs, never decode across an eviction.
use crate::{client_session::VideoPacket, media_plane::is_hevc_keyframe};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

/// The same dependency-aware policy is used before native and mobile decoders.
/// Payloads retired by `push` are returned so callers can free them after unlocking.
pub struct EncodedQueue<T> {
    packets: VecDeque<T>,
    capacity: usize,
    source: Option<u64>,
    waiting: bool,
    keyframe_pending: bool,
}
#[derive(Debug, Default)]
pub struct Push {
    pub accepted: bool,
    pub dropped: usize,
    pub needs_keyframe: bool,
}
impl<T> EncodedQueue<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            packets: VecDeque::with_capacity(capacity),
            capacity,
            source: None,
            waiting: false,
            keyframe_pending: false,
        }
    }
    pub fn push(&mut self, packet: T, keyframe: bool, source: u64) -> (Push, VecDeque<T>) {
        let mut result = Push::default();
        let mut discarded = VecDeque::new();
        if self.source.is_some_and(|current| source < current) {
            discarded.push_back(packet);
            result.dropped = 1;
            result.needs_keyframe = self.waiting;
            return (result, discarded);
        }
        let new_source = self.source.is_some_and(|current| current != source);
        self.source = Some(source);
        if keyframe || new_source {
            discarded =
                std::mem::replace(&mut self.packets, VecDeque::with_capacity(self.capacity));
            self.waiting = !keyframe;
            self.keyframe_pending = false;
        } else if !self.waiting && self.packets.len() == self.capacity {
            discarded =
                std::mem::replace(&mut self.packets, VecDeque::with_capacity(self.capacity));
            // Keep the not-yet-decoded keyframe, but retire its P/B tail as a
            // whole. No later dependent is admitted until a fresh keyframe.
            if self.keyframe_pending {
                self.packets
                    .push_back(discarded.pop_front().expect("pending keyframe"));
            }
            self.waiting = true;
        }
        if self.waiting {
            discarded.push_back(packet);
            result.needs_keyframe = true;
        } else {
            self.packets.push_back(packet);
            self.keyframe_pending |= keyframe;
            result.accepted = true;
        }
        result.dropped = discarded.len();
        (result, discarded)
    }
    pub fn pop_front(&mut self) -> Option<T> {
        let packet = self.packets.pop_front();
        if packet.is_some() {
            self.keyframe_pending = false;
        }
        packet
    }
    pub fn clear(&mut self) {
        self.packets.clear();
        self.source = None;
        self.waiting = true;
        self.keyframe_pending = false;
    }
    pub fn len(&self) -> usize {
        self.packets.len()
    }
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }
}
#[derive(Clone)]
pub struct VideoSender {
    state: Arc<Mutex<EncodedQueue<VideoPacket>>>,
    wake: mpsc::Sender<()>,
}
pub struct VideoReceiver {
    state: Arc<Mutex<EncodedQueue<VideoPacket>>>,
    wake: mpsc::Receiver<()>,
}
pub fn channel(capacity: usize) -> (VideoSender, VideoReceiver) {
    let state = Arc::new(Mutex::new(EncodedQueue::new(capacity)));
    let (tx, rx) = mpsc::channel(1);
    (
        VideoSender {
            state: state.clone(),
            wake: tx,
        },
        VideoReceiver { state, wake: rx },
    )
}
impl VideoSender {
    pub fn try_send(
        &self,
        packet: VideoPacket,
    ) -> Result<Push, mpsc::error::SendError<VideoPacket>> {
        if self.wake.is_closed() {
            return Err(mpsc::error::SendError(packet));
        }
        let keyframe = is_hevc_keyframe(&packet.0.payload);
        let source = packet.2;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.wake.is_closed() {
            return Err(mpsc::error::SendError(packet));
        }
        let (result, discarded) = state.push(packet, keyframe, source);
        drop(state);
        // One wakeup is sufficient for all queued packets. Never await the
        // decoder on the socket/control actor, or release payloads under its lock.
        if result.accepted {
            let _ = self.wake.try_send(());
        }
        drop(discarded);
        Ok(result)
    }
    pub async fn send(
        &self,
        packet: VideoPacket,
    ) -> Result<Push, mpsc::error::SendError<VideoPacket>> {
        self.try_send(packet)
    }
    pub fn is_closed(&self) -> bool {
        self.wake.is_closed()
    }
}
impl VideoReceiver {
    pub fn try_recv(&mut self) -> Result<VideoPacket, mpsc::error::TryRecvError> {
        let packet = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front();
        if let Some(packet) = packet {
            return Ok(packet);
        }
        Err(if self.wake.is_closed() {
            mpsc::error::TryRecvError::Disconnected
        } else {
            mpsc::error::TryRecvError::Empty
        })
    }
    pub async fn recv(&mut self) -> Option<VideoPacket> {
        std::future::poll_fn(|cx| self.poll_recv(cx)).await
    }
    pub fn poll_recv(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<VideoPacket>> {
        loop {
            if let Ok(packet) = self.try_recv() {
                return std::task::Poll::Ready(Some(packet));
            }
            match self.wake.poll_recv(cx) {
                std::task::Poll::Ready(Some(())) => continue,
                std::task::Poll::Ready(None) => {
                    return std::task::Poll::Ready(self.try_recv().ok());
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
    pub fn len(&self) -> usize {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
impl Drop for VideoReceiver {
    fn drop(&mut self) {
        self.wake.close();
        let discarded =
            std::mem::take(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()).packets);
        drop(discarded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(sequence: u16, keyframe: bool, source: u64) -> VideoPacket {
        (
            protocol::RtpPacket {
                header: protocol::RtpHeader {
                    version: 2,
                    payload_type: 16,
                    sequence_number: sequence,
                    timestamp: 1,
                    ssrc: 7,
                },
                payload: vec![0, 0, 1, if keyframe { 0x26 } else { 0x02 }, 1],
            },
            protocol::FrameTimingCheckpoints::new(0),
            source,
        )
    }
    #[test]
    fn newest_keyframe_replaces_the_entire_older_chain() {
        let (tx, mut rx) = channel(4);
        tx.try_send(packet(0, true, 1)).unwrap();
        tx.try_send(packet(1, false, 1)).unwrap();
        assert_eq!(tx.try_send(packet(2, true, 1)).unwrap().dropped, 2);
        tx.try_send(packet(3, false, 1)).unwrap();
        assert_eq!(rx.try_recv().unwrap().0.header.sequence_number, 2);
        assert_eq!(rx.try_recv().unwrap().0.header.sequence_number, 3);
    }
    #[test]
    fn overload_never_retains_dependents_of_a_discarded_reference() {
        let (tx, mut rx) = channel(2);
        tx.try_send(packet(0, true, 1)).unwrap();
        tx.try_send(packet(1, false, 1)).unwrap();
        let full = tx.try_send(packet(2, false, 1)).unwrap();
        assert_eq!(full.dropped, 2);
        assert!(full.needs_keyframe);
        assert_eq!(rx.try_recv().unwrap().0.header.sequence_number, 0);
        assert!(rx.is_empty());
        assert!(tx.try_send(packet(3, false, 1)).unwrap().needs_keyframe);
        tx.try_send(packet(4, true, 1)).unwrap();
        tx.try_send(packet(5, false, 1)).unwrap();
        assert_eq!(rx.try_recv().unwrap().0.header.sequence_number, 4);
        assert_eq!(rx.try_recv().unwrap().0.header.sequence_number, 5);
    }
    #[test]
    fn old_source_cannot_replace_the_current_keyframe() {
        let (tx, mut rx) = channel(2);
        tx.try_send(packet(0, true, 1)).unwrap();
        tx.try_send(packet(1, true, 2)).unwrap();
        assert_eq!(tx.try_send(packet(2, true, 1)).unwrap().dropped, 1);
        assert_eq!(rx.try_recv().unwrap().2, 2);
    }
    #[test]
    fn overflow_after_decoding_the_keyframe_waits_for_a_fresh_chain() {
        let (tx, mut rx) = channel(2);
        tx.try_send(packet(0, true, 1)).unwrap();
        assert_eq!(rx.try_recv().unwrap().0.header.sequence_number, 0);
        tx.try_send(packet(1, false, 1)).unwrap();
        tx.try_send(packet(2, false, 1)).unwrap();
        let full = tx.try_send(packet(3, false, 1)).unwrap();
        assert_eq!(full.dropped, 3);
        assert!(full.needs_keyframe);
        assert!(rx.is_empty());
        assert!(tx.try_send(packet(4, false, 1)).unwrap().needs_keyframe);
        let recovered = tx.try_send(packet(5, true, 1)).unwrap();
        assert!(!recovered.needs_keyframe);
        assert_eq!(rx.try_recv().unwrap().0.header.sequence_number, 5);
    }
    #[test]
    fn a_source_change_cannot_admit_an_old_or_incomplete_chain() {
        let (tx, mut rx) = channel(2);
        tx.try_send(packet(0, true, 1)).unwrap();
        assert!(tx.try_send(packet(1, false, 2)).unwrap().needs_keyframe);
        assert!(rx.is_empty());
        assert!(tx.try_send(packet(2, true, 1)).unwrap().needs_keyframe);
        assert!(rx.is_empty());
        assert!(!tx.try_send(packet(3, true, 2)).unwrap().needs_keyframe);
        assert_eq!(rx.try_recv().unwrap().2, 2);
    }
    #[test]
    fn clearing_for_a_new_session_accepts_its_fresh_epoch_only_at_a_keyframe() {
        let mut queue = EncodedQueue::new(2);
        queue.push(1, true, 9);
        queue.clear();
        assert!(!queue.push(2, false, 0).0.accepted);
        assert!(queue.is_empty());
        assert!(queue.push(3, true, 0).0.accepted);
        assert_eq!(queue.pop_front(), Some(3));
    }
    #[tokio::test]
    async fn wakeup_and_shutdown_do_not_block_or_lose_the_last_frame() {
        let (tx, mut rx) = channel(2);
        tx.try_send(packet(0, true, 1)).unwrap();
        drop(tx);
        assert!(rx.recv().await.is_some());
        assert!(rx.recv().await.is_none());
        let (tx, rx) = channel(2);
        drop(rx);
        assert!(tx.try_send(packet(1, true, 1)).is_err());
    }
    #[tokio::test]
    async fn cancelling_an_empty_receive_cannot_consume_a_later_keyframe() {
        let (tx, mut rx) = channel(2);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), rx.recv())
                .await
                .is_err()
        );
        let consumer = tokio::spawn(async move { rx.recv().await });
        tokio::task::yield_now().await;
        tx.try_send(packet(7, true, 1)).unwrap();
        let packet = tokio::time::timeout(std::time::Duration::from_secs(1), consumer)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(packet.0.header.sequence_number, 7);
    }
}
