use std::error::Error;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tokio::sync::{Mutex as AsyncMutex, mpsc};

/// Long-running ffmpeg capture → HEVC Annex-B pipeline used by Linux and Windows hosts.
/// Stdout is drained on a dedicated bounded reader so cancellation never waits on a
/// blocking pipe read and an overloaded viewer cannot grow memory without bound.
pub struct FfmpegHevcSource {
    child: Mutex<Option<Child>>,
    access_units: AsyncMutex<mpsc::Receiver<Result<(Vec<u8>, bool), String>>>,
    reader: Mutex<Option<std::thread::JoinHandle<()>>>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// Drop the old OS capture process before creating its replacement. On failure
/// the slot stays empty rather than keeping a stale encoder with new settings.
pub(crate) fn replace_capture_source<T, E>(
    slot: &mut Option<T>,
    create: impl FnOnce() -> Result<T, E>,
) -> Result<(), E> {
    drop(slot.take());
    *slot = Some(create()?);
    Ok(())
}

impl FfmpegHevcSource {
    pub fn start(
        width: u32,
        height: u32,
        fps: u32,
        bitrate_kbps: u32,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Self::spawn(capture_command(width, height, fps, bitrate_kbps)?,width,height,fps,bitrate_kbps)
    }

    #[cfg(target_os="windows")]
    pub(crate) fn start_native(selection:&crate::windows_sources::Selection,width:u32,height:u32,fps:u32,kbps:u32)->Result<Self,Box<dyn Error+Send+Sync>> {
        let program=crate::capture_readiness::ffmpeg_program()?;
        let bounds=selection.current_bounds()?;
        crate::capture_readiness::require_srgb_capture_profile(&selection.monitor)?;
        let canvas=crate::windows_capture_plan::fit(bounds,width,height)?;
        let command=crate::windows_capture_plan::command(&program,selection.handle,selection.window,canvas,fps,kbps,selection.encoder);
        Self::spawn(command,canvas.0,canvas.1,fps,kbps)
    }

    fn spawn(mut command:Command,width:u32,height:u32,fps:u32,bitrate_kbps:u32)->Result<Self,Box<dyn Error+Send+Sync>> {
        let backend = crate::capture_backend::ffmpeg_input_format(&command);
        // Keep startup/codec failures in the host's existing diagnostic log.
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = command.spawn().map_err(|err| {
            format!("Native HEVC pipeline failed to start: {err}")
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or("ffmpeg stdout was not captured")?;
        let (access_tx, access_rx) = mpsc::channel(4);
        let reader = match std::thread::Builder::new()
            .name("weavelo-ffmpeg-reader".into())
            .spawn(move || drain_hevc_reader(stdout, access_tx))
        {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to start FFmpeg reader thread: {error}").into());
            }
        };
        println!(
            "[FfmpegHevc] started with output bounds {width}x{height}@{fps} {bitrate_kbps} kbps ({})",
            backend
        );
        Ok(Self {
            child: Mutex::new(Some(child)),
            access_units: AsyncMutex::new(access_rx),
            reader: Mutex::new(Some(reader)),
            width,
            height,
            fps,
        })
    }

    pub async fn pull_access_unit(&self) -> Result<(Vec<u8>, bool), Box<dyn Error + Send + Sync>> {
        let mut receiver = self.access_units.lock().await;
        match receiver.recv().await {
            Some(Ok(packet)) => Ok(packet),
            Some(Err(error)) => Err(error.into()),
            None => Err("ffmpeg HEVC pipeline ended".into()),
        }
    }
}

fn drain_hevc_reader<R: Read>(
    mut stdout: R,
    access_tx: mpsc::Sender<Result<(Vec<u8>, bool), String>>,
) {
    let mut leftover = Vec::new();
    let mut buf = [0u8; 32_768];
    loop {
        while let Some(au) = take_access_unit(&mut leftover) {
            let is_keyframe = is_hevc_keyframe(&au);
            // These are already encoded references. Apply bounded backpressure
            // on this dedicated reader instead of silently corrupting the GOP.
            if access_tx.blocking_send(Ok((au, is_keyframe))).is_err() {
                return;
            }
        }

        match stdout.read(&mut buf) {
            Ok(0) => {
                let _ = access_tx.try_send(Err("ffmpeg HEVC pipeline ended".into()));
                return;
            }
            Ok(read) => leftover.extend_from_slice(&buf[..read]),
            Err(error) => {
                let _ = access_tx.try_send(Err(format!("ffmpeg HEVC read failed: {error}")));
                return;
            }
        }
    }
}

impl Drop for FfmpegHevcSource {
    fn drop(&mut self) {
        // Wake a reader blocked on the bounded channel before joining it.
        self.access_units.get_mut().close();
        if let Ok(mut child) = self.child.lock()
            && let Some(mut child) = child.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Ok(mut reader) = self.reader.lock()
            && let Some(reader) = reader.take()
        {
            let _ = reader.join();
        }
    }
}

fn capture_command(
    width: u32,
    height: u32,
    fps: u32,
    bitrate_kbps: u32,
) -> Result<Command, Box<dyn Error + Send + Sync>> {
    #[cfg(target_os = "windows")]
    let program = crate::capture_readiness::ffmpeg_program()?;
    #[cfg(not(target_os = "windows"))]
    let program = std::path::PathBuf::from("ffmpeg");
    capture_command_using(&program, width, height, fps, bitrate_kbps)
}

fn capture_command_using(
    program: &std::path::Path,
    width: u32,
    height: u32,
    fps: u32,
    bitrate_kbps: u32,
) -> Result<Command, Box<dyn Error + Send + Sync>> {
    let mut cmd = Command::new(program);
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
    #[cfg(target_os="windows")]
    {
        let selection=crate::windows_sources::resolve(protocol::session::CaptureSource::MainDisplay)?;
        crate::capture_readiness::require_srgb_capture_profile(&selection.monitor)?;
        let canvas=crate::windows_capture_plan::fit(selection.bounds,width,height)?;
        return Ok(crate::windows_capture_plan::command(program,selection.handle,selection.window,canvas,fps,bitrate_kbps,selection.encoder));
    }
    #[cfg(target_os = "linux")]
    let x11_display = {
        crate::linux_capture_geometry::require_current_x11_desktop()?;
        std::env::var("DISPLAY").ok()
    };
    #[cfg(target_os = "linux")]
    {
        if let Some(display) = &x11_display {
            let region = crate::linux_capture_geometry::capture_region(display)?;
            let plan = crate::linux_capture_geometry::X11CapturePlan::new(region, width, height)?;
            plan.append_args(&mut cmd, display, fps);
            println!(
                "[FfmpegHevc] X11 compatibility plan: root source {}x{} at +{},{}; encoded {}x{} within requested {}x{}",
                region.width,
                region.height,
                region.x,
                region.y,
                plan.output_width,
                plan.output_height,
                width,
                height,
            );
        } else {
            cmd.args([
                "-f",
                "kmsgrab",
                "-framerate",
                &fps.to_string(),
                "-i",
                "/dev/dri/card0",
            ]);
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = (width, height, fps);
        return Err("ffmpeg HEVC capture is only implemented for Linux and Windows".into());
    }
    let bitrate = format!("{bitrate_kbps}k");
    let keyint = fps.max(1).to_string();
    let mut encoder_options =
        format!("keyint={keyint}:min-keyint={keyint}:repeat-headers=1:bframes=0");
    #[cfg(target_os = "linux")]
    if x11_display.is_some() {
        // Match the explicit RGB -> limited BT.709 conversion in X11CapturePlan.
        // Unknown source primaries/transfer stay unspecified. KMS/native YUV
        // sources keep their own metadata path, never this RGB declaration.
        cmd.args(["-color_range", "tv", "-colorspace", "bt709"]);
        encoder_options.push_str(":colormatrix=bt709:range=limited");
    }
    cmd.args([
        "-pix_fmt",
        "yuv420p",
        "-c:v",
        "libx265",
        "-preset",
        "ultrafast",
        "-tune",
        "zerolatency",
        "-x265-params",
        &encoder_options,
        "-b:v",
        &bitrate,
        "-maxrate",
        &bitrate,
        "-bufsize",
        &bitrate,
        "-f",
        "hevc",
        "pipe:1",
    ]);
    Ok(cmd)
}

fn start_code_len(data: &[u8]) -> usize {
    if data.len() >= 4 && data[0] == 0 && data[1] == 0 && data[2] == 0 && data[3] == 1 {
        4
    } else if data.len() >= 3 && data[0] == 0 && data[1] == 0 && data[2] == 1 {
        3
    } else {
        0
    }
}

fn find_start(data: &[u8], from: usize) -> Option<usize> {
    let slice = data.get(from..)?;
    (0..slice.len())
        .find_map(|offset| (start_code_len(&slice[offset..]) > 0).then_some(from + offset))
}

fn hevc_nal_type(nal: &[u8]) -> Option<u8> {
    let prefix = start_code_len(nal);
    nal.get(prefix).map(|byte| (byte >> 1) & 0x3f)
}

fn is_hevc_vcl(nal_type: u8) -> bool {
    nal_type <= 31
}

fn is_hevc_keyframe(au: &[u8]) -> bool {
    let mut offset = 0;
    while offset < au.len() {
        let Some(next) = find_start(au, offset + 1) else {
            return matches!(hevc_nal_type(&au[offset..]), Some(16..=21));
        };
        if matches!(hevc_nal_type(&au[offset..next]), Some(16..=21)) {
            return true;
        }
        offset = next;
    }
    false
}

fn take_access_unit(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let first = find_start(buffer, 0)?;
    if first > 0 {
        buffer.drain(..first);
    }
    let mut offset = 0;
    let mut saw_vcl = false;
    loop {
        let Some(start) = find_start(buffer, offset) else {
            return None;
        };
        let Some(next) = find_start(buffer, start + 3) else {
            return None;
        };
        let nal_type = hevc_nal_type(&buffer[start..next]).unwrap_or(0);
        // HEVC prefix parameter sets/AUD/prefix-SEI belong to the next
        // picture, not the preceding VCL. Keeping them on a delta frame
        // strands a receiver that skipped that frame while awaiting an IDR.
        if saw_vcl && matches!(nal_type, 32..=35 | 39) {
            return Some(buffer.drain(..start).collect());
        }
        if is_hevc_vcl(nal_type) {
            if saw_vcl {
                let au = buffer.drain(..start).collect();
                return Some(au);
            }
            saw_vcl = true;
        }
        offset = next;
        if offset > 2 * 1024 * 1024 {
            let au = buffer.drain(..next).collect();
            return Some(au);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires DISPLAY and an independently observed WEAVELO_X11_EXPECTED_ROOT; metadata only"]
    fn linux_x11_command_reads_root_metadata_without_capturing() {
        let display = std::env::var("DISPLAY").expect("explicit X11 display required");
        let expected = std::env::var("WEAVELO_X11_EXPECTED_ROOT")
            .expect("independently observed root geometry required");
        let (width, height) = expected.split_once('x').expect("expected WIDTHxHEIGHT");
        let expected = (
            width.parse::<u32>().unwrap(),
            height.parse::<u32>().unwrap(),
        );
        let source = crate::linux_capture_geometry::capture_region(&display).unwrap();
        assert_eq!((source.x, source.y), (0, 0));
        assert_eq!((source.width, source.height), expected);
        let plan = crate::linux_capture_geometry::X11CapturePlan::new(source, 1920, 1080).unwrap();

        // Exercise the real Linux command constructor, never Command::spawn.
        let command = capture_command(1920, 1080, 60, 6000).unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        let input_size = format!("{}x{}", source.width, source.height);
        let input = format!("{display}+0,0");
        let filter = format!("scale=w={}:h={}:in_range=pc:out_range=tv:out_color_matrix=bt709", plan.output_width, plan.output_height);
        for (flag, value) in [
            ("-video_size", input_size.as_str()),
            ("-i", input.as_str()),
            ("-vf", filter.as_str()),
            ("-c:v", "libx265"),
            ("-pix_fmt", "yuv420p"),
        ] {
            assert!(args.windows(2).any(|pair| pair == [flag, value]));
        }
        assert_eq!(
            crate::capture_backend::ffmpeg_input_format(&command),
            "x11grab"
        );
        assert!(args.windows(2).any(|pair| pair == ["-colorspace", "bt709"]));
        assert!(args.iter().any(|value| value.ends_with(":colormatrix=bt709:range=limited")));
        assert!(!args.iter().any(|value| value.contains("colorprim=") || value.contains("transfer=")));
        println!("metadata-only Linux command verified: {command:?}");
    }

    #[test]
    fn replacement_drops_previous_capture_before_factory_runs() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Capture(Arc<AtomicUsize>);
        impl Drop for Capture {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let live = Arc::new(AtomicUsize::new(1));
        let mut slot = Some(Capture(live.clone()));
        replace_capture_source(&mut slot, || {
            assert_eq!(live.load(Ordering::SeqCst), 0);
            live.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(Capture(live.clone()))
        })
        .unwrap();
        assert_eq!(live.load(Ordering::SeqCst), 1);
        drop(slot);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn failed_replacement_leaves_no_stale_capture() {
        let mut slot = Some(42u32);
        let error = replace_capture_source(&mut slot, || Err::<u32, _>("startup failed"));
        assert_eq!(error, Err("startup failed"));
        assert!(slot.is_none());
    }



    #[tokio::test]
    async fn bounded_reader_preserves_encoded_references_under_backpressure() {
        let mut input = Vec::new();
        for byte in 0u8..20 {
            input.extend_from_slice(&[0, 0, 0, 1, 0x02, byte]);
        }
        input.extend_from_slice(&[0, 0, 0, 1, 0x02, 0xff]);
        input.extend_from_slice(&[0, 0, 0, 1, 0x02, 0xfe]);
        let (tx, mut rx) = mpsc::channel(2);
        let reader = std::thread::spawn(move || {
            drain_hevc_reader(std::io::Cursor::new(input), tx);
        });
        let mut delivered = Vec::new();
        while let Some(packet) = rx.recv().await {
            let Ok((bytes, _)) = packet else {
                break;
            };
            delivered.push(*bytes.last().unwrap());
        }
        reader.join().unwrap();
        assert_eq!(delivered, (0u8..20).collect::<Vec<_>>());
    }

    #[test]
    fn closing_receiver_releases_a_backpressured_reader() {
        let mut input = Vec::new();
        for byte in 0u8..20 {
            input.extend_from_slice(&[0, 0, 0, 1, 0x02, byte]);
        }
        let (tx, mut rx) = mpsc::channel(1);
        let (finished, observed) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            drain_hevc_reader(std::io::Cursor::new(input), tx);
            finished.send(()).unwrap();
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while rx.is_empty() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(!rx.is_empty(), "reader did not fill its bounded output");
        rx.close();
        observed
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn splits_two_vcl_nals_into_access_units() {
        let mut buffer = vec![
            0, 0, 0, 1, 0x02, 0xAA, // trail VCL
            0, 0, 0, 1, 0x02, 0xBB, // next picture VCL
            0, 0, 0, 1, 0x02, // incomplete
        ];
        let first = take_access_unit(&mut buffer).expect("first AU");
        assert_eq!(first, vec![0, 0, 0, 1, 0x02, 0xAA]);
        assert!(take_access_unit(&mut buffer).is_none());
    }

    #[test]
    fn repeated_parameter_sets_stay_with_following_keyframe() {
        fn nal(kind: u8) -> Vec<u8> {
            vec![0, 0, 0, 1, kind << 1, 1, 0x80]
        }
        let mut buffer = Vec::new();
        for kind in [1, 32, 33, 34, 39, 21, 1, 1] {
            buffer.extend(nal(kind));
        }
        assert_eq!(take_access_unit(&mut buffer).unwrap(), nal(1));
        let mut expected = Vec::new();
        for kind in [32, 33, 34, 39, 21] {
            expected.extend(nal(kind));
        }
        let keyframe = take_access_unit(&mut buffer).unwrap();
        assert_eq!(keyframe, expected);
        assert!(is_hevc_keyframe(&keyframe));
    }

    #[test]
    fn suffix_sei_remains_on_previous_picture_and_aud_starts_next() {
        fn nal(kind: u8) -> Vec<u8> {
            vec![0, 0, 1, kind << 1, 1, 0x80]
        }
        let mut buffer = Vec::new();
        for kind in [1, 40, 35, 32, 33, 34, 20, 1, 1] {
            buffer.extend(nal(kind));
        }
        let mut first = nal(1);
        first.extend(nal(40));
        assert_eq!(take_access_unit(&mut buffer).unwrap(), first);
        let mut next = Vec::new();
        for kind in [35, 32, 33, 34, 20] {
            next.extend(nal(kind));
        }
        assert_eq!(take_access_unit(&mut buffer).unwrap(), next);
    }

    #[test]
    fn detects_idr_keyframe() {
        let idr = vec![0, 0, 0, 1, 40, 0x00]; // type 20
        assert!(is_hevc_keyframe(&idr));
        let trail = vec![0, 0, 0, 1, 0x02, 0x00];
        assert!(!is_hevc_keyframe(&trail));
    }
}
