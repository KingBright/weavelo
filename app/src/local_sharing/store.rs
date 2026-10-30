//! Only the canonical user profile stores OS-issued restore requests.
//! No grant data is logged, exported through IPC or treated as peer identity.
use crate::sharing_permissions::ShareKind;
use host::{PersistentPortalRestore, PickerSources};
use remote_core::shared_files::ShareScope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const APP: &str = "fun.hackerlife.weavelo.desktop";
const LIMIT: u64 = 64 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub id: u64,
    pub kind: ShareKind,
    pub grant: PersistentPortalRestore,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: u32,
    application: String,
    scope: ShareScope,
    records: Vec<Record>,
}

fn invalid() -> io::Error {
    io::Error::other("Saved sharing selection is invalid; existing data was retained")
}
fn validate(raw: &serde_json::Value) -> io::Result<Document> {
    let doc: Document = serde_json::from_value(raw.clone()).map_err(|_| invalid())?;
    if doc.schema != 1
        || doc.application != APP
        || doc.records.len() > 8
        || (doc.scope.is_none() && !doc.records.is_empty())
    {
        return Err(invalid());
    }
    let mut ids = HashSet::new();
    for record in &doc.records {
        let (sources, control) = selection(record.kind);
        if record.id == 0
            || !ids.insert(record.id)
            || !record.grant.matches_selection(sources, control)
        {
            return Err(invalid());
        }
    }
    Ok(doc)
}
pub(super) fn selection(kind: ShareKind) -> (PickerSources, bool) {
    match kind {
        ShareKind::Window => (PickerSources::Window, false),
        ShareKind::Display => (PickerSources::Monitor, false),
        ShareKind::ControlledDisplay => (PickerSources::Monitor, true),
    }
}
fn empty(scope: ShareScope) -> serde_json::Value {
    serde_json::json!({"schema":1,"application":APP,"scope":scope,"records":[]})
}

pub(super) struct Store {
    path: PathBuf,
    raw: serde_json::Value,
    digest: Option<[u8; 32]>,
}
impl Store {
    pub fn load(profile: &Path) -> io::Result<Self> {
        remote_core::desktop_storage::prepare_profile(profile)?;
        let path = profile.join("sharing-grants.json");
        let bytes = Self::read(&path)?;
        let raw = match &bytes {
            Some(bytes) => serde_json::from_slice(bytes).map_err(|_| invalid())?,
            None => empty(None),
        };
        validate(&raw)?;
        Ok(Self {
            path,
            raw,
            digest: bytes.map(|b| Sha256::digest(b).into()),
        })
    }
    fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
        let Some(file) = remote_core::desktop_storage::open_regular(path, true)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(invalid());
        }
        Ok(Some(bytes))
    }
    fn commit(&mut self, next: serde_json::Value) -> io::Result<()> {
        validate(&next)?;
        let parent = self.path.parent().ok_or_else(invalid)?;
        crate::private_ipc::private_directory(parent)?;
        let current = Self::read(&self.path)?.map(|b| Sha256::digest(b).into());
        if current != self.digest {
            return Err(io::Error::other(
                "Saved sharing selection changed; existing data was retained",
            ));
        }
        let bytes = serde_json::to_vec(&next).map_err(|_| invalid())?;
        if bytes.len() as u64 > LIMIT {
            return Err(invalid());
        }
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        remote_core::desktop_storage::regular_file(&self.path, true)?;
        file.persist(&self.path).map_err(|e| e.error)?;
        fs::File::open(parent)?.sync_all()?;
        self.digest = Some(Sha256::digest(bytes).into());
        self.raw = next;
        Ok(())
    }
    pub fn keys(&self, scope: ShareScope) -> Vec<(u64, ShareKind)> {
        let Ok(doc) = validate(&self.raw) else {
            return Vec::new();
        };
        if scope.is_none() || doc.scope != scope {
            return Vec::new();
        }
        doc.records.into_iter().map(|r| (r.id, r.kind)).collect()
    }
    /// Persist consumption before starting a portal request. A failed disk
    /// update cannot replay this token or silently start a fresh selection.
    pub fn take(&mut self, scope: ShareScope, id: u64) -> io::Result<Option<Record>> {
        if !self.keys(scope).iter().any(|(key, _)| *key == id) {
            return Ok(None);
        }
        let mut next = self.raw.clone();
        let records = next["records"].as_array_mut().ok_or_else(invalid)?;
        let index = records
            .iter()
            .position(|r| r["id"].as_u64() == Some(id))
            .ok_or_else(invalid)?;
        let record = serde_json::from_value(records.remove(index)).map_err(|_| invalid())?;
        self.commit(next)?;
        Ok(Some(record))
    }
    pub fn put(&mut self, scope: ShareScope, record: Record) -> io::Result<()> {
        if scope.is_none() {
            return Err(invalid());
        }
        let doc = validate(&self.raw)?;
        let mut next = if doc.scope == scope {
            self.raw.clone()
        } else {
            empty(scope)
        };
        let records = next["records"].as_array_mut().ok_or_else(invalid)?;
        records.retain(|r| r["id"].as_u64() != Some(record.id));
        records.push(serde_json::to_value(record).map_err(|_| invalid())?);
        self.commit(next)
    }
    pub fn forget_all(&mut self) -> io::Result<()> {
        let scope = validate(&self.raw)?.scope;
        self.forget(scope, None)
    }
    pub fn forget(&mut self, scope: ShareScope, id: Option<u64>) -> io::Result<()> {
        let doc = validate(&self.raw)?;
        if doc.scope != scope || doc.records.is_empty() {
            return Ok(());
        }
        let mut next = self.raw.clone();
        next["records"]
            .as_array_mut()
            .ok_or_else(invalid)?
            .retain(|r| id.is_some_and(|id| r["id"].as_u64() != Some(id)));
        self.commit(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(id: u64) -> Record {
        Record {id,kind:ShareKind::Display,grant:serde_json::from_value(serde_json::json!({"sources":"Monitor","control_input":false,"token":"private-storage-fixture"})).unwrap()}
    }
    #[test]
    fn private_restore_is_network_bound_and_consumed_on_disk_before_use() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let scope = Some([1; 32]);
        let mut store = Store::load(dir.path()).unwrap();
        store.put(scope, record(1)).unwrap();
        assert_eq!(
            fs::metadata(&store.path).unwrap().permissions().mode() & 0o077,
            0
        );
        assert!(store.keys(Some([2; 32])).is_empty());
        assert!(store.take(Some([2; 32]), 1).unwrap().is_none());
        assert_eq!(store.take(scope, 1).unwrap().unwrap().id, 1);
        assert!(Store::load(dir.path()).unwrap().keys(scope).is_empty());
    }
    #[test]
    fn unrelated_edits_and_symlinks_are_retained() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Some([1; 32]);
        let mut store = Store::load(dir.path()).unwrap();
        store.put(scope, record(1)).unwrap();
        let edited = b"{\"user_data\":\"retain\"}";
        fs::write(&store.path, edited).unwrap();
        assert!(store.forget(scope, None).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), edited);
        fs::remove_file(&store.path).unwrap();
        std::os::unix::fs::symlink(dir.path().join("unrelated"), &store.path).unwrap();
        assert!(Store::load(dir.path()).is_err());
    }
}
