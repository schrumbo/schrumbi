use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::v1::schema::{EnvVariable, McpServerStdio};
use agent_client_protocol::{AcpAgent, AcpAgentConfig};
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
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
use crate::core::ai::agents::{self, Missing, Runner};
use crate::core::ai::tools::{self, Preview};
use crate::core::ai::{detect, transcript};
use crate::core::i18n::{tr, trf};
use crate::core::library::Route;
use crate::core::prefs;

const FRAME: Duration = Duration::from_millis(8);
const REPARSE: Duration = Duration::from_millis(120);
const SELECTION_CHARS: usize = 4000;
const SHOWN_OUTPUT: usize = 4000;
const HIDDEN_TOOLS: [&str; 3] = ["ToolSearch", "TodoWrite", "ExitPlanMode"];

/// Something the agent sees with the next message.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Chip {
    Note(PathBuf),
    Pdf(PathBuf, usize),
    Selection(String),
    Module(String),
    Deadline(String),
}

struct Tool {
    id: String,
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
}

enum Item {
    User { text: String, chips: Vec<String> },
    Agent { text: String, parsed: Option<Rc<ParsedNote>>, parsed_len: usize },
    Tool(Tool),
    Edit(Edit),
    Notice { text: String, restart: bool },
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
    scroll: ScrollHandle,
    stick: bool,
    idle: Option<Task<()>>,
    reparse: Option<Task<()>>,
    focus_input: bool,
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

fn first_added(before: &str, after: &str) -> Option<String> {
    similar::TextDiff::from_lines(before, after).iter_all_changes().find(|c| c.tag() == similar::ChangeTag::Insert).map(|c| c.value().trim().to_string()).filter(|l| !l.is_empty())
}

fn missing_text(m: &Missing) -> String {
    match m {
        Missing::Adapter => tr("Für Claude fehlt noch ein Baustein. Hol ihn in Einstellungen → KI."),
        Missing::Node => tr("Codex braucht Node.js. Installiere es und versuch es noch einmal."),
        Missing::Program => tr("Das Programm ist nicht mehr installiert."),
        Missing::Command => tr("Der eigene Befehl ist leer."),
    }
}

/// The few things worth asking where the user is right now: label, icon, prompt and the chip that goes with it.
pub fn suggestions(cx: &App) -> Vec<(String, &'static str, String)> {
    let (route, doc) = state::read(cx, |l| (l.route().clone(), l.focused_document().map(Path::to_path_buf)));
    let mut out = Vec::new();
    match (&route, doc) {
        (Route::Doc(_), Some(doc)) if is_pdf(&doc) => {
            out.push((tr("Karten aus dieser Seite"), "layers", tr("Mach Karteikarten aus dieser PDF-Seite und ergänze sie in der passenden Notiz.")));
            out.push((tr("Diese Seite erklären"), "info", tr("Erklär mir diese PDF-Seite in einfachen Worten.")));
            out.push((tr("Zusammenfassen"), "file-text", tr("Fass dieses PDF kurz zusammen.")));
        }
        (Route::Doc(_), Some(doc)) if crate::core::markdown::is_markdown(&doc) => {
            out.push((tr("Karten aus dieser Notiz"), "layers", tr("Mach Karteikarten aus dieser Notiz und ergänze sie darin.")));
            out.push((tr("Abfragen"), "graduation-cap", tr("Frag mich zu dieser Notiz ab, eine Frage nach der anderen. Schreib nichts.")));
            out.push((tr("Zusammenfassen"), "file-text", tr("Fass diese Notiz kurz zusammen.")));
        }
        (Route::Today, _) => out.push((tr("Woche planen"), "calendar", tr("Schau dir meine Fristen der nächsten 14 Tage und diese Woche an und schlag mir einen Plan vor."))),
        (Route::Inbox, _) if state::read(cx, |l| !l.inbox.is_empty()) => out.push((tr("Inbox aufräumen"), "inbox", tr("Schlag mir für jede Datei in der Inbox einen Ordner und Namen vor und leg sie nach meinem Ja ab."))),
        _ => {}
    }
    out
}

fn chip_label(chip: &Chip, cx: &App) -> String {
    match chip {
        Chip::Note(p) => name_of(p),
        Chip::Pdf(p, page) => trf("%@ · S. %lld", &[&name_of(p), &(page + 1)]),
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
        Chip::Pdf(..) => "file",
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
        Self { input, items: Vec::new(), tools: HashMap::new(), expanded: HashSet::new(), session: None, turn: false, queued: 0, removed: HashSet::new(), extra: Vec::new(), last_prompt: None, scroll: ScrollHandle::new(), stick: true, idle: None, reparse: None, focus_input: true, _subs: subs }
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
        self.stick = true;
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
        let setup = acp::Setup { cwd: root.clone(), mcp, meta: launch.claude.then(|| agents::claude_meta(p.allow_execute)), mode: launch.claude.then(|| "default".to_string()), policy: acp::Policy { root: root.clone(), allow_execute: p.allow_execute } };
        let agent = AcpAgent::new(AcpAgentConfig::new(launch.program.clone()).args(launch.args.clone()).envs(launch.env.clone()));
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
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.session.is_none() {
            self.start(cx);
        }
        let chips = self.chips(cx);
        let lines = context_lines(&chips, cx);
        let prompt = if lines.is_empty() { text.clone() } else { format!("{text}\n\n[Context from Schrumbi]\n{}", lines.join("\n")) };
        let labels: Vec<String> = chips.iter().map(|c| chip_label(c, cx)).collect();
        let Some(session) = self.session.as_ref() else { return };
        if session.commands.try_send(Command::Prompt(prompt)).is_err() {
            return;
        }
        let id = session.id.clone();
        self.log(vec![transcript::Entry { role: transcript::Role::User, text: text.clone(), at: chrono::Utc::now().timestamp() }], cx);
        self.items.push(Item::User { text: text.clone(), chips: labels });
        self.last_prompt = Some(text);
        if self.turn {
            self.queued += 1;
        } else {
            self.turn = true;
            ai::begin_turn(&id, cx);
        }
        self.extra.clear();
        self.idle = None;
        self.stick = true;
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = &self.session
            && self.turn
        {
            let _ = s.commands.try_send(Command::Cancel);
        }
        cx.notify();
    }

    fn log(&self, entries: Vec<transcript::Entry>, cx: &mut Context<Self>) {
        let Some(file) = self.session.as_ref().and_then(|s| s.file.clone()) else { return };
        cx.background_executor().spawn(async move { transcript::append(&file, &entries) }).detach();
    }

    fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Ready { .. } => {
                if let Some(s) = self.session.as_mut() {
                    s.ready = true;
                }
            }
            Event::Update(Update::Text(text)) => {
                match self.items.last_mut() {
                    Some(Item::Agent { text: t, .. }) => t.push_str(&text),
                    _ => self.items.push(Item::Agent { text, parsed: None, parsed_len: 0 }),
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
                if let Some(Item::Tool(t)) = self.tools.get(&tool).and_then(|i| self.items.get_mut(*i)) {
                    t.denied = true;
                    t.status = Status::Failed;
                }
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
                self.items.push(Item::Tool(Tool { id: id.clone(), name: None, title: String::new(), kind: Kind::Other, status: Status::Pending, input: None, output: None, denied: false }));
                self.tools.insert(id, self.items.len() - 1);
                self.items.len() - 1
            }
            None => return,
        };
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
        let answer = ai::ask_change(&key, Preview { title, lines }, cx);
        cx.spawn(async move |this, cx| {
            let allow = matches!(answer.recv().await, Ok(Answer::Allow | Answer::Today));
            if allow {
                let _ = this.update(cx, |p, _| {
                    if let Some(s) = p.session.as_mut() {
                        s.granted.extend(places);
                    }
                });
            }
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
            ai::ask_change("write_file", preview, cx)
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
            let find = first_added(before.as_deref().unwrap_or_default(), &after);
            let target = path.clone();
            act(cx, move |l| {
                if let Some(find) = find {
                    l.find_in_file = Some((target.clone(), find, 0));
                }
                l.quiet_open = true;
                l.open_file(&target);
            });
        }
        self.log(vec![transcript::Entry { role: transcript::Role::Tool, text: trf("+%lld −%lld in %@", &[&added, &removed, &name_of(&path)]), at: chrono::Utc::now().timestamp() }], cx);
        self.items.push(Item::Edit(Edit { path, before, after, added, removed, undone: false }));
        self.stick = true;
        cx.notify();
    }

    fn turn_ended(&mut self, result: Result<Stop, String>, cx: &mut Context<Self>) {
        let Some(id) = self.session.as_ref().map(|s| s.id.clone()) else { return };
        ai::end_turn(&id, cx);
        match result {
            Ok(Stop::Done) => {}
            Ok(Stop::Cancelled) => self.notice(tr("Abgebrochen."), false),
            Ok(Stop::Refused) => self.notice(tr("Die KI hat das abgelehnt."), false),
            Ok(Stop::Limit) => self.notice(tr("Die Antwort wurde unterwegs gekürzt."), false),
            Err(e) => self.notice(trf("Das hat nicht geklappt: %@", &[&e.chars().take(200).collect::<String>()]), false),
        }
        let shell_off = !prefs::with(|p| p.ai.allow_execute);
        for item in &mut self.items {
            if let Item::Tool(t) = item
                && matches!(t.status, Status::Pending | Status::Running)
            {
                t.denied = shell_off && (t.kind == Kind::Execute || t.name.as_deref().is_some_and(|n| matches!(n, "Bash" | "PowerShell")));
                t.status = Status::Failed;
            }
        }
        let mut entries: Vec<transcript::Entry> = Vec::new();
        let now = chrono::Utc::now().timestamp();
        for item in self.items.iter().rev() {
            match item {
                Item::User { .. } => break,
                Item::Agent { text, .. } => entries.push(transcript::Entry { role: transcript::Role::Agent, text: text.clone(), at: now }),
                Item::Tool(t) => entries.push(transcript::Entry { role: transcript::Role::Tool, text: t.title.clone(), at: now }),
                _ => {}
            }
        }
        entries.reverse();
        self.log(entries, cx);
        self.reparse_now();
        if self.queued > 0 {
            self.queued -= 1;
            ai::begin_turn(&id, cx);
        } else {
            self.turn = false;
            self.schedule_idle(cx);
        }
    }

    fn schedule_reparse(&mut self, cx: &mut Context<Self>) {
        if self.reparse.is_some() {
            return;
        }
        self.reparse = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REPARSE).await;
            let _ = this.update(cx, |p, cx| {
                p.reparse = None;
                p.reparse_now();
                cx.notify();
            });
        }));
    }

    fn reparse_now(&mut self) {
        for item in self.items.iter_mut().rev().take(4) {
            if let Item::Agent { text, parsed, parsed_len } = item
                && *parsed_len != text.len()
            {
                *parsed = Some(Rc::new(markdown_view::parse(text)));
                *parsed_len = text.len();
            }
        }
    }

    fn undo_edit(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(Item::Edit(e)) = self.items.get(at) else { return };
        let (Some(before), path, after) = (e.before.clone(), e.path.clone(), e.after.clone()) else { return };
        let now = state::read(cx, |l| tools::text_now(l, &path));
        if now.as_deref() != Some(after.as_str()) {
            return;
        }
        let done = act(cx, |l| {
            let key = l.buffer_key(&path);
            l.edit_note(&key, "ai", trf("KI-Änderung an %@ zurückgenommen", &[&name_of(&path)]), |_| before)
        });
        if done && let Some(Item::Edit(e)) = self.items.get_mut(at) {
            e.undone = true;
        }
        cx.notify();
    }

    fn save_as_note(&mut self, cx: &mut Context<Self>) {
        let mut entries = Vec::new();
        for item in &self.items {
            match item {
                Item::User { text, .. } => entries.push(transcript::Entry { role: transcript::Role::User, text: text.clone(), at: 0 }),
                Item::Agent { text, .. } => entries.push(transcript::Entry { role: transcript::Role::Agent, text: text.clone(), at: 0 }),
                Item::Tool(t) if !t.denied => entries.push(transcript::Entry { role: transcript::Role::Tool, text: t.title.clone(), at: 0 }),
                Item::Edit(e) => entries.push(transcript::Entry { role: transcript::Role::Tool, text: trf("+%lld −%lld in %@", &[&e.added, &e.removed, &name_of(&e.path)]), at: 0 }),
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
                l.edit_note(&key, "ai", trf("%@ angelegt", &[&name_of(&path)]), |_| body);
                l.open_document(&path);
            }
        });
    }

    fn new_conversation(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        self.items.clear();
        self.tools.clear();
        self.expanded.clear();
        self.extra.clear();
        self.removed.clear();
        self.start(cx);
        self.focus_input = true;
        cx.notify();
    }

    fn header(&self, cx: &mut Context<Self>, t: &Theme) -> Div {
        let p = prefs::with(|p| p.ai.clone());
        let runners = agents::runners(&p);
        let chosen = agents::chosen(&p);
        let me = cx.entity();
        let (me2, me3) = (me.clone(), me.clone());
        let has_items = !self.items.is_empty();
        let mut head = div().h(px(super::theme::control::BAR)).flex_none().flex().items_center().gap(px(space::XS)).px(px(space::L)).border_b_1().border_color(t.separator).child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::BOLD).child(tr("KI")));
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
        head.child(div().flex_1())
            .child(Button::new("ai-save").ghost().size(px(super::theme::control::S)).rounded_full().icon(menu_icon("file-down")).disabled(!has_items).tooltip(tr("Als Notiz speichern")).on_click(move |_, _, cx| me2.update(cx, |p, cx| p.save_as_note(cx))))
            .child(Button::new("ai-new").ghost().size(px(super::theme::control::S)).rounded_full().icon(menu_icon("message-square-plus")).disabled(!has_items).tooltip(tr("Neue Unterhaltung")).on_click(move |_, _, cx| me3.update(cx, |p, cx| p.new_conversation(cx))))
            .child(Button::new("ai-close").ghost().size(px(super::theme::control::S)).rounded_full().icon(menu_icon("x")).tooltip(format!("{}{}", tr("KI ausblenden"), crate::core::shortcuts::ShortcutAction::ToggleAi.hint())).on_click(|_, _, cx| ai::close_column(cx)))
    }

    fn empty(&self, cx: &mut Context<Self>, t: &Theme) -> Div {
        let me = cx.entity();
        let mut list = div().flex().flex_col().gap(px(space::XS)).w_full();
        for (i, (label, icon_name, prompt)) in suggestions(cx).into_iter().take(3).enumerate() {
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
                        let prompt = prompt.clone();
                        me.update(cx, |p, cx| p.send(prompt, cx));
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
        let current = state::read(cx, |l| tools::text_now(l, &edit.path));
        let can_undo = edit.before.is_some() && !edit.undone && current.as_deref() == Some(edit.after.as_str());
        let undo_tip = if edit.undone { tr("Schon zurückgenommen") } else if edit.before.is_none() { tr("Eine neu angelegte Datei nimmt das normale Rückgängig zurück") } else if !can_undo { tr("Die Datei hat sich seitdem geändert") } else { tr("Nur diese Änderung zurücknehmen") };
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
            .child(Button::new(("ai-edit-view", at)).ghost().xsmall().label(tr("Ansehen")).on_click(move |_, window, cx| show_diff(&path, &before, &after, window, cx)))
            .child(Button::new(("ai-edit-undo", at)).ghost().xsmall().label(tr("Rückgängig")).disabled(!can_undo).tooltip(undo_tip).on_click(move |_, _, cx| me.update(cx, |p, cx| p.undo_edit(at, cx))))
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

    fn stream(&self, cx: &mut Context<Self>, t: &Theme) -> Vec<AnyElement> {
        let mut out = Vec::new();
        for (at, item) in self.items.iter().enumerate() {
            let el = match item {
                Item::User { text, chips } => {
                    let mut row = div().flex().flex_wrap().gap(px(space::XS));
                    for c in chips {
                        row = row.child(div().px(px(space::S)).rounded_full().bg(t.ink.opacity(0.06)).text_size(px(font::CAPTION)).text_color(t.secondary).child(c.clone()));
                    }
                    div().flex().flex_col().gap(px(space::XS)).p(px(space::M)).rounded(px(radius::CARD)).bg(t.ink.opacity(0.05)).child(div().child(text.clone())).when(!chips.is_empty(), |d| d.child(row)).into_any_element()
                }
                Item::Agent { text, parsed, parsed_len } => match parsed {
                    Some(p) if *parsed_len == text.len() || text.len() < *parsed_len + 400 => div().min_w_0().child(markdown_view::render(p, None, None, t)).into_any_element(),
                    _ => div().min_w_0().child(text.clone()).into_any_element(),
                },
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
            };
            out.push(el);
        }
        for w in ai::waiting(cx) {
            out.push(self.ask_card(&w, t));
        }
        let starting = self.session.as_ref().is_some_and(|s| !s.ready);
        if starting || self.turn {
            let label = if starting { tr("Startet …") } else { tr("Arbeitet …") };
            out.push(div().flex().items_center().gap(px(space::S)).text_size(px(font::CALLOUT)).text_color(t.secondary).child(motion::loading(iconsize::XS, t.secondary)).child(label).into_any_element());
        }
        out
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
        let labels: Vec<(Chip, String)> = removed.into_iter().map(|c| { let l = chip_label(&c, cx); (c, l) }).collect();
        let me_add = me.clone();
        let add = Button::new("ai-context").ghost().xsmall().icon(menu_icon("plus")).label(tr("Kontext")).disabled(labels.is_empty() && tabs.is_empty()).dropdown_menu(move |mut menu, _, _| {
            for (c, label) in labels.clone() {
                let me = me_add.clone();
                menu = menu.item(PopupMenuItem::new(label).icon(menu_icon(chip_icon(&c))).on_click(move |_, _, cx| {
                    let c = c.clone();
                    me.update(cx, |p, cx| {
                        p.removed.remove(&c);
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

/// Shows a change side by side: before on the left, after on the right.
pub fn show_diff(path: &Path, before: &str, after: &str, window: &mut Window, cx: &mut App) {
    let name = name_of(path);
    let closed = |t: &str| if t.ends_with('\n') || t.is_empty() { t.to_string() } else { format!("{t}\n") };
    let (old, new) = (closed(before), closed(after));
    let patch = similar::TextDiff::configure().timeout(Duration::from_secs(2)).diff_lines(&old, &new).unified_diff().header(&name, &name).to_string();
    let files = gpui_kit::component::diff::DiffFile::parse(&patch).unwrap_or_default();
    if window.has_active_dialog(cx) {
        return;
    }
    let state = cx.new(|cx| gpui_kit::component::diff::DiffState::new(files, cx).with_mode(gpui_kit::component::diff::DiffMode::Split));
    window.open_dialog(cx, move |d, _, cx| {
        let t = Theme::get(cx);
        d.entrance(DialogEntrance::FadeSlide).w(px(760.)).p_0().on_ok(super::sheets::pass_enter).child(
            div()
                .h(px(480.))
                .flex()
                .flex_col()
                .child(div().flex_none().py(px(space::M)).flex().border_b_1().border_color(t.separator).text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).text_color(t.secondary).child(div().flex_1().px(px(space::L)).child(tr("Vorher"))).child(div().flex_1().px(px(space::L)).child(tr("Nachher"))))
                .child(div().flex_1().min_h_0().child(gpui_kit::component::diff::Diff::new(&state).header_visible(false).hunk_separator(gpui_kit::component::diff::DiffHunkSeparator::Simple)))
                .child(div().flex_none().p(px(space::M)).flex().justify_end().border_t_1().border_color(t.separator).child(Button::new("ai-diff-close").small().primary().label(tr("Schließen")).on_click(|_, window, cx| window.close_dialog(cx)))),
        )
    });
}

impl Render for AiPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.take_requests(window, cx);
        let t = Theme::get(cx);
        let at_end = {
            let (offset, max) = (self.scroll.offset().y, self.scroll.max_offset().y);
            -offset >= max - px(space::XL)
        };
        if std::mem::take(&mut self.stick) || at_end {
            self.scroll.scroll_to_bottom();
        }
        let body: Vec<AnyElement> = if self.items.is_empty() && ai::waiting(cx).is_empty() { vec![self.empty(cx, &t).into_any_element()] } else { self.stream(cx, &t) };
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
            .child(div().id("ai-stream").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.scroll).child(div().flex().flex_col().gap(px(space::M)).p(px(space::L)).children(body)))
            .child(composer)
    }
}

/// The context chip for a note or PDF, for menus that ask about a file.
pub fn file_chip(path: &Path) -> Chip {
    if is_pdf(path) { Chip::Pdf(path.to_path_buf(), ai::pdf_focus(path).map_or(0, |f| f.page)) } else { Chip::Note(path.to_path_buf()) }
}

/// The runner the column would start, for the settings.
pub fn runner_now() -> Option<Runner> {
    agents::chosen(&prefs::with(|p| p.ai.clone()))
}
