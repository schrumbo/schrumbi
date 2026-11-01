use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use chrono::{Datelike, Days, Local};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::json::to_swift_json;
use super::model::{Module, PALETTE, Semester, StudyConfig};
use super::{day, prefs};

pub const CONFIG_NAME: &str = "schrumbi.json";
pub const STATE_NAME: &str = ".schrumbi-state.json";
pub const BACKUP_FOLDER: &str = ".schrumbi-backup";
const BACKUP_INTERVAL: Duration = Duration::from_secs(10 * 60);
const BACKUPS_KEPT: usize = 12;

/// The workspace from the first CLI argument, else the last one opened.
pub fn resolve() -> Option<PathBuf> {
    if let Some(dir) = std::env::args_os().skip(1).map(PathBuf::from).find(|p| p.is_dir()) {
        return Some(dir);
    }
    prefs::get().workspace.filter(|p| p.is_dir())
}

/// `Ok(None)` when the file does not exist.
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
    Ok(Some(serde_json::from_slice(&bytes).with_context(|| path.display().to_string())?))
}

/// Writes atomically after taking a rolling backup.
pub fn save<T: Serialize>(value: &T, path: &Path) -> Result<()> {
    backup(path);
    let json = to_swift_json(value)?;
    let tmp = path.with_file_name(format!(".{}.tmp", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Renames files written by earlier versions (studium.json, .studium-state.json, .studium-backup).
pub fn migrate_legacy(root: &Path) {
    for (old, new) in [("studium.json", CONFIG_NAME), (".studium-state.json", STATE_NAME), (".studium-backup", BACKUP_FOLDER)] {
        let (o, n) = (root.join(old), root.join(new));
        if o.exists() && !n.exists() {
            let _ = std::fs::rename(o, n);
        }
    }
}

fn backup_base(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().trim_matches('.').to_string()).unwrap_or_default()
}

/// Backups of `path`, newest first.
pub fn backups(path: &Path) -> Vec<PathBuf> {
    let dir = path.with_file_name(BACKUP_FOLDER);
    let prefix = format!("{}-", backup_base(path));
    let mut items: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
    items.retain(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix)));
    items.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    items
}

/// Copies the current file into `.schrumbi-backup/` at most every 10 minutes, keeping the latest 12.
pub fn backup(path: &Path) {
    if !path.exists() {
        return;
    }
    let dir = path.with_file_name(BACKUP_FOLDER);
    let _ = std::fs::create_dir_all(&dir);
    if let Some(last) = backups(path).first()
        && let Ok(created) = std::fs::metadata(last).and_then(|m| m.created().or_else(|_| m.modified()))
        && SystemTime::now().duration_since(created).unwrap_or_default() < BACKUP_INTERVAL
    {
        return;
    }
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = std::fs::copy(path, dir.join(format!("{}-{stamp}.json", backup_base(path))));
    for old in backups(path).into_iter().skip(BACKUPS_KEPT) {
        let _ = std::fs::remove_file(old);
    }
}

/// Replaces `path` with its newest backup that decodes as `T`; the broken file is kept next to it.
pub fn restore_latest<T: DeserializeOwned>(path: &Path) -> bool {
    for candidate in backups(path) {
        let Ok(data) = std::fs::read(&candidate) else { continue };
        if serde_json::from_slice::<T>(&data).is_err() {
            continue;
        }
        let broken = path.with_file_name(format!("{}.defekt", path.file_name().unwrap_or_default().to_string_lossy()));
        let _ = std::fs::remove_file(&broken);
        let _ = std::fs::rename(path, &broken);
        return std::fs::write(path, data).is_ok();
    }
    false
}

/// Starter config for an empty folder: one semester around today, one module per top-level folder.
pub fn starter_config(root: &Path) -> StudyConfig {
    let monday = day::monday(Local::now().date_naive());
    let end = monday + Days::new(7 * 15 - 1);
    let semester = Semester { id: "s1".into(), name: semester_name(monday), start: day::format(monday), end: day::format(end), ..Default::default() };
    let mut folders: Vec<String> = std::fs::read_dir(root)
        .map(|d| d.filter_map(Result::ok).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    folders.retain(|n| !n.starts_with('.') && !n.starts_with('_'));
    folders.sort();
    let mut taken = Vec::new();
    let modules = folders.iter().enumerate().map(|(i, name)| module_for_folder(name, &semester.id, i, &mut taken)).collect();
    let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    StudyConfig::new(name, String::new(), vec![semester], modules)
}

/// A module for a folder at any depth, named after the folder's last part.
pub fn module_for_folder(folder: &str, semester: &str, position: usize, taken: &mut Vec<String>) -> Module {
    let name = folder.rsplit('/').next().unwrap_or(folder).to_string();
    let code = abbreviation_for(&name, taken);
    taken.push(code.clone());
    Module { id: folder.to_lowercase(), code, name, semester: semester.to_string(), folder: folder.to_string(), color: PALETTE[position % PALETTE.len()].into(), ..Default::default() }
}

fn abbreviation_for(name: &str, taken: &[String]) -> String {
    super::model::unique_abbreviation(name, taken)
}

/// "WS 2026/27" for dates from September to February, otherwise "SS 2027".
pub fn semester_name(date: chrono::NaiveDate) -> String {
    let (y, m) = (date.year(), date.month());
    if m >= 9 {
        format!("WS {y}/{:02}", (y + 1) % 100)
    } else if m <= 2 {
        format!("WS {}/{:02}", y - 1, y % 100)
    } else {
        format!("SS {y}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_semesters() {
        assert_eq!(semester_name(chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()), "WS 2026/27");
        assert_eq!(semester_name(chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()), "WS 2026/27");
        assert_eq!(semester_name(chrono::NaiveDate::from_ymd_opt(2027, 4, 1).unwrap()), "SS 2027");
    }
}
