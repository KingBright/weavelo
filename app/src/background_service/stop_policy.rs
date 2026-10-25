//! Read-only policy gate for the original per-user service. No start/stop/enable.
use serde_json::Value;
use std::{
    io,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(super) fn validate_launchd(value: &Value, executable: &str) -> Result<(), String> {
    if value["Label"] != remote_core::desktop_identity::identity().service_label
        || value["ProgramArguments"] != serde_json::json!([executable, "--background-service"])
        || value.get("Program").is_some_and(|p| p != executable)
    {
        return Err("Original per-user background launch identity required".into());
    }
    let triggers = [
        "StartInterval",
        "StartCalendarInterval",
        "WatchPaths",
        "QueueDirectories",
        "StartOnMount",
        "LaunchEvents",
        "Sockets",
        "MachServices",
    ];
    if triggers.iter().any(|key| value.get(*key).is_some()) {
        return Err("Independent background activation can undo explicit quit".into());
    }
    match value.get("KeepAlive") {
        None | Some(Value::Bool(false)) => Ok(()),
        Some(Value::Object(policy))
            if !policy.is_empty()
                && policy.iter().all(|(key, val)| {
                    (key == "SuccessfulExit" && *val == Value::Bool(false))
                        || (key == "Crashed" && *val == Value::Bool(true))
                }) =>
        {
            Ok(())
        }
        _ => Err("Background normal exit may restart; failure-only policy required".into()),
    }
}
pub(super) fn validate_systemd(text: &str, pid: u32) -> Result<(), String> {
    let mut fields = std::collections::BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line.split_once('=').ok_or("Invalid service observation")?;
        if fields.insert(key, value).is_some() {
            return Err("Duplicate service field".into());
        }
    }
    if fields.get("MainPID").copied() != Some(pid.to_string().as_str())
        || !matches!(fields.get("Restart").copied(), Some("no" | "on-failure"))
        || fields.get("ActiveState").copied() != Some("active")
        || fields.get("SubState").copied() != Some("running")
        || [
            "Job",
            "TriggeredBy",
            "RestartForceExitStatus",
            "ExecStopPost",
            "OnSuccess",
        ]
        .iter()
        .any(|k| fields.get(k).copied() != Some(""))
    {
        return Err(
            "Original running unit with failure-only restart and no external activation required"
                .into(),
        );
    }
    Ok(())
}
pub(super) fn systemd_object_path(unit: &str) -> String {
    let escaped=unit.bytes().map(|byte|if byte.is_ascii_alphanumeric() {
        (byte as char).to_string()
    } else {format!("_{byte:02x}")}).collect::<String>();
    format!("/org/freedesktop/systemd1/unit/{escaped}")
}
pub(super) fn complete_empty_exec_stop_post(text:&str, document:&Value)->Result<String,String> {
    if document.as_object().is_none_or(|value|value.len()!=2)
        || document["type"]!="a(sasbttttuii)" || document["data"]!=serde_json::json!([]) {
        return Err("Loaded ExecStopPost is nonempty or unavailable; stop refused".into());
    }
    let values=text.lines().filter_map(|line|line.strip_prefix("ExecStopPost=")).collect::<Vec<_>>();
    if values.len()>1 || values.iter().any(|value|!value.is_empty()) {
        return Err("Conflicting ExecStopPost observations; stop refused".into());
    }
    let mut completed=text.to_owned();
    if values.is_empty() {if !completed.ends_with('\n'){completed.push('\n');}completed.push_str("ExecStopPost=\n");}
    Ok(completed)
}
fn output(command: &mut Command) -> io::Result<Vec<u8>> {
    // Only this owned read-only helper is cancelled if it stalls.
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(io::Error::other("Background supervisor observation failed"));
            }
            use std::io::Read;
            let mut bytes = Vec::new();
            child
                .stdout
                .take()
                .unwrap()
                .take(65537)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 65536 {
                return Err(io::Error::other("Supervisor observation exceeds bound"));
            }
            return Ok(bytes);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Supervisor observation timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
pub(super) fn verify(
    pid: u32,
    executable: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let home = std::env::var_os("HOME").ok_or("Existing service home required")?;
        let path = std::path::PathBuf::from(home)
            .join(format!("Library/LaunchAgents/{}.plist", remote_core::desktop_identity::identity().service_label));
        let m = path.symlink_metadata()?;
        if !m.is_file()
            || m.uid() != unsafe { libc::geteuid() }
            || m.permissions().mode() & 0o022 != 0
            || m.len() > 65536
        {
            return Err("Original same-user launch policy is unsafe or unavailable".into());
        }
        let bytes = output(
            Command::new("/usr/bin/plutil")
                .args(["-convert", "json", "-o", "-"])
                .arg(&path),
        )?;
        validate_launchd(
            &serde_json::from_slice(&bytes)?,
            executable.to_str().ok_or("Executable path is not UTF-8")?,
        )?;
        // Check the loaded original job belongs to this PID; no service mutation.
        let label = format!("gui/{}/{}", unsafe {
            libc::geteuid()
        }, remote_core::desktop_identity::identity().service_label);
        let loaded = String::from_utf8(output(
            Command::new("/bin/launchctl").args(["print", &label]),
        )?)?;
        if !loaded
            .lines()
            .any(|line| line.trim() == format!("pid = {pid}"))
        {
            return Err("Loaded launchd owner changed".into());
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let _ = executable;
        let bytes = output(Command::new("systemctl").args([
            "--user",
            "show",
            "--all",
            &remote_core::desktop_identity::identity().linux_service,
            "-p",
            "MainPID",
            "-p",
            "Restart",
            "-p",
            "ActiveState",
            "-p",
            "SubState",
            "-p",
            "Job",
            "-p",
            "TriggeredBy",
            "-p",
            "RestartForceExitStatus",
            "-p",
            "ExecStopPost",
            "-p",
            "OnSuccess",
        ]))?;
        // systemctl versions may omit deprecated empty Exec* properties even
        // with --all. Require the original typed D-Bus property to prove empty;
        // absence alone never grants a normal/upgrade shutdown.
        let unit=remote_core::desktop_identity::identity().linux_service;
        let document=output(Command::new("busctl").args(["--user","--json=short","get-property",
            "org.freedesktop.systemd1",&systemd_object_path(&unit),"org.freedesktop.systemd1.Service","ExecStopPost"]))?;
        let completed=complete_empty_exec_stop_post(&String::from_utf8(bytes)?,&serde_json::from_slice(&document)?)?;
        validate_systemd(&completed, pid)?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (pid, executable);
        Err("No reviewed separated background supervisor on this platform; stop is not authorized by this entry".into())
    }
}
