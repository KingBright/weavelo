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

/// Persistent credential store for paired PS5 consoles (`0o600` on Unix).
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
            .join("weavelo")
            .join("ps5_consoles.json")
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
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let payload = ConsoleStoreFile {
            version: 1,
            consoles: self.consoles.clone(),
        };
        let json = serde_json::to_vec_pretty(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let tmp_path = self.path.with_extension("json.tmp");
        write_private_file(&tmp_path, &json)?;
        if self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
        std::fs::rename(&tmp_path, &self.path)
    }
}

fn write_private_file(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).write(true).truncate(true);
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
    Ok(())
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
