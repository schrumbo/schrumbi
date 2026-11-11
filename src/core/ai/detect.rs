use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::RwLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::core::git::cmd::spawn;
use crate::core::prefs;

/// The name under which the agent sees Schrumbi's tools.
pub const SERVER_NAME: &str = "schrumbi";

/// A program that can drive the assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Agent {
    Claude,
    Codex,
    OpenCode,
}

impl Agent {
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::OpenCode];

    /// The command on `PATH`.
    pub fn command(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::OpenCode => "opencode",
        }
    }

    /// The everyday name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::OpenCode => "OpenCode",
        }
    }
}

/// An installed program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub agent: Agent,
    pub path: PathBuf,
}

static FOUND: RwLock<Option<Vec<Cli>>> = RwLock::new(None);

/// The program `name` on `PATH`; on Windows also its `.exe`, `.cmd` or `.bat`.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        if cfg!(windows) {
            ["exe", "cmd", "bat"].iter().map(|ext| dir.join(format!("{name}.{ext}"))).find(|p| p.is_file())
        } else {
            Some(dir.join(name)).filter(|p| p.is_file())
        }
    })
}

/// Folders where installers put command-line programs that a terminal finds through the shell profile.
fn install_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = [".local/bin", ".claude/local", ".npm-global/bin", ".bun/bin", ".volta/bin", ".opencode/bin", ".cargo/bin", "Library/pnpm"].iter().map(|d| home.join(d)).collect();
    dirs.extend(nvm_dirs(home));
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/home/linuxbrew/.linuxbrew/bin"].map(PathBuf::from));
    dirs
}

fn version_key(name: &str) -> Vec<u32> {
    name.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// nvm's Node folders, the default alias first, then newest first.
fn nvm_dirs(home: &Path) -> Vec<PathBuf> {
    let mut names: Vec<String> = std::fs::read_dir(home.join(".nvm/versions/node")).into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str().map(String::from)).collect();
    names.sort_by_key(|n| std::cmp::Reverse(version_key(n)));
    let alias = std::fs::read_to_string(home.join(".nvm/alias/default")).unwrap_or_default();
    let alias = alias.trim().trim_start_matches('v');
    if !alias.is_empty() && alias.starts_with(|c: char| c.is_ascii_digit()) {
        let wanted = version_key(alias);
        if let Some(i) = names.iter().position(|n| version_key(n).starts_with(&wanted)) {
            let name = names.remove(i);
            names.insert(0, name);
        }
    }
    names.into_iter().map(|n| home.join(".nvm/versions/node").join(n).join("bin")).collect()
}

/// `PATH` with the usual install folders appended that exist and are missing; an empty `PATH` starts from the system folders.
pub fn widened_path(current: &std::ffi::OsStr, home: &Path) -> Option<std::ffi::OsString> {
    let mut paths: Vec<PathBuf> = std::env::split_paths(current).filter(|p| !p.as_os_str().is_empty()).collect();
    if paths.is_empty() {
        paths = vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin"), PathBuf::from("/usr/sbin"), PathBuf::from("/sbin")];
    }
    let before = if current.is_empty() { 0 } else { paths.len() };
    for dir in install_dirs(home) {
        if !paths.contains(&dir) && dir.is_dir() {
            paths.push(dir);
        }
    }
    (paths.len() > before).then(|| std::env::join_paths(paths).ok()).flatten()
}

/// Apps started from the Finder or a desktop launcher get a minimal `PATH` without the folders the shell profile adds,
/// so `claude`, `npx` and `node` would be missing; call before any thread starts.
pub fn widen_path() {
    if !cfg!(unix) {
        return;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    if let Some(path) = widened_path(&std::env::var_os("PATH").unwrap_or_default(), &home) {
        unsafe { std::env::set_var("PATH", path) };
    }
}

/// Looks for the programs on `PATH`; a few file checks, no process starts.
pub fn scan() -> Vec<Cli> {
    Agent::ALL.into_iter().filter_map(|agent| which(agent.command()).map(|path| Cli { agent, path })).collect()
}

/// Scans again and remembers the result.
pub fn refresh() -> Vec<Cli> {
    let found = scan();
    *FOUND.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(found.clone());
    found
}

/// What the last scan found; empty before the first scan.
pub fn found() -> Vec<Cli> {
    FOUND.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone().unwrap_or_default()
}

/// Whether the assistant is on: switched on and at least one program installed or an own command set.
pub fn active() -> bool {
    prefs::with(|p| p.ai.enabled && (!p.ai.custom_command.trim().is_empty() || !found().is_empty()))
}

/// A command for `cli` with the given arguments; `.cmd` shims on Windows run through `cmd`.
fn command(cli: &Cli, args: &[&str]) -> Command {
    let shim = cli.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if cfg!(windows) && shim {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(&cli.path).args(args);
        c
    } else {
        let mut c = Command::new(&cli.path);
        c.args(args);
        c
    }
}

/// The arguments that register Schrumbi's tools with the program, after the program itself; `None` when it has no such command.
pub fn register_args(agent: Agent, exe: &Path) -> Option<Vec<String>> {
    let exe = exe.to_string_lossy().to_string();
    match agent {
        Agent::Claude => Some(vec!["mcp".into(), "add".into(), "--scope".into(), "user".into(), SERVER_NAME.into(), "--".into(), exe, "mcp".into()]),
        Agent::Codex => Some(vec!["mcp".into(), "add".into(), SERVER_NAME.into(), "--".into(), exe, "mcp".into()]),
        Agent::OpenCode => None,
    }
}

fn quoted(arg: &str) -> String {
    if arg.contains([' ', '"']) { format!("\"{}\"", arg.replace('"', "\\\"")) } else { arg.to_string() }
}

/// The registration as a line to paste into a terminal; OpenCode gets its config snippet instead.
pub fn register_text(agent: Agent, exe: &Path) -> String {
    match register_args(agent, exe) {
        Some(args) => std::iter::once(agent.command().to_string()).chain(args.into_iter().map(|a| quoted(&a))).collect::<Vec<_>>().join(" "),
        None => format!("{{\"mcp\": {{\"{SERVER_NAME}\": {{\"type\": \"local\", \"command\": [{}, \"mcp\"]}}}}}}", serde_json::Value::String(exe.to_string_lossy().to_string())),
    }
}

/// The server command on its own, for any other program.
pub fn server_text(exe: &Path) -> String {
    format!("{} mcp", quoted(&exe.to_string_lossy()))
}

/// Registers Schrumbi's tools with the program.
pub fn register(cli: &Cli, exe: &Path) -> Result<(), String> {
    let args = register_args(cli.agent, exe).ok_or_else(|| "no registration command".to_string())?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = spawn(command(cli, &args), None, Duration::from_secs(20)).map_err(|e| format!("{e:?}"))?;
    let said = if out.stderr.trim().is_empty() { out.stdout.trim().to_string() } else { out.stderr.trim().to_string() };
    if out.code == 0 || said.to_lowercase().contains("already exists") { Ok(()) } else { Err(said) }
}

/// The path of this app's executable, as the registration should spell it.
pub fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok().map(|p| dunce::canonicalize(&p).unwrap_or(p))
}

fn status_args(agent: Agent) -> Option<&'static [&'static str]> {
    match agent {
        Agent::Claude => Some(&["auth", "status"]),
        Agent::Codex => Some(&["login", "status"]),
        Agent::OpenCode => None,
    }
}

fn login_args(agent: Agent) -> &'static [&'static str] {
    match agent {
        Agent::Claude => &["auth", "login"],
        Agent::Codex => &["login"],
        Agent::OpenCode => &["auth", "login"],
    }
}

/// Whether the user is signed in to the program; `None` when it cannot tell.
pub fn logged_in(cli: &Cli) -> Option<bool> {
    let args = status_args(cli.agent)?;
    let out = spawn(command(cli, args), None, Duration::from_secs(8)).ok()?;
    Some(out.code == 0)
}

/// Opens the system terminal with the program's sign-in command.
pub fn open_login(cli: &Cli, root: &Path) -> std::io::Result<()> {
    let line = std::iter::once(quoted(&cli.path.to_string_lossy())).chain(login_args(cli.agent).iter().map(|a| (*a).to_string())).collect::<Vec<_>>().join(" ");
    let mut c = if cfg!(target_os = "macos") {
        let mut c = Command::new("osascript");
        c.arg("-e").arg(format!("tell application \"Terminal\" to do script \"{}\"", line.replace('\\', "\\\\").replace('"', "\\\"")));
        c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg("start").arg("").arg("/D").arg(root).arg("cmd").arg("/K").arg(&line);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        c
    } else {
        let mut c = Command::new("x-terminal-emulator");
        c.arg("-e").arg(&line).current_dir(root);
        c
    };
    c.spawn().map(drop)
}

/// Opens the system terminal in `root`.
pub fn open_terminal(root: &Path) -> std::io::Result<()> {
    let mut c = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-a").arg("Terminal").arg(root);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg("start").arg("").arg("/D").arg(root).arg("cmd");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        c
    } else {
        let mut c = Command::new("x-terminal-emulator");
        c.current_dir(root);
        c
    };
    c.spawn().map(drop)
}
