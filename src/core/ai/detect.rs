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

/// Looks for the programs on `PATH`; a few file checks, no process starts.
pub fn scan() -> Vec<Cli> {
    let Some(path) = std::env::var_os("PATH") else { return Vec::new() };
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    let mut out = Vec::new();
    for agent in Agent::ALL {
        let found = dirs.iter().find_map(|dir| {
            if cfg!(windows) {
                ["exe", "cmd", "bat"].iter().map(|ext| dir.join(format!("{}.{ext}", agent.command()))).find(|p| p.is_file())
            } else {
                Some(dir.join(agent.command())).filter(|p| p.is_file())
            }
        });
        if let Some(path) = found {
            out.push(Cli { agent, path });
        }
    }
    out
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

/// Whether the assistant is on: switched on and at least one program installed.
pub fn active() -> bool {
    prefs::with(|p| p.ai.enabled) && !found().is_empty()
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
