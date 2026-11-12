use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use super::cmd::{BULK, Git, GitError, LOCAL};
use super::merge_json;
use crate::core::i18n::tr;
use crate::core::library::unique_path;
use crate::core::naming::{extension, stem};

/// What the two sides did to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    Text,
    Binary,
    /// Changed here, deleted on the other device.
    DeletedThere,
    /// Deleted here, changed on the other device.
    DeletedHere,
}

/// One file both devices changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictFile {
    pub path: String,
    pub kind: ConflictKind,
    /// Blob ids of the common version, this device and the other device.
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

/// What should become of a conflicted file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// This device's version keeps the name, the other one becomes a copy.
    Both,
    Mine,
    Theirs,
}

/// What resolving did, including how to take it back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub copies: Vec<PathBuf>,
    pub pulled: Vec<PathBuf>,
    /// The state before merging.
    pub before: String,
    /// Files whose chosen version replaced this device's.
    pub touched: Vec<String>,
}

/// Parses `git merge-tree --write-tree -z`: the tree, then `mode oid stage\tpath` entries, an empty field, then messages.
pub fn parse_stages(out: &str) -> BTreeMap<String, [Option<String>; 3]> {
    let mut files: BTreeMap<String, [Option<String>; 3]> = BTreeMap::new();
    for entry in out.split('\0').skip(1).take_while(|e| !e.is_empty()) {
        let Some((meta, path)) = entry.split_once('\t') else { continue };
        let mut parts = meta.split(' ');
        let (_, Some(oid), Some(stage)) = (parts.next(), parts.next(), parts.next().and_then(|s| s.parse::<usize>().ok())) else { continue };
        if (1..=3).contains(&stage) {
            files.entry(path.to_string()).or_default()[stage - 1] = Some(oid.to_string());
        }
    }
    files
}

impl Git {
    fn blob_text(&self, oid: &str) -> Result<String, GitError> {
        self.run_bulk(&["cat-file", "blob", oid], None)
    }

    fn blob_is_binary(&self, oid: &str) -> bool {
        self.output(&["cat-file", "blob", oid], None, BULK).is_ok_and(|o| o.raw.iter().take(8000).any(|b| *b == 0))
    }

    /// Conflicts a merge with the upstream would have. Nothing on disk changes.
    /// The second list holds the app's own files with their merged text, which need no decision.
    pub fn conflicts(&self) -> Result<(Vec<ConflictFile>, Vec<(String, String)>), GitError> {
        let out = self.output(&["merge-tree", "--write-tree", "--no-messages", "-z", "HEAD", "@{u}"], None, BULK)?;
        match out.code {
            0 => return Ok((Vec::new(), Vec::new())),
            1 => {}
            _ => return Err(GitError::Other(out.stderr.trim().to_string())),
        }
        let mut files = Vec::new();
        let mut automatic = Vec::new();
        for (path, [base, ours, theirs]) in parse_stages(&out.stdout) {
            if (ours.is_some() && theirs.is_none() && self.left_out_upstream(&path)) || (ours.is_none() && theirs.is_some() && self.left_out_here(&path)) {
                automatic.push((path, String::new()));
                continue;
            }
            if let (Some(o), Some(t)) = (&ours, &theirs) {
                let base_text = base.as_deref().and_then(|b| self.blob_text(b).ok());
                if let (Ok(o), Ok(t)) = (self.blob_text(o), self.blob_text(t))
                    && let Some(merged) = merge_json::merge_file(&path, base_text.as_deref(), &o, &t)
                {
                    automatic.push((path, merged));
                    continue;
                }
            }
            let kind = match (&ours, &theirs) {
                (Some(_), None) => ConflictKind::DeletedThere,
                (None, Some(_)) => ConflictKind::DeletedHere,
                (Some(o), Some(t)) if self.blob_is_binary(o) || self.blob_is_binary(t) => ConflictKind::Binary,
                _ => ConflictKind::Text,
            };
            files.push(ConflictFile { path, kind, base, ours, theirs });
        }
        Ok((files, automatic))
    }

    /// When a blob was last changed on a side, as a Unix time.
    pub fn side_time(&self, rev: &str, path: &str) -> Option<i64> {
        self.run(&["log", "-1", "--format=%ct", rev, "--", path]).ok()?.trim().parse().ok()
    }

    /// Merges the upstream with these decisions; if anything fails the working tree goes back to how it was.
    pub fn merge_resolved(&self, choices: &HashMap<String, Choice>, automatic: &[(String, String)], message: &str) -> Result<Resolved, GitError> {
        let before = self.run(&["rev-parse", "HEAD"])?.trim().to_string();
        let kept = self.untracked_app_data();
        let started = self.output(&["merge", "-q", "--no-commit", "--no-ff", "--no-edit", "@{u}"], None, BULK)?;
        if started.code != 0 && !self.git_dir().join("MERGE_HEAD").exists() {
            let _ = self.restore_app_data(&kept, message);
            return Err(super::cmd::classify(&started.stderr));
        }
        match self.finish_merge(choices, automatic, message, &before) {
            Ok(resolved) => {
                self.restore_app_data(&kept, message)?;
                Ok(resolved)
            }
            Err(e) => {
                let _ = self.run_bulk(&["merge", "--abort"], None);
                let _ = self.restore_app_data(&kept, message);
                Err(e)
            }
        }
    }

    fn finish_merge(&self, choices: &HashMap<String, Choice>, automatic: &[(String, String)], message: &str, before: &str) -> Result<Resolved, GitError> {
        let mut resolved = Resolved { before: before.to_string(), ..Default::default() };
        for (path, text) in automatic {
            if self.left_out_upstream(path) || self.left_out_here(path) {
                self.run(&["rm", "-q", "--cached", "--ignore-unmatch", "--", path])?;
                continue;
            }
            std::fs::write(self.root.join(path), text).map_err(|e| GitError::Other(e.to_string()))?;
            self.run(&["add", "--", path])?;
        }
        let unmerged = self.run_bulk(&["diff", "--name-only", "--diff-filter=U", "-z"], None)?;
        for path in unmerged.split('\0').filter(|p| !p.is_empty()) {
            let choice = *choices.get(path).ok_or_else(|| GitError::Conflicts(vec![path.to_string()]))?;
            let stage = |n: u8| self.run(&["rev-parse", "-q", "--verify", &format!(":{n}:{path}")]).is_ok();
            let (has_ours, has_theirs) = (stage(2), stage(3));
            if choice == Choice::Both && has_ours && has_theirs {
                self.run(&["checkout", "--theirs", "--", path])?;
                let target = self.conflict_copy_path(path);
                std::fs::rename(self.root.join(path), &target).map_err(|e| GitError::Other(e.to_string()))?;
                let rel = target.strip_prefix(&self.root).unwrap_or(&target).to_string_lossy().replace('\\', "/");
                self.run(&["checkout", "--ours", "--", path])?;
                self.run(&["add", "--", path, &rel])?;
                resolved.copies.push(target);
                continue;
            }
            let keep_theirs = match choice {
                Choice::Theirs => true,
                Choice::Mine => false,
                Choice::Both => !has_ours,
            };
            resolved.touched.push(path.to_string());
            let exists = if keep_theirs { has_theirs } else { has_ours };
            if exists {
                self.run(&["checkout", if keep_theirs { "--theirs" } else { "--ours" }, "--", path])?;
                self.run(&["add", "--", path])?;
            } else {
                self.run(&["rm", "-q", "--ignore-unmatch", "--", path])?;
            }
        }
        self.checked(&["commit", "-q", "--cleanup=whitespace", "-F", "-"], Some(message.as_bytes().to_vec()), BULK)?;
        resolved.pulled = self.settle_times(before)?;
        self.keep_app_data(before);
        Ok(resolved)
    }

    fn conflict_copy_path(&self, path: &str) -> PathBuf {
        let full = self.root.join(path);
        let name = full.file_name().unwrap_or_default().to_string_lossy().to_string();
        let ext = extension(&name);
        let suffix = if ext.is_empty() { String::new() } else { format!(".{ext}") };
        let date = chrono::Local::now().format("%Y-%m-%d");
        unique_path(&full.with_file_name(format!("{} ({} {date}){suffix}", stem(&name), tr("Konflikt"))))
    }

    /// Takes a resolution back as a new state: the files return to their version from before the merge.
    /// History is never rewritten, so this also works after uploading.
    pub fn undo_resolution(&self, resolved: &Resolved, message: &str) -> Result<(), GitError> {
        for path in &resolved.touched {
            let existed = self.run(&["cat-file", "-e", &format!("{}:{path}", resolved.before)]).is_ok();
            if existed {
                self.run(&["checkout", &resolved.before, "--", path])?;
            } else {
                self.run(&["rm", "-q", "--ignore-unmatch", "--", path])?;
            }
        }
        for copy in &resolved.copies {
            let rel = copy.strip_prefix(&self.root).unwrap_or(copy).to_string_lossy().replace('\\', "/");
            self.run(&["rm", "-q", "--ignore-unmatch", "--", &rel])?;
        }
        let staged = self.output(&["diff", "--cached", "--quiet"], None, LOCAL)?;
        if staged.code != 0 {
            self.checked(&["commit", "-q", "--cleanup=whitespace", "-F", "-"], Some(message.as_bytes().to_vec()), BULK)?;
        }
        Ok(())
    }

    /// The text of a blob, for comparing.
    pub fn text_of(&self, oid: Option<&str>) -> String {
        oid.and_then(|o| self.blob_text(o).ok()).unwrap_or_default()
    }

    /// Writes a blob to `target`, for previews.
    pub fn write_blob(&self, oid: &str, target: &std::path::Path) -> Result<(), GitError> {
        let out = self.output(&["cat-file", "blob", oid], None, BULK)?;
        std::fs::write(target, out.raw).map_err(|e| GitError::Other(e.to_string()))
    }
}
