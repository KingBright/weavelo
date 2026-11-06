#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // A Dock tile owns only an AppKit event loop and its parent pipe. Avoid a
    // full network/codec executor, with one worker per CPU, for every window.
    #[cfg(all(target_os = "macos", feature = "gpui-restoration"))]
    if std::env::args().skip(1).eq(["--macos-dock-tile"]) {
        return weavelo_app::restored_ui::run_macos_dock_tile();
    }
    #[cfg(target_os = "linux")]
    prepare_linux_desktop_launch()?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(weavelo_app::entrypoint::run())
}

#[cfg(target_os = "linux")]
fn prepare_linux_desktop_launch() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::ffi::OsString;

    if !std::env::args().skip(1).eq(["--desktop-launch"]) {
        return Ok(());
    }

    let home = installed_linux_home(&std::env::current_exe()?)?;
    let profile = remote_core::desktop_identity::profile("linux", &home, None);
    let identity = remote_core::desktop_identity::identity();
    let settings: [(&str, OsString); 5] = [
        ("WEAVELO_DEVICE_GROUP_DIR", profile.into_os_string()),
        (
            "WEAVELO_HOST_BIND_ADDR",
            format!("0.0.0.0:{}", identity.control_port).into(),
        ),
        (
            "WEAVELO_DISCOVERY_PORT",
            identity.discovery_port.to_string().into(),
        ),
        ("WEAVELO_RELAY", "0".into()),
        ("WEAVELO_P2P", "0".into()),
    ];
    // SAFETY: this exact launch mode is handled in main before creating the Tokio runtime
    // or any worker threads, so no other thread can read the process environment concurrently.
    unsafe {
        for (key, value) in settings {
            std::env::set_var(key, value);
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn installed_linux_home(executable: &std::path::Path) -> Result<std::path::PathBuf, &'static str> {
    let executable = executable
        .canonicalize()
        .map_err(|_| "Linux desktop launch requires the installed Weavelo executable")?;
    let mut home = executable.clone();
    if executable.file_name() != Some(std::ffi::OsStr::new("weavelo")) {
        return Err("Linux desktop launch requires the installed Weavelo executable");
    }
    for expected_parent in ["desktop", "weavelo", "opt", ".local"] {
        if !home.pop() || home.file_name() != Some(std::ffi::OsStr::new(expected_parent)) {
            return Err("Linux desktop launch requires the installed Weavelo executable");
        }
    }
    if !home.pop() {
        return Err("Linux desktop launch requires the installed Weavelo executable");
    }
    if executable != home.join(".local/opt/weavelo/desktop/weavelo") {
        return Err("Linux desktop launch executable path is not canonical");
    }
    Ok(home)
}

#[cfg(test)]
mod tests {
    use super::installed_linux_home;
    use std::{fs, path::Path};

    #[test]
    fn desktop_launch_accepts_only_the_installed_canonical_binary_path() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let executable = home.join(".local/opt/weavelo/desktop/weavelo");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, b"fixture").unwrap();
        assert_eq!(installed_linux_home(&executable).unwrap(), home);

        let wrapper = home.join(".local/bin/weavelo-desktop");
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::write(&wrapper, b"fixture").unwrap();
        assert!(installed_linux_home(&wrapper).is_err());
        assert!(installed_linux_home(Path::new("/usr/bin/weavelo")).is_err());
    }
}
