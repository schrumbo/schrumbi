use std::path::{Path, PathBuf};
use std::process::Command;

use super::cmd::{Git, GitError, LOCAL, NETWORK, spawn};

/// Lines every synced workspace should ignore: backups, temporary files, and what apps regenerate.
pub const IGNORE_LINES: [&str; 8] = [".schrumbi-backup/", ".*.tmp", "*.defekt", "Schrumbi-Kalender.ics", ".DS_Store", ".obsidian/workspace*.json", ".obsidian/cache", ".trash/"];

/// The app's own files at the top of the study folder; they sync unless the user leaves them out.
pub const APP_DATA: [&str; 3] = [crate::core::workspace::CONFIG_NAME, crate::core::workspace::STATE_NAME, crate::core::cards::STORE_NAME];

/// What setting up sync would find and do. Reading it runs Git only after the user asked for setup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inspection {
    pub has_repo: bool,
    /// The repository root when it lies above the workspace; sync needs them to be the same.
    pub outer_root: Option<PathBuf>,
    pub remote: Option<String>,
    pub has_identity: bool,
    pub missing_ignores: Vec<String>,
    pub tracked_backups: usize,
    pub nested: Vec<String>,
    /// Name of the cloud service the folder lives in.
    pub cloud: Option<String>,
    pub obsidian_git: bool,
    /// Whether `gh` is signed in, so a private repository can be created.
    pub gh: bool,
    /// Known only through `gh`.
    pub public: Option<bool>,
}

/// The cloud service a path belongs to, if any.
pub fn cloud_service(path: &Path) -> Option<&'static str> {
    let text = path.to_string_lossy().replace('\\', "/");
    [("Library/Mobile Documents", "iCloud"), ("/Dropbox", "Dropbox"), ("/OneDrive", "OneDrive"), ("/Google Drive", "Google Drive"), ("Library/CloudStorage", "Cloud")].iter().find(|(needle, _)| text.contains(needle)).map(|(_, name)| *name)
}

/// Whether Obsidian's Git plugin is on; two programs syncing the same folder would fight.
pub fn obsidian_git_enabled(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".obsidian/community-plugins.json")).is_ok_and(|s| s.contains("obsidian-git"))
}

fn missing_ignores(root: &Path) -> Vec<String> {
    let current = std::fs::read_to_string(root.join(".gitignore")).unwrap_or_default();
    IGNORE_LINES.iter().filter(|l| !current.lines().any(|c| c.trim() == **l)).map(|l| l.to_string()).collect()
}

/// The `.gitignore` text; a missing file is empty, one that is not text is an error so it is never overwritten.
fn read_ignore(path: &Path) -> Result<String, GitError> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|_| GitError::Other(".gitignore".into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(GitError::Other(e.to_string())),
    }
}

fn lists(ignore: &str, name: &str) -> bool {
    ignore.lines().any(|l| l.trim() == name)
}

fn gh(args: &[&str], dir: &Path) -> Option<String> {
    if crate::core::drive::fenced("network", format!("gh {}", args.join(" "))) {
        return None;
    }
    let mut c = Command::new(gh_path()?);
    c.args(args).current_dir(dir).env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1");
    let out = spawn(c, None, NETWORK).ok()?;
    (out.code == 0).then_some(out.stdout)
}

fn gh_path() -> Option<PathBuf> {
    let name = if cfg!(windows) { "gh.exe" } else { "gh" };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into()]).map(|d: PathBuf| d.join(name)).find(|p| p.is_file())
}

/// Looks at the workspace; touches nothing.
pub fn inspect(git: &Git) -> Inspection {
    let root = &git.root;
    let mut i = Inspection { cloud: cloud_service(root).map(String::from), obsidian_git: obsidian_git_enabled(root), missing_ignores: missing_ignores(root), ..Default::default() };
    i.gh = gh(&["auth", "status"], root).is_some();
    if let Ok(top) = git.run(&["rev-parse", "--show-toplevel"]) {
        let top = PathBuf::from(top.trim());
        let same = dunce::canonicalize(&top).ok() == dunce::canonicalize(root).ok();
        if same {
            i.has_repo = true;
        } else {
            i.outer_root = Some(top);
            return i;
        }
    }
    let identity = |key: &str| git.run(&["config", "--get", key]).is_ok_and(|v| !v.trim().is_empty());
    i.has_identity = identity("user.name") && identity("user.email");
    if !i.has_repo {
        return i;
    }
    i.remote = git.run(&["remote", "get-url", "origin"]).ok().map(|u| u.trim().to_string());
    i.tracked_backups = git.run(&["ls-files", "-z", "--", ".schrumbi-backup"]).map_or(0, |o| o.split('\0').filter(|p| !p.is_empty()).count());
    i.nested = git.status().map_or_else(|_| Vec::new(), |s| s.nested);
    if i.gh && i.remote.is_some() {
        i.public = gh(&["repo", "view", "--json", "isPrivate", "--jq", ".isPrivate"], root).map(|v| v.trim() == "false");
    }
    i
}

impl Git {
    /// Starts a repository that keeps line endings as the files have them, so Obsidian and other apps see the same bytes on every device.
    pub fn init_repo(&self) -> Result<(), GitError> {
        self.run(&["init", "-q"])?;
        self.run(&["config", "core.autocrlf", "false"]).map(|_| ())
    }

    /// Whether the remote answers; for checking an address before it is saved.
    pub fn test_remote(&self, url: &str) -> Result<(), GitError> {
        self.run_network(&["ls-remote", "--exit-code", "--heads", "--", url]).map(|_| ()).or_else(|e| if matches!(&e, GitError::Other(m) if m.is_empty()) { Ok(()) } else { Err(e) })
    }

    pub fn set_remote(&self, url: &str) -> Result<(), GitError> {
        if self.run(&["remote", "get-url", "origin"]).is_ok() {
            self.run(&["remote", "set-url", "origin", url]).map(|_| ())
        } else {
            self.run(&["remote", "add", "origin", url]).map(|_| ())
        }
    }

    pub fn set_identity(&self, name: &str, email: &str) -> Result<(), GitError> {
        self.run(&["config", "user.name", name])?;
        self.run(&["config", "user.email", email]).map(|_| ())
    }

    /// Creates a private repository with `gh` and sets it as the remote.
    pub fn create_private_remote(&self, name: &str) -> Result<(), GitError> {
        let out = gh(&["repo", "create", name, "--private", "--source", ".", "--remote", "origin"], &self.root);
        out.map(|_| ()).ok_or_else(|| GitError::Other("gh repo create".into()))
    }

    /// Writes the missing ignore lines, ignores nested repositories, takes backups out of the history
    /// (they stay on disk) and records that as one state. Returns that state's id, if one was made.
    pub fn clean_up(&self, ignore_lines: &[String], nested: &[String], untrack_backups: bool, message: &str) -> Result<Option<String>, GitError> {
        let mut add: Vec<String> = ignore_lines.to_vec();
        let path = self.root.join(".gitignore");
        let current = read_ignore(&path)?;
        for folder in nested {
            let line = format!("{folder}/");
            if !current.lines().any(|l| l.trim() == line) {
                add.push(line);
            }
        }
        if !add.is_empty() {
            let separator = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
            std::fs::write(&path, format!("{current}{separator}{}\n", add.join("\n"))).map_err(|e| GitError::Other(e.to_string()))?;
            self.run(&["add", "--", ".gitignore"])?;
        }
        if untrack_backups {
            self.run(&["rm", "-r", "-q", "--cached", "--ignore-unmatch", "--", ".schrumbi-backup"])?;
        }
        self.run(&["rm", "-q", "--cached", "--ignore-unmatch", "--", ".obsidian/workspace.json", ".obsidian/workspace-mobile.json", ".obsidian/workspace"])?;
        let staged = self.output(&["diff", "--cached", "--quiet"], None, LOCAL)?;
        if staged.code == 0 {
            return Ok(None);
        }
        self.checked(&["commit", "-q", "--cleanup=whitespace", "-F", "-"], Some(message.as_bytes().to_vec()), LOCAL)?;
        Ok(Some(self.run(&["rev-parse", "HEAD"])?.trim().to_string()))
    }

    /// Takes the cleanup back as a new state; files on disk were never touched by it.
    pub fn undo_clean_up(&self, state: &str) -> Result<(), GitError> {
        self.run(&["revert", "--no-edit", state]).map(|_| ())
    }

    /// Whether the app's own files travel with the folder: none of them is in `.gitignore`.
    pub fn app_data_synced(&self) -> bool {
        let current = read_ignore(&self.root.join(".gitignore")).unwrap_or_default();
        !APP_DATA.iter().any(|name| lists(&current, name))
    }

    /// Takes the app's own files in or out of sync. Out: they go into `.gitignore` and leave the history, the files stay on disk.
    /// In: those lines go again and the next sync takes the files back. Returns the id of the state it made, if any.
    pub fn set_app_data_synced(&self, on: bool, message: &str) -> Result<Option<String>, GitError> {
        let path = self.root.join(".gitignore");
        let current = read_ignore(&path)?;
        let text = if on {
            current.split_inclusive('\n').filter(|l| !APP_DATA.contains(&l.trim())).collect::<String>()
        } else {
            let add: Vec<&str> = APP_DATA.iter().copied().filter(|name| !lists(&current, name)).collect();
            let separator = if add.is_empty() || current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
            if add.is_empty() { current.clone() } else { format!("{current}{separator}{}\n", add.join("\n")) }
        };
        if text != current {
            let temp = path.with_extension("schrumbi-tmp");
            std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, &path)).map_err(|e| {
                let _ = std::fs::remove_file(&temp);
                GitError::Other(e.to_string())
            })?;
            self.run(&["add", "--", ".gitignore"])?;
        }
        if !on {
            let mut args = vec!["rm", "-q", "--cached", "--ignore-unmatch", "--"];
            args.extend(APP_DATA);
            self.run(&args)?;
        }
        let staged = self.output(&["diff", "--cached", "--quiet"], None, LOCAL)?;
        if staged.code == 0 {
            return Ok(None);
        }
        self.checked(&["commit", "-q", "--cleanup=whitespace", "-F", "-"], Some(message.as_bytes().to_vec()), LOCAL)?;
        Ok(Some(self.run(&["rev-parse", "HEAD"])?.trim().to_string()))
    }

    /// Whether the other device took this app file out of sync: it is gone from the upstream and listed in its `.gitignore`.
    pub(super) fn left_out_upstream(&self, name: &str) -> bool {
        APP_DATA.contains(&name)
            && self.run(&["cat-file", "-e", &format!("@{{u}}:{name}")]).is_err()
            && self.run(&["show", "@{u}:.gitignore"]).is_ok_and(|ignore| lists(&ignore, name))
    }

    /// After taking in the other device's states: an app file it left out of sync stays here as it was, for it is still in use.
    pub(super) fn keep_app_data(&self, before: &str) {
        let ignore = read_ignore(&self.root.join(".gitignore")).unwrap_or_default();
        for name in APP_DATA {
            let path = self.root.join(name);
            if path.exists() || !lists(&ignore, name) {
                continue;
            }
            if let Ok(blob) = self.output(&["cat-file", "blob", &format!("{before}:{name}")], None, LOCAL)
                && blob.code == 0
            {
                let _ = std::fs::write(&path, blob.raw);
            }
        }
    }

    /// Whether this device took the app file out of sync: it is not in the history here and listed in the local `.gitignore`.
    pub(super) fn left_out_here(&self, name: &str) -> bool {
        APP_DATA.contains(&name)
            && self.run(&["cat-file", "-e", &format!("HEAD:{name}")]).is_err()
            && read_ignore(&self.root.join(".gitignore")).is_ok_and(|ignore| lists(&ignore, name))
    }

    /// The app files that are on disk here but not in the history, read before taking in the other device's states,
    /// for Git overwrites ignored files without asking.
    pub(super) fn untracked_app_data(&self) -> Vec<(&'static str, Vec<u8>)> {
        APP_DATA
            .iter()
            .filter(|name| self.run(&["cat-file", "-e", &format!("HEAD:{name}")]).is_err())
            .filter_map(|name| std::fs::read(self.root.join(name)).ok().map(|bytes| (*name, bytes)))
            .collect()
    }

    /// Puts this device's app files back after taking in the other device's states. A file still left out here stays as it was;
    /// one that syncs again keeps both devices' changes and is saved as a new state. Returns whether a state was made.
    pub(super) fn restore_app_data(&self, kept: &[(&str, Vec<u8>)], message: &str) -> Result<bool, GitError> {
        let ignore = read_ignore(&self.root.join(".gitignore")).unwrap_or_default();
        let mut staged = false;
        for (name, bytes) in kept {
            let path = self.root.join(name);
            let now = std::fs::read(&path).ok();
            if now.as_deref() == Some(bytes.as_slice()) {
                continue;
            }
            if lists(&ignore, name) {
                std::fs::write(&path, bytes).map_err(|e| GitError::Other(e.to_string()))?;
                continue;
            }
            let local = String::from_utf8_lossy(bytes);
            let merged = match now.as_deref().map(String::from_utf8_lossy) {
                Some(theirs) => super::merge_json::merge_file(name, None, &local, &theirs).unwrap_or_else(|| local.to_string()),
                None => local.to_string(),
            };
            std::fs::write(&path, merged).map_err(|e| GitError::Other(e.to_string()))?;
            self.run(&["add", "--", name])?;
            staged = true;
        }
        if !staged || self.output(&["diff", "--cached", "--quiet"], None, LOCAL)?.code == 0 {
            return Ok(false);
        }
        self.checked(&["commit", "-q", "--cleanup=whitespace", "-F", "-"], Some(message.as_bytes().to_vec()), LOCAL)?;
        Ok(true)
    }

    /// When the newest state was made, as a Unix time; `None` before the first.
    pub fn last_commit_time(&self) -> Option<i64> {
        self.run(&["--no-optional-locks", "log", "-1", "--format=%ct"]).ok()?.trim().parse().ok()
    }

    /// Whether any remote is set, which sync needs for uploading.
    pub fn has_remote(&self) -> bool {
        self.run(&["remote"]).is_ok_and(|r| r.lines().any(|l| !l.trim().is_empty()))
    }
}
