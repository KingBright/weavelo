//! Explicitly published open file handles. Remote callers never supply local paths.
//! Publications last for this process and device-group key; clearing prevents new pulls.
use protocol::shared_files::{MAX_SHARED_NAME_BYTES, SHARED_PAGE_SIZE, SharedFileInfo};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
pub type ShareScope = Option<[u8; 32]>;
pub fn share_scope_for_key(key:&[u8])->[u8;32] {Sha256::digest(key).into()}
pub fn current_share_scope() -> ShareScope {
    crate::session_crypto::load_session_psk().map(|key|share_scope_for_key(&key))
}
#[derive(Debug)]
pub struct SharedFile {
    pub info: SharedFileInfo,
    pub file: Arc<tokio::sync::Mutex<File>>,
    scope: ShareScope,
    publication: Option<(String, PathBuf, String)>,
}
#[derive(Debug, Default)]
pub struct SharedFileCatalog {
    entries: Mutex<BTreeMap<u64, Arc<SharedFile>>>,
}
static CATALOG: OnceLock<Arc<SharedFileCatalog>> = OnceLock::new();
pub fn shared_file_catalog() -> Arc<SharedFileCatalog> {
    CATALOG
        .get_or_init(|| Arc::new(SharedFileCatalog::default()))
        .clone()
}
impl SharedFileCatalog {
    pub async fn publish(&self, path: &Path, scope: ShareScope) -> Result<SharedFileInfo, String> {
        let (file, name, size) = Self::open_selected(path, scope).await?;
        self.insert(file, name, size, scope, None)
    }

    /// Open and verify in the serving process. Only verified bytes become
    /// visible, and an uncertain local request can reuse its publication ID.
    pub async fn publish_verified(
        &self, path: &Path, scope: ShareScope, request_id: &str, sha256: &str,
    ) -> Result<SharedFileInfo, String> {
        if request_id.is_empty() || request_id.len() > 128
            || !request_id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || sha256.len() != 64
            || !sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        { return Err("Invalid publication identity or SHA-256".into()); }
        let binding = (request_id.to_owned(), path.to_owned(), sha256.to_owned());
        if let Some(info) = self.existing(scope, &binding)? { return Ok(info); }
        let (mut file, name, size) = Self::open_selected(path, scope).await?;
        let mut digest = Sha256::new();
        let mut buffer = [0; 65536];
        let mut bytes = 0u64;
        loop {
            let count = file.read(&mut buffer).await.map_err(|_| "Cannot read selected file")?;
            if count == 0 { break; }
            bytes += count as u64;
            if bytes > size { return Err("Selected file changed during verification".into()); }
            digest.update(&buffer[..count]);
        }
        if bytes != size || format!("{:x}", digest.finalize()) != sha256 {
            return Err("Selected file content differs from the confirmed SHA-256".into());
        }
        file.rewind().await.map_err(|_| "Cannot rewind selected file")?;
        self.insert(file, name, size, scope, Some(binding))
    }

    fn existing(&self, scope: ShareScope, binding: &(String, PathBuf, String)) -> Result<Option<SharedFileInfo>, String> {
        Self::existing_in(&self.entries.lock().unwrap(), scope, binding)
    }
    fn existing_in(entries: &BTreeMap<u64, Arc<SharedFile>>, scope: ShareScope, binding: &(String, PathBuf, String)) -> Result<Option<SharedFileInfo>, String> {
        if let Some(entry) = entries.values().find(|e| e.publication.as_ref().is_some_and(|p| p.0 == binding.0)) {
            if scope.is_some() && entry.scope == scope && entry.publication.as_ref() == Some(binding) {
                return Ok(Some(entry.info.clone()));
            }
            return Err("Publication request ID is already bound to another file or group".into());
        }
        Ok(None)
    }
    async fn open_selected(path: &Path, scope: ShareScope) -> Result<(File, String, u64), String> {
        if scope.is_none() {
            return Err("Join a device network before sharing files".into());
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= MAX_SHARED_NAME_BYTES
                    && *s != "."
                    && *s != ".."
                    && !s
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
            })
            .ok_or("Unsupported shared file name")?
            .to_string();
        // Open once at explicit local selection. A later path/symlink replacement
        // cannot redirect a remote fetch to another file.
        let file = File::open(path)
            .await
            .map_err(|_| "Cannot open selected file")?;
        let metadata = file
            .metadata()
            .await
            .map_err(|_| "Cannot inspect selected file")?;
        if !metadata.is_file() {
            return Err("Select a regular file, not a directory".into());
        }
        if metadata.len() > crate::file_transfer::FileTransferPolicy::default().max_file_bytes {
            return Err("Selected file exceeds the transfer limit".into());
        }
        Ok((file, name, metadata.len()))
    }
    fn insert(&self, file: File, name: String, size_bytes: u64, scope: ShareScope, publication: Option<(String, PathBuf, String)>) -> Result<SharedFileInfo, String> {
        let mut entries = self.entries.lock().unwrap();
        if let Some(binding) = &publication
            && let Some(info) = Self::existing_in(&entries, scope, binding)? { return Ok(info); }
        entries.retain(|_, e| e.scope == scope);
        if entries.len() >= 64 {
            return Err("Clear shared files before publishing more than 64 files".into());
        }
        let id = loop {
            let id = rand::random::<u64>() & i64::MAX as u64;
            if id != 0 && !entries.contains_key(&id) {
                break id;
            }
        };
        let info = SharedFileInfo {
            id,
            name,
            size_bytes,
        };
        entries.insert(
            id,
            Arc::new(SharedFile {
                info: info.clone(),
                file: Arc::new(tokio::sync::Mutex::new(file)),
                scope,
                publication,
            }),
        );
        Ok(info)
    }
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }
    pub fn revoke(&self, scope: ShareScope, id: u64) {
        let mut entries = self.entries.lock().unwrap();
        if entries.get(&id).is_some_and(|entry| entry.scope == scope) { entries.remove(&id); }
    }
    pub fn page(&self, scope: ShareScope, after: u64) -> (Vec<SharedFileInfo>, Option<u64>) {
        let entries = self.entries.lock().unwrap();
        let mut matches = entries
            .values()
            .filter(|f| f.scope == scope && scope.is_some() && f.info.id > after);
        let page: Vec<_> = matches
            .by_ref()
            .take(SHARED_PAGE_SIZE)
            .map(|f| f.info.clone())
            .collect();
        let next = matches.next().and_then(|_| page.last().map(|f| f.id));
        (page, next)
    }
    pub fn get(&self, scope: ShareScope, id: u64) -> Option<Arc<SharedFile>> {
        self.entries
            .lock()
            .unwrap()
            .get(&id)
            .filter(|f| scope.is_some() && f.scope == scope)
            .cloned()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn verified_publication_is_invisible_on_mismatch_and_reuses_its_open_handle() {
        let dir = std::env::temp_dir().join(format!("weavelo-publish-{}", rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("selected.txt");
        std::fs::write(&path, b"selected").unwrap();
        let catalog = SharedFileCatalog::default();
        let scope = Some([31; 32]);
        assert!(catalog.publish_verified(&path, scope, "request-1", &"0".repeat(64)).await.is_err());
        assert!(catalog.page(scope, 0).0.is_empty());
        let sha = format!("{:x}", Sha256::digest(b"selected"));
        let selected = catalog.publish_verified(&path, scope, "request-1", &sha).await.unwrap();
        std::fs::rename(&path, dir.join("original.txt")).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        let repeated = catalog.publish_verified(&path, scope, "request-1", &sha).await.unwrap();
        assert_eq!(selected.id, repeated.id);
        assert_eq!(catalog.page(scope, 0).0.len(), 1);
        let original = catalog.get(scope, selected.id).unwrap();
        let mut actual = Vec::new();
        original.file.lock().await.read_to_end(&mut actual).await.unwrap();
        assert_eq!(actual, b"selected");
        assert!(catalog.publish_verified(&path, scope, "request-1", &"1".repeat(64)).await.is_err());
        assert!(catalog.publish_verified(&path, Some([32; 32]), "request-1", &sha).await.is_err());
        assert!(catalog.page(Some([32; 32]), 0).0.is_empty());
        catalog.revoke(Some([32; 32]), selected.id);
        assert!(catalog.get(scope, selected.id).is_some());
        catalog.revoke(scope, selected.id);
        assert!(catalog.page(scope, 0).0.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn only_explicit_files_are_listed_and_scopes_are_isolated() {
        let dir = std::env::temp_dir().join(format!("rp-shared-{}", rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("selected.txt");
        std::fs::write(&path, b"selected").unwrap();
        let catalog = SharedFileCatalog::default();
        let scope = Some([7; 32]);
        assert!(catalog.page(scope, 0).0.is_empty());
        assert!(catalog.publish(&path, None).await.is_err());
        let info = catalog.publish(&path, scope).await.unwrap();
        assert_eq!(catalog.page(scope, 0).0, vec![info.clone()]);
        assert!(catalog.get(Some([8; 32]), info.id).is_none());
        catalog.clear();
        assert!(catalog.get(scope, info.id).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn held_file_does_not_follow_replaced_path() {
        use tokio::io::AsyncReadExt;
        let dir = std::env::temp_dir().join(format!("rp-pinned-{}", rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("selected.txt");
        std::fs::write(&path, b"original").unwrap();
        let catalog = SharedFileCatalog::default();
        let scope = Some([9; 32]);
        let info = catalog.publish(&path, scope).await.unwrap();
        std::fs::rename(&path, dir.join("original.txt")).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        let entry = catalog.get(scope, info.id).unwrap();
        let mut file = entry.file.lock().await;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, b"original");
        drop(file);
        drop(entry);
        catalog.clear();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
