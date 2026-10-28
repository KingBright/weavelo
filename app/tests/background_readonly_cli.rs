#![cfg(target_os="linux")]

use std::{fs, io::{Read,Write}, os::unix::fs::{MetadataExt, PermissionsExt}, os::unix::net::UnixListener, process::Command};
use sha2::{Digest,Sha256};

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
    let listener=UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket,fs::Permissions::from_mode(0o600)).unwrap();
    // Every claimed field is correct for this PID except the executable: the
    // real server is this test process, not the product binary it impersonates.
    let hash=format!("{:x}",Sha256::digest(fs::read(env!("CARGO_BIN_EXE_weavelo")).unwrap()));
    let server=std::thread::spawn(move|| {
        let (mut stream,_)=listener.accept().unwrap();
        let mut header=[0;4];stream.read_exact(&mut header).unwrap();
        let len=u32::from_be_bytes(header) as usize;assert!(len>0&&len<4096);
        let mut request=vec![0;len];stream.read_exact(&mut request).unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&request).unwrap(),serde_json::json!({"ReadOnlyStatus":{"schema":2}}));
        let pid=std::process::id();let stat=fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let start_ticks=stat[stat.rfind(") ").unwrap()+2..].split_whitespace().nth(19).unwrap().parse::<u64>().unwrap();
        let bytes=serde_json::to_vec(&serde_json::json!({"ReadOnlyStatus":{
            "schema":2,"role":"Background","version":env!("CARGO_PKG_VERSION"),
            "generation":format!("{pid}-forged"),"executable_sha256":hash,
            "pid":pid,"start_ticks":start_ticks,"idle_now":true}})).unwrap();
        stream.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();stream.write_all(&bytes).unwrap();
    });
    let output=Command::new(env!("CARGO_BIN_EXE_weavelo"))
        .arg("--background-readonly-status-json")
        .env("WEAVELO_DEVICE_GROUP_DIR",dir.path())
        .output().unwrap();
    server.join().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-pending"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("peer executable differs"));
    let after=pending.metadata().unwrap();
    assert_eq!((before.ino(),before.len(),before.mtime(),before.mtime_nsec(),before.atime(),before.atime_nsec(),before.permissions().mode()),
        (after.ino(),after.len(),after.mtime(),after.mtime_nsec(),after.atime(),after.atime_nsec(),after.permissions().mode()));
    assert_eq!(fs::read(&pending).unwrap(),synthetic);
    assert!(!dir.path().join("preferences.json").exists());
    assert!(!dir.path().join("mesh.conf").exists());
    assert!(!dir.path().join("mesh.secret").exists());
}
