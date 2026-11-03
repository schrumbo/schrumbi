use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

const LATEST: &str = "https://api.github.com/repos/schrumbo/schrumbi/releases/latest";

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    html_url: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
}

/// A newer version than the running one.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    /// The installer for this platform and its checksum file; `None` means updating by hand.
    pub installer: Option<(String, String)>,
}

/// Which installer this platform can apply by itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Mac,
    Windows,
    AppImage,
    Manual,
}

/// How the running app was installed.
pub fn target() -> Target {
    if cfg!(target_os = "macos") {
        Target::Mac
    } else if cfg!(windows) {
        Target::Windows
    } else if std::env::var_os("APPIMAGE").is_some() {
        Target::AppImage
    } else {
        Target::Manual
    }
}

fn matches(name: &str, target: Target) -> bool {
    match target {
        Target::Mac => name.ends_with(".dmg"),
        Target::Windows => name.ends_with(".exe"),
        Target::AppImage => name.ends_with(".AppImage"),
        Target::Manual => false,
    }
}

/// Reads GitHub's release JSON; `None` if it is no newer stable version than `current`.
pub fn parse(json: &str, current: &str, target: Target) -> Option<Release> {
    let r: ApiRelease = serde_json::from_str(json).ok()?;
    let version = r.tag_name.trim_start_matches('v').to_string();
    if r.prerelease || r.draft || semver::Version::parse(&version).ok()? <= semver::Version::parse(current).ok()? {
        return None;
    }
    let installer = r.assets.iter().find(|a| matches(&a.name, target)).and_then(|a| {
        let checksum = format!("{}.sha256", a.name);
        r.assets.iter().find(|s| s.name == checksum).map(|s| (a.browser_download_url.clone(), s.browser_download_url.clone()))
    });
    Some(Release { version, notes: r.body, page: r.html_url, installer })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(600))).user_agent(concat!("Schrumbi/", env!("CARGO_PKG_VERSION"))).build().into()
}

fn latest_url() -> String {
    if cfg!(debug_assertions)
        && let Ok(url) = std::env::var("SCHRUMBI_UPDATE_URL")
    {
        return url;
    }
    LATEST.to_string()
}

/// Asks GitHub for the latest release. `Ok(None)` means up to date.
pub fn check() -> Result<Option<Release>, String> {
    let body = agent().get(latest_url()).header("Accept", "application/vnd.github+json").call().map_err(|e| e.to_string())?.into_body().read_to_string().map_err(|e| e.to_string())?;
    Ok(parse(&body, env!("CARGO_PKG_VERSION"), target()))
}

/// The hash in a `sha256sum` line.
pub fn parse_checksum(line: &str) -> Option<String> {
    let hash = line.split_whitespace().next()?.to_ascii_lowercase();
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
}

pub fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Downloads the installer and verifies it against its checksum file.
pub fn download(url: &str, checksum_url: &str, progress: impl Fn(f32)) -> Result<PathBuf, String> {
    let agent = agent();
    let line = agent.get(checksum_url).call().map_err(|e| e.to_string())?.into_body().read_to_string().map_err(|e| e.to_string())?;
    let expected = parse_checksum(&line).ok_or("invalid checksum file")?;
    let name = url.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("schrumbi-update");
    let path = std::env::temp_dir().join(name);
    let response = agent.get(url).call().map_err(|e| e.to_string())?;
    let total = response.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut buf = vec![0; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if let Some(total) = total {
            progress((done as f32 / total.max(1) as f32).min(1.));
        }
    }
    drop(file);
    if file_hash(&path)? != expected {
        let _ = std::fs::remove_file(&path);
        return Err("checksum mismatch".into());
    }
    Ok(path)
}

/// The app bundle around an executable, e.g. `/Applications/X.app`.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

/// Replaces the running app with the installer's and starts the new version shortly after this one quits.
pub fn install(installer: &Path) -> Result<(), String> {
    if super::drive::fenced("install update", installer.display()) {
        return Err("an agent drives this instance".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    match target() {
        Target::Mac => install_mac(installer, &exe),
        Target::Windows => install_windows(installer, &exe),
        Target::AppImage => install_appimage(installer),
        Target::Manual => Err("manual".into()),
    }
}

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let status = std::process::Command::new(program).args(args).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("{program}: {status}")) }
}

fn install_mac(dmg: &Path, exe: &Path) -> Result<(), String> {
    let bundle = bundle_of(exe).ok_or("not running from an app bundle")?;
    let mount = std::env::temp_dir().join(format!("schrumbi-update-{}", std::process::id()));
    std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
    run("hdiutil", &["attach".as_ref(), dmg.as_os_str(), "-nobrowse".as_ref(), "-readonly".as_ref(), "-mountpoint".as_ref(), mount.as_os_str()])?;
    let staged = bundle.with_extension("app.new");
    let copied = std::fs::read_dir(&mount).map_err(|e| e.to_string()).and_then(|entries| {
        let source = entries.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "app")).ok_or("no app in the disk image".to_string())?;
        let _ = std::fs::remove_dir_all(&staged);
        run("ditto", &[source.as_os_str(), staged.as_os_str()])
    });
    let _ = run("hdiutil", &["detach".as_ref(), mount.as_os_str(), "-force".as_ref()]);
    copied?;
    let old = bundle.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(&bundle, &old).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&staged, &bundle) {
        let _ = std::fs::rename(&old, &bundle);
        return Err(e.to_string());
    }
    let _ = std::fs::remove_dir_all(&old);
    std::process::Command::new("/bin/sh").args(["-c", "sleep 1; open \"$0\""]).arg(&bundle).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

fn install_windows(installer: &Path, exe: &Path) -> Result<(), String> {
    let quote = |p: &Path| p.to_string_lossy().replace('\'', "''");
    let script = format!("Start-Sleep 2; Start-Process -Wait -FilePath '{}' -ArgumentList '/S'; Start-Process -FilePath '{}'", quote(installer), quote(exe));
    std::process::Command::new("powershell").args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script]).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

fn install_appimage(new: &Path) -> Result<(), String> {
    let current = PathBuf::from(std::env::var_os("APPIMAGE").ok_or("no AppImage")?);
    let staged = current.with_extension("new");
    std::fs::copy(new, &staged).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&staged, &current).map_err(|e| e.to_string())?;
    std::process::Command::new("/bin/sh").args(["-c", "sleep 1; exec \"$0\""]).arg(&current).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Target, bundle_of, download, file_hash, parse, parse_checksum};

    fn json(tag: &str, pre: bool) -> String {
        serde_json::json!({
            "tag_name": tag, "body": "Neu", "html_url": "https://x/r", "prerelease": pre,
            "assets": [
                { "name": "S-1.dmg", "browser_download_url": "https://x/dmg" }, { "name": "S-1.dmg.sha256", "browser_download_url": "https://x/dmg.sha" },
                { "name": "S.exe", "browser_download_url": "https://x/exe" },
                { "name": "S.AppImage", "browser_download_url": "https://x/ai" }, { "name": "S.AppImage.sha256", "browser_download_url": "https://x/ai.sha" },
                { "name": "s.deb", "browser_download_url": "https://x/deb" },
            ]
        })
        .to_string()
    }

    #[test]
    fn finds_only_newer_stable_versions() {
        assert_eq!(parse(&json("v0.2.0", false), "0.1.0", Target::Mac).unwrap().version, "0.2.0");
        assert_eq!(parse(&json("v0.1.0", false), "0.1.0", Target::Mac), None);
        assert_eq!(parse(&json("v0.0.9", false), "0.1.0", Target::Mac), None);
        assert_eq!(parse(&json("v0.2.0", true), "0.1.0", Target::Mac), None);
        assert_eq!(parse(&json("v0.10.0", false), "0.9.0", Target::Mac).unwrap().version, "0.10.0");
        assert_eq!(parse(&json("kaputt", false), "0.1.0", Target::Mac), None);
        assert_eq!(parse("kein json", "0.1.0", Target::Mac), None);
    }

    #[test]
    fn picks_the_installer_for_the_platform() {
        let installer = |t| parse(&json("v1.0.0", false), "0.1.0", t).unwrap().installer;
        assert_eq!(installer(Target::Mac), Some(("https://x/dmg".into(), "https://x/dmg.sha".into())));
        assert_eq!(installer(Target::AppImage), Some(("https://x/ai".into(), "https://x/ai.sha".into())));
        assert_eq!(installer(Target::Manual), None);
        assert_eq!(installer(Target::Windows), None, "no checksum file, so no automatic install");
    }

    #[test]
    fn reads_and_computes_checksums() {
        let hash = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert_eq!(parse_checksum(&format!("{hash}  file.dmg\n")).as_deref(), Some(hash));
        assert_eq!(parse_checksum(&hash.to_uppercase()).as_deref(), Some(hash));
        assert_eq!(parse_checksum("kurz  file.dmg"), None);
        assert_eq!(parse_checksum(""), None);
        let path = std::env::temp_dir().join(format!("schrumbi-hash-{}", std::process::id()));
        std::fs::write(&path, "hello world").unwrap();
        assert_eq!(file_hash(&path).unwrap(), hash);
        let _ = std::fs::remove_file(path);
    }

    /// Serves one response per connection: the checksum line first, then the file.
    fn serve(responses: Vec<Vec<u8>>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for body in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 2048];
                let _ = std::io::Read::read(&mut stream, &mut request);
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = std::io::Write::write_all(&mut stream, head.as_bytes());
                let _ = std::io::Write::write_all(&mut stream, &body);
            }
        });
        format!("http://{address}")
    }

    #[test]
    fn rejects_a_download_with_the_wrong_checksum() {
        let hash = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let base = serve(vec![format!("{hash}  f\n").into_bytes(), b"hello world".to_vec()]);
        let ok = download(&format!("{base}/schrumbi-ok-{}", std::process::id()), &format!("{base}/sum"), |_| {}).unwrap();
        assert_eq!(std::fs::read(&ok).unwrap(), b"hello world");
        let _ = std::fs::remove_file(ok);

        let name = format!("schrumbi-bad-{}", std::process::id());
        let base = serve(vec![format!("{hash}  f\n").into_bytes(), b"manipuliert".to_vec()]);
        assert_eq!(download(&format!("{base}/{name}"), &format!("{base}/sum"), |_| {}), Err("checksum mismatch".to_string()));
        assert!(!std::env::temp_dir().join(name).exists());
    }

    #[test]
    fn finds_the_app_bundle() {
        assert_eq!(bundle_of(std::path::Path::new("/Applications/S.app/Contents/MacOS/s")).unwrap(), std::path::Path::new("/Applications/S.app"));
        assert_eq!(bundle_of(std::path::Path::new("/repo/target/debug/s")), None);
    }
}
