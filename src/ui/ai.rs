use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use chrono::{Local, NaiveDate};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::base::FocusTrapElement;
use serde_json::{Value, json};

use super::motion::Soft;
use super::state::{self, act};
use super::theme::{Theme, control, font, hover, iconsize, radius, space};
use super::widgets::*;
use crate::core::ai::detect::{self, Agent};
use crate::core::ai::instance::{self, Instance};
use crate::core::ai::prompts::Flow;
use crate::core::ai::tools::{self, Preview};
use crate::core::ai::turn::Turns;
use crate::core::i18n::{tr, trf};
use crate::core::prefs;

const ASK_WAIT: Duration = Duration::from_secs(600);
const OFF: &str = "The assistant is switched off in Schrumbi (Settings → KI).";
const DECLINED: &str = "The user declined this change in Schrumbi. Nothing was written.";
const UNANSWERED: &str = "Nobody answered the permission request in Schrumbi within ten minutes. Nothing was written.";
const GONE: &str = "The caller of this tool went away before the user answered. Nothing was written.";
const LINE_LIMIT: usize = 1 << 20;
const FIRST_LINE: Duration = Duration::from_secs(30);
const PROBE: Duration = Duration::from_millis(500);
const READ_ONLY: &str = "This turn is read-only (explaining or quizzing): Schrumbi writes nothing now. The user can insert your answer into a note themselves or ask for the change in a new message.";

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
    scope: Option<Scope>,
    terminal: bool,
    preview: Preview,
    reply: async_channel::Sender<Answer>,
}

/// What „heute erlauben“ covers: one tool on one note, file or folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub key: String,
    pub folder: bool,
}

impl Scope {
    /// One tool on one path.
    pub fn of(tool: &str, path: &std::path::Path, folder: bool) -> Self {
        Self { key: format!("{tool} {}", path.display()), folder }
    }
}

/// A waiting permission request as the column shows it.
#[derive(Clone)]
pub struct Waiting {
    pub id: u64,
    pub session: String,
    pub tool: String,
    pub preview: Preview,
    /// Whether „heute erlauben“ is offered, and for a folder rather than a file.
    pub today: Option<bool>,
    /// From a program in the terminal rather than the column.
    pub terminal: bool,
}

/// What the column should do next, queued from menus, the palette or shortcuts.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Focus,
    Chip(super::ai_panel::Chip),
    Send(String),
    Flow(Flow, Vec<super::ai_panel::Chip>, Option<String>),
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
    allowed: Option<(NaiveDate, Option<PathBuf>, HashSet<String>)>,
    open_groups: HashSet<String>,
    asks: VecDeque<Ask>,
    next: u64,
    scanning: bool,
    registrations: HashMap<Agent, Registration>,
    requests: Vec<Request>,
    panel: Option<WeakEntity<super::ai_panel::AiPanel>>,
    download: Option<Download>,
    logins: HashMap<Agent, bool>,
    read_only: HashSet<String>,
    fills: HashMap<PathBuf, WeakEntity<super::filing_form::FilingForm>>,
    prefills: HashMap<PathBuf, crate::core::ai::inbox::Row>,
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
    use gpui_kit::component::menu::PopupMenuItem;
    if !active() {
        return menu;
    }
    let path = path.to_path_buf();
    menu.separator().submenu_with_icon(Some(menu_icon("bot")), tr("KI"), window, cx, move |m, _, _| {
        let pdf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        let text = crate::core::markdown::is_markdown(&path);
        let whole = super::ai_panel::whole_file_chip(&path);
        let ask = path.clone();
        let mut m = m.item(PopupMenuItem::new(tr("Frag die KI zu dieser Datei")).on_click(move |_, _, cx| request(Request::Chip(super::ai_panel::file_chip(&ask)), cx)));
        if pdf || text {
            let chip = whole.clone();
            m = m.item(PopupMenuItem::new(Flow::Cards.label()).on_click(move |_, _, cx| start_flow(Flow::Cards, vec![chip.clone()], cx)));
        }
        if pdf {
            let chip = whole.clone();
            m = m.item(PopupMenuItem::new(Flow::Summary.label()).on_click(move |_, _, cx| start_flow(Flow::Summary, vec![chip.clone()], cx)));
        }
        if text {
            let chip = whole.clone();
            m = m.item(PopupMenuItem::new(Flow::Quiz.label()).on_click(move |_, _, cx| start_flow(Flow::Quiz, vec![chip.clone()], cx)));
        }
        m
    })
}

/// „KI ›“ for PDF pages: the page under the pointer or the pages picked in the page list, with the selected text when there is one.
pub fn pdf_submenu(menu: gpui_kit::component::menu::PopupMenu, path: &std::path::Path, pages: (usize, usize), total: usize, selection: Option<String>, window: &mut Window, cx: &mut Context<gpui_kit::component::menu::PopupMenu>) -> gpui_kit::component::menu::PopupMenu {
    use gpui_kit::component::menu::PopupMenuItem;
    use super::ai_panel::Chip;
    if !active() {
        return menu;
    }
    let path = path.to_path_buf();
    let several = pages.1 > pages.0;
    let mut chips = vec![if several { Chip::Pages(path.clone(), pages.0, pages.1) } else { Chip::Pdf(path.clone(), pages.0) }];
    let selected = selection.filter(|t| !t.trim().is_empty());
    if let Some(text) = &selected {
        chips.push(Chip::Selection(text.clone()));
    }
    let whole = Chip::Pages(path, 0, total.saturating_sub(1));
    menu.separator().submenu_with_icon(Some(menu_icon("bot")), tr("KI"), window, cx, move |m, _, _| {
        let some = selected.is_some();
        let (ask, cards, explain, quiz) = (chips.clone(), chips.clone(), chips.clone(), chips.clone());
        let whole = whole.clone();
        let ask_label = if some { tr("Frag die KI zu dieser Auswahl") } else if several { tr("Frag die KI zu diesen Seiten") } else { tr("Frag die KI zu dieser Seite") };
        let cards_label = if some { tr("Karten aus der Auswahl") } else if several { tr("Karten aus diesen Seiten") } else { tr("Karten aus dieser Seite") };
        m.item(PopupMenuItem::new(ask_label).on_click(move |_, _, cx| {
            for c in ask.clone() {
                request(Request::Chip(c), cx);
            }
        }))
        .item(PopupMenuItem::new(cards_label).on_click(move |_, _, cx| start_flow(Flow::Cards, cards.clone(), cx)))
        .item(PopupMenuItem::new(if some { tr("Passage erklären") } else { Flow::Explain.label() }).on_click(move |_, _, cx| start_flow(Flow::Explain, explain.clone(), cx)))
        .item(PopupMenuItem::new(Flow::Quiz.label()).on_click(move |_, _, cx| start_flow(Flow::Quiz, quiz.clone(), cx)))
        .separator()
        .item(PopupMenuItem::new(Flow::Summary.label()).on_click(move |_, _, cx| start_flow(Flow::Summary, vec![whole.clone()], cx)))
    })
}

/// Opens the column and starts a study task with its context chips.
pub fn start_flow(flow: Flow, chips: Vec<super::ai_panel::Chip>, cx: &mut App) {
    request(Request::Flow(flow, chips, None), cx);
}

/// The filing form's „Mit KI ausfüllen“: the agent reads the file and its reading comes back into that form.
pub fn fill_filing(path: PathBuf, form: WeakEntity<super::filing_form::FilingForm>, cx: &mut App) {
    if !active() || !cx.has_global::<Ai>() {
        return;
    }
    cx.global_mut::<Ai>().fills.insert(path.clone(), form);
    request(Request::Flow(Flow::Inbox, vec![super::ai_panel::Chip::Note(path)], Some(tr("Lies diese Datei und füll das Ablegen für mich aus."))), cx);
}

/// What the agent read for a file whose filing form opens now, e.g. after „Bearbeiten“ in the column.
pub fn take_prefill(path: &std::path::Path, cx: &mut App) -> Option<crate::core::ai::inbox::Row> {
    if cx.has_global::<Ai>() { cx.global_mut::<Ai>().prefills.remove(path) } else { None }
}

/// Keeps a row for the filing form that opens next for its file.
pub fn keep_prefill(row: crate::core::ai::inbox::Row, cx: &mut App) {
    if cx.has_global::<Ai>() {
        cx.global_mut::<Ai>().prefills.insert(row.path.clone(), row);
    }
}

/// Lets every filing form waiting for the agent stop waiting, e.g. when its turn ended without an answer.
pub fn release_fills(cx: &mut App) {
    if !cx.has_global::<Ai>() {
        return;
    }
    let waiting: Vec<WeakEntity<super::filing_form::FilingForm>> = cx.global_mut::<Ai>().fills.drain().map(|(_, f)| f).collect();
    for form in waiting.into_iter().filter_map(|f| f.upgrade()) {
        form.update(cx, |f, cx| f.ai_done(cx));
    }
}

/// Makes the current turn of a column session read-only: its writes are declined without asking.
pub fn set_read_only(session: &str, on: bool, cx: &mut App) {
    if cx.has_global::<Ai>() {
        let ai = cx.global_mut::<Ai>();
        if on {
            ai.read_only.insert(session.to_string());
        } else {
            ai.read_only.remove(session);
        }
    }
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

fn set_column(on: bool) {
    COLUMN.store(on, Ordering::Relaxed);
    if prefs::with(|p| p.ai.column_open) != on {
        prefs::update(|p| p.ai.column_open = on);
    }
    if on {
        prefs::mark_used(COLUMN_USED);
    }
}

const COLUMN_USED: &str = "ai-column";

/// The KI button and its shortcut: opens the right panel on the KI tab, or closes the panel when that tab is shown.
pub fn toggle_column(cx: &mut App) {
    if column_open() { close_panel(cx) } else { open_column(cx) }
}

/// Closes the right panel, whichever tab it shows.
pub fn close_panel(cx: &mut App) {
    set_column(false);
    forget_allowed(cx);
    if state::read(cx, |l| l.show_inspector()) {
        act(cx, |l| l.toggle_inspector());
    }
    cx.refresh_windows();
}

/// Switches the right panel to its details tab, without closing it in between.
pub fn show_details(cx: &mut App) {
    set_column(false);
    if state::read(cx, |l| l.inspector_kind().is_some() && !l.show_inspector()) {
        act(cx, |l| l.toggle_inspector());
    }
    cx.refresh_windows();
}

/// The switch at the top of the right panel: „Details | KI“ where both are offered, else the name of what is shown.
pub fn panel_tabs(ki: bool, t: &Theme, cx: &App) -> AnyElement {
    let details = state::read(cx, |l| l.inspector_kind().is_some());
    if !(details && active()) {
        return div().text_size(px(font::BODY)).font_weight(FontWeight::SEMIBOLD).child(if ki { tr("KI") } else { tr("Details") }).into_any_element();
    }
    let segment = |id: &'static str, label: String, selected: bool, hint: String| {
        div()
            .id(id)
            .px(px(space::S))
            .h(px(control::S - 4.))
            .flex()
            .items_center()
            .rounded(px(radius::INNER))
            .text_size(px(font::CALLOUT))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .key_nav()
            .tip(hint)
            .when(selected, |d| d.bg(t.content).shadow_xs().text_color(t.primary))
            .when(!selected, |d| d.soft(t.ink.opacity(hover::ROW), radius::INNER).text_color(t.secondary))
            .child(label)
    };
    div()
        .flex()
        .items_center()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(radius::CONTROL))
        .bg(t.ink.opacity(0.06))
        .child(segment("panel-tab-details", tr("Details"), !ki, format!("{}{}", tr("Details"), crate::core::shortcuts::ShortcutAction::ToggleInspector.hint())).on_click(|_, _, cx| show_details(cx)))
        .child(segment("panel-tab-ai", tr("KI"), ki, format!("{}{}", tr("KI"), crate::core::shortcuts::ShortcutAction::ToggleAi.hint())).on_click(|_, _, cx| open_column(cx)))
        .into_any_element()
}

/// Opens the column and gives its input the keyboard.
pub fn open_column(cx: &mut App) {
    request(Request::Focus, cx);
}

/// Shows or hides Details; while the column is open it gives the place back to Details first.
pub fn toggle_details(cx: &mut App) {
    if column_open() {
        show_details(cx);
    } else {
        act(cx, |l| l.toggle_inspector());
    }
}

/// Closes the column; a running session keeps going until it idles out.
pub fn close_column(cx: &mut App) {
    set_column(false);
    forget_allowed(cx);
    cx.refresh_windows();
}

fn forget_allowed(cx: &mut App) {
    if cx.has_global::<Ai>() {
        cx.global_mut::<Ai>().allowed = None;
    }
}

/// Queues something for the column and opens it.
pub fn request(r: Request, cx: &mut App) {
    if !active() || !cx.has_global::<Ai>() {
        return;
    }
    set_column(true);
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
    cx.try_global::<Ai>()
        .map(|ai| ai.asks.iter().map(|a| Waiting { id: a.id, session: a.session.clone(), tool: a.tool.clone(), preview: a.preview.clone(), today: a.scope.as_ref().map(|s| s.folder), terminal: a.terminal }).collect())
        .unwrap_or_default()
}

/// The waiting requests in groups of the same tool from the same session, in the order they came; a group of several is answered together or entry by entry.
pub fn waiting_groups(cx: &App) -> Vec<Vec<Waiting>> {
    let mut groups: Vec<Vec<Waiting>> = Vec::new();
    for w in waiting(cx) {
        match groups.iter_mut().find(|g| g[0].session == w.session && g[0].tool == w.tool) {
            Some(g) => g.push(w),
            None => groups.push(vec![w]),
        }
    }
    groups
}

fn group_key(w: &Waiting) -> String {
    format!("{} {}", w.session, w.tool)
}

/// Asks the user about a change the agent wants to make, in the column or the floating card; `scope` is what „heute erlauben“ would cover, `None` offers no such button.
pub fn ask_change(tool: &str, scope: Option<Scope>, preview: Preview, cx: &mut App) -> (Option<u64>, async_channel::Receiver<Answer>) {
    if scope.as_ref().is_some_and(|s| allowed_today(s, cx)) {
        let (tx, rx) = async_channel::bounded(1);
        let _ = tx.try_send(Answer::Allow);
        return (None, rx);
    }
    let (id, rx) = ask_for("", tool, scope, false, preview, cx);
    (Some(id), rx)
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
    alive: Arc<AtomicBool>,
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

/// Whether a search for the programs is running.
pub fn scanning(cx: &App) -> bool {
    cx.try_global::<Ai>().is_some_and(|ai| ai.scanning)
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
        if prefs::with(|p| p.ai.column_open) {
            COLUMN.store(true, Ordering::Relaxed);
        }
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
                let out = respond(&call.body, &call.alive, cx).await;
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

#[derive(Debug, PartialEq)]
enum Line {
    Text(Vec<u8>),
    TooLong,
    End,
}

/// Reads one line of at most `LINE_LIMIT` bytes.
fn read_line(reader: &mut impl BufRead) -> Line {
    let mut buf = Vec::new();
    match reader.by_ref().take(LINE_LIMIT as u64 + 1).read_until(b'\n', &mut buf) {
        Ok(0) | Err(_) => Line::End,
        Ok(_) if buf.len() > LINE_LIMIT => Line::TooLong,
        Ok(_) => Line::Text(buf),
    }
}

/// Whether the other end of a waiting connection has closed it.
fn gone(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let closed = match stream.peek(&mut [0; 1]) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
    };
    let _ = stream.set_nonblocking(false);
    closed
}

/// One local connection: a caller has `FIRST_LINE` to send a line with the token, lines are capped, a wrong token closes it,
/// and while a call waits for the user the connection is watched so a card never outlives its caller.
fn serve(stream: TcpStream, token: &str, stop: &AtomicBool, tx: &async_channel::Sender<Call>) {
    let (Ok(mut writer), Ok(probe), Ok(guard)) = (stream.try_clone(), stream.try_clone(), stream.try_clone()) else { return };
    let _ = stream.set_read_timeout(Some(FIRST_LINE));
    let known = Arc::new(AtomicBool::new(false));
    let watched = known.clone();
    let _ = std::thread::Builder::new().name("ai-deadline".into()).spawn(move || {
        std::thread::sleep(FIRST_LINE);
        if !watched.load(Ordering::Relaxed) {
            let _ = guard.shutdown(std::net::Shutdown::Both);
        }
    });
    let mut reader = BufReader::new(stream);
    let mut trusted = false;
    loop {
        let line = match read_line(&mut reader) {
            Line::Text(line) => line,
            Line::TooLong => {
                let _ = writeln!(writer, "{}", json!({ "ok": false, "error": "The request is larger than 1 MB; send less text at once." })).and_then(|()| writer.flush());
                break;
            }
            Line::End => break,
        };
        let mut close = false;
        let out = match serde_json::from_slice::<Value>(&line) {
            _ if stop.load(Ordering::Relaxed) => json!({ "ok": false, "error": OFF }),
            Err(e) => {
                close = !trusted;
                json!({ "ok": false, "error": format!("bad request: {e}") })
            }
            Ok(body) if body["token"].as_str() != Some(token) => {
                close = true;
                json!({ "ok": false, "error": "wrong token" })
            }
            Ok(body) => {
                if !trusted {
                    trusted = true;
                    known.store(true, Ordering::Relaxed);
                    let _ = probe.set_read_timeout(None);
                }
                let (reply, answer) = mpsc::channel();
                let alive = Arc::new(AtomicBool::new(true));
                if tx.send_blocking(Call { body, reply, alive: alive.clone() }).is_err() {
                    json!({ "ok": false, "error": OFF })
                } else {
                    loop {
                        match answer.recv_timeout(PROBE) {
                            Ok(out) => break out,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                if alive.load(Ordering::Relaxed) && gone(&probe) {
                                    alive.store(false, Ordering::Relaxed);
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => break json!({ "ok": false, "error": OFF }),
                        }
                    }
                }
            }
        };
        if writeln!(writer, "{out}").and_then(|()| writer.flush()).is_err() || close {
            break;
        }
    }
}

async fn respond(body: &Value, alive: &AtomicBool, cx: &mut AsyncApp) -> Value {
    let session = body["session"].as_str().unwrap_or_default().to_string();
    let name = body["tool"].as_str().unwrap_or_default().to_string();
    let args = body.get("args").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    match run(&session, &name, &args, alive, cx).await {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

/// What „heute erlauben“ would cover for an MCP write: the note for note writes, the folder for new notes, nothing for the rest; only for the session that asked.
fn scope_of(session: &str, name: &str, args: &Value, cx: &App) -> Option<Scope> {
    let folder = name == "create_note";
    state::read(cx, |l| tools::scope(l, name, args)).map(|p| Scope::of(&format!("{session} {name}"), &p, folder))
}

async fn run(session: &str, name: &str, args: &Value, alive: &AtomicBool, cx: &mut AsyncApp) -> Result<Value, String> {
    if !cx.update(|cx| active() && cx.global::<Ai>().server.is_some()) {
        return Err(OFF.into());
    }
    cx.update(follow_root);
    let pinned = cx.update(|cx| state::read(cx, |l| tools::pinned(l, name, args)));
    let args = &pinned;
    if tools::is_write(name) {
        if cx.update(|cx| cx.global::<Ai>().read_only.contains(session)) {
            return Err(READ_ONLY.into());
        }
        let preview = cx.update(|cx| state::read(cx, |l| tools::preview(l, name, args)))?;
        let scope = cx.update(|cx| scope_of(session, name, args, cx));
        if !cx.update(|cx| scope.as_ref().is_some_and(|s| allowed_today(s, cx))) {
            let terminal = cx.update(|cx| !cx.global::<Ai>().turns.is_bounded(session));
            let (id, answer) = cx.update(|cx| ask_for(session, name, scope, terminal, preview, cx));
            let mut reply = Box::pin(answer.recv());
            let answer = loop {
                let tick = Box::pin(cx.background_executor().timer(PROBE));
                match futures::future::select(reply, tick).await {
                    futures::future::Either::Left((answer, _)) => break answer.unwrap_or(Answer::Decline),
                    futures::future::Either::Right((_, pending)) => {
                        reply = pending;
                        if !alive.load(Ordering::Relaxed) {
                            cx.update(|cx| answer_ask(id, Answer::Decline, cx));
                            return Err(GONE.into());
                        }
                    }
                }
            };
            match answer {
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
        if !alive.load(Ordering::Relaxed) {
            return Err(GONE.into());
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
    if name == "propose_inbox"
        && out.is_ok()
        && let Ok(rows) = state::read(cx, |l| crate::core::ai::inbox::plan(l, args))
    {
        let mut rest = Vec::new();
        for row in rows {
            match cx.global_mut::<Ai>().fills.remove(&row.path).and_then(|f| f.upgrade()) {
                Some(form) => {
                    keep_prefill(row, cx);
                    form.update(cx, |_, cx| cx.notify());
                }
                None => rest.push(row),
            }
        }
        if !rest.is_empty()
            && let Some(panel) = cx.global::<Ai>().panel.clone().and_then(|p| p.upgrade())
        {
            panel.update(cx, |p, cx| p.record_inbox(session, rest, cx));
        }
    }
    if name == "propose_setup"
        && out.is_ok()
        && let Some(panel) = cx.global::<Ai>().panel.clone().and_then(|p| p.upgrade())
    {
        let proposal = args.get("proposal").cloned().unwrap_or_default();
        panel.update(cx, |p, cx| p.record_setup(session, proposal, cx));
    }
    if write && let Ok(value) = &out {
        let made = value["path"].as_str().or_else(|| value["note"].as_str()).and_then(|r| state::read(cx, |l| l.root.as_ref().map(|root| root.join(r))));
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

fn allowed_today(scope: &Scope, cx: &App) -> bool {
    let today = Local::now().date_naive();
    let root = state::read(cx, |l| l.root.clone());
    cx.global::<Ai>().allowed.as_ref().is_some_and(|(day, at, keys)| *day == today && *at == root && keys.contains(&scope.key))
}

/// Declines every waiting change of `session`, e.g. when its turn was stopped.
pub fn decline_session(session: &str, cx: &mut App) {
    let ids: Vec<u64> = cx.try_global::<Ai>().map(|ai| ai.asks.iter().filter(|a| a.session == session).map(|a| a.id).collect()).unwrap_or_default();
    for id in ids {
        answer_ask(id, Answer::Decline, cx);
    }
}

fn ask_for(session: &str, name: &str, scope: Option<Scope>, terminal: bool, preview: Preview, cx: &mut App) -> (u64, async_channel::Receiver<Answer>) {
    let (reply, answer) = async_channel::bounded(1);
    let ai = cx.global_mut::<Ai>();
    ai.next += 1;
    let id = ai.next;
    ai.asks.push_back(Ask { id, session: session.to_string(), tool: name.to_string(), scope, terminal, preview, reply });
    cx.refresh_windows();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(ASK_WAIT).await;
        cx.update(|cx| answer_ask(id, Answer::Timeout, cx));
    })
    .detach();
    (id, answer)
}

/// Answers the waiting write `id`; „heute erlauben“ lets the same tool on the same note or folder through until midnight, in this study folder.
pub fn answer_ask(id: u64, answer: Answer, cx: &mut App) {
    let root = state::read(cx, |l| l.root.clone());
    let ai = cx.global_mut::<Ai>();
    let Some(at) = ai.asks.iter().position(|a| a.id == id) else { return };
    let Some(ask) = ai.asks.remove(at) else { return };
    if answer == Answer::Today
        && let Some(scope) = &ask.scope
    {
        let today = Local::now().date_naive();
        let allowed = ai.allowed.get_or_insert_with(|| (today, root.clone(), HashSet::new()));
        if allowed.0 != today || allowed.1 != root {
            *allowed = (today, root, HashSet::new());
        }
        allowed.2.insert(scope.key.clone());
    }
    let _ = ask.reply.try_send(answer);
    cx.refresh_windows();
}

/// Answers every request of a group at once.
pub fn answer_all(ids: &[u64], answer: Answer, cx: &mut App) {
    for id in ids {
        answer_ask(*id, answer, cx);
    }
}

fn toggle_group(key: String, cx: &mut App) {
    let open = &mut cx.global_mut::<Ai>().open_groups;
    if !open.remove(&key) {
        open.insert(key);
    }
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
        let named = !prefs::with(|p| p.used_features.contains(COLUMN_USED));
        let button = Button::new("ai-button").ghost().rounded_full().when(open, |b| b.bg(t.ink.opacity(0.14))).icon(menu_icon("bot")).tooltip(tip).on_click(|_, _, cx| toggle_column(cx));
        if named { button.h(px(control::M)).px(px(space::M)).label(tr("KI")) } else { button.size(px(control::M)) }
    })
}

const SHOWN_CHARS: usize = 400;
const CARD_SCROLL: f32 = 320.;

fn shortened(line: &str) -> String {
    if line.chars().count() <= SHOWN_CHARS { line.to_string() } else { format!("{}…", line.chars().take(SHOWN_CHARS).collect::<String>()) }
}

/// The lines of a change, how many are left out, and „Ansehen“ for the whole text side by side.
fn change_box(id: u64, preview: &Preview, t: &Theme) -> Option<AnyElement> {
    if preview.lines.is_empty() && preview.hidden == 0 && preview.full.is_none() {
        return None;
    }
    let mut lines = div().id(("ai-ask-lines", id)).max_h(px(CARD_SCROLL)).overflow_y_scroll().flex().flex_col().gap(px(space::XXS)).font_family(Theme::mono_family()).text_size(px(font::CAPTION)).text_color(t.secondary);
    for line in &preview.lines {
        lines = lines.child(div().child(if tools::has_link(line) { line.clone() } else { shortened(line) }));
    }
    let more = match preview.hidden {
        0 => None,
        1 => Some(tr("1 weitere Zeile nicht gezeigt")),
        n => Some(trf("%lld weitere Zeilen nicht gezeigt", &[&n])),
    };
    let look = preview.full.clone().map(|full| {
        Button::new(("ai-ask-look", id)).ghost().xsmall().label(tr("Ansehen")).tooltip(tr("Die ganze Änderung ansehen, bevor du entscheidest")).on_click(move |_, window, cx| super::ai_panel::show_diff(&full.path, &full.before, &full.after, None, window, cx))
    });
    let footer = (more.is_some() || look.is_some()).then(|| div().flex().items_center().gap(px(space::S)).child(div().flex_1().min_w_0().text_size(px(font::CAPTION)).text_color(t.tertiary).children(more)).children(look));
    Some(div().flex().flex_col().gap(px(space::XS)).p(px(space::S)).rounded(px(radius::INNER)).bg(t.ink.opacity(0.05)).when(!preview.lines.is_empty(), |d| d.child(lines)).children(footer).into_any_element())
}

fn today_label(w: &Waiting) -> Option<String> {
    w.today.map(|folder| if folder { tr("In diesem Ordner heute erlauben") } else if w.tool == "write_file" { tr("In dieser Datei heute erlauben") } else { tr("In dieser Notiz heute erlauben") })
}

const ASK_ENTER_GRACE: Duration = Duration::from_millis(600);

/// The focus of the floating permission card: the card itself, its three buttons and what had the focus before it appeared.
pub struct AskFocus {
    card: FocusHandle,
    buttons: [FocusHandle; 3],
    shown: u64,
    since: std::time::Instant,
    before: Option<WeakFocusHandle>,
}

impl AskFocus {
    fn new(cx: &mut Context<Self>) -> Self {
        Self { card: cx.focus_handle(), buttons: std::array::from_fn(|_| cx.focus_handle()), shown: 0, since: std::time::Instant::now(), before: None }
    }
}

type AskAction = Rc<dyn Fn(&mut Window, &mut App)>;

type AskStops = Option<(Entity<AskFocus>, FocusHandle, [FocusHandle; 3])>;

fn ask_stop(stops: &AskStops, at: usize) -> Option<FocusHandle> {
    stops.as_ref().map(|(_, _, handles)| handles[at].clone())
}

fn ask_track(button: Button, handle: Option<FocusHandle>) -> Button {
    match handle {
        Some(h) => button.track_focus(&h),
        None => button,
    }
}

fn ask_keys(order: Vec<(FocusHandle, AskAction)>, focus: Entity<AskFocus>) -> impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static {
    move |e, window, cx| {
        let k = &e.keystroke;
        if k.key != "enter" || k.modifiers.shift || k.modifiers.platform || k.modifiers.control || k.modifiers.alt || order.is_empty() {
            return;
        }
        cx.stop_propagation();
        if focus.read(cx).since.elapsed() < ASK_ENTER_GRACE {
            return;
        }
        let at = order.iter().position(|(h, _)| h.is_focused(window));
        (order[at.unwrap_or(order.len() - 1)].1)(window, cx);
    }
}

/// The inside of a permission card: one request with its change, or several of the same kind, answered together or one by one. With `focus` the card takes keys: Tab walks the buttons, Enter does the focused one or allows, Esc declines.
pub fn ask_body(group: &[Waiting], focus: Option<&Entity<AskFocus>>, cx: &App, t: &Theme) -> Option<Div> {
    let first = group.first()?;
    let stops = focus.map(|f| (f.clone(), f.read(cx).card.clone(), f.read(cx).buttons.clone()));
    let header = if first.terminal { tr("Ein KI-Programm im Terminal möchte etwas ändern") } else { tr("Die KI möchte etwas ändern") };
    let body = div().flex().flex_col().gap(px(space::S)).child(div().flex().items_center().gap(px(space::S)).child(icon("bot", iconsize::S, t.secondary)).child(div().flex_1().text_size(px(font::CAPTION)).text_color(t.secondary).child(header)));
    let buttons = div().flex().flex_wrap().justify_end().gap(px(space::S));
    if let [w] = group {
        let id = w.id;
        let today_text = today_label(w);
        let has_today = today_text.is_some();
        let today = today_text.map(|label| Button::new(("ai-today", id)).small().label(label).tooltip(tr("Gleiche Änderungen an derselben Stelle bis Mitternacht ohne Nachfrage")).on_click(move |_, _, cx| answer_ask(id, Answer::Today, cx)));
        let (decline, today_stop, allow) = (ask_stop(&stops, 0), ask_stop(&stops, 1), ask_stop(&stops, 2));
        let body = match &stops {
            Some((focus, card, handles)) => {
                let mut order: Vec<(FocusHandle, AskAction)> = vec![(handles[0].clone(), Rc::new(move |_, cx| answer_ask(id, Answer::Decline, cx)))];
                if has_today {
                    order.push((handles[1].clone(), Rc::new(move |_, cx| answer_ask(id, Answer::Today, cx))));
                }
                order.push((handles[2].clone(), Rc::new(move |_, cx| answer_ask(id, Answer::Allow, cx))));
                body.track_focus(card).capture_key_down(ask_keys(order, focus.clone()))
            }
            None => body,
        };
        return Some(
            body.child(div().font_weight(FontWeight::SEMIBOLD).child(w.preview.title.clone()))
                .children(change_box(id, &w.preview, t))
                .child(
                    buttons
                        .child(ask_track(Button::new(("ai-decline", id)).ghost().small().label(tr("Ablehnen")).on_click(move |_, _, cx| answer_ask(id, Answer::Decline, cx)), decline))
                        .children(today.map(|b| ask_track(b, today_stop)))
                        .child(ask_track(Button::new(("ai-allow", id)).primary().small().label(tr("Erlauben")).on_click(move |_, _, cx| answer_ask(id, Answer::Allow, cx)), allow)),
                ),
        );
    }
    let key = group_key(first);
    let open = cx.try_global::<Ai>().is_some_and(|ai| ai.open_groups.contains(&key));
    let ids: Vec<u64> = group.iter().map(|w| w.id).collect();
    let mut rows = div().id(("ai-group-rows", first.id)).max_h(px(CARD_SCROLL)).overflow_y_scroll().flex().flex_col().gap(px(space::XS));
    for w in group {
        let id = w.id;
        let row = div().flex().items_center().gap(px(space::S)).child(div().flex_1().min_w_0().text_size(px(font::CALLOUT)).child(w.preview.title.clone()));
        rows = rows.child(if open {
            div().flex().flex_col().gap(px(space::XS)).child(row.child(Button::new(("ai-row-decline", id)).ghost().xsmall().label(tr("Ablehnen")).on_click(move |_, _, cx| answer_ask(id, Answer::Decline, cx)))).children(change_box(id, &w.preview, t))
        } else {
            row
        });
    }
    let first_id = first.id;
    let (toggle_key, decline, allow) = (key.clone(), ids.clone(), ids.clone());
    let body = match &stops {
        Some((focus, card, handles)) => {
            let (key, decline, allow) = (key.clone(), decline.clone(), allow.clone());
            let order: Vec<(FocusHandle, AskAction)> = vec![
                (handles[0].clone(), Rc::new(move |_, cx| toggle_group(key.clone(), cx))),
                (handles[1].clone(), Rc::new(move |_, cx| answer_all(&decline, Answer::Decline, cx))),
                (handles[2].clone(), Rc::new(move |_, cx| answer_all(&allow, Answer::Allow, cx))),
            ];
            body.track_focus(card).capture_key_down(ask_keys(order, focus.clone()))
        }
        None => body,
    };
    let (open_stop, decline_stop, allow_stop) = (ask_stop(&stops, 0), ask_stop(&stops, 1), ask_stop(&stops, 2));
    Some(
        body.child(div().font_weight(FontWeight::SEMIBOLD).child(trf("%lld Änderungen auf einmal", &[&group.len()])))
            .child(rows)
            .child(
                buttons
                    .child(ask_track(Button::new(("ai-group-open", first_id)).ghost().small().label(if open { tr("Einklappen") } else { tr("Einzeln ansehen") }).on_click(move |_, _, cx| toggle_group(toggle_key.clone(), cx)), open_stop))
                    .child(ask_track(Button::new(("ai-group-decline", first_id)).ghost().small().label(tr("Alle ablehnen")).on_click(move |_, _, cx| answer_all(&decline, Answer::Decline, cx)), decline_stop))
                    .child(ask_track(Button::new(("ai-group-allow", first_id)).primary().small().label(tr("Alle erlauben")).on_click(move |_, _, cx| answer_all(&allow, Answer::Allow, cx)), allow_stop)),
            ),
    )
}

fn arrive_ask_focus(focus: &Entity<AskFocus>, id: u64, window: &mut Window, cx: &mut App) {
    if focus.read(cx).shown == id {
        return;
    }
    let card = focus.read(cx).card.clone();
    let own = focus.read(cx).buttons.iter().any(|b| b.contains_focused(window, cx)) || card.contains_focused(window, cx);
    let before = if own { None } else { window.focused(cx).map(|h| h.downgrade()) };
    focus.update(cx, |f, _| {
        if f.shown == 0 {
            f.before = before;
        }
        f.shown = id;
        f.since = std::time::Instant::now();
    });
    window.defer(cx, move |window, cx| window.focus(&card, cx));
}

fn release_ask_focus(focus: &Entity<AskFocus>, window: &mut Window, cx: &mut App) {
    if focus.read(cx).shown == 0 {
        return;
    }
    let before = focus.update(cx, |f, _| {
        f.shown = 0;
        f.before.take()
    });
    if let Some(handle) = before.and_then(|h| h.upgrade()) {
        window.defer(cx, move |window, cx| window.focus(&handle, cx));
    }
}

/// The card that asks before the agent changes something. It takes the focus when it appears and gives it back when the last request is answered.
pub fn ask_card(window: &mut Window, cx: &mut App, t: &Theme) -> Option<Stateful<Div>> {
    let focus = window.use_keyed_state("ai-ask-focus", cx, |_, cx| AskFocus::new(cx));
    let groups = if column_open() { Vec::new() } else { waiting_groups(cx) };
    let Some(group) = groups.first() else {
        release_ask_focus(&focus, window, cx);
        return None;
    };
    let more: usize = groups.iter().skip(1).map(Vec::len).sum();
    arrive_ask_focus(&focus, group.first().map_or(0, |w| w.id), window, cx);
    let body = ask_body(group, Some(&focus), cx, t)?;
    let ids: Vec<u64> = group.iter().map(|w| w.id).collect();
    let card = focus.read(cx).card.clone();
    Some(
        div()
            .id("ai-ask-slot")
            .absolute()
            .top(px(control::BAR + space::L))
            .right(px(space::L))
            .bottom(px(space::L))
            .w(px(380.))
            .max_w_full()
            .flex()
            .flex_col()
            .justify_end()
            .child(
                super::drive::marked(div().id("ai-ask"), "dialog")
                    .occlude()
                    .max_h_full()
                    .overflow_y_scroll()
                    .p(px(space::L))
                    .rounded(px(radius::CARD))
                    .bg(t.material)
                    .border_1()
                    .border_color(t.separator)
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap(px(space::S))
                    .on_action(move |_: &super::main_window::Dismiss, _, cx| answer_all(&ids, Answer::Decline, cx))
                    .child(body)
                    .when(more > 0, |d| d.child(caption(if more == 1 { tr("Danach wartet noch 1 Anfrage") } else { trf("Danach warten noch %lld Anfragen", &[&more]) }, t)))
                    .focus_trap("ai-ask-trap", &card),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::{LINE_LIMIT, Line, gone, read_line};
    use std::io::{BufReader, Cursor};
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn a_line_longer_than_the_limit_ends_the_connection() {
        let mut ok = BufReader::new(Cursor::new(b"{\"a\":1}\nrest".to_vec()));
        assert_eq!(read_line(&mut ok), Line::Text(b"{\"a\":1}\n".to_vec()));
        let mut huge = BufReader::new(Cursor::new(vec![b'x'; LINE_LIMIT + 10]));
        assert_eq!(read_line(&mut huge), Line::TooLong);
        let mut empty = BufReader::new(Cursor::new(Vec::new()));
        assert_eq!(read_line(&mut empty), Line::End);
    }

    #[test]
    fn a_caller_that_hangs_up_is_noticed_while_its_card_waits() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let caller = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        assert!(!gone(&server), "a waiting caller is still there");
        drop(caller);
        let mut seen = false;
        for _ in 0..50 {
            if gone(&server) {
                seen = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(seen, "the hang-up is seen");
    }
}
