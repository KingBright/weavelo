//! Native file commands reuse the original GUI connection and transfer service.
//! A private durable request reservation prevents retrying uncertain sends.
use super::original_owner::OriginalOwner;
use client::{TransferDirection, TransferStatus};
use protocol::shared_files::{SharedFileInfo, SharedFileRequest, SharedFileResponse};
use remote_core::file_transfer_runtime::FileTransferEvent;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum Action {
    Send {
        target: String,
        path: PathBuf,
        sha256: String,
    },
    ListShared { target: String, after_id: u64 },
    FetchShared { target: String, file_id: u64, list_request_id: String, sha256: String },
    Status,
    Resume,
    SetEnabled {
        enabled: bool,
        expected_preferences_sha256: String,
        expected_group_scope: String,
    },
    Peers,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    request_id: String,
    generation: String,
    target: String,
    path: PathBuf,
    sha256: String,
    bytes: u64,
    state: String,
    terminal: bool,
    connection_id: Option<u32>,
    transfer_id: Option<u64>,
    transferred_bytes: u64,
    detail: String,
    remote_content_verified: bool,
    #[serde(default)]
    group_scope: Option<String>,
    #[serde(default)]
    resumed_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shared: Option<SharedRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
enum SharedSelection {
    List { after_id: u64 },
    Fetch { list_request_id: String, file: SharedFileInfo, connection_id: u32 },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SharedRecord {
    selection: SharedSelection,
    native_request_id: Option<u64>,
    entries: Vec<SharedFileInfo>,
    next_after_id: Option<u64>,
    received_path: Option<PathBuf>,
}
fn group_scope() -> Option<String> {
    remote_core::session_crypto::paired_file_session_key()
        .map(|key| format!("{:x}", Sha256::digest(key)))
}
fn safe_to_restart(record: &Record) -> bool {
    record.shared.is_none()
        && record.terminal
        && record.state == "failed_before_send"
        && record.connection_id.is_none()
        && record.transfer_id.is_none()
        && record.transferred_bytes == 0
}
fn refresh(owner: &OriginalOwner, generation: &str, record: &mut Record) {
    // A late confirmation may update the original identity only. No connection
    // is opened and no new file command is issued by a query or resume.
    if record.shared.is_some() || record.generation != generation || record.group_scope != group_scope() {
        return;
    }
    let Some(id) = record.transfer_id else {
        return;
    };
    let Ok((connection, transfers)) = owner.cli_file_connection(
        &record.target,
        remote_core::shared_files::current_share_scope(),
    ) else {
        return;
    };
    if connection.upgrade().map(|c| c.id) != record.connection_id {
        return;
    }
    let values = transfers.lock().unwrap().identified_snapshots();
    if let Some((_, _, snapshot)) = values.iter().find(|(direction, selected, s)| {
        *direction == TransferDirection::Outgoing
            && *selected == id
            && s.total_bytes == record.bytes
            && record.path.file_name().and_then(|s| s.to_str()) == Some(s.label.as_str())
    }) {
        record.transferred_bytes = snapshot.transferred_bytes;
        record.state = match snapshot.status {
            TransferStatus::Running => "sending",
            TransferStatus::Completed => "completed",
            TransferStatus::Cancelled => "cancelled",
            TransferStatus::Error => "error",
        }
        .into();
        record.terminal = snapshot.status != TransferStatus::Running;
        record.detail = snapshot.detail.clone();
    }
}
fn launch(owner: Arc<OriginalOwner>, path: PathBuf, mut record: Record, operation:super::upgrade_quit::Operation) {
    let scope = remote_core::shared_files::current_share_scope();
    tokio::spawn(async move {
        let _operation=operation;
        let result = if record.shared.is_some() {
            shared_request(&owner, &path, &mut record, scope).await
        } else { send(&owner, &path, &mut record, scope).await };
        if let Err(error) = result {
            record.state = if record.connection_id.is_some() {
                "not_confirmed"
            } else {
                "failed_before_send"
            }
            .into();
            record.terminal = true;
            record.detail = error.to_string();
            let _ = save(&path, &record);
        }
    });
}
fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn valid_target(target: &str) -> bool {
    !target.is_empty() && target.len() <= 256 && !target.chars().any(char::is_control)
}
fn valid_shared_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= protocol::shared_files::MAX_SHARED_NAME_BYTES
        && !matches!(name, "." | "..")
        && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
}
fn selected_share(listed: &Record, generation: &str, target: &str, scope: Option<&str>, file_id: u64)
    -> Result<(SharedFileInfo, u32), Error> {
    let page = listed.shared.as_ref().ok_or("A shared-list receipt is required")?;
    if !matches!(page.selection, SharedSelection::List { .. }) || !listed.terminal
        || listed.state != "completed" || listed.generation != generation || listed.target != target
        || scope.is_none() || listed.group_scope.as_deref() != scope {
        return Err("Refresh the shared list on this device, connection and paired group".into());
    }
    let file = page.entries.iter().find(|e| e.id == file_id && file_id != 0)
        .filter(|e| valid_shared_name(&e.name))
        .ok_or("Only a file in the completed shared-list page can be fetched")?;
    Ok((file.clone(), listed.connection_id.ok_or("Listed connection is unavailable")?))
}
fn serial() -> &'static tokio::sync::Mutex<()> {
    static SERIAL: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    SERIAL.get_or_init(|| tokio::sync::Mutex::new(()))
}
fn verify_received(path: &Path, receive_dir: &Path, expected: &str, bytes: u64) -> Result<(), Error> {
    // Only the actual completion event's file, inside the configured receive folder.
    // Remote callers never supply a destination path and existing files are retained.
    if path.parent() != Some(fs::canonicalize(receive_dir)?.as_path()) {
        return Err("Received file is outside the original receive folder".into());
    }
    if digest(path)? != (expected.to_owned(), bytes) {
        return Err("Received file differs from the expected SHA-256 or size".into());
    }
    Ok(())
}
fn digest(path: &Path) -> Result<(String, u64), Error> {
    if !path.is_absolute()
        || path
            .ancestors()
            .any(|p| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()))
    {
        return Err("Choose an absolute non-linked source file".into());
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Err("Source must be a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || meta.nlink() != 1 {
            return Err("Source must be an owned, unaliased regular file".into());
        }
    }
    if meta.len() > remote_core::file_transfer::FileTransferPolicy::default().max_file_bytes {
        return Err("Source exceeds the existing file transfer limit".into());
    }
    let mut input = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if input.metadata()?.len() != meta.len() {
        return Err("Source changed during verification".into());
    }
    Ok((format!("{:x}", hash.finalize()), meta.len()))
}
fn read(path: &Path) -> Result<Record, Error> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.len() > 16384 {
        return Err("Invalid file request receipt".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if meta.uid() != unsafe { libc::geteuid() }
            || meta.nlink() != 1
            || meta.permissions().mode() & 0o077 != 0
        {
            return Err("Private file request receipt required".into());
        }
    }
    let record: Record = serde_json::from_slice(&fs::read(path)?)?;
    if record.schema != 1 || !valid_id(&record.request_id) {
        return Err("Unsupported file request receipt".into());
    }
    Ok(record)
}
fn reserve(path: &Path, record: &Record) -> Result<bool, Error> {
    use std::fs::OpenOptions;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut out) => {
            out.write_all(&serde_json::to_vec(record)?)?;
            out.sync_all()?;
            #[cfg(unix)]
            fs::File::open(path.parent().ok_or("Missing receipt directory")?)?.sync_all()?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let old = read(path)?;
            if old.target != record.target
                || old.path != record.path
                || old.sha256 != record.sha256
                || old.bytes != record.bytes
                || old.shared.as_ref().map(|s| &s.selection) != record.shared.as_ref().map(|s| &s.selection)
            {
                return Err("Request ID is already bound to a different transfer".into());
            }
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}
fn save(path: &Path, record: &Record) -> Result<(), Error> {
    let previous = read(path)?;
    if previous.request_id != record.request_id
        || previous.target != record.target
        || previous.path != record.path
        || previous.sha256 != record.sha256
        || previous.bytes != record.bytes
        || previous.shared.as_ref().map(|s| &s.selection) != record.shared.as_ref().map(|s| &s.selection)
    {
        return Err("File request receipt changed".into());
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing receipt directory")?)?;
    temp.write_all(&serde_json::to_vec(record)?)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    #[cfg(unix)]
    fs::File::open(path.parent().ok_or("Missing receipt directory")?)?.sync_all()?;
    Ok(())
}
fn directory(gui_directory: &Path) -> Result<PathBuf, Error> {
    crate::private_ipc::private_directory(gui_directory)?;
    let dir = gui_directory.join("file-requests");
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    // Existing paths are validated, never chmodded or followed through a link.
    crate::private_ipc::private_directory(&dir)?;
    Ok(dir)
}
fn response(record: &Record) -> Result<serde_json::Value, Error> {
    Ok(serde_json::json!({"ok":true,"file_request":record}))
}
pub(crate) fn dispatch(
    owner: Arc<OriginalOwner>,
    control_directory: &Path,
    generation: &str,
    id: &str,
    action: Action,
    operation: super::upgrade_quit::Operation,
) -> Result<serde_json::Value, Error> {
    let configuration = crate::UnifiedRuntimeConfig::from_env()?;
    dispatch_scoped(
        owner,
        control_directory,
        &configuration,
        generation,
        id,
        action,
        operation,
    )
}
fn settings_profile<'a>(
    control_directory: &Path,
    configured_profile: &'a Path,
) -> Result<&'a Path, Error> {
    if control_directory != configured_profile
        && control_directory != configured_profile.join("gui-state")
    {
        return Err("GUI control directory differs from the configured instance profile".into());
    }
    crate::private_ipc::private_directory(configured_profile)?;
    crate::private_ipc::private_directory(control_directory)?;
    Ok(configured_profile)
}
fn dispatch_scoped(
    owner: Arc<OriginalOwner>,
    control_directory: &Path,
    configuration: &crate::UnifiedRuntimeConfig,
    generation: &str,
    id: &str,
    action: Action,
    operation: super::upgrade_quit::Operation,
) -> Result<serde_json::Value, Error> {
    let profile = settings_profile(control_directory, &configuration.mesh_dir)?;
    if !valid_id(id) {
        return Err("Invalid file request ID".into());
    }
    if matches!(&action, Action::Peers) {
        let (version, enabled) =
            crate::preferences::UserPreferences::file_transfer_setting(profile)?;
        return Ok(serde_json::json!({"ok":true,"generation":generation,
            "preferences_path":profile.join("preferences.json"),"preferences_sha256":version,"stored_file_transfer_enabled":enabled,"group_scope":group_scope(),
            "file_transfer_preference_enabled":owner.native_file_transfer_enabled(&configuration),
            "paired_file_scope_present":remote_core::session_crypto::paired_file_session_key().is_some(),
            "devices":owner.snapshot().devices.iter().map(|d|serde_json::json!({"device_id":d.device_id,"display_name":d.display_name,"online":d.online})).collect::<Vec<_>>(),
            "receive_dir":owner.receive_dir()}));
    }
    if let Action::SetEnabled {
        enabled,
        expected_preferences_sha256,
        expected_group_scope,
    } = &action
    {
        if !valid_sha(expected_preferences_sha256)
            || !valid_sha(expected_group_scope)
            || group_scope().as_ref() != Some(expected_group_scope)
        {
            return Err(
                "Expected current paired group and preferences version are required".into(),
            );
        }
        if *enabled {
            remote_core::desktop_storage::receive_directory_at(profile, None)?;
        }
        let (version, actual) = crate::preferences::UserPreferences::set_file_transfer(
            profile,
            expected_preferences_sha256,
            *enabled,
        )?;
        return Ok(serde_json::json!({"ok":true,"preferences_sha256":version,
            "file_transfer_enabled":actual,"group_scope":group_scope(),
            "restart_new_instance_to_apply_host_setting":true}));
    }
    let path = directory(control_directory)?.join(format!("{id}.json"));
    let _request_lock = remote_core::desktop_storage::lock_file(&path.with_extension("lock"))?;
    match action {
        Action::Status | Action::Resume => {
            let mut record = read(&path)?;
            if matches!(action, Action::Resume) && record.shared.is_some() {
                return Err("Shared requests are never replayed; observe this ID and refresh the list separately".into());
            }
            let before = record.clone();
            refresh(&owner, generation, &mut record);
            if before != record {
                save(&path, &record)?;
            }
            if matches!(action, Action::Resume) && safe_to_restart(&record) {
                if !owner.native_file_transfer_enabled(&crate::UnifiedRuntimeConfig::from_env()?)
                    || group_scope().is_none()
                {
                    return Err("Enable files in the paired new instance before resuming".into());
                }
                if digest(&record.path)? != (record.sha256.clone(), record.bytes) {
                    return Err("Original source changed; original request retained".into());
                }
                record.generation = generation.into();
                record.group_scope = group_scope();
                record.resumed_attempts = record
                    .resumed_attempts
                    .checked_add(1)
                    .ok_or("Resume limit reached")?;
                record.state = "accepted".into();
                record.terminal = false;
                record.detail =
                    "Explicit resume of verified zero-admission failure; same request binding"
                        .into();
                save(&path, &record)?;
                let reply = response(&record)?;
                launch(owner, path, record, operation);
                return Ok(reply);
            }
            let mut result = response(&record)?;
            result["owner_generation_current"] = serde_json::json!(record.generation == generation);
            result["resume_policy"] = serde_json::json!(if record.state == "completed" {
                "already_completed_no_send"
            } else if !record.terminal {
                "observe_original_native_retries_no_new_send"
            } else {
                "verify_receiver_before_any_new_send"
            });
            Ok(result)
        }
        Action::ListShared { .. } | Action::FetchShared { .. } => {
            let (target, selection, sha256, bytes) = match action {
                Action::ListShared { target, after_id } => (target, SharedSelection::List { after_id }, String::new(), 0),
                Action::FetchShared { target, file_id, list_request_id, sha256 } => {
                    if !valid_id(&list_request_id) || !valid_sha(&sha256) || file_id == 0 {
                        return Err("A completed list request, shared file ID and expected SHA-256 are required".into());
                    }
                    let listed = read(&directory(control_directory)?.join(format!("{list_request_id}.json")))?;
                    let (file, connection_id) = selected_share(&listed, generation, &target, group_scope().as_deref(), file_id)?;
                    let bytes = file.size_bytes;
                    (target, SharedSelection::Fetch { list_request_id, file, connection_id }, sha256, bytes)
                }
                _ => unreachable!(),
            };
            if !valid_target(&target) { return Err("A target device ID is required".into()); }
            let record = Record {
                schema: 1, request_id: id.into(), generation: generation.into(), target,
                path: PathBuf::new(), sha256, bytes, state: "accepted".into(), terminal: false,
                connection_id: None, transfer_id: None, transferred_bytes: 0, detail: String::new(),
                remote_content_verified: false, group_scope: group_scope(), resumed_attempts: 0,
                shared: Some(SharedRecord { selection, native_request_id: None, entries: Vec::new(),
                    next_after_id: None, received_path: None }),
            };
            if !reserve(&path, &record)? { return response(&read(&path)?); }
            let reply = response(&record)?;
            launch(owner, path, record, operation);
            Ok(reply)
        }
        Action::Send {
            target,
            path: source,
            sha256,
        } => {
            if target.is_empty()
                || target.len() > 256
                || target.chars().any(char::is_control)
                || sha256.len() != 64
                || !sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err("Target device and lowercase SHA-256 are required".into());
            }
            let (actual, bytes) = digest(&source)?;
            if actual != sha256 {
                return Err("Selected source differs from expected SHA-256".into());
            }
            let record = Record {
                schema: 1,
                request_id: id.into(),
                generation: generation.into(),
                target,
                path: source,
                sha256,
                bytes,
                state: "accepted".into(),
                terminal: false,
                connection_id: None,
                transfer_id: None,
                transferred_bytes: 0,
                detail:
                    "Reserved before any connection or send; query this ID if the reply is lost"
                        .into(),
                remote_content_verified: false,
                group_scope: group_scope(),
                resumed_attempts: 0,
                shared: None,
            };
            if !reserve(&path, &record)? {
                return response(&read(&path)?);
            }
            // The reservation survives a disconnected caller or GUI restart. It
            // is never deleted or automatically replayed, including after failure.
            let reply = response(&record)?;
            launch(owner, path, record, operation);
            Ok(reply)
        }
        Action::Peers | Action::SetEnabled { .. } => unreachable!(),
    }
}
async fn shared_request(
    owner: &OriginalOwner, path: &Path, record: &mut Record,
    scope: remote_core::shared_files::ShareScope,
) -> Result<(), Error> {
    let _serial = serial().lock().await;
    if scope.is_none() || record.group_scope != group_scope()
        || scope != remote_core::shared_files::current_share_scope() {
        return Err("Paired group changed after reservation".into());
    }
    let selection = record.shared.as_ref().ok_or("Missing shared request")?.selection.clone();
    owner.connect_silent_files(&record.target).await?;
    let (connection, _) = owner.cli_file_connection(&record.target, scope)?;
    let connection_id = connection.upgrade().ok_or("File connection closed")?.id;
    let request = match &selection {
        SharedSelection::List { after_id } => SharedFileRequest::List { after_id: *after_id },
        SharedSelection::Fetch { file, connection_id: listed, .. } => {
            if *listed != connection_id { return Err("Listed connection changed; refresh the shared list".into()); }
            SharedFileRequest::Fetch { file_id: file.id }
        }
    };
    record.connection_id = Some(connection_id);
    record.state = "queued".into();
    save(path, record)?;
    let (request_id, mut events) = owner.cli_shared_request(&connection, scope, request)?;
    record.shared.as_mut().unwrap().native_request_id = Some(request_id);
    save(path, record)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    let mut completed = Vec::new();
    loop {
        if scope != remote_core::shared_files::current_share_scope() || connection.upgrade().is_none() {
            return Err("Original connection or paired group changed; request was not replayed".into());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Shared request completion not observed; query this ID before any new request".into());
        }
        let event = match tokio::time::timeout(Duration::from_millis(150), events.recv()).await {
            Err(_) => continue,
            Ok(Err(_)) => return Err("Original file observation channel closed or lagged; do not replay".into()),
            Ok(Ok(event)) => event,
        };
        match event {
            FileTransferEvent::SharedResponse { request_id: id, response } if id == request_id => match response {
                SharedFileResponse::Rejected { message } => return Err(message.into()),
                SharedFileResponse::Page { entries, next_after_id } if matches!(selection, SharedSelection::List { .. }) => {
                    if entries.len() > protocol::shared_files::SHARED_PAGE_SIZE
                        || entries.iter().any(|e| !valid_shared_name(&e.name)) {
                        return Err("Invalid shared-file page".into());
                    }
                    let shared = record.shared.as_mut().unwrap();
                    shared.entries = entries;
                    shared.next_after_id = next_after_id;
                    record.state = "completed".into();
                    record.terminal = true;
                    return save(path, record);
                }
                SharedFileResponse::Queued { transfer_id } if matches!(selection, SharedSelection::Fetch { .. }) => {
                    record.transfer_id = Some(transfer_id);
                    record.state = "receiving".into();
                    save(path, record)?;
                }
                _ => return Err("Shared response differs from the original request".into()),
            },
            FileTransferEvent::IncomingCompleted { transfer_id, path: received, size_bytes, group: None, .. }
                if matches!(selection, SharedSelection::Fetch { .. }) => {
                    if completed.len() >= 32 { return Err("Too many concurrent incoming files to identify this pull".into()); }
                    completed.push((transfer_id, received, size_bytes));
                }
            _ => {}
        }
        if let Some((_, received, bytes)) = completed.iter().find(|(id, _, _)| Some(*id) == record.transfer_id) {
            if *bytes != record.bytes { return Err("Pulled file differs from the listed size".into()); }
            let received = received.clone();
            let selected = received.clone();
            let directory = owner.receive_dir();
            let sha256 = record.sha256.clone();
            let bytes = record.bytes;
            tokio::task::spawn_blocking(move || verify_received(&selected, &directory, &sha256, bytes)).await??;
            if scope != remote_core::shared_files::current_share_scope() || connection.upgrade().is_none() {
                return Err("Original connection or paired group changed during verification".into());
            }
            record.shared.as_mut().unwrap().received_path = Some(received);
            record.transferred_bytes = bytes;
            record.remote_content_verified = true;
            record.state = "completed".into();
            record.terminal = true;
            return save(path, record);
        }
    }
}
async fn send(
    owner: &OriginalOwner,
    path: &Path,
    record: &mut Record,
    scope: remote_core::shared_files::ShareScope,
) -> Result<(), Error> {
    let _serial = serial().lock().await;
    if !owner.native_file_transfer_enabled(&crate::UnifiedRuntimeConfig::from_env()?) {
        return Err("File transfers are disabled in the existing product settings".into());
    }
    if scope != remote_core::shared_files::current_share_scope()
        || record.group_scope.is_none()
        || record.group_scope != group_scope()
    {
        return Err("Device group changed after reservation".into());
    }
    owner.connect_silent_files(&record.target).await?;
    let (connection, transfers) = owner.cli_file_connection(&record.target, scope)?;
    let baseline = transfers
        .lock()
        .unwrap()
        .identified_snapshots()
        .into_iter()
        .filter_map(|(direction, id, _)| (direction == TransferDirection::Outgoing).then_some(id))
        .collect::<Vec<_>>();
    if digest(&record.path)? != (record.sha256.clone(), record.bytes) {
        return Err("Source changed before queuing".into());
    }
    record.connection_id = connection.upgrade().map(|c| c.id);
    record.state = "queued".into();
    record.detail.clear();
    save(path, record)?;
    owner.send_files_to(&connection, scope, vec![record.path.clone()]);
    let label = record
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Source name is not UTF-8")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        if scope != remote_core::shared_files::current_share_scope()
            || connection.upgrade().is_none()
        {
            return Err("Network or original connection changed; do not retry this request".into());
        }
        let values = transfers.lock().unwrap().identified_snapshots();
        let matches = values
            .iter()
            .filter(|(direction, id, s)| {
                *direction == TransferDirection::Outgoing
                    && record.transfer_id.map_or(
                        !baseline.contains(id) && s.label == label && s.total_bytes == record.bytes,
                        |selected| selected == *id,
                    )
            })
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err("Ambiguous native transfer identity; no success inferred".into());
        }
        if let Some((_, id, snapshot)) = matches.first() {
            record.transfer_id = Some(*id);
            record.transferred_bytes = snapshot.transferred_bytes;
            record.state = match snapshot.status {
                TransferStatus::Running => "sending",
                TransferStatus::Completed => "completed",
                TransferStatus::Cancelled => "cancelled",
                TransferStatus::Error => "error",
            }
            .into();
            record.terminal = snapshot.status != TransferStatus::Running;
            record.detail = snapshot.detail.clone();
            save(path, record)?;
            if record.terminal {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Native transfer completion not observed within 120 seconds; query this request before any new send".into());
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}
pub(crate) fn command(profile: &Path, args: &[String]) -> Result<serde_json::Value, Error> {
    use super::directed_receive::{Request, exchange};
    let directory = crate::private_ipc::gui_directory(profile)?;
    let status = exchange(&directory, &Request::Status)?;
    if status["executable_sha256"] != crate::background_service::executable_hash()? {
        return Err("CLI and original GUI executable identity differ".into());
    }
    let generation = status["generation"]
        .as_str()
        .ok_or("GUI generation unavailable")?
        .to_owned();
    let (id, action) = parse(args)?;
    let result = exchange(
        &directory,
        &Request::File {
            generation,
            request_id: id,
            deadline_ms: crate::unix_now_ms().saturating_add(5000),
            action,
        },
    )?;
    if result["ok"] != true {
        return Err(result["error"]
            .as_str()
            .unwrap_or("Original GUI refused file request")
            .to_owned()
            .into());
    }
    Ok(result)
}
fn parse(args: &[String]) -> Result<(String, Action), Error> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice(){
        ["--file-peers-json"]=>Ok((format!("peers-{}",crate::unix_now_ms()),Action::Peers)),
        ["--file-status",id] if valid_id(id)=>Ok(((*id).into(),Action::Status)),
        ["--file-shared-list",target,"--after-id",after,"--request-id",id]
            if valid_target(target) && valid_id(id) => Ok(((*id).into(), Action::ListShared {
                target: (*target).into(), after_id: after.parse()?,
            })),
        ["--file-shared-fetch",target,file,"--from-list",listed,"--sha256",sha,"--request-id",id,"--confirm-fetch"]
            if valid_target(target) && valid_id(listed) && valid_id(id) && valid_sha(sha)
                && file.parse::<u64>().is_ok_and(|n| n > 0) => Ok(((*id).into(), Action::FetchShared {
                    target: (*target).into(), file_id: file.parse()?, list_request_id: (*listed).into(), sha256: (*sha).into(),
                })),
        ["--file-resume",id,"--confirm-resume"] if valid_id(id)=>Ok(((*id).into(),Action::Resume)),
        ["--file-set-enabled",enabled,"--expected-preferences-sha256",version,"--expected-group-scope",scope,"--confirm-change"]
            if matches!(*enabled,"true"|"false") && valid_sha(version) && valid_sha(scope)
            =>Ok((format!("setting-{}",crate::unix_now_ms()),Action::SetEnabled{enabled:*enabled=="true",expected_preferences_sha256:(*version).into(),expected_group_scope:(*scope).into()})),
        ["--file-send",target,path,"--sha256",sha,"--request-id",id,"--confirm-send"] if valid_id(id)=>Ok(((*id).into(),Action::Send{target:(*target).into(),path:PathBuf::from(path),sha256:(*sha).into()})),
        _=>Err("Usage: --file-peers-json | --file-status REQUEST_ID | --file-shared-list TARGET --after-id CURSOR --request-id ID | --file-shared-fetch TARGET FILE_ID --from-list LIST_REQUEST_ID --sha256 SHA256 --request-id ID --confirm-fetch | --file-resume REQUEST_ID --confirm-resume | --file-set-enabled true|false --expected-preferences-sha256 SHA256 --expected-group-scope SHA256 --confirm-change | --file-send TARGET_DEVICE_ID ABSOLUTE_SOURCE --sha256 SHA256 --request-id REQUEST_ID --confirm-send".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sample(path: PathBuf) -> Record {
        Record {
            schema: 1,
            request_id: "send-1".into(),
            generation: "gui-1".into(),
            target: "paired-peer".into(),
            path,
            sha256: "a".repeat(64),
            bytes: 4,
            state: "accepted".into(),
            terminal: false,
            connection_id: None,
            transfer_id: None,
            transferred_bytes: 0,
            detail: String::new(),
            remote_content_verified: false,
            group_scope: Some("paired-scope".into()),
            resumed_attempts: 0,
            shared: None,
        }
    }
    #[test]
    fn requires_explicit_send_confirmation_and_safe_query_id() {
        assert!(parse(&["--file-send".into(), "peer".into(), "/tmp/file".into()]).is_err());
        assert!(parse(&["--file-status".into(), "../receipt".into()]).is_err());
        assert!(parse(&["--file-status".into(), "send-1".into()]).is_ok());
    }
    fn list_sample() -> Record {
        let mut record = sample(PathBuf::new());
        record.request_id = "list-1".into();
        record.state = "completed".into();
        record.terminal = true;
        record.connection_id = Some(17);
        record.sha256.clear();
        record.bytes = 0;
        record.shared = Some(SharedRecord {
            selection: SharedSelection::List { after_id: 0 }, native_request_id: Some(8),
            entries: vec![SharedFileInfo { id: 42, name: "explicit-test.txt".into(), size_bytes: 4 }],
            next_after_id: None, received_path: None,
        });
        record
    }
    #[test]
    fn shared_cli_requires_opaque_id_completed_list_hash_and_confirmation() {
        let args = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        assert!(parse(&args(&["--file-shared-list", "peer", "--after-id", "0", "--request-id", "list-1"])).is_ok());
        let sha = "a".repeat(64);
        let good = args(&["--file-shared-fetch", "peer", "42", "--from-list", "list-1", "--sha256", &sha, "--request-id", "pull-1", "--confirm-fetch"]);
        assert!(matches!(parse(&good).unwrap().1, Action::FetchShared { file_id: 42, .. }));
        assert!(parse(&good[..good.len()-1]).is_err());
        for invalid in ["0", "/Users/other/private.txt", "../42", "-1"] {
            let mut bad = good.clone(); bad[2] = invalid.into();
            assert!(parse(&bad).is_err());
        }
        let mut bad = good.clone(); bad[4] = "../list-1".into();
        assert!(parse(&bad).is_err());
        let mut bad = good; bad[6] = "unknown".into();
        assert!(parse(&bad).is_err());
    }
    #[test]
    fn pull_selection_is_bound_to_completed_list_peer_group_owner_and_entry() {
        let original = list_sample();
        let select = |r: &Record| selected_share(r, "gui-1", "paired-peer", Some("paired-scope"), 42);
        assert_eq!(select(&original).unwrap().1, 17);
        assert!(selected_share(&original, "gui-1", "paired-peer", Some("paired-scope"), 43).is_err());
        let mut altered = original.clone(); altered.target = "other-peer".into(); assert!(select(&altered).is_err());
        let mut altered = original.clone(); altered.generation = "old-owner".into(); assert!(select(&altered).is_err());
        let mut altered = original.clone(); altered.group_scope = Some("other-group".into()); assert!(select(&altered).is_err());
        let mut altered = original.clone(); altered.terminal = false; assert!(select(&altered).is_err());
        let mut altered = original.clone(); altered.state = "queued".into(); assert!(select(&altered).is_err());
        let mut altered = original.clone(); altered.connection_id = None; assert!(select(&altered).is_err());
        let mut altered = original.clone(); altered.shared.as_mut().unwrap().entries[0].name = "../private".into(); assert!(select(&altered).is_err());
        assert!(selected_share(&original, "gui-1", "paired-peer", None, 42).is_err());
    }
    #[test]
    fn shared_receipts_preserve_legacy_sends_and_reject_operation_rebinding_or_replay() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = sample(PathBuf::from("/payload"));
        let bytes = serde_json::to_vec(&legacy).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("\"shared\""));
        assert_eq!(serde_json::from_slice::<Record>(&bytes).unwrap(), legacy);
        let path = dir.path().join("list-1.json");
        let mut listed = list_sample();
        assert!(reserve(&path, &listed).unwrap());
        assert!(!reserve(&path, &listed).unwrap());
        listed.shared.as_mut().unwrap().selection = SharedSelection::List { after_id: 42 };
        assert!(reserve(&path, &listed).is_err());
        listed.state = "failed_before_send".into(); listed.connection_id = None;
        assert!(!safe_to_restart(&listed));
    }
    #[test]
    fn pulled_content_must_match_expected_digest_and_original_receive_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let received = root.join("test.txt"); fs::write(&received, b"test").unwrap();
        let sha = format!("{:x}", Sha256::digest(b"test"));
        assert!(verify_received(&received, &root, &sha, 4).is_ok());
        assert!(verify_received(&received, &root, &"b".repeat(64), 4).is_err());
        assert!(verify_received(&received, &root, &sha, 5).is_err());
        let other = tempfile::tempdir().unwrap();
        assert!(verify_received(&received, other.path(), &sha, 4).is_err());
        #[cfg(unix)] {
            let alias = root.join("alias"); std::os::unix::fs::symlink(&received, &alias).unwrap();
            assert!(verify_received(&alias, &root, &sha, 4).is_err());
        }
    }
    #[test]
    fn request_reservation_survives_restart_and_rejects_rebinding() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("send-1.json");
        let mut r = sample(d.path().join("payload"));
        assert!(reserve(&path, &r).unwrap());
        assert!(!reserve(&path, &r).unwrap());
        r.state = "completed".into();
        r.terminal = true;
        r.transfer_id = Some(17);
        save(&path, &r).unwrap();
        assert!(!reserve(&path, &r).unwrap());
        assert_eq!(read(&path).unwrap(), r);
        r.target = "different-peer".into();
        assert!(reserve(&path, &r).is_err());
    }
    #[test]
    fn source_digest_rejects_symlink_and_directory() {
        let d = tempfile::tempdir().unwrap();
        let p = fs::canonicalize(d.path()).unwrap().join("payload");
        fs::write(&p, b"test").unwrap();
        assert_eq!(
            digest(&p).unwrap(),
            (format!("{:x}", Sha256::digest(b"test")), 4)
        );
        assert!(digest(d.path()).is_err());
        #[cfg(unix)]
        {
            let link = d.path().join("alias");
            std::os::unix::fs::symlink(&p, &link).unwrap();
            assert!(digest(&link).is_err());
        }
    }
    #[test]
    fn receipt_directory_initializes_once_and_rejects_an_existing_link() {
        let fixture = tempfile::tempdir().unwrap();
        let gui = fs::canonicalize(fixture.path()).unwrap();
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; fs::set_permissions(&gui,fs::Permissions::from_mode(0o700)).unwrap(); }
        let selected = directory(&gui).unwrap();
        assert_eq!(directory(&gui).unwrap(), selected);
        crate::private_ipc::private_directory(&selected).unwrap();
        fs::remove_dir(&selected).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&gui, &selected).unwrap();
            assert!(directory(&gui).is_err());
            assert!(
                selected
                    .symlink_metadata()
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }

    #[tokio::test]
    async fn attached_frontend_uses_effective_preference_without_a_legacy_file_owner() {
        let (_tx, rx) = tokio::sync::watch::channel(Default::default());
        let backend = Arc::new(
            crate::UnifiedServiceOwner::from_background_snapshot(rx)
                .await
                .unwrap(),
        );
        assert!(!backend.file_transfer_enabled());
        let owner = OriginalOwner::new(backend);
        let mut configuration = crate::UnifiedRuntimeConfig::app_defaults();
        assert!(owner.native_file_transfer_enabled(&configuration));
        configuration.enable_file_transfer = false;
        assert!(!owner.native_file_transfer_enabled(&configuration));
        let standalone = OriginalOwner::new(Arc::new(
            crate::UnifiedServiceOwner::start(crate::UnifiedServiceOwnerConfig::default())
                .await
                .unwrap(),
        ));
        configuration.enable_file_transfer = true;
        assert!(!standalone.native_file_transfer_enabled(&configuration));
    }

    #[test]
    fn resume_refuses_unknown_admission_and_preserves_completed_receipts() {
        let mut r = sample(PathBuf::from("/payload"));
        r.terminal = true;
        r.state = "failed_before_send".into();
        assert!(safe_to_restart(&r));
        r.connection_id = Some(7);
        assert!(!safe_to_restart(&r));
        r.connection_id = None;
        r.transfer_id = Some(9);
        assert!(!safe_to_restart(&r));
        r.transfer_id = None;
        r.transferred_bytes = 1;
        assert!(!safe_to_restart(&r));
        r.transferred_bytes = 0;
        r.state = "not_confirmed".into();
        assert!(!safe_to_restart(&r));
        r.state = "completed".into();
        assert!(!safe_to_restart(&r));
        assert!(parse(&["--file-resume".into(), "send-1".into()]).is_err());
        assert!(
            parse(&[
                "--file-resume".into(),
                "send-1".into(),
                "--confirm-resume".into()
            ])
            .is_ok()
        );
    }
    #[tokio::test]
    async fn gui_control_directory_keeps_settings_in_profile_and_preserves_original_receipts() {
        let fixture = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(fixture.path()).unwrap().join("Weavelo");
        remote_core::desktop_storage::prepare_profile(&base).unwrap();
        let legacy = base.join("preferences.json");
        fs::write(
            &legacy,
            br#"{"side_services":{"file_transfer":false},"user_note":{"keep":true}}"#,
        )
        .unwrap();
        let legacy_bytes = fs::read(&legacy).unwrap();
        let profile = base.join("Desktop");
        remote_core::desktop_storage::prepare_profile(&profile).unwrap();
        let control = profile.join("gui-state");
        remote_core::desktop_storage::prepare_profile(&control).unwrap();
        let mut configuration = crate::UnifiedRuntimeConfig::app_defaults();
        configuration.mesh_dir = profile.clone();
        let (_tx, rx) = tokio::sync::watch::channel(Default::default());
        let backend = Arc::new(
            crate::UnifiedServiceOwner::from_background_snapshot(rx)
                .await
                .unwrap(),
        );
        let owner = OriginalOwner::new(backend);
        let peers = dispatch_scoped(
            owner.clone(),
            &control,
            &configuration,
            "gui-1",
            "query-1",
            Action::Peers,
            super::super::upgrade_quit::Operation::begin(&Default::default()),
        )
        .unwrap();
        assert_eq!(peers["stored_file_transfer_enabled"], false);
        assert_eq!(
            peers["preferences_path"],
            serde_json::json!(profile.join("preferences.json"))
        );
        assert!(
            !control.join("preferences.json").exists(),
            "IPC state is not a preferences profile"
        );
        let scoped = settings_profile(&control, &configuration.mesh_dir).unwrap();
        let version = peers["preferences_sha256"].as_str().unwrap();
        crate::preferences::UserPreferences::set_file_transfer(scoped, version, true).unwrap();
        assert!(
            crate::preferences::UserPreferences::load_from_path(&profile.join("preferences.json"))
                .unwrap()
                .side_services
                .file_transfer
        );
        assert_eq!(fs::read(&legacy).unwrap(), legacy_bytes);
        let receipt_path = directory(&control).unwrap().join("send-1.json");
        let receipt = sample(base.join("payload"));
        assert!(reserve(&receipt_path, &receipt).unwrap());
        let status = dispatch_scoped(
            owner,
            &control,
            &configuration,
            "gui-1",
            "send-1",
            Action::Status,
            super::super::upgrade_quit::Operation::begin(&Default::default()),
        )
        .unwrap();
        assert_eq!(status["file_request"]["request_id"], "send-1");
        assert_eq!(read(&receipt_path).unwrap(), receipt);
        assert!(
            !profile.join("file-requests").exists(),
            "Original receipt location must remain unchanged"
        );
        assert!(
            settings_profile(&profile, &profile).is_ok(),
            "Standalone GUI uses the same settings root"
        );
        assert!(settings_profile(&base, &profile).is_err());
    }

    #[test]
    fn incomplete_reservation_is_not_silently_replaced() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("send-1.json");
        fs::write(&p, b"partial").unwrap();
        assert!(reserve(&p, &sample(d.path().join("payload"))).is_err());
        assert_eq!(fs::read(&p).unwrap(), b"partial");
    }
}
