//! Windows temporary default-render routing, guarded by a dedicated child.
//!
//! The chosen endpoint is explicit; metadata does not classify an endpoint as
//! virtual or establish ownership. Per-app Windows output overrides are not
//! inspected or changed, and may continue to play on a physical device. Callers
//! must disclose this before offering private mode.
//!
//! `worker_main` must be wired to a private CLI entry in the same executable.
//! Its stdin is a lifetime lease: the parent sends one endpoint ID, then keeps
//! the pipe open. EOF (including parent process death) causes restoration.
//! Killing the worker itself or power loss cannot be recovered by this guard.

use std::ffi::c_void;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;

pub const WORKER_ARG: &str = "--native-audio-route-worker";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

unsafe extern "C" {
    fn weavelo_win_audio_endpoints(
        cb: unsafe extern "C" fn(
            *mut c_void,
            *const u16,
            *const u16,
            *const u16,
            *const u16,
            *const u16,
            *const u16,
        ),
        context: *mut c_void,
    ) -> i32;
    fn weavelo_win_audio_defaults(
        cb: unsafe extern "C" fn(*mut c_void, u32, *const u16),
        context: *mut c_void,
    ) -> i32;
    fn weavelo_win_audio_policy_probe() -> i32;
    fn weavelo_win_audio_route_start(
        target: *const u16,
        out: *mut *mut c_void,
        rollback_error: *mut i32,
        rollback_roles: *mut u32,
    ) -> i32;
    fn weavelo_win_audio_route_restore(handle: *mut c_void, failed_roles: *mut u32) -> i32;
    fn weavelo_win_audio_route_destroy(handle: *mut c_void);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderEndpoint {
    pub id: String,
    pub name: String,
    pub instance_id: String,
    pub hardware_id: String,
    pub manufacturer: String,
    pub enumerator: String,
}

fn hr(operation: &str, code: i32) -> io::Result<()> {
    if code < 0 {
        Err(io::Error::other(format!(
            "{operation}: HRESULT 0x{:08x}",
            code as u32
        )))
    } else {
        Ok(())
    }
}

// Windows APIs provide NUL-terminated strings that live through the callback.
unsafe fn wide(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
}

pub fn render_endpoints() -> io::Result<Vec<RenderEndpoint>> {
    unsafe extern "C" fn collect(
        ctx: *mut c_void,
        id: *const u16,
        name: *const u16,
        instance: *const u16,
        hardware: *const u16,
        manufacturer: *const u16,
        enumerator: *const u16,
    ) {
        let endpoints = unsafe { &mut *(ctx as *mut Vec<RenderEndpoint>) };
        endpoints.push(RenderEndpoint {
            id: unsafe { wide(id) },
            name: unsafe { wide(name) },
            instance_id: unsafe { wide(instance) },
            hardware_id: unsafe { wide(hardware) },
            manufacturer: unsafe { wide(manufacturer) },
            enumerator: unsafe { wide(enumerator) },
        });
    }
    let mut endpoints = Vec::new();
    hr("enumerate active render endpoints", unsafe {
        weavelo_win_audio_endpoints(collect, &mut endpoints as *mut _ as *mut c_void)
    })?;
    Ok(endpoints)
}

/// IDs indexed by console, multimedia, communications role.
pub fn default_render_roles() -> io::Result<[String; 3]> {
    unsafe extern "C" fn collect(ctx: *mut c_void, role: u32, id: *const u16) {
        if let Some(slot) = unsafe { &mut *(ctx as *mut [String; 3]) }.get_mut(role as usize) {
            *slot = unsafe { wide(id) };
        }
    }
    let mut ids: [String; 3] = Default::default();
    hr("read render defaults", unsafe {
        weavelo_win_audio_defaults(collect, &mut ids as *mut _ as *mut c_void)
    })?;
    Ok(ids)
}

/// Only tests whether the undocumented COM interface is available.
pub fn probe_default_setter() -> io::Result<()> {
    hr("IPolicyConfig unavailable", unsafe {
        weavelo_win_audio_policy_probe()
    })
}

struct Route {
    handle: *mut c_void,
    _same_thread: PhantomData<Rc<()>>,
}
impl Route {
    fn start(id: &str) -> io::Result<Self> {
        if id.is_empty() || id.len() > 4096 || id.contains(['\0', '\n', '\r']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid endpoint ID",
            ));
        }
        let wide: Vec<u16> = std::ffi::OsStr::new(id)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut handle = std::ptr::null_mut();
        let mut rollback = 0;
        let mut roles = 0;
        let code = unsafe {
            weavelo_win_audio_route_start(wide.as_ptr(), &mut handle, &mut rollback, &mut roles)
        };
        if code < 0 {
            return Err(io::Error::other(format!("set render defaults: HRESULT 0x{:08x}; rollback HRESULT 0x{:08x}, failed role mask 0x{roles:x}", code as u32, rollback as u32)));
        }
        Ok(Self {
            handle,
            _same_thread: PhantomData,
        })
    }
    fn restore(&mut self) -> io::Result<()> {
        let mut roles = 0;
        let code = unsafe { weavelo_win_audio_route_restore(self.handle, &mut roles) };
        if code < 0 {
            return Err(io::Error::other(format!(
                "restore render defaults: HRESULT 0x{:08x}, failed role mask 0x{roles:x}",
                code as u32
            )));
        }
        Ok(())
    }
}
impl Drop for Route {
    fn drop(&mut self) {
        unsafe { weavelo_win_audio_route_destroy(self.handle) }
    }
}

/// Child CLI entry. Only call in the dedicated worker process, not the GUI.
/// Errors, including failed rollback, go to stderr and a non-zero process exit.
pub fn worker_main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    let n = (&mut input).take(4098).read_line(&mut line)?;
    if n == 0 || !line.ends_with('\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing endpoint ID",
        ));
    }
    line.pop();
    if line.ends_with('\r') {
        line.pop();
    }
    let mut route = Route::start(&line)?;
    if let Err(err) = writeln!(io::stdout().lock(), "READY").and_then(|_| io::stdout().flush()) {
        let restore = route.restore();
        return Err(io::Error::other(format!(
            "worker readiness failed: {err}; restore: {restore:?}"
        )));
    }
    let mut sink = io::sink();
    let read = io::copy(&mut input, &mut sink);
    let restored = route.restore();
    read.and(restored)
}

/// Owns the parent end of the worker lease. Supply a Command configured to run
/// `worker_main`; its stdin/stdout are reserved by this function. Explicitly call
/// `close` to observe restore failures. Drop closes the pipe but cannot report.
pub struct RouteWorker {
    child: Child,
    lease: Option<ChildStdin>,
}
impl RouteWorker {
    pub fn spawn_from_current_exe(endpoint: &str) -> io::Result<Self> {
        let mut command = Command::new(std::env::current_exe()?.canonicalize()?);
        command.arg(WORKER_ARG).creation_flags(CREATE_NO_WINDOW);
        Self::spawn(&mut command, endpoint)
    }

    pub fn spawn(command: &mut Command, endpoint: &str) -> io::Result<Self> {
        if endpoint.is_empty() || endpoint.len() > 4096 || endpoint.contains(['\0', '\n', '\r']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid endpoint ID",
            ));
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut lease = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("missing worker stdin"))?;
        let startup = (|| {
            writeln!(lease, "{endpoint}")?;
            lease.flush()?;
            let mut response = String::new();
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("missing worker stdout"))?;
            BufReader::new(stdout).take(128).read_line(&mut response)?;
            if response.trim_end_matches(['\r', '\n']) != "READY" {
                return Err(io::Error::other(
                    "audio route worker failed before readiness; see worker stderr",
                ));
            }
            Ok(())
        })();
        if let Err(error) = startup {
            drop(lease);
            let _ = child.wait();
            return Err(error);
        }
        Ok(Self {
            child,
            lease: Some(lease),
        })
    }
    pub fn close(mut self) -> io::Result<()> {
        self.lease.take();
        let status = self.child.wait()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "audio route worker restore failed: {status}; see worker stderr"
            )));
        }
        Ok(())
    }
}
impl Drop for RouteWorker {
    fn drop(&mut self) {
        self.lease.take(); /* Child restores on EOF; use close for its result. */
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_multiline_or_oversized_endpoint_ids_before_spawning() {
        let mut cmd = Command::new("cmd.exe");
        assert_eq!(
            RouteWorker::spawn(&mut cmd, "").err().map(|e| e.kind()),
            Some(io::ErrorKind::InvalidInput)
        );
        assert_eq!(
            RouteWorker::spawn(&mut cmd, "bad\nline")
                .err()
                .map(|e| e.kind()),
            Some(io::ErrorKind::InvalidInput)
        );
        let huge = "x".repeat(4097);
        assert_eq!(
            RouteWorker::spawn(&mut cmd, &huge).err().map(|e| e.kind()),
            Some(io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn enumerates_render_endpoints_without_mutating_system_defaults() {
        let endpoints = render_endpoints().expect("MMDeviceEnumerator should execute");
        for ep in &endpoints {
            assert!(!ep.id.is_empty());
        }
        if !endpoints.is_empty() {
            let roles = default_render_roles().expect("active render system has role defaults");
            for role_id in &roles {
                assert!(!role_id.is_empty());
            }
        }
        let _ = probe_default_setter();
    }
}
