use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse, McpServer, McpServerStdio, NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest, PromptResponse, ReadTextFileRequest, RequestPermissionOutcome, RequestPermissionRequest, SessionId, SessionMode, SessionModeState, SessionNotification, SessionUpdate, SetSessionModeRequest, SetSessionModeResponse, StopReason, TextContent, ToolCall, ToolCallLocation, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind, WriteTextFileRequest,
};
use agent_client_protocol::{Agent, Channel, Client};

use super::acp::{self, Command, Decision, Event, Kind, Policy, Setup, Status, Stop, Update};

type Log = Arc<Mutex<Vec<String>>>;

fn note(log: &Log, line: String) {
    log.lock().unwrap().push(line);
}

fn options() -> Vec<PermissionOption> {
    vec![PermissionOption::new("allow", "Yes", PermissionOptionKind::AllowOnce), PermissionOption::new("always", "Always", PermissionOptionKind::AllowAlways), PermissionOption::new("reject", "No", PermissionOptionKind::RejectOnce)]
}

fn call(id: &str, name: &str, kind: ToolKind, at: Option<&Path>) -> ToolCallUpdate {
    let mut fields = ToolCallUpdateFields::new().kind(kind).name(name.to_string()).title(name.to_string());
    if let Some(at) = at {
        fields = fields.locations(vec![ToolCallLocation::new(at)]);
    }
    ToolCallUpdate::new(id.to_string(), fields)
}

fn outcome(o: &RequestPermissionOutcome) -> String {
    match o {
        RequestPermissionOutcome::Selected(s) => s.option_id.0.to_string(),
        _ => "cancelled".into(),
    }
}

async fn ask(connection: &ConnectionTo<Client>, log: &Log, session: &SessionId, update: ToolCallUpdate) -> Result<(), agent_client_protocol::Error> {
    let id = update.tool_call_id.0.to_string();
    let response = connection.send_request(RequestPermissionRequest::new(session.clone(), update, options())).block_task().await?;
    note(log, format!("permission {id} {}", outcome(&response.outcome)));
    Ok(())
}

async fn fake_agent(channel: Channel, log: Log, root: PathBuf, modes: Vec<&'static str>) -> Result<(), agent_client_protocol::Error> {
    let (l1, l2, l3, l4, l5) = (log.clone(), log.clone(), log.clone(), log.clone(), log.clone());
    let (stop_tx, stop_rx) = async_channel::unbounded::<()>();
    Agent
        .v1()
        .name("fake")
        .on_receive_request(
            async move |r: InitializeRequest, responder, _c| {
                note(&l1, format!("init fs={}", r.client_capabilities.fs.write_text_file));
                responder.respond(InitializeResponse::new(r.protocol_version).agent_capabilities(AgentCapabilities::new()))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |r: NewSessionRequest, responder, _c| {
                let servers: Vec<String> = r.mcp_servers.iter().map(|s| if let McpServer::Stdio(s) = s { format!("{} {}", s.name, s.args.join(" ")) } else { String::new() }).collect();
                let meta = r.meta.as_ref().map(|m| m.contains_key("claudeCode")).unwrap_or(false);
                note(&l2, format!("session mcp=[{}] meta={meta}", servers.join(",")));
                responder.respond(NewSessionResponse::new(SessionId::new("s1")).modes(SessionModeState::new(modes[0], modes.iter().map(|m| SessionMode::new(*m, *m)).collect())))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |r: SetSessionModeRequest, responder, _c| {
                note(&l3, format!("mode {}", r.mode_id.0));
                responder.respond(SetSessionModeResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |_n: CancelNotification, _cx| {
                note(&l4, "cancel".into());
                let _ = stop_tx.try_send(());
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |r: PromptRequest, responder, connection| {
                let text = r.prompt.iter().filter_map(|b| if let ContentBlock::Text(t) = b { Some(t.text.clone()) } else { None }).collect::<Vec<_>>().join("");
                note(&l5, format!("prompt {text}"));
                let (log, root, stop_rx, session) = (l5.clone(), root.clone(), stop_rx.clone(), r.session_id.clone());
                let c = connection.clone();
                connection.spawn(async move {
                    let reason = match text.as_str() {
                        "hello" => {
                            for chunk in ["Hal", "lo"] {
                                c.send_notification(SessionNotification::new(session.clone(), SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(chunk))))))?;
                            }
                            c.send_notification(SessionNotification::new(session.clone(), SessionUpdate::ToolCall(ToolCall::new("t1", "Read N.md").kind(ToolKind::Read).status(ToolCallStatus::InProgress))))?;
                            c.send_notification(SessionNotification::new(session.clone(), SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("t1", ToolCallUpdateFields::new().status(ToolCallStatus::Completed)))))?;
                            StopReason::EndTurn
                        }
                        "foreign" => {
                            ask(&c, &log, &session, call("patch", "apply_patch", ToolKind::Edit, Some(&root.join("N.md")))).await?;
                            ask(&c, &log, &session, call("rm", "delete", ToolKind::Delete, Some(&root.join("N.md")))).await?;
                            ask(&c, &log, &session, call("mv", "move", ToolKind::Move, Some(&root.join("N.md")))).await?;
                            ask(&c, &log, &session, call("own", "mcp__schrumbi-app__write_note", ToolKind::Edit, None)).await?;
                            StopReason::EndTurn
                        }
                        "perms" => {
                            ask(&c, &log, &session, call("own", "mcp__schrumbi-app__add_cards", ToolKind::Other, None)).await?;
                            ask(&c, &log, &session, call("shell", "Bash", ToolKind::Execute, None)).await?;
                            ask(&c, &log, &session, call("read", "Read", ToolKind::Read, Some(&root.join("N.md")))).await?;
                            ask(&c, &log, &session, call("edit-yes", "Edit", ToolKind::Edit, Some(&root.join("N.md")))).await?;
                            ask(&c, &log, &session, call("edit-no", "Edit", ToolKind::Edit, Some(&root.join("N.md")))).await?;
                            ask(&c, &log, &session, call("edit-gone", "Edit", ToolKind::Edit, Some(&root.join("N.md")))).await?;
                            StopReason::EndTurn
                        }
                        "files" => {
                            let read = c.send_request(ReadTextFileRequest::new(session.clone(), root.join("N.md"))).block_task().await;
                            note(&log, format!("read {}", read.map(|r| r.content).unwrap_or_else(|e| format!("error {}", e.message))));
                            let write = c.send_request(WriteTextFileRequest::new(session.clone(), root.join("N.md"), "neu\n")).block_task().await;
                            note(&log, format!("write {}", write.map(|_| "ok".to_string()).unwrap_or_else(|e| format!("error {}", e.message))));
                            StopReason::EndTurn
                        }
                        "wait" => {
                            let _ = stop_rx.recv().await;
                            StopReason::Cancelled
                        }
                        _ => StopReason::EndTurn,
                    };
                    responder.respond(PromptResponse::new(reason))
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(channel)
        .await
}

fn root() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("schrumbi-acp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dunce::canonicalize(&dir).unwrap()
}

fn setup(root: &Path, allow_execute: bool) -> Setup {
    let server = McpServerStdio::new(acp::OWN_SERVER, PathBuf::from("schrumbi")).args(vec!["mcp".into(), "--instance".into(), "1".into()]);
    Setup { cwd: root.to_path_buf(), mcp: Some(server), meta: Some(super::agents::claude_meta(allow_execute)), modes: vec!["default".into()], require_mode: false, policy: Policy { root: root.to_path_buf(), allow_execute, client_edits: true } }
}

fn codex_setup(root: &Path) -> Setup {
    Setup { cwd: root.to_path_buf(), mcp: None, meta: None, modes: vec![super::agents::CODEX_SAFE_MODE.into()], require_mode: true, policy: Policy { root: root.to_path_buf(), allow_execute: false, client_edits: false } }
}

fn session<F, Fut>(allow_execute: bool, ui: F) -> Vec<String>
where
    F: FnOnce(async_channel::Receiver<Event>, async_channel::Sender<Command>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    session_as(|root| setup(root, allow_execute), vec!["auto", "default"], ui)
}

fn session_as<S, F, Fut>(make: S, modes: Vec<&'static str>, ui: F) -> Vec<String>
where
    S: FnOnce(&Path) -> Setup,
    F: FnOnce(async_channel::Receiver<Event>, async_channel::Sender<Command>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let root = root();
    let log: Log = Arc::default();
    let (client_side, agent_side) = Channel::duplex();
    let (events_tx, events_rx) = async_channel::unbounded();
    let (commands_tx, commands_rx) = async_channel::unbounded();
    let agent = fake_agent(agent_side, log.clone(), root.clone(), modes);
    let client = acp::run(client_side, make(&root), events_tx, commands_rx);
    let ui = ui(events_rx, commands_tx);
    futures::executor::block_on(async {
        let agent = Box::pin(agent);
        let both = Box::pin(async {
            let (result, ()) = futures::join!(client, ui);
            assert!(result.is_ok(), "{result:?}");
        });
        let _ = futures::future::select(both, agent).await;
    });
    let out = log.lock().unwrap().clone();
    out
}

async fn until_turn_end(events: &async_channel::Receiver<Event>, mut each: impl FnMut(Event)) -> Result<Stop, String> {
    loop {
        match events.recv().await.expect("events end") {
            Event::TurnEnded(r) => return r,
            other => each(other),
        }
    }
}

#[test]
fn a_session_opens_with_schrumbis_server_and_asking_mode_and_streams_a_turn() {
    let mut text = String::new();
    let mut tools = Vec::new();
    let mut ready = false;
    let log = session(false, |events, commands| async move {
        commands.send(Command::Prompt("hello".into())).await.unwrap();
        let stop = until_turn_end(&events, |e| match e {
            Event::Ready => ready = true,
            Event::Update(Update::Text(t)) => text.push_str(&t),
            Event::Update(Update::Tool { id, new, patch }) => tools.push((id, new, patch.status)),
            _ => {}
        })
        .await;
        assert_eq!(stop, Ok(Stop::Done));
        assert!(ready);
        assert_eq!(text, "Hallo");
        assert_eq!(tools, vec![("t1".to_string(), true, Some(Status::Running)), ("t1".to_string(), false, Some(Status::Done))]);
    });
    assert_eq!(log[0], "init fs=true");
    assert_eq!(log[1], format!("session mcp=[{} mcp --instance 1] meta=true", acp::OWN_SERVER));
    assert_eq!(log[2], "mode default");
    assert_eq!(log[3], "prompt hello");
}

#[test]
fn permissions_route_own_tools_and_reads_through_deny_shell_silently_and_ask_for_edits() {
    let mut asked = Vec::new();
    let mut denied = Vec::new();
    let log = session(false, |events, commands| async move {
        commands.send(Command::Prompt("perms".into())).await.unwrap();
        let stop = until_turn_end(&events, |e| match e {
            Event::Ask { ask, reply } => {
                asked.push(ask.tool.clone());
                assert_eq!(ask.patch.kind, Some(Kind::Edit));
                match ask.tool.as_str() {
                    "edit-yes" => reply.send(true),
                    "edit-no" => reply.send(false),
                    _ => drop(reply),
                }
            }
            Event::Denied { tool } => denied.push(tool),
            _ => {}
        })
        .await;
        assert_eq!(stop, Ok(Stop::Done));
        assert_eq!(asked, vec!["edit-yes", "edit-no", "edit-gone"]);
        assert_eq!(denied, vec!["shell"]);
    });
    let permissions: Vec<&String> = log.iter().filter(|l| l.starts_with("permission")).collect();
    assert_eq!(permissions, vec!["permission own allow", "permission shell reject", "permission read allow", "permission edit-yes allow", "permission edit-no reject", "permission edit-gone cancelled"]);
}

#[test]
fn shell_commands_ask_once_the_user_allowed_them() {
    let mut asked = Vec::new();
    session(true, |events, commands| async move {
        commands.send(Command::Prompt("perms".into())).await.unwrap();
        let _ = until_turn_end(&events, |e| {
            if let Event::Ask { ask, reply } = e {
                asked.push(ask.tool.clone());
                reply.send(false);
            }
        })
        .await;
        assert_eq!(asked.first().map(String::as_str), Some("shell"));
    });
}

#[test]
fn file_reads_and_writes_wait_for_the_column_and_carry_its_answer() {
    let log = session(false, |events, commands| async move {
        commands.send(Command::Prompt("files".into())).await.unwrap();
        let _ = until_turn_end(&events, |e| match e {
            Event::Read { reply, .. } => reply.send(Ok("alt\n".into())),
            Event::Write { content, reply, .. } => {
                assert_eq!(content, "neu\n");
                reply.send(Err("The note changed since you read it.".into()));
            }
            _ => {}
        })
        .await;
    });
    assert!(log.contains(&"read alt\n".to_string()), "{log:?}");
    assert!(log.iter().any(|l| l.starts_with("write error") && l.contains("changed")), "{log:?}");
}

#[test]
fn cancel_ends_the_turn_and_a_prompt_sent_meanwhile_waits_for_it() {
    let log = session(false, |events, commands| async move {
        commands.send(Command::Prompt("wait".into())).await.unwrap();
        commands.send(Command::Prompt("hello".into())).await.unwrap();
        commands.send(Command::Cancel).await.unwrap();
        assert_eq!(until_turn_end(&events, drop).await, Ok(Stop::Cancelled));
        assert_eq!(until_turn_end(&events, drop).await, Ok(Stop::Done));
    });
    let order: Vec<&String> = log.iter().filter(|l| l.starts_with("prompt") || *l == "cancel").collect();
    assert_eq!(order, vec!["prompt wait", "cancel", "prompt hello"]);
}

#[test]
fn reads_outside_the_folder_and_parent_steps_are_not_inside() {
    let root = root();
    assert!(acp::inside(&root, &root.join("a").join("neu.md")));
    assert!(!acp::inside(&root, &root.join("..").join("x.md")));
    assert!(!acp::inside(&root, root.parent().unwrap()));
    let request = RequestPermissionRequest::new("s", call("r", "Read", ToolKind::Read, Some(root.parent().unwrap())), options());
    assert_eq!(acp::decide(&request, &Policy { root: root.clone(), allow_execute: false, client_edits: true }), Decision::Ask);
    let shell = RequestPermissionRequest::new("s", ToolCallUpdate::new("b", ToolCallUpdateFields::new().name("Bash".to_string())), options());
    assert_eq!(acp::decide(&shell, &Policy { root, allow_execute: false, client_edits: true }), Decision::Reject);
}

#[test]
fn searches_and_reads_without_locations_are_judged_by_their_input() {
    let root = root();
    let policy = Policy { root: root.clone(), allow_execute: false, client_edits: true };
    let with = |name: &str, kind: ToolKind, input: serde_json::Value| RequestPermissionRequest::new("s", ToolCallUpdate::new("g", ToolCallUpdateFields::new().kind(kind).name(name.to_string()).raw_input(input)), options());
    let outside = root.parent().unwrap().to_string_lossy().to_string();
    assert_eq!(acp::decide(&with("Grep", ToolKind::Search, serde_json::json!({ "pattern": "x", "path": outside })), &policy), Decision::Ask);
    assert_eq!(acp::decide(&with("Glob", ToolKind::Search, serde_json::json!({ "pattern": format!("{outside}/**/*.md") })), &policy), Decision::Ask);
    assert_eq!(acp::decide(&with("Glob", ToolKind::Search, serde_json::json!({ "pattern": "../**/*" })), &policy), Decision::Ask);
    assert_eq!(acp::decide(&with("Glob", ToolKind::Search, serde_json::json!({ "pattern": "~/.ssh/*" })), &policy), Decision::Ask);
    assert_eq!(acp::decide(&with("Read", ToolKind::Read, serde_json::json!({})), &policy), Decision::Ask, "a read that names nothing asks");
    assert_eq!(acp::decide(&with("Grep", ToolKind::Search, serde_json::json!({ "pattern": "Grenzwert", "path": "MA" })), &policy), Decision::Allow);
    assert_eq!(acp::decide(&with("Grep", ToolKind::Search, serde_json::json!({ "pattern": "Grenzwert" })), &policy), Decision::Allow, "no path searches the study folder");
}

#[test]
fn an_allow_never_picks_always_so_no_rule_lands_in_the_users_folder() {
    let only_always = vec![PermissionOption::new("always", "Always", PermissionOptionKind::AllowAlways)];
    assert_eq!(outcome(&acp::answer(&only_always, true).outcome), "cancelled");
    assert_eq!(outcome(&acp::answer(&options(), true).outcome), "allow");
}

#[test]
fn own_tools_are_known_by_their_server_prefix() {
    assert!(acp::is_own_tool("mcp__schrumbi-app__add_cards"));
    assert!(acp::is_own_tool("mcp__schrumbi__read_note"));
    assert!(!acp::is_own_tool("mcp__other__add_cards"));
    assert!(!acp::is_own_tool("Edit"));
}

#[test]
fn own_commands_keep_windows_paths_and_quoted_spaces() {
    use super::agents::split_command;
    assert_eq!(split_command(r#""C:\Program Files\nodejs\node.exe" C:\a\index.js --x"#), vec![r"C:\Program Files\nodejs\node.exe", r"C:\a\index.js", "--x"]);
    assert_eq!(split_command("  opencode   acp "), vec!["opencode", "acp"]);
    assert_eq!(split_command(r#"a "" b"#), vec!["a", "", "b"]);
}

#[test]
fn an_own_command_takes_leading_variables_and_spots_the_claude_adapter() {
    let prefs = crate::core::prefs::AiPrefs { custom_command: r#"FOO=1 node "C:\x\claude-agent-acp\dist\index.js""#.into(), ..Default::default() };
    let launch = super::agents::launch(super::agents::Runner::Custom, &prefs).unwrap();
    assert_eq!(launch.program, PathBuf::from("node"));
    assert_eq!(launch.args, vec![r"C:\x\claude-agent-acp\dist\index.js"]);
    assert!(launch.env.contains(&("FOO".into(), "1".into())));
    assert!(launch.claude);
}

#[test]
fn claude_sessions_block_its_own_file_edits_and_the_shell_unless_allowed() {
    let blocked = |allow: bool| super::agents::claude_meta(allow)["claudeCode"]["options"]["disallowedTools"].as_array().unwrap().iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>();
    assert!(blocked(false).contains(&"Edit".to_string()) && blocked(false).contains(&"Bash".to_string()));
    assert!(blocked(true).contains(&"Write".to_string()) && !blocked(true).contains(&"Bash".to_string()));
}

#[test]
fn the_adapter_download_unpacks_inside_its_folder_and_refuses_escaping_entries() {
    use std::io::Write;
    let dir = root().join("unpack");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let make = |name: &str, entries: &[&str]| {
        let path = dir.join(name);
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for e in entries {
            zip.start_file(*e, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"bin").unwrap();
        }
        zip.finish().unwrap();
        path
    };
    let good = make("good.zip", &["claude-agent-acp.exe", "claude.exe"]);
    super::install::unpack(&good, &dir.join("out")).unwrap();
    assert_eq!(std::fs::read(dir.join("out").join("claude.exe")).unwrap(), b"bin");
    assert_eq!(super::install::sha256_of(&dir.join("out").join("claude.exe")).unwrap(), "51a1f05af85e342e3c849b47d387086476282d5f50dc240c19216d6edfb1eb5a");
    let bad = make("bad.zip", &["../escaped.exe"]);
    assert!(super::install::unpack(&bad, &dir.join("bad")).is_err());
    assert!(!dir.join("escaped.exe").exists());
}

#[test]
fn codex_starts_only_in_its_read_only_mode() {
    let log = session_as(codex_setup, vec!["auto", "read-only", "full-access"], |events, commands| async move {
        commands.send(Command::Prompt("hello".into())).await.unwrap();
        assert_eq!(until_turn_end(&events, drop).await, Ok(Stop::Done));
    });
    assert!(log.contains(&"mode read-only".to_string()), "{log:?}");
}

#[test]
fn an_agent_without_an_asking_mode_is_ended_before_its_first_prompt() {
    let mut unsafe_seen = false;
    let log = session_as(codex_setup, vec!["auto", "full-access"], |events, commands| async move {
        let _ = commands.send(Command::Prompt("hello".into())).await;
        while let Ok(e) = events.recv().await {
            if matches!(e, Event::Unsafe) {
                unsafe_seen = true;
            }
            assert!(!matches!(e, Event::Ready), "no session may start");
        }
        assert!(unsafe_seen);
    });
    assert!(!log.iter().any(|l| l.starts_with("prompt") || l.starts_with("mode")), "{log:?}");
}

#[test]
fn agents_that_edit_on_their_own_are_refused_and_nobody_may_delete_or_move() {
    let mut asked = Vec::new();
    let mut denied = Vec::new();
    let log = session_as(codex_setup, vec!["read-only"], |events, commands| async move {
        commands.send(Command::Prompt("foreign".into())).await.unwrap();
        let _ = until_turn_end(&events, |e| match e {
            Event::Ask { ask, reply } => {
                asked.push(ask.tool.clone());
                reply.send(true);
            }
            Event::Denied { tool } => denied.push(tool),
            _ => {}
        })
        .await;
        assert!(asked.is_empty(), "nothing is even asked: {asked:?}");
        assert_eq!(denied, vec!["patch", "rm", "mv"]);
    });
    let permissions: Vec<&String> = log.iter().filter(|l| l.starts_with("permission")).collect();
    assert_eq!(permissions, vec!["permission patch reject", "permission rm reject", "permission mv reject", "permission own allow"]);
    let claude = Policy { root: root(), allow_execute: true, client_edits: true };
    let delete = RequestPermissionRequest::new("s", call("d", "Delete", ToolKind::Delete, Some(&root().join("N.md"))), options());
    assert_eq!(acp::decide(&delete, &claude), Decision::Reject, "deleting is refused even for an agent that writes through Schrumbi");
}

#[test]
fn codex_and_opencode_start_with_their_own_edits_switched_off() {
    assert!(super::agents::codex_env().contains(&("INITIAL_AGENT_MODE".into(), "read-only".into())));
    let env = super::agents::opencode_env(false);
    let permission: serde_json::Value = serde_json::from_str(&env.iter().find(|(k, _)| k == "OPENCODE_PERMISSION").unwrap().1).unwrap();
    assert_eq!((permission["edit"].as_str(), permission["bash"].as_str()), (Some("deny"), Some("deny")));
    let config: serde_json::Value = serde_json::from_str(&env.iter().find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT").unwrap().1).unwrap();
    assert_eq!(config["permission"]["edit"], "deny");
    let shell: serde_json::Value = serde_json::from_str(&super::agents::opencode_env(true)[0].1).unwrap();
    assert_eq!((shell["edit"].as_str(), shell["bash"].as_str()), (Some("deny"), Some("ask")));
    let custom = super::agents::launch(super::agents::Runner::Custom, &crate::core::prefs::AiPrefs { custom_command: "some-agent acp".into(), ..Default::default() }).unwrap();
    assert!(!custom.claude && custom.modes.is_empty(), "an unknown agent's own edits are refused, not asked");
}
