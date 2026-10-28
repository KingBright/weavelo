#![cfg(target_os="linux")]

use std::{fs, os::unix::fs::{MetadataExt, PermissionsExt}, os::unix::net::UnixListener, process::Command};

#[test]
fn cli_rejects_unrelated_socket_peer_without_touching_pending_recovery() {
    let dir=tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(),fs::Permissions::from_mode(0o700)).unwrap();
    let pending=dir.path().join("mesh.secret-recovery");
    let synthetic=b"synthetic-pending-recovery-not-a-valid-record";
    fs::write(&pending,synthetic).unwrap();
    fs::set_permissions(&pending,fs::Permissions::from_mode(0o600)).unwrap();
    let before=pending.metadata().unwrap();
    let socket=dir.path().join("background.sock");
    let _listener=UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket,fs::Permissions::from_mode(0o600)).unwrap();
    let output=Command::new(env!("CARGO_BIN_EXE_weavelo"))
        .arg("--background-readonly-status-json")
        .env("WEAVELO_DEVICE_GROUP_DIR",dir.path())
        .output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-pending"));
    let after=pending.metadata().unwrap();
    assert_eq!((before.ino(),before.len(),before.mtime(),before.mtime_nsec(),before.atime(),before.atime_nsec(),before.permissions().mode()),
        (after.ino(),after.len(),after.mtime(),after.mtime_nsec(),after.atime(),after.atime_nsec(),after.permissions().mode()));
    assert_eq!(fs::read(&pending).unwrap(),synthetic);
    assert!(!dir.path().join("preferences.json").exists());
    assert!(!dir.path().join("mesh.conf").exists());
    assert!(!dir.path().join("mesh.secret").exists());
}
