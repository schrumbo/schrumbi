use super::cmd::{Git, GitError};

/// How a file differs from the last state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    New,
    Modified,
    Deleted,
    Renamed { from: String },
    Conflicted,
}

/// A changed file, its path relative to the repository and with forward slashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub kind: ChangeKind,
}

/// Branch and working tree at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoStatus {
    /// `None` when no branch is checked out.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub changed: Vec<ChangedFile>,
    /// Folders with a repository of their own; they are never added.
    pub nested: Vec<String>,
}

impl Git {
    /// Reads the status without taking Git's optional locks.
    pub fn status(&self) -> Result<RepoStatus, GitError> {
        let out = self.run_bulk(&["--no-optional-locks", "status", "--porcelain=v2", "--branch", "-z", "-uall"], None)?;
        Ok(parse(&out))
    }
}

/// Obsidian rewrites its window layout on every click; that is never a change worth keeping.
fn is_window_state(path: &str) -> bool {
    path.strip_prefix(".obsidian/").is_some_and(|name| name == "workspace" || (name.starts_with("workspace") && name.ends_with(".json")))
}

/// Parses `git status --porcelain=v2 --branch -z`.
pub fn parse(out: &str) -> RepoStatus {
    let mut s = RepoStatus::default();
    let mut has_counts = false;
    let mut fields = out.split('\0');
    while let Some(line) = fields.next() {
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix("# ") {
            let mut words = header.split(' ');
            match words.next() {
                Some("branch.head") => s.branch = words.next().filter(|b| *b != "(detached)").map(String::from),
                Some("branch.upstream") => s.upstream = words.next().map(String::from),
                Some("branch.ab") => {
                    has_counts = true;
                    s.ahead = words.next().and_then(|a| a.trim_start_matches('+').parse().ok()).unwrap_or(0);
                    s.behind = words.next().and_then(|b| b.trim_start_matches('-').parse().ok()).unwrap_or(0);
                }
                _ => {}
            }
            continue;
        }
        let kind = line.as_bytes()[0];
        match kind {
            b'?' => {
                let path = &line[2..];
                match path.strip_suffix('/') {
                    Some(folder) => s.nested.push(folder.to_string()),
                    None => s.changed.push(ChangedFile { path: path.to_string(), kind: ChangeKind::New }),
                }
            }
            b'1' | b'2' | b'u' => {
                let parts = match kind {
                    b'1' => 9,
                    b'2' => 10,
                    _ => 11,
                };
                let items: Vec<&str> = line.splitn(parts, ' ').collect();
                let (Some(xy), Some(sub), Some(path)) = (items.get(1), items.get(2), items.last()) else { continue };
                if sub.starts_with('S') {
                    s.nested.push(path.to_string());
                    if kind == b'2' {
                        fields.next();
                    }
                    continue;
                }
                let change = match kind {
                    b'u' => ChangeKind::Conflicted,
                    b'2' => ChangeKind::Renamed { from: fields.next().unwrap_or_default().to_string() },
                    _ if xy.contains('D') => ChangeKind::Deleted,
                    _ if xy.starts_with('A') => ChangeKind::New,
                    _ => ChangeKind::Modified,
                };
                s.changed.push(ChangedFile { path: path.to_string(), kind: change });
            }
            _ => {}
        }
    }
    if !has_counts {
        s.upstream = None;
    }
    s.changed.retain(|c| !is_window_state(&c.path));
    s
}
