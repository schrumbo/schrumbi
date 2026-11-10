use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
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
    session: String,
    tool: String,
    preview: Preview,
    reply: async_channel::Sender<Answer>,
}

/// A waiting permission request as the column shows it.
#[derive(Clone)]
pub struct Waiting {
    pub id: u64,
    pub preview: Preview,
}

/// What the column should do next, queued from menus, the palette or shortcuts.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Focus,
    Chip(super::ai_panel::Chip),
    Send(String),
}

/// How the user answered a write of the agent.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Allow,
    Today,
    Decline,
    Timeout,
}

/// Where the download of the Claude adapter stands.
#[derive(Clone, PartialEq)]
pub enum Download {
    Running(f32),
    Failed(String),
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
    requests: Vec<Request>,
    panel: Option<WeakEntity<super::ai_panel::AiPanel>>,
    download: Option<Download>,
    logins: HashMap<Agent, bool>,
}

impl Global for Ai {}

static COLUMN: AtomicBool = AtomicBool::new(false);

/// The page and selection of the PDF shown last while the column was open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfFocus {
    pub path: PathBuf,
    pub pages: usize,
    pub page: usize,
    pub selection: Option<((usize, usize), (usize, usize))>,
    pub text: String,
}

static PDF_FOCUS: Mutex<Option<PdfFocus>> = Mutex::new(None);

/// Whether the column is open; cheap enough for every frame.
pub fn column_open() -> bool {
    COLUMN.load(Ordering::Relaxed)
}

/// Remembers the page and selection a PDF view shows while the column is open; the selection's text is read only when it changed.
pub fn note_pdf(path: &std::path::Path, pages: usize, page: usize, selection: Option<((usize, usize), (usize, usize))>, text: impl FnOnce() -> String) -> bool {
    let mut focus = PDF_FOCUS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let same = focus.as_ref().is_some_and(|f| f.path == path && f.selection == selection);
    if same && focus.as_ref().is_some_and(|f| f.page == page && f.pages == pages) {
        return false;
    }
    let text = if same { focus.as_ref().map(|f| f.text.clone()).unwrap_or_default() } else if selection.is_some() { text() } else { String::new() };
    *focus = Some(PdfFocus { path: path.to_path_buf(), pages, page, selection, text });
    true
}

/// Redraws the column, e.g. after the PDF page under it changed.
pub fn refresh_panel(cx: &mut App) {
    if let Some(panel) = cx.try_global::<Ai>().and_then(|ai| ai.panel.clone()).and_then(|p| p.upgrade()) {
        panel.update(cx, |_, cx| cx.notify());
    }
}

/// What `note_pdf` saw last for `path`.
pub fn pdf_focus(path: &std::path::Path) -> Option<PdfFocus> {
    PDF_FOCUS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().filter(|f| f.path == path)
}

/// „KI ›“ in a file's context menu, only while the assistant is active.
pub fn file_submenu(menu: gpui_kit::component::menu::PopupMenu, path: &std::path::Path, window: &mut Window, cx: &mut Context<gpui_kit::component::menu::PopupMenu>) -> gpui_kit::component::menu::PopupMenu {
    if !active() {
        return menu;
    }
    let path = path.to_path_buf();
    menu.separator().submenu_with_icon(Some(menu_icon("bot")), tr("KI"), window, cx, move |m, _, _| {
        let (ask, cards, sum) = (path.clone(), path.clone(), path.clone());
        let pdf = sum.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        let text = crate::core::markdown::is_markdown(&cards);
        let m = m.item(gpui_kit::component::menu::PopupMenuItem::new(tr("Frag die KI zu dieser Datei")).on_click(move |_, _, cx| request(Request::Chip(super::ai_panel::file_chip(&ask)), cx)));
        let m = if pdf || text {
            m.item(gpui_kit::component::menu::PopupMenuItem::new(tr("Karten daraus machen")).on_click(move |_, _, cx| ask_with(tr("Mach Karteikarten aus dieser Datei und ergänze sie in der passenden Notiz."), Some(super::ai_panel::file_chip(&cards)), cx)))
        } else {
            m
        };
        m.item(gpui_kit::component::menu::PopupMenuItem::new(tr("Zusammenfassen")).on_click(move |_, _, cx| ask_with(tr("Fass diese Datei kurz zusammen."), Some(super::ai_panel::file_chip(&sum)), cx)))
    })
}

/// „KI ›“ in a deadline's context menu, only while the assistant is active.
pub fn deadline_submenu(menu: gpui_kit::component::menu::PopupMenu, id: &str, window: &mut Window, cx: &mut Context<gpui_kit::component::menu::PopupMenu>) -> gpui_kit::component::menu::PopupMenu {
    if !active() {
        return menu;
    }
    let id = id.to_string();
    menu.separator().submenu_with_icon(Some(menu_icon("bot")), tr("KI"), window, cx, move |m, _, _| {
        let (plan, ask) = (id.clone(), id.clone());
        m.item(gpui_kit::component::menu::PopupMenuItem::new(tr("Mit KI planen")).on_click(move |_, _, cx| ask_with(tr("Hilf mir, diese Abgabe zu planen: was ist zu tun und bis wann? Leg nach meinem Ja Aufgaben an."), Some(super::ai_panel::Chip::Deadline(plan.clone())), cx)))
            .item(gpui_kit::component::menu::PopupMenuItem::new(tr("Frag die KI dazu")).on_click(move |_, _, cx| request(Request::Chip(super::ai_panel::Chip::Deadline(ask.clone())), cx)))
    })
}

/// Opens or closes the column; closing hands the place back to Details.
pub fn toggle_column(cx: &mut App) {
    if column_open() { close_column(cx) } else { open_column(cx) }
}

/// Opens the column and gives its input the keyboard.
pub fn open_column(cx: &mut App) {
    request(Request::Focus, cx);
}

/// Shows or hides Details; while the column is open it gives the place back to Details first.
pub fn toggle_details(cx: &mut App) {
    if column_open() {
        close_column(cx);
        if !state::read(cx, |l| l.show_inspector()) {
            act(cx, |l| l.toggle_inspector());
        }
    } else {
        act(cx, |l| l.toggle_inspector());
    }
}

/// Closes the column; a running session keeps going until it idles out.
pub fn close_column(cx: &mut App) {
    COLUMN.store(false, Ordering::Relaxed);
    cx.refresh_windows();
}

/// Queues something for the column and opens it.
pub fn request(r: Request, cx: &mut App) {
    if !active() || !cx.has_global::<Ai>() {
        return;
    }
    COLUMN.store(true, Ordering::Relaxed);
    cx.global_mut::<Ai>().requests.push(r);
    if let Some(panel) = cx.global::<Ai>().panel.clone().and_then(|p| p.upgrade()) {
        panel.update(cx, |_, cx| cx.notify());
    }
    cx.refresh_windows();
}

/// Opens the column and sends `prompt` at once, the given context chip added first.
pub fn ask_with(prompt: String, chip: Option<super::ai_panel::Chip>, cx: &mut App) {
    if let Some(chip) = chip {
        request(Request::Chip(chip), cx);
    }
    request(Request::Send(prompt), cx);
}

/// Takes the queued requests for the column.
pub fn take_requests(cx: &mut App) -> Vec<Request> {
    if cx.has_global::<Ai>() { std::mem::take(&mut cx.global_mut::<Ai>().requests) } else { Vec::new() }
}

/// Remembers the column view so tool calls can report their edits to it.
pub fn set_panel(panel: WeakEntity<super::ai_panel::AiPanel>, cx: &mut App) {
    if cx.has_global::<Ai>() {
        cx.global_mut::<Ai>().panel = Some(panel);
    }
}

/// The permission requests waiting for an answer, oldest first.
pub fn waiting(cx: &App) -> Vec<Waiting> {
    cx.try_global::<Ai>().map(|ai| ai.asks.iter().map(|a| Waiting { id: a.id, preview: a.preview.clone() }).collect()).unwrap_or_default()
}

/// Asks the user about a change the agent wants to make, in the column or the floating card; `key` names the kind of change for „Immer heute“.
pub fn ask_change(key: &str, preview: Preview, cx: &mut App) -> (Option<u64>, async_channel::Receiver<Answer>) {
    if allowed_today(key, cx) {
        let (tx, rx) = async_channel::bounded(1);
        let _ = tx.try_send(Answer::Allow);
        return (None, rx);
    }
    let rx = ask(key, preview, cx);
    (Some(cx.global::<Ai>().next), rx)
}

/// Runs a change for a column session as one step of its turn.
pub fn run_for(session: &str, cx: &mut App, change: impl FnOnce(&mut crate::core::library::Library) -> Result<tools::Outcome, String>) -> Result<Value, String> {
    let mut turns = std::mem::take(&mut cx.global_mut::<Ai>().turns);
    let out = act(cx, |l| turns.run(l, session, change));
    cx.global_mut::<Ai>().turns = turns;
    out
}

/// Starts a turn with a real end for a column session.
pub fn begin_turn(session: &str, cx: &mut App) {
    if cx.has_global::<Ai>() {
        cx.global_mut::<Ai>().turns.begin(session);
    }
}

/// Ends the turn of a column session; its next change is a new undo step.
pub fn end_turn(session: &str, cx: &mut App) {
    if cx.has_global::<Ai>() {
        cx.global_mut::<Ai>().turns.end(session);
    }
}

/// Forgets a column session that ended.
pub fn close_session(session: &str, cx: &mut App) {
    if cx.has_global::<Ai>() {
        cx.global_mut::<Ai>().turns.close(session);
    }
}

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
    scan(false, cx);
}

/// Looks again and asks each program whether the user is signed in; for the settings.
pub fn recheck(cx: &mut App) {
    scan(true, cx);
}

fn scan(logins: bool, cx: &mut App) {
    if !cx.has_global::<Ai>() || std::mem::replace(&mut cx.global_mut::<Ai>().scanning, true) {
        return;
    }
    let scan = cx.background_executor().spawn(async { detect::refresh() });
    cx.spawn(async move |cx| {
        let found = scan.await;
        cx.update(|cx| {
            cx.global_mut::<Ai>().scanning = false;
            sync(cx);
        });
        if !logins || !cx.update(|_| prefs::with(|p| p.ai.enabled)) {
            return;
        }
        for cli in found.into_iter().filter(|c| c.agent != Agent::OpenCode) {
            let agent = cli.agent;
            let state = cx.background_executor().spawn(async move { detect::logged_in(&cli) }).await;
            cx.update(|cx| {
                match state {
                    Some(on) => cx.global_mut::<Ai>().logins.insert(agent, on),
                    None => cx.global_mut::<Ai>().logins.remove(&agent),
                };
                cx.refresh_windows();
            });
        }
    })
    .detach();
}

/// Whether the user is signed in to `agent`; `None` while unknown.
pub fn login_state(agent: Agent, cx: &App) -> Option<bool> {
    cx.try_global::<Ai>().and_then(|ai| ai.logins.get(&agent).copied())
}

/// Opens the system terminal with the program's sign-in; the tab checks again when it is shown next.
pub fn login(agent: Agent, cx: &mut App) {
    let Some(cli) = detect::found().into_iter().find(|c| c.agent == agent) else { return };
    let root = state::read(cx, |l| l.root.clone()).or_else(dirs::home_dir).unwrap_or_default();
    if crate::core::drive::fenced("login", agent.label()) {
        return;
    }
    let _ = detect::open_login(&cli, &root);
    if cx.has_global::<Ai>() {
        cx.global_mut::<Ai>().logins.remove(&agent);
    }
}

/// Where the adapter download stands, if one ran.
pub fn download(cx: &App) -> Option<Download> {
    cx.try_global::<Ai>().and_then(|ai| ai.download.clone())
}

/// Downloads the pinned Claude adapter in the background, with progress for the settings row.
pub fn install_claude(cx: &mut App) {
    if !cx.has_global::<Ai>() || matches!(cx.global::<Ai>().download, Some(Download::Running(_))) {
        return;
    }
    cx.global_mut::<Ai>().download = Some(Download::Running(0.));
    cx.refresh_windows();
    let (tx, rx) = async_channel::unbounded::<f32>();
    let job = cx.background_executor().spawn(async move { crate::core::ai::install::install_claude(move |p| { let _ = tx.try_send(p); }) });
    cx.spawn(async move |cx| {
        while let Ok(mut p) = rx.recv().await {
            while let Ok(next) = rx.try_recv() {
                p = next;
            }
            cx.update(|cx| {
                cx.global_mut::<Ai>().download = Some(Download::Running(p));
                cx.refresh_windows();
            });
            cx.background_executor().timer(Duration::from_millis(250)).await;
        }
        let result = job.await;
        cx.update(|cx| {
            cx.global_mut::<Ai>().download = result.err().map(Download::Failed);
            cx.refresh_windows();
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
        COLUMN.store(false, Ordering::Relaxed);
        if let Some(panel) = cx.global::<Ai>().panel.clone().and_then(|p| p.upgrade()) {
            panel.update(cx, |p, cx| p.stop(cx));
        }
    }
    if want != running {
        super::app::bind_keys(cx);
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
            let answer = cx.update(|cx| ask_for(session, name, preview, cx));
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
    let write = tools::is_write(name);
    let before = if write { state::read(cx, |l| tools::target(l, name, args).map(|p| (p.clone(), tools::text_now(l, &p)))) } else { None };
    let mut turns = std::mem::take(&mut cx.global_mut::<Ai>().turns);
    let out = if write || name == "open" {
        act(cx, |l| turns.call(l, session, name, args))
    } else {
        state::lib(cx).update(cx, |l, _| turns.call(l, session, name, args))
    };
    cx.global_mut::<Ai>().turns = turns;
    if write && let Ok(value) = &out {
        let made = value["path"].as_str().and_then(|r| state::read(cx, |l| l.root.as_ref().map(|root| root.join(r))));
        let (path, old) = match before {
            Some((p, old)) => (Some(p), old),
            None => (made, None),
        };
        let panel = cx.global::<Ai>().panel.clone().and_then(|p| p.upgrade());
        if let (Some(path), Some(panel)) = (path, panel)
            && let Some(new) = state::read(cx, |l| tools::text_now(l, &path))
        {
            panel.update(cx, |p, cx| p.record_edit(session, path, old, new, cx));
        }
    }
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
    ask_for("", name, preview, cx)
}

/// Declines every waiting change of `session`, e.g. when its turn was stopped.
pub fn decline_session(session: &str, cx: &mut App) {
    let ids: Vec<u64> = cx.try_global::<Ai>().map(|ai| ai.asks.iter().filter(|a| a.session == session).map(|a| a.id).collect()).unwrap_or_default();
    for id in ids {
        answer_ask(id, Answer::Decline, cx);
    }
}

fn ask_for(session: &str, name: &str, preview: Preview, cx: &mut App) -> async_channel::Receiver<Answer> {
    let (reply, answer) = async_channel::bounded(1);
    let ai = cx.global_mut::<Ai>();
    ai.next += 1;
    let id = ai.next;
    ai.asks.push_back(Ask { id, session: session.to_string(), tool: name.to_string(), preview, reply });
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

/// The toolbar button while the assistant is active; it opens and closes the column.
pub fn toolbar_button(t: &Theme) -> Option<Button> {
    toolbar_button_shown().then(|| {
        let open = column_open();
        let tip = format!("{}{}", if open { tr("KI ausblenden") } else { tr("KI einblenden") }, crate::core::shortcuts::ShortcutAction::ToggleAi.hint());
        Button::new("ai-button").ghost().size(px(control::M)).rounded_full().bg(t.ink.opacity(if open { 0.14 } else { 0.06 })).border_1().border_color(t.ink.opacity(0.1)).icon(menu_icon("bot")).tooltip(tip).on_click(|_, _, cx| toggle_column(cx))
    })
}

/// The card that asks before the agent changes something; it never takes the focus.
pub fn ask_card(cx: &App, t: &Theme) -> Option<Stateful<Div>> {
    if column_open() {
        return None;
    }
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
