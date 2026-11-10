use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use chrono::{Local, NaiveDate};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::{Value, json};

use super::state::{self, act};
use super::theme::{Theme, control, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::ai::detect::{self, Agent};
use crate::core::ai::instance::{self, Instance};
use crate::core::ai::tools::{self, Preview};
use crate::core::ai::turn::Turns;
use crate::core::i18n::tr;
use crate::core::prefs;

const ASK_WAIT: Duration = Duration::from_secs(600);
const OFF: &str = "The assistant is switched off in Schrumbi (Settings → KI).";
const DECLINED: &str = "The user declined this change in Schrumbi. Nothing was written.";
const UNANSWERED: &str = "Nobody answered the permission request in Schrumbi within ten minutes. Nothing was written.";

struct Server {
    port: u16,
    token: String,
    root: Option<PathBuf>,
    started: i64,
    stop: Arc<AtomicBool>,
}

struct Ask {
    id: u64,
    tool: String,
    preview: Preview,
    reply: async_channel::Sender<Answer>,
}

/// How the user answered a write of the agent.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Allow,
    Today,
    Decline,
    Timeout,
}

/// What became of an „Eintragen“ click.
#[derive(Clone, PartialEq, Eq)]
pub enum Registration {
    Running,
    Done,
    Failed(String),
}

#[derive(Default)]
struct Ai {
    server: Option<Server>,
    turns: Turns,
    allowed: Option<(NaiveDate, HashSet<String>)>,
    asks: VecDeque<Ask>,
    next: u64,
    scanning: bool,
    registrations: HashMap<Agent, Registration>,
}

impl Global for Ai {}

struct Call {
    body: Value,
    reply: mpsc::Sender<Value>,
}

/// Looks for the programs in the background and starts the local channel when the assistant is active.
pub fn init(cx: &mut App) {
    cx.set_global(Ai::default());
    cx.on_app_quit(|cx| {
        if let Some(server) = cx.global_mut::<Ai>().server.take() {
            stop(&server);
        }
        async {}
    })
    .detach();
    rescan(cx);
}

/// Looks for Claude Code, Codex and OpenCode again: a few file checks off the UI thread.
pub fn rescan(cx: &mut App) {
    if !cx.has_global::<Ai>() || std::mem::replace(&mut cx.global_mut::<Ai>().scanning, true) {
        return;
    }
    let scan = cx.background_executor().spawn(async { detect::refresh() });
    cx.spawn(async move |cx| {
        scan.await;
        cx.update(|cx| {
            cx.global_mut::<Ai>().scanning = false;
            sync(cx);
        });
    })
    .detach();
}

/// Whether the assistant is active: switched on and a supported program installed.
pub fn active() -> bool {
    detect::active()
}

/// Switches the assistant on or off.
pub fn set_enabled(on: bool, cx: &mut App) {
    prefs::update(|p| p.ai.enabled = on);
    sync(cx);
}

/// What the last „Eintragen“ for `agent` did.
pub fn registration(agent: Agent, cx: &App) -> Option<Registration> {
    cx.try_global::<Ai>().and_then(|ai| ai.registrations.get(&agent).cloned())
}

/// Registers Schrumbi's tools with the program, in the background.
pub fn register(cli: detect::Cli, cx: &mut App) {
    let Some(exe) = detect::exe() else { return };
    let agent = cli.agent;
    let own_config = match agent {
        Agent::Claude => std::env::var_os("CLAUDE_CONFIG_DIR").is_some(),
        Agent::Codex => std::env::var_os("CODEX_HOME").is_some(),
        Agent::OpenCode => false,
    };
    if !own_config && crate::core::drive::fenced("register", detect::register_text(agent, &exe)) {
        return;
    }
    cx.global_mut::<Ai>().registrations.insert(agent, Registration::Running);
    cx.refresh_windows();
    let job = cx.background_executor().spawn(async move { detect::register(&cli, &exe) });
    cx.spawn(async move |cx| {
        let result = job.await;
        cx.update(|cx| {
            cx.global_mut::<Ai>().registrations.insert(agent, match result {
                Ok(()) => Registration::Done,
                Err(e) => Registration::Failed(e),
            });
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Starts or ends the local channel so it runs exactly while the assistant is active.
fn sync(cx: &mut App) {
    let want = active();
    let running = cx.global::<Ai>().server.is_some();
    if want && !running {
        let root = state::read(cx, |l| l.root.clone());
        cx.global_mut::<Ai>().server = start(root, cx);
    } else if !want && running {
        if let Some(server) = cx.global_mut::<Ai>().server.take() {
            stop(&server);
        }
        let asks: Vec<Ask> = cx.global_mut::<Ai>().asks.drain(..).collect();
        for ask in asks {
            let _ = ask.reply.try_send(Answer::Decline);
        }
    }
    cx.refresh_windows();
}

fn start(root: Option<PathBuf>, cx: &mut App) -> Option<Server> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let started = chrono::Utc::now().timestamp();
    instance::write(&Instance { pid: std::process::id(), port, token: token.clone(), root: root.clone(), started }).ok()?;
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = async_channel::unbounded::<Call>();
    {
        let (stop, token) = (stop.clone(), token.clone());
        let spawned = std::thread::Builder::new().name("ai-server".into()).spawn(move || {
            for stream in listener.incoming() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let (tx, stop, token) = (tx.clone(), stop.clone(), token.clone());
                let _ = std::thread::Builder::new().name("ai-client".into()).spawn(move || serve(stream, &token, &stop, &tx));
            }
        });
        if spawned.is_err() {
            instance::remove(std::process::id());
            return None;
        }
    }
    cx.spawn(async move |cx| {
        while let Ok(call) = rx.recv().await {
            cx.spawn(async move |cx| {
                let out = respond(&call.body, cx).await;
                let _ = call.reply.send(out);
            })
            .detach();
        }
    })
    .detach();
    Some(Server { port, token, root, started, stop })
}

fn stop(server: &Server) {
    server.stop.store(true, Ordering::Relaxed);
    let _ = TcpStream::connect(("127.0.0.1", server.port));
    instance::remove(std::process::id());
}

fn serve(stream: TcpStream, token: &str, stop: &AtomicBool, tx: &async_channel::Sender<Call>) {
    let Ok(mut writer) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let out = match serde_json::from_str::<Value>(&line) {
            _ if stop.load(Ordering::Relaxed) => json!({ "ok": false, "error": OFF }),
            Err(e) => json!({ "ok": false, "error": format!("bad request: {e}") }),
            Ok(body) if body["token"].as_str() != Some(token) => json!({ "ok": false, "error": "wrong token" }),
            Ok(body) => {
                let (reply, answer) = mpsc::channel();
                if tx.send_blocking(Call { body, reply }).is_err() {
                    json!({ "ok": false, "error": OFF })
                } else {
                    answer.recv().unwrap_or_else(|_| json!({ "ok": false, "error": OFF }))
                }
            }
        };
        if writeln!(writer, "{out}").and_then(|()| writer.flush()).is_err() {
            break;
        }
    }
}

async fn respond(body: &Value, cx: &mut AsyncApp) -> Value {
    let session = body["session"].as_str().unwrap_or_default().to_string();
    let name = body["tool"].as_str().unwrap_or_default().to_string();
    let args = body.get("args").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    match run(&session, &name, &args, cx).await {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

async fn run(session: &str, name: &str, args: &Value, cx: &mut AsyncApp) -> Result<Value, String> {
    if !cx.update(|cx| active() && cx.global::<Ai>().server.is_some()) {
        return Err(OFF.into());
    }
    cx.update(follow_root);
    if tools::is_write(name) {
        let preview = cx.update(|cx| state::read(cx, |l| tools::preview(l, name, args)))?;
        if !cx.update(|cx| allowed_today(name, cx)) {
            let answer = cx.update(|cx| ask(name, preview, cx));
            match answer.recv().await.unwrap_or(Answer::Decline) {
                Answer::Allow | Answer::Today => {}
                Answer::Decline => {
                    cx.update(|cx| cx.global_mut::<Ai>().turns.end(session));
                    return Err(DECLINED.into());
                }
                Answer::Timeout => return Err(UNANSWERED.into()),
            }
            if !cx.update(|_| active()) {
                return Err(OFF.into());
            }
        }
    }
    cx.update(|cx| call(session, name, args, cx))
}

fn call(session: &str, name: &str, args: &Value, cx: &mut App) -> Result<Value, String> {
    let mut turns = std::mem::take(&mut cx.global_mut::<Ai>().turns);
    let out = if tools::is_write(name) || name == "open" {
        act(cx, |l| turns.call(l, session, name, args))
    } else {
        state::lib(cx).update(cx, |l, _| turns.call(l, session, name, args))
    };
    cx.global_mut::<Ai>().turns = turns;
    out
}

fn follow_root(cx: &mut App) {
    let root = state::read(cx, |l| l.root.clone());
    let Some(server) = cx.global_mut::<Ai>().server.as_mut() else { return };
    if server.root != root {
        server.root = root.clone();
        let _ = instance::write(&Instance { pid: std::process::id(), port: server.port, token: server.token.clone(), root, started: server.started });
    }
}

fn allowed_today(name: &str, cx: &App) -> bool {
    let today = Local::now().date_naive();
    cx.global::<Ai>().allowed.as_ref().is_some_and(|(day, tools)| *day == today && tools.contains(name))
}

fn ask(name: &str, preview: Preview, cx: &mut App) -> async_channel::Receiver<Answer> {
    let (reply, answer) = async_channel::bounded(1);
    let ai = cx.global_mut::<Ai>();
    ai.next += 1;
    let id = ai.next;
    ai.asks.push_back(Ask { id, tool: name.to_string(), preview, reply });
    cx.refresh_windows();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(ASK_WAIT).await;
        cx.update(|cx| answer_ask(id, Answer::Timeout, cx));
    })
    .detach();
    answer
}

/// Answers the waiting write `id`; „Immer heute“ lets this kind of change through until midnight.
pub fn answer_ask(id: u64, answer: Answer, cx: &mut App) {
    let ai = cx.global_mut::<Ai>();
    let Some(at) = ai.asks.iter().position(|a| a.id == id) else { return };
    let Some(ask) = ai.asks.remove(at) else { return };
    if answer == Answer::Today {
        let today = Local::now().date_naive();
        let allowed = ai.allowed.get_or_insert_with(|| (today, HashSet::new()));
        if allowed.0 != today {
            *allowed = (today, HashSet::new());
        }
        allowed.1.insert(ask.tool.clone());
    }
    let _ = ask.reply.try_send(answer);
    cx.refresh_windows();
}

/// Whether the toolbar shows the assistant's button.
pub fn toolbar_button_shown() -> bool {
    active() && prefs::with(|p| p.ai.toolbar_button)
}

/// The toolbar button while the assistant is active; it opens Einstellungen → KI.
pub fn toolbar_button() -> Option<Button> {
    toolbar_button_shown().then(|| {
        Button::new("ai-button").ghost().size(px(control::M)).rounded_full().icon(menu_icon("bot")).tooltip(tr("KI")).on_click(|_, _, cx| super::settings::open_ai(cx))
    })
}

/// The card that asks before the agent changes something; it never takes the focus.
pub fn ask_card(cx: &App, t: &Theme) -> Option<Stateful<Div>> {
    let ai = cx.try_global::<Ai>()?;
    let ask = ai.asks.front()?;
    let (id, more) = (ask.id, ai.asks.len() - 1);
    let mut lines = div().flex().flex_col().gap(px(space::XXS)).font_family(Theme::mono_family()).text_size(px(font::CAPTION)).text_color(t.secondary);
    for line in &ask.preview.lines {
        lines = lines.child(div().truncate().child(line.clone()));
    }
    let shown = !ask.preview.lines.is_empty();
    Some(
        super::drive::marked(div().id("ai-ask"), "dialog")
            .occlude()
            .absolute()
            .right(px(space::L))
            .bottom(px(space::L))
            .w(px(380.))
            .p(px(space::L))
            .rounded(px(radius::CARD))
            .bg(t.material)
            .border_1()
            .border_color(t.separator)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(space::M))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .child(icon("bot", iconsize::S, t.secondary))
                    .child(div().flex_1().text_size(px(font::CAPTION)).text_color(t.secondary).child(tr("Die KI möchte etwas ändern")))
                    .when(more > 0, |d| d.child(caption(format!("+{more}"), t))),
            )
            .child(div().font_weight(FontWeight::SEMIBOLD).child(ask.preview.title.clone()))
            .when(shown, |d| d.child(div().p(px(space::S)).rounded(px(radius::INNER)).bg(t.ink.opacity(0.05)).child(lines)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(space::S))
                    .child(Button::new("ai-decline").ghost().small().label(tr("Ablehnen")).on_click(move |_, _, cx| answer_ask(id, Answer::Decline, cx)))
                    .child(Button::new("ai-today").small().label(tr("Immer heute")).tooltip(tr("Erlaubt diese Art Änderung bis Mitternacht ohne Nachfrage")).on_click(move |_, _, cx| answer_ask(id, Answer::Today, cx)))
                    .child(Button::new("ai-allow").primary().small().label(tr("Erlauben")).on_click(move |_, _, cx| answer_ask(id, Answer::Allow, cx))),
            ),
    )
}
