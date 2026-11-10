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

/// The entries of a session file; unreadable lines are skipped.
pub fn load(file: &Path) -> Vec<Entry> {
    std::fs::read_to_string(file).map(|text| text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
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
