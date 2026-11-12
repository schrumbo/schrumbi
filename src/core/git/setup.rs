use std::path::{Path, PathBuf};
use std::process::Command;

use super::cmd::{Git, GitError, LOCAL, NETWORK, spawn};

/// Lines every synced workspace should ignore: backups, temporary files, and what apps regenerate.
pub const IGNORE_LINES: [&str; 8] = [".schrumbi-backup/", ".*.tmp", "*.defekt", "Schrumbi-Kalender.ics", ".DS_Store", ".obsidian/workspace*.json", ".obsidian/cache", ".trash/"];

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
        let current = std::fs::read_to_string(&path).unwrap_or_default();
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
}
