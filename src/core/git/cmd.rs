use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long local commands may take.
pub const LOCAL: Duration = Duration::from_secs(30);
/// How long commands that talk to the remote may take.
pub const NETWORK: Duration = Duration::from_secs(120);

/// Flags and subcommands that could destroy work; `Git` refuses them whatever the caller asks for.
const FORBIDDEN_FLAGS: [&str; 3] = ["--force", "--force-with-lease", "--hard"];
const FORBIDDEN_COMMANDS: [&str; 4] = ["clean", "stash", "rebase", "filter-branch"];

/// What went wrong, sorted by what the user can do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    Missing,
    TooOld(String),
    Offline,
    Auth,
    NoRemote,
    NoIdentity,
    Locked,
    Rejected,
    Detached,
    ExternalMerge,
    Conflicts(Vec<String>),
    Timeout,
    Other(String),
}

/// A finished process. A non-zero `code` is not an error here, some commands use it as an answer.
#[derive(Debug, Clone)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Sorts Git's English error text; `Git` forces that language with `LC_ALL=C`.
pub fn classify(stderr: &str) -> GitError {
    let has = |needles: &[&str]| needles.iter().any(|n| stderr.contains(n));
    if has(&["index.lock", "Another git process"]) {
        GitError::Locked
    } else if has(&["Please tell me who you are", "empty ident name", "unable to auto-detect email"]) {
        GitError::NoIdentity
    } else if has(&["[rejected]", "non-fast-forward", "fetch first", "failed to push some refs"]) {
        GitError::Rejected
    } else if has(&["Permission denied", "Authentication failed", "could not read Username", "could not read Password", "Host key verification failed", "terminal prompts disabled", "Repository not found"]) {
        GitError::Auth
    } else if has(&["Could not resolve host", "Network is unreachable", "Connection timed out", "Operation timed out", "Connection refused", "No route to host", "Failed to connect", "Connection reset", "Connection closed"]) {
        GitError::Offline
    } else {
        GitError::Other(stderr.trim().to_string())
    }
}

/// Runs `exe`, kills it after `timeout` and reads its pipes on their own threads, so full pipes never block it.
pub fn spawn(mut command: Command, input: Option<Vec<u8>>, timeout: Duration) -> Result<Output, GitError> {
    command.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { GitError::Missing } else { GitError::Other(e.to_string()) })?;
    if let (Some(mut stdin), Some(input)) = (child.stdin.take(), input) {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let stdout = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let stderr = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitError::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(GitError::Other(e.to_string())),
        }
    };
    Ok(Output { code: status.code().unwrap_or(-1), stdout: stdout.join().unwrap_or_default(), stderr: stderr.join().unwrap_or_default() })
}

/// The system's Git, working on one folder.
#[derive(Debug, Clone)]
pub struct Git {
    pub exe: PathBuf,
    pub root: PathBuf,
}

impl Git {
    pub fn new(exe: PathBuf, root: &Path) -> Self {
        Self { exe, root: root.to_path_buf() }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(&self.exe);
        c.current_dir(&self.root)
            .args(["-c", "core.quotePath=false", "-c", "core.fsmonitor=false"])
            .args(args)
            .env("LC_ALL", "C")
            .env("LANGUAGE", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_MERGE_AUTOEDIT", "no")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE");
        if let Some(path) = extended_path() {
            c.env("PATH", path);
        }
        c
    }

    fn refuse(args: &[&str]) -> Result<(), GitError> {
        let command = args.iter().find(|a| !a.starts_with('-'));
        let flag = args.iter().find(|a| FORBIDDEN_FLAGS.contains(a));
        match flag.or(command.filter(|c| FORBIDDEN_COMMANDS.contains(c))) {
            Some(a) => Err(GitError::Other(format!("refused: {a}"))),
            None => Ok(()),
        }
    }

    /// Runs a command and returns its output whatever the exit code.
    pub fn output(&self, args: &[&str], input: Option<Vec<u8>>, timeout: Duration) -> Result<Output, GitError> {
        Self::refuse(args)?;
        spawn(self.command(args), input, timeout)
    }

    /// Runs a local command; a non-zero exit code becomes an error.
    pub fn run(&self, args: &[&str]) -> Result<String, GitError> {
        self.checked(args, None, LOCAL)
    }

    /// Runs a command that talks to the remote, without ever asking on a terminal.
    pub fn run_network(&self, args: &[&str]) -> Result<String, GitError> {
        Self::refuse(args)?;
        let mut c = self.command(args);
        if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() && self.run(&["config", "--get", "core.sshCommand"]).is_err() {
            c.env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=15");
        }
        let out = spawn(c, None, NETWORK)?;
        if out.code == 0 { Ok(out.stdout) } else { Err(classify(&out.stderr)) }
    }

    pub fn checked(&self, args: &[&str], input: Option<Vec<u8>>, timeout: Duration) -> Result<String, GitError> {
        let out = self.output(args, input, timeout)?;
        if out.code == 0 {
            Ok(out.stdout)
        } else {
            Err(classify(if out.stderr.trim().is_empty() { &out.stdout } else { &out.stderr }))
        }
    }

    /// Git's own folder for this working tree.
    pub fn git_dir(&self) -> PathBuf {
        self.root.join(".git")
    }
}

/// Apps started from the Finder get a minimal `PATH`, so hooks and `ssh` would not find what the terminal finds.
fn extended_path() -> Option<std::ffi::OsString> {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<PathBuf> = std::env::split_paths(&current).collect();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let extra = PathBuf::from(extra);
        if cfg!(unix) && !paths.contains(&extra) && extra.is_dir() {
            paths.push(extra);
        }
    }
    std::env::join_paths(paths).ok()
}
