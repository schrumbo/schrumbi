use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::core::prefs;

/// The Claude adapter release Schrumbi downloads; bump only after a round trip with the new one.
pub const CLAUDE_VERSION: &str = "0.104.1";

struct Build {
    os: &'static str,
    arch: &'static str,
    file: &'static str,
    sha256: &'static str,
}

const BUILDS: [Build; 4] = [
    Build { os: "macos", arch: "aarch64", file: "claude-agent-acp-darwin-arm64.zip", sha256: "c525aaafd98a23a0da7b85ce46527532b0d3399e5394aa715f669a7674f94ae0" },
    Build { os: "macos", arch: "x86_64", file: "claude-agent-acp-darwin-x64.zip", sha256: "e6ccbc6a21989d4967a62b18749dc36b9ea925194e3d24254ccddd3ad45eae8c" },
    Build { os: "windows", arch: "x86_64", file: "claude-agent-acp-windows-x64.zip", sha256: "a98356963cc0fcbd8b04ccdb93d326c06d96741d47d23a13aaf34e67ef7dd1e4" },
    Build { os: "windows", arch: "aarch64", file: "claude-agent-acp-windows-arm64.zip", sha256: "25b53ae419cf9d527d4cdc8f88891f03a4f05a56bf0e6f19cf7b643674f63620" },
];

fn build() -> Option<&'static Build> {
    BUILDS.iter().find(|b| b.os == std::env::consts::OS && b.arch == std::env::consts::ARCH)
}

/// Whether this platform has a download; elsewhere the adapter runs through Node.
pub fn downloadable() -> bool {
    build().is_some()
}

fn folder() -> Option<PathBuf> {
    prefs::data_dir().map(|d| d.join("ai").join("agents").join(format!("claude-agent-acp-{CLAUDE_VERSION}")))
}

fn binary_name() -> &'static str {
    if cfg!(windows) { "claude-agent-acp.exe" } else { "claude-agent-acp" }
}

/// The downloaded Claude adapter, if it is there.
pub fn claude_adapter() -> Option<PathBuf> {
    folder().map(|f| f.join(binary_name())).filter(|p| p.is_file())
}

/// The sha256 of a file, as lowercase hex.
pub fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Unpacks the adapter's zip into `into`, refusing entries that would land outside it.
pub fn unpack(zip_path: &Path, into: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = entry.enclosed_name() else { return Err("unsafe path in archive".into()) };
        let target = into.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(())
}

/// Downloads the pinned Claude adapter, checks its sha256 and unpacks it into the app data; reports progress from 0 to 1.
pub fn install_claude(progress: impl Fn(f32)) -> Result<PathBuf, String> {
    if crate::core::drive::fenced("download claude adapter", CLAUDE_VERSION) {
        return Err("an agent drives this instance".into());
    }
    let build = build().ok_or("no download for this platform")?;
    let folder = folder().ok_or("no data folder")?;
    let url = format!("https://github.com/agentclientprotocol/claude-agent-acp/releases/download/v{CLAUDE_VERSION}/{}", build.file);
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(1800))).user_agent(concat!("Schrumbi/", env!("CARGO_PKG_VERSION"))).build().into();
    let response = agent.get(&url).call().map_err(|e| e.to_string())?;
    let total = response.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let staging = folder.with_extension("download");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let zip_path = staging.join(build.file);
    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
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
    let hash = sha256_of(&zip_path).map_err(|e| e.to_string())?;
    if hash != build.sha256 {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("checksum mismatch".into());
    }
    let unpacked = staging.join("out");
    unpack(&zip_path, &unpacked)?;
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::rename(&unpacked, &folder).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&staging);
    claude_adapter().ok_or_else(|| "the download holds no adapter".into())
}
