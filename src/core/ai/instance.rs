use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::prefs;

/// A running Schrumbi that accepts tool calls: `schrumbi mcp` finds it through this file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub root: Option<PathBuf>,
    /// Seconds since 1970.
    pub started: i64,
}

/// Where instance files live; one per app process, named by its pid.
pub fn dir() -> Option<PathBuf> {
    prefs::data_dir().map(|d| d.join("ai").join("instances"))
}

fn file(pid: u32) -> Option<PathBuf> {
    dir().map(|d| d.join(format!("{pid}.json")))
}

/// Writes the file for this instance, replacing it in one step.
pub fn write(instance: &Instance) -> std::io::Result<()> {
    let Some(path) = file(instance.pid) else { return Err(std::io::Error::other("no data folder")) };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(instance).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

/// Removes the file for `pid`, if any.
pub fn remove(pid: u32) {
    if let Some(path) = file(pid) {
        let _ = std::fs::remove_file(path);
    }
}

/// Every live instance; files of dead processes are removed on the way.
pub fn all() -> Vec<Instance> {
    let Some(dir) = dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(instance) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Instance>(&b).ok()) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if alive(instance.pid) {
            out.push(instance);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.started));
    out
}

/// The instance a call from `cwd` belongs to: the forced pid, else the one whose folder holds `cwd`;
/// the youngest only while no instance has a folder open, so a terminal elsewhere never reaches the study folder.
pub fn pick(cwd: &Path, forced: Option<u32>) -> Option<Instance> {
    pick_from(all(), cwd, forced)
}

fn pick_from(all: Vec<Instance>, cwd: &Path, forced: Option<u32>) -> Option<Instance> {
    if let Some(pid) = forced {
        return all.into_iter().find(|i| i.pid == pid);
    }
    let cwd = dunce::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let holds = |i: &Instance| i.root.as_ref().is_some_and(|r| cwd.starts_with(dunce::canonicalize(r).unwrap_or_else(|_| r.clone())));
    if let Some(found) = all.iter().find(|i| holds(i)) {
        return Some(found.clone());
    }
    if all.iter().any(|i| i.root.is_some()) {
        return None;
    }
    all.into_iter().next()
}

/// The study folders running instances have open, for telling a terminal where the tools work.
pub fn roots() -> Vec<PathBuf> {
    all().into_iter().filter_map(|i| i.root).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(pid: u32, root: Option<&Path>, started: i64) -> Instance {
        Instance { pid, port: 1, token: String::new(), root: root.map(Path::to_path_buf), started }
    }

    #[test]
    fn a_terminal_outside_every_study_folder_reaches_no_instance() {
        let base = std::env::temp_dir().join(format!("schrumbi-pick-{}", std::process::id()));
        let (study, elsewhere) = (base.join("Studium"), base.join("Downloads"));
        std::fs::create_dir_all(&study).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let both = vec![at(2, Some(&study), 20), at(1, None, 10)];
        assert_eq!(pick_from(both.clone(), &study.join("MA"), None).map(|i| i.pid), Some(2));
        assert_eq!(pick_from(both.clone(), &elsewhere, None), None);
        assert_eq!(pick_from(both, &elsewhere, Some(1)).map(|i| i.pid), Some(1));
        assert_eq!(pick_from(vec![at(1, None, 10)], &elsewhere, None).map(|i| i.pid), Some(1));
    }
}

#[cfg(unix)]
pub fn alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else { return false };
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

/// Whether the process still runs.
#[cfg(windows)]
pub fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        ok && code == STILL_ACTIVE as u32
    }
}

#[cfg(not(any(unix, windows)))]
pub fn alive(_: u32) -> bool {
    true
}
