//! Camera portal permission, PipeWire camera identities, and headless V4L2
//! passive camera fallback for embedded/robot Linux endpoints (e.g. Reachy-Mini).
use crate::linux_frame::{
    BorrowedPlane, FrameBudget, FrameFormat, FrameMailbox, FrameStamp, MAX_FRAME_BYTES, OwnedFrame,
    PixelFormat, SourceColor, Transform,
};
use futures_util::StreamExt;
use protocol::session::{CameraFacing, CaptureSource, CaptureSourceInfo};
use remote_core::cameras::{CameraRegistry, NativeCamera};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::Read,
    os::fd::{AsRawFd, OwnedFd},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use zbus::{
    Connection, Proxy,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const CAMERA: &str = "org.freedesktop.portal.Camera";
static ACCESS: AtomicI32 = AtomicI32::new(0);
static REQUEST: AtomicU64 = AtomicU64::new(1);
static V4L2_GENERATION: AtomicU64 = AtomicU64::new(100_000);
type Result<T> = std::result::Result<T, String>;

#[derive(Clone)]
enum NodeKind {
    PipeWire { id: u32, serial: u64, cookie: u32 },
    V4l2 { device_path: PathBuf },
}

#[derive(Clone)]
struct Node {
    kind: NodeKind,
    name: String,
    width: u32,
    height: u32,
}
impl Node {
    fn key(&self) -> String {
        match &self.kind {
            NodeKind::PipeWire { serial, cookie, .. } => format!("{cookie}:{serial}"),
            NodeKind::V4l2 { device_path } => format!("v4l2:{}", device_path.display()),
        }
    }
    fn camera(&self) -> NativeCamera {
        NativeCamera {
            key: self.key(),
            name: self.name.clone(),
            facing: CameraFacing::Unspecified,
            switch_group: None,
            width: self.width,
            height: self.height,
            rotation: 0,
        }
    }
}

struct State {
    registry: CameraRegistry,
    nodes: HashMap<String, Node>,
    cache: Option<(Instant, Vec<CaptureSourceInfo>)>,
}
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(State {
            registry: CameraRegistry::default(),
            nodes: HashMap::new(),
            cache: None,
        })
    })
}
pub fn access() -> i32 {
    ACCESS.load(Ordering::Relaxed)
}
async fn portal() -> Result<(Connection, Proxy<'static>)> {
    let connection = zbus::connection::Builder::session()
        .map_err(|e| e.to_string())?
        .method_timeout(Duration::from_secs(5))
        .build()
        .await
        .map_err(|e| e.to_string())?;
    let proxy = Proxy::new_owned(connection.clone(), DEST, PATH, CAMERA)
        .await
        .map_err(|e| e.to_string())?;
    Ok((connection, proxy))
}
async fn remote() -> Result<(Connection, OwnedFd)> {
    let (connection, proxy) = portal().await?;
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    let fd: zbus::zvariant::OwnedFd = proxy
        .call("OpenPipeWireRemote", &(options,))
        .await
        .map_err(|_| "Allow camera in Weavelo Sharing & permissions on this Linux device".to_string())?;
    ACCESS.store(3, Ordering::Relaxed);
    Ok((connection, fd.into()))
}
struct PendingRequest {
    connection: Connection,
    path: String,
    complete: bool,
}
impl Drop for PendingRequest {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        let connection = self.connection.clone();
        let path = self.path.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Ok(proxy) =
                    Proxy::new_owned(connection, DEST, path, "org.freedesktop.portal.Request").await
                {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(3),
                        proxy.call::<_,_,()>("Close", &()),
                    )
                    .await;
                }
            });
        }
    }
}
/// Requests real local portal consent; it never answers the system dialog.
pub async fn request_access() -> Result<bool> {
    let (connection, proxy) = portal().await?;
    let sender = connection
        .unique_name()
        .ok_or("Camera portal connection has no unique name")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let token = format!(
        "weavelo_camera_{}_{}",
        std::process::id(),
        REQUEST.fetch_add(1, Ordering::Relaxed)
    );
    let path = format!("{PATH}/request/{sender}/{token}");
    let mut pending = PendingRequest {
        connection: connection.clone(),
        path: path.clone(),
        complete: false,
    };
    let request = Proxy::new_owned(
        connection,
        DEST,
        path.clone(),
        "org.freedesktop.portal.Request",
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut responses = request
        .receive_signal("Response")
        .await
        .map_err(|e| e.to_string())?;
    let mut options = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    let returned: OwnedObjectPath = proxy
        .call("AccessCamera", &(options,))
        .await
        .map_err(|e| e.to_string())?;
    if returned.as_str() != path {
        return Err("Camera portal changed its request identity".into());
    }
    let signal = tokio::time::timeout(Duration::from_secs(120), responses.next())
        .await
        .map_err(|_| "Camera permission request timed out".to_string())?
        .ok_or("Camera permission request closed")?;
    let (code, _): (u32, HashMap<String, OwnedValue>) =
        signal.body().deserialize().map_err(|e| e.to_string())?;
    pending.complete = true;
    ACCESS.store(if code == 0 { 3 } else { 2 }, Ordering::Relaxed);
    state().lock().unwrap_or_else(|e| e.into_inner()).cache = None;
    Ok(code == 0)
}

fn inventory(fd: OwnedFd) -> Result<Vec<Node>> {
    use pipewire as pw;
    crate::linux_pipewire::native::initialize();
    let loop_ = pw::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pw::context::ContextRc::new(&loop_, None).map_err(|e| e.to_string())?;
    let core = context.connect_fd_rc(fd, None).map_err(|e| e.to_string())?;
    let registry = core.get_registry().map_err(|e| e.to_string())?;
    let nodes = Rc::new(RefCell::new(HashMap::new()));
    let cookie = Rc::new(Cell::new(None));
    let complete = Rc::new(Cell::new(false));
    let failed = Rc::new(Cell::new(false));
    let pending = core.sync(0).map_err(|e| e.to_string())?;
    let (end, done, server, error) = (loop_.clone(), complete.clone(), cookie.clone(), failed.clone());
    let error_loop = loop_.clone();
    let _core = core
        .add_listener_local()
        .info(move |info| server.set(Some(info.cookie())))
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                done.set(true);
                end.quit();
            }
        })
        .error(move |_, _, _, _| {
            error.set(true);
            error_loop.quit();
        })
        .register();
    let added = nodes.clone();
    let removed = nodes.clone();
    let _registry = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ != pw::types::ObjectType::Node {
                return;
            }
            let Some(properties) = global.props.as_ref() else {
                return;
            };
            if !properties
                .get("media.class")
                .is_some_and(|value| value.starts_with("Video/Source"))
            {
                return;
            }
            let Some(serial) = properties
                .get("object.serial")
                .and_then(|s| s.parse::<u64>().ok())
            else {
                return;
            };
            let name = properties
                .get("node.description")
                .or_else(|| properties.get("node.nick"))
                .or_else(|| properties.get("node.name"))
                .unwrap_or("Camera")
                .chars()
                .take(256)
                .collect();
            let (width, height) = properties
                .get("video.size")
                .and_then(|s| s.split_once('x'))
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                .unwrap_or((0, 0));
            let mut nodes = added.borrow_mut();
            if nodes.len() <= remote_core::cameras::MAX_CAMERAS {
                nodes.insert(
                    global.id,
                    (global.id, serial, name, width, height),
                );
            }
        })
        .global_remove(move |id| {
            removed.borrow_mut().remove(&id);
        })
        .register();
    let timer_loop = loop_.clone();
    let timer = loop_.loop_().add_timer(move |_| timer_loop.quit());
    timer
        .update_timer(Some(Duration::from_secs(3)), None)
        .into_result()
        .map_err(|e| e.to_string())?;
    loop_.run();
    if failed.get() || !complete.get() {
        return Err("Camera PipeWire enumeration did not complete".into());
    }
    let cookie = cookie
        .get()
        .ok_or("Camera PipeWire server identity is unavailable")?;
    let mut entries: Vec<_> = nodes.borrow().values().cloned().collect();
    if entries.len() > remote_core::cameras::MAX_CAMERAS {
        return Err("Too many camera devices".into());
    }
    entries.sort_by_key(|(_, serial, _, _, _)| *serial);
    Ok(entries
        .into_iter()
        .map(|(id, serial, name, width, height)| Node {
            kind: NodeKind::PipeWire { id, serial, cookie },
            name,
            width,
            height,
        })
        .collect())
}

fn is_v4l2_video_capture_device(path: &Path) -> bool {
    #[repr(C)]
    struct V4l2Capability {
        driver: [u8; 16],
        card: [u8; 32],
        bus_info: [u8; 32],
        version: u32,
        capabilities: u32,
        device_caps: u32,
        reserved: [u32; 3],
    }
    // VIDIOC_QUERYCAP = _IOR('V', 0, struct v4l2_capability) = 0x80685600 on Linux
    const VIDIOC_QUERYCAP: libc::c_ulong = 0x8068_5600;
    const V4L2_CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
    const V4L2_CAP_VIDEO_CAPTURE_MPLANE: u32 = 0x0000_1000;
    const V4L2_CAP_DEVICE_CAPS: u32 = 0x8000_0000;

    let Ok(file) = std::fs::OpenOptions::new().read(true).open(path) else {
        return false;
    };
    let mut cap = V4l2Capability {
        driver: [0; 16],
        card: [0; 32],
        bus_info: [0; 32],
        version: 0,
        capabilities: 0,
        device_caps: 0,
        reserved: [0; 3],
    };
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), VIDIOC_QUERYCAP as _, &mut cap) };
    if rc != 0 {
        return false;
    }
    let effective = if (cap.capabilities & V4L2_CAP_DEVICE_CAPS) != 0 {
        cap.device_caps
    } else {
        cap.capabilities
    };
    (effective & (V4L2_CAP_VIDEO_CAPTURE | V4L2_CAP_VIDEO_CAPTURE_MPLANE)) != 0
}

fn enumerate_headless_v4l2() -> Vec<Node> {
    let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("video"))
        .collect();
    names.sort_by_key(|n| n.trim_start_matches("video").parse::<u32>().unwrap_or(u32::MAX));
    let mut nodes = Vec::new();
    for dev_name in names {
        if nodes.len() >= remote_core::cameras::MAX_CAMERAS {
            break;
        }
        let dev_path = PathBuf::from("/dev").join(&dev_name);
        if !is_v4l2_video_capture_device(&dev_path) {
            continue;
        }
        let sys_name_path = PathBuf::from("/sys/class/video4linux")
            .join(&dev_name)
            .join("name");
        let label = std::fs::read_to_string(sys_name_path)
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Camera ({dev_name})"));
        let lower = label.to_ascii_lowercase();
        // Skip internal ISP / codec / loopback metadata devices on Raspberry Pi / embedded SoCs
        if lower.contains("isp") || lower.contains("bcm2835-codec") || lower.contains("pispbe") || lower.contains("rpivid") {
            continue;
        }
        nodes.push(Node {
            kind: NodeKind::V4l2 {
                device_path: dev_path,
            },
            name: label.chars().take(256).collect(),
            width: 1280,
            height: 720,
        });
    }
    nodes
}

fn headless_v4l2_allowed() -> bool {
    (std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none())
        || matches!(
            std::env::var("WEAVELO_V4L2_CAMERA").as_deref(),
            Ok("1" | "true" | "TRUE" | "yes" | "YES")
        )
}

fn query() -> Result<Vec<Node>> {
    let portal_result = std::thread::spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let (_connection, fd) = runtime.block_on(remote())?;
        inventory(fd)
    })
    .join()
    .map_err(|_| "Camera catalog worker failed".to_string())?;

    match portal_result {
        Ok(nodes) if !nodes.is_empty() => Ok(nodes),
        Ok(_) if headless_v4l2_allowed() => {
            let v4l2 = enumerate_headless_v4l2();
            if !v4l2.is_empty() {
                ACCESS.store(3, Ordering::Relaxed);
            }
            Ok(v4l2)
        }
        Ok(nodes) => Ok(nodes),
        Err(_portal_err) if headless_v4l2_allowed() => {
            let v4l2 = enumerate_headless_v4l2();
            if !v4l2.is_empty() {
                ACCESS.store(3, Ordering::Relaxed);
                Ok(v4l2)
            } else {
                Err(_portal_err)
            }
        }
        Err(err) => Err(err),
    }
}

pub(crate) fn list() -> Result<Vec<CaptureSourceInfo>> {
    let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, sources)) = &state.cache
        && at.elapsed() < Duration::from_secs(1)
    {
        return Ok(sources.clone());
    }
    let nodes = match query() {
        Ok(nodes) => nodes,
        Err(error) => {
            ACCESS.store(2, Ordering::Relaxed);
            state.nodes.clear();
            let _ = state.registry.reconcile(Vec::new());
            state.cache = None;
            return Err(error);
        }
    };
    let sources = state
        .registry
        .reconcile(nodes.iter().map(Node::camera).collect())
        .map_err(str::to_owned)?;
    state.nodes = nodes.into_iter().map(|node| (node.key(), node)).collect();
    state.cache = Some((Instant::now(), sources.clone()));
    Ok(sources)
}

struct V4l2CameraWorker {
    mailbox: Arc<FrameMailbox>,
    generation: u64,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Drop for V4l2CameraWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.mailbox.close();
        if let Ok(mut guard) = self.child.lock()
            && let Some(mut child) = guard.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(thread) = self
            .thread
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = std::thread::Builder::new()
                .name("weavelo-v4l2-reap".into())
                .spawn(move || {
                    let _ = thread.join();
                });
        }
    }
}

async fn prepare_v4l2_camera_capture(
    device_path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<V4l2CameraWorker> {
    let out_w = (width.clamp(160, 1920) & !1).max(160);
    let out_h = (height.clamp(120, 1080) & !1).max(120);
    let out_fps = fps.clamp(5, 60);
    let generation = V4L2_GENERATION.fetch_add(1, Ordering::Relaxed);
    let mailbox = Arc::new(
        FrameMailbox::native_stream(generation, Duration::from_millis(250))
            .map_err(|e| format!("{e:?}"))?,
    );
    let y_bytes = (out_w as usize) * (out_h as usize);
    let uv_bytes = y_bytes / 2;
    let total_bytes = y_bytes + uv_bytes;
    let per_frame = total_bytes.max(4 * 1024 * 1024).min(MAX_FRAME_BYTES);
    let budget = FrameBudget::new(per_frame, (per_frame * 3).min(2 * MAX_FRAME_BYTES))
        .map_err(|e| format!("V4L2 buffer budget: {e:?}"))?;

    let spawn_ffmpeg = move |mjpeg_input: bool| -> std::result::Result<Child, String> {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-fflags", "nobuffer", "-flags", "low_delay"]);
        cmd.args(["-f", "v4l2"]);
        if mjpeg_input {
            cmd.args(["-input_format", "mjpeg"]);
        }
        cmd.args([
            "-framerate",
            &out_fps.to_string(),
            "-video_size",
            &format!("{out_w}x{out_h}"),
            "-i",
        ]);
        cmd.arg(&device_path);
        cmd.args([
            "-vf",
            &format!("scale={out_w}:{out_h}:out_range=tv:out_color_matrix=bt709,format=nv12"),
            "-pix_fmt",
            "nv12",
            "-f",
            "rawvideo",
            "pipe:1",
        ]);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        cmd.spawn()
            .map_err(|e| format!("Failed to start ffmpeg for V4L2 camera {}: {e}", device_path.display()))
    };

    let stop = Arc::new(AtomicBool::new(false));
    let child_slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let (first_tx, first_rx) = oneshot::channel::<OwnedFrame>();

    let worker_mailbox = mailbox.clone();
    let worker_stop = stop.clone();
    let worker_child = child_slot.clone();
    let thread = std::thread::Builder::new()
        .name("weavelo-v4l2-cam".into())
        .spawn(move || {
            let mut first_sender = Some(first_tx);
            for mjpeg in [true, false] {
                if worker_stop.load(Ordering::Acquire) {
                    break;
                }
                let Ok(mut child) = spawn_ffmpeg(mjpeg) else {
                    continue;
                };
                let Some(mut stdout) = child.stdout.take() else {
                    let _ = child.kill();
                    let _ = child.wait();
                    continue;
                };
                *worker_child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
                let mut raw = vec![0u8; total_bytes];
                let mut seq = 0u64;
                let mut got_any = false;
                while !worker_stop.load(Ordering::Acquire) && !worker_mailbox.is_closed() {
                    if stdout.read_exact(&mut raw).is_err() {
                        break;
                    }
                    got_any = true;
                    seq += 1;
                    let fmt = FrameFormat {
                        width: out_w,
                        height: out_h,
                        pixel_format: PixelFormat::Nv12,
                        color: SourceColor {
                            range: 2,   // limited range (SPA_VIDEO_COLOR_RANGE_16_235)
                            matrix: 2,  // BT.709 (SPA_VIDEO_COLOR_MATRIX_BT709)
                            transfer: 0,
                            primaries: 0,
                        },
                        crop: None,
                        transform: Transform::Identity,
                    };
                    let planes = [
                        BorrowedPlane {
                            data: &raw[..y_bytes],
                            mapping_offset: 0,
                            chunk_offset: 0,
                            chunk_size: y_bytes as u32,
                            stride: out_w as i32,
                        },
                        BorrowedPlane {
                            data: &raw[y_bytes..],
                            mapping_offset: 0,
                            chunk_offset: 0,
                            chunk_size: uv_bytes as u32,
                            stride: out_w as i32,
                        },
                    ];
                    let stamp = FrameStamp {
                        generation,
                        format_revision: 0,
                        sequence: Some(seq),
                        pipewire_pts_ns: None,
                        arrival_ts_us: remote_core::quanta_now_us(),
                    };
                    if let Ok(owned) = OwnedFrame::copy_from_budget(fmt, &planes, stamp, Some(&budget)) {
                        if let Some(tx) = first_sender.take() {
                            if tx.send(owned).is_err() {
                                break;
                            }
                        } else {
                            worker_mailbox.publish(owned);
                        }
                    }
                }
                if let Ok(mut guard) = worker_child.lock()
                    && let Some(mut running_child) = guard.take()
                {
                    let _ = running_child.kill();
                    let _ = running_child.wait();
                }
                if got_any || first_sender.is_none() {
                    break;
                }
            }
            worker_mailbox.close();
        })
        .map_err(|e| e.to_string())?;

    let worker = V4l2CameraWorker {
        mailbox: mailbox.clone(),
        generation,
        stop,
        child: child_slot,
        thread: Mutex::new(Some(thread)),
    };
    let first = tokio::time::timeout(Duration::from_secs(10), first_rx)
        .await
        .map_err(|_| "V4L2 camera did not produce an initial frame within 10s".to_string())?
        .map_err(|_| "V4L2 camera stream stopped before producing a frame".to_string())?;
    mailbox.publish(first);
    Ok(worker)
}

pub(crate) enum Capture {
    PipeWire {
        native: crate::linux_pipewire::native::CameraCapture,
        _portal: Connection,
    },
    V4l2(V4l2CameraWorker),
}
impl Capture {
    pub(crate) fn mailbox(&self) -> Arc<FrameMailbox> {
        match self {
            Self::PipeWire { native, .. } => native.mailbox.clone(),
            Self::V4l2(v) => v.mailbox.clone(),
        }
    }
    pub(crate) fn generation(&self) -> u64 {
        match self {
            Self::PipeWire { native, .. } => native.generation,
            Self::V4l2(v) => v.generation,
        }
    }
}

pub(crate) async fn start(
    source: CaptureSource,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<Capture> {
    let node = tokio::task::spawn_blocking(move || {
        list()?;
        let state = state().lock().unwrap_or_else(|e| e.into_inner());
        let camera = state
            .registry
            .resolve(source)
            .ok_or("The selected camera is no longer available")?;
        state
            .nodes
            .get(&camera.key)
            .cloned()
            .ok_or("The selected camera is no longer available".to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    match node.kind {
        NodeKind::PipeWire { id, serial, cookie } => {
            let (connection, fd) = remote().await?;
            let native = crate::linux_pipewire::native::prepare_camera_capture(
                fd, id, serial, cookie, width, height, fps,
            )
            .await?;
            Ok(Capture::PipeWire {
                native,
                _portal: connection,
            })
        }
        NodeKind::V4l2 { device_path } => {
            let v4l2 = prepare_v4l2_camera_capture(device_path, width, height, fps).await?;
            Ok(Capture::V4l2(v4l2))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipewire_server_restart_retires_even_a_reused_camera_serial() {
        let node = Node {
            kind: NodeKind::PipeWire {
                id: 3,
                serial: 90,
                cookie: 4,
            },
            name: "Camera".into(),
            width: 0,
            height: 0,
        };
        let mut registry = CameraRegistry::default();
        let first = registry.reconcile(vec![node.camera()]).unwrap();
        let changed = Node {
            kind: NodeKind::PipeWire {
                id: 3,
                serial: 90,
                cookie: 5,
            },
            name: "Camera".into(),
            width: 0,
            height: 0,
        };
        let second = registry.reconcile(vec![changed.camera()]).unwrap();
        assert_ne!(first[0].source, second[0].source);
        assert!(registry.resolve(first[0].source).is_none());
        assert!(!second[0].supports_input);
        assert!(second[0].process_id.is_none());
    }
}
