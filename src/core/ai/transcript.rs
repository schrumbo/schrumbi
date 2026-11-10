use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::prefs;

/// Who said a line of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Agent,
    Tool,
}

/// One line of a session as it is kept in the app data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub role: Role,
    pub text: String,
    /// Seconds since 1970.
    pub at: i64,
}

/// Where the sessions of the study folder `root` are kept: in the app data, never in the folder itself.
pub fn dir(root: &Path) -> Option<PathBuf> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()).hash(&mut h);
    prefs::data_dir().map(|d| d.join("ai").join(format!("{:016x}", h.finish())).join("sessions"))
}

/// The file for a new session started now.
pub fn file_for(root: &Path, id: &str) -> Option<PathBuf> {
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S");
    dir(root).map(|d| d.join(format!("{stamp}-{id}.jsonl")))
}

/// Appends entries to a session file, one JSON line each.
pub fn append(file: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = std::fs::OpenOptions::new().create(true).append(true).open(file)?;
    for entry in entries {
        writeln!(out, "{}", serde_json::to_string(entry).map_err(std::io::Error::other)?)?;
    }
    out.flush()
}

/// A session as a Markdown note: the user's messages as quotes, the answers as text, tool steps as a list.
pub fn markdown(title: &str, entries: &[Entry]) -> String {
    let mut out = format!("# {title}\n");
    let mut in_tools = false;
    for entry in entries {
        match entry.role {
            Role::User => {
                out.push('\n');
                for line in entry.text.lines() {
                    out.push_str("> ");
                    out.push_str(line);
                    out.push('\n');
                }
                in_tools = false;
            }
            Role::Agent => {
                out.push('\n');
                out.push_str(entry.text.trim_end());
                out.push('\n');
                in_tools = false;
            }
            Role::Tool => {
                if !in_tools {
                    out.push('\n');
                }
                out.push_str("- ");
                out.push_str(entry.text.trim());
                out.push('\n');
                in_tools = true;
            }
        }
    }
    out
}

/// An earlier session of a study folder, for the column's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Past {
    pub file: PathBuf,
    /// When it started, from the file name.
    pub started: Option<chrono::NaiveDateTime>,
    /// The first message of the user.
    pub first: String,
}

/// The sessions of the study folder `root`, newest first, at most `limit`; sessions without a message of the user are left out.
pub fn list(root: &Path, limit: usize) -> Vec<Past> {
    let Some(dir) = dir(root) else { return Vec::new() };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "jsonl")).collect()).unwrap_or_default();
    files.sort();
    files.reverse();
    let mut out = Vec::new();
    for file in files {
        if out.len() >= limit {
            break;
        }
        let Some(first) = first_message(&file) else { continue };
        out.push(Past { started: started(&file), file, first });
    }
    out
}

/// When a session started, from its file name.
pub fn started(file: &Path) -> Option<chrono::NaiveDateTime> {
    let name = file.file_name()?.to_string_lossy().to_string();
    name.get(..17).and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d-%H%M%S").ok())
}

fn first_message(file: &Path) -> Option<String> {
    use std::io::BufRead;
    let reader = std::io::BufReader::new(std::fs::File::open(file).ok()?);
    reader.lines().take(50).map_while(Result::ok).filter_map(|l| serde_json::from_str::<Entry>(&l).ok()).find(|e| e.role == Role::User).map(|e| e.text)
}

/// Every entry of a session file; lines that cannot be read are skipped.
pub fn load(file: &Path) -> Vec<Entry> {
    std::fs::read_to_string(file).map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// An earlier session as context for its continuation, the newest part kept when it is long.
pub fn recap(entries: &[Entry], chars: usize) -> String {
    let mut out = String::new();
    for e in entries.iter().rev() {
        let who = match e.role {
            Role::User => "User",
            Role::Agent => "You",
            Role::Tool => "Step",
        };
        let line = format!("{who}: {}\n", e.text.trim());
        if out.chars().count() + line.chars().count() > chars {
            break;
        }
        out.insert_str(0, &line);
    }
    out
}
