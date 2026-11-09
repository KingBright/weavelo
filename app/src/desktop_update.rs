//! Release discovery and handoff to the trusted platform installer.
use serde::Deserialize;
use std::io::Read;

const MANIFEST: &str = "https://weavelo.hackerlife.fun:8443/download/desktop-releases.json";
const LIMIT: u64 = 128 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub source_commit: String,
    pub same_source: Option<bool>,
}

#[derive(Deserialize)]
struct Manifest { schema: u32, releases: Vec<Entry> }
#[derive(Deserialize)]
struct Entry { file: String, version: String, source_commit: String }

fn select(data: &[u8], prefix: &str, current: Option<&str>) -> Result<Release, String> {
    let manifest: Manifest = serde_json::from_slice(data).map_err(|_| "Invalid release information")?;
    if manifest.schema != 1 { return Err("Unsupported release information".into()); }
    let mut matches = manifest.releases.into_iter().filter(|entry| {
        entry.file.starts_with(prefix) && entry.file.ends_with(".zip")
            && !entry.file.contains(['/', '\\'])
    });
    let entry = matches.next().ok_or("No package for this device")?;
    if matches.next().is_some() { return Err("Ambiguous release information".into()); }
    if entry.source_commit.len() != 40 || !entry.source_commit.bytes().all(|c| c.is_ascii_hexdigit())
        || entry.version.is_empty() || entry.version.len() > 80 || entry.version.chars().any(char::is_control) {
        return Err("Invalid release information".into());
    }
    Ok(Release { same_source: current.map(|commit| commit == entry.source_commit), version: entry.version, source_commit: entry.source_commit })
}

pub fn check() -> Result<Release, String> {
    let prefix = if cfg!(all(target_os="macos", target_arch="aarch64")) { "Weavelo-desktop-macos-arm64-" }
        else if cfg!(all(target_os="linux", target_arch="x86_64")) { "Weavelo-desktop-linux-x64-" }
        else if cfg!(all(target_os="windows", target_arch="x86_64")) { "Weavelo-Windows-x64-" }
        else { return Err("No package for this device".into()); };
    #[cfg(target_os="macos")]
    let signer_manifest = verified_local_app().ok().map(|(_, _, signer)| {
        format!("https://weavelo.hackerlife.fun:8443/download/desktop-releases-macos-{signer}.json")
    });
    #[cfg(not(target_os="macos"))]
    let signer_manifest: Option<String> = None;
    let data = match signer_manifest {
        Some(url) => match fetch_metadata(&url)? {
            Some(data) => data,
            None => fetch_metadata(MANIFEST)?.ok_or("No package for this device")?,
        },
        None => fetch_metadata(MANIFEST)?.ok_or("No package for this device")?,
    };
    let identity = crate::build_identity::compiled();
    let current = identity.as_ref().filter(|v| v["source"]["state"] == "clean")
        .and_then(|v| v["source"]["commit"].as_str());
    select(&data, prefix, current)
}

fn fetch_metadata(url: &str) -> Result<Option<Vec<u8>>, String> {
    #[cfg(target_os="windows")]
    let curl = std::path::PathBuf::from(std::env::var_os("SystemRoot").ok_or("Update check unavailable")?).join("System32").join("curl.exe");
    #[cfg(not(target_os="windows"))]
    let curl = std::path::PathBuf::from("/usr/bin/curl");
    let mut child = std::process::Command::new(curl)
        .args(["-q", "--silent", "--show-error", "--proto", "=https", "--connect-timeout", "10",
            "--max-time", "20", "--max-filesize", "131072", "--write-out", "%{http_code}", url])
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
        .spawn().map_err(|_| "Update check unavailable")?;
    let mut data = Vec::new();
    let read = child.stdout.take().ok_or("Update check unavailable")?.take(LIMIT + 4).read_to_end(&mut data);
    if read.is_err() || data.len() as u64 > LIMIT + 3 {
        let _ = child.kill(); let _ = child.wait();
        return Err("Invalid release information".into());
    }
    if !child.wait().map_err(|_| "Update check unavailable")?.success() { return Err("Update check failed; try again when online".into()); }
    let code = data.len().checked_sub(3).ok_or("Invalid release information")?;
    match &data[code..] {
        b"404" => Ok(None),
        b"200" => { data.truncate(code); Ok(Some(data)) }
        _ => Err("Update check failed; try again when online".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
    #[test]
    fn selection_is_platform_specific_and_source_comparison_is_optional() {
        let doc = format!(r#"{{"schema":1,"releases":[{{"file":"Weavelo-Windows-x64-a.zip","version":"2","source_commit":"{HASH}"}},{{"file":"Weavelo-desktop-linux-x64-a.zip","version":"2","source_commit":"{HASH}"}}]}}"#);
        assert_eq!(select(doc.as_bytes(), "Weavelo-desktop-linux-x64-", Some(HASH)).unwrap().same_source, Some(true));
        assert_eq!(select(doc.as_bytes(), "Weavelo-desktop-linux-x64-", None).unwrap().same_source, None);
        assert_eq!(select(doc.as_bytes(), "Weavelo-desktop-linux-x64-", Some("other")).unwrap().same_source, Some(false));
    }
    #[test]
    fn rejects_ambiguous_or_unsafe_entries() {
        let entry = format!(r#"{{"file":"Weavelo-Windows-x64-a.zip","version":"2","source_commit":"{HASH}"}}"#);
        let duplicate = format!(r#"{{"schema":1,"releases":[{entry},{entry}]}}"#);
        assert!(select(duplicate.as_bytes(), "Weavelo-Windows-x64-", None).is_err());
        let bad = format!(r#"{{"schema":1,"releases":[{{"file":"Weavelo-Windows-x64-../bad.zip","version":"2","source_commit":"{HASH}"}}]}}"#);
        assert!(select(bad.as_bytes(), "Weavelo-Windows-x64-", None).is_err());
    }
}

#[cfg(target_os = "macos")]
fn verified_local_app() -> Result<(std::path::PathBuf, std::path::PathBuf, String), String> {
    use std::path::PathBuf;
    use std::process::Command;
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("Cannot locate the installed application")?)
        .canonicalize().map_err(|_| "Cannot locate the installed application")?;
    let app = home.join("Applications/Weavelo/Weavelo.app");
    let executable = app.join("Contents/MacOS/weavelo").canonicalize()
        .map_err(|_| "Cannot locate the installed application")?;
    if std::env::current_exe().and_then(|p| p.canonicalize()).ok().as_ref() != Some(&executable) {
        return Err("Updates require the installed Weavelo application".into());
    }
    let resolved_app = app.canonicalize().map_err(|_| "Cannot locate the installed application")?;
    if resolved_app != app {
        return Err("Installed application path is linked or has changed".into());
    }
    let app = resolved_app;
    if !Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"])
        .arg(&app).status().map_err(|_| "Cannot verify the installed application")?.success() {
        return Err("Cannot verify the installed application".into());
    }
    // The exact allowed leaf certificates are compiled into this signed executable;
    // the bundled policy files alone cannot authorize the helper to run.
    let policies = [include_str!("../../deploy/macos/desktop-signing-policy.json"),
        include_str!("../../deploy/macos/book-desktop-signing-policy.json")];
    let signer = policies.iter().find_map(|raw| {
        let policy = serde_json::from_str::<serde_json::Value>(raw).ok()?;
        let leaf = policy["certificate_sha1"].as_str()?;
        if policy["bundle_id"] != "fun.hackerlife.weavelo.desktop" || leaf.len() != 40
            || !leaf.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
        let requirement = format!("=identifier \"fun.hackerlife.weavelo.desktop\" and certificate leaf = H\"{leaf}\"");
        Command::new("/usr/bin/codesign").args(["--verify", "--strict", "-R", &requirement])
            .arg(&app).stderr(std::process::Stdio::null()).status().ok()?
            .success().then(|| leaf.to_ascii_lowercase())
    }).ok_or("Installed application signer is not authorized")?;
    Ok((home, app, signer))
}

#[cfg(target_os = "macos")]
pub fn install_latest_macos() -> Result<String, String> {
    use std::process::Command;
    let (home, app, _) = verified_local_app()?;
    let helper = app.join("Contents/Resources/desktop-updater/scripts/desktop_in_app_update.py");
    if !helper.is_file() { return Err("This installation does not include the trusted updater".into()); }
    // Isolated Python excludes the working directory and user site; explicitly
    // load only the helper modules inside the just-verified signed bundle.
    let bootstrap = "import runpy,sys; p=sys.argv[1]; sys.path.insert(0,p.rsplit('/',1)[0]); runpy.run_path(p,run_name='__main__')";
    let mut command = Command::new("/usr/bin/python3");
    command.env_clear().env("HOME", &home).env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .args(["-I", "-B", "-c", bootstrap]).arg(helper);
    for key in ["USER", "LOGNAME"] {
        if let Some(value) = std::env::var_os(key) { command.env(key, value); }
    }
    let output = command.output().map_err(|_| "Trusted Python updater is unavailable")?;
    let result: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Trusted updater did not return a result")?;
    if result["ok"] == true {
        Ok(if result["outcome"] == "already_installed" { "Latest package is already installed" }
            else { "Update completed" }.into())
    } else {
        Err(result["error"].as_str().unwrap_or("Update failed").chars().take(1000).collect())
    }
}
