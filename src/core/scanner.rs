use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Days, Local, NaiveTime};
use glob::{MatchOptions, Pattern};
use walkdir::WalkDir;

use super::filing::FileRole;
use super::model::{Assignment, Evaluation, Kind, Status, StudyConfig, StudyState};
use super::naming::Naming;
use super::{day, text};

const SKIP_DIRS: [&str; 5] = [".git", "node_modules", ".build", "DerivedData", "build"];

/// A file found in an assignment folder.
#[derive(Debug, Clone, PartialEq)]
pub struct FileInfo {
    pub path: PathBuf,
    pub date: DateTime<Local>,
}

/// A folder to list, recursively or not.
pub type FolderRequest = (PathBuf, bool);
pub type Listings = HashMap<FolderRequest, Vec<FileInfo>>;

pub fn file_name(p: &Path) -> &str {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

/// Maps `.Name.pdf.icloud` placeholders to `Name.pdf`; hides other hidden files.
fn visible(path: &Path) -> Option<PathBuf> {
    let name = file_name(path);
    if let Some(real) = name.strip_prefix('.').and_then(|n| n.strip_suffix(".icloud")) {
        return Some(path.with_file_name(real));
    }
    (!name.starts_with('.')).then(|| path.to_path_buf())
}

/// Date in the file name, else its modification time.
pub fn file_date(path: &Path) -> DateTime<Local> {
    if let Some(d) = day::date_in_name(file_name(path)).and_then(|d| day::at(d, NaiveTime::MIN)) {
        return d;
    }
    std::fs::metadata(path).and_then(|m| m.modified()).map(DateTime::<Local>::from).unwrap_or(DateTime::<Local>::MIN_UTC.into())
}

fn list(folder: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = WalkDir::new(folder)
        .min_depth(1)
        .max_depth(if recursive { 5 } else { 1 })
        .into_iter()
        .filter_entry(|e| !e.file_type().is_dir() || !(file_name(e.path()).starts_with('.') || SKIP_DIRS.contains(&file_name(e.path()))))
        .filter_map(Result::ok)
        .filter(|e| !e.file_type().is_dir())
        .filter_map(|e| visible(e.path()))
        .collect();
    out.sort_by(|a, b| text::natural_cmp(file_name(a), file_name(b)));
    out
}

/// Top-level files of a folder, iCloud placeholders included under their real name.
pub fn files(folder: &Path, pattern: &str) -> Vec<PathBuf> {
    list(folder, false).into_iter().filter(|p| matches(file_name(p), pattern)).collect()
}

pub fn requests(assignments: &[Assignment]) -> HashSet<FolderRequest> {
    assignments.iter().filter(|a| a.tracks_files()).map(|a| (a.folder.clone(), a.recursive)).collect()
}

/// Lists every distinct folder once; touches the file system, so run it off the main thread.
pub fn listings(requests: &HashSet<FolderRequest>) -> Listings {
    requests
        .iter()
        .map(|k| {
            let files = list(&k.0, k.1).into_iter().map(|path| FileInfo { date: file_date(&path), path }).collect();
            (k.clone(), files)
        })
        .collect()
}

pub fn matches(name: &str, glob: &str) -> bool {
    let opts = MatchOptions { case_sensitive: false, ..MatchOptions::new() };
    Pattern::new(if glob.is_empty() { "*" } else { glob }).is_ok_and(|p| p.matches_with(name, opts))
}

/// Derives every assignment's status from cached folder listings and the user's overrides; pure and fast.
pub fn evaluate(assignments: &[Assignment], listings: &Listings, config: &StudyConfig, state: &StudyState, now: DateTime<Local>, root: Option<&Path>) -> HashMap<String, Evaluation> {
    let namings: HashMap<&str, Naming> = config.modules.iter().map(|m| (m.id.as_str(), Naming::new(m, config.semester(&m.semester), config))).collect();
    let mut result = HashMap::with_capacity(assignments.len());
    for a in assignments {
        let past = now > a.due;
        let override_ = state.overrides.get(&a.id);
        if !a.tracks_files() {
            let e = match override_ {
                Some(o) => Evaluation { status: o.status, is_override: true, completed_on_time: Some(o.date <= a.due), ..Evaluation::empty() },
                None if a.kind == Kind::Exam => Evaluation { status: if past { Status::Done } else { Status::Open }, completed_on_time: Some(true), ..Evaluation::empty() },
                None => Evaluation { status: if past { Status::Overdue } else { Status::Open }, ..Evaluation::empty() },
            };
            result.insert(a.id.clone(), e);
            continue;
        }
        let all = listings.get(&(a.folder.clone(), a.recursive)).map(Vec::as_slice).unwrap_or_default();
        let candidates = all.iter().filter(|f| matches(file_name(&f.path), &a.file_pattern));
        let mut matched: Vec<PathBuf> = if a.per_week_folder {
            candidates.map(|f| f.path.clone()).collect()
        } else {
            let due_day = day::local_date(a.due);
            let from = a.window_start.map(day::local_date).unwrap_or(due_day - Days::new(7));
            candidates
                .filter(|f| {
                    let d = day::local_date(f.date);
                    d > from && d <= due_day
                })
                .map(|f| f.path.clone())
                .collect()
        };
        let briefs: Vec<PathBuf> = matched.iter().filter(|p| FileRole::is_brief(file_name(p), &config.author)).cloned().collect();
        matched.retain(|p| !briefs.contains(p));
        if let (Some(linked), Some(root)) = (state.linked_files.get(&a.id), root) {
            for rel in linked {
                let p = root.join(rel);
                if !matched.contains(&p) && p.exists() {
                    matched.push(p);
                }
            }
        }
        let naming = namings.get(a.module_id.as_str());
        let misnamed: Vec<PathBuf> = if a.upload { matched.iter().filter(|p| !naming.is_none_or(|n| n.is_valid(file_name(p)))).cloned().collect() } else { Vec::new() };
        let briefs: Vec<PathBuf> = briefs.into_iter().filter(|b| !matched.contains(b)).collect();
        let e = if let Some(o) = override_ {
            let on_time = o.date <= a.due || matched.is_empty() || matched.iter().any(|p| file_date(p) <= a.due);
            Evaluation { status: o.status, files: matched, misnamed, is_override: true, completed_on_time: o.status.is_closed().then_some(on_time), briefs }
        } else {
            let (status, on_time) = if matched.is_empty() {
                (
                    match (past, a.optional) {
                        (true, true) => Status::Skipped,
                        (true, false) => Status::Overdue,
                        _ => Status::Open,
                    },
                    None,
                )
            } else if !a.file_pattern.contains('.') && matched.iter().all(|p| FileRole::is_draft(p)) {
                (Status::InProgress, None)
            } else if !a.upload {
                (Status::Done, Some(matched.iter().any(|p| file_date(p) <= a.due)))
            } else {
                (if misnamed.len() < matched.len() { Status::Ready } else { Status::InProgress }, None)
            };
            Evaluation { status, files: matched, misnamed, is_override: false, completed_on_time: on_time, briefs }
        };
        result.insert(a.id.clone(), e);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_ignore_case() {
        assert!(matches("Abgabe.PDF", "*.pdf"));
        assert!(matches("x", ""));
    }
}
