use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::v1::schema::{EnvVariable, McpServerStdio};
use agent_client_protocol::{AcpAgent, AcpAgentConfig};
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;

use super::ai::{self, Answer, Request};
use super::markdown_view::{self, ParsedNote};
use super::motion::{self, Soft};
use super::state::{self, act};
use super::theme::{Theme, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::ai::acp::{self, Command, Event, Kind, Status, Stop, ToolPatch, Update};
use crate::core::ai::agents::{self, Missing};
use crate::core::ai::prompts::Flow;
use crate::core::ai::setup::{self, Plan};
use crate::core::ai::tools::{self, Preview};
use crate::core::ai::{detect, transcript};
use crate::core::i18n::{tr, trf};
use crate::core::library::Route;
use crate::core::prefs;

const FRAME: Duration = Duration::from_millis(16);
const REPARSE: Duration = Duration::from_millis(120);
const SELECTION_CHARS: usize = 4000;
const SHOWN_OUTPUT: usize = 4000;
const HIDDEN_TOOLS: [&str; 3] = ["ToolSearch", "TodoWrite", "ExitPlanMode"];
const RECAP_CHARS: usize = 6000;
const HISTORY_SHOWN: usize = 20;

/// Something the agent sees with the next message.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Chip {
    Note(PathBuf),
    Pdf(PathBuf, usize),
    /// Pages of a PDF, first and last counted from 0.
    Pages(PathBuf, usize, usize),
    Selection(String),
    Module(String),
    Deadline(String),
    /// The week starting on this Monday.
    Week(chrono::NaiveDate),
}

struct Tool {
    name: Option<String>,
    title: String,
    kind: Kind,
    status: Status,
    input: Option<Value>,
    output: Option<String>,
    denied: bool,
}

struct Edit {
    path: PathBuf,
    before: Option<String>,
    after: String,
    added: usize,
    removed: usize,
    undone: bool,
    stale: bool,
}

enum Item {
    User { text: String, chips: Vec<String> },
    Agent { text: String, parsed: Option<Rc<ParsedNote>>, parsed_len: usize, insert: Option<Insert> },
    Tool(Tool),
    Edit(Edit),
    Notice { text: String, restart: bool },
    Setup(SetupCard),
    Inbox(InboxCard),
}

/// The agent's reading of inbox files: one row per file, taken or left, filed in one step.
struct InboxCard {
    rows: Vec<crate::core::ai::inbox::Row>,
    chosen: HashSet<usize>,
    edited: HashSet<usize>,
    state: CardState,
    error: Option<String>,
}

/// Where an explanation can go when the user wants it in a note.
struct Insert {
    note: PathBuf,
    done: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CardState {
    Open,
    Applied(usize, usize),
    Replaced,
    Dismissed,
}

/// The agent's setup proposal: items the user ticks and takes in one step.
struct SetupCard {
    plan: Plan,
    chosen: HashSet<String>,
    state: CardState,
    renames_open: bool,
    error: Option<String>,
}

/// What one turn was started for.
#[derive(Clone, Default)]
struct TurnInfo {
    flow: Option<Flow>,
    source: Option<PathBuf>,
}

struct Session {
    id: String,
    commands: async_channel::Sender<Command>,
    ready: bool,
    file: Option<PathBuf>,
    seen: HashMap<PathBuf, String>,
    granted: HashSet<PathBuf>,
    stopped: bool,
    _run: Task<()>,
    _pump: Task<()>,
}

/// The column „KI“: the conversation with the user's agent, its tool steps, its changes and the input.
pub struct AiPanel {
    input: Entity<TextareaState>,
    items: Vec<Item>,
    tools: HashMap<String, usize>,
    expanded: HashSet<usize>,
    session: Option<Session>,
    turn: bool,
    queued: usize,
    removed: HashSet<Chip>,
    extra: Vec<Chip>,
    last_prompt: Option<String>,
    list: ListState,
    listed: usize,
    dirty: BTreeSet<usize>,
    tail: (Vec<ai::Waiting>, Option<String>),
    idle: Option<Task<()>>,
    reparse: Option<Task<()>>,
    focus_input: bool,
    cancelling: bool,
    asks: Vec<u64>,
    pending: VecDeque<TurnInfo>,
    recap: Option<String>,
    hint: Option<&'static str>,
    logged: usize,
    log_task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn is_pdf(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

fn arg<'a>(input: Option<&'a Value>, key: &str) -> Option<&'a str> {
    input.and_then(|v| v.get(key)).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn file_arg(input: Option<&Value>, key: &str) -> Option<String> {
    arg(input, key).map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string())
}

/// The German title of a tool step, from its name and arguments; falls back to the agent's own title.
pub fn title_for(name: Option<&str>, kind: Kind, input: Option<&Value>, fallback: &str) -> String {
    let own = name.and_then(|n| n.strip_prefix("mcp__")).and_then(|r| r.split_once("__")).map(|(_, tool)| tool);
    let file = |k: &str| file_arg(input, k).unwrap_or_default();
    let quoted = |k: &str| arg(input, k).unwrap_or_default().to_string();
    match own.or(name) {
        Some("get_context") => tr("Schaut, was offen ist"),
        Some("search") => trf("Sucht „%@“", &[&quoted("query")]),
        Some("read_note") => trf("Liest %@", &[&file("path")]),
        Some("read_pdf_text") => trf("Liest %@, Seite %@", &[&file("path"), &quoted("pages")]),
        Some("list_folder") => tr("Schaut in den Ordner"),
        Some("list_deadlines") => tr("Liest die Fristen"),
        Some("list_modules") => tr("Liest die Module"),
        Some("get_week") => tr("Liest die Woche"),
        Some("list_cards") => tr("Liest die Karten"),
        Some("write_note") => trf("Ändert %@", &[&file("path")]),
        Some("create_note") => trf("Legt „%@“ an", &[&quoted("title")]),
        Some("add_cards") => {
            let n = input.and_then(|v| v.get("cards")).and_then(Value::as_array).map_or(0, Vec::len);
            if n == 1 { trf("Ergänzt 1 Karte in %@", &[&file("note")]) } else { trf("Ergänzt %lld Karten in %@", &[&n, &file("note")]) }
        }
        Some("add_deadline") => trf("Legt „%@“ an", &[&quoted("title")]),
        Some("set_status") => tr("Ändert einen Status"),
        Some("file_document") => trf("Legt %@ ab", &[&file("path")]),
        Some("open") => trf("Öffnet %@", &[&file("path")]),
        Some("list_inbox") => tr("Schaut in die Inbox"),
        Some("propose_filing") => tr("Sucht Plätze für die Inbox"),
        Some("read_setup") => tr("Liest die Einstellungen"),
        Some("propose_setup") => tr("Schlägt eine Einrichtung vor"),
        Some("apply_setup") => tr("Übernimmt die Einrichtung"),
        Some("propose_inbox") => tr("Schlägt vor, wohin die Dateien gehören"),
        Some("apply_inbox") => tr("Legt die Dateien ab"),
        Some("Read") => trf("Liest %@", &[&file("file_path")]),
        Some("Glob" | "Grep") => trf("Sucht „%@“", &[&quoted("pattern")]),
        Some("WebFetch") => trf("Ruft %@ ab", &[&quoted("url")]),
        Some("WebSearch") => trf("Sucht im Netz: %@", &[&quoted("query")]),
        Some("Bash" | "PowerShell") => trf("Befehl: %@", &[&quoted("command")]),
        Some("Edit" | "Write" | "MultiEdit") => trf("Ändert %@", &[&file("file_path")]),
        _ => match kind {
            Kind::Read if !fallback.is_empty() => fallback.to_string(),
            Kind::Read => tr("Liest"),
            Kind::Search => tr("Sucht"),
            Kind::Edit => tr("Ändert eine Datei"),
            Kind::Execute => tr("Befehl"),
            Kind::Fetch => tr("Ruft eine Seite ab"),
            Kind::Think => tr("Denkt nach"),
            _ if fallback.is_empty() => tr("Arbeitet"),
            _ => fallback.to_string(),
        },
    }
}

fn tool_icon(name: Option<&str>, kind: Kind) -> &'static str {
    let own = name.and_then(|n| n.strip_prefix("mcp__")).and_then(|r| r.split_once("__")).map(|(_, tool)| tool);
    match own {
        Some("search") => return "search",
        Some(tool) if tools::is_write(tool) => return "pencil",
        Some(_) => return "eye",
        None => {}
    }
    match kind {
        Kind::Read => "eye",
        Kind::Edit | Kind::Delete | Kind::Move => "pencil",
        Kind::Search => "search",
        Kind::Execute => "terminal",
        Kind::Fetch => "globe",
        _ => "bot",
    }
}

fn counts(before: &str, after: &str) -> (usize, usize) {
    let diff = similar::TextDiff::from_lines(before, after);
    diff.iter_all_changes().fold((0, 0), |(a, r), c| match c.tag() {
        similar::ChangeTag::Insert => (a + 1, r),
        similar::ChangeTag::Delete => (a, r + 1),
        similar::ChangeTag::Equal => (a, r),
    })
}

/// The line ranges of `after` that differ from `before`, counted from 0; a pure deletion marks the line where it was.
pub fn changed_lines(before: &str, after: &str) -> Vec<(usize, usize)> {
    let total = after.split('\n').count().max(1);
    let mut out: Vec<(usize, usize)> = Vec::new();
    for op in similar::TextDiff::from_lines(before, after).ops() {
        let range = op.new_range();
        let span = match op.tag() {
            similar::DiffTag::Equal => continue,
            similar::DiffTag::Delete => (range.start.min(total - 1), range.start.min(total - 1)),
            _ if range.is_empty() => continue,
            _ => (range.start, range.end - 1),
        };
        match out.last_mut() {
            Some(last) if span.0 <= last.1 + 1 => last.1 = last.1.max(span.1),
            _ => out.push(span),
        }
    }
    out
}

fn edit_line(e: &Edit) -> String {
    trf("+%lld −%lld in %@", &[&e.added, &e.removed, &name_of(&e.path)])
}

fn missing_text(m: &Missing) -> String {
    match m {
        Missing::Adapter => tr("Für Claude fehlt noch ein Baustein. Hol ihn in Einstellungen → KI."),
        Missing::Node => tr("Codex braucht Node.js. Installiere es und versuch es noch einmal."),
        Missing::Program => tr("Das Programm ist nicht mehr installiert."),
        Missing::Command => tr("Der eigene Befehl ist leer."),
    }
}

/// A study task worth offering where the user is right now.
#[derive(Clone)]
pub struct Suggestion {
    pub label: String,
    pub icon: &'static str,
    pub flow: Flow,
    pub chips: Vec<Chip>,
}

/// The few tasks worth offering where the user is right now, at most three.
pub fn suggestions(cx: &App) -> Vec<Suggestion> {
    let (route, doc, inbox, monday) = state::read(cx, |l| (l.route().clone(), l.focused_document().map(Path::to_path_buf), !l.inbox.is_empty(), crate::core::day::monday(l.today())));
    let s = |label: String, flow: Flow, chips: Vec<Chip>| Suggestion { label, icon: flow.icon(), flow, chips };
    let mut out = Vec::new();
    match (&route, doc) {
        (Route::Doc(_), Some(doc)) if is_pdf(&doc) => {
            let focus = ai::pdf_focus(&doc);
            let page = Chip::Pdf(doc.clone(), focus.as_ref().map_or(0, |f| f.page));
            match focus.as_ref().map(|f| f.text.clone()).filter(|t| !t.trim().is_empty()) {
                Some(text) => {
                    out.push(s(tr("Karten aus der Auswahl"), Flow::Cards, vec![page.clone(), Chip::Selection(text.clone())]));
                    out.push(s(tr("Passage erklären"), Flow::Explain, vec![page, Chip::Selection(text)]));
                }
                None => {
                    out.push(s(tr("Karten aus dieser Seite"), Flow::Cards, vec![page.clone()]));
                    out.push(s(tr("Diese Seite erklären"), Flow::Explain, vec![page]));
                }
            }
            out.push(s(Flow::Summary.label(), Flow::Summary, vec![whole_file_chip(&doc)]));
        }
        (Route::Doc(_), Some(doc)) if crate::core::markdown::is_markdown(&doc) => {
            out.push(s(tr("Karten aus dieser Notiz"), Flow::Cards, vec![Chip::Note(doc.clone())]));
            out.push(s(Flow::Quiz.label(), Flow::Quiz, vec![Chip::Note(doc)]));
        }
        (Route::Today | Route::Grid, _) => out.push(s(Flow::Week.label(), Flow::Week, vec![Chip::Week(monday)])),
        (Route::Inbox, _) if inbox => out.push(s(Flow::Inbox.label(), Flow::Inbox, Vec::new())),
        (Route::Cards, _) => out.push(s(tr("Karten abfragen"), Flow::Quiz, Vec::new())),
        _ => {}
    }
    out.truncate(3);
    out
}

/// Whether setting up with the assistant is worth offering in the empty column: modules, semester, naming or the Moodle calendar are still open.
pub fn setup_wanted(cx: &App) -> bool {
    use crate::core::setup::{Item as Step, settled};
    let Some(root) = state::read(cx, |l| l.root.clone()) else { return false };
    let facts = state::read(cx, |l| super::setup_card::facts(l, cx));
    let marks = prefs::setup_of(&root);
    [Step::Modules, Step::Semester, Step::Naming, Step::Calendar].into_iter().any(|s| !settled(s, &facts, &marks.skipped, &marks.confirmed))
}

/// The chip for a whole file: all pages of a PDF, else the note.
pub fn whole_file_chip(path: &Path) -> Chip {
    if is_pdf(path) {
        let pages = ai::pdf_focus(path).map(|f| f.pages).filter(|n| *n > 0).or_else(|| crate::core::pdf_render::PdfDoc::open(path).map(|d| d.page_count())).unwrap_or(1);
        Chip::Pages(path.to_path_buf(), 0, pages.saturating_sub(1))
    } else {
        Chip::Note(path.to_path_buf())
    }
}

fn chip_label(chip: &Chip, cx: &App) -> String {
    match chip {
        Chip::Note(p) => name_of(p),
        Chip::Pdf(p, page) => trf("%@ · S. %lld", &[&name_of(p), &(page + 1)]),
        Chip::Pages(p, first, last) if first == last => trf("%@ · S. %lld", &[&name_of(p), &(first + 1)]),
        Chip::Pages(p, first, last) => trf("%@ · S. %lld–%lld", &[&name_of(p), &(first + 1), &(last + 1)]),
        Chip::Week(monday) => state::read(cx, |l| l.lecture_week_containing(*monday).map(|w| w.label())).unwrap_or_else(|| trf("Woche ab %@", &[&crate::core::day::short_date(*monday)])),
        Chip::Selection(text) => {
            let short: String = text.chars().take(24).collect();
            if text.chars().count() > 24 { format!("„{}…“", short.trim_end()) } else { format!("„{short}“") }
        }
        Chip::Module(id) => state::read(cx, |l| l.module_by_id(id).map(|m| m.code.clone())).unwrap_or_default(),
        Chip::Deadline(id) => state::read(cx, |l| l.assignment(id).map(|a| a.title.clone())).unwrap_or_default(),
    }
}

fn chip_icon(chip: &Chip) -> &'static str {
    match chip {
        Chip::Note(_) => "file-text",
        Chip::Pdf(..) | Chip::Pages(..) => "file",
        Chip::Week(_) => "calendar-range",
        Chip::Selection(_) => "text-select",
        Chip::Module(_) => "book-open",
        Chip::Deadline(_) => "calendar-clock",
    }
}

fn context_lines(chips: &[Chip], cx: &App) -> Vec<String> {
    let rel = |p: &Path| state::read(cx, |l| l.relative_path(p));
    chips
        .iter()
        .filter_map(|c| match c {
            Chip::Note(p) => Some(format!("- Open note: {}", rel(p))),
            Chip::Pdf(p, page) => {
                let total = ai::pdf_focus(p).map(|f| f.pages).filter(|n| *n > 0).map(|n| format!(" of {n}")).unwrap_or_default();
                Some(format!("- PDF: {}, page {}{total}", rel(p), page + 1))
            }
            Chip::Pages(p, first, last) => {
                let total = ai::pdf_focus(p).map(|f| f.pages).filter(|n| *n > 0).map(|n| format!(" of {n}")).unwrap_or_default();
                Some(format!("- PDF: {}, pages {}-{}{total}", rel(p), first + 1, last + 1))
            }
            Chip::Week(monday) => Some(format!("- Week: {} to {}", monday.format("%Y-%m-%d"), (*monday + chrono::Days::new(6)).format("%Y-%m-%d"))),
            Chip::Selection(text) => {
                let cut: String = text.chars().take(SELECTION_CHARS).collect();
                Some(format!("- Selected text:\n> {}", cut.replace('\n', "\n> ")))
            }
            Chip::Module(id) => state::read(cx, |l| l.module_by_id(id).map(|m| format!("- Module: {} ({}), id {}", m.code, m.name, m.id))),
            Chip::Deadline(id) => state::read(cx, |l| l.assignment(id).map(|a| format!("- Deadline: {} (id {}, due {})", a.title, a.id, a.due.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")))),
        })
        .collect()
}

fn diff_lines(diffs: &[acp::Diff]) -> Vec<String> {
    let mut out = Vec::new();
    for d in diffs {
        let old = d.old.clone().unwrap_or_default();
        for change in similar::TextDiff::from_lines(&old, &d.new).iter_all_changes() {
            let sign = match change.tag() {
                similar::ChangeTag::Insert => "+ ",
                similar::ChangeTag::Delete => "− ",
                similar::ChangeTag::Equal => continue,
            };
            out.push(format!("{sign}{}", change.value().trim_end()));
            if out.len() >= 8 {
                return out;
            }
        }
    }
    out
}

impl AiPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).placeholder(tr("Frag etwas oder gib eine Aufgabe …")).auto_grow(1, 8).submit_on_enter(true));
        let lib = state::lib(cx);
        let subs = vec![
            cx.observe(&lib, |_, _, cx| cx.notify()),
            cx.subscribe_in(&input, window, |this, input, e: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = e {
                    let text = input.read(cx).value().to_string();
                    if text.trim().is_empty() {
                        return;
                    }
                    this.send(text, cx);
                    let input = input.clone();
                    window.defer(cx, move |window, cx| input.update(cx, |s, cx| s.set_value("", window, cx)));
                }
            }),
        ];
        ai::set_panel(cx.entity().downgrade(), cx);
        Self { input, items: Vec::new(), tools: HashMap::new(), expanded: HashSet::new(), session: None, turn: false, queued: 0, removed: HashSet::new(), extra: Vec::new(), last_prompt: None, list: Self::new_list(), listed: 0, dirty: BTreeSet::new(), tail: (Vec::new(), None), idle: None, reparse: None, focus_input: true, cancelling: false, asks: Vec::new(), pending: VecDeque::new(), recap: None, hint: None, logged: 0, log_task: None, _subs: subs }
    }

    fn new_list() -> ListState {
        let list = ListState::new(0, ListAlignment::Bottom, px(600.));
        list.set_follow_mode(FollowMode::Tail);
        list
    }

    fn touch(&mut self, at: usize) {
        self.dirty.insert(at);
    }

    fn auto_chips(&self, cx: &App) -> Vec<Chip> {
        let (route, doc, module, deadline) = state::read(cx, |l| {
            let doc = matches!(l.route(), Route::Doc(_)).then(|| l.focused_document().map(Path::to_path_buf)).flatten();
            let module = match l.route() {
                Route::Module(id) => Some(id.clone()),
                _ => doc.as_deref().and_then(|d| l.module_containing(d)).map(|m| m.id.clone()),
            };
            (l.route().clone(), doc, module, l.selection().map(String::from))
        });
        let mut out = Vec::new();
        if let Some(doc) = doc {
            if is_pdf(&doc) {
                let focus = ai::pdf_focus(&doc);
                out.push(Chip::Pdf(doc.clone(), focus.as_ref().map_or(0, |f| f.page)));
                if let Some(text) = focus.map(|f| f.text).filter(|t| !t.trim().is_empty()) {
                    out.push(Chip::Selection(text));
                }
            } else {
                out.push(Chip::Note(doc));
            }
        }
        if let Some(m) = module {
            out.push(Chip::Module(m));
        }
        if !matches!(route, Route::Doc(_))
            && let Some(d) = deadline
        {
            out.push(Chip::Deadline(d));
        }
        if matches!(route, Route::Today | Route::Grid) {
            out.push(Chip::Week(state::read(cx, |l| crate::core::day::monday(l.today()))));
        }
        out
    }

    fn chips(&self, cx: &App) -> Vec<Chip> {
        let mut out = self.extra.clone();
        for c in self.auto_chips(cx) {
            if !self.removed.contains(&c) && !out.contains(&c) {
                out.push(c);
            }
        }
        out
    }

    fn take_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for r in ai::take_requests(cx) {
            match r {
                Request::Focus => self.focus_input = true,
                Request::Chip(c) => {
                    self.removed.remove(&c);
                    if !self.extra.contains(&c) {
                        self.extra.push(c);
                    }
                    self.focus_input = true;
                }
                Request::Send(text) => self.send(text, cx),
                Request::Flow(flow, chips, text) => self.start_flow_with(flow, chips, text, cx),
            }
        }
        if self.session.is_none() && self.items.is_empty() {
            self.start(cx);
        }
        if std::mem::take(&mut self.focus_input) {
            let input = self.input.clone();
            window.defer(cx, move |window, cx| input.update(cx, |s, cx| s.focus(window, cx)));
        }
    }

    fn notice(&mut self, text: String, restart: bool) {
        self.items.push(Item::Notice { text, restart });
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Some(root) = state::read(cx, |l| l.root.clone()) else { return };
        let p = prefs::with(|p| p.ai.clone());
        let Some(runner) = agents::chosen(&p) else { return };
        let launch = match agents::launch(runner, &p) {
            Ok(l) => l,
            Err(m) => {
                self.notice(missing_text(&m), false);
                return;
            }
        };
        if crate::core::drive::fenced("start agent", launch.program.display()) && std::env::var_os("SCHRUMBI_AI_AGENT_OK").is_none() {
            self.notice(tr("In dieser Testumgebung startet kein Programm."), false);
            return;
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mcp = detect::exe().map(|exe| {
            let (args, env) = agents::server_command(&id);
            McpServerStdio::new(acp::OWN_SERVER, exe).args(args).env(env.into_iter().map(|(k, v)| EnvVariable::new(k, v)).collect())
        });
        let setup = acp::Setup { cwd: root.clone(), mcp, meta: launch.claude.then(|| agents::claude_meta(p.allow_execute)), modes: launch.modes.clone(), require_mode: launch.require_mode, policy: acp::Policy { root: root.clone(), allow_execute: p.allow_execute, client_edits: launch.claude } };
        let (program, args) = crate::core::ai::guard::wrap(&launch.program, &launch.args).unwrap_or_else(|| (launch.program.clone(), launch.args.clone()));
        let agent = AcpAgent::new(AcpAgentConfig::new(program).args(args).envs(launch.env.clone()));
        let (events_tx, events_rx) = async_channel::unbounded::<Event>();
        let (commands_tx, commands_rx) = async_channel::unbounded::<Command>();
        let outcome: Arc<Mutex<Option<Result<(), String>>>> = Arc::default();
        let slot = outcome.clone();
        let run = cx.background_executor().spawn(async move {
            let keep = events_tx.clone();
            let result = acp::run(agent, setup, events_tx, commands_rx).await;
            *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
            drop(keep);
        });
        let pump = cx.spawn(async move |this, cx| {
            while let Ok(first) = events_rx.recv().await {
                let mut batch = vec![first];
                while let Ok(e) = events_rx.try_recv() {
                    batch.push(e);
                }
                if this.update(cx, |p, cx| {
                    for e in batch {
                        p.apply(e, cx);
                    }
                    cx.notify();
                })
                .is_err()
                {
                    return;
                }
                cx.background_executor().timer(FRAME).await;
            }
            let result = outcome.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
            let _ = this.update(cx, |p, cx| p.ended(result, cx));
        });
        let file = transcript::file_for(&root, &id);
        self.session = Some(Session { id, commands: commands_tx, ready: false, file, seen: HashMap::new(), granted: HashSet::new(), stopped: false, _run: run, _pump: pump });
        self.schedule_idle(cx);
    }

    /// Ends the agent process: its input closes and it quits on its own.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.flush_log(cx);
        if let Some(mut s) = self.session.take() {
            s.stopped = true;
            ai::close_session(&s.id, cx);
            s._run.detach();
        }
        self.turn = false;
        self.queued = 0;
        self.idle = None;
        cx.notify();
    }

    fn ended(&mut self, result: Option<Result<(), String>>, cx: &mut Context<Self>) {
        self.flush_log(cx);
        let Some(s) = self.session.take() else { return };
        ai::close_session(&s.id, cx);
        self.turn = false;
        self.queued = 0;
        if !s.stopped {
            let why = match result {
                Some(Err(e)) => e.lines().find(|l| !l.trim().is_empty()).unwrap_or_default().chars().take(200).collect::<String>(),
                _ => String::new(),
            };
            self.notice(if why.is_empty() { tr("Das Programm wurde beendet.") } else { trf("Das Programm wurde beendet: %@", &[&why]) }, true);
        }
        cx.notify();
    }

    fn schedule_idle(&mut self, cx: &mut Context<Self>) {
        let minutes = u64::from(prefs::with(|p| p.ai.idle_minutes).max(1));
        self.idle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(minutes * 60)).await;
            let _ = this.update(cx, |p, cx| {
                if !p.turn && p.session.is_some() {
                    p.stop(cx);
                    p.notice(trf("Nach %lld Minuten Pause beendet. Die nächste Nachricht startet neu.", &[&minutes]), false);
                }
            });
        }));
    }

    fn send(&mut self, text: String, cx: &mut Context<Self>) {
        self.dispatch(text, None, true, cx);
    }

    fn start_flow(&mut self, flow: Flow, chips: Vec<Chip>, cx: &mut Context<Self>) {
        self.start_flow_with(flow, chips, None, cx);
    }

    fn start_flow_with(&mut self, flow: Flow, chips: Vec<Chip>, text: Option<String>, cx: &mut Context<Self>) {
        for c in chips {
            self.removed.remove(&c);
            if !self.extra.contains(&c) {
                self.extra.push(c);
            }
        }
        if text.is_some() && flow == Flow::Inbox {
            self.hint = Some("The filing window of this file takes your propose_inbox result directly. Afterwards tell the user in one or two sentences what you filled in and that they check it in the filing window and press Ablegen.");
        }
        self.dispatch(text.unwrap_or_else(|| flow.prompt()), Some(flow), true, cx);
    }

    /// Sends a message; `shown` puts it in the stream with its chips, otherwise it goes to the agent alone, e.g. what the user took from a setup card.
    fn dispatch(&mut self, text: String, flow: Option<Flow>, shown: bool, cx: &mut Context<Self>) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.session.is_none() {
            self.start(cx);
        }
        let chips = if shown { self.chips(cx) } else { Vec::new() };
        let mut lines = context_lines(&chips, cx);
        let source = chips.iter().find_map(|c| match c {
            Chip::Note(p) | Chip::Pdf(p, _) | Chip::Pages(p, ..) => Some(p.clone()),
            _ => None,
        });
        if flow == Some(Flow::Cards)
            && let Some(src) = &source
        {
            lines.push(format!("- Cards go to: {}", state::read(cx, |l| l.relative_path(&tools::card_note(l, src)))));
        }
        let mut prompt = text.clone();
        if let Some(recap) = self.recap.take() {
            prompt = format!("[Earlier conversation of this user, continued now]\n{recap}\n[New message]\n{prompt}");
        }
        if !lines.is_empty() {
            prompt.push_str(&format!("\n\n[Context from Schrumbi]\n{}", lines.join("\n")));
        }
        if let Some(f) = flow {
            prompt.push_str(&format!("\n\n[How Schrumbi wants this done]\n{}", f.guide()));
        }
        if let Some(hint) = self.hint.take() {
            prompt.push_str(&format!("\n{hint}"));
        }
        let labels: Vec<String> = chips.iter().map(|c| chip_label(c, cx)).collect();
        let Some(session) = self.session.as_ref() else { return };
        if session.commands.try_send(Command::Prompt(prompt)).is_err() {
            return;
        }
        if shown {
            self.log(vec![transcript::Entry { role: transcript::Role::User, text: text.clone(), at: chrono::Utc::now().timestamp() }], cx);
            self.items.push(Item::User { text: text.clone(), chips: labels });
            self.last_prompt = Some(text);
            self.extra.clear();
        }
        self.pending.push_back(TurnInfo { flow, source });
        if self.turn {
            self.queued += 1;
        } else {
            self.turn = true;
            self.begin(cx);
        }
        self.idle = None;
        self.list.scroll_to_end();
        cx.notify();
    }

    fn begin(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.session.as_ref().map(|s| s.id.clone()) else { return };
        ai::begin_turn(&id, cx);
        let read_only = self.pending.front().and_then(|t| t.flow).is_some_and(Flow::read_only);
        ai::set_read_only(&id, read_only, cx);
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &self.session else { return };
        if !self.turn {
            return;
        }
        let _ = s.commands.try_send(Command::Cancel);
        self.cancelling = true;
        self.turn = false;
        self.queued = 0;
        let session = s.id.clone();
        self.pending.clear();
        ai::set_read_only(&session, false, cx);
        for id in std::mem::take(&mut self.asks) {
            ai::answer_ask(id, Answer::Decline, cx);
        }
        ai::decline_session(&session, cx);
        self.reparse_now();
        self.notice(tr("Abgebrochen."), false);
        cx.notify();
    }

    fn log(&mut self, entries: Vec<transcript::Entry>, cx: &mut Context<Self>) {
        let Some(file) = self.session.as_ref().and_then(|s| s.file.clone()) else { return };
        if entries.is_empty() {
            return;
        }
        let before = self.log_task.take();
        self.log_task = Some(cx.background_executor().spawn(async move {
            if let Some(before) = before {
                before.await;
            }
            let _ = transcript::append(&file, &entries);
        }));
    }

    /// Writes the steps, answers and changes shown since the last time to the session file, in the order the column shows them.
    fn flush_log(&mut self, cx: &mut Context<Self>) {
        let now = chrono::Utc::now().timestamp();
        let entries: Vec<transcript::Entry> = self.items[self.logged.min(self.items.len())..]
            .iter()
            .filter_map(|item| match item {
                Item::Agent { text, .. } if !text.trim().is_empty() => Some(transcript::Entry { role: transcript::Role::Agent, text: text.clone(), at: now }),
                Item::Tool(t) => Some(transcript::Entry { role: transcript::Role::Tool, text: t.title.clone(), at: now }),
                Item::Edit(e) => Some(transcript::Entry { role: transcript::Role::Tool, text: edit_line(e), at: now }),
                _ => None,
            })
            .collect();
        self.logged = self.items.len();
        self.log(entries, cx);
    }

    fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        let event = match event {
            Event::Update(_) if self.cancelling => return,
            Event::Ask { reply, .. } if self.cancelling => {
                reply.send(false);
                return;
            }
            Event::TurnEnded(_) if self.cancelling => {
                self.cancelling = false;
                self.flush_log(cx);
                if let Some(id) = self.session.as_ref().map(|s| s.id.clone()) {
                    ai::end_turn(&id, cx);
                }
                self.schedule_idle(cx);
                return;
            }
            other => other,
        };
        match event {
            Event::Ready => {
                if let Some(s) = self.session.as_mut() {
                    s.ready = true;
                }
            }
            Event::Update(Update::Text(text)) => {
                match self.items.last_mut() {
                    Some(Item::Agent { text: t, .. }) => {
                        t.push_str(&text);
                        self.touch(self.items.len() - 1);
                    }
                    _ => self.items.push(Item::Agent { text, parsed: None, parsed_len: 0, insert: None }),
                }
                self.schedule_reparse(cx);
            }
            Event::Update(Update::Tool { id, new, patch }) => self.tool(id, new, patch),
            Event::Ask { ask, reply } => self.ask(ask, reply, cx),
            Event::Read { path, line, limit, reply } => {
                let result = state::read(cx, |l| tools::read_file(l, &path, line, limit));
                if result.is_ok()
                    && let Some(text) = state::read(cx, |l| tools::inside_root(l, &path).ok().and_then(|p| tools::text_now(l, &p)))
                    && let Some(s) = self.session.as_mut()
                {
                    s.seen.insert(path, tools::version_of(&text));
                }
                reply.send(result);
            }
            Event::Write { path, content, reply } => self.write(path, content, reply, cx),
            Event::Denied { tool } => {
                if let Some(at) = self.tools.get(&tool).copied()
                    && let Some(Item::Tool(t)) = self.items.get_mut(at)
                {
                    t.denied = true;
                    t.status = Status::Failed;
                    self.touch(at);
                }
            }
            Event::Unsafe => {
                self.stop(cx);
                self.notice(tr("Schrumbi startet dieses Programm nicht: Es lässt sich nicht so einstellen, dass es vor jeder Änderung fragt."), false);
            }
            Event::TurnEnded(result) => self.turn_ended(result, cx),
        }
    }

    fn tool(&mut self, id: String, new: bool, patch: ToolPatch) {
        if patch.name.as_deref().is_some_and(|n| HIDDEN_TOOLS.contains(&n)) {
            return;
        }
        let at = match self.tools.get(&id) {
            Some(at) => *at,
            None if new || patch.title.is_some() => {
                self.items.push(Item::Tool(Tool { name: None, title: String::new(), kind: Kind::Other, status: Status::Pending, input: None, output: None, denied: false }));
                self.tools.insert(id, self.items.len() - 1);
                self.items.len() - 1
            }
            None => return,
        };
        self.touch(at);
        let Some(Item::Tool(t)) = self.items.get_mut(at) else { return };
        if let Some(n) = patch.name {
            t.name = Some(n);
        }
        if let Some(k) = patch.kind {
            t.kind = k;
        }
        if let Some(s) = patch.status {
            t.status = if t.denied { Status::Failed } else { s };
        }
        if let Some(i) = patch.input.filter(|i| i.as_object().is_none_or(|o| !o.is_empty())) {
            t.input = Some(i);
        }
        if let Some(o) = patch.output {
            t.output = Some(o);
        }
        let fallback = patch.title.unwrap_or_else(|| t.title.clone());
        t.title = title_for(t.name.as_deref(), t.kind, t.input.as_ref(), &fallback);
    }

    fn ask(&mut self, ask: acp::Ask, reply: acp::Reply<bool>, cx: &mut Context<Self>) {
        let kind = ask.patch.kind.unwrap_or(Kind::Other);
        let name = ask.patch.name.clone();
        let input = ask.patch.input.clone().or_else(|| self.tools.get(&ask.tool).and_then(|i| if let Some(Item::Tool(t)) = self.items.get(*i) { t.input.clone() } else { None }));
        let title = title_for(name.as_deref(), kind, input.as_ref(), ask.patch.title.as_deref().unwrap_or_default());
        let lines = ask.patch.diffs.as_deref().map(diff_lines).unwrap_or_default();
        let key = name.unwrap_or_else(|| format!("{kind:?}"));
        let places: Vec<PathBuf> = ask.patch.locations.iter().flatten().map(|(p, _)| p.clone()).chain(ask.patch.diffs.iter().flatten().map(|d| d.path.clone())).collect();
        let (ask_id, answer) = ai::ask_change(&key, Preview { title, lines }, cx);
        self.asks.extend(ask_id);
        cx.spawn(async move |this, cx| {
            let allow = matches!(answer.recv().await, Ok(Answer::Allow | Answer::Today));
            let _ = this.update(cx, |p, _| {
                p.asks.retain(|a| Some(*a) != ask_id);
                if allow && let Some(s) = p.session.as_mut() {
                    s.granted.extend(places);
                }
            });
            reply.send(allow);
        })
        .detach();
    }

    fn write(&mut self, path: PathBuf, content: String, reply: acp::Reply<Result<(), String>>, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else {
            reply.send(Err("The session ended.".into()));
            return;
        };
        let granted = session.granted.remove(&path);
        let (id, seen) = (session.id.clone(), session.seen.get(&path).cloned());
        let preview = match state::read(cx, |l| tools::write_preview(l, &path, &content)) {
            Ok(p) => p,
            Err(e) => {
                reply.send(Err(e));
                return;
            }
        };
        let answer = if granted {
            let (tx, rx) = async_channel::bounded(1);
            let _ = tx.try_send(Answer::Allow);
            rx
        } else {
            let (ask_id, rx) = ai::ask_change("write_file", preview, cx);
            self.asks.extend(ask_id);
            rx
        };
        cx.spawn(async move |this, cx| {
            if !matches!(answer.recv().await, Ok(Answer::Allow | Answer::Today)) {
                reply.send(Err("The user declined this change in Schrumbi. Nothing was written.".into()));
                return;
            }
            let result = this.update(cx, |p, cx| {
                let before = state::read(cx, |l| tools::text_now(l, &path));
                let out = ai::run_for(&id, cx, |l| tools::write_file(l, &path, &content, seen.as_deref()));
                if out.is_ok()
                    && let Some(after) = state::read(cx, |l| tools::text_now(l, &path))
                {
                    if let Some(s) = p.session.as_mut() {
                        s.seen.insert(path.clone(), tools::version_of(&after));
                    }
                    p.record_edit(&id, path.clone(), before, after, cx);
                }
                out.map(drop)
            });
            reply.send(result.unwrap_or_else(|_| Err("Schrumbi closed the column.".into())));
        })
        .detach();
    }

    /// Shows a change the agent made in this session as a card, and follows it when „Folgen“ is on.
    pub fn record_edit(&mut self, session: &str, path: PathBuf, before: Option<String>, after: String, cx: &mut Context<Self>) {
        if self.session.as_ref().is_none_or(|s| s.id != session) {
            return;
        }
        let (added, removed) = counts(before.as_deref().unwrap_or_default(), &after);
        if added == 0 && removed == 0 {
            return;
        }
        if prefs::with(|p| p.ai.follow_edits) && path.exists() {
            let lines = changed_lines(before.as_deref().unwrap_or_default(), &after);
            let target = path.clone();
            act(cx, move |l| {
                if !lines.is_empty() {
                    l.changed_lines = Some((target.clone(), lines));
                }
                l.quiet_open = true;
                l.open_file(&target);
            });
        }
        self.items.push(Item::Edit(Edit { path, before, after, added, removed, undone: false, stale: false }));
        cx.notify();
    }

    fn turn_ended(&mut self, result: Result<Stop, String>, cx: &mut Context<Self>) {
        let Some(id) = self.session.as_ref().map(|s| s.id.clone()) else { return };
        ai::end_turn(&id, cx);
        ai::set_read_only(&id, false, cx);
        ai::release_fills(cx);
        let done = self.pending.pop_front().unwrap_or_default();
        if done.flow == Some(Flow::Explain)
            && result == Ok(Stop::Done)
            && let Some(src) = done.source
        {
            let note = if crate::core::markdown::is_markdown(&src) { src } else { src.with_extension("md") };
            if let Some(at) = self.items.iter().rposition(|i| matches!(i, Item::Agent { .. })).filter(|at| !self.items[*at..].iter().any(|i| matches!(i, Item::User { .. })))
                && let Some(Item::Agent { insert, .. }) = self.items.get_mut(at)
            {
                *insert = Some(Insert { note, done: false });
                self.dirty.insert(at);
            }
        }
        match result {
            Ok(Stop::Done) => {}
            Ok(Stop::Cancelled) => self.notice(tr("Abgebrochen."), false),
            Ok(Stop::Refused) => self.notice(tr("Die KI hat das abgelehnt."), false),
            Ok(Stop::Limit) => self.notice(tr("Die Antwort wurde unterwegs gekürzt."), false),
            Err(e) => self.notice(trf("Das hat nicht geklappt: %@", &[&e.chars().take(200).collect::<String>()]), false),
        }
        let shell_off = !prefs::with(|p| p.ai.allow_execute);
        let mut touched = Vec::new();
        for (at, item) in self.items.iter_mut().enumerate() {
            if let Item::Tool(t) = item
                && matches!(t.status, Status::Pending | Status::Running)
            {
                t.denied = shell_off && (t.kind == Kind::Execute || t.name.as_deref().is_some_and(|n| matches!(n, "Bash" | "PowerShell")));
                t.status = Status::Failed;
                touched.push(at);
            }
        }
        self.dirty.extend(touched);
        self.flush_log(cx);
        self.reparse_now();
        if self.queued > 0 {
            self.queued -= 1;
            self.begin(cx);
        } else {
            self.turn = false;
            self.schedule_idle(cx);
        }
    }

    fn schedule_reparse(&mut self, cx: &mut Context<Self>) {
        if self.reparse.is_some() {
            return;
        }
        let grown = self.items.last().map_or(0, |i| if let Item::Agent { text, .. } = i { text.len() } else { 0 });
        let wait = REPARSE + Duration::from_millis(u64::try_from(grown / 40).unwrap_or(u64::MAX).min(600));
        self.reparse = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |p, cx| {
                p.reparse = None;
                p.reparse_now();
                cx.notify();
            });
        }));
    }

    fn reparse_now(&mut self) {
        let count = self.items.len();
        let mut touched = Vec::new();
        for (back, item) in self.items.iter_mut().rev().take(4).enumerate() {
            if let Item::Agent { text, parsed, parsed_len, .. } = item
                && *parsed_len != text.len()
            {
                *parsed = Some(Rc::new(markdown_view::parse(text)));
                *parsed_len = text.len();
                touched.push(count - 1 - back);
            }
        }
        self.dirty.extend(touched);
    }

    fn undo_edit(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(Item::Edit(e)) = self.items.get(at) else { return };
        let (Some(before), path, after) = (e.before.clone(), e.path.clone(), e.after.clone()) else { return };
        self.touch(at);
        let now = state::read(cx, |l| tools::text_now(l, &path));
        if now.as_deref() != Some(after.as_str()) {
            if let Some(Item::Edit(e)) = self.items.get_mut(at) {
                e.stale = true;
            }
            cx.notify();
            return;
        }
        let done = act(cx, |l| {
            let key = l.buffer_key(&path);
            let kind = format!("ai-undo:{}", key.display());
            l.edit_note(&key, &kind, trf("KI-Änderung an %@ zurückgenommen", &[&name_of(&path)]), |_| before)
        });
        if done && let Some(Item::Edit(e)) = self.items.get_mut(at) {
            e.undone = true;
        }
        cx.notify();
    }

    /// Shows the agent's setup proposal as a card; an earlier open card is marked as replaced.
    pub fn record_setup(&mut self, session: &str, proposal: Value, cx: &mut Context<Self>) {
        if self.session.as_ref().is_none_or(|s| s.id != session) {
            return;
        }
        let Ok(plan) = state::read(cx, |l| setup::plan(l, &proposal)) else { return };
        let mut touched = Vec::new();
        for (at, item) in self.items.iter_mut().enumerate() {
            if let Item::Setup(card) = item
                && card.state == CardState::Open
            {
                card.state = CardState::Replaced;
                touched.push(at);
            }
        }
        self.dirty.extend(touched);
        let chosen = plan.items.iter().map(|i| i.key.clone()).collect();
        self.items.push(Item::Setup(SetupCard { plan, chosen, state: CardState::Open, renames_open: false, error: None }));
        cx.notify();
    }

    /// Shows the agent's reading of inbox files as a table to take row by row.
    pub fn record_inbox(&mut self, session: &str, rows: Vec<crate::core::ai::inbox::Row>, cx: &mut Context<Self>) {
        if self.session.as_ref().is_none_or(|s| s.id != session) || rows.is_empty() {
            return;
        }
        let mut touched = Vec::new();
        for (at, item) in self.items.iter_mut().enumerate() {
            if let Item::Inbox(card) = item
                && card.state == CardState::Open
            {
                card.state = CardState::Replaced;
                touched.push(at);
            }
        }
        self.dirty.extend(touched);
        let chosen = (0..rows.len()).collect();
        self.items.push(Item::Inbox(InboxCard { rows, chosen, edited: HashSet::new(), state: CardState::Open, error: None }));
        cx.notify();
    }

    fn apply_inbox(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(Item::Inbox(card)) = self.items.get(at) else { return };
        if card.state != CardState::Open {
            return;
        }
        let rows: Vec<crate::core::ai::inbox::Row> = card.rows.iter().enumerate().filter(|(i, _)| card.chosen.contains(i) && !card.edited.contains(i)).map(|(_, r)| r.clone()).collect();
        if rows.is_empty() {
            return;
        }
        let total = card.rows.len();
        let session = self.session.as_ref().map_or_else(|| "inbox".to_string(), |s| s.id.clone());
        let result = ai::run_for(&session, cx, |l| crate::core::ai::inbox::apply(l, &rows));
        self.dirty.insert(at);
        if let Some(Item::Inbox(card)) = self.items.get_mut(at) {
            match result {
                Ok(_) => {
                    card.state = CardState::Applied(rows.len(), total);
                    card.error = None;
                }
                Err(e) => card.error = Some(e),
            }
        }
        cx.notify();
    }

    fn edit_inbox_row(&mut self, at: usize, row: usize, cx: &mut Context<Self>) {
        let Some(Item::Inbox(card)) = self.items.get_mut(at) else { return };
        let Some(r) = card.rows.get(row).cloned() else { return };
        card.edited.insert(row);
        self.dirty.insert(at);
        let (path, deadline) = (r.path.clone(), r.deadline.clone());
        ai::keep_prefill(r, cx);
        act(cx, |l| l.request_filing(vec![path], deadline));
        cx.notify();
    }

    fn inbox_card(&self, at: usize, card: &InboxCard, cx: &mut Context<Self>, t: &Theme) -> AnyElement {
        let me = cx.entity();
        let open = card.state == CardState::Open;
        let head = match card.state {
            CardState::Open => tr("So würde ich ablegen"),
            CardState::Applied(n, total) => trf("Abgelegt: %lld von %lld Dateien", &[&n, &total]),
            CardState::Replaced => tr("Durch einen neuen Vorschlag ersetzt"),
            CardState::Dismissed => tr("Verworfen"),
        };
        let mut rows = div().flex().flex_col().gap(px(space::S));
        for (i, r) in card.rows.iter().enumerate() {
            let edited = card.edited.contains(&i);
            let checked = card.chosen.contains(&i) && !edited;
            let (me_check, me_edit) = (me.clone(), me.clone());
            let module = r.module.as_deref().and_then(|m| state::read(cx, |l| l.module_by_id(m).map(|m| m.code.clone()))).unwrap_or_default();
            let mut facts = vec![r.kind.label()];
            if !module.is_empty() {
                facts.insert(0, module);
            }
            if let Some(d) = r.date {
                facts.push(d.format("%d.%m.%Y").to_string());
            }
            let deadline = r.deadline.as_deref().and_then(|id| state::read(cx, |l| l.assignment(id).map(|a| a.title.clone()))).map(|d| trf("Frist: %@", &[&d])).or_else(|| r.new_deadline.as_ref().map(|n| trf("Neue Frist: %@ bis %@", &[&n.title, &n.due.format("%d.%m. %H:%M").to_string()])));
            let target = format!("{}/{}", state::read(cx, |l| l.relative_path(&r.folder)), r.name);
            rows = rows.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(space::S))
                    .when(!open && !checked, |d| d.opacity(0.5))
                    .child(Checkbox::new(SharedString::from(format!("ai-inbox-{at}-{i}"))).checked(checked).disabled(!open || edited).on_click(move |v, _, cx| {
                        let on = *v;
                        me_check.update(cx, |p, cx| {
                            if let Some(Item::Inbox(c)) = p.items.get_mut(at) {
                                if on {
                                    c.chosen.insert(i);
                                } else {
                                    c.chosen.remove(&i);
                                }
                            }
                            p.dirty.insert(at);
                            cx.notify();
                        });
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(space::XXS))
                            .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(name_of(&r.path)))
                            .child(div().min_w_0().truncate().text_size(px(font::CALLOUT)).text_color(t.secondary).child(facts.join(" · ")))
                            .children(deadline.map(|d| div().min_w_0().truncate().text_size(px(font::CALLOUT)).text_color(t.secondary).child(d)))
                            .child(div().min_w_0().truncate().font_family(Theme::mono_family()).text_size(px(font::CAPTION)).text_color(t.tertiary).child(if edited { tr("Im Ablegen-Fenster") } else { format!("→ {target}") })),
                    )
                    .when(open && !edited, |d| d.child(Button::new(SharedString::from(format!("ai-inbox-edit-{at}-{i}"))).ghost().xsmall().label(tr("Bearbeiten")).tooltip(tr("Im Ablegen-Fenster ändern")).on_click(move |_, _, cx| me_edit.update(cx, |p, cx| p.edit_inbox_row(at, i, cx))))),
            );
        }
        let chosen = card.rows.iter().enumerate().filter(|(i, _)| card.chosen.contains(i) && !card.edited.contains(i)).count();
        let (me_apply, me_dismiss) = (me.clone(), me.clone());
        let actions = open.then(|| {
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(space::S))
                .child(Button::new(("ai-inbox-dismiss", at)).ghost().small().label(tr("Verwerfen")).on_click(move |_, _, cx| {
                    me_dismiss.update(cx, |p, cx| {
                        if let Some(Item::Inbox(c)) = p.items.get_mut(at) {
                            c.state = CardState::Dismissed;
                        }
                        p.dirty.insert(at);
                        cx.notify();
                    });
                }))
                .child(Button::new(("ai-inbox-apply", at)).primary().small().label(if chosen == 1 { tr("1 Datei ablegen") } else { trf("%lld Dateien ablegen", &[&chosen]) }).disabled(chosen == 0).on_click(move |_, _, cx| me_apply.update(cx, |p, cx| p.apply_inbox(at, cx))))
        });
        super::drive::marked(div().id(("ai-inbox", at)), "dialog")
            .flex()
            .flex_col()
            .gap(px(space::M))
            .p(px(space::M))
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(if open { t.accent.opacity(0.5) } else { t.separator })
            .when(open, |d| d.bg(t.accent.opacity(0.04)))
            .child(div().flex().items_center().gap(px(space::S)).child(icon(Flow::Inbox.icon(), iconsize::S, t.secondary)).child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(head)))
            .when(card.state != CardState::Replaced, |d| d.child(rows))
            .children(card.error.clone().map(|e| div().text_size(px(font::CALLOUT)).text_color(t.secondary).child(e)))
            .children(actions)
            .into_any_element()
    }

    fn toggle_setup_item(&mut self, at: usize, key: &str, on: bool, cx: &mut Context<Self>) {
        if let Some(Item::Setup(card)) = self.items.get_mut(at)
            && card.state == CardState::Open
        {
            if on {
                card.chosen.insert(key.to_string());
            } else {
                card.chosen.remove(key);
            }
            self.dirty.insert(at);
            cx.notify();
        }
    }

    fn apply_setup(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(Item::Setup(card)) = self.items.get(at) else { return };
        if card.state != CardState::Open {
            return;
        }
        let plan = card.plan.clone();
        let keys: Vec<String> = plan.items.iter().filter(|i| card.chosen.contains(&i.key)).map(|i| i.key.clone()).collect();
        if keys.is_empty() {
            return;
        }
        let session = self.session.as_ref().map(|s| s.id.clone());
        let result = ai::run_for(session.as_deref().unwrap_or("setup"), cx, |l| setup::apply(l, &plan, &keys));
        self.dirty.insert(at);
        let Some(Item::Setup(card)) = self.items.get_mut(at) else { return };
        match result {
            Ok(_) => {
                card.state = CardState::Applied(keys.len(), plan.items.len());
                card.error = None;
                let taken: Vec<&str> = plan.items.iter().filter(|i| keys.contains(&i.key)).map(|i| i.title.as_str()).collect();
                let left: Vec<&str> = plan.items.iter().filter(|i| !keys.contains(&i.key)).map(|i| i.title.as_str()).collect();
                if session.is_some() && !self.turn {
                    self.notice(if keys.len() == 1 { tr("1 Punkt übernommen.") } else { trf("%lld Punkte übernommen.", &[&keys.len()]) }, false);
                    let note = format!("[Schrumbi] The user applied these setup items: {}. Not taken: {}. Ask in one short sentence whether everything fits now; propose again only if they want changes.", taken.join("; "), if left.is_empty() { "none".to_string() } else { left.join("; ") });
                    self.dispatch(note, None, false, cx);
                }
            }
            Err(e) => card.error = Some(e),
        }
        cx.notify();
    }

    fn dismiss_setup(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(Item::Setup(card)) = self.items.get_mut(at)
            && card.state == CardState::Open
        {
            card.state = CardState::Dismissed;
            self.dirty.insert(at);
            cx.notify();
        }
    }

    fn insert_answer(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(Item::Agent { text, insert: Some(target), .. }) = self.items.get(at) else { return };
        if target.done {
            return;
        }
        let (note, body) = (target.note.clone(), text.trim().to_string());
        let done = act(cx, |l| {
            let (path, made) = if note.exists() {
                (note.clone(), false)
            } else {
                let folder = note.parent().map(Path::to_path_buf).unwrap_or_default();
                let stem = note.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                match l.create_note(&stem, &folder, "md") {
                    Some(p) => (p, true),
                    None => return false,
                }
            };
            let key = l.buffer_key(&path);
            let label = trf("Erklärung in %@ eingefügt", &[&name_of(&path)]);
            let written = l.edit_note(&key, "ai-insert", label.clone(), |before| {
                let mut out = before.to_string();
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                if !out.is_empty() && !out.ends_with("\n\n") {
                    out.push('\n');
                }
                out.push_str(&body);
                out.push('\n');
                out
            });
            if written && made {
                l.join_latest(2, "square-pen", label);
            }
            written
        });
        if done && let Some(Item::Agent { insert: Some(target), .. }) = self.items.get_mut(at) {
            target.done = true;
            self.dirty.insert(at);
        }
        cx.notify();
    }

    fn open_past(&mut self, file: PathBuf, cx: &mut Context<Self>) {
        let entries = transcript::load(&file);
        if entries.is_empty() {
            return;
        }
        self.new_conversation_quiet(cx);
        let when = transcript::started(&file).map(|d| d.format("%d.%m.%Y %H:%M").to_string()).unwrap_or_default();
        self.notice(trf("Unterhaltung vom %@. Schreib weiter, dann liest die KI sie mit.", &[&when]), false);
        for e in &entries {
            match e.role {
                transcript::Role::User => self.items.push(Item::User { text: e.text.clone(), chips: Vec::new() }),
                transcript::Role::Agent => self.items.push(Item::Agent { text: e.text.clone(), parsed: Some(Rc::new(markdown_view::parse(&e.text))), parsed_len: e.text.len(), insert: None }),
                transcript::Role::Tool => self.items.push(Item::Tool(Tool { name: None, title: e.text.clone(), kind: Kind::Other, status: Status::Done, input: None, output: None, denied: false })),
            }
        }
        self.logged = self.items.len();
        self.recap = Some(transcript::recap(&entries, RECAP_CHARS));
        self.focus_input = true;
        cx.notify();
    }

    fn save_as_note(&mut self, cx: &mut Context<Self>) {
        let mut entries = Vec::new();
        for item in &self.items {
            match item {
                Item::User { text, .. } => entries.push(transcript::Entry { role: transcript::Role::User, text: text.clone(), at: 0 }),
                Item::Agent { text, .. } => entries.push(transcript::Entry { role: transcript::Role::Agent, text: text.clone(), at: 0 }),
                Item::Tool(t) if !t.denied => entries.push(transcript::Entry { role: transcript::Role::Tool, text: t.title.clone(), at: 0 }),
                Item::Edit(e) => entries.push(transcript::Entry { role: transcript::Role::Tool, text: edit_line(e), at: 0 }),
                _ => {}
            }
        }
        if entries.is_empty() {
            return;
        }
        let title = trf("KI-Sitzung %@", &[&chrono::Local::now().format("%d.%m.%Y %H.%M").to_string()]);
        let body = transcript::markdown(&title, &entries);
        let folder = state::read(cx, |l| {
            let doc = l.active_document.clone();
            doc.as_deref().and_then(|d| l.module_containing(d)).map(|m| m.id.clone()).and_then(|id| l.suggested_folders(Some(&id)).into_iter().next()).or_else(|| l.root.clone())
        });
        let Some(folder) = folder else { return };
        act(cx, |l| {
            if let Some(path) = l.create_note(&title, &folder, "md") {
                let key = l.buffer_key(&path);
                let label = trf("%@ angelegt", &[&name_of(&path)]);
                if l.edit_note(&key, "ai", label.clone(), |_| body) {
                    l.join_latest(2, "square-pen", label);
                }
                l.open_document(&path);
            }
        });
    }

    fn new_conversation(&mut self, cx: &mut Context<Self>) {
        self.new_conversation_quiet(cx);
        self.start(cx);
        self.focus_input = true;
        cx.notify();
    }

    fn new_conversation_quiet(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        self.pending.clear();
        self.recap = None;
        self.items.clear();
        self.logged = 0;
        self.list = Self::new_list();
        self.listed = 0;
        self.dirty.clear();
        self.tools.clear();
        self.expanded.clear();
        self.extra.clear();
        self.removed.clear();
    }

    fn header(&self, cx: &mut Context<Self>, t: &Theme) -> Div {
        let p = prefs::with(|p| p.ai.clone());
        let runners = agents::runners(&p);
        let chosen = agents::chosen(&p);
        let me = cx.entity();
        let (me2, me3) = (me.clone(), me.clone());
        let has_items = !self.items.is_empty();
        let mut head = div().h(px(super::theme::control::SUBBAR)).flex_none().flex().items_center().gap(px(space::XS)).pl(px(space::S)).pr(px(space::S)).border_b_1().border_color(t.separator).child(ai::panel_tabs(true, t, cx));
        if runners.len() > 1 {
            let label = chosen.map(|r| tr(r.label())).unwrap_or_default();
            let me = me.clone();
            head = head.child(Button::new("ai-agent").ghost().xsmall().label(label).dropdown_menu(move |mut menu, _, _| {
                for r in runners.clone() {
                    let me = me.clone();
                    menu = menu.item(PopupMenuItem::new(tr(r.label())).checked(Some(r) == chosen).on_click(move |_, _, cx| {
                        prefs::update(|p| p.ai.agent = r.key().to_string());
                        me.update(cx, |panel, cx| panel.new_conversation(cx));
                    }));
                }
                menu
            }));
        }
        let root = state::read(cx, |l| l.root.clone());
        let me4 = me.clone();
        let history = Button::new("ai-history").ghost().size(px(super::theme::control::S)).rounded_full().icon(menu_icon("history")).tooltip(tr("Frühere Unterhaltungen")).dropdown_menu(move |mut menu, _, _| {
            let past = root.as_deref().map(|r| transcript::list(r, HISTORY_SHOWN)).unwrap_or_default();
            if past.is_empty() {
                return menu.item(PopupMenuItem::new(tr("Noch keine früheren Unterhaltungen")).disabled(true));
            }
            for p in past {
                let me = me4.clone();
                let when = p.started.map(|d| d.format("%d.%m. %H:%M").to_string()).unwrap_or_default();
                let first: String = p.first.lines().next().unwrap_or_default().chars().take(48).collect();
                let file = p.file.clone();
                menu = menu.item(PopupMenuItem::new(format!("{when}  {first}")).on_click(move |_, _, cx| {
                    let file = file.clone();
                    me.update(cx, |panel, cx| panel.open_past(file, cx));
                }));
            }
            menu
        });
        head.child(div().flex_1())
            .child(history)
            .child(Button::new("ai-save").ghost().size(px(super::theme::control::S)).rounded_full().icon(menu_icon("file-down")).disabled(!has_items).tooltip(tr("Als Notiz speichern")).on_click(move |_, _, cx| me2.update(cx, |p, cx| p.save_as_note(cx))))
            .child(Button::new("ai-new").ghost().size(px(super::theme::control::S)).rounded_full().icon(menu_icon("message-square-plus")).disabled(!has_items).tooltip(tr("Neue Unterhaltung")).on_click(move |_, _, cx| me3.update(cx, |p, cx| p.new_conversation(cx))))
            .child(Button::new("ai-close").ghost().size(px(super::theme::control::S)).rounded_full().icon(menu_icon("x")).tooltip(format!("{}{}", tr("KI ausblenden"), crate::core::shortcuts::ShortcutAction::ToggleAi.hint())).on_click(|_, _, cx| ai::close_panel(cx)))
    }

    fn empty(&self, cx: &mut Context<Self>, t: &Theme) -> Div {
        let me = cx.entity();
        let mut list = div().flex().flex_col().gap(px(space::XS)).w_full();
        let mut offered = suggestions(cx);
        if setup_wanted(cx) {
            offered.push(Suggestion { label: Flow::Setup.label(), icon: Flow::Setup.icon(), flow: Flow::Setup, chips: Vec::new() });
        }
        for (i, Suggestion { label, icon: icon_name, flow, chips }) in offered.into_iter().take(4).enumerate() {
            let me = me.clone();
            list = list.child(
                div()
                    .id(("ai-suggest", i))
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .px(px(space::M))
                    .py(px(space::S))
                    .rounded(px(radius::CONTROL))
                    .border_1()
                    .border_color(t.separator)
                    .cursor_pointer()
                    .key_nav()
                    .soft(t.ink.opacity(0.05), radius::CONTROL)
                    .child(icon(icon_name, iconsize::S, t.secondary))
                    .child(div().flex_1().truncate().child(label))
                    .on_click(move |_, _, cx| {
                        let chips = chips.clone();
                        me.update(cx, |p, cx| p.start_flow(flow, chips, cx));
                    }),
            );
        }
        let starting = self.session.as_ref().is_some_and(|s| !s.ready);
        div()
            .flex()
            .flex_col()
            .gap(px(space::M))
            .p(px(space::L))
            .child(div().text_color(t.secondary).child(tr("Frag etwas zu deinen Notizen, PDFs oder Fristen. Ändern darf die KI nur mit deinem Ja, und alles ist rückgängig.")))
            .child(list)
            .when(starting, |d| d.child(div().flex().items_center().gap(px(space::S)).text_size(px(font::CALLOUT)).text_color(t.tertiary).child(motion::loading(iconsize::XS, t.tertiary)).child(tr("Startet …"))))
    }

    fn tool_card(&self, at: usize, tool: &Tool, cx: &mut Context<Self>, t: &Theme) -> AnyElement {
        let open = self.expanded.contains(&at);
        let me = cx.entity();
        let status = match tool.status {
            _ if tool.denied => icon("ban", iconsize::XS, t.tertiary).into_any_element(),
            Status::Pending | Status::Running => motion::loading(iconsize::XS, t.secondary).into_any_element(),
            Status::Done => icon("check", iconsize::XS, t.tertiary).into_any_element(),
            Status::Failed => icon("x", iconsize::XS, t.tertiary).into_any_element(),
        };
        let title = if tool.denied { trf("%@ (nicht erlaubt)", &[&tool.title]) } else { tool.title.clone() };
        let has_detail = tool.input.is_some() || tool.output.is_some();
        let mut card = div().flex().flex_col().child(
            div()
                .id(("ai-tool", at))
                .flex()
                .items_center()
                .gap(px(space::S))
                .px(px(space::S))
                .py(px(space::XS))
                .rounded(px(radius::INNER))
                .text_size(px(font::CALLOUT))
                .text_color(t.secondary)
                .when(has_detail, |d| d.cursor_pointer().key_nav().soft(t.ink.opacity(0.05), radius::INNER))
                .child(icon(tool_icon(tool.name.as_deref(), tool.kind), iconsize::XS, t.tertiary))
                .child(div().flex_1().min_w_0().truncate().child(title))
                .child(status)
                .when(has_detail, |d| d.child(motion::chevron(("ai-tool-chevron", at), open, t.tertiary)))
                .on_click(move |_, _, cx| {
                    me.update(cx, |p, cx| {
                        if !p.expanded.remove(&at) {
                            p.expanded.insert(at);
                        }
                        p.touch(at);
                        cx.notify();
                    });
                }),
        );
        if open {
            let mut raw = String::new();
            if let Some(i) = &tool.input {
                raw.push_str(&serde_json::to_string_pretty(i).unwrap_or_default());
            }
            if let Some(o) = &tool.output {
                if !raw.is_empty() {
                    raw.push_str("\n\n");
                }
                let cut: String = o.chars().take(SHOWN_OUTPUT).collect();
                raw.push_str(&cut);
                if o.chars().count() > SHOWN_OUTPUT {
                    raw.push('…');
                }
            }
            card = card.child(div().ml(px(space::XL)).mt(px(space::XXS)).p(px(space::S)).rounded(px(radius::INNER)).bg(t.ink.opacity(0.04)).font_family(Theme::mono_family()).text_size(px(font::CAPTION)).text_color(t.secondary).child(raw));
        }
        card.into_any_element()
    }

    fn edit_card(&self, at: usize, edit: &Edit, cx: &mut Context<Self>, t: &Theme) -> AnyElement {
        let me = cx.entity();
        let (path, before, after) = (edit.path.clone(), edit.before.clone().unwrap_or_default(), edit.after.clone());
        let can_undo = edit.before.is_some() && !edit.undone && !edit.stale;
        let undo_tip = if edit.undone { tr("Schon zurückgenommen") } else if edit.before.is_none() { tr("Eine neu angelegte Datei nimmt das normale Rückgängig zurück") } else if edit.stale { tr("Die Datei hat sich seitdem geändert") } else { tr("Nur diese Änderung zurücknehmen") };
        div()
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::M))
            .py(px(space::S))
            .rounded(px(radius::CONTROL))
            .border_1()
            .border_color(t.separator)
            .child(icon("pencil", iconsize::XS, t.secondary))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .gap(px(space::XS))
                    .text_size(px(font::CALLOUT))
                    .child(div().flex_none().text_color(t.green_text).font_features(tabular()).child(format!("+{}", edit.added)))
                    .child(div().flex_none().text_color(t.secondary).font_features(tabular()).child(format!("−{}", edit.removed)))
                    .child(div().min_w_0().truncate().child(trf("in %@", &[&name_of(&edit.path)])))
                    .when(edit.undone, |d| d.text_color(t.tertiary)),
            )
            .child(Button::new(("ai-edit-view", at)).ghost().xsmall().label(tr("Ansehen")).on_click({
                let me = me.clone();
                move |_, window, cx| {
                    let me = me.clone();
                    let discard: Option<Rc<dyn Fn(&mut App)>> = can_undo.then(|| Rc::new(move |cx: &mut App| me.update(cx, |p, cx| p.undo_edit(at, cx))) as Rc<dyn Fn(&mut App)>);
                    show_diff(&path, &before, &after, discard, window, cx);
                }
            }))
            .child(Button::new(("ai-edit-undo", at)).ghost().xsmall().label(tr("Rückgängig")).disabled(!can_undo).tooltip(undo_tip).on_click(move |_, _, cx| me.update(cx, |p, cx| p.undo_edit(at, cx))))
            .into_any_element()
    }

    fn setup_card(&self, at: usize, card: &SetupCard, cx: &mut Context<Self>, t: &Theme) -> AnyElement {
        let me = cx.entity();
        let open = card.state == CardState::Open;
        let head = match card.state {
            CardState::Open => tr("Vorschlag zur Einrichtung"),
            CardState::Applied(n, total) => trf("Übernommen: %lld von %lld Punkten", &[&n, &total]),
            CardState::Replaced => tr("Durch einen neuen Vorschlag ersetzt"),
            CardState::Dismissed => tr("Verworfen"),
        };
        let mut rows = div().flex().flex_col().gap(px(space::S));
        for (i, item) in card.plan.items.iter().enumerate() {
            let (me, key) = (me.clone(), item.key.clone());
            let checked = card.chosen.contains(&item.key);
            let mut lines = div().flex().flex_col().gap(px(space::XXS)).pl(px(space::XL)).text_size(px(font::CALLOUT)).text_color(t.secondary);
            for line in &item.lines {
                lines = lines.child(div().min_w_0().truncate().child(line.clone()));
            }
            rows = rows.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::XXS))
                    .when(!open && !checked, |d| d.opacity(0.5))
                    .child(Checkbox::new(SharedString::from(format!("ai-setup-{at}-{i}"))).label(item.title.clone()).checked(checked).disabled(!open).on_click(move |v, _, cx| {
                        let (key, on) = (key.clone(), *v);
                        me.update(cx, |p, cx| p.toggle_setup_item(at, &key, on, cx));
                    }))
                    .when(!item.lines.is_empty(), |d| d.child(lines)),
            );
        }
        let renames = if card.plan.renames.is_empty() {
            None
        } else {
            let me = me.clone();
            let shown = card.renames_open;
            let mut list = div().flex().flex_col().gap(px(space::XXS)).font_family(Theme::mono_family()).text_size(px(font::CAPTION)).text_color(t.secondary);
            if shown {
                for r in &card.plan.renames {
                    list = list.child(div().min_w_0().truncate().child(format!("{} → {}", r.from, r.to)));
                }
            }
            let count = card.plan.renames.len();
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .child(
                        div()
                            .id(("ai-setup-renames", at))
                            .flex()
                            .items_center()
                            .gap(px(space::XS))
                            .text_size(px(font::CALLOUT))
                            .text_color(t.secondary)
                            .cursor_pointer()
                            .key_nav()
                            .soft(t.ink.opacity(0.05), radius::INNER)
                            .child(motion::chevron(("ai-setup-renames-chevron", at), shown, t.tertiary))
                            .child(div().flex_1().min_w_0().child(if count == 1 { tr("1 Datei würde die KI umbenennen. Schrumbi tut das nicht von selbst.") } else { trf("%lld Dateien würde die KI umbenennen. Schrumbi tut das nicht von selbst.", &[&count]) }))
                            .on_click(move |_, _, cx| {
                                me.update(cx, |p, cx| {
                                    if let Some(Item::Setup(c)) = p.items.get_mut(at) {
                                        c.renames_open = !c.renames_open;
                                    }
                                    p.dirty.insert(at);
                                    cx.notify();
                                });
                            }),
                    )
                    .when(shown, |d| d.child(list.p(px(space::S)).rounded(px(radius::INNER)).bg(t.ink.opacity(0.04)))),
            )
        };
        let chosen = card.chosen.len();
        let (me_apply, me_dismiss) = (me.clone(), me.clone());
        let actions = open.then(|| {
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(space::S))
                .child(Button::new(("ai-setup-dismiss", at)).ghost().small().label(tr("Verwerfen")).on_click(move |_, _, cx| me_dismiss.update(cx, |p, cx| p.dismiss_setup(at, cx))))
                .child(Button::new(("ai-setup-apply", at)).primary().small().label(if chosen == card.plan.items.len() { tr("Alles übernehmen") } else { trf("%lld übernehmen", &[&chosen]) }).disabled(chosen == 0).on_click(move |_, _, cx| me_apply.update(cx, |p, cx| p.apply_setup(at, cx))))
        });
        super::drive::marked(div().id(("ai-setup", at)), "dialog")
            .flex()
            .flex_col()
            .gap(px(space::M))
            .p(px(space::M))
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(if open { t.accent.opacity(0.5) } else { t.separator })
            .when(open, |d| d.bg(t.accent.opacity(0.04)))
            .child(div().flex().items_center().gap(px(space::S)).child(icon(Flow::Setup.icon(), iconsize::S, t.secondary)).child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(head)))
            .when(!card.plan.workflow.is_empty() && open, |d| d.child(div().text_size(px(font::CALLOUT)).text_color(t.secondary).child(card.plan.workflow.clone())))
            .when(!card.plan.items.is_empty() && card.state != CardState::Replaced, |d| d.child(rows))
            .children(renames)
            .children(card.error.clone().map(|e| div().text_size(px(font::CALLOUT)).text_color(t.secondary).child(e)))
            .children(actions)
            .into_any_element()
    }

    fn ask_card(&self, w: &ai::Waiting, t: &Theme) -> AnyElement {
        let id = w.id;
        let mut lines = div().flex().flex_col().gap(px(space::XXS)).font_family(Theme::mono_family()).text_size(px(font::CAPTION)).text_color(t.secondary);
        for line in &w.preview.lines {
            lines = lines.child(div().truncate().child(line.clone()));
        }
        let shown = !w.preview.lines.is_empty();
        super::drive::marked(div().id(("ai-ask-inline", id)), "dialog")
            .flex()
            .flex_col()
            .gap(px(space::S))
            .p(px(space::M))
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(t.accent.opacity(0.5))
            .bg(t.accent.opacity(0.04))
            .child(div().text_size(px(font::CAPTION)).text_color(t.secondary).child(tr("Die KI möchte etwas ändern")))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(w.preview.title.clone()))
            .when(shown, |d| d.child(div().p(px(space::S)).rounded(px(radius::INNER)).bg(t.ink.opacity(0.05)).child(lines)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(space::S))
                    .child(Button::new(("ai-inline-decline", id)).ghost().small().label(tr("Ablehnen")).on_click(move |_, _, cx| ai::answer_ask(id, Answer::Decline, cx)))
                    .child(Button::new(("ai-inline-today", id)).small().label(tr("Immer heute")).tooltip(tr("Erlaubt diese Art Änderung bis Mitternacht ohne Nachfrage")).on_click(move |_, _, cx| ai::answer_ask(id, Answer::Today, cx)))
                    .child(Button::new(("ai-inline-allow", id)).primary().small().label(tr("Erlauben")).on_click(move |_, _, cx| ai::answer_ask(id, Answer::Allow, cx))),
            )
            .into_any_element()
    }

    fn item_element(&self, at: usize, cx: &mut Context<Self>, t: &Theme) -> AnyElement {
        let Some(item) = self.items.get(at) else { return div().into_any_element() };
        match item {
            Item::User { text, chips } => {
                let mut row = div().flex().flex_wrap().justify_end().gap(px(space::XS));
                for c in chips {
                    row = row.child(div().max_w(px(180.)).truncate().px(px(space::S)).rounded_full().bg(t.ink.opacity(0.06)).text_size(px(font::CAPTION)).text_color(t.tertiary).child(c.clone()));
                }
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(space::XS))
                    .pl(px(space::XL))
                    .child(div().px(px(space::M)).py(px(space::S)).rounded(px(radius::CARD)).bg(t.ink.opacity(0.07)).line_height(relative(1.45)).child(text.clone()))
                    .when(!chips.is_empty(), |d| d.child(row))
                    .into_any_element()
            }
            Item::Agent { text, parsed, parsed_len, insert } => {
                let body = match parsed {
                    Some(p) if *parsed_len == text.len() || text.len() < *parsed_len + 400 => div().min_w_0().line_height(relative(1.5)).child(markdown_view::render(p, None, None, t)).into_any_element(),
                    _ => div().min_w_0().line_height(relative(1.5)).child(text.clone()).into_any_element(),
                };
                match insert {
                    Some(target) => {
                        let me = cx.entity();
                        let note = name_of(&target.note);
                        let action = if target.done {
                            div().flex().items_center().gap(px(space::XS)).text_size(px(font::CALLOUT)).text_color(t.tertiary).child(icon("check", iconsize::XS, t.tertiary)).child(trf("In %@ eingefügt", &[&note])).into_any_element()
                        } else {
                            Button::new(("ai-insert", at)).ghost().xsmall().icon(menu_icon("corner-down-left")).label(tr("In Notiz einfügen")).tooltip(trf("Hängt die Erklärung an %@ an", &[&note])).on_click(move |_, _, cx| me.update(cx, |p, cx| p.insert_answer(at, cx))).into_any_element()
                        };
                        div().flex().flex_col().gap(px(space::S)).child(body).child(div().flex().child(action)).into_any_element()
                    }
                    None => body,
                }
            }
            Item::Setup(card) => self.setup_card(at, card, cx, t),
            Item::Inbox(card) => self.inbox_card(at, card, cx, t),
            Item::Tool(tool) => self.tool_card(at, tool, cx, t),
            Item::Edit(e) => self.edit_card(at, e, cx, t),
            Item::Notice { text, restart } => {
                let me = cx.entity();
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .text_size(px(font::CALLOUT))
                    .text_color(t.secondary)
                    .child(icon("info", iconsize::XS, t.tertiary))
                    .child(div().flex_1().min_w_0().child(text.clone()))
                    .when(*restart, |d| d.child(Button::new(("ai-restart", at)).ghost().xsmall().label(tr("Neu starten")).on_click(move |_, _, cx| me.update(cx, |p, cx| { p.start(cx); cx.notify(); }))))
                    .into_any_element()
            }
        }
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::get(cx);
        let items = self.items.len();
        let (asks, status) = &self.tail;
        let last = ix + 1 == items + asks.len() + usize::from(status.is_some());
        let el = if ix < items {
            self.item_element(ix, cx, &t)
        } else if let Some(w) = asks.get(ix - items) {
            self.ask_card(w, &t)
        } else {
            let label = status.clone().unwrap_or_default();
            div().flex().items_center().gap(px(space::S)).text_size(px(font::CALLOUT)).text_color(t.secondary).child(motion::loading(iconsize::XS, t.secondary)).child(label).into_any_element()
        };
        let step = |i: usize| matches!(self.items.get(i), Some(Item::Tool(_) | Item::Edit(_)));
        let top = match self.items.get(ix) {
            Some(Item::User { .. }) if ix > 0 => space::XL,
            _ if ix > 0 && step(ix) && step(ix - 1) => space::XXS,
            _ => space::M,
        };
        div().px(px(space::L)).pt(px(top)).when(last, |d| d.pb(px(space::L))).child(el).into_any_element()
    }

    fn sync_list(&mut self, cx: &mut Context<Self>) {
        let asks = ai::waiting(cx);
        let starting = self.session.as_ref().is_some_and(|s| !s.ready);
        let status = (starting || self.turn).then(|| if starting { tr("Startet …") } else { tr("Arbeitet …") });
        self.tail = (asks, status);
        let items = self.items.len();
        let tail = self.tail.0.len() + usize::from(self.tail.1.is_some());
        let total = self.list.item_count();
        let kept = self.listed.min(total).min(items);
        for at in std::mem::take(&mut self.dirty) {
            if at < kept {
                self.list.splice(at..at + 1, 1);
            }
        }
        self.list.splice(kept..total, items - kept + tail);
        self.listed = items;
    }

    fn composer(&self, cx: &mut Context<Self>, t: &Theme) -> Div {
        let me = cx.entity();
        let chips = self.chips(cx);
        let mut row = div().flex().flex_wrap().gap(px(space::XS));
        for (i, chip) in chips.iter().enumerate() {
            let (me, c) = (me.clone(), chip.clone());
            row = row.child(
                div()
                    .id(("ai-chip", i))
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .pl(px(space::S))
                    .pr(px(space::XS))
                    .rounded_full()
                    .bg(t.ink.opacity(0.06))
                    .text_size(px(font::CAPTION))
                    .text_color(t.secondary)
                    .child(icon(chip_icon(chip), iconsize::XS, t.tertiary))
                    .child(div().max_w(px(160.)).truncate().child(chip_label(chip, cx)))
                    .child(
                        div()
                            .id(("ai-chip-x", i))
                            .rounded_full()
                            .cursor_pointer()
                            .soft(t.ink.opacity(0.08), radius::FULL)
                            .tooltip(tip(tr("Nicht mitschicken")))
                            .child(icon("x", iconsize::XS, t.tertiary))
                            .on_click(move |_, _, cx| {
                                let c = c.clone();
                                me.update(cx, |p, cx| {
                                    p.extra.retain(|e| *e != c);
                                    p.removed.insert(c);
                                    cx.notify();
                                });
                            }),
                    ),
            );
        }
        let removed: Vec<Chip> = self.auto_chips(cx).into_iter().filter(|c| self.removed.contains(c)).collect();
        let tabs: Vec<PathBuf> = state::read(cx, |l| l.open_documents.clone()).into_iter().filter(|p| !chips.iter().any(|c| matches!(c, Chip::Note(q) | Chip::Pdf(q, _) if q == p))).take(8).collect();
        let mut labels: Vec<(Chip, String)> = removed.into_iter().map(|c| { let l = chip_label(&c, cx); (c, l) }).collect();
        let week = Chip::Week(state::read(cx, |l| crate::core::day::monday(l.today())));
        if !chips.contains(&week) && !labels.iter().any(|(c, _)| *c == week) {
            let label = tr("Diese Woche");
            labels.push((week, label));
        }
        let soon: Vec<(String, String)> = state::read(cx, |l| l.upcoming_days(14).into_iter().take(5).map(|a| (a.id.clone(), a.title.clone())).collect());
        for (id, title) in soon {
            let chip = Chip::Deadline(id);
            if !chips.contains(&chip) && !labels.iter().any(|(c, _)| *c == chip) {
                labels.push((chip, title));
            }
        }
        let me_add = me.clone();
        let add = Button::new("ai-context").ghost().xsmall().icon(menu_icon("plus")).label(tr("Kontext")).disabled(labels.is_empty() && tabs.is_empty()).dropdown_menu(move |mut menu, _, _| {
            for (c, label) in labels.clone() {
                let me = me_add.clone();
                menu = menu.item(PopupMenuItem::new(label).icon(menu_icon(chip_icon(&c))).on_click(move |_, _, cx| {
                    let c = c.clone();
                    me.update(cx, |p, cx| {
                        if !p.removed.remove(&c) && !p.extra.contains(&c) {
                            p.extra.push(c);
                        }
                        cx.notify();
                    });
                }));
            }
            for path in tabs.clone() {
                let me = me_add.clone();
                let chip = if is_pdf(&path) { Chip::Pdf(path.clone(), 0) } else { Chip::Note(path.clone()) };
                menu = menu.item(PopupMenuItem::new(name_of(&path)).icon(menu_icon(chip_icon(&chip))).on_click(move |_, _, cx| {
                    let chip = chip.clone();
                    me.update(cx, |p, cx| {
                        if !p.extra.contains(&chip) {
                            p.extra.push(chip);
                        }
                        cx.notify();
                    });
                }));
            }
            menu
        });
        let follow = prefs::with(|p| p.ai.follow_edits);
        let follow_button = Button::new("ai-follow").ghost().xsmall().icon(menu_icon("crosshair")).label(tr("Folgen")).when(follow, |b| b.bg(t.ink.opacity(0.1))).tooltip(if follow { tr("Öffnet geänderte Dateien und zeigt die Stelle. Klick schaltet aus.") } else { tr("Geänderte Dateien öffnen und die Stelle zeigen") }).on_click(move |_, _, cx| {
            prefs::update(|p| p.ai.follow_edits = !follow);
            cx.refresh_windows();
        });
        let empty = self.input.read(cx).value().trim().is_empty();
        let me_send = me.clone();
        let input = self.input.clone();
        let action = if self.turn && empty {
            Button::new("ai-stop").small().icon(menu_icon("square")).tooltip(format!("{} ({})", tr("Stopp"), crate::core::prefs::Shortcut { key: ".".into(), command: true, shift: false, option: false, control: false }.display())).on_click(move |_, _, cx| me_send.update(cx, |p, cx| p.cancel(cx)))
        } else {
            Button::new("ai-send").primary().small().icon(menu_icon("arrow-up")).disabled(empty).tooltip(tr("Senden (⏎)")).on_click(move |_, window, cx| {
                let text = input.read(cx).value().to_string();
                me_send.update(cx, |p, cx| p.send(text, cx));
                input.update(cx, |s, cx| s.set_value("", window, cx));
            })
        };
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .p(px(space::M))
            .border_t_1()
            .border_color(t.separator)
            .when(!chips.is_empty(), |d| d.child(row))
            .child(Textarea::new(&self.input))
            .child(div().flex().items_center().gap(px(space::XS)).child(add).child(follow_button).child(div().flex_1()).child(action))
    }
}

/// Shows a change side by side: before on the left, after on the right; with `discard` the user keeps it or takes it back.
pub fn show_diff(path: &Path, before: &str, after: &str, discard: Option<Rc<dyn Fn(&mut App)>>, window: &mut Window, cx: &mut App) {
    let name = name_of(path);
    let closed = |t: &str| if t.ends_with('\n') || t.is_empty() { t.to_string() } else { format!("{t}\n") };
    let (old, new) = (closed(before), closed(after));
    let patch = similar::TextDiff::configure().timeout(Duration::from_secs(2)).diff_lines(&old, &new).unified_diff().header(&name, &name).to_string();
    let files = gpui_kit::component::diff::DiffFile::parse(&patch).unwrap_or_default();
    if window.has_active_dialog(cx) {
        return;
    }
    let state = cx.new(|cx| gpui_kit::component::diff::DiffState::new(files, cx).with_mode(gpui_kit::component::diff::DiffMode::Unified));
    window.open_dialog(cx, move |d, _, cx| {
        let t = Theme::get(cx);
        d.entrance(DialogEntrance::FadeSlide).w(px(760.)).p_0().on_ok(super::sheets::pass_enter).child(
            div()
                .h(px(480.))
                .flex()
                .flex_col()
                .child(div().flex_none().py(px(space::M)).px(px(space::L)).flex().border_b_1().border_color(t.separator).text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).text_color(t.secondary).child(name.clone()))
                .child(div().flex_1().min_h_0().child(gpui_kit::component::diff::Diff::new(&state).header_visible(false).hunk_separator(gpui_kit::component::diff::DiffHunkSeparator::Simple)))
                .child(match discard.clone() {
                    Some(discard) => div().flex_none().p(px(space::M)).flex().justify_end().gap(px(space::S)).border_t_1().border_color(t.separator)
                        .child(Button::new("ai-diff-discard").small().ghost().label(tr("Verwerfen")).tooltip(tr("Nimmt nur diese Änderung zurück")).on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            discard(cx);
                        }))
                        .child(Button::new("ai-diff-keep").small().primary().label(tr("Behalten")).on_click(|_, window, cx| window.close_dialog(cx))),
                    None => div().flex_none().p(px(space::M)).flex().justify_end().border_t_1().border_color(t.separator).child(Button::new("ai-diff-close").small().primary().label(tr("Schließen")).on_click(|_, window, cx| window.close_dialog(cx))),
                }),
        )
    });
}

impl Render for AiPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.take_requests(window, cx);
        let t = Theme::get(cx);
        let empty = self.items.is_empty() && ai::waiting(cx).is_empty();
        if !empty {
            self.sync_list(cx);
        }
        let body = if empty {
            div().id("ai-stream").flex_1().min_h_0().overflow_y_scroll().child(self.empty(cx, &t)).into_any_element()
        } else {
            div().flex_1().min_h_0().child(list(self.list.clone(), cx.processor(|this, ix, _, cx| this.row(ix, cx))).size_full()).into_any_element()
        };
        let header = self.header(cx, &t);
        let composer = self.composer(cx, &t);
        div()
            .id("ai-panel")
            .key_context("AiPanel")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.content)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                let command = k.modifiers.platform || k.modifiers.control;
                if command && k.key == "." {
                    this.cancel(cx);
                    cx.stop_propagation();
                } else if k.key == "up" && !command && this.input.read(cx).value().is_empty() && let Some(last) = this.last_prompt.clone() {
                    this.input.update(cx, |s, cx| s.set_value(last, window, cx));
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .child(body)
            .child(composer)
    }
}

/// The context chip for a note or PDF, for menus that ask about a file.
pub fn file_chip(path: &Path) -> Chip {
    if is_pdf(path) { Chip::Pdf(path.to_path_buf(), ai::pdf_focus(path).map_or(0, |f| f.page)) } else { Chip::Note(path.to_path_buf()) }
}
