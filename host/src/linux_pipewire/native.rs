//! Linux SDK-backed implementation. No default PipeWire socket is opened.
//! All native objects and borrowed buffers live on one dedicated worker thread.
use super::{CopyContract, MappedChunk, WorkerError, validate_chunk_flags};
use crate::linux_frame::{
    Crop, FrameFormat, FrameMailbox, MAX_FRAME_BYTES, OwnedFrame, PixelFormat, SourceColor,
    Transform,
};
use crate::linux_frame::dmabuf::{self, BufferKey};
use crate::linux_portal::runtime::{PortalCallError, PortalCaptureLease, PreparedPortalCapture};
use pipewire as pw;
use pw::{properties::properties, spa};
use spa::buffer::{
    DataFlags, DataType,
    meta::{MetaHeader, MetaHeaderFlags, MetaVideoCrop, MetaVideoTransform},
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::File,
    io::{Cursor, Seek, SeekFrom},
    os::fd::{BorrowedFd, OwnedFd},
    ptr::NonNull,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{oneshot, watch};

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
const RAW_FRAME_MAX_AGE: Duration = Duration::from_millis(250);

/// Raw pointers never leave the originating loop. A removed buffer keeps only
/// its outstanding key, so a late GPU return cannot queue a recycled address.
#[derive(Default)]
struct BorrowPool {
    next: u64,
    slots: BTreeMap<u64, BorrowSlot>,
}
struct BorrowSlot {
    pointer: Option<NonNull<pw::sys::pw_buffer>>,
    held: Option<BufferKey>,
}
impl BorrowPool {
    fn add(&mut self, pointer: NonNull<pw::sys::pw_buffer>) -> Result<(), WorkerError> {
        if self.slots.len() >= 64 || self.slots.values().any(|slot|slot.pointer == Some(pointer)) {
            return Err(WorkerError::Buffer);
        }
        self.next = self.next.checked_add(1).ok_or(WorkerError::Revision)?;
        self.slots.insert(self.next, BorrowSlot {pointer:Some(pointer), held:None});
        Ok(())
    }
    fn remove(&mut self, pointer: NonNull<pw::sys::pw_buffer>) {
        let Some((&id, slot)) = self.slots.iter_mut().find(|(_,slot)|slot.pointer == Some(pointer)) else {return;};
        slot.pointer = None;
        if slot.held.is_none() {self.slots.remove(&id);}
    }
    fn held(&self) -> usize {self.slots.values().filter(|slot|slot.held.is_some()).count()}
    fn borrow(&mut self, pointer: NonNull<pw::sys::pw_buffer>, generation:u64, revision:u64) -> Result<BufferKey,WorkerError> {
        if self.held() >= dmabuf::MAX_BORROWED {return Err(WorkerError::Budget);}
        let (&id,slot) = self.slots.iter_mut().find(|(_,slot)|slot.pointer == Some(pointer))
            .ok_or(WorkerError::Buffer)?;
        if slot.held.is_some() {return Err(WorkerError::Buffer);}
        let key=BufferKey {generation,revision,id};
        slot.held=Some(key);
        Ok(key)
    }
    fn returned(&mut self, key:BufferKey) -> Option<NonNull<pw::sys::pw_buffer>> {
        let slot=self.slots.get_mut(&key.id)?;
        if slot.held != Some(key) {return None;}
        slot.held=None;
        let pointer=slot.pointer;
        if pointer.is_none() {self.slots.remove(&key.id);}
        pointer
    }
}

pub(crate) fn initialize() {
    static INIT:std::sync::Once=std::sync::Once::new();
    INIT.call_once(pw::init);
}

struct Worker {
    stop: Arc<AtomicBool>,
    mailbox: Arc<FrameMailbox>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.mailbox.close();
        // The worker's 20ms loop timer observes closure even without new pixels.
        // Joining never blocks the GUI/async executor. Native fixture acceptance
        // must additionally measure actual disconnect/FD reclamation on Linux.
        if let Some(thread) = self
            .thread
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = std::thread::Builder::new()
                .name("rp-pw-reap".into())
                .spawn(move || {
                    let _ = thread.join();
                });
        }
    }
}

async fn revoked(receiver: &mut watch::Receiver<bool>) {
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

/// Connect only the portal's restricted FD/node, validate the first actual pixel
/// format, then return a connection-scoped candidate. No network commit occurs.
pub async fn prepare_pipewire_capture(
    lease: PortalCaptureLease,
) -> Result<PreparedPortalCapture, PortalCallError> {
    prepare_capture(lease, None).await
}
pub async fn prepare_pipewire_capture_bounded(
    lease: PortalCaptureLease,
    per_frame: usize,
    total_owned: usize,
) -> Result<PreparedPortalCapture, PortalCallError> {
    let budget = crate::linux_frame::FrameBudget::new(per_frame, total_owned)
        .map_err(|_| PortalCallError::InvalidSource)?;
    prepare_capture(lease, Some(budget)).await
}
async fn prepare_capture(
    mut lease: PortalCaptureLease,
    budget: Option<Arc<crate::linux_frame::FrameBudget>>,
) -> Result<PreparedPortalCapture, PortalCallError> {
    let budget=Some(match budget {
        Some(budget)=>budget,
        None=>crate::linux_frame::FrameBudget::new(MAX_FRAME_BYTES,2*MAX_FRAME_BYTES)
            .map_err(|_|PortalCallError::InvalidSource)?,
    });
    let generation = lease.selected_source().generation;
    let serial = lease.selected_source().pipewire_serial;
    let mut revoked_rx = lease.revocation();
    let (node, fd) = lease.take_pipewire_remote()?;
    let mailbox = Arc::new(
        FrameMailbox::native_stream(generation, RAW_FRAME_MAX_AGE)
            .map_err(|_| PortalCallError::Generation)?,
    );
    let stop = Arc::new(AtomicBool::new(false));
    let (first_tx, first_rx) = oneshot::channel();
    let (done_tx, done_rx) = oneshot::channel();
    let worker_mailbox = mailbox.clone();
    let worker_stop = stop.clone();
    let thread = std::thread::Builder::new()
        .name("rp-pipewire".into())
        .spawn(move || {
            if let Err(error) = run(
                fd,
                node,
                serial,
                generation,
                worker_mailbox.clone(),
                worker_stop,
                first_tx,
                budget,
                None,
                None,
            ) {
                eprintln!("WEAVELO_PIPEWIRE_ERROR stage=stream error={error}");
            }
            worker_mailbox.close();
            let _ = done_tx.send(());
        })
        .map_err(|_| PortalCallError::Transport)?;
    let worker = Worker {
        stop,
        mailbox: mailbox.clone(),
        thread: Mutex::new(Some(thread)),
    };
    let first = tokio::select! {
        biased;
        _ = revoked(&mut revoked_rx) => return Err(PortalCallError::Revoked),
        result = tokio::time::timeout(FIRST_FRAME_TIMEOUT, first_rx) => result
            .map_err(|_| PortalCallError::Timeout)?
            .map_err(|_| PortalCallError::InvalidSource)?,
    };
    let prepared = lease.prepare_with_mailbox(first, mailbox)?;
    prepared.attach_worker(Box::new(worker), done_rx);
    Ok(prepared)
}

/// A camera owns a restricted remote and raw worker, without inventing a
/// ScreenCast session or desktop input grant. The existing checked CPU buffer
/// contract is reused; this is not a claim of zero-copy camera capture.
pub(crate) struct CameraCapture {
    pub(crate) mailbox:Arc<FrameMailbox>,
    pub(crate) generation:u64,
    _worker:Worker,
}
pub(crate) async fn prepare_camera_capture(fd:OwnedFd,node:u32,serial:u64,cookie:u32,width:u32,height:u32,fps:u32)
    ->Result<CameraCapture,String> {
    static GENERATION:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
    let generation=GENERATION.fetch_add(1,Ordering::Relaxed);
    let mailbox=Arc::new(FrameMailbox::native_stream(generation,RAW_FRAME_MAX_AGE).map_err(|e|format!("{e:?}"))?);
    let stop=Arc::new(AtomicBool::new(false));let (first,received)=oneshot::channel();
    let frame_budget=(u64::from(width)*u64::from(height)*4).max(16*1024*1024)
        .min(crate::linux_frame::MAX_FRAME_BYTES as u64) as usize;
    let budget=crate::linux_frame::FrameBudget::new(frame_budget,frame_budget*2)
        .map_err(|e|format!("Camera buffer budget: {e:?}"))?;
    let (frames,cancel)=(mailbox.clone(),stop.clone());
    let thread=std::thread::Builder::new().name("weavelo-camera".into()).spawn(move|| {
        if let Err(error)=run(fd,node,Some(serial),generation,frames.clone(),cancel,first,Some(budget),Some(cookie),Some((width,height,fps))) {
            eprintln!("WEAVELO_PIPEWIRE_ERROR stage=camera error={error}");
        }
        frames.close();
    }).map_err(|e|e.to_string())?;
    let worker=Worker{stop,mailbox:mailbox.clone(),thread:Mutex::new(Some(thread))};
    let first=tokio::time::timeout(FIRST_FRAME_TIMEOUT,received).await
        .map_err(|_|"Camera did not produce a native frame".to_string())?
        .map_err(|_|"Camera stopped; check availability and its native pixel/color format".to_string())?;
    mailbox.publish(first);
    Ok(CameraCapture{mailbox,generation,_worker:worker})
}

struct Data {
    contract: CopyContract,
    first: Option<oneshot::Sender<OwnedFrame>>,
    loop_: pw::main_loop::MainLoopRc,
    mailbox: Arc<FrameMailbox>,
    connected: bool,
    modifier: Option<u64>,
    pool: Rc<RefCell<BorrowPool>>,
    returned: pw::channel::Sender<BufferKey>,
}
impl Data {
    fn fail(&mut self) {
        self.mailbox.close();
        self.first.take();
        self.loop_.quit();
    }
    fn deliver(&mut self,result:Result<OwnedFrame,WorkerError>) {
        match result {
            Ok(frame)=> {
                if let Some(first)=self.first.take() {
                    if first.send(frame).is_err() {self.fail();}
                } else {self.mailbox.publish(frame);}
            }
            Err(WorkerError::Budget)=>{},
            Err(error)=> {
                eprintln!("WEAVELO_PIPEWIRE_ERROR stage=frame error={error:?}");
                self.fail();
            }
        }
    }
}

// Like pipewire::Buffer::find_meta, but only used while our originating worker
// owns the raw dequeued buffer. No metadata or native pointer escapes this call.
unsafe fn raw_meta<T:spa::buffer::meta::Metadata>(buffer:&spa::sys::spa_buffer)->Option<&T> {
    let pointer=unsafe {spa::sys::spa_buffer_find_meta_data(buffer,T::META_TYPE,std::mem::size_of::<T>())};
    unsafe {(pointer as *const T).as_ref()}
}

fn process_dmabuf(stream:&pw::stream::Stream,data:&mut Data) {
    let Some(pointer)=NonNull::new(unsafe {stream.dequeue_raw_buffer()}) else {return;};
    if data.mailbox.is_closed() || data.mailbox.is_paused() || !data.contract.is_negotiated() {
        unsafe {stream.queue_raw_buffer(pointer.as_ptr());} return;
    }
    let Some(buffer)=(unsafe {pointer.as_ref().buffer.as_ref()}) else {
        unsafe {stream.queue_raw_buffer(pointer.as_ptr());} data.fail();return;
    };
    let header=unsafe {raw_meta::<MetaHeader>(buffer)};
    if header.is_some_and(|header|header.flags().intersects(MetaHeaderFlags::CORRUPTED|MetaHeaderFlags::GAP)) {
        unsafe {stream.queue_raw_buffer(pointer.as_ptr());} return;
    }
    if header.is_some_and(|header|header.flags().contains(MetaHeaderFlags::DISCONT))
        && data.contract.discontinuity().is_err() {
        unsafe {stream.queue_raw_buffer(pointer.as_ptr());} data.fail();return;
    }
    let lease=data.pool.borrow_mut().borrow(pointer,data.contract.generation,data.contract.revision());
    let key=match lease {
        Ok(key)=>key,
        Err(error)=> {
            unsafe {stream.queue_raw_buffer(pointer.as_ptr());}
            if error != WorkerError::Budget {data.fail();}
            return;
        }
    };
    let returned=data.returned.clone();
    // Install the exactly-once return before any fallible FD/layout work.
    let mut gpu=dmabuf::DmaBufFrame::new(dmabuf::DRM_ARGB8888,Vec::new(),Vec::new(),key,
        move|key|{let _=returned.send(key);});
    let result=(|| {
        if buffer.n_datas == 0 || buffer.n_datas as usize > dmabuf::MAX_PLANES || buffer.datas.is_null() {
            return Err(WorkerError::Buffer);
        }
        let planes=unsafe {std::slice::from_raw_parts(buffer.datas,buffer.n_datas as usize)};
        let mut objects=BTreeMap::new();
        for plane in planes {
            if plane.type_ != DataType::DmaBuf.as_raw() || plane.flags & DataFlags::READABLE.bits() == 0
                || plane.chunk.is_null() {return Err(WorkerError::Buffer);}
            let chunk=unsafe {&*plane.chunk};
            validate_chunk_flags(chunk.flags)?;
            if chunk.size == 0 || chunk.stride <= 0 {return Err(WorkerError::Buffer);}
            let fd=i32::try_from(plane.fd).ok().filter(|fd|*fd>=0).ok_or(WorkerError::Buffer)?;
            let object=if let Some(index)=objects.get(&fd) {*index} else {
                // A dup retains storage; the worker lease, not the dup, prevents
                // KWin from overwriting it while VAAPI reads the frame.
                let owned=unsafe {BorrowedFd::borrow_raw(fd)}.try_clone_to_owned().map_err(|_|WorkerError::Buffer)?;
                let mut file=File::from(owned);
                let size=file.seek(SeekFrom::End(0)).map_err(|_|WorkerError::Buffer)?;
                let index=gpu.objects.len() as u32;
                gpu.objects.push(dmabuf::Object {fd:file,size,modifier:data.modifier.ok_or(WorkerError::Format)?});
                objects.insert(fd,index);index
            };
            gpu.planes.push(dmabuf::Plane {object,
                offset:plane.mapoffset.checked_add(chunk.offset).ok_or(WorkerError::Buffer)?,
                stride:chunk.stride as u32});
        }
        let crop=unsafe {raw_meta::<MetaVideoCrop>(buffer)}.filter(|crop|crop.meta_region().is_valid())
            .map(|crop| {let region=crop.meta_region();Crop {x:region.position().x as u32,
                y:region.position().y as u32,width:region.size().width,height:region.size().height}});
        let transform=unsafe {raw_meta::<MetaVideoTransform>(buffer)}.map_or(Transform::Identity,|meta| {
            match meta.transform().as_raw() {0=>Transform::Identity,value=>Transform::Unsupported(value)}
        });
        data.contract.import_dmabuf(gpu,crop,transform,header.map(|header|header.seq()),
            header.and_then(|header|(header.pts()!=i64::MIN).then_some(header.pts())),remote_core::quanta_now_us())
    })();
    data.deliver(result);
}

fn format(raw: spa::param::video::VideoInfoRaw) -> Result<FrameFormat, WorkerError> {
    let pixel_format = match raw.format() {
        spa::param::video::VideoFormat::NV12 => PixelFormat::Nv12,
        spa::param::video::VideoFormat::I420 => PixelFormat::I420,
        spa::param::video::VideoFormat::BGRA => PixelFormat::Bgra,
        spa::param::video::VideoFormat::BGRx => PixelFormat::Bgrx,
        spa::param::video::VideoFormat::RGBA => PixelFormat::Rgba,
        spa::param::video::VideoFormat::RGBx => PixelFormat::Rgbx,
        _ => return Err(WorkerError::Format),
    };
    if raw.interlace_mode() != spa::param::video::VideoInterlaceMode::Progressive
        || raw.views() > 1
        || (matches!(pixel_format,PixelFormat::Nv12|PixelFormat::I420) && raw.as_raw().chroma_site != 0)
    {
        return Err(WorkerError::Format);
    }
    let raw = raw.as_raw();
    Ok(FrameFormat {
        width: raw.size.width,
        height: raw.size.height,
        pixel_format,
        color: SourceColor {
            range: raw.color_range,
            matrix: raw.color_matrix,
            transfer: raw.transfer_function,
            primaries: raw.color_primaries,
        },
        crop: None,
        transform: Transform::Identity,
    })
}

fn run(
    fd: OwnedFd,
    node: u32,
    serial: Option<u64>,
    generation: u64,
    mailbox: Arc<FrameMailbox>,
    stop: Arc<AtomicBool>,
    first: oneshot::Sender<OwnedFrame>,
    budget: Option<Arc<crate::linux_frame::FrameBudget>>,
    server_cookie:Option<u32>,
    camera_mode:Option<(u32,u32,u32)>,
) -> Result<(), Box<dyn std::error::Error>> {
    initialize();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_fd_rc(fd, None)?;
    let error_loop = mainloop.clone();
    let error_mailbox = mailbox.clone();
    let identity_loop=mainloop.clone();let identity_mailbox=mailbox.clone();
    let _core_listener = core
        .add_listener_local()
        .info(move|info| {
            if server_cookie.is_some_and(|expected|expected!=info.cookie()) {
                identity_mailbox.close();identity_loop.quit();
            }
        })
        .error(move |_, _, _, _| {
            error_mailbox.close();
            error_loop.quit();
        })
        .register();
    let mut properties = properties! {
        *pw::keys::MEDIA_TYPE => "Video", *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => if camera_mode.is_some() {"Camera"} else {"Screen"},
    };
    if let Some(serial) = serial {
        properties.insert("target.object", serial.to_string());
    }
    let stream = pw::stream::StreamRc::new(core.clone(), "Weavelo local share", properties)?;
    let pool=Rc::new(RefCell::new(BorrowPool::default()));
    let (returned, receiver)=pw::channel::channel::<BufferKey>();
    let return_pool=pool.clone();let return_stream=stream.clone();
    let _returns=receiver.attach(mainloop.loop_(),move|key| {
        let pointer=return_pool.borrow_mut().returned(key);
        if let Some(pointer)=pointer {
            // Only this loop can return this registration, after the final
            // encoder reference and GPU completion have released its lease.
            unsafe {return_stream.queue_raw_buffer(pointer.as_ptr());}
        }
    });
    #[cfg(feature="native-linux-video")]
    let capture_modifier=if camera_mode.is_none() {
        match crate::linux_raw_encode::vaapi::HardwareEncoder::capture_modifier() {
            Ok(modifier)=>Some(modifier),
            Err(error)=>{eprintln!("WEAVELO_PIPEWIRE_STORAGE dma_buf=unavailable reason={error}");None}
        }
    } else {None};
    #[cfg(not(feature="native-linux-video"))]
    let capture_modifier:Option<u64>=None;
    let _listener = stream
        .add_local_listener_with_user_data(Data {
            contract: CopyContract::new(generation, mailbox.clone()).with_budget(budget),
            first: Some(first),
            loop_: mainloop.clone(),
            mailbox: mailbox.clone(),
            connected: false,
            modifier: None,
            pool: pool.clone(),
            returned,
        })
        .add_buffer(|_,data,pointer| {
            let result=NonNull::new(pointer).ok_or(WorkerError::Buffer)
                .and_then(|pointer|data.pool.borrow_mut().add(pointer));
            if result.is_err() {data.fail();}
        })
        .remove_buffer(|_,data,pointer| {
            if let Some(pointer)=NonNull::new(pointer) {data.pool.borrow_mut().remove(pointer);}
        })
        .state_changed(|_, data, _, new| {
            if matches!(new, pw::stream::StreamState::Error(_))
                || (data.connected && matches!(new, pw::stream::StreamState::Unconnected))
            {
                data.fail();
            }
            if matches!(
                new,
                pw::stream::StreamState::Connecting
                    | pw::stream::StreamState::Streaming
                    | pw::stream::StreamState::Paused
            ) {
                data.connected = true;
            }
        })
        .param_changed(move |stream, data, id, param| {
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let result = (|| {
                let Some(param) = param else {
                    data.modifier=None;
                    return data.contract.renegotiate(None).map(|_| ());
                };
                let (media, subtype) = spa::param::format_utils::parse_format(param)
                    .map_err(|_| WorkerError::Format)?;
                if media != spa::param::format::MediaType::Video
                    || subtype != spa::param::format::MediaSubtype::Raw
                {
                    return Err(WorkerError::Format);
                }
                let mut raw = spa::param::video::VideoInfoRaw::default();
                raw.parse(param).map_err(|_| WorkerError::Format)?;
                data.modifier=raw.flags().contains(spa::param::video::VideoFlags::MODIFIER).then_some(raw.modifier());
                if data.modifier.is_some() && (data.modifier != capture_modifier
                    || raw.format() != spa::param::video::VideoFormat::BGRA) {
                    return Err(WorkerError::Format);
                }
                let metadata = raw.as_raw();
                eprintln!("WEAVELO_PIPEWIRE_FORMAT width={} height={} format={} range={} matrix={} transfer={} primaries={} chroma={} flags={}",
                    metadata.size.width, metadata.size.height, metadata.format, metadata.color_range,
                    metadata.color_matrix, metadata.transfer_function, metadata.color_primaries,
                    metadata.chroma_site, metadata.flags);
                data.contract.renegotiate(Some(format(raw)?))?;
                eprintln!("WEAVELO_PIPEWIRE_STORAGE dma_buf={} modifier={:?}",data.modifier.is_some(),data.modifier);
                request_buffers(stream,data.modifier.is_some()).map_err(|_| WorkerError::Buffer)
            })();
            if let Err(error) = result {
                eprintln!("WEAVELO_PIPEWIRE_ERROR stage=negotiate error={error:?}");
                data.fail();
            }
        })
        .process(|stream, data| {
            if data.modifier.is_some() {
                process_dmabuf(stream,data);
                return;
            }
            // Buffer::drop queues exactly once on every return/error path. Never hold
            // a PipeWire buffer during mailbox wait, FFmpeg write or GUI rendering.
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            if data.mailbox.is_closed()
                || data.mailbox.is_paused()
                || !data.contract.is_negotiated()
            {
                return;
            }
            let header = buffer.find_meta::<MetaHeader>();
            if header.is_some_and(|header| {
                header
                    .flags()
                    .intersects(MetaHeaderFlags::CORRUPTED | MetaHeaderFlags::GAP)
            }) {
                return;
            }
            let discontinuity =
                header.is_some_and(|header| header.flags().contains(MetaHeaderFlags::DISCONT));
            let sequence = header.map(|header| header.seq());
            let pts = header.and_then(|header| (header.pts() != i64::MIN).then_some(header.pts()));
            let crop = buffer
                .find_meta::<MetaVideoCrop>()
                .filter(|crop| crop.meta_region().is_valid())
                .map(|crop| {
                    let region = crop.meta_region();
                    Crop {
                        x: region.position().x as u32,
                        y: region.position().y as u32,
                        width: region.size().width,
                        height: region.size().height,
                    }
                });
            let transform =
                buffer
                    .find_meta::<MetaVideoTransform>()
                    .map_or(Transform::Identity, |meta| {
                        match meta.transform().as_raw() {
                            0 => Transform::Identity,
                            value => Transform::Unsupported(value),
                        }
                    });
            let result = (|| {
                if discontinuity {
                    data.contract.discontinuity()?;
                }
                let datas = buffer.datas_mut();
                if datas.is_empty() || datas.len() > 3 {
                    return Err(WorkerError::Buffer);
                }
                let mut chunks = Vec::with_capacity(datas.len());
                for data in datas {
                    let raw = data.as_raw();
                    if !matches!(data.type_(), DataType::MemPtr | DataType::MemFd)
                        || !data.flags().contains(DataFlags::READABLE)
                        || raw.data.is_null()
                        || raw.chunk.is_null()
                        || raw.maxsize as usize > MAX_FRAME_BYTES
                    {
                        return Err(WorkerError::Buffer);
                    }
                    let chunk = data.chunk();
                    // Reject neutral/recycled storage before forming a byte
                    // slice, even when size and stride look like a real frame.
                    let flags = chunk.flags().bits();
                    validate_chunk_flags(flags)?;
                    // MAP_BUFFERS supplies data.data at the mapping's mapoffset.
                    // Read-only slices allow legitimate overlapping plane mappings;
                    // no unchecked offset is added and no native pointer escapes.
                    let bytes = unsafe {
                        std::slice::from_raw_parts(raw.data.cast::<u8>(), raw.maxsize as usize)
                    };
                    chunks.push(MappedChunk {
                        bytes,
                        flags,
                        mapping_offset: raw.mapoffset,
                        offset: chunk.offset(),
                        size: chunk.size(),
                        stride: chunk.stride(),
                    });
                }
                data.contract.copy(
                    data.contract.revision(),
                    &chunks,
                    crop,
                    transform,
                    sequence,
                    pts,
                    remote_core::quanta_now_us(),
                )
            })();
            // Return buffer before handing owned bytes across the thread boundary.
            drop(buffer);
            data.deliver(result);
        })
        .register()?;
    let mut object = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            spa::param::format::MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            spa::param::format::MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::NV12,
            spa::param::video::VideoFormat::NV12,
            spa::param::video::VideoFormat::I420
        )
    );
    // KWin's mapped-memory stream exposes packed RGB, not planar YUV.
    // Negotiate its color contract explicitly; never fill unknown metadata
    // after capture or infer a YUV matrix from the screen dimensions.
    let mut rgb=spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,spa::param::ParamType::EnumFormat,
        spa::pod::property!(spa::param::format::FormatProperties::MediaType,Id,spa::param::format::MediaType::Video),
        spa::pod::property!(spa::param::format::FormatProperties::MediaSubtype,Id,spa::param::format::MediaSubtype::Raw),
        spa::pod::property!(spa::param::format::FormatProperties::VideoFormat,Choice,Enum,Id,
            spa::param::video::VideoFormat::BGRA,spa::param::video::VideoFormat::BGRA,
            spa::param::video::VideoFormat::BGRx,spa::param::video::VideoFormat::RGBA,spa::param::video::VideoFormat::RGBx)
    );
    for (key,value) in [
        (spa::param::format::FormatProperties::VideoColorRange,1), // full-range packed RGB
        (spa::param::format::FormatProperties::VideoColorMatrix,1), // RGB
        (spa::param::format::FormatProperties::VideoTransferFunction,7), // sRGB
        (spa::param::format::FormatProperties::VideoColorPrimaries,1), // BT.709
    ] {rgb.properties.push(spa::pod::property!(key,spa::pod::Value::Id(spa::utils::Id(value))));}
    if let Some((width,height,fps))=camera_mode {
        for format in [&mut object,&mut rgb] {
            format.properties.push(spa::pod::property!(spa::param::format::FormatProperties::VideoSize,
                Choice,Range,Rectangle,spa::utils::Rectangle{width,height},
                spa::utils::Rectangle{width:16,height:16},spa::utils::Rectangle{width:8192,height:8192}));
            format.properties.push(spa::pod::property!(spa::param::format::FormatProperties::VideoFramerate,
                Choice,Range,Fraction,spa::utils::Fraction{num:fps,denom:1},
                spa::utils::Fraction{num:1,denom:1},spa::utils::Fraction{num:fps,denom:1}));
        }
    }
    let mut objects=Vec::new();
    if let Some(modifier)=capture_modifier {
        let mut dma=rgb.clone();
        dma.properties.retain(|property|property.key != spa::param::format::FormatProperties::VideoFormat.as_raw());
        dma.properties.push(spa::pod::property!(spa::param::format::FormatProperties::VideoFormat,Id,spa::param::video::VideoFormat::BGRA));
        let mut property=spa::pod::property!(spa::param::format::FormatProperties::VideoModifier,spa::pod::Value::Long(modifier as i64));
        property.flags=spa::pod::PropertyFlags::MANDATORY;
        dma.properties.push(property);objects.push(dma);
    }
    objects.extend([object,rgb]);
    let formats=objects.into_iter().map(|format| {
        spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()),&spa::pod::Value::Object(format))
            .map(|result|result.0.into_inner())
    }).collect::<Result<Vec<_>,_>>()?;
    let mut pods=formats.iter().map(|bytes|spa::pod::Pod::from_bytes(bytes).ok_or("invalid format pod"))
        .collect::<Result<Vec<_>,_>>()?;
    stream.connect(
        spa::utils::Direction::Input,
        if serial.is_some() { None } else { Some(node) },
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut pods,
    )?;
    let timer_loop = mainloop.clone();
    let timer_mailbox=mailbox.clone();
    let timer = mainloop.loop_().add_timer(move |_| {
        if stop.load(Ordering::Acquire) || timer_mailbox.is_closed() {
            timer_loop.quit();
        }
    });
    timer
        .update_timer(
            Some(Duration::from_millis(1)),
            Some(Duration::from_millis(20)),
        )
        .into_result()?;
    mainloop.run();
    mailbox.close();
    let _=stream.set_active(false);
    // Keep the stream, registrations and return source alive until every GPU
    // consumer has finished. Duplicating an FD alone does not prevent reuse.
    while pool.borrow().held() != 0 {
        mainloop.loop_().iterate(pw::loop_::Timeout::Finite(Duration::from_millis(20)));
    }
    let _ = stream.disconnect();
    Ok(())
}

fn request_buffers(stream: &pw::stream::Stream, dmabuf:bool) -> Result<(), Box<dyn std::error::Error>> {
    use spa::pod::{ChoiceValue, Object, Property, Value};
    use spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Id};
    let mask = if dmabuf {1u32 << DataType::DmaBuf.as_raw()}
        else {(1u32 << DataType::MemPtr.as_raw()) | (1u32 << DataType::MemFd.as_raw())} as i32;
    let mut objects = vec![Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: spa::sys::SPA_PARAM_Buffers,
        properties: vec![Property::new(
            spa::sys::SPA_PARAM_BUFFERS_dataType,
            Value::Choice(ChoiceValue::Int(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Flags {
                    default: mask,
                    flags: vec![mask],
                },
            ))),
        )],
    }];
    if dmabuf {
        objects[0].properties.push(Property::new(spa::sys::SPA_PARAM_BUFFERS_buffers,
            Value::Choice(ChoiceValue::Int(Choice(ChoiceFlags::empty(),
                ChoiceEnum::Range {default:8,min:4,max:8})))));
    }
    for (kind, size) in [
        (
            spa::sys::SPA_META_Header,
            std::mem::size_of::<spa::sys::spa_meta_header>(),
        ),
        (
            spa::sys::SPA_META_VideoCrop,
            std::mem::size_of::<spa::sys::spa_meta_region>(),
        ),
        (
            spa::sys::SPA_META_VideoTransform,
            std::mem::size_of::<spa::sys::spa_meta_videotransform>(),
        ),
    ] {
        objects.push(Object {
            type_: spa::sys::SPA_TYPE_OBJECT_ParamMeta,
            id: spa::sys::SPA_PARAM_Meta,
            properties: vec![
                Property::new(spa::sys::SPA_PARAM_META_type, Value::Id(Id(kind))),
                Property::new(spa::sys::SPA_PARAM_META_size, Value::Int(size as i32)),
            ],
        });
    }
    let bytes = objects
        .into_iter()
        .map(|object| {
            spa::pod::serialize::PodSerializer::serialize(
                Cursor::new(Vec::new()),
                &Value::Object(object),
            )
            .map(|result| result.0.into_inner())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut pods = bytes
        .iter()
        .map(|bytes| spa::pod::Pod::from_bytes(bytes).ok_or("invalid buffers pod"))
        .collect::<Result<Vec<_>, _>>()?;
    stream.update_params(&mut pods)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pointer(id:usize)->NonNull<pw::sys::pw_buffer> {
        NonNull::new((id*4096) as *mut pw::sys::pw_buffer).unwrap()
    }
    #[test]
    fn removed_buffer_return_cannot_queue_a_reused_native_address() {
        let mut pool=BorrowPool::default();let address=pointer(1);
        pool.add(address).unwrap();let old=pool.borrow(address,7,1).unwrap();
        pool.remove(address);pool.add(address).unwrap();
        let new=pool.borrow(address,7,2).unwrap();
        assert_ne!(old.id,new.id);assert_eq!(pool.held(),2);
        assert!(pool.returned(old).is_none());assert_eq!(pool.held(),1);
        assert!(pool.returned(old).is_none());
        assert!(pool.returned(BufferKey {generation:8,..new}).is_none());
        assert!(pool.returned(BufferKey {revision:1,..new}).is_none());
        assert_eq!(pool.held(),1);assert_eq!(pool.returned(new),Some(address));
        assert_eq!(pool.held(),0);assert!(pool.returned(new).is_none());
    }
    #[test]
    fn gpu_leases_are_bounded_and_only_final_returns_reopen_capacity() {
        let mut pool=BorrowPool::default();
        for id in 1..=5 {pool.add(pointer(id)).unwrap();}
        let leases=(1..=4).map(|id|pool.borrow(pointer(id),2,3).unwrap()).collect::<Vec<_>>();
        assert_eq!(pool.borrow(pointer(5),2,3),Err(WorkerError::Budget));
        assert_eq!(pool.returned(leases[0]),Some(pointer(1)));
        let final_lease=pool.borrow(pointer(5),2,3).unwrap();
        for key in leases.into_iter().skip(1).chain([final_lease]) {assert!(pool.returned(key).is_some());}
        assert_eq!(pool.held(),0);
        assert!(pool.borrow(pointer(99),2,3).is_err());
    }
}
