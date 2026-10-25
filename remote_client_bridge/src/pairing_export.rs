//! Explicit, read-only invitation export. Never add secrets to periodic identity telemetry.
use remote_core::mesh::AppPrivateMeshConfigStore;
use remote_core::{encode_pairing_qr, net::DEFAULT_CONTROL_PORT};

pub(crate) fn invitation_json(store: &AppPrivateMeshConfigStore) -> Result<String, &'static str> {
    let config = store
        .load()
        .map_err(|_| "Cannot read the existing device group")?
        .ok_or("No device group is available")?;
    let code = config.invite_code();
    let payload = encode_pairing_qr(&code, DEFAULT_CONTROL_PORT)
        .map_err(|_| "Cannot encode this invitation")?;
    Ok(serde_json::json!({"code": code, "qr_payload": payload, "network_name": config.network_name}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use remote_core::mesh::{MESH_CONFIG_FILE_NAME, MESH_SECRET_FILE_NAME, MeshConfig};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "weavelo-invitation-export-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
    #[test]
    fn exporting_an_existing_group_is_read_only_and_uses_the_shared_codec() {
        let root = root();
        let store = AppPrivateMeshConfigStore::new(&root);
        let config = MeshConfig::generate("Synthetic phone");
        store.save(&config).unwrap();
        let metadata_before = fs::read(root.join(MESH_CONFIG_FILE_NAME)).unwrap();
        let secret_before = fs::read(root.join(MESH_SECRET_FILE_NAME)).unwrap();
        let exported: serde_json::Value =
            serde_json::from_str(&invitation_json(&store).unwrap()).unwrap();
        let payload =
            remote_core::parse_pairing_qr(exported["qr_payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload.invite_code, exported["code"].as_str().unwrap());
        let joined =
            MeshConfig::from_invite_code(&payload.invite_code, "Synthetic desktop").unwrap();
        assert_eq!(joined.network_secret, config.network_secret);
        assert_eq!(joined.network_name, config.network_name);
        assert_ne!(joined.node_id, config.node_id);
        assert_eq!(
            fs::read(root.join(MESH_CONFIG_FILE_NAME)).unwrap(),
            metadata_before
        );
        assert_eq!(
            fs::read(root.join(MESH_SECRET_FILE_NAME)).unwrap(),
            secret_before
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn export_never_creates_a_missing_group() {
        let root = root();
        assert!(invitation_json(&AppPrivateMeshConfigStore::new(&root)).is_err());
        assert!(!root.exists());
    }
    #[cfg(unix)]
    #[test]
    fn export_preserves_private_storage_rejection() {
        use std::os::unix::fs::PermissionsExt;
        let root = root();
        let store = AppPrivateMeshConfigStore::new(&root);
        store
            .save(&MeshConfig::generate("Synthetic phone"))
            .unwrap();
        let secret = root.join(MESH_SECRET_FILE_NAME);
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(invitation_json(&store).is_err());
        assert_eq!(
            fs::metadata(&secret).unwrap().permissions().mode() & 0o777,
            0o644
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn committed_readback_failure_survives_noop_until_runtime_reconciliation() {
        use remote_core::mesh::MeshStoreError;
        use std::sync::atomic::AtomicBool;
        let needed = AtomicBool::new(false);
        let saved = MeshConfig::generate("Synthetic saved group");
        let failed = crate::pairing_change_outcome(Err(MeshStoreError::CommittedDurabilityUnconfirmed), &needed,
            || Err(MeshStoreError::InvalidMetadata("synthetic busy read".into())));
        assert!(failed.is_err());
        assert!(needed.load(Ordering::Relaxed));
        let (config, changed, warning) = crate::pairing_change_outcome(Ok((saved.clone(), false)), &needed,
            || panic!("no extra read or replay on an already verified no-op")).unwrap();
        assert_eq!(config, saved);
        assert!(changed && warning);
        needed.store(false, Ordering::Relaxed); // Caller reconciles once, then clears.
        let (_, changed, warning) = crate::pairing_change_outcome(Ok((saved, false)), &needed,
            || panic!("already verified no-op")).unwrap();
        assert!(!changed && !warning);
    }

    #[test]
    fn committed_and_unknown_outcomes_never_return_success_without_saved_state() {
        use remote_core::mesh::MeshStoreError;
        use std::sync::atomic::AtomicBool;
        let needed = AtomicBool::new(false);
        assert!(crate::pairing_change_outcome(Err(MeshStoreError::CommittedDurabilityUnconfirmed), &needed,
            || Ok(None)).is_err());
        assert!(needed.load(Ordering::Relaxed));
        assert!(crate::pairing_change_outcome(Err(MeshStoreError::TransactionOutcomeUnknown), &needed,
            || panic!("unknown must not be replayed or treated as committed")).is_err());
        let config = MeshConfig::generate("Synthetic");
        let (actual, changed, warning) = crate::pairing_change_outcome(
            Err(MeshStoreError::CommittedDurabilityUnconfirmed), &needed, || Ok(Some(config.clone()))).unwrap();
        assert_eq!(actual, config);
        assert!(changed && warning);
    }

    #[test]
    fn closed_reload_receiver_retains_reconcile_flag_and_never_reports_discovering() {
        use std::sync::atomic::AtomicBool;
        let needed = AtomicBool::new(false);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        drop(rx);
        let error = crate::finish_pairing_reconcile(tx.send(()).is_ok(), &needed).unwrap_err();
        assert!(needed.load(Ordering::Relaxed));
        assert!(error.contains("is saved") && error.contains("refresh is unavailable"));
        let saved = MeshConfig::generate("Synthetic");
        let (_, changed, _) = crate::pairing_change_outcome(Ok((saved, false)), &needed,
            || panic!("known state must not be replayed")).unwrap();
        assert!(changed);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        crate::finish_pairing_reconcile(tx.send(()).is_ok(), &needed).unwrap();
        assert!(!needed.load(Ordering::Relaxed));
        rx.try_recv().unwrap();
        assert!(rx.try_recv().is_err());
    }

}
