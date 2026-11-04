//! Owned CPU frames at the restricted PipeWire/encoder boundary.
//!
//! SPA mappings and buffers stay on the PipeWire thread. A borrowed plane begins
//! at SPA data.data (the mapping offset has already been applied by PipeWire).
//! Only checked, tightly packed copies and metadata cross this boundary.

use std::sync::Mutex;
use tokio::sync::Notify;

pub(crate) mod dmabuf;

pub const MAX_DIMENSION: u32 = 8192;
pub const MAX_PIXELS: u64 = 33_554_432;
pub const MAX_FRAME_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra,
    Bgrx,
    Rgba,
    Rgbx,
    Nv12,
    I420,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceColor {
    // Preserve the exact SPA enums, including zero (unknown). Conversion to
    // codec values belongs to the encoder, never to a resolution heuristic.
    pub range: u32,
    pub matrix: u32,
    pub transfer: u32,
    pub primaries: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Identity,
    Unsupported(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameFormat {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub color: SourceColor,
    pub crop: Option<Crop>,
    pub transform: Transform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameStamp {
    pub generation: u64,
    /// Local negotiation epoch; never serialized into the network protocol.
    pub format_revision: u64,
    pub sequence: Option<u64>,
    /// PipeWire's nanosecond PTS. Its epoch is not the host timing epoch.
    pub pipewire_pts_ns: Option<i64>,
    /// Host quanta arrival time; no inferred capture time is claimed.
    pub arrival_ts_us: u64,
}

pub struct BorrowedPlane<'a> {
    pub data: &'a [u8],
    /// Original mapping offset is metadata, not another index into data.data.
    pub mapping_offset: u32,
    pub chunk_offset: u32,
    pub chunk_size: u32,
    pub stride: i32,
}

pub struct FrameBudget {
    per_frame: usize,
    total: usize,
    used: std::sync::atomic::AtomicUsize,
    recycled: Mutex<Option<Vec<Vec<u8>>>>,
}
impl std::fmt::Debug for FrameBudget {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
        f.debug_struct("FrameBudget").field("per_frame",&self.per_frame).field("total",&self.total)
            .field("used",&self.used.load(std::sync::atomic::Ordering::Acquire)).finish_non_exhaustive()
    }
}
impl FrameBudget {
    pub fn new(per_frame: usize, total: usize) -> Result<std::sync::Arc<Self>, FrameError> {
        if per_frame == 0
            || per_frame > MAX_FRAME_BYTES
            || total < per_frame
            || total > 2 * MAX_FRAME_BYTES
        {
            return Err(FrameError::Budget);
        }
        Ok(std::sync::Arc::new(Self {
            per_frame,
            total,
            used: std::sync::atomic::AtomicUsize::new(0),
            recycled: Mutex::new(None),
        }))
    }
    fn reserve(self: &std::sync::Arc<Self>, bytes: usize) -> Result<FrameMemory, FrameError> {
        use std::sync::atomic::Ordering;
        if bytes > self.per_frame {
            return Err(FrameError::Budget);
        }
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|n| *n <= self.total)
            })
            .map_err(|_| FrameError::Budget)?;
        Ok(FrameMemory {
            budget: self.clone(),
            bytes,
        })
    }
    fn acquire(self:&std::sync::Arc<Self>,bytes:usize,layouts:&[PlaneLayout])
        ->Result<(FrameMemory,Vec<Vec<u8>>),FrameError> {
        // A free allocation must be removed before reserving more live bytes.
        // Holding this lock through admission keeps live + cached <= total.
        let mut cache=self.recycled.lock().unwrap_or_else(|error|error.into_inner());
        let recycled=cache.take().filter(|planes| planes.len()==layouts.len()
            && planes.iter().zip(layouts).all(|(plane,layout)|plane.capacity()==layout.copied_row_bytes*layout.copied_rows));
        let memory=self.reserve(bytes)?;
        Ok((memory,recycled.unwrap_or_default()))
    }
}
#[derive(Debug)]
struct FrameMemory {
    budget: std::sync::Arc<FrameBudget>,
    bytes: usize,
}
impl Drop for FrameMemory {
    fn drop(&mut self) {
        self.budget
            .used
            .fetch_sub(self.bytes, std::sync::atomic::Ordering::AcqRel);
    }
}

#[derive(Clone, Debug)]
pub struct OwnedFrame {
    pub source_format: FrameFormat,
    pub width: u32,
    pub height: u32,
    pub planes: std::sync::Arc<Vec<Vec<u8>>>,
    pub(crate) dmabuf: Option<std::sync::Arc<dmabuf::DmaBufFrame>>,
    _memory: Option<std::sync::Arc<FrameMemory>>,
    pub mapping_offsets: Vec<u32>,
    pub stamp: FrameStamp,
    pub(crate) received_at: std::time::Instant,
    pub(crate) initial_snapshot: bool,
    /// Per-view metadata on this immutable capture lease. Raw storage is shared;
    /// a viewer never changes another viewer's crop or capture format.
    pub(crate) viewport:Option<protocol::viewport::FrameGeometry>,
}

impl Drop for OwnedFrame {
    fn drop(&mut self) {
        let Some(memory)=self._memory.as_ref() else {return;};
        // Cloned snapshots and consumers keep immutable pixels. Only the very
        // last frame/plane owner may return an allocation for later writes.
        if std::sync::Arc::strong_count(memory)!=1 || std::sync::Arc::strong_count(&self.planes)!=1 {return;}
        static EMPTY:std::sync::OnceLock<std::sync::Arc<Vec<Vec<u8>>>>=std::sync::OnceLock::new();
        let planes=std::mem::replace(&mut self.planes,EMPTY.get_or_init(||std::sync::Arc::new(Vec::new())).clone());
        if let Ok(planes)=std::sync::Arc::try_unwrap(planes) {
            let capacity=planes.iter().map(Vec::capacity).sum::<usize>();
            if capacity<=memory.bytes {
                *memory.budget.recycled.lock().unwrap_or_else(|error|error.into_inner())=Some(planes);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    Dimensions,
    UnsupportedTransform,
    Crop,
    PlaneCount,
    Stride,
    Bounds,
    Budget,
    Generation,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid native Linux frame: {self:?}")
    }
}
impl std::error::Error for FrameError {}

struct PlaneLayout {
    row_bytes: usize,
    rows: usize,
    crop_x_bytes: usize,
    crop_y: usize,
    copied_row_bytes: usize,
    copied_rows: usize,
}

impl FrameFormat {
    fn crop_rect(self) -> Result<Crop, FrameError> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_DIMENSION
            || self.height > MAX_DIMENSION
            || u64::from(self.width) * u64::from(self.height) > MAX_PIXELS
        {
            return Err(FrameError::Dimensions);
        }
        if self.transform != Transform::Identity {
            return Err(FrameError::UnsupportedTransform);
        }
        let crop = self.crop.unwrap_or(Crop {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        });
        if crop.width == 0
            || crop.height == 0
            || crop
                .x
                .checked_add(crop.width)
                .is_none_or(|x| x > self.width)
            || crop
                .y
                .checked_add(crop.height)
                .is_none_or(|y| y > self.height)
        {
            return Err(FrameError::Crop);
        }
        if matches!(self.pixel_format, PixelFormat::Nv12 | PixelFormat::I420)
            && [
                self.width,
                self.height,
                crop.x,
                crop.y,
                crop.width,
                crop.height,
            ]
            .into_iter()
            .any(|value| value % 2 != 0)
        {
            return Err(FrameError::Crop);
        }
        Ok(crop)
    }

    fn plane_layouts(self, crop: Crop) -> Vec<PlaneLayout> {
        let layout = |subsample: u32, bytes: u32| PlaneLayout {
            row_bytes: (self.width / subsample * bytes) as usize,
            rows: (self.height / subsample) as usize,
            crop_x_bytes: (crop.x / subsample * bytes) as usize,
            crop_y: (crop.y / subsample) as usize,
            copied_row_bytes: (crop.width / subsample * bytes) as usize,
            copied_rows: (crop.height / subsample) as usize,
        };
        match self.pixel_format {
            PixelFormat::Bgra | PixelFormat::Bgrx | PixelFormat::Rgba | PixelFormat::Rgbx => {
                vec![layout(1, 4)]
            }
            PixelFormat::Nv12 => vec![layout(1, 1), layout(2, 2)],
            PixelFormat::I420 => vec![layout(1, 1), layout(2, 1), layout(2, 1)],
        }
    }
}

impl OwnedFrame {
    pub(crate) fn from_dmabuf(format: FrameFormat, gpu: dmabuf::DmaBufFrame,
        stamp: FrameStamp) -> Result<Self, FrameError> {
        let crop = format.crop_rect()?;
        let frame = Self { source_format:format,width:crop.width,height:crop.height,
            planes:std::sync::Arc::new(Vec::new()),dmabuf:Some(std::sync::Arc::new(gpu)),
            _memory:None,mapping_offsets:Vec::new(),stamp,
            received_at:std::time::Instant::now(),initial_snapshot:false,viewport:None };
        frame.validate_owned()?;
        Ok(frame)
    }
    pub(crate) fn is_expired(&self, max_age: std::time::Duration) -> bool {
        self.received_at.elapsed() > max_age
    }
    /// Recheck the public owned boundary before passing bytes to a subprocess.
    pub fn validate_owned(&self) -> Result<(), FrameError> {
        if self.stamp.generation == 0 {
            return Err(FrameError::Generation);
        }
        let crop = self.source_format.crop_rect()?;
        if (self.width, self.height) != (crop.width, crop.height) {
            return Err(FrameError::Dimensions);
        }
        if let Some(gpu) = self.dmabuf.as_ref() {
            if !self.planes.is_empty() || !self.mapping_offsets.is_empty()
                || gpu.key.generation != self.stamp.generation
                || gpu.key.revision != self.stamp.format_revision {
                return Err(FrameError::Generation);
            }
            return gpu.validate(self.source_format);
        }
        let layouts = self.source_format.plane_layouts(crop);
        if self.planes.len() != layouts.len() || self.mapping_offsets.len() != layouts.len() {
            return Err(FrameError::PlaneCount);
        }
        let mut total = 0usize;
        for (plane, layout) in self.planes.iter().zip(layouts) {
            let expected = layout
                .copied_row_bytes
                .checked_mul(layout.copied_rows)
                .ok_or(FrameError::Budget)?;
            if plane.len() != expected {
                return Err(FrameError::Bounds);
            }
            total = total.checked_add(expected).ok_or(FrameError::Budget)?;
        }
        if total > MAX_FRAME_BYTES {
            return Err(FrameError::Budget);
        }
        Ok(())
    }

    pub fn copy_from(
        format: FrameFormat,
        planes: &[BorrowedPlane<'_>],
        stamp: FrameStamp,
    ) -> Result<Self, FrameError> {
        Self::copy_from_budget(format, planes, stamp, None)
    }
    pub fn copy_from_budget(
        format: FrameFormat,
        planes: &[BorrowedPlane<'_>],
        stamp: FrameStamp,
        budget: Option<&std::sync::Arc<FrameBudget>>,
    ) -> Result<Self, FrameError> {
        if stamp.generation == 0 {
            return Err(FrameError::Generation);
        }
        let crop = format.crop_rect()?;
        let layouts = format.plane_layouts(crop);
        if layouts.len() != planes.len() {
            return Err(FrameError::PlaneCount);
        }
        // Validate every plane before allocating or copying any of them.
        let mut total = 0usize;
        for (plane, layout) in planes.iter().zip(&layouts) {
            let stride = usize::try_from(plane.stride).map_err(|_| FrameError::Stride)?;
            if stride < layout.row_bytes {
                return Err(FrameError::Stride);
            }
            let begin = plane.chunk_offset as usize;
            let end = begin
                .checked_add(plane.chunk_size as usize)
                .ok_or(FrameError::Bounds)?;
            let required = stride
                .checked_mul(layout.rows - 1)
                .and_then(|value| value.checked_add(layout.row_bytes))
                .ok_or(FrameError::Bounds)?;
            if end > plane.data.len() || required > plane.chunk_size as usize {
                return Err(FrameError::Bounds);
            }
            let bytes = layout
                .copied_row_bytes
                .checked_mul(layout.copied_rows)
                .ok_or(FrameError::Budget)?;
            total = total.checked_add(bytes).ok_or(FrameError::Budget)?;
            if total > MAX_FRAME_BYTES {
                return Err(FrameError::Budget);
            }
        }
        // Reserve checked output size before allocating a plane. The reservation
        // follows the owned frame through first-frame, mailbox and consumer lifetimes.
        let (memory,mut owned)=match budget {
            Some(budget)=> {let (memory,planes)=budget.acquire(total,&layouts)?;(Some(memory),planes)},
            None=>(None,Vec::new()),
        };
        for (index,(plane, layout)) in planes.iter().zip(&layouts).enumerate() {
            let stride = plane.stride as usize;
            if index==owned.len() {owned.push(Vec::with_capacity(layout.copied_row_bytes*layout.copied_rows));}
            let copy=&mut owned[index];copy.clear();
            if stride==layout.copied_row_bytes {
                let begin=plane.chunk_offset as usize+layout.crop_y*stride+layout.crop_x_bytes;
                copy.extend_from_slice(&plane.data[begin..begin+layout.copied_row_bytes*layout.copied_rows]);
            } else {for row in 0..layout.copied_rows {
                let begin = plane.chunk_offset as usize
                    + (layout.crop_y + row) * stride
                    + layout.crop_x_bytes;
                copy.extend_from_slice(&plane.data[begin..begin + layout.copied_row_bytes]);
            }}
        }
        Ok(Self {
            source_format: format,
            width: crop.width,
            height: crop.height,
            planes: std::sync::Arc::new(owned),
            dmabuf: None,
            _memory: memory.map(std::sync::Arc::new),
            mapping_offsets: planes.iter().map(|plane| plane.mapping_offset).collect(),
            stamp,
            received_at: std::time::Instant::now(),
            initial_snapshot: false,
            viewport:None,
        })
    }
}

struct MailboxState {
    latest: Option<(OwnedFrame, std::time::Instant)>,
    // Private current-source pixels, never a queue or a renewed capture time.
    // Only native damage-driven sources retain this shared immutable lease.
    snapshot:Option<OwnedFrame>,
    format_revision: u64,
    dimensions: Option<(u32, u32)>,
    delivered: bool,
    paused: bool,
    closed: bool,
}

/// Only unsubmitted raw frames may replace one another. No compressed AU type
/// is accepted here. Closing revokes the generation and wakes blocked readers.
pub struct FrameMailbox {
    generation: u64,
    max_age: Option<std::time::Duration>,
    allow_initial_snapshot: bool,
    state: Mutex<MailboxState>,
    changed: Notify,
    children: Mutex<Vec<std::sync::Weak<FrameMailbox>>>,
}

impl FrameMailbox {
    pub fn new(generation: u64) -> Result<Self, FrameError> {
        Self::with_max_age(generation, None)
    }

    pub(crate) fn with_max_age(
        generation: u64,
        max_age: Option<std::time::Duration>,
    ) -> Result<Self, FrameError> {
        Self::with_policy(generation, max_age, false)
    }
    pub(crate) fn native_stream(
        generation: u64,
        max_age: std::time::Duration,
    ) -> Result<Self, FrameError> {
        Self::with_policy(generation, Some(max_age), true)
    }
    fn with_policy(
        generation: u64,
        max_age: Option<std::time::Duration>,
        allow_initial_snapshot: bool,
    ) -> Result<Self, FrameError> {
        if generation == 0 {
            return Err(FrameError::Generation);
        }
        Ok(Self {
            generation,
            max_age,
            allow_initial_snapshot,
            state: Mutex::new(MailboxState {
                latest: None,
                format_revision: 0,
                dimensions: None,
                delivered: false,
                snapshot:None,
                paused: false,
                closed: false,
            }),
            changed: Notify::new(),
            children: Mutex::new(Vec::new()),
        })
    }

    /// One latest raw frame per viewer; pixel planes and their memory charge
    /// remain shared. A paused/closed viewer never changes its parent source.
    pub(crate) fn fork(&self) -> Result<std::sync::Arc<Self>, FrameError> {
        let mut children=self.children.lock().unwrap_or_else(|e|e.into_inner());
        children.retain(|child|child.strong_count()>0);
        if children.len()>=8 {return Err(FrameError::Budget);}
        let state=self.state.lock().unwrap_or_else(|e|e.into_inner());
        if state.closed {return Err(FrameError::Generation);}
        let child=std::sync::Arc::new(Self::with_policy(self.generation,self.max_age,self.allow_initial_snapshot)?);
        {
            let mut next=child.state.lock().unwrap_or_else(|e|e.into_inner());
            next.latest=state.latest.clone();
            next.snapshot=state.snapshot.clone();
            next.dimensions=state.dimensions;
            next.format_revision=state.format_revision;
            next.paused=state.paused;
        }
        children.push(std::sync::Arc::downgrade(&child));
        Ok(child)
    }

    pub fn publish(&self, frame: OwnedFrame) -> bool {
        self.publish_inner(frame, false)
    }
    pub(crate) fn publish_initial(&self, frame: OwnedFrame) -> bool {
        self.publish_inner(frame, true)
    }
    fn publish_inner(&self, frame: OwnedFrame, only_if_empty: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed
            || frame.stamp.generation != self.generation
            || frame.stamp.format_revision != state.format_revision
        {
            return false;
        }
        let old_snapshot=if self.allow_initial_snapshot&&(!only_if_empty||state.snapshot.is_none()) {
            state.dimensions=Some((frame.width,frame.height));state.snapshot.replace(frame.clone())
        }else{None};
        let mut old_latest=None;
        if !state.paused&&(!only_if_empty || state.latest.is_none()) {
            let selected=if only_if_empty {state.snapshot.as_ref().unwrap_or(&frame)}else{&frame}.clone();
            state.dimensions = Some((selected.width, selected.height));
            let received=selected.received_at;old_latest=state.latest.replace((selected,received));
        }
        let paused=state.paused;
        drop(state);
        drop(old_latest);drop(old_snapshot);
        if !paused{self.changed.notify_one();}
        self.children.lock().unwrap_or_else(|e|e.into_inner()).retain(|child| {
            let Some(child)=child.upgrade() else {return false;};
            child.publish_inner(frame.clone(),only_if_empty);
            true
        });
        !paused
    }

    /// Invalidate queued pixels before admitting a newly negotiated format.
    pub(crate) fn set_format_revision(&self, revision: u64) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed || revision <= state.format_revision {
            return false;
        }
        state.format_revision = revision;
        let previous=(state.latest.take(),state.snapshot.take());
        drop(state);
        drop(previous);
        self.children.lock().unwrap_or_else(|e|e.into_inner()).retain(|child| {
            let Some(child)=child.upgrade() else {return false;};
            child.set_format_revision(revision);
            true
        });
        true
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed
    }
    pub(crate) fn is_paused(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .paused
    }
    /// A caller-held frame reference cannot survive renegotiation or pause.
    pub(crate) fn matches_input_epoch(&self, revision: u64, size: [u32; 2]) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        !state.closed
            && !state.paused
            && state.format_revision == revision
            && state.dimensions == Some((size[0], size[1]))
    }

    pub(crate) fn dimensions(&self) -> Option<(u32, u32)> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .dimensions
    }

    /// A new viewport may reuse a valid static image. Its capture stamp and
    /// received_at remain original, and epoch/pause/revocation still apply.
    pub(crate) fn snapshot(&self)->Option<OwnedFrame>{
        let state=self.state.lock().unwrap_or_else(|e|e.into_inner());
        if state.closed||state.paused{return None;}
        let mut frame=state.snapshot.clone()?;frame.initial_snapshot=true;Some(frame)
    }
    pub(crate) fn accepted_snapshot(&self,stamp:FrameStamp){
        let mut state=self.state.lock().unwrap_or_else(|e|e.into_inner());
        let same=state.latest.as_ref().is_some_and(|(frame,_)|frame.stamp.generation==stamp.generation
            &&frame.stamp.format_revision==stamp.format_revision&&frame.stamp.arrival_ts_us==stamp.arrival_ts_us&&frame.stamp.sequence==stamp.sequence);
        let previous=if same{state.delivered=true;state.latest.take()}else{None};drop(state);drop(previous);
    }

    pub fn set_paused(&self, paused: bool) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.paused = paused;
        let mut previous=None;
        if paused {
            previous=state.latest.take();
        }else if self.allow_initial_snapshot {
            // Ingress kept only the current private source lease during pause.
            // Resuming cannot mistake its original age for a new capture.
            if let Some(frame)=state.snapshot.clone(){let received=frame.received_at;previous=state.latest.replace((frame,received));state.delivered=false;}
        }
        drop(state);
        drop(previous);
        if !paused{self.changed.notify_one();}
        self.children.lock().unwrap_or_else(|e|e.into_inner()).retain(|child| {
            let Some(child)=child.upgrade() else {return false;};
            child.set_paused(paused);
            true
        });
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        let previous=(state.latest.take(),state.snapshot.take());
        drop(state);
        drop(previous);
        self.changed.notify_waiters();
        self.children.lock().unwrap_or_else(|e|e.into_inner()).retain(|child| {
            let Some(child)=child.upgrade() else {return false;};
            child.close();
            true
        });
    }

    pub async fn receive(&self) -> Result<OwnedFrame, FrameError> {
        loop {
            let notification = self.changed.notified();
            tokio::pin!(notification);
            notification.as_mut().enable();
            {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if state.closed {
                    return Err(FrameError::Generation);
                }
                if let Some((mut frame, published)) = state.latest.take() {
                    // A damage-only source may remain unchanged between picker
                    // confirmation and the viewer's explicit source switch. Its
                    // latest valid snapshot is available once to that first
                    // consumer, retaining the original arrival/copy timestamp.
                    // Pause, format changes and revocation clear queued pixels.
                    let initial = self.allow_initial_snapshot && !state.delivered;
                    if initial || self.max_age.is_none_or(|age| published.elapsed() <= age) {
                        frame.initial_snapshot = initial;
                        state.delivered = true;
                        return Ok(frame);
                    }
                }
            }
            notification.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_pool_reuses_only_the_last_owner_and_keeps_live_plus_cached_bounded() {
        let budget=FrameBudget::new(32,64).unwrap();
        let make=|value:u8| {
            let bytes=[value;32];
            OwnedFrame::copy_from_budget(format(PixelFormat::Rgba),&[BorrowedPlane {data:&bytes,
                mapping_offset:0,chunk_offset:0,chunk_size:32,stride:16}],stamp(value as u64),Some(&budget)).unwrap()
        };
        let a=make(1);let retained=a.clone();drop(a);
        assert!(budget.recycled.lock().unwrap().is_none());
        let b=make(2);let pointer=b.planes[0].as_ptr();drop(b);
        assert!(budget.recycled.lock().unwrap().is_some());
        let c=make(3);
        assert_eq!(c.planes[0].as_ptr(),pointer);
        assert_eq!(retained.planes[0],[1;32]);assert_eq!(c.planes[0],[3;32]);
        assert!(budget.recycled.lock().unwrap().is_none());
        assert_eq!(budget.used.load(std::sync::atomic::Ordering::Acquire),64);
        drop(retained);drop(c);
        assert_eq!(budget.used.load(std::sync::atomic::Ordering::Acquire),0);
        assert_eq!(budget.recycled.lock().unwrap().as_ref().unwrap().iter().map(Vec::capacity).sum::<usize>(),32);
    }

    #[test]
    fn malformed_input_cannot_consume_the_pool_and_changed_layout_is_fully_rewritten() {
        let budget=FrameBudget::new(64,128).unwrap();
        let bytes=[7;32];let planes=[BorrowedPlane {data:&bytes,mapping_offset:0,chunk_offset:0,chunk_size:32,stride:16}];
        let frame=OwnedFrame::copy_from_budget(format(PixelFormat::Rgba),&planes,stamp(1),Some(&budget)).unwrap();drop(frame);
        assert!(OwnedFrame::copy_from_budget(format(PixelFormat::Rgba),&[],stamp(2),Some(&budget)).is_err());
        assert!(budget.recycled.lock().unwrap().is_some());
        let mut changed=format(PixelFormat::Rgba);changed.width=8;
        let bytes=[9;64];let planes=[BorrowedPlane {data:&bytes,mapping_offset:0,chunk_offset:0,chunk_size:64,stride:32}];
        let frame=OwnedFrame::copy_from_budget(changed,&planes,stamp(3),Some(&budget)).unwrap();
        assert_eq!(frame.planes[0],[9;64]);assert_eq!((frame.width,frame.height),(8,2));
        assert!(budget.recycled.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn shared_viewers_reuse_pixels_and_keep_pause_close_and_epochs_independent() {
        let source=FrameMailbox::new(1).unwrap();
        let a=source.fork().unwrap();
        let b=source.fork().unwrap();
        assert!(source.publish(packed(1)));
        let first=a.receive().await.unwrap();
        let second=b.receive().await.unwrap();
        assert!(std::sync::Arc::ptr_eq(&first.planes,&second.planes));
        a.set_paused(true);
        assert!(!source.is_paused() && !b.is_paused());
        assert!(source.publish(packed(1)));
        assert!(b.receive().await.is_ok());
        assert!(source.set_format_revision(2));
        assert!(!b.matches_input_epoch(0,[first.width,first.height]));
        let mut changed=packed(1);changed.stamp.format_revision=2;
        assert!(source.publish(changed));
        assert_eq!(b.receive().await.unwrap().stamp.format_revision,2);
        a.close();
        assert!(!source.is_closed() && !b.is_closed());
        source.close();
        assert!(b.receive().await.is_err());
    }

    #[tokio::test]
    async fn native_static_snapshots_keep_capture_age_and_resume_with_current_source_pixels() {
        let source=FrameMailbox::native_stream(1,std::time::Duration::from_millis(1)).unwrap();
        let a=source.fork().unwrap();let b=source.fork().unwrap();
        let mut frame=packed(1);frame.received_at=std::time::Instant::now()-std::time::Duration::from_secs(2);
        assert!(source.publish(frame.clone()));let initial=a.receive().await.unwrap();let cached=a.snapshot().unwrap();
        assert!(cached.is_expired(std::time::Duration::from_secs(1)));assert_eq!(cached.stamp.arrival_ts_us,initial.stamp.arrival_ts_us);
        assert!(std::sync::Arc::ptr_eq(&initial.planes,&cached.planes));assert!(cached.initial_snapshot);
        a.set_paused(true);assert!(a.snapshot().is_none());assert!(source.publish(packed(2)));
        assert_eq!(b.snapshot().unwrap().stamp.sequence,Some(2));a.set_paused(false);
        let resumed=a.receive().await.unwrap();assert_eq!(resumed.stamp.sequence,Some(2));assert!(resumed.initial_snapshot);
        a.close();assert!(a.snapshot().is_none());assert!(b.snapshot().is_some());
        source.set_format_revision(2);assert!(b.snapshot().is_none());source.close();assert!(b.snapshot().is_none());
    }

    #[tokio::test]
    async fn process_mailboxes_do_not_retain_pixels_for_native_viewport_replay() {
        let source=FrameMailbox::new(1).unwrap();let frame=packed(1);let pixels=std::sync::Arc::downgrade(&frame.planes);
        assert!(source.publish(frame));drop(source.receive().await.unwrap());assert!(source.snapshot().is_none());assert!(pixels.upgrade().is_none());
    }

    #[test]
    fn bounded_four_k_copy_reserves_before_allocation_and_releases_on_drop() {
        let format = FrameFormat {
            width: 3840,
            height: 2160,
            pixel_format: PixelFormat::Nv12,
            color: SourceColor::default(),
            crop: None,
            transform: Transform::Identity,
        };
        let bytes = vec![0; 12_441_600];
        let mut planes = vec![
            BorrowedPlane {
                data: &bytes,
                mapping_offset: 0,
                chunk_offset: 0,
                chunk_size: 8_294_400,
                stride: 3840,
            },
            BorrowedPlane {
                data: &bytes,
                mapping_offset: 0,
                chunk_offset: 8_294_400,
                chunk_size: 4_147_200,
                stride: 3840,
            },
        ];
        let budget = FrameBudget::new(16 * 1024 * 1024, 32 * 1024 * 1024).unwrap();
        let a = OwnedFrame::copy_from_budget(format, &planes, stamp(1), Some(&budget)).unwrap();
        let b = OwnedFrame::copy_from_budget(format, &planes, stamp(2), Some(&budget)).unwrap();
        assert_eq!(
            budget.used.load(std::sync::atomic::Ordering::Acquire),
            24_883_200
        );
        assert_eq!(
            OwnedFrame::copy_from_budget(format, &planes, stamp(3), Some(&budget)).unwrap_err(),
            FrameError::Budget
        );
        drop(a);
        let c = OwnedFrame::copy_from_budget(format, &planes, stamp(4), Some(&budget)).unwrap();
        drop(b);
        drop(c);
        assert_eq!(budget.used.load(std::sync::atomic::Ordering::Acquire), 0);
        planes[1].stride = 8192;
        assert_eq!(
            OwnedFrame::copy_from_budget(format, &planes, stamp(5), Some(&budget)).unwrap_err(),
            FrameError::Bounds
        );
        assert_eq!(budget.used.load(std::sync::atomic::Ordering::Acquire), 0);
        let small = FrameBudget::new(8 * 1024 * 1024, 16 * 1024 * 1024).unwrap();
        planes[1].stride = 3840;
        assert_eq!(
            OwnedFrame::copy_from_budget(format, &planes, stamp(6), Some(&small)).unwrap_err(),
            FrameError::Budget
        );
    }

    fn format(pixel_format: PixelFormat) -> FrameFormat {
        FrameFormat {
            width: 4,
            height: 2,
            pixel_format,
            color: SourceColor::default(),
            crop: None,
            transform: Transform::Identity,
        }
    }
    fn stamp(sequence: u64) -> FrameStamp {
        FrameStamp {
            generation: 1,
            format_revision: 0,
            sequence: Some(sequence),
            pipewire_pts_ns: None,
            arrival_ts_us: 4321,
        }
    }
    fn packed(sequence: u64) -> OwnedFrame {
        let data = [sequence as u8; 32];
        OwnedFrame::copy_from(
            format(PixelFormat::Rgba),
            &[BorrowedPlane {
                data: &data,
                mapping_offset: 0,
                chunk_offset: 0,
                chunk_size: 32,
                stride: 16,
            }],
            stamp(sequence),
        )
        .unwrap()
    }

    #[test]
    fn packed_padding_offset_and_mapping_origin_are_independent() {
        for pixel_format in [
            PixelFormat::Rgba,
            PixelFormat::Rgbx,
            PixelFormat::Bgra,
            PixelFormat::Bgrx,
        ] {
            let mut bytes = vec![99; 46];
            bytes[3..19].fill(1);
            bytes[23..39].fill(2);
            let frame = OwnedFrame::copy_from(
                format(pixel_format),
                &[BorrowedPlane {
                    data: &bytes,
                    mapping_offset: 4096,
                    chunk_offset: 3,
                    chunk_size: 36,
                    stride: 20,
                }],
                stamp(7),
            )
            .unwrap();
            assert_eq!(frame.planes[0], [vec![1; 16], vec![2; 16]].concat());
            assert_eq!(frame.mapping_offsets, vec![4096]);
            bytes.fill(0);
            assert_eq!(frame.planes[0][16], 2); // No mapping pointer survived.
        }
    }

    #[test]
    fn nv12_and_i420_preserve_separate_chroma_planes() {
        let y = [1, 2, 3, 4, 99, 99, 5, 6, 7, 8];
        let uv = [9, 10, 11, 12];
        let u = [9, 11];
        let v = [10, 12];
        let plane = |data, size, stride| BorrowedPlane {
            data,
            mapping_offset: 0,
            chunk_offset: 0,
            chunk_size: size,
            stride,
        };
        let nv12 = OwnedFrame::copy_from(
            format(PixelFormat::Nv12),
            &[plane(&y, 10, 6), plane(&uv, 4, 4)],
            stamp(1),
        )
        .unwrap();
        assert_eq!(*nv12.planes, vec![vec![1, 2, 3, 4, 5, 6, 7, 8], uv.to_vec()]);
        let i420 = OwnedFrame::copy_from(
            format(PixelFormat::I420),
            &[plane(&y, 10, 6), plane(&u, 2, 2), plane(&v, 2, 2)],
            stamp(1),
        )
        .unwrap();
        assert_eq!(i420.planes[1], u);
        assert_eq!(i420.planes[2], v);
    }

    #[test]
    fn crop_copies_rows_and_preserves_original_format_and_metadata() {
        let mut source = format(PixelFormat::Rgba);
        source.crop = Some(Crop {
            x: 1,
            y: 1,
            width: 2,
            height: 1,
        });
        source.color = SourceColor {
            range: 1,
            matrix: 0,
            transfer: 4,
            primaries: 7,
        };
        let bytes: Vec<_> = (0..32).collect();
        let mut metadata = stamp(9);
        metadata.pipewire_pts_ns = Some(8_000_000_000);
        let frame = OwnedFrame::copy_from(
            source,
            &[BorrowedPlane {
                data: &bytes,
                mapping_offset: 0,
                chunk_offset: 0,
                chunk_size: 32,
                stride: 16,
            }],
            metadata,
        )
        .unwrap();
        assert_eq!((frame.width, frame.height), (2, 1));
        assert_eq!(frame.planes[0], bytes[20..28]);
        assert_eq!(frame.source_format, source);
        assert_eq!(frame.stamp, metadata);
        assert_ne!(
            frame.stamp.arrival_ts_us,
            frame.stamp.pipewire_pts_ns.unwrap() as u64 / 1000
        );
    }

    #[test]
    fn malformed_layouts_never_allocate_a_frame() {
        let data = [0; 32];
        for (offset, size, stride, expected) in [
            (0, 32, -16, FrameError::Stride),
            (0, 32, 15, FrameError::Stride),
            (0, 31, 16, FrameError::Bounds),
            (1, 32, 16, FrameError::Bounds),
            (u32::MAX, u32::MAX, 16, FrameError::Bounds),
            (0, 32, i32::MAX, FrameError::Bounds),
        ] {
            assert_eq!(
                OwnedFrame::copy_from(
                    format(PixelFormat::Bgra),
                    &[BorrowedPlane {
                        data: &data,
                        mapping_offset: 0,
                        chunk_offset: offset,
                        chunk_size: size,
                        stride
                    }],
                    stamp(1)
                )
                .unwrap_err(),
                expected
            );
        }
        assert_eq!(
            OwnedFrame::copy_from(format(PixelFormat::I420), &[], stamp(1)).unwrap_err(),
            FrameError::PlaneCount
        );
    }

    #[test]
    fn unknown_color_is_retained_and_unsupported_geometry_fails_closed() {
        assert_eq!(packed(1).source_format.color, SourceColor::default());
        for (source, expected) in [
            (
                FrameFormat {
                    width: u32::MAX,
                    ..format(PixelFormat::Rgba)
                },
                FrameError::Dimensions,
            ),
            (
                FrameFormat {
                    transform: Transform::Unsupported(1),
                    ..format(PixelFormat::Rgba)
                },
                FrameError::UnsupportedTransform,
            ),
            (
                FrameFormat {
                    crop: Some(Crop {
                        x: u32::MAX,
                        y: 0,
                        width: 2,
                        height: 2,
                    }),
                    ..format(PixelFormat::Nv12)
                },
                FrameError::Crop,
            ),
            (
                FrameFormat {
                    crop: Some(Crop {
                        x: 1,
                        y: 0,
                        width: 2,
                        height: 2,
                    }),
                    ..format(PixelFormat::Nv12)
                },
                FrameError::Crop,
            ),
        ] {
            assert_eq!(
                OwnedFrame::copy_from(source, &[], stamp(1)).unwrap_err(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn mailbox_retains_newest_unsubmitted_frame_and_rejects_old_generation() {
        let mailbox = FrameMailbox::new(1).unwrap();
        assert!(mailbox.publish(packed(1)));
        assert!(mailbox.publish(packed(2)));
        let mut stale = packed(3);
        stale.stamp.generation = 2;
        assert!(!mailbox.publish(stale));
        assert_eq!(mailbox.receive().await.unwrap().stamp.sequence, Some(2));
        mailbox.set_paused(true);
        assert!(!mailbox.publish(packed(4)));
        mailbox.set_paused(false);
        assert!(mailbox.publish(packed(5)));
        assert_eq!(mailbox.receive().await.unwrap().stamp.sequence, Some(5));
        mailbox.close();
        assert!(!mailbox.publish(packed(6)));
        assert_eq!(mailbox.receive().await.unwrap_err(), FrameError::Generation);
    }

    #[tokio::test]
    async fn closing_a_mailbox_wakes_a_waiting_consumer() {
        let mailbox = std::sync::Arc::new(FrameMailbox::new(1).unwrap());
        let consumer = mailbox.clone();
        let task = tokio::spawn(async move { consumer.receive().await });
        tokio::task::yield_now().await;
        mailbox.close();
        let result = tokio::time::timeout(std::time::Duration::from_millis(100), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err(), FrameError::Generation);
    }
}
