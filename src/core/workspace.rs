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
    save_bytes(value, path).map(|_| ())
}

/// Like `save`, and returns the bytes written.
pub fn save_bytes<T: Serialize>(value: &T, path: &Path) -> Result<Vec<u8>> {
    backup(path);
    let json = to_swift_json(value)?;
    let tmp = path.with_file_name(format!(".{}.tmp", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, path)?;
    Ok(json.into())
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

/// Starter config for a folder without one: the current term as German universities usually hold it,
/// and the folders that look like modules. A vault starts without modules and never gets folders made for it.
pub fn starter_config(root: &Path) -> StudyConfig {
    let semester = semester_guess(Local::now().date_naive());
    let is_vault = root.join(".obsidian").is_dir();
    let found = super::folders::all_folders(root);
    let chosen = super::folders::suggested_modules(&found, is_vault);
    let mut taken = Vec::new();
    let modules = chosen.iter().enumerate().map(|(i, folder)| module_for_folder(folder, &semester.id, i, &mut taken)).collect();
    let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut config = StudyConfig::new(name, String::new(), vec![semester], modules);
    config.create_folders = !is_vault;
    config
}

fn week_of(year: i32, month: u32, day_of_month: u32) -> chrono::NaiveDate {
    day::monday(chrono::NaiveDate::from_ymd_opt(year, month, day_of_month).unwrap_or_default())
}

/// The lecture period most German universities keep around `date`: winter from the week of 15 October to the
/// week of 7 February with the days around Christmas off, summer from the week of 15 April to the week of 18 July.
/// A guess the user confirms or corrects; `is_guessed` tells it apart from entered dates.
pub fn semester_guess(date: chrono::NaiveDate) -> Semester {
    let (y, m) = (date.year(), date.month());
    let name = semester_name(date);
    if (3..=8).contains(&m) {
        return Semester { id: "s1".into(), name, start: day::format(week_of(y, 4, 15)), end: day::format(week_of(y, 7, 18) + Days::new(4)), ..Default::default() };
    }
    let y = if m >= 9 { y } else { y - 1 };
    let christmas = super::model::Break { name: "Weihnachten".into(), start: day::format(chrono::NaiveDate::from_ymd_opt(y, 12, 24).unwrap_or_default()), end: day::format(chrono::NaiveDate::from_ymd_opt(y + 1, 1, 6).unwrap_or_default()) };
    Semester { id: "s1".into(), name, start: day::format(week_of(y, 10, 15)), end: day::format(week_of(y + 1, 2, 7) + Days::new(4)), breaks: vec![christmas], ..Default::default() }
}

/// Whether a semester still carries the dates `semester_guess` made up, so nobody has confirmed them.
pub fn is_guessed(semester: &Semester) -> bool {
    let Some(start) = semester.start_date() else { return false };
    let guess = semester_guess(start);
    guess.start == semester.start && guess.end == semester.end
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

    #[test]
    fn guesses_the_usual_german_terms_and_recognises_its_own_guess() {
        let d = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let winter = semester_guess(d(2026, 11, 10));
        assert_eq!((winter.name.as_str(), winter.start.as_str(), winter.end.as_str()), ("WS 2026/27", "2026-10-12", "2027-02-05"));
        assert_eq!(winter.breaks.len(), 1);
        assert_eq!(semester_guess(d(2027, 1, 20)), winter);
        let summer = semester_guess(d(2027, 5, 3));
        assert_eq!((summer.name.as_str(), summer.start.as_str(), summer.end.as_str()), ("SS 2027", "2027-04-12", "2027-07-16"));
        assert!(summer.breaks.is_empty());
        assert!(is_guessed(&winter) && is_guessed(&summer));
        let mut edited = winter.clone();
        edited.end = "2027-02-12".into();
        assert!(!is_guessed(&edited));
        assert!(!is_guessed(&Semester::default()), "a semester without dates is not a guess");
    }
}
