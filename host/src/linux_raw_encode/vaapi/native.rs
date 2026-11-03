//! One latest raw frame, one synchronous GPU encode, ordered compressed output.
//! The driver sees borrowed input only during upload; encoded packets need no
//! following frame or process EOF before they can be delivered.
use super::HardwareEncoder;
use super::super::{Input, NativeEncodeTiming, RawChunk, RawSettings, Result, validate_native_input};
use crate::linux_frame::{FrameFormat, OwnedFrame, PixelFormat};
use std::{ffi::{CString, c_char, c_int, c_void}, sync::{Arc, atomic::{AtomicBool, Ordering}}, thread::JoinHandle};
use tokio::sync::{Mutex, mpsc};
use std::os::fd::AsRawFd;

#[repr(C)]
#[derive(Clone,Copy,Default)]
struct DrmObject {fd:c_int,size:u64,modifier:u64}
#[repr(C)]
#[derive(Clone,Copy,Default)]
struct DrmPlane {object:u32,offset:u32,stride:u32}
#[repr(C)]
#[derive(Default)]
struct DrmFrame {fourcc:u32,objects:u32,planes:u32,object:[DrmObject;4],plane:[DrmPlane;4]}
extern "C" fn release_dma_lease(lease:*mut c_void) {
    if !lease.is_null() {
        drop(unsafe {Box::from_raw(lease.cast::<Arc<crate::linux_frame::dmabuf::DmaBufFrame>>())});
    }
}

#[repr(C)]
#[derive(Default)]
struct Encoded {
    data: *const u8, bytes: usize, pts: i64,
    upload_vpp_us: u64, encode_us: u64, key: c_int,
}
unsafe extern "C" {
    fn weavelo_encoder_create(device:*const c_char,pixel:*const c_char,width:c_int,height:c_int,
        output_width:c_int,output_height:c_int,fps:c_int,bitrate:c_int,range:c_int,matrix:c_int,
        transfer:c_int,primaries:c_int,out:*mut *mut c_void)->c_int;
    fn weavelo_encoder_frame(encoder:*mut c_void,data:*const *const u8,strides:*const c_int,
        pts:i64,out:*mut Encoded)->c_int;
    fn weavelo_encoder_create_drm(device:*const c_char,pixel:*const c_char,width:c_int,height:c_int,
        crop_x:c_int,crop_y:c_int,crop_width:c_int,crop_height:c_int,
        output_width:c_int,output_height:c_int,fps:c_int,bitrate:c_int,range:c_int,matrix:c_int,
        transfer:c_int,primaries:c_int,out:*mut *mut c_void)->c_int;
    fn weavelo_encoder_frame_drm(encoder:*mut c_void,input:*const DrmFrame,pts:i64,
        lease:*mut c_void,release:extern "C" fn(*mut c_void),out:*mut Encoded)->c_int;
    fn weavelo_encoder_drm_modifier(device:*const c_char,modifier:*mut u64)->c_int;
    fn weavelo_encoder_destroy(encoder:*mut c_void);
    fn weavelo_encoder_request_keyframe(encoder:*mut c_void);
}
struct Encoder(*mut c_void);
impl Drop for Encoder { fn drop(&mut self) { unsafe { weavelo_encoder_destroy(self.0); } } }

pub(super) fn capture_modifier(hardware:&HardwareEncoder)->Result<u64> {
    let device=CString::new(hardware.device.as_os_str().as_encoded_bytes())?;
    let mut modifier=0;
    let result=unsafe {weavelo_encoder_drm_modifier(device.as_ptr(),&mut modifier)};
    if result<0 || modifier==0x00ff_ffff_ffff_ffff {
        return Err(format!("Direct VAAPI DRM import unavailable ({result})").into());
    }
    Ok(modifier)
}

pub(crate) struct NativeSource {
    input: Arc<Input>,
    force_keyframe: Arc<AtomicBool>,
    output: Mutex<mpsc::Receiver<std::result::Result<RawChunk,String>>>,
    worker: Option<JoinHandle<()>>,
    format: FrameFormat, width: u32, height: u32, generation: u64, revision: u64,
    dmabuf: bool,
}
impl NativeSource {
    pub(crate) fn start(first:&OwnedFrame,settings:RawSettings,hardware:&HardwareEncoder)->Result<Self> {
        validate_native_input(first)?;
        settings.validate_for(first.width,first.height)?;
        let signal=HardwareEncoder::signal(first)?;
        let device=CString::new(hardware.device.as_os_str().as_encoded_bytes())?;
        let pixel=CString::new(match first.source_format.pixel_format {
            PixelFormat::Bgra=>"bgra",PixelFormat::Bgrx=>"bgr0",PixelFormat::Rgba=>"rgba",
            PixelFormat::Rgbx=>"rgb0",PixelFormat::Nv12=>"nv12",PixelFormat::I420=>"yuv420p",
        })?;
        let (width,height)=(first.width,first.height);
        let dmabuf=first.dmabuf.is_some();
        let source=first.source_format;
        let crop=source.crop.unwrap_or(crate::linux_frame::Crop {
            x:0,y:0,width:source.width,height:source.height });
        let input=Arc::new(Input::default());
        let force_keyframe=Arc::new(AtomicBool::new(false));
        let keyframe_request=force_keyframe.clone();
        let (sender,receiver)=mpsc::channel(2);
        let pending=input.clone();
        let worker=std::thread::Builder::new().name("weavelo-vaapi-encode".into()).spawn(move || {
            let result=(||->Result<()> {
                let mut encoder=Encoder(std::ptr::null_mut());
                let result=if dmabuf {unsafe {
                    weavelo_encoder_create_drm(device.as_ptr(),pixel.as_ptr(),source.width as i32,source.height as i32,
                        crop.x as i32,crop.y as i32,crop.width as i32,crop.height as i32,
                        settings.width.min(width) as i32,settings.height.min(height) as i32,settings.fps as i32,
                        settings.bitrate_kbps as i32,if signal.full_range {2}else{1},signal.matrix.unwrap() as i32,
                        signal.transfer.unwrap() as i32,signal.primaries.unwrap() as i32,&mut encoder.0)
                }}else{unsafe {weavelo_encoder_create(device.as_ptr(),pixel.as_ptr(),width as i32,height as i32,
                    settings.width.min(width) as i32,settings.height.min(height) as i32,settings.fps as i32,
                    settings.bitrate_kbps as i32,if signal.full_range {2}else{1},signal.matrix.unwrap() as i32,
                    signal.transfer.unwrap() as i32,signal.primaries.unwrap() as i32,&mut encoder.0)}};
                if result<0 {return Err(format!("Native VAAPI encoder initialization failed ({result})").into());}
                let mut sequence=0i64;
                while let Some(frame)=pending.receive() {
                    let started=remote_core::timing::quanta_now_us();
                    if keyframe_request.swap(false, Ordering::AcqRel) {
                        unsafe { weavelo_encoder_request_keyframe(encoder.0); }
                    }
                    let mut planes=[std::ptr::null();3];
                    for (slot,plane) in planes.iter_mut().zip(frame.planes.iter()) {*slot=plane.as_ptr();}
                    let rgb=matches!(frame.source_format.pixel_format,PixelFormat::Bgra|PixelFormat::Bgrx|PixelFormat::Rgba|PixelFormat::Rgbx);
                    let strides=if rgb {[width as i32*4,0,0]}else if frame.source_format.pixel_format==PixelFormat::Nv12 {
                        [width as i32,width as i32,0]
                    }else{[width as i32,width as i32/2,width as i32/2]};
                    let mut packet=Encoded::default();
                    let result=if let Some(gpu)=frame.dmabuf.as_ref() {
                        let mut descriptor=DrmFrame {fourcc:gpu.fourcc,objects:gpu.objects.len() as u32,
                            planes:gpu.planes.len() as u32,..Default::default()};
                        for (target,object) in descriptor.object.iter_mut().zip(&gpu.objects) {
                            *target=DrmObject {fd:object.fd.as_raw_fd(),size:object.size,modifier:object.modifier};
                        }
                        for (target,plane) in descriptor.plane.iter_mut().zip(&gpu.planes) {
                            *target=DrmPlane {object:plane.object,offset:plane.offset,stride:plane.stride};
                        }
                        let lease=Box::into_raw(Box::new(gpu.clone())).cast::<c_void>();
                        unsafe {weavelo_encoder_frame_drm(encoder.0,&descriptor,sequence,lease,release_dma_lease,&mut packet)}
                    }else{unsafe {weavelo_encoder_frame(encoder.0,planes.as_ptr(),strides.as_ptr(),sequence,&mut packet)}};
                    if result<0 {return Err(format!("Native VAAPI frame encoding failed ({result})").into());}
                    if packet.data.is_null() || packet.bytes==0 || packet.bytes>super::super::MAX_AU_BYTES || packet.pts!=sequence {
                        return Err("Native VAAPI packet ownership/timing mismatch".into());
                    }
                    // Copy compressed bytes before the next native call. Raw pixels
                    // retain their existing allocation, and never cross a pipe.
                    let nalu=unsafe {std::slice::from_raw_parts(packet.data,packet.bytes)}.to_vec();
                    if packet.key!=0 && !matches!(remote_core::hevc_sequence::sequence_info(&nalu),
                        Ok(Some(sequence)) if sequence.signal==Some(signal)) {
                        return Err("Native VAAPI bitstream does not preserve capture color metadata".into());
                    }
                    let chunk=RawChunk {nalu,is_keyframe:packet.key!=0,stamp:frame.stamp,
                        encode_done_ts_us:remote_core::timing::quanta_now_us(),native_timing:Some(NativeEncodeTiming {
                            queue_us:started.saturating_sub(frame.stamp.arrival_ts_us),
                            upload_vpp_us:packet.upload_vpp_us,encode_us:packet.encode_us,
                        })};
                    // VPP/encoding has completed and the AU owns its bytes.
                    // Network backpressure must not keep raw PipeWire storage.
                    drop(frame);
                    if sender.blocking_send(Ok(chunk)).is_err() {break;}
                    sequence=sequence.checked_add(1).ok_or("Native VAAPI sequence exhausted")?;
                }
                Ok(())
            })();
            if let Err(error)=result {let _=sender.blocking_send(Err(error.to_string()));}
            pending.close();
        })?;
        Ok(Self {input,force_keyframe,output:Mutex::new(receiver),worker:Some(worker),format:first.source_format,
            width,height,generation:first.stamp.generation,revision:first.stamp.format_revision,dmabuf})
    }
    pub(crate) fn matches(&self,frame:&OwnedFrame)->bool {
        self.generation==frame.stamp.generation && self.revision==frame.stamp.format_revision
            && self.dmabuf==frame.dmabuf.is_some()
            && self.format==frame.source_format && (self.width,self.height)==(frame.width,frame.height)
    }
    pub(crate) fn submit(&self,frame:OwnedFrame)->Result<()> {
        frame.validate_owned()?;
        if !self.matches(&frame) {return Err("Native VAAPI input geometry/generation changed".into());}
        self.input.submit(frame)
    }
    pub(crate) fn request_keyframe(&self) {
        self.force_keyframe.store(true, Ordering::Release);
    }
    pub(crate) async fn pull(&self)->Result<RawChunk> {
        self.output.lock().await.recv().await.ok_or("Native VAAPI encoder ended")?.map_err(Into::into)
    }
}
impl Drop for NativeSource {
    fn drop(&mut self) {
        self.input.close();
        self.output.get_mut().close();
        if let Some(worker)=self.worker.take() {let _=worker.join();}
    }
}
