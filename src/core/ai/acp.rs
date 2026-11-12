use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    CancelNotification, ClientCapabilities, ContentBlock, FileSystemCapabilities, InitializeRequest, McpServer, McpServerStdio, Meta, NewSessionRequest, PermissionOption, PermissionOptionKind, PromptRequest, ReadTextFileRequest, ReadTextFileResponse, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome, SessionNotification, SessionUpdate, SetSessionModeRequest, StopReason, TextContent, ToolCallContent, ToolCallLocation, ToolCallStatus, ToolKind, WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::{Agent, Client, ConnectTo};
use futures::future::Either;
use serde_json::Value;

/// The name under which the agent sees the tools Schrumbi hands it with each session.
pub const OWN_SERVER: &str = "schrumbi-app";

/// What Schrumbi lets an agent do without asking the user.
#[derive(Debug, Clone)]
pub struct Policy {
    pub root: PathBuf,
    pub allow_execute: bool,
    /// The agent writes files only through the client (`fs/write_text_file`), as Claude's adapter does; any other agent's own edits are refused.
    pub client_edits: bool,
}

/// How a permission request of the agent is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Reject,
    Ask,
}

/// What kind of work a tool call does, for its icon and title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    Other,
}

/// Where a tool call stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Running,
    Done,
    Failed,
}

/// A change to a text file the agent shows before or after making it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub path: PathBuf,
    pub old: Option<String>,
    pub new: String,
}

/// The fields of a tool call that one update sets; unset fields stay as they were.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolPatch {
    pub name: Option<String>,
    pub title: Option<String>,
    pub kind: Option<Kind>,
    pub status: Option<Status>,
    pub input: Option<Value>,
    pub output: Option<String>,
    pub diffs: Option<Vec<Diff>>,
    pub locations: Option<Vec<(PathBuf, Option<u32>)>>,
}

/// What the agent streams during a turn, reduced to what the column shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Text(String),
    Tool { id: String, new: bool, patch: ToolPatch },
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Done,
    Cancelled,
    Limit,
    Refused,
}

/// A permission request that needs the user's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    pub tool: String,
    pub patch: ToolPatch,
}

/// An answer that must be given exactly once; dropping it unanswered answers with the fallback.
pub struct Reply<T>(Option<Box<dyn FnOnce(Option<T>) + Send>>);

impl<T> Reply<T> {
    pub fn new(f: impl FnOnce(Option<T>) + Send + 'static) -> Self {
        Self(Some(Box::new(f)))
    }

    /// Sends the answer.
    pub fn send(mut self, value: T) {
        if let Some(f) = self.0.take() {
            f(Some(value));
        }
    }
}

impl<T> Drop for Reply<T> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f(None);
        }
    }
}

/// What the connection reports to the column.
pub enum Event {
    Ready,
    Update(Update),
    Ask { ask: Ask, reply: Reply<bool> },
    Read { path: PathBuf, line: Option<u32>, limit: Option<u32>, reply: Reply<Result<String, String>> },
    Write { path: PathBuf, content: String, reply: Reply<Result<(), String>> },
    /// A tool call refused without asking, e.g. a shell command while those are off.
    Denied { tool: String },
    /// The agent offers none of the modes that make it ask before changes; Schrumbi ends the session before the first prompt.
    Unsafe,
    TurnEnded(Result<Stop, String>),
}

/// What the column asks of the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Prompt(String),
    Cancel,
}

/// How to open a session.
pub struct Setup {
    pub cwd: PathBuf,
    pub mcp: Option<McpServerStdio>,
    pub meta: Option<Meta>,
    /// Session modes that make the agent ask before changes, best first, e.g. Claude's „default“ or Codex's „read-only“.
    pub modes: Vec<String>,
    /// Without one of `modes` the agent could change files on its own, so the session ends at once.
    pub require_mode: bool,
    pub policy: Policy,
}

/// Whether the tool belongs to the server Schrumbi hands the agent; those calls ask in Schrumbi itself.
pub fn is_own_tool(name: &str) -> bool {
    name.strip_prefix("mcp__").and_then(|rest| rest.split_once("__")).is_some_and(|(server, _)| server == OWN_SERVER || server == super::detect::SERVER_NAME)
}

fn own_server_meta(meta: Option<&Meta>) -> bool {
    meta.and_then(|m| m.get("claudeCode")).and_then(|c| c.get("mcpServer")).and_then(|s| s.get("name")).and_then(Value::as_str).is_some_and(|n| n == OWN_SERVER || n == super::detect::SERVER_NAME)
}

fn settled(p: &Path) -> PathBuf {
    dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Whether `path` lies inside `root`, also for files that do not exist yet; `..` never counts as inside.
pub fn inside(root: &Path, path: &Path) -> bool {
    if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return false;
    }
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name().map(std::ffi::OsStr::to_os_string) else { return false };
        rest.push(name);
        if !existing.pop() {
            return false;
        }
    }
    let mut full = settled(&existing);
    for name in rest.into_iter().rev() {
        full.push(name);
    }
    full.starts_with(settled(root))
}

fn kind_of(kind: ToolKind) -> Kind {
    match kind {
        ToolKind::Read => Kind::Read,
        ToolKind::Edit => Kind::Edit,
        ToolKind::Delete => Kind::Delete,
        ToolKind::Move => Kind::Move,
        ToolKind::Search => Kind::Search,
        ToolKind::Execute => Kind::Execute,
        ToolKind::Think => Kind::Think,
        ToolKind::Fetch => Kind::Fetch,
        _ => Kind::Other,
    }
}

fn status_of(status: ToolCallStatus) -> Status {
    match status {
        ToolCallStatus::InProgress => Status::Running,
        ToolCallStatus::Completed => Status::Done,
        ToolCallStatus::Failed => Status::Failed,
        _ => Status::Pending,
    }
}

const SHELLS: [&str; 4] = ["Bash", "PowerShell", "shell", "execute"];

/// Decides a permission request: Schrumbi's own tools ask in Schrumbi, reads inside the folder pass, shell commands follow the setting,
/// deleting and moving are refused, edits ask only for agents that write through Schrumbi, the rest asks.
pub fn decide(request: &RequestPermissionRequest, policy: &Policy) -> Decision {
    let call = &request.tool_call;
    let name = call.fields.name.as_deref().unwrap_or_default();
    if is_own_tool(name) || own_server_meta(call.meta.as_ref()) {
        return Decision::Allow;
    }
    let kind = call.fields.kind.map(kind_of).unwrap_or(if SHELLS.contains(&name) { Kind::Execute } else { Kind::Other });
    match kind {
        Kind::Execute if policy.allow_execute => Decision::Ask,
        Kind::Execute | Kind::Delete | Kind::Move => Decision::Reject,
        Kind::Edit if !policy.client_edits => Decision::Reject,
        Kind::Read | Kind::Search | Kind::Think => {
            let mut places: Vec<PathBuf> = call.fields.locations.as_deref().unwrap_or_default().iter().map(|l| l.path.clone()).collect();
            places.extend(named_paths(call.fields.raw_input.as_ref()));
            let known = !places.is_empty() || kind != Kind::Read;
            if known && places.iter().all(|p| inside(&policy.root, &policy.root.join(p))) { Decision::Allow } else { Decision::Ask }
        }
        _ => Decision::Ask,
    }
}

/// The paths a tool's input names: its path fields, and patterns that start at a root or step up; relative ones count from the study folder.
fn named_paths(input: Option<&Value>) -> Vec<PathBuf> {
    let Some(input) = input.and_then(Value::as_object) else { return Vec::new() };
    let expand = |s: &str| match s.strip_prefix('~') {
        Some(rest) => dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")).join(rest.trim_start_matches(['/', '\\'])),
        None => PathBuf::from(s),
    };
    let mut out: Vec<PathBuf> = ["path", "file_path", "notebook_path", "directory", "dir"].iter().filter_map(|k| input.get(*k).and_then(Value::as_str)).filter(|s| !s.trim().is_empty()).map(expand).collect();
    for key in ["pattern", "glob"] {
        if let Some(p) = input.get(key).and_then(Value::as_str) {
            let path = Path::new(p);
            let rooted = path.has_root() || p.starts_with('~') || matches!(p.as_bytes(), [l, b':', ..] if l.is_ascii_alphabetic());
            if rooted || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                out.push(expand(p));
            }
        }
    }
    out
}

/// The answer to send: allow once, never „always“ (Claude would write that rule into the user's folder), or reject.
pub fn answer(options: &[PermissionOption], allow: bool) -> RequestPermissionResponse {
    let wanted: &[PermissionOptionKind] = if allow { &[PermissionOptionKind::AllowOnce] } else { &[PermissionOptionKind::RejectOnce, PermissionOptionKind::RejectAlways] };
    let chosen = wanted.iter().find_map(|k| options.iter().find(|o| o.kind == *k));
    match chosen {
        Some(o) => RequestPermissionResponse::new(RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(o.option_id.clone()))),
        None => RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
    }
}

fn text_of(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

fn output_of(content: &[ToolCallContent]) -> Option<String> {
    let text: Vec<String> = content
        .iter()
        .filter_map(|c| match c {
            ToolCallContent::Content(c) => text_of(&c.content),
            _ => None,
        })
        .collect();
    (!text.is_empty()).then(|| text.join("\n"))
}

fn diffs_of(content: &[ToolCallContent]) -> Vec<Diff> {
    content
        .iter()
        .filter_map(|c| match c {
            ToolCallContent::Diff(d) => Some(Diff { path: d.path.clone(), old: d.old_text.clone(), new: d.new_text.clone() }),
            _ => None,
        })
        .collect()
}

fn places(locations: &[ToolCallLocation]) -> Vec<(PathBuf, Option<u32>)> {
    locations.iter().map(|l| (l.path.clone(), l.line)).collect()
}

fn patch_of_update(fields: &agent_client_protocol::v1::schema::ToolCallUpdateFields) -> ToolPatch {
    ToolPatch {
        name: fields.name.clone(),
        title: fields.title.clone(),
        kind: fields.kind.map(kind_of),
        status: fields.status.map(status_of),
        input: fields.raw_input.clone(),
        output: fields.content.as_deref().and_then(output_of),
        diffs: fields.content.as_deref().map(diffs_of).filter(|d| !d.is_empty()),
        locations: fields.locations.as_deref().map(places),
    }
}

/// Reduces an agent's session update to what the column shows; the rest is ignored.
pub fn translate(update: SessionUpdate) -> Option<Update> {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => text_of(&chunk.content).map(Update::Text),
        SessionUpdate::ToolCall(call) => Some(Update::Tool {
            id: call.tool_call_id.0.to_string(),
            new: true,
            patch: ToolPatch {
                name: call.name,
                title: Some(call.title),
                kind: Some(kind_of(call.kind)),
                status: Some(status_of(call.status)),
                input: call.raw_input,
                output: output_of(&call.content),
                diffs: Some(diffs_of(&call.content)).filter(|d| !d.is_empty()),
                locations: Some(places(&call.locations)),
            },
        }),
        SessionUpdate::ToolCallUpdate(update) => Some(Update::Tool { id: update.tool_call_id.0.to_string(), new: false, patch: patch_of_update(&update.fields) }),
        _ => None,
    }
}

fn stop_of(reason: StopReason) -> Stop {
    match reason {
        StopReason::Cancelled => Stop::Cancelled,
        StopReason::Refusal => Stop::Refused,
        StopReason::EndTurn => Stop::Done,
        _ => Stop::Limit,
    }
}

fn fs_error(message: String) -> agent_client_protocol::Error {
    agent_client_protocol::Error::new(-32002, message)
}

/// Runs one agent session over `transport` until the column drops its command sender or the agent ends.
pub async fn run(transport: impl ConnectTo<Client> + 'static, setup: Setup, events: async_channel::Sender<Event>, commands: async_channel::Receiver<Command>) -> Result<(), String> {
    let policy = setup.policy.clone();
    let (on_update, on_ask, on_read, on_write) = (events.clone(), events.clone(), events.clone(), events.clone());
    Client
        .v1()
        .name("schrumbi")
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                if let Some(update) = translate(notification.update) {
                    let _ = on_update.try_send(Event::Update(update));
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                let tool = request.tool_call.tool_call_id.0.to_string();
                match decide(&request, &policy) {
                    Decision::Allow => responder.respond(answer(&request.options, true)),
                    Decision::Reject => {
                        let _ = on_ask.try_send(Event::Denied { tool });
                        responder.respond(answer(&request.options, false))
                    }
                    Decision::Ask => {
                        let options = request.options.clone();
                        let reply = Reply::new(move |allowed: Option<bool>| {
                            let response = match allowed {
                                Some(allow) => answer(&options, allow),
                                None => RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                            };
                            let _ = responder.respond(response);
                        });
                        let _ = on_ask.try_send(Event::Ask { ask: Ask { tool, patch: patch_of_update(&request.tool_call.fields) }, reply });
                        Ok(())
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ReadTextFileRequest, responder, _connection| {
                let reply = Reply::new(move |result: Option<Result<String, String>>| {
                    let _ = match result {
                        Some(Ok(text)) => responder.respond(ReadTextFileResponse::new(text)),
                        Some(Err(e)) => responder.respond_with_error(fs_error(e)),
                        None => responder.respond_with_error(fs_error("Schrumbi closed the session.".into())),
                    };
                });
                let _ = on_read.try_send(Event::Read { path: request.path, line: request.line, limit: request.limit, reply });
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: WriteTextFileRequest, responder, _connection| {
                let reply = Reply::new(move |result: Option<Result<(), String>>| {
                    let _ = match result {
                        Some(Ok(())) => responder.respond(WriteTextFileResponse::new()),
                        Some(Err(e)) => responder.respond_with_error(fs_error(e)),
                        None => responder.respond_with_error(fs_error("Schrumbi closed the session.".into())),
                    };
                });
                let _ = on_write.try_send(Event::Write { path: request.path, content: request.content, reply });
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |connection: ConnectionTo<Agent>| session(connection, setup, events, commands).await)
        .await
        .map_err(|e| e.to_string())
}

async fn session(connection: ConnectionTo<Agent>, setup: Setup, events: async_channel::Sender<Event>, commands: async_channel::Receiver<Command>) -> Result<(), agent_client_protocol::Error> {
    let caps = ClientCapabilities::new().fs(FileSystemCapabilities::new().read_text_file(true).write_text_file(true));
    connection.send_request(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(caps)).block_task().await?;
    let mut request = NewSessionRequest::new(setup.cwd).mcp_servers(setup.mcp.into_iter().map(McpServer::Stdio).collect());
    if let Some(meta) = setup.meta {
        request = request.meta(meta);
    }
    let session = connection.send_request(request).block_task().await?;
    let id = session.session_id.clone();
    let offered = |mode: &String| session.modes.as_ref().is_some_and(|m| m.available_modes.iter().any(|x| *x.id.0 == **mode));
    match setup.modes.iter().find(|m| offered(m)) {
        Some(mode) if session.modes.as_ref().is_some_and(|m| *m.current_mode_id.0 != **mode) => {
            connection.send_request(SetSessionModeRequest::new(id.clone(), mode.clone())).block_task().await?;
        }
        Some(_) => {}
        None if setup.require_mode => {
            let _ = events.try_send(Event::Unsafe);
            return Ok(());
        }
        None => {}
    }
    let _ = events.try_send(Event::Ready);
    let mut queue = VecDeque::new();
    loop {
        let text = match queue.pop_front() {
            Some(text) => text,
            None => match commands.recv().await {
                Ok(Command::Prompt(text)) => text,
                Ok(Command::Cancel) => continue,
                Err(_) => return Ok(()),
            },
        };
        let mut prompt = Box::pin(connection.send_request(PromptRequest::new(id.clone(), vec![ContentBlock::Text(TextContent::new(text))])).block_task());
        let result = loop {
            let next = Box::pin(commands.recv());
            match futures::future::select(prompt, next).await {
                Either::Left((result, _)) => break result,
                Either::Right((command, pending)) => {
                    prompt = pending;
                    match command {
                        Ok(Command::Cancel) => connection.send_notification(CancelNotification::new(id.clone()))?,
                        Ok(Command::Prompt(text)) => queue.push_back(text),
                        Err(_) => {
                            let _ = connection.send_notification(CancelNotification::new(id.clone()));
                            return Ok(());
                        }
                    }
                }
            }
        };
        let _ = events.try_send(Event::TurnEnded(result.map(|r| stop_of(r.stop_reason)).map_err(|e| e.to_string())));
    }
}
