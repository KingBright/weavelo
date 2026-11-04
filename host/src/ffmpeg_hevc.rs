use std::error::Error;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tokio::sync::{Mutex as AsyncMutex, mpsc};

/// Long-running process capture → HEVC pipeline for the Linux process backend.
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

    fn spawn(mut command:Command,width:u32,height:u32,fps:u32,bitrate_kbps:u32)->Result<Self,Box<dyn Error+Send+Sync>> {
        let backend = crate::capture_backend::ffmpeg_input_format(&command);
        // A static source may produce only one picture. Length-framed packets
        // let us submit that picture immediately, without waiting for the next AUD
        // or guessing that an arbitrary stdout read ended at a frame boundary.
        command.args(["-flush_packets", "1", "-f", "avi", "pipe:1"]);
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
    if let Err(error) = read_avi_header(&mut stdout) {
        let _ = access_tx.try_send(Err(format!("Native HEVC packet header: {error}")));
        return;
    }
    loop {
        match read_avi_packet(&mut stdout) {
            Ok(packet) => {
                let is_keyframe = is_hevc_keyframe(&packet);
                // Compressed references must remain ordered and complete.
                if access_tx.blocking_send(Ok((packet, is_keyframe))).is_err() { return; }
            }
            Err(error) => {
                let _ = access_tx.try_send(Err(format!("Native HEVC packet stream: {error}")));
                return;
            }
        }
    }
}

const MAX_PACKET_BYTES: usize = 8 * 1024 * 1024;

fn invalid_packet(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn read_avi_header(reader: &mut impl Read) -> std::io::Result<()> {
    let mut header = [0; 12];
    reader.read_exact(&mut header)?;
    if &header[..4] != b"RIFF" || &header[8..] != b"AVI " {
        return Err(invalid_packet("Expected the local encoder's AVI packet framing"));
    }
    Ok(())
}

fn read_avi_packet(reader: &mut impl Read) -> std::io::Result<Vec<u8>> {
    loop {
        let mut header = [0; 8];
        reader.read_exact(&mut header)?;
        let size = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
        let tag = &header[..4];
        if tag == b"LIST" || tag == b"RIFF" {
            if size < 4 { return Err(invalid_packet("Truncated AVI list")); }
            let mut kind = [0; 4];
            reader.read_exact(&mut kind)?;
            if (tag == b"LIST" && matches!(&kind, b"movi" | b"rec "))
                || (tag == b"RIFF" && &kind == b"AVIX") {
                continue;
            }
            skip_avi_bytes(reader, size - 4 + (size & 1))?;
        } else if tag == b"00dc" {
            // AVI may represent an idle timestamp gap with an empty chunk.
            if size == 0 { continue; }
            if size > MAX_PACKET_BYTES { return Err(invalid_packet("HEVC packet exceeds the local read budget")); }
            let mut packet = vec![0; size];
            reader.read_exact(&mut packet)?;
            if size & 1 != 0 { reader.read_exact(&mut [0])?; }
            if start_code_len(&packet) == 0 { return Err(invalid_packet("HEVC packet is not Annex-B")); }
            return Ok(packet);
        } else {
            skip_avi_bytes(reader, size + (size & 1))?;
        }
    }
}

fn skip_avi_bytes(reader: &mut impl Read, size: usize) -> std::io::Result<()> {
    if size > MAX_PACKET_BYTES { return Err(invalid_packet("AVI metadata exceeds the local read budget")); }
    let copied = std::io::copy(&mut reader.take(size as u64), &mut std::io::sink())?;
    if copied != size as u64 { return Err(std::io::ErrorKind::UnexpectedEof.into()); }
    Ok(())
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

pub(crate) fn is_hevc_keyframe(au: &[u8]) -> bool {
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



    fn packet_stream(packets: &[Vec<u8>]) -> Vec<u8> {
        let mut input = b"RIFF".to_vec();
        input.extend(u32::MAX.to_le_bytes());
        input.extend(b"AVI LIST");
        input.extend(8u32.to_le_bytes());
        input.extend(b"hdrltestLIST");
        input.extend(u32::MAX.to_le_bytes());
        input.extend(b"movi");
        for packet in packets {
            input.extend(b"00dc");
            input.extend((packet.len() as u32).to_le_bytes());
            input.extend(packet);
            if packet.len() & 1 != 0 { input.push(0); }
        }
        input
    }

    #[tokio::test]
    async fn bounded_reader_preserves_encoded_references_under_backpressure() {
        let packets = (0u8..20).map(|byte| vec![0, 0, 0, 1, 0x02, byte]).collect::<Vec<_>>();
        let input = packet_stream(&packets);
        let (tx, mut rx) = mpsc::channel(2);
        let reader = std::thread::spawn(move || drain_hevc_reader(std::io::Cursor::new(input), tx));
        let mut delivered = Vec::new();
        while let Some(Ok((bytes, _))) = rx.recv().await { delivered.push(bytes); }
        reader.join().unwrap();
        assert_eq!(delivered, packets);
    }

    #[test]
    fn closing_receiver_releases_a_backpressured_reader() {
        let packets = (0u8..20).map(|byte| vec![0, 0, 0, 1, 0x02, byte]).collect::<Vec<_>>();
        let input = packet_stream(&packets);
        let (tx, mut rx) = mpsc::channel(1);
        let (finished, observed) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            drain_hevc_reader(std::io::Cursor::new(input), tx);
            finished.send(()).unwrap();
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while rx.is_empty() && std::time::Instant::now() < deadline { std::thread::yield_now(); }
        assert!(!rx.is_empty(), "reader did not fill its bounded output");
        rx.close();
        observed.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        reader.join().unwrap();
    }

    #[tokio::test]
    async fn static_first_picture_is_delivered_while_the_pipe_stays_open() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut producer = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (consumer, _) = listener.accept().unwrap();
        let packet = vec![0, 0, 0, 1, 40, 1, 0x80];
        let (tx, mut rx) = mpsc::channel(1);
        let reader = std::thread::spawn(move || drain_hevc_reader(consumer, tx));
        producer.write_all(&packet_stream(&[packet.clone()])).unwrap();
        // No second picture, delimiter or producer EOF is provided.
        let first = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await;
        rx.close();
        drop(producer);
        reader.join().unwrap();
        let (actual, keyframe) = first.expect("static first picture stalled").unwrap().unwrap();
        assert_eq!(actual, packet);
        assert!(keyframe);
    }

    #[test]
    fn short_reads_keep_idr_parameter_sets_and_suffixes_on_their_own_packets() {
        struct Short(std::io::Cursor<Vec<u8>>);
        impl Read for Short {
            fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
                let length = into.len().min(1);
                self.0.read(&mut into[..length])
            }
        }
        fn nal(kind: u8) -> Vec<u8> { vec![0, 0, 0, 1, kind << 1, 1, 0x80] }
        let first = [nal(1), nal(40)].concat();
        let idr = [nal(35), nal(32), nal(33), nal(34), nal(39), nal(20)].concat();
        let mut input = Short(std::io::Cursor::new(packet_stream(&[first.clone(), idr.clone()])));
        read_avi_header(&mut input).unwrap();
        assert_eq!(read_avi_packet(&mut input).unwrap(), first);
        let actual = read_avi_packet(&mut input).unwrap();
        assert_eq!(actual, idr);
        assert!(is_hevc_keyframe(&actual));
    }

    #[test]
    fn idle_chunks_and_open_dml_segments_do_not_drop_compressed_pictures() {
        let packet = vec![0, 0, 0, 1, 40, 1, 0x80];
        let mut input = packet_stream(&[Vec::new()]);
        input.extend(b"RIFF"); input.extend(u32::MAX.to_le_bytes()); input.extend(b"AVIXLIST");
        input.extend(u32::MAX.to_le_bytes()); input.extend(b"movi00dc");
        input.extend((packet.len() as u32).to_le_bytes()); input.extend(&packet); input.push(0);
        let mut input = std::io::Cursor::new(input);
        read_avi_header(&mut input).unwrap();
        assert_eq!(read_avi_packet(&mut input).unwrap(), packet);
    }

    #[test]
    fn malformed_oversized_and_truncated_packets_fail_before_delivery() {
        assert!(read_avi_header(&mut std::io::Cursor::new([0u8; 12])).is_err());
        for tag in [b"00dc", b"JUNK"] {
            let mut bytes = tag.to_vec();
            bytes.extend((MAX_PACKET_BYTES as u32 + 1).to_le_bytes());
            assert_eq!(read_avi_packet(&mut std::io::Cursor::new(bytes)).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        }
        let packet = vec![0, 0, 0, 1, 40, 1, 0x80];
        let mut bytes = packet_stream(&[packet]); bytes.truncate(bytes.len()-2);
        let mut input = std::io::Cursor::new(bytes); read_avi_header(&mut input).unwrap();
        assert_eq!(read_avi_packet(&mut input).unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
        let mut input = std::io::Cursor::new(packet_stream(&[vec![1, 2, 3, 4]]));
        read_avi_header(&mut input).unwrap();
        assert_eq!(read_avi_packet(&mut input).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn detects_idr_keyframe() {
        let idr = vec![0, 0, 0, 1, 40, 0x00]; // type 20
        assert!(is_hevc_keyframe(&idr));
        let trail = vec![0, 0, 0, 1, 0x02, 0x00];
        assert!(!is_hevc_keyframe(&trail));
    }
}
