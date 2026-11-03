//! Checked DRM descriptors and one last-owner return to the originating worker.
//! A duplicated FD keeps storage alive; the return lease also prevents reuse.
use super::{FrameError, FrameFormat, MAX_FRAME_BYTES, PixelFormat};
use std::{fmt, fs::File};

pub(crate) const MAX_PLANES: usize = 4;
pub(crate) const MAX_BORROWED: usize = 4;
pub(crate) const DRM_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
pub(crate) const DRM_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
pub(crate) const DRM_ABGR8888: u32 = u32::from_le_bytes(*b"AB24");
pub(crate) const DRM_XBGR8888: u32 = u32::from_le_bytes(*b"XB24");
pub(crate) const DRM_NV12: u32 = u32::from_le_bytes(*b"NV12");
pub(crate) const DRM_YUV420: u32 = u32::from_le_bytes(*b"YU12");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BufferKey {
    pub generation: u64,
    pub revision: u64,
    pub id: u64,
}
pub(crate) struct Object {
    pub fd: File,
    pub size: u64,
    pub modifier: u64,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plane {
    pub object: u32,
    pub offset: u32,
    pub stride: u32,
}

pub(crate) struct DmaBufFrame {
    pub fourcc: u32,
    pub objects: Vec<Object>,
    pub planes: Vec<Plane>,
    pub key: BufferKey,
    returned: Option<Box<dyn FnOnce(BufferKey) + Send + Sync>>,
}
impl fmt::Debug for DmaBufFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DmaBufFrame").field("fourcc", &self.fourcc)
            .field("planes", &self.planes).field("key", &self.key).finish_non_exhaustive()
    }
}
impl Drop for DmaBufFrame {
    fn drop(&mut self) {
        if let Some(returned) = self.returned.take() { returned(self.key); }
    }
}
impl DmaBufFrame {
    pub(crate) fn new(fourcc: u32, objects: Vec<Object>, planes: Vec<Plane>, key: BufferKey,
        returned: impl FnOnce(BufferKey) + Send + Sync + 'static) -> Self {
        Self { fourcc, objects, planes, key, returned: Some(Box::new(returned)) }
    }
    pub(crate) fn validate(&self, format: FrameFormat) -> Result<(), FrameError> {
        let expected = match format.pixel_format {
            PixelFormat::Bgra => DRM_ARGB8888, PixelFormat::Bgrx => DRM_XRGB8888,
            PixelFormat::Rgba => DRM_ABGR8888, PixelFormat::Rgbx => DRM_XBGR8888,
            PixelFormat::Nv12 => DRM_NV12, PixelFormat::I420 => DRM_YUV420,
        };
        if self.fourcc != expected || self.objects.is_empty() || self.objects.len() > MAX_PLANES
            || self.planes.is_empty() || self.planes.len() > MAX_PLANES {
            return Err(FrameError::PlaneCount);
        }
        let mut total = 0u64;
        for object in &self.objects {
            if object.size == 0 || object.size > MAX_FRAME_BYTES as u64 {
                return Err(FrameError::Budget);
            }
            total = total.checked_add(object.size).ok_or(FrameError::Budget)?;
        }
        if total > MAX_FRAME_BYTES as u64 { return Err(FrameError::Budget); }
        for plane in &self.planes {
            let object = self.objects.get(plane.object as usize).ok_or(FrameError::PlaneCount)?;
            if plane.stride == 0 || plane.stride > i32::MAX as u32
                || u64::from(plane.offset) >= object.size {
                return Err(FrameError::Bounds);
            }
        }
        // Nonlinear modifiers can contain auxiliary planes; their layout belongs
        // to DRM/VAAPI, never to the CPU row-copy adapter. Check exact linear rows.
        if self.objects.iter().all(|object| object.modifier == 0) {
            let rows = match format.pixel_format {
                PixelFormat::Nv12 => vec![(format.width,format.height),(format.width,format.height/2)],
                PixelFormat::I420 => vec![(format.width,format.height),(format.width/2,format.height/2),(format.width/2,format.height/2)],
                _ => vec![(format.width.checked_mul(4).ok_or(FrameError::Bounds)?,format.height)],
            };
            if rows.len() != self.planes.len() { return Err(FrameError::PlaneCount); }
            for (plane,(row_bytes,rows)) in self.planes.iter().zip(rows) {
                let end = u64::from(plane.offset).checked_add(u64::from(plane.stride)
                    .checked_mul(u64::from(rows.saturating_sub(1))).ok_or(FrameError::Bounds)?)
                    .and_then(|offset|offset.checked_add(u64::from(row_bytes))).ok_or(FrameError::Bounds)?;
                if plane.stride < row_bytes || end > self.objects[plane.object as usize].size {
                    return Err(FrameError::Bounds);
                }
            }
        }
        if self.key.generation == 0 || self.key.revision == 0 || self.key.id == 0 {
            return Err(FrameError::Generation);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc,Mutex};
    fn fixture(returned: Arc<Mutex<Vec<BufferKey>>>) -> DmaBufFrame {
        // This model only owns the FD; the real DRM import is a hardware test.
        let file = File::open(std::env::current_exe().unwrap()).unwrap();
        DmaBufFrame::new(DRM_ARGB8888,vec![Object {fd:file,size:128,modifier:0}],
            vec![Plane {object:0,offset:0,stride:32}],BufferKey {generation:7,revision:3,id:11},
            move |key|returned.lock().unwrap().push(key))
    }
    fn format() -> FrameFormat {
        FrameFormat {width:8,height:4,pixel_format:PixelFormat::Bgra,
            color:Default::default(),crop:None,transform:super::super::Transform::Identity}
    }
    #[test]
    fn dma_lease_returns_once_after_every_alias_and_preserves_origin_epoch() {
        let returned = Arc::new(Mutex::new(Vec::new()));
        let frame = Arc::new(fixture(returned.clone())); let other = frame.clone();
        frame.validate(format()).unwrap();drop(frame);assert!(returned.lock().unwrap().is_empty());
        drop(other);assert_eq!(*returned.lock().unwrap(),vec![BufferKey {generation:7,revision:3,id:11}]);
    }
    #[test]
    fn dma_layout_rejects_overruns_wrong_formats_and_invalid_objects() {
        let mut frame = fixture(Default::default());
        frame.planes[0].offset=1;assert_eq!(frame.validate(format()),Err(FrameError::Bounds));
        frame.planes[0].offset=0;frame.planes[0].object=1;
        assert_eq!(frame.validate(format()),Err(FrameError::PlaneCount));
        frame.planes[0].object=0;frame.fourcc=DRM_NV12;
        assert_eq!(frame.validate(format()),Err(FrameError::PlaneCount));
    }
    #[test]
    fn shared_mailboxes_keep_gpu_storage_until_all_viewers_and_source_close() {
        let returned=Arc::new(Mutex::new(Vec::new()));
        let source=super::super::FrameMailbox::new(7).unwrap();
        assert!(source.set_format_revision(3));
        let first=source.fork().unwrap();let second=source.fork().unwrap();
        let frame=super::super::OwnedFrame::from_dmabuf(format(),fixture(returned.clone()),
            super::super::FrameStamp {generation:7,format_revision:3,sequence:Some(1),
                pipewire_pts_ns:None,arrival_ts_us:1}).unwrap();
        assert!(source.publish(frame));first.close();assert!(returned.lock().unwrap().is_empty());
        source.close();assert!(second.is_closed());assert_eq!(returned.lock().unwrap().len(),1);
    }
}
