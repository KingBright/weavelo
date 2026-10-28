use super::*;
use serde_json::json;
fn fixture() -> (Value, Value, Actor, Call) {
    let actor = Actor {
        owner: "synthetic-owner".into(),
        task_id: "11111111-1111-1111-1111-111111111111".into(),
        executor_device_id: "22222222-2222-2222-2222-222222222222".into(),
    };
    let target = Target {
        peer_device_id: "synthetic-remote-peer".into(),
        source: protocol::session::CaptureSource::Window(42),
        connection_id: 7,
    };
    let route = json!({"profile":"/synthetic/gui","generation":"1-2","gui_executable_sha256":"c".repeat(64),"target":target});
    let root = json!({"enabled":true,"observe":true,"input":false,"capability_version":7,
        "identity":{"owner":actor.owner,"device_id":actor.executor_device_id,"desktop_id":target.desktop_id()},
        "desktop_session":"weavelo","desktop_uid":unsafe {libc::geteuid()},"max_frame_bytes":1024,
        "weavelo":route,"worker_sha256":"a".repeat(64),"agent_sha256":"b".repeat(64)});
    let worker = json!({"enabled":true,"allow_input":false,"app_id":"fun.hackerlife.octendra.desktop-worker",
        "identity":{"owner":actor.owner,"task_id":actor.task_id,"device_id":actor.executor_device_id,"desktop_id":target.desktop_id()},
        "desktop_session":"weavelo","desktop_uid":unsafe {libc::geteuid()},"capability_version":7,
        "max_frame_bytes":1024,"expires_at_ms":crate::unix_now_ms()+30000,"weavelo":route,
        "worker_sha256":"a".repeat(64),"agent_sha256":"b".repeat(64)});
    let call = Call::Open {
        actor: actor.clone(),
        target,
        lease_id: "33333333-3333-3333-3333-333333333333".into(),
        lease_ms: 1000,
        max_bytes: 1024,
        control: false,
    };
    (root, worker, actor, call)
}
fn check(root: &Value, worker: &Value, actor: &Actor, call: &Call) -> Result<(), Error> {
    authorize(
        root,
        worker,
        actor,
        call,
        Path::new("/synthetic/gui"),
        "1-2",
        &"c".repeat(64),
    )
}
#[test]
fn existing_authority_requires_exact_owner_task_executor_remote_source_and_version() {
    let (root, worker, actor, call) = fixture();
    assert!(check(&root, &worker, &actor, &call).is_ok());
    for field in ["owner", "task_id", "device_id", "desktop_id"] {
        let mut other = worker.clone();
        other["identity"][field] = json!("wrong");
        assert!(check(&root, &other, &actor, &call).is_err(), "{field}");
    }
    for field in ["capability_version", "agent_sha256", "worker_sha256"] {
        let mut other = worker.clone();
        other[field] = json!(0);
        assert!(check(&root, &other, &actor, &call).is_err(), "{field}");
    }
    let mut other = root.clone();
    other["weavelo"]["target"]["source"] = json!({"Window":43});
    assert!(check(&other, &worker, &actor, &call).is_err());
    let mut other = root.clone();
    other["identity"]["device_id"] = json!("synthetic-remote-peer");
    assert!(check(&other, &worker, &actor, &call).is_err());
}

#[test]
fn authorization_failures_identify_the_changed_boundary_without_values() {
    let (root, worker, actor, call) = fixture();
    for (pointer, value, expected) in [
        ("/enabled", json!(false), "octendra_authority_disabled"),
        (
            "/identity/owner",
            json!("private-owner"),
            "octendra_authority_owner_mismatch",
        ),
        (
            "/weavelo/gui_executable_sha256",
            json!("old-hash"),
            "octendra_authority_gui_executable_changed",
        ),
    ] {
        let mut changed = root.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert_eq!(check(&changed, &worker, &actor, &call), Err(expected));
        assert!(!expected.contains("private-owner"));
    }
    for (pointer, value, expected) in [
        (
            "/weavelo/generation",
            json!("old-generation"),
            "octendra_task_gui_generation_mismatch",
        ),
        (
            "/identity/task_id",
            json!("other-task"),
            "octendra_task_id_mismatch",
        ),
        (
            "/capability_version",
            json!(0),
            "octendra_task_capability_version_mismatch",
        ),
        (
            "/agent_sha256",
            json!("old-agent"),
            "octendra_task_agent_changed",
        ),
        (
            "/worker_sha256",
            json!("old-worker"),
            "octendra_task_worker_changed",
        ),
        (
            "/expires_at_ms",
            json!(0),
            "octendra_task_authority_expired",
        ),
        (
            "/expires_at_ms",
            json!("invalid"),
            "octendra_task_expiry_invalid",
        ),
        (
            "/expires_at_ms",
            json!(crate::unix_now_ms() + 60_000),
            "octendra_task_expiry_out_of_bounds",
        ),
    ] {
        let mut changed = worker.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert_eq!(check(&root, &changed, &actor, &call), Err(expected));
    }
    let mut root = root;
    let mut worker = worker;
    root["input"] = json!(true);
    worker["allow_input"] = json!(true);
    assert_eq!(
        check(&root, &worker, &actor, &call),
        Err("octendra_authority_input_platform_mismatch")
    );
}

#[test]
fn approved_target_reopens_in_a_new_gui_but_old_workers_and_other_apps_do_not() {
    let (mut root, mut worker, actor, call) = fixture();
    root["weavelo"]["generation"] = json!("previous-generation");
    assert!(check(&root, &worker, &actor, &call).is_ok());
    worker["weavelo"]["generation"] = json!("previous-generation");
    assert_eq!(
        check(&root, &worker, &actor, &call),
        Err("octendra_task_gui_generation_mismatch")
    );
    worker["weavelo"]["generation"] = json!("1-2");
    root["weavelo"]["gui_executable_sha256"] = json!("different-app");
    assert_eq!(
        check(&root, &worker, &actor, &call),
        Err("octendra_authority_gui_executable_changed")
    );
}

#[test]
fn insecure_locator_parent_keeps_its_specific_failure_and_never_configures_authority() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    private_locator(dir.path(), &dir.path().join("desktop-launch.json"));
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let authority = Authority::new(dir.path(), "1-2", &"c".repeat(64));
    assert_eq!(
        authority.launch().unwrap_err(),
        "authority_directory_not_private"
    );
    assert!(!authority.configured());
}
#[test]
fn old_local_desktop_grant_and_input_enabled_grants_do_not_authorize_remote_observation() {
    let (root, worker, actor, call) = fixture();
    let mut old = root.clone();
    old.as_object_mut().unwrap().remove("weavelo");
    assert!(check(&old, &worker, &actor, &call).is_err());
    let mut enabled = root.clone();
    enabled["input"] = json!(true);
    assert!(check(&enabled, &worker, &actor, &call).is_err());
    let mut expired = worker.clone();
    expired["expires_at_ms"] = json!(crate::unix_now_ms() - 1);
    assert!(check(&root, &expired, &actor, &call).is_err());
    let mut over = worker.clone();
    over["max_frame_bytes"] = json!(1023);
    assert!(check(&root, &over, &actor, &call).is_err());
}
#[test]
fn absent_locator_is_unavailable_without_trusting_request_paths_or_process_names() {
    let dir = tempfile::tempdir().unwrap();
    let authority = Authority::new(dir.path(), "1-2", &"c".repeat(64));
    assert!(!authority.configured());
    assert!(authority.verify(&fixture().3, None, None).is_err());
}

#[test]
fn control_requires_both_existing_grants_and_explicit_remote_mac_key_space() {
    let (mut root, mut worker, actor, mut call) = fixture();
    let Call::Open { control, .. } = &mut call else {
        unreachable!()
    };
    *control = true;
    assert!(check(&root, &worker, &actor, &call).is_err());
    root["input"] = json!(true);
    worker["allow_input"] = json!(true);
    assert!(check(&root, &worker, &actor, &call).is_err());
    root["weavelo"]["input_platform"] = json!("macos");
    worker["weavelo"] = root["weavelo"].clone();
    assert!(check(&root, &worker, &actor, &call).is_ok());
    let act = Call::Act {
        actor: actor.clone(),
        lease_id: call.identity().1.into(),
        source_revision: 7,
        frame_sequence: 3,
        action: super::super::computer_use::Action::KeyPress { key_code: 0 },
    };
    assert!(check(&root, &worker, &actor, &act).is_ok());
    root["input"] = json!(false);
    worker["allow_input"] = json!(false);
    assert!(check(&root, &worker, &actor, &act).is_err());
}

#[test]
fn authority_fingerprint_pins_locator_and_both_exact_existing_records() {
    assert_eq!(
        authority_digest(b"locator", b"root", b"worker"),
        authority_digest(b"locator", b"root", b"worker")
    );
    assert_ne!(
        authority_digest(b"locator", b"root", b"worker"),
        authority_digest(b"locator", b"root2", b"worker")
    );
    assert_ne!(
        authority_digest(b"locator", b"root", b"worker"),
        authority_digest(b"locator", b"root", b"worker2")
    );
    assert_ne!(
        authority_digest(b"one", b"root", b"worker"),
        authority_digest(b"two", b"root", b"worker")
    );
    assert_ne!(
        authority_digest(b"a", b"b", b"cd"),
        authority_digest(b"a", b"bc", b"d")
    );
}

fn private_locator(directory: &Path, path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = directory.join("octendra-authority.json");
    std::fs::write(
        &file,
        serde_json::to_vec(&json!({"desktop_launch":path})).unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn locator_approved_after_gui_start_is_seen_without_restarting_the_generation() {
    let dir = tempfile::tempdir().unwrap();
    let authority = Authority::new(dir.path(), "unchanged-generation", &"c".repeat(64));
    assert!(authority.launch().is_err());
    let target = dir.path().join("desktop-launch.json");
    private_locator(dir.path(), &target);
    assert_eq!(authority.launch().unwrap().0, target);
    assert_eq!(authority.configured(), cfg!(target_os = "macos"));
    // A locator selects existing authority, but never grants it: no launch
    // approval/worker/process proof exists in this private filesystem fixture.
    assert!(authority.verify(&fixture().3, None, None).is_err());
    std::fs::remove_file(dir.path().join("octendra-authority.json")).unwrap();
    assert!(authority.launch().is_err());
    assert!(!authority.configured());
}

#[test]
fn locator_retargeting_changes_a_live_authority_fingerprint_even_for_identical_grants() {
    let dir = tempfile::tempdir().unwrap();
    let authority = Authority::new(dir.path(), "unchanged-generation", &"c".repeat(64));
    let first = dir.path().join("first/desktop-launch.json");
    private_locator(dir.path(), &first);
    let (path, bytes) = authority.launch().unwrap();
    assert_eq!(path, first);
    let old = authority_digest(&bytes, b"same-root", b"same-worker");
    let second = dir.path().join("second/desktop-launch.json");
    private_locator(dir.path(), &second);
    let (path, bytes) = authority.launch().unwrap();
    assert_eq!(path, second);
    assert_ne!(old, authority_digest(&bytes, b"same-root", b"same-worker"));
}

#[test]
fn late_locator_keeps_private_file_schema_and_no_follow_checks() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let authority = Authority::new(dir.path(), "1-2", &"c".repeat(64));
    let path = dir.path().join("octendra-authority.json");
    private_locator(dir.path(), &dir.path().join("desktop-launch.json"));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(authority.launch().is_err());
    std::fs::remove_file(&path).unwrap();
    let elsewhere = dir.path().join("fixture-target");
    std::fs::write(&elsewhere, b"{}").unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    assert!(authority.launch().is_err());
    std::fs::remove_file(&path).unwrap();
    private_locator(dir.path(), Path::new("relative/desktop-launch.json"));
    assert!(authority.launch().is_err());
    private_locator(dir.path(), &dir.path().join("another.json"));
    assert!(authority.launch().is_err());
}
