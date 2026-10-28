//! Publications belong to the process serving incoming file connections.
use super::{Client, Error, Request, Response};
use protocol::shared_files::SharedFileInfo;
use remote_core::shared_files::{ShareScope, current_share_scope, shared_file_catalog};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::{Path, PathBuf}, sync::{Arc, Mutex}, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Publication {
    pub request_id: String,
    pub path: PathBuf,
    pub sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    generation: String,
    executable_sha256: String,
    scope: ShareScope,
    request_id: String,
    sha256: String,
    pub file: SharedFileInfo,
}

pub(super) async fn answer(
    publication: Publication, scope: ShareScope, expected: String, hash: String,
    generation: String, executable_sha256: String,
    owner: &crate::UnifiedServiceOwner, closed: Arc<Mutex<bool>>,
) -> Response {
    let valid = || expected == generation && hash == executable_sha256
        && scope.is_some() && scope == current_share_scope()
        && !*closed.lock().unwrap();
    let file = match publish_current(&publication, scope, &shared_file_catalog(), owner, valid).await {
        Ok(file) => file,
        Err(error) => return Response::Rejected(error),
    };
    Response::Published(Receipt { generation, executable_sha256, scope,
        request_id: publication.request_id, sha256: publication.sha256, file })
}

async fn publish_current(
    publication: &Publication, scope: ShareScope,
    catalog: &remote_core::shared_files::SharedFileCatalog,
    owner: &crate::UnifiedServiceOwner, current: impl Fn() -> bool,
) -> Result<SharedFileInfo, String> {
    let valid = || owner.receiving_files_enabled() && current();
    if !valid() { return Err("Publication owner, group or file setting changed".into()); }
    if !publication.path.is_absolute() { return Err("Select an absolute file path".into()); }
    let file = catalog.publish_verified(&publication.path, scope, &publication.request_id, &publication.sha256).await?;
    if !valid() {
        catalog.revoke(scope, file.id);
        return Err("Publication owner, group or file setting changed".into());
    }
    Ok(file)
}

impl Client {
    pub(crate) fn publish_file(&self, publication: Publication, scope: ShareScope) -> Result<Receipt, Error> {
        let generation = self.cached.lock().unwrap().generation.clone();
        let response = super::exchange_response_timeout(&self.profile, &Request::PublishShared {
            generation: generation.clone(), executable_sha256: self.executable_sha256.clone(),
            scope, publication: publication.clone(),
        }, Duration::from_secs(60))?;
        match response {
            Response::Published(receipt) if receipt.generation == generation
                && receipt.executable_sha256 == self.executable_sha256
                && receipt.scope == scope && scope.is_some()
                && receipt.request_id == publication.request_id && receipt.sha256 == publication.sha256 => Ok(receipt),
            Response::Rejected(error) => Err(error.into()),
            _ => Err("Publication response does not match the verified owner and request".into()),
        }
    }
}

fn digest(path: &Path) -> Result<String, Error> {
    let mut file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !path.is_absolute() || !metadata.is_file()
        || metadata.len() > remote_core::file_transfer::FileTransferPolicy::default().max_file_bytes {
        return Err("Select a regular file within the transfer limit".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 { break; }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) async fn publish_selected(path: PathBuf, scope: ShareScope) -> Result<SharedFileInfo, Error> {
    let selected = path.clone();
    let sha256 = tokio::task::spawn_blocking(move || digest(&selected)).await??;
    if scope.is_none() || scope != current_share_scope() { return Err("Device group changed; select the file again".into()); }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let publication = Publication { request_id: format!("publish-{}-{}-{}", std::process::id(), crate::unix_now_ms(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)), path, sha256 };
    let profile = remote_core::mesh::default_app_private_mesh_dir();
    if super::attachment_required(&profile)? {
        return tokio::task::spawn_blocking(move || Client::connect(&profile)?.publish_file(publication, scope).map(|r| r.file)).await?;
    }
    Ok(shared_file_catalog().publish_verified(&publication.path, scope, &publication.request_id, &publication.sha256).await?)
}

pub(crate) fn command(profile: &Path, args: &[String]) -> Result<serde_json::Value, Error> {
    let values = args.iter().map(String::as_str).collect::<Vec<_>>();
    let ["--file-publish", path, "--sha256", sha256, "--request-id", request_id, "--confirm-publish"] = values.as_slice()
        else { return Err("Publish requires one file, SHA-256, request ID and --confirm-publish".into()); };
    let client = Client::connect(profile)?;
    let scope = current_share_scope();
    let receipt = client.publish_file(Publication { path: PathBuf::from(path), sha256: (*sha256).into(), request_id: (*request_id).into() }, scope)?;
    Ok(serde_json::json!({ "ok":true, "state":"published", "publication":receipt,
        "lifetime":"current background process and device group" }))
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn passive_owner(enabled: bool) -> crate::UnifiedServiceOwner {
        crate::UnifiedServiceOwner::start(crate::UnifiedServiceOwnerConfig {
            passive_host: Some(host::service::HostServiceConfig {
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                stats: remote_core::stats::Statistics::new(),
                enable_file_transfer: enabled,
                enable_clipboard_sync: false,
                enable_talkback: false,
            }),
            ..Default::default()
        }).await.unwrap()
    }
    #[tokio::test]
    async fn serving_catalog_only_admits_confirmed_bytes_for_the_current_owner() {
        let owner = passive_owner(true).await;
        assert!(!owner.file_transfer_enabled(), "Background has no client receiver");
        let directory = std::env::temp_dir().join(format!("weavelo-publication-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("selected.txt");
        let catalog = remote_core::shared_files::SharedFileCatalog::default();
        let scope = Some([61; 32]);
        let publication = Publication { request_id: "selected-1".into(), path: path.clone(),
            sha256: format!("{:x}", Sha256::digest(b"selected")) };
        assert!(publish_current(&publication, scope, &catalog, &owner, || false).await.is_err());
        assert!(catalog.page(scope, 0).0.is_empty());
        std::fs::write(&path, b"selected").unwrap();
        let calls = std::cell::Cell::new(0);
        assert!(publish_current(&publication, scope, &catalog, &owner, || {
            calls.set(calls.get() + 1); calls.get() == 1
        }).await.is_err());
        assert!(catalog.page(scope, 0).0.is_empty());
        let file = publish_current(&publication, scope, &catalog, &owner, || true).await.unwrap();
        let replay = publish_current(&publication, scope, &catalog, &owner, || true).await.unwrap();
        assert_eq!(file.id, replay.id);
        assert_eq!(file.size_bytes, 8);
        assert_eq!(catalog.page(scope, 0).0.len(), 1);
        let disabled = passive_owner(false).await;
        let frontend = crate::UnifiedServiceOwner::start(Default::default()).await.unwrap();
        let denied = remote_core::shared_files::SharedFileCatalog::default();
        for owner in [&disabled, &frontend] {
            assert!(publish_current(&publication, scope, &denied, owner, || true).await.is_err());
            assert!(denied.page(scope, 0).0.is_empty());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
