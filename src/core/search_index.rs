use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Local, TimeZone};
use rayon::prelude::*;
use regex::Regex;
use rusqlite::{Connection, params};
use walkdir::WalkDir;

use super::naming::extension;
use super::workspace::CONFIG_NAME;

#[derive(Debug, Clone, PartialEq)]
pub struct IndexedFile {
    pub path: PathBuf,
    pub name: String,
    pub modified: DateTime<Local>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub file: IndexedFile,
    pub snippet: Option<String>,
}

const TEXT_EXTENSIONS: [&str; 4] = ["txt", "md", "html", "csv"];
const MAX_TEXT: usize = 200_000;
const MAX_BYTES: u64 = 60 * 1024 * 1024;
/// `fts` rows share their rowid with `files`, so updates need no scan of the full-text table.
const SCHEMA: i64 = 2;

/// Incremental full-text index in SQLite FTS5, stored in the app's data folder.
pub struct SearchIndex {
    db: Mutex<Option<Connection>>,
}

impl SearchIndex {
    pub fn open() -> Self {
        let db = crate::core::prefs::data_dir().and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            Self::connect(&dir.join("search.sqlite"))
        });
        Self { db: Mutex::new(db) }
    }

    /// Opens the database; an index from an older schema is rebuilt, it only holds derived data.
    fn connect(path: &Path) -> Option<Connection> {
        let c = Connection::open(path).ok()?;
        let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).ok()?;
        if version < SCHEMA {
            c.execute_batch("DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS fts;").ok()?;
        }
        c.execute_batch(&format!(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS files(id INTEGER PRIMARY KEY, path TEXT UNIQUE, name TEXT, modified REAL);
             CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(name, body, tokenize='unicode61 remove_diacritics 2');
             PRAGMA user_version={SCHEMA};"
        ))
        .ok()?;
        Some(c)
    }

    fn with<T: Default>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> T {
        self.db.lock().unwrap().as_ref().and_then(|c| f(c).ok()).unwrap_or_default()
    }

    /// Walks `root`, re-extracting only changed files and dropping deleted ones. Returns the file count.
    pub fn update(&self, root: &Path, exclude: &[String]) -> usize {
        let known: HashMap<String, f64> = self.with(|c| {
            let mut s = c.prepare("SELECT path, modified FROM files")?;
            s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?.collect()
        });
        let skip: HashSet<&str> = exclude.iter().map(String::as_str).collect();
        let mut seen: HashSet<String> = HashSet::new();
        let mut pending: Vec<(PathBuf, String, String, f64, bool)> = Vec::new();
        let walker = WalkDir::new(root).min_depth(1).into_iter().filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') && !(e.file_type().is_dir() && (skip.contains(name.as_ref()) || is_package(e.path())))
        });
        for entry in walker.filter_map(Result::ok).filter(|e| !e.file_type().is_dir()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == CONFIG_NAME {
                continue;
            }
            let path = entry.path().to_string_lossy().to_string();
            let meta = entry.metadata().ok();
            let modified = meta.as_ref().and_then(|m| m.modified().ok()).map_or(0., |t| t.duration_since(std::time::UNIX_EPOCH).map_or(0., |d| d.as_secs_f64()));
            seen.insert(path.clone());
            if known.get(&path).is_some_and(|old| (old - modified).abs() < 0.5) {
                continue;
            }
            let small = meta.is_some_and(|m| m.len() <= MAX_BYTES);
            pending.push((entry.path().to_path_buf(), path, name, modified, small));
        }
        let changed: Vec<(String, String, f64, String)> = pending
            .into_par_iter()
            .map(|(file, path, name, modified, small)| {
                let body = if small { extract(&file) } else { String::new() };
                (path, name, modified, body)
            })
            .collect();
        let removed: Vec<&String> = known.keys().filter(|p| !seen.contains(*p)).collect();
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            let remove = |path: &str| -> rusqlite::Result<()> {
                tx.execute("DELETE FROM fts WHERE rowid = (SELECT id FROM files WHERE path = ?1)", [path])?;
                tx.execute("DELETE FROM files WHERE path = ?1", [path])?;
                Ok(())
            };
            for p in &removed {
                remove(p)?;
            }
            for (path, name, modified, body) in &changed {
                if known.contains_key(path) {
                    remove(path)?;
                }
                tx.execute("INSERT INTO files(path, name, modified) VALUES(?1, ?2, ?3)", params![path, name, modified])?;
                tx.execute("INSERT INTO fts(rowid, name, body) VALUES(last_insert_rowid(), ?1, ?2)", params![name, body])?;
            }
            tx.commit()
        });
        seen.len()
    }

    /// Clears the index, e.g. when switching workspaces.
    pub fn reset(&self) {
        self.with(|c| c.execute_batch("DELETE FROM files; DELETE FROM fts;"));
    }

    pub fn count(&self) -> usize {
        self.with(|c| c.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0)).map(|n| n as usize))
    }

    /// Every indexed file, newest first.
    pub fn all(&self) -> Vec<IndexedFile> {
        self.with(|c| {
            let mut s = c.prepare("SELECT path, name, modified FROM files ORDER BY modified DESC")?;
            s.query_map([], file)?.collect()
        })
    }

    /// Full-text matches only (body), every term as a prefix; `prefix` limits results to a folder.
    pub fn content_search(&self, terms: &[String], prefix: Option<&Path>, limit: usize) -> Vec<SearchHit> {
        if terms.is_empty() {
            return Vec::new();
        }
        let query = format!("body : {}", terms.iter().map(|t| format!("\"{}\"*", t.replace('"', ""))).collect::<Vec<_>>().join(" "));
        let like = format!("{}%", prefix.map(|p| p.to_string_lossy().to_string()).unwrap_or_default());
        self.with(|c| {
            let mut s = c.prepare(
                "SELECT f.path, f.name, f.modified, snippet(fts, 1, '', '', '…', 14) FROM fts JOIN files f ON f.id = fts.rowid WHERE fts MATCH ?1 AND f.path LIKE ?2 ORDER BY rank LIMIT ?3",
            )?;
            s.query_map(params![query, like, limit as i64], |r| Ok(SearchHit { file: file(r)?, snippet: r.get::<_, Option<String>>(3)?.map(|s| s.replace('\n', " ")) }))?.collect()
        })
    }

    pub fn recent(&self, limit: usize, extensions: &[&str], prefix: Option<&Path>) -> Vec<IndexedFile> {
        let like = format!("{}%", prefix.map(|p| p.to_string_lossy().to_string()).unwrap_or_default());
        let files: Vec<IndexedFile> = self.with(|c| {
            let mut s = c.prepare("SELECT path, name, modified FROM files WHERE path LIKE ?1 ORDER BY modified DESC LIMIT 400")?;
            s.query_map([like], file)?.collect()
        });
        files.into_iter().filter(|f| extensions.contains(&extension(&f.name).to_lowercase().as_str())).take(limit).collect()
    }
}

fn file(r: &rusqlite::Row) -> rusqlite::Result<IndexedFile> {
    let secs: f64 = r.get(2)?;
    Ok(IndexedFile {
        path: PathBuf::from(r.get::<_, String>(0)?),
        name: r.get(1)?,
        modified: Local.timestamp_opt(secs as i64, ((secs.fract()) * 1e9) as u32).single().unwrap_or_default(),
    })
}

/// macOS bundles (apps, Keynote/Pages packages) are files to the user.
fn is_package(p: &Path) -> bool {
    ["app", "key", "pages", "numbers", "rtfd", "bundle", "photoslibrary", "xcodeproj"].contains(&extension(&p.to_string_lossy()).to_lowercase().as_str())
}

/// Searchable text of a file: PDF text, or plain text with HTML tags removed.
pub fn extract(path: &Path) -> String {
    static SCRIPT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<(script|style)[^>]*>.*?</(script|style)>").unwrap());
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
    static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let ext = extension(&name).to_lowercase();
    let take = |s: String| s.chars().take(MAX_TEXT).collect::<String>();
    if ext == "pdf" {
        return lopdf::Document::load(path)
            .ok()
            .map(|d| {
                let pages: Vec<u32> = d.get_pages().keys().copied().collect();
                take(d.extract_text(&pages).unwrap_or_default())
            })
            .unwrap_or_default();
    }
    if !TEXT_EXTENSIONS.contains(&ext.as_str()) {
        return String::new();
    }
    let Ok(mut s) = std::fs::read_to_string(path) else { return String::new() };
    if ext == "html" {
        s = SCRIPT.replace_all(&s, " ").into_owned();
        s = TAG.replace_all(&s, " ").into_owned();
        s = SPACE.replace_all(&s, " ").into_owned();
    }
    take(s)
}

#[cfg(test)]
mod index_tests {
    use super::SearchIndex;
    use std::sync::Mutex;

    fn index(name: &str) -> (SearchIndex, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("schrumbi-index-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("root");
        std::fs::create_dir_all(&root).unwrap();
        (SearchIndex { db: Mutex::new(SearchIndex::connect(&dir.join("i.sqlite"))) }, root)
    }

    #[test]
    fn follows_new_changed_and_deleted_files() {
        let (index, root) = index("changes");
        let search = |term: &str| index.content_search(&[term.to_string()], None, 10).len();
        std::fs::write(root.join("a.md"), "apfel").unwrap();
        std::fs::write(root.join("b.md"), "birne").unwrap();
        assert_eq!(index.update(&root, &[]), 2);
        assert_eq!((search("apfel"), search("birne")), (1, 1));

        std::fs::remove_file(root.join("b.md")).unwrap();
        std::fs::write(root.join("c.md"), "kirsche").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(root.join("a.md"), "traube").unwrap();
        assert_eq!(index.update(&root, &[]), 2);
        assert_eq!(index.count(), 2);
        assert_eq!((search("apfel"), search("birne"), search("traube"), search("kirsche")), (0, 0, 1, 1));
    }

    #[test]
    fn indexes_many_files_quickly() {
        let (index, root) = index("many");
        for i in 0..3000 {
            std::fs::write(root.join(format!("f{i}.md")), format!("inhalt {i}")).unwrap();
        }
        let start = std::time::Instant::now();
        assert_eq!(index.update(&root, &[]), 3000);
        assert!(start.elapsed().as_secs() < 20, "{:?}", start.elapsed());
        println!("3000 files: {:?}", start.elapsed());
    }
}
