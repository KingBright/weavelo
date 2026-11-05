use protocol::appearance::{DeviceBadge, DeviceBadgePreset as Preset};

pub fn automatic() -> Preset {
    #[cfg(target_os = "macos")]
    {
        static MODEL: std::sync::OnceLock<Preset> = std::sync::OnceLock::new();
        return *MODEL.get_or_init(|| {
            let model = std::process::Command::new("/usr/sbin/system_profiler")
                .args([
                    "-timeout",
                    "2",
                    "SPHardwareDataType",
                    "-detailLevel",
                    "mini",
                    "-json",
                ])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
                .and_then(|value| {
                    value["SPHardwareDataType"][0]["machine_name"]
                        .as_str()
                        .map(str::to_owned)
                });
            match model.as_deref() {
                Some(name) if name.starts_with("MacBook") => Preset::MacBook,
                Some("Mac Studio") => Preset::MacStudio,
                _ => Preset::Computer,
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    if cfg!(target_os = "android") {
        Preset::Phone
    } else if cfg!(target_os = "windows") {
        Preset::Windows
    } else if cfg!(target_os = "linux") {
        Preset::Linux
    } else {
        Preset::Computer
    }
}

pub fn resolve(value: DeviceBadge) -> Result<DeviceBadge, String> {
    match value {
        DeviceBadge::Automatic => Ok(DeviceBadge::Preset(automatic())),
        DeviceBadge::Custom(ref icon) => {
            crate::application_icons::decode_artwork(icon, false)?;
            Ok(value)
        }
        _ => Ok(value),
    }
}

/// The GUI writes this existing profile document atomically. The background
/// reads it only when a viewer requests artwork, so changes need no restart.
pub fn desktop_badge() -> Result<DeviceBadge, String> {
    use std::io::Read;
    let path = crate::mesh::default_app_private_mesh_dir().join("preferences.json");
    let mut file =
        match crate::desktop_storage::open_regular(&path, true).map_err(|e| e.to_string())? {
            Some(file) => file,
            None => return resolve(DeviceBadge::Automatic),
        };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("device preferences are oversized".into());
    }
    let document: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let badge = document["ui"]
        .get("device_badge")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    resolve(badge)
}
