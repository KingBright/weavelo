//! Native VAAPI upload, scaling and HEVC encoding. Capability probes use one
//! synthetic frame; real camera pixels never pass through a CPU color converter.
use super::{RawSettings,Result,encoded_color};
use crate::linux_frame::OwnedFrame;
use std::{path::PathBuf,process::{Command,Stdio},sync::{Mutex,OnceLock},time::{Duration,Instant}};
use std::os::unix::fs::FileTypeExt;

#[derive(Clone)]
pub(crate) struct HardwareEncoder {device:PathBuf}
impl HardwareEncoder {
    pub(crate) fn probe()->Result<Self> {
        static DEVICE:OnceLock<Mutex<Option<HardwareEncoder>>>=OnceLock::new();
        let mut cached=DEVICE.get_or_init(Default::default).lock().unwrap_or_else(|e|e.into_inner());
        if let Some(encoder)=cached.as_ref() && encoder.device.metadata().is_ok_and(|m|m.file_type().is_char_device()) {
            return Ok(encoder.clone());
        }
        let mut devices=std::fs::read_dir("/dev/dri").into_iter().flatten().filter_map(|entry| {
            let entry=entry.ok()?;let name=entry.file_name();let name=name.to_str()?;
            let number=name.strip_prefix("renderD")?;
            if number.is_empty() || !number.bytes().all(|c|c.is_ascii_digit()) || !entry.file_type().ok()?.is_char_device(){return None;}
            Some(entry.path())
        }).collect::<Vec<_>>();
        devices.sort();
        for device in devices.into_iter().take(8) {
            let encoder=Self{device};let mut command=Command::new("ffmpeg");
            command.args(["-hide_banner","-nostdin","-v","error"]);encoder.configure_device(&mut command);
            // Actual HEVC availability, not an encoder-name/configuration check.
            // 640x360 also meets AMD's native minimum of 130x128 on HO5.
            command.args(["-f","lavfi","-i","color=black:size=640x360:rate=30","-vf","format=nv12,hwupload",
                "-an","-c:v","hevc_vaapi","-bf","0","-frames:v","1","-f","hevc","pipe:1"])
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            let mut child=command.spawn()?;let deadline=Instant::now()+Duration::from_secs(3);
            loop {
                if let Some(status)=child.try_wait()? {
                    if status.success(){*cached=Some(encoder.clone());return Ok(encoder);}
                    break;
                }
                if Instant::now()>=deadline {let _=child.kill();let _=child.wait();break;}
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        Err("Camera sharing requires an available native VAAPI HEVC encoder; no software fallback was started".into())
    }
    pub(super) fn configure_device(&self,command:&mut Command) {
        command.args(["-init_hw_device",&format!("vaapi=weavelo:{}",self.device.display()),"-filter_hw_device","weavelo"]);
    }
    pub(super) fn signal(frame:&OwnedFrame)->Result<remote_core::hevc_sequence::VideoSignal> {
        let (range,matrix,transfer,primaries)=encoded_color(frame.source_format)?;
        if !matches!(range,1|2) || !matches!(matrix,1|6) || !matches!(transfer,1|13) || primaries!=1 {
            return Err("Native camera encoding requires explicit supported SDR color metadata".into());
        }
        Ok(remote_core::hevc_sequence::VideoSignal{full_range:range==2,
            matrix:Some(matrix as u8),transfer:Some(transfer as u8),primaries:Some(primaries as u8)})
    }
    pub(super) fn configure_output(&self,command:&mut Command,frame:&OwnedFrame,settings:RawSettings)->Result<()> {
        let signal=Self::signal(frame)?;
        let matrix=if signal.matrix==Some(1){"bt709"}else{"smpte170m"};
        let transfer=if signal.transfer==Some(13){"iec61966-2-1"}else{"bt709"};
        let range=if signal.full_range {"full"}else{"limited"};
        let filter=format!("hwupload,scale_vaapi=w={}:h={}:force_original_aspect_ratio=decrease:force_divisible_by=2:format=nv12:out_color_matrix={matrix}:out_range={range}:out_color_primaries=bt709:out_color_transfer={transfer}",
            settings.width.min(frame.width),settings.height.min(frame.height));
        // Mesa's HEVC encoder omits primaries/transfer even when AVCodecContext
        // is explicit. Signal the actual input/VPP contract in SPS; never infer
        // missing camera metadata. The reader verifies the resulting bitstream.
        let metadata=format!("hevc_metadata=aud=insert:video_full_range_flag={}:colour_primaries=1:transfer_characteristics={}:matrix_coefficients={}",
            u8::from(signal.full_range),signal.transfer.unwrap(),signal.matrix.unwrap());
        command.args(["-vf",&filter,"-c:v","hevc_vaapi","-bf","0","-g",&settings.fps.to_string(),"-bsf:v",&metadata]);
        Ok(())
    }
}
