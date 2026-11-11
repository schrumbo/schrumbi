use std::path::PathBuf;

use serde_json::json;

use super::detect::{self, Agent};
use super::install;
use crate::core::prefs::AiPrefs;

/// The Claude adapter through Node when there is no download for this platform.
pub const CLAUDE_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp@0.104.1";
/// The Codex adapter, which needs Node.
pub const CODEX_PACKAGE: &str = "@agentclientprotocol/codex-acp@2.6.0";

/// What the agent is told for every session in the column.
pub const SESSION_PROMPT: &str = "You run inside Schrumbi, the user's study planner, in a side column next to their notes and PDFs. \
For every change in the study folder use the schrumbi-app tools (write_note, add_cards, create_note, add_deadline, set_status, file_document): \
they ask the user and are undoable. Never edit files directly and never run shell commands. Call get_context first. \
Keep answers short and in the language get_context reports.";

/// Who runs in the column: one of the known programs or the user's own command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Runner {
    Known(Agent),
    Custom,
}

impl Runner {
    /// The name in settings and the column header.
    pub fn label(self) -> &'static str {
        match self {
            Runner::Known(a) => a.label(),
            Runner::Custom => "Eigener Befehl",
        }
    }

    /// How the choice is stored in the settings.
    pub fn key(self) -> &'static str {
        match self {
            Runner::Known(Agent::Claude) => "claude",
            Runner::Known(Agent::Codex) => "codex",
            Runner::Known(Agent::OpenCode) => "opencode",
            Runner::Custom => "custom",
        }
    }
}

/// What went wrong with an agent, read from its error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trouble {
    /// The user is not signed in with the provider.
    SignIn,
    /// The provider cannot be reached.
    Offline,
    /// The provider is overloaded or the account hit its limit for now.
    Busy,
    Other,
}

/// Sorts an agent's error message into what the user can do about it.
pub fn trouble(message: &str) -> Trouble {
    let m = message.to_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| m.contains(w));
    if any(&["auth_required", "authentication required", "not logged in", "not authenticated", "please log in", "please login", "/login", "unauthorized", "invalid api key", "401"]) {
        Trouble::SignIn
    } else if any(&["enotfound", "econnrefused", "econnreset", "etimedout", "eai_again", "fetch failed", "network", "getaddrinfo", "connection refused", "connection error", "socket hang up", "offline"]) {
        Trouble::Offline
    } else if any(&["rate limit", "rate_limit", "overloaded", "usage limit", "too many requests", "429", "529", "quota"]) {
        Trouble::Busy
    } else {
        Trouble::Other
    }
}

/// How to start an agent process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub runner: Runner,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Session options only Claude's adapter reads: it may not edit files or run commands itself.
    pub claude: bool,
    /// Session modes that make the agent ask before changes, best first.
    pub modes: Vec<String>,
    /// The agent starts only in one of `modes`.
    pub require_mode: bool,
}

/// Why an agent cannot start yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Missing {
    /// The Claude adapter is neither downloaded nor reachable through Node.
    Adapter,
    /// The Codex adapter needs Node.
    Node,
    /// The program is not installed.
    Program,
    /// The own command is empty or cannot be read.
    Command,
}

/// Splits a command line at spaces; double quotes keep spaces and backslashes stay as they are, so Windows paths survive.
pub fn split_command(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut started) = (false, false);
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// The programs the column can run now, in the order they are offered.
pub fn runners(prefs: &AiPrefs) -> Vec<Runner> {
    let mut out: Vec<Runner> = detect::found().iter().map(|c| Runner::Known(c.agent)).collect();
    if !prefs.custom_command.trim().is_empty() {
        out.push(Runner::Custom);
    }
    out
}

/// The runner the user chose, or the first one there is.
pub fn chosen(prefs: &AiPrefs) -> Option<Runner> {
    let all = runners(prefs);
    all.iter().copied().find(|r| r.key() == prefs.agent).or_else(|| all.first().copied())
}

fn cli(agent: Agent) -> Option<PathBuf> {
    detect::found().into_iter().find(|c| c.agent == agent).map(|c| c.path)
}

/// Variables a parent Claude Code session sets; a nested Claude refuses to start or joins that session when they are inherited.
const INHERITED: [&str; 9] = ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_SESSION_ATTENDED", "CLAUDE_CODE_EXECPATH", "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_CODE_MESSAGING_TOKEN", "CLAUDE_PID"];

fn cleared() -> Vec<(String, String)> {
    INHERITED.iter().filter(|k| std::env::var_os(k).is_some()).map(|k| ((*k).to_string(), String::new())).collect()
}

/// How to start `runner` with these settings.
pub fn launch(runner: Runner, prefs: &AiPrefs) -> Result<Launch, Missing> {
    match runner {
        Runner::Known(Agent::Claude) => {
            let mut env = cleared();
            if let Some(cli) = cli(Agent::Claude).filter(|p| p.extension().is_none_or(|e| e.eq_ignore_ascii_case("exe"))) {
                env.push(("CLAUDE_CODE_EXECUTABLE".into(), cli.to_string_lossy().to_string()));
            }
            if let Some(adapter) = install::claude_adapter() {
                return Ok(Launch { runner, program: adapter, args: Vec::new(), env, claude: true, modes: claude_modes(), require_mode: false });
            }
            let npx = detect::which("npx").ok_or(Missing::Adapter)?;
            Ok(Launch { runner, program: npx, args: vec!["-y".into(), CLAUDE_PACKAGE.into()], env, claude: true, modes: claude_modes(), require_mode: false })
        }
        Runner::Known(Agent::Codex) => {
            let npx = detect::which("npx").ok_or(Missing::Node)?;
            Ok(Launch { runner, program: npx, args: vec!["-y".into(), CODEX_PACKAGE.into()], env: codex_env(), claude: false, modes: vec![CODEX_SAFE_MODE.into()], require_mode: true })
        }
        Runner::Known(Agent::OpenCode) => {
            let program = cli(Agent::OpenCode).ok_or(Missing::Program)?;
            Ok(Launch { runner, program, args: vec!["acp".into()], env: opencode_env(prefs.allow_execute), claude: false, modes: Vec::new(), require_mode: false })
        }
        Runner::Custom => {
            let mut parts = split_command(&prefs.custom_command).into_iter().peekable();
            let mut env = Vec::new();
            while let Some((k, v)) = parts.peek().and_then(|p| p.split_once('=')).filter(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).map(|(k, v)| (k.to_string(), v.to_string())) {
                env.push((k, v));
                parts.next();
            }
            let program = parts.next().ok_or(Missing::Command)?;
            let claude = program.contains("claude-agent-acp") || parts.peek().is_some_and(|p| p.contains("claude-agent-acp"));
            let mut args: Vec<String> = parts.collect();
            if claude {
                env.extend(cleared());
            }
            args.retain(|a| !a.is_empty());
            Ok(Launch { runner, program: PathBuf::from(program), args, env, claude, modes: if claude { claude_modes() } else { Vec::new() }, require_mode: false })
        }
    }
}

/// The Codex mode in which it changes no file and runs commands only read-only; Schrumbi starts Codex only in it.
pub const CODEX_SAFE_MODE: &str = "read-only";

fn claude_modes() -> Vec<String> {
    vec!["default".into()]
}

/// What Codex's adapter starts with: its read-only mode, also before the session switches to it.
pub fn codex_env() -> Vec<(String, String)> {
    vec![("INITIAL_AGENT_MODE".into(), CODEX_SAFE_MODE.into())]
}

/// What OpenCode starts with: its own file edits switched off, so changes go through Schrumbi's tools; the shell only when allowed, and then it asks.
pub fn opencode_env(allow_execute: bool) -> Vec<(String, String)> {
    let permission = json!({ "edit": "deny", "bash": if allow_execute { "ask" } else { "deny" }, "webfetch": "ask" });
    vec![("OPENCODE_PERMISSION".into(), permission.to_string()), ("OPENCODE_CONFIG_CONTENT".into(), json!({ "permission": permission }).to_string())]
}

/// The options Claude's adapter takes with a new session: no file edits or shell of its own, Schrumbi's prompt appended.
pub fn claude_meta(allow_execute: bool) -> serde_json::Map<String, serde_json::Value> {
    let mut blocked = vec!["Edit", "Write", "MultiEdit", "NotebookEdit"];
    if !allow_execute {
        blocked.extend(["Bash", "PowerShell", "BashOutput", "KillShell"]);
    }
    let meta = json!({ "systemPrompt": { "append": SESSION_PROMPT }, "claudeCode": { "options": { "disallowedTools": blocked } } });
    meta.as_object().cloned().unwrap_or_default()
}

/// The arguments and variables that start Schrumbi's tool server for a session of this app.
pub fn server_command(session: &str) -> (Vec<String>, Vec<(String, String)>) {
    let mut env = vec![("SCHRUMBI_AI_SESSION".to_string(), session.to_string())];
    if let Some(home) = std::env::var_os("SCHRUMBI_HOME") {
        env.push(("SCHRUMBI_HOME".into(), home.to_string_lossy().to_string()));
    }
    (vec!["mcp".into(), "--instance".into(), std::process::id().to_string()], env)
}
