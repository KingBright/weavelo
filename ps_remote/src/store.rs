use serde::{Deserialize, Serialize};
use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Derives a deterministic non-zero IEEE-754 / `org.json` safe positive integer
/// identifier (53-bit) from a PS5 `host-id`.
pub fn console_id_from_host_id(host_id: &str) -> u64 {
    let normalized = host_id.trim().to_ascii_uppercase();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in normalized.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let folded = (hash ^ (hash >> 53)) & ((1u64 << 53) - 1);
    if folded == 0 {
        1
    } else {
        folded
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedConsole {
    pub console_id: u64,
    pub host_id: String,
    pub nickname: String,
    pub host_addr: Option<IpAddr>,
    pub account_id: [u8; 8],
    pub rp_regist_key: [u8; 16],
    pub rp_key: [u8; 16],
    pub rp_key_type: u32,
    pub ps5_mac: [u8; 6],
    pub paired_at_unix: u64,
}

impl PairedConsole {
    pub fn now_unix_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConsoleStoreFile {
    version: u32,
    consoles: Vec<PairedConsole>,
}

const MAX_STORE_FILE_BYTES: u64 = 1024 * 1024;

/// Persistent credential store for paired PS5 consoles (`0o600` on Unix, owner-only DACL on Windows).
#[derive(Debug, Clone)]
pub struct ConsoleStore {
    path: PathBuf,
    consoles: Vec<PairedConsole>,
}

impl ConsoleStore {
    pub fn default_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".config")));
        #[cfg(not(target_os = "windows"))]
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));

        base.unwrap_or_else(|| PathBuf::from("."))
            .join("ps_remote")
            .join("consoles.json")
    }

    pub fn load_default() -> io::Result<Self> {
        Self::load(Self::default_path())
    }

    pub fn load(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                consoles: Vec::new(),
            });
        }
        let _guard = StoreFileLock::acquire(&path, false)?;
        Self::load_unlocked(path)
    }

    /// Atomically locks, loads, mutates, and saves the console store at `path`.
    pub fn update<R>(
        path: impl Into<PathBuf>,
        mutate: impl FnOnce(&mut Self) -> R,
    ) -> io::Result<R> {
        let path = path.into();
        ensure_private_parent_dir(&path)?;
        let _guard = StoreFileLock::acquire(&path, true)?;
        let mut store = Self::load_unlocked(path)?;
        let out = mutate(&mut store);
        store.save_unlocked()?;
        Ok(out)
    }

    fn load_unlocked(path: PathBuf) -> io::Result<Self> {
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    path,
                    consoles: Vec::new(),
                });
            }
            Err(e) => return Err(e),
        };
        if meta.len() > MAX_STORE_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "console store exceeds 1 MiB size limit",
            ));
        }
        let bytes = std::fs::read(&path)?;
        if bytes.is_empty() {
            return Ok(Self {
                path,
                consoles: Vec::new(),
            });
        }
        let parsed: ConsoleStoreFile = serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Self {
            path,
            consoles: parsed.consoles,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> &[PairedConsole] {
        &self.consoles
    }

    pub fn get(&self, console_id: u64) -> Option<&PairedConsole> {
        self.consoles.iter().find(|c| c.console_id == console_id)
    }

    pub fn get_by_host_id(&self, host_id: &str) -> Option<&PairedConsole> {
        self.consoles
            .iter()
            .find(|c| c.host_id.eq_ignore_ascii_case(host_id.trim()))
    }

    pub fn upsert(&mut self, console: PairedConsole) {
        if let Some(existing) = self
            .consoles
            .iter_mut()
            .find(|c| c.console_id == console.console_id || c.host_id.eq_ignore_ascii_case(&console.host_id))
        {
            *existing = console;
        } else {
            self.consoles.push(console);
        }
    }

    pub fn remove(&mut self, console_id: u64) -> Option<PairedConsole> {
        let pos = self
            .consoles
            .iter()
            .position(|c| c.console_id == console_id)?;
        Some(self.consoles.remove(pos))
    }

    pub fn save(&self) -> io::Result<()> {
        ensure_private_parent_dir(&self.path)?;
        let _guard = StoreFileLock::acquire(&self.path, true)?;
        self.save_unlocked()
    }

    fn save_unlocked(&self) -> io::Result<()> {
        ensure_private_parent_dir(&self.path)?;
        let payload = ConsoleStoreFile {
            version: 1,
            consoles: self.consoles.clone(),
        };
        let json = serde_json::to_vec_pretty(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let file_name = self
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("consoles.json");
        let nonce: u64 = rand::random();
        let tmp_path = self
            .path
            .with_file_name(format!(".{file_name}.tmp-{}-{nonce:016x}", std::process::id()));
        let write_res = (|| -> io::Result<()> {
            write_private_file(&tmp_path, &json)?;
            #[cfg(windows)]
            if self.path.exists() {
                let _ = std::fs::remove_file(&self.path);
            }
            std::fs::rename(&tmp_path, &self.path)?;
            #[cfg(windows)]
            restrict_windows_file_to_current_user(&self.path)?;
            Ok(())
        })();
        if write_res.is_err() {
            let _ = std::fs::remove_file(&tmp_path);
        }
        write_res
    }
}

struct StoreFileLock(std::fs::File);

impl StoreFileLock {
    fn acquire(store_path: &Path, exclusive: bool) -> io::Result<Self> {
        ensure_private_parent_dir(store_path)?;
        let file_name = store_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("consoles.json");
        let lock_path = store_path.with_file_name(format!(".{file_name}.lock"));
        let mut opts = std::fs::OpenOptions::new();
        opts.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(&lock_path)?;
        #[cfg(windows)]
        let _ = restrict_windows_file_to_current_user(&lock_path);
        if exclusive {
            file.lock()?;
        } else {
            file.lock_shared()?;
        }
        Ok(Self(file))
    }
}

impl Drop for StoreFileLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn ensure_private_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if !parent.exists() {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(parent)?;
        }
    }
    Ok(())
}

fn write_private_file(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    restrict_windows_file_to_current_user(path)?;
    Ok(())
}

#[cfg(windows)]
fn restrict_windows_file_to_current_user(path: &Path) -> io::Result<()> {
    use core::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    struct SidAndAttributes {
        sid: *mut c_void,
        attributes: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(
            process: *mut c_void,
            desired_access: u32,
            token: *mut *mut c_void,
        ) -> i32;
        fn GetTokenInformation(
            token: *mut c_void,
            info_class: u32,
            info: *mut c_void,
            info_len: u32,
            return_len: *mut u32,
        ) -> i32;
        fn ConvertSidToStringSidW(sid: *mut c_void, string_sid: *mut *mut u16) -> i32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            string_sd: *const u16,
            revision: u32,
            sd: *mut *mut c_void,
            sd_size: *mut u32,
        ) -> i32;
        fn GetSecurityDescriptorDacl(
            sd: *mut c_void,
            dacl_present: *mut i32,
            dacl: *mut *mut c_void,
            dacl_defaulted: *mut i32,
        ) -> i32;
        fn SetNamedSecurityInfoW(
            object_name: *const u16,
            object_type: u32,
            security_info: u32,
            owner: *mut c_void,
            group: *mut c_void,
            dacl: *mut c_void,
            sacl: *mut c_void,
        ) -> u32;
    }

    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_USER_CLASS: u32 = 1;
    const SDDL_REVISION_1: u32 = 1;
    const SE_FILE_OBJECT: u32 = 1;
    const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
    const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;

    unsafe {
        let mut token: *mut c_void = core::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut needed: u32 = 0;
        let _ = GetTokenInformation(
            token,
            TOKEN_USER_CLASS,
            core::ptr::null_mut(),
            0,
            &mut needed,
        );
        if needed == 0 {
            let err = io::Error::last_os_error();
            let _ = CloseHandle(token);
            return Err(err);
        }
        let mut buf = vec![0u8; needed as usize];
        if GetTokenInformation(
            token,
            TOKEN_USER_CLASS,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        ) == 0
        {
            let err = io::Error::last_os_error();
            let _ = CloseHandle(token);
            return Err(err);
        }
        let _ = CloseHandle(token);

        let user = &*(buf.as_ptr().cast::<SidAndAttributes>());
        let mut sid_ptr: *mut u16 = core::ptr::null_mut();
        if ConvertSidToStringSidW(user.sid, &mut sid_ptr) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0usize;
        while *sid_ptr.add(len) != 0 {
            len += 1;
        }
        let sid_str = String::from_utf16_lossy(core::slice::from_raw_parts(sid_ptr, len));
        let _ = LocalFree(sid_ptr.cast());

        let sddl = format!("D:P(A;;FA;;;{sid_str})(A;;FA;;;SY)(A;;FA;;;BA)");
        let sddl_w: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut sd: *mut c_void = core::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_w.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            core::ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut present: i32 = 0;
        let mut defaulted: i32 = 0;
        let mut dacl: *mut c_void = core::ptr::null_mut();
        if GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) == 0
            || present == 0
        {
            let err = io::Error::last_os_error();
            let _ = LocalFree(sd);
            return Err(err);
        }
        let path_w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let status = SetNamedSecurityInfoW(
            path_w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            dacl,
            core::ptr::null_mut(),
        );
        let _ = LocalFree(sd);
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn roundtrips_store_atomically() {
        let dir = std::env::temp_dir().join(format!("weavelo_ps5_store_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("ps5_consoles.json");

        let mut store = ConsoleStore::load(&path).unwrap();
        assert!(store.list().is_empty());

        let id = console_id_from_host_id("aabbccddeeff");
        assert_eq!(id, console_id_from_host_id("AABBCCDDEEFF"));
        assert_ne!(id, 0);
        assert!(id <= (1u64 << 53) - 1);

        let console = PairedConsole {
            console_id: id,
            host_id: "AABBCCDDEEFF".to_string(),
            nickname: "LivingRoom".to_string(),
            host_addr: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50))),
            account_id: [1, 2, 3, 4, 5, 6, 7, 8],
            rp_regist_key: *b"12345678\0\0\0\0\0\0\0\0",
            rp_key: [0xaa; 16],
            rp_key_type: 2,
            ps5_mac: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
            paired_at_unix: 1_700_000_000,
        };
        store.upsert(console.clone());
        store.save().unwrap();

        let reloaded = ConsoleStore::load(&path).unwrap();
        assert_eq!(reloaded.get(id), Some(&console));
        assert_eq!(reloaded.get_by_host_id("aabbccddeeff"), Some(&console));

        let mut mutable = reloaded;
        assert!(mutable.remove(id).is_some());
        mutable.save().unwrap();
        assert!(ConsoleStore::load(&path).unwrap().list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
