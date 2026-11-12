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
    assignments.iter().filter(|a| a.tracks_files()).flat_map(|a| std::iter::once((a.folder.clone(), a.recursive)).chain(a.legacy.clone())).collect()
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

/// Linked files that are not on disk; touches the file system, so run it off the main thread.
pub fn matches(name: &str, glob: &str) -> bool {
    let opts = MatchOptions { case_sensitive: false, ..MatchOptions::new() };
    Pattern::new(if glob.is_empty() { "*" } else { glob }).is_ok_and(|p| p.matches_with(name, opts))
}

fn window(a: &Assignment, date: DateTime<Local>) -> bool {
    let due_day = day::local_date(a.due);
    let from = a.window_start.map(day::local_date).unwrap_or(due_day - Days::new(7));
    let d = day::local_date(date);
    d > from && d <= due_day
}

fn names_it(a: &Assignment, name: &str, naming: Option<&Naming>) -> bool {
    let folded = text::fold(name);
    a.keywords.iter().chain([&a.short, &a.title]).map(|w| text::fold(w)).filter(|w| w.chars().count() >= 2).any(|w| folded.contains(&w)) || naming.is_some_and(|n| n.is_valid(name))
}

/// What older versions counted for a deadline by date in a shared folder; adopted once as links so nothing changes status.
pub fn adoptable(assignments: &[Assignment], listings: &Listings, config: &StudyConfig) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for a in assignments.iter().filter(|a| a.tracks_files() && !a.owns_folder()) {
        let Some(legacy) = &a.legacy else { continue };
        let files = listings.get(legacy).map(Vec::as_slice).unwrap_or_default();
        for f in files.iter().filter(|f| matches(file_name(&f.path), &a.file_pattern) && window(a, f.date) && !FileRole::is_brief(file_name(&f.path), &config.author)) {
            out.push((a.id.clone(), f.path.clone()));
        }
    }
    out
}

/// Derives every assignment's status from its linked files, files in a folder of its own, and the user's overrides; pure and fast.
pub fn evaluate(assignments: &[Assignment], listings: &Listings, config: &StudyConfig, state: &StudyState, now: DateTime<Local>, root: Option<&Path>, gone: &HashSet<PathBuf>) -> HashMap<String, Evaluation> {
    let namings: HashMap<&str, Naming> = config.modules.iter().map(|m| (m.id.as_str(), Naming::new(m, config.semester(&m.semester), config))).collect();
    let claimed: HashSet<PathBuf> = root.map(|r| state.linked_files.values().flatten().map(|rel| r.join(rel)).collect()).unwrap_or_default();
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
        let own = listings.get(&(a.folder.clone(), a.recursive)).map(Vec::as_slice).unwrap_or_default();
        let mut matched: Vec<PathBuf> = if a.owns_folder() { own.iter().filter(|f| matches(file_name(&f.path), &a.file_pattern)).map(|f| f.path.clone()).collect() } else { Vec::new() };
        let mut missing = Vec::new();
        if let (Some(linked), Some(root)) = (state.linked_files.get(&a.id), root) {
            for p in linked.iter().map(|rel| root.join(rel)) {
                if gone.contains(&p) {
                    missing.push(p);
                } else if !matched.contains(&p) {
                    matched.push(p);
                }
            }
        }
        let briefs: Vec<PathBuf> = matched.iter().filter(|p| FileRole::is_brief(file_name(p), &config.author)).cloned().collect();
        matched.retain(|p| !briefs.contains(p));
        let naming = namings.get(a.module_id.as_str());
        let mut e = if let Some(o) = override_ {
            let on_time = o.date <= a.due || matched.is_empty() || matched.iter().any(|p| file_date(p) <= a.due);
            Evaluation { status: o.status, files: matched, is_override: true, completed_on_time: o.status.is_closed().then_some(on_time), briefs, missing, ..Evaluation::empty() }
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
                (Status::Ready, None)
            };
            Evaluation { status, files: matched, is_override: false, completed_on_time: on_time, briefs, missing, ..Evaluation::empty() }
        };
        if !e.status.is_closed() && !a.owns_folder() {
            e.suggested = suggestions(a, own, listings, &claimed, naming, config);
        }
        result.insert(a.id.clone(), e);
    }
    result
}

/// Unlinked files from around the deadline: anything in its folder, and in the folder older versions searched only what names it. Best first.
fn suggestions(a: &Assignment, own: &[FileInfo], listings: &Listings, claimed: &HashSet<PathBuf>, naming: Option<&Naming>, config: &StudyConfig) -> Vec<PathBuf> {
    let legacy = a.legacy.as_ref().filter(|l| **l != (a.folder.clone(), a.recursive)).and_then(|l| listings.get(l)).map(Vec::as_slice).unwrap_or_default();
    let fresh = |f: &&FileInfo| window(a, f.date) && !claimed.contains(&f.path);
    let inside: HashSet<&Path> = if legacy.is_empty() { HashSet::new() } else { own.iter().filter(fresh).map(|f| f.path.as_path()).collect() };
    let mut found: Vec<(bool, DateTime<Local>, PathBuf)> = own
        .iter()
        .filter(fresh)
        .map(|f| (f, true))
        .chain(legacy.iter().filter(fresh).filter(|f| !inside.contains(f.path.as_path())).map(|f| (f, false)))
        .filter(|(f, _)| matches(file_name(&f.path), &a.file_pattern) && !FileRole::is_brief(file_name(&f.path), &config.author))
        .filter_map(|(f, inside)| {
            let named = names_it(a, file_name(&f.path), naming);
            (inside || named).then(|| (named, f.date, f.path.clone()))
        })
        .collect();
    found.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.cmp(&x.1)));
    found.into_iter().take(3).map(|(_, _, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_ignore_case() {
        assert!(matches("Abgabe.PDF", "*.pdf"));
        assert!(matches("x", ""));
    }

    #[test]
    fn closing_without_files_after_the_due_date_stays_on_time() {
        use chrono::{Duration, TimeZone, Utc};
        use super::super::model::Override;
        let now = Local.with_ymd_and_hms(2026, 11, 12, 12, 0, 0).unwrap();
        let a = Assignment {
            id: "a".into(),
            module_id: "m".into(),
            semester_id: "s".into(),
            rule_id: None,
            title: "Blatt".into(),
            short: "B".into(),
            kind: Kind::Submission,
            due: now - Duration::days(3),
            week: None,
            iso_week: None,
            optional: false,
            upload: false,
            folder: PathBuf::new(),
            per_week_folder: false,
            file_pattern: String::new(),
            window_start: None,
            recursive: false,
            keywords: Vec::new(),
            is_moved: false,
            legacy: None,
        };
        let mut state = StudyState::default();
        state.overrides.insert("a".into(), Override { status: Status::Done, date: Utc::now() });
        let found = evaluate(std::slice::from_ref(&a), &Listings::new(), &StudyConfig::default(), &state, now, None, &HashSet::new());
        assert_eq!(found["a"].completed_on_time, Some(true));
    }
}
