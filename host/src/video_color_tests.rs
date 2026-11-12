//! Real hardware encoder regression using generated NV12 buffers, not capture.
use super::*;
use core_foundation::base::CFTypeRef;
use core_foundation::{base::TCFType,boolean::CFBoolean,dictionary::CFDictionary,string::CFString};
use screencapturekit::{
    cm::{CMSampleBuffer, CMTime as SampleTime},
    cv::CVPixelBuffer,
};
use std::time::Duration;
use remote_core::hevc_sequence;

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferCreate(a:*const c_void,width:usize,height:usize,format:u32,attrs:*const c_void,out:*mut *mut c_void)->i32;
    static kCVPixelBufferIOSurfacePropertiesKey:*const c_void;
    static kCVPixelBufferMetalCompatibilityKey:*const c_void;
    fn CVBufferSetAttachment(buffer: *const c_void, key: CFTypeRef, value: CFTypeRef, mode: u32);
    static kCVImageBufferColorPrimariesKey: CFTypeRef;
    static kCVImageBufferTransferFunctionKey: CFTypeRef;
    static kCVImageBufferYCbCrMatrixKey: CFTypeRef;
    static kCVImageBufferColorPrimaries_ITU_R_709_2: CFTypeRef;
    static kCVImageBufferTransferFunction_ITU_R_709_2: CFTypeRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_709_2: CFTypeRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_601_4: CFTypeRef;
}
fn generated_frame(
    width: u32,
    height: u32,
    index: i64,
    matrix709: bool,
) -> Result<MacVideoFrame, Box<dyn Error + Send + Sync>> {
    // Match real native capture: a plain CPU allocation cannot be imported by
    // Metal and is not a valid fixture for the viewport GPU path.
    let surface=CFDictionary::<CFString,CFBoolean>::from_CFType_pairs(&[]);
    let keys=unsafe{[kCVPixelBufferIOSurfacePropertiesKey,kCVPixelBufferMetalCompatibilityKey]}
        .map(|key|unsafe{CFString::wrap_under_get_rule(key.cast())});
    let attrs=CFDictionary::from_CFType_pairs(&[(keys[0].as_CFType(),surface.as_CFType()),(keys[1].as_CFType(),CFBoolean::true_value().as_CFType())]);
    let mut raw=std::ptr::null_mut();
    let status=unsafe{CVPixelBufferCreate(std::ptr::null(),width as usize,height as usize,0x34323076,attrs.as_concrete_TypeRef().cast(),&mut raw)};
    if status!=0{return Err(format!("synthetic native CV buffer: {status}").into());}
    let buffer=CVPixelBuffer::from_raw(raw).ok_or("Missing synthetic IOSurface buffer")?;
    {
        let mut guard = buffer
            .lock_read_write()
            .map_err(|e| format!("synthetic CV lock: {e}"))?;
        if guard.plane_count() != 2 {
            return Err("test requires NV12 two-plane buffer".into());
        }
        for plane in 0..2 {
            let size = guard
                .bytes_per_row_of_plane(plane)
                .checked_mul(guard.height_of_plane(plane))
                .ok_or("plane overflow")?;
            let ptr = guard
                .base_address_of_plane_mut(plane)
                .ok_or("test plane missing")?;
            unsafe {
                std::ptr::write_bytes(ptr, if plane == 0 { 96 } else { 128 }, size);
            }
        }
    }
    unsafe {
        for (key, value) in [
            (
                kCVImageBufferColorPrimariesKey,
                kCVImageBufferColorPrimaries_ITU_R_709_2,
            ),
            (
                kCVImageBufferTransferFunctionKey,
                kCVImageBufferTransferFunction_ITU_R_709_2,
            ),
            (
                kCVImageBufferYCbCrMatrixKey,
                if matrix709 {
                    kCVImageBufferYCbCrMatrix_ITU_R_709_2
                } else {
                    kCVImageBufferYCbCrMatrix_ITU_R_601_4
                },
            ),
        ] {
            CVBufferSetAttachment(buffer.as_ptr().cast(), key, value, 1);
        }
    }
    let sample = CMSampleBuffer::create_for_image_buffer(
        &buffer,
        SampleTime::new(index, 30),
        SampleTime::new(1, 30),
    )
    .map_err(|e| format!("synthetic sample: {e}"))?;
    Ok(MacVideoFrame {
        width,
        height,
        window_crop: None,
        sample_buffer: std::sync::Arc::new(sample),
        capture_time_ms: (index * 33) as u32,
        timing: protocol::FrameTimingCheckpoints::default(),
        pacing_ts_us: index as u64 * 1_000_000 / 30,
    })
}

#[tokio::test]
async fn static_iosurface_redraw_changes_roi_without_changing_capture_clock_or_encoding_idle_duplicates()->Result<(),Box<dyn Error+Send+Sync>> {
    use protocol::viewport::{NormalizedRect,ViewportDemand};
    let mut encoder=MacVideoEncoder::new(320,180,30,4000)?;
    let mut settings=crate::StreamSettings{width:320,height:180,fps:30,bitrate_kbps:4000,paused:false,source_revision:3,viewport:None};
    encoder.viewport_settings(&settings,protocol::session::CaptureSource::MainDisplay)?;
    let mut frame=generated_frame(640,360,0,true)?;frame.timing=protocol::FrameTimingCheckpoints::new(remote_core::timing::quanta_now_us());
    encoder.submit_frame(frame.clone()).await?;let first=tokio::time::timeout(Duration::from_secs(3),encoder.pull_encoded_chunk()).await??;
    assert!(!encoder.redraw_if_needed(frame.clone()).await?,"Idle viewport must not re-encode the same pixels");
    settings.viewport=Some(ViewportDemand{id:7,source_revision:3,revision:1,visible:NormalizedRect{x:250_000,y:250_000,width:500_000,height:500_000},width:320,height:180});
    encoder.viewport_settings(&settings,protocol::session::CaptureSource::MainDisplay)?;
    let requested=remote_core::timing::quanta_now_us();assert!(encoder.redraw_if_needed(frame.clone()).await?);
    let moved=tokio::time::timeout(Duration::from_secs(3),encoder.pull_encoded_chunk()).await??;
    let geometry=protocol::viewport::split_frame(&moved.nalu)?.0.unwrap();assert_eq!(geometry.demand_revision,1);assert!(geometry.crop.x>0&&geometry.crop.y>0);
    assert_eq!(moved.timing.capture_ts_us,first.timing.capture_ts_us);assert!(moved.admitted_at_us>=requested);
    assert!(!encoder.redraw_if_needed(frame).await?);Ok(())
}

#[tokio::test]
async fn odd_native_iosurface_keeps_full_pixels_and_can_crop_the_bottom_right_edge()->Result<(),Box<dyn Error+Send+Sync>> {
    use protocol::viewport::{NormalizedRect,ViewportDemand,PixelRect};
    let mut encoder=MacVideoEncoder::new(320,180,30,4000)?;
    let mut settings=crate::StreamSettings{width:320,height:180,fps:30,bitrate_kbps:4000,paused:false,source_revision:3,viewport:None};
    encoder.viewport_settings(&settings,protocol::session::CaptureSource::MainDisplay)?;
    let mut frame=generated_frame(641,371,0,true)?;frame.timing=protocol::FrameTimingCheckpoints::new(remote_core::timing::quanta_now_us());
    encoder.submit_frame(frame.clone()).await?;let first=tokio::time::timeout(Duration::from_secs(3),encoder.pull_encoded_chunk()).await??;
    let initial=protocol::viewport::split_frame(&first.nalu)?.0.unwrap();assert_eq!(initial.crop,PixelRect{x:0,y:0,width:641,height:371});
    let visible=NormalizedRect{x:500_000,y:500_000,width:500_000,height:500_000};
    settings.viewport=Some(ViewportDemand{id:7,source_revision:3,revision:1,visible,width:320,height:180});
    encoder.viewport_settings(&settings,protocol::session::CaptureSource::MainDisplay)?;assert!(encoder.redraw_if_needed(frame).await?);
    let moved=tokio::time::timeout(Duration::from_secs(3),encoder.pull_encoded_chunk()).await??;
    let geometry=protocol::viewport::split_frame(&moved.nalu)?.0.unwrap();assert!(geometry.crop.contains(remote_core::viewport::visible_pixels(visible,[641,371])));
    assert_eq!(moved.timing.capture_ts_us,first.timing.capture_ts_us);Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_source_color_survives_hardware_hevc_and_session_resize()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let mut encoder = MacVideoEncoder::new(640, 360, 30, 2000)?;
    for (index, (w, h, matrix709, expected)) in [
        (640, 360, true, 1),
        (640, 360, false, 6),
        (320, 240, true, 1),
    ]
    .into_iter()
    .enumerate()
    {
        let frame = generated_frame(w, h, index as i64, matrix709)?;
        encoder.submit_frame(frame).await?;
        unsafe {
            let status = VTCompressionSessionCompleteFrames(
                encoder.session.unwrap(),
                core_media_sys::CMTime {
                    value: 0,
                    timescale: 0,
                    flags: 0,
                    epoch: 0,
                },
            );
            if status != 0 {
                return Err(format!("test encoder flush: {status}").into());
            }
        }
        let chunk =
            tokio::time::timeout(Duration::from_secs(5), encoder.pull_encoded_chunk()).await??;
        let sequence =
            hevc_sequence::sequence_info(&chunk.nalu)?.ok_or("hardware output omitted SPS")?;
        let signal = sequence
            .signal
            .ok_or("hardware output omitted color signal")?;
        assert_eq!(
            signal.matrix,
            Some(expected),
            "the source matrix must be encoded, not inferred by receiver"
        );
        assert_eq!(signal.primaries, Some(1));
        assert_eq!(signal.transfer, Some(1));
        assert!(!signal.full_range);
        assert_eq!([sequence.visible[2], sequence.visible[3]], [w, h]);
        println!(
            "SOURCE_COLOR_HEVC_VERIFIED size={w}x{h} matrix={expected} primaries=1 transfer=1 range=limited keyframe={}",
            chunk.is_keyframe
        );
    }
    Ok(())
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn native_encoder_backpressure_keeps_every_compressed_reference()
-> Result<(),Box<dyn Error+Send+Sync>> {
    let mut encoder=MacVideoEncoder::new(640,360,30,2000)?;
    for index in 0..ENCODE_CAPACITY {
        encoder.submit_frame(generated_frame(640,360,index as i64,true)?).await?;
    }
    assert!(!encoder.can_submit_frame());
    unsafe { assert_eq!(VTCompressionSessionCompleteFrames(encoder.session.unwrap(),core_media_sys::CMTime{value:0,timescale:0,flags:0,epoch:0}),0); }
    let first=tokio::time::timeout(Duration::from_secs(5),encoder.pull_encoded_chunk()).await??;
    assert!(first.is_keyframe);assert!(!encoder.can_submit_frame());
    drop(first);assert!(encoder.can_submit_frame());
    let mut count=1;
    while count<ENCODE_CAPACITY {
        let chunk=tokio::time::timeout(Duration::from_secs(5),encoder.pull_encoded_chunk()).await??;
        assert!(!chunk.nalu.is_empty());count+=1;
    }
    assert_eq!(count,ENCODE_CAPACITY);
    Ok(())
}
