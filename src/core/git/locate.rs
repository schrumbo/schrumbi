use std::path::{Path, PathBuf};
use std::process::Command;

use super::cmd::{GitError, LOCAL, spawn};

/// `merge-tree --write-tree`, which previews conflicts without touching files, came with this version.
const MINIMUM: (u32, u32) = (2, 38);

/// Finds a usable Git: the given path, then `PATH`, then the usual places.
pub fn find(preferred: Option<&Path>) -> Result<PathBuf, GitError> {
    let mut old = None;
    for candidate in candidates(preferred) {
        match version(&candidate) {
            Some(v) if at_least(&v) => return Ok(candidate),
            Some(v) => old = Some(v),
            None => {}
        }
    }
    Err(old.map_or(GitError::Missing, GitError::TooOld))
}

fn candidates(preferred: Option<&Path>) -> Vec<PathBuf> {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    let mut list: Vec<PathBuf> = preferred.map(Path::to_path_buf).into_iter().collect();
    if let Some(path) = std::env::var_os("PATH") {
        list.extend(std::env::split_paths(&path).map(|d| d.join(name)));
    }
    if cfg!(windows) {
        list.push(PathBuf::from(r"C:\Program Files\Git\cmd\git.exe"));
    } else {
        list.push(PathBuf::from("/opt/homebrew/bin/git"));
        list.push(PathBuf::from("/usr/local/bin/git"));
        list.push(PathBuf::from("/usr/bin/git"));
    }
    list.retain(|p| p.is_file() && usable_stub(p));
    list.dedup();
    list
}

/// On macOS `/usr/bin/git` without developer tools only opens an install dialog when run.
fn usable_stub(path: &Path) -> bool {
    if !cfg!(target_os = "macos") || path != Path::new("/usr/bin/git") {
        return true;
    }
    let mut c = Command::new("/usr/bin/xcode-select");
    c.arg("-p");
    spawn(c, None, LOCAL).is_ok_and(|o| o.code == 0)
}

fn version(exe: &Path) -> Option<String> {
    let mut c = Command::new(exe);
    c.arg("--version").env("LC_ALL", "C");
    let out = spawn(c, None, LOCAL).ok()?;
    (out.code == 0).then(|| parse_version(&out.stdout))?
}

/// `git version 2.50.1 (Apple Git-155)` → `2.50.1`.
pub fn parse_version(text: &str) -> Option<String> {
    text.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit())).map(String::from)
}

pub fn at_least(version: &str) -> bool {
    let mut parts = version.split('.').filter_map(|p| p.parse::<u32>().ok());
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0)) >= MINIMUM
}
