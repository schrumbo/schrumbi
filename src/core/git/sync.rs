use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use super::cmd::{Git, GitError};
use super::status::{ChangeKind, ChangedFile, RepoStatus};

/// GitHub refuses files of 100 MB; these stay local and are reported.
pub const MAX_FILE: u64 = 95 * 1024 * 1024;
/// More deletions than this, and at least 30 % of the tracked files, pause automatic saving.
const MANY_DELETED: usize = 20;
/// How often a push is retried after someone else pushed first.
const PUSH_ATTEMPTS: usize = 3;

/// The step a sync is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Saving,
    Fetching,
    Merging,
    Uploading,
}

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Number of files in the new state, if one was made.
    pub committed: usize,
    /// Files that came from the remote, absolute.
    pub pulled: Vec<PathBuf>,
    pub pushed: bool,
    /// The remote could not be reached; everything is saved locally.
    pub offline: bool,
    pub too_large: Vec<String>,
    pub nested: Vec<String>,
}

impl Git {
    /// Stops when the repository is in a state only the user should change.
    fn preflight(&self) -> Result<RepoStatus, GitError> {
        let dir = self.git_dir();
        if dir.join("index.lock").exists() {
            return Err(GitError::Locked);
        }
        if ["MERGE_HEAD", "rebase-merge", "rebase-apply", "CHERRY_PICK_HEAD", "REVERT_HEAD", "BISECT_LOG"].iter().any(|n| dir.join(n).exists()) {
            return Err(GitError::ExternalMerge);
        }
        let status = self.status()?;
        if status.branch.is_none() {
            return Err(GitError::Detached);
        }
        Ok(status)
    }

    /// Stages everything except nested repositories and oversized files, then makes one new state.
    /// Unless `deletions_allowed`, a state that deletes many tracked files at once is not made.
    fn commit_all(&self, status: &RepoStatus, message: &dyn Fn(&[ChangedFile]) -> String, deletions_allowed: bool, report: &mut Report) -> Result<(), GitError> {
        report.nested = status.nested.clone();
        let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_string();
        let arrived: std::collections::HashSet<String> = status.changed.iter().filter(|f| f.kind == ChangeKind::New).map(|f| name(&f.path)).collect();
        let deleted = status.changed.iter().filter(|f| f.kind == ChangeKind::Deleted && !arrived.contains(&name(&f.path))).count();
        if !deletions_allowed && deleted > MANY_DELETED {
            let tracked = self.run_bulk(&["ls-files", "-z"], None)?.split('\0').filter(|p| !p.is_empty()).count();
            if deleted * 10 > tracked * 3 {
                return Err(GitError::ManyDeleted(deleted));
            }
        }
        let mut files = Vec::new();
        for f in &status.changed {
            let big = f.kind != ChangeKind::Deleted && std::fs::metadata(self.root.join(&f.path)).is_ok_and(|m| m.len() > MAX_FILE);
            if big {
                report.too_large.push(f.path.clone());
            } else {
                files.push(f.clone());
            }
        }
        if files.is_empty() {
            return Ok(());
        }
        let mut list = Vec::new();
        for f in &files {
            list.extend_from_slice(f.path.as_bytes());
            list.push(0);
        }
        self.run_bulk(&["add", "-A", "--pathspec-from-file=-", "--pathspec-file-nul"], Some(list))?;
        let text = message(&files);
        let text = if text.trim().is_empty() { "Stand".to_string() } else { text };
        self.run_bulk(&["commit", "-q", "--cleanup=whitespace", "-F", "-"], Some(text.into_bytes()))?;
        report.committed = files.len();
        Ok(())
    }

    fn head(&self) -> Option<String> {
        self.run(&["rev-parse", "HEAD"]).ok().map(|s| s.trim().to_string())
    }

    /// Takes in the upstream: fast-forward, or a merge when both sides moved. Conflicts change nothing.
    fn integrate(&self, report: &mut Report) -> Result<(), GitError> {
        let status = self.status()?;
        if status.upstream.is_none() || status.behind == 0 {
            return Ok(());
        }
        let before = self.head();
        if status.ahead == 0 {
            self.run_bulk(&["merge", "-q", "--ff-only", "@{u}"], None)?;
        } else {
            let (conflicts, automatic) = self.conflicts()?;
            if !conflicts.is_empty() {
                return Err(GitError::Conflicts(conflicts.into_iter().map(|c| c.path).collect()));
            }
            if !automatic.is_empty() {
                let resolved = self.merge_resolved(&HashMap::new(), &automatic, &crate::core::i18n::tr("Stand vom anderen Gerät übernommen"))?;
                report.pulled.extend(resolved.pulled);
                return Ok(());
            }
            let message = crate::core::i18n::tr("Stand vom anderen Gerät übernommen");
            self.run_bulk(&["merge", "-q", "-m", &message, "@{u}"], None)?;
        }
        if let Some(before) = before {
            report.pulled.extend(self.settle_times(&before)?);
        }
        Ok(())
    }

    /// Gives files changed since `from` the time of their last state, because deadlines are judged by file time
    /// and checking out would stamp them with now. Returns those files.
    pub(super) fn settle_times(&self, from: &str) -> Result<Vec<PathBuf>, GitError> {
        let out = self.run_bulk(&["log", "--format=%x01%ct", "--name-only", "-z", &format!("{from}..HEAD")], None)?;
        let mut times: HashMap<String, u64> = HashMap::new();
        parse_log_times(&out, &mut times);
        let changed = self.run_bulk(&["diff", "--name-only", "--no-renames", "-z", from, "HEAD"], None)?;
        let mut files = Vec::new();
        for rel in changed.split('\0').filter(|p| !p.is_empty()) {
            let path = self.root.join(rel);
            if let (Some(time), Ok(file)) = (times.get(rel), std::fs::File::options().write(true).open(&path)) {
                let _ = file.set_modified(UNIX_EPOCH + Duration::from_secs(*time));
            }
            files.push(path);
        }
        Ok(files)
    }

    /// The remote to use: `origin` if there is one, else the first.
    fn remote(&self) -> Option<String> {
        let remotes = self.run(&["remote"]).ok()?;
        let mut names = remotes.lines().map(str::trim).filter(|r| !r.is_empty());
        let first = names.next()?;
        Some(if first == "origin" || names.any(|n| n == "origin") { "origin" } else { first }.to_string())
    }

    fn fetch(&self) -> Result<(), GitError> {
        self.run_network(&["fetch", "-q"]).map(|_| ())
    }

    fn push(&self, status: &RepoStatus, remote: &str) -> Result<(), GitError> {
        if status.upstream.is_some() {
            self.run_network(&["push", "--progress"]).map(|_| ())
        } else {
            self.run_network(&["push", "--progress", "-u", remote, "HEAD"]).map(|_| ())
        }
    }

    /// One full sync: save, fetch, take in, upload. A missing connection is not a failure, the state stays local.
    /// Many files gone at once are only recorded when `deletions_allowed`, which the user confirmed.
    pub fn sync(&self, message: &dyn Fn(&[ChangedFile]) -> String, progress: &dyn Fn(Step), deletions_allowed: bool) -> Result<Report, GitError> {
        let mut report = Report::default();
        progress(Step::Saving);
        let status = self.preflight()?;
        self.commit_all(&status, message, deletions_allowed, &mut report)?;
        let Some(remote) = self.remote() else { return Err(GitError::NoRemote) };
        for attempt in 0..PUSH_ATTEMPTS {
            progress(Step::Fetching);
            match self.fetch() {
                Ok(()) => {}
                Err(GitError::Offline | GitError::Timeout) => {
                    report.offline = true;
                    return Ok(report);
                }
                Err(e) => return Err(e),
            }
            progress(Step::Merging);
            self.integrate(&mut report)?;
            let now = self.status()?;
            if now.upstream.is_some() && now.ahead == 0 {
                return Ok(report);
            }
            progress(Step::Uploading);
            match self.push(&now, &remote) {
                Ok(()) => {
                    report.pushed = true;
                    return Ok(report);
                }
                Err(GitError::Offline | GitError::Timeout) => {
                    report.offline = true;
                    return Ok(report);
                }
                Err(GitError::Rejected) if attempt + 1 < PUSH_ATTEMPTS => {}
                Err(e) => return Err(e),
            }
        }
        Err(GitError::Rejected)
    }
}

/// Collects the newest time per path from `git log --format=%x01%ct --name-only -z`, which lists newest first.
pub fn parse_log_times(out: &str, times: &mut HashMap<String, u64>) {
    let mut current = 0;
    for piece in out.split('\0') {
        let piece = piece.trim_start_matches('\n');
        let path = if let Some(header) = piece.strip_prefix('\u{1}') {
            let (time, rest) = header.split_once('\n').unwrap_or((header, ""));
            current = time.trim().parse().unwrap_or(0);
            rest.trim_start_matches('\n')
        } else {
            piece
        };
        if !path.is_empty() {
            times.entry(path.to_string()).or_insert(current);
        }
    }
}
