use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Local, NaiveDate};
use regex::Regex;

use super::filing::FileRole;
use super::model::{LectureWeek, Module, StudyConfig};
use super::naming::{Naming, extension, stem};
use super::search_index::IndexedFile;
use super::text::fold;
use super::{day, markdown, schedule};

/// A workspace file with the context needed to browse it: module, lecture week and place inside the module.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogFile {
    pub file: IndexedFile,
    pub module_id: Option<String>,
    /// Lecture week, from a `Wxx` folder or else from the file's date.
    pub week: Option<u32>,
    /// Folders between the module (or week) folder and the file.
    pub place: Vec<String>,
    pub date: NaiveDate,
    pub is_brief: bool,
    /// Inside a git repository or similar project folder; hidden while browsing, found by search.
    pub in_project: bool,
    pub haystack: String,
    pub folded_name: String,
    /// Readable title: a note's first heading or the title part of a convention name.
    pub title: Option<String>,
}

impl CatalogFile {
    /// Title when known, else the file name without extension.
    pub fn display_name(&self) -> String {
        self.title.clone().unwrap_or_else(|| stem(&self.file.name).to_string())
    }

    pub fn path(&self) -> &Path {
        &self.file.path
    }

    pub fn name(&self) -> &str {
        &self.file.name
    }
}

/// File kinds offered as a filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    Documents,
    Notes,
    Pdf,
    Office,
    Image,
    All,
}

const OFFICE: [&str; 14] = ["doc", "docx", "pages", "key", "ppt", "pptx", "numbers", "xls", "xlsx", "odt", "ods", "odp", "rtf", "csv"];
const IMAGES: [&str; 8] = ["png", "jpg", "jpeg", "heic", "gif", "webp", "tiff", "svg"];
const DOCS: [&str; 16] = ["pdf", "doc", "docx", "pages", "key", "ppt", "pptx", "numbers", "xls", "xlsx", "md", "txt", "rtf", "fig", "sketch", "zip"];

impl FileKind {
    pub const ALL: [FileKind; 6] = [Self::Documents, Self::Notes, Self::Pdf, Self::Office, Self::Image, Self::All];

    pub fn key(self) -> &'static str {
        match self {
            Self::Documents => "documents",
            Self::Notes => "notes",
            Self::Pdf => "pdf",
            Self::Office => "office",
            Self::Image => "image",
            Self::All => "all",
        }
    }

    pub fn from_key(k: &str) -> Self {
        Self::ALL.into_iter().find(|f| f.key() == k).unwrap_or(Self::Documents)
    }

    pub fn matches(self, name: &str) -> bool {
        let ext = extension(name).to_lowercase();
        let e = ext.as_str();
        match self {
            Self::Documents => DOCS.contains(&e) || IMAGES.contains(&e) || e == "markdown",
            Self::Notes => ["md", "markdown", "mdown", "txt"].contains(&e),
            Self::Pdf => e == "pdf",
            Self::Office => OFFICE.contains(&e),
            Self::Image => IMAGES.contains(&e),
            Self::All => true,
        }
    }
}

type Stamp = (std::time::SystemTime, u64);
type Headings = HashMap<PathBuf, (Stamp, Option<String>)>;

static HEADINGS: LazyLock<Mutex<HashMap<PathBuf, Headings>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Builds the catalog from indexed files; run off the main thread. Headings are read again only for files changed since the last build.
pub fn build(files: &[IndexedFile], root: &Path, config: &StudyConfig) -> Vec<CatalogFile> {
    let mut modules: Vec<&Module> = config.modules.iter().collect();
    modules.sort_by_key(|m| std::cmp::Reverse(m.folder.len()));
    let weeks: HashMap<&str, Vec<LectureWeek>> = config.semesters.iter().map(|s| (s.id.as_str(), schedule::lecture_weeks(s))).collect();
    let namings: HashMap<&str, Naming> = config.modules.iter().map(|m| (m.id.as_str(), Naming::new(m, config.semester(&m.semester), config))).collect();
    let mut project_dirs: HashMap<PathBuf, bool> = HashMap::new();
    let mut is_project = |components: &[&str], base: &Path| -> bool {
        let mut path = base.to_path_buf();
        for c in components.iter().take(components.len().saturating_sub(1)) {
            path.push(c);
            let hit = *project_dirs.entry(path.clone()).or_insert_with(|| path.join(".git").exists() || path.join("package.json").exists());
            if hit {
                return true;
            }
        }
        false
    };
    let known = HEADINGS.lock().map(|mut all| all.remove(root)).ok().flatten().unwrap_or_default();
    let mut kept: Headings = HashMap::with_capacity(known.len());
    let catalog = files
        .iter()
        .filter_map(|f| {
            let rel = super::text::slash_path(f.path.strip_prefix(root).ok()?);
            let module = modules.iter().find(|m| rel.starts_with(&format!("{}/", m.folder))).copied();
            let inner = module.map_or(rel.as_str(), |m| &rel[m.folder.len() + 1..]);
            let all: Vec<&str> = inner.split('/').collect();
            let mut components: Vec<&str> = all.clone();
            let date = day::date_in_name(&f.name).unwrap_or_else(|| day::local_date(f.modified));
            let mut week = None;
            if let Some(i) = components[..components.len() - 1].iter().position(|c| week_index(c).is_some()) {
                week = week_index(components[i]);
                components.remove(i);
            } else if let Some(list) = module.and_then(|m| weeks.get(m.semester.as_str()))
                && list.first().is_some_and(|w| date >= w.monday)
            {
                week = list.iter().rev().find(|w| w.monday <= date).map(|w| w.index);
            }
            let place: Vec<String> = components[..components.len() - 1].iter().map(|s| s.to_string()).collect();
            let base = module.map_or_else(|| root.to_path_buf(), |m| root.join(&m.folder));
            let stamp = std::fs::metadata(&f.path).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
            let heading = match known.get(&f.path).filter(|(seen, _)| Some(*seen) == stamp) {
                Some((_, heading)) => heading.clone(),
                None => note_heading(&f.path),
            };
            if let Some(stamp) = stamp {
                kept.insert(f.path.clone(), (stamp, heading.clone()));
            }
            let title = heading.or_else(|| module.and_then(|m| namings.get(m.id.as_str())?.parse(&f.name)).map(|p| p.title.replace('-', " ")));
            let haystack = fold(&[module.map_or("", |m| m.code.as_str())].into_iter().chain(place.iter().map(String::as_str)).chain([f.name.as_str(), title.as_deref().unwrap_or("")]).collect::<Vec<_>>().join(" "));
            Some(CatalogFile {
                file: f.clone(),
                module_id: module.map(|m| m.id.clone()),
                week,
                place,
                date,
                is_brief: FileRole::is_brief(&f.name, &config.author),
                in_project: is_project(&all, &base),
                haystack,
                folded_name: fold(&format!("{} {}", f.name, title.as_deref().unwrap_or(""))),
                title,
            })
        })
        .collect();
    if let Ok(mut all) = HEADINGS.lock() {
        all.insert(root.to_path_buf(), kept);
    }
    catalog
}

/// First Markdown heading, read from the start of the file.
fn note_heading(path: &Path) -> Option<String> {
    if !markdown::is_markdown(path) {
        return None;
    }
    let mut buf = vec![0; 2048];
    let n = std::fs::File::open(path).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    let head = String::from_utf8_lossy(&buf);
    for line in head.split('\n').take(25) {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("title:") {
            return Some(rest.trim_matches(|c: char| c == ' ' || c == '"' || c == '\'').to_string());
        }
        if let Some(rest) = t.strip_prefix("# ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Letters of `term` appearing in order in `text` (typo-tolerant fallback); tighter matches score higher.
pub fn fuzzy(term: &str, text: &str) -> Option<f64> {
    let n = term.chars().count();
    if n < 3 {
        return None;
    }
    let mut it = text.chars();
    let (mut gaps, mut matched) = (0, 0);
    'outer: for c in term.chars() {
        for t in it.by_ref() {
            if t == c {
                matched += 1;
                continue 'outer;
            }
            if matched > 0 {
                gaps += 1;
            }
        }
        return None;
    }
    Some((1.5 - gaps as f64 / (n * 3) as f64).max(0.2))
}

/// Fuzzy relevance when no exact match was found.
pub fn fuzzy_score(f: &CatalogFile, terms: &[String], current_week: Option<u32>) -> Option<f64> {
    let mut score = 0.;
    for t in terms {
        score += fuzzy(t, &f.folded_name).or_else(|| fuzzy(t, &f.haystack).map(|s| s / 2.))?;
    }
    Some(score + recency(f.week, current_week) / 2.)
}

/// `W03`, `W3_KW44`, `Woche 3` → 3.
pub fn week_index(name: &str) -> Option<u32> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:w|woche[ _-]?)(\d{1,2})(?:$|[_\- ])").unwrap());
    RE.captures(name).and_then(|c| c[1].parse().ok())
}

/// A parsed search: free terms plus filters typed into the field ("w3", "kw44", "woche 3", a module code).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    pub terms: Vec<String>,
    pub week: Option<u32>,
    pub module_id: Option<String>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.week.is_none() && self.module_id.is_none()
    }

    pub fn text(&self) -> String {
        self.terms.join(" ")
    }

    pub fn parse(raw: &str, modules: &[Module], weeks: &[LectureWeek]) -> Self {
        static WOCHE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bwoche\s*(\d{1,2})\b").unwrap());
        static W: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^w(\d{1,2})$").unwrap());
        static KW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^kw(\d{1,2})$").unwrap());
        let mut q = Query::default();
        let mut s = fold(raw);
        if let Some(c) = WOCHE.captures(&s.clone()) {
            q.week = c[1].parse().ok();
            s = s.replacen(&c[0], "", 1);
        }
        for token in s.split([' ', ',']).filter(|t| !t.is_empty()) {
            if q.week.is_none()
                && let Some(c) = W.captures(token)
            {
                q.week = c[1].parse().ok();
            } else if q.week.is_none()
                && let Some(kw) = KW.captures(token).and_then(|c| c[1].parse::<u32>().ok())
            {
                q.week = weeks.iter().find(|w| w.iso_week == kw).map(|w| w.index);
            } else if let Some(m) = modules.iter().find(|m| q.module_id.is_none() && token.chars().count() >= 2 && fold(&m.code) == token) {
                q.module_id = Some(m.id.clone());
            } else {
                q.terms.push(token.to_string());
            }
        }
        q
    }
}

/// Relevance of a name/path match; `None` when a term is missing. Recent weeks rank higher.
pub fn score(f: &CatalogFile, terms: &[String], current_week: Option<u32>) -> Option<f64> {
    let mut score = 0.;
    for t in terms {
        if f.folded_name.contains(t.as_str()) {
            score += if f.folded_name.starts_with(t.as_str()) || f.folded_name.contains(&format!("_{t}")) || f.folded_name.contains(&format!("-{t}")) { 4. } else { 3. };
        } else if f.haystack.contains(t.as_str()) {
            score += 1.;
        } else {
            return None;
        }
    }
    Some(score + recency(f.week, current_week))
}

pub fn recency(week: Option<u32>, current: Option<u32>) -> f64 {
    let (Some(w), Some(c)) = (week, current) else { return 0. };
    match c as i64 - w as i64 {
        ..=0 => 4.,
        1 => 3.,
        2 => 2.,
        3..=4 => 1.,
        _ => 0.,
    }
}

pub fn modified_date(t: DateTime<Local>) -> NaiveDate {
    day::local_date(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_week_folders() {
        assert_eq!(week_index("W03_KW44"), Some(3));
        assert_eq!(week_index("Woche 12"), Some(12));
        assert_eq!(week_index("Wiki"), None);
    }

    #[test]
    fn parses_queries() {
        let m = Module { id: "p1".into(), code: "P1".into(), ..Default::default() };
        let q = Query::parse("p1 radar w3", &[m], &[]);
        assert_eq!(q.module_id.as_deref(), Some("p1"));
        assert_eq!(q.week, Some(3));
        assert_eq!(q.terms, ["radar"]);
    }

    #[test]
    fn headings_are_read_again_only_for_changed_files() {
        let dir = std::env::temp_dir().join(format!("schrumbi-headings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.md"), dir.join("b.md"));
        std::fs::write(&a, "# Alt\n").unwrap();
        std::fs::write(&b, "# Bleibt\n").unwrap();
        let then = Local::now();
        let files = |at: DateTime<Local>| {
            vec![
                IndexedFile { path: a.clone(), name: "a.md".into(), modified: at },
                IndexedFile { path: b.clone(), name: "b.md".into(), modified: then },
            ]
        };
        let titles = |c: Vec<CatalogFile>| c.into_iter().map(|f| f.title).collect::<Vec<_>>();
        let config = StudyConfig::default();
        assert_eq!(titles(build(&files(then), &dir, &config)), [Some("Alt".to_string()), Some("Bleibt".to_string())]);
        let kept = std::fs::metadata(&b).unwrap().modified().unwrap();
        std::fs::write(&a, "# Neu\n").unwrap();
        std::fs::write(&b, "# Anders\n").unwrap();
        std::fs::File::options().write(true).open(&b).unwrap().set_modified(kept).unwrap();
        let later = then + chrono::Duration::seconds(5);
        assert_eq!(titles(build(&files(later), &dir, &config)), [Some("Neu".to_string()), Some("Bleibt".to_string())]);
    }

    #[test]
    fn headings_follow_the_file_even_when_the_index_time_stood_still() {
        let dir = std::env::temp_dir().join(format!("schrumbi-headings-fresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.md");
        std::fs::write(&a, "# Alt\n").unwrap();
        let then = Local::now();
        let files = vec![IndexedFile { path: a.clone(), name: "a.md".into(), modified: then }];
        let config = StudyConfig::default();
        assert_eq!(build(&files, &dir, &config)[0].title.as_deref(), Some("Alt"));
        std::fs::write(&a, "# Neuer\n").unwrap();
        assert_eq!(build(&files, &dir, &config)[0].title.as_deref(), Some("Neuer"));
        let before = std::fs::metadata(&a).unwrap().modified().unwrap();
        std::fs::write(&a, "# Neuem\n").unwrap();
        std::fs::File::options().write(true).open(&a).unwrap().set_modified(before + std::time::Duration::from_millis(50)).unwrap();
        assert_eq!(build(&files, &dir, &config)[0].title.as_deref(), Some("Neuem"));
    }

    #[test]
    fn fuzzy_matches_in_order() {
        assert!(fuzzy("rdr", "radar").is_some());
        assert!(fuzzy("xyz", "radar").is_none());
    }
}
