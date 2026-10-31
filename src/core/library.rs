use super::errors::explain;
use super::next_action::{self, NextAction};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Datelike, Days, Duration, Local, NaiveDate, Utc};

use super::catalog::{self, CatalogFile};
use super::filing::{self, BRIEF_TAG, FileFacts, FileRole};
use super::i18n::{tr, trf};
use super::ics::{self, CalendarEvent};
use super::model::*;
use super::naming::{Naming, Parsed, extension};
use super::note_buffer::NoteBuffer;
use super::scanner::{self, Listings};
use super::search_index::SearchIndex;
use super::text::fold;
use super::workspace::{self, CONFIG_NAME, STATE_NAME};
use super::{day, prefs, schedule};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Route {
    Today,
    Grid,
    Inbox,
    Search,
    Checks,
    Record,
    Module(String),
    Doc(PathBuf),
}

/// How many pages Back remembers.
const ROUTE_HISTORY: usize = 50;

pub type Undo = Box<dyn FnOnce(&mut Library)>;

/// A short confirmation at the bottom of the window, optionally undoable.
pub struct Toast {
    pub id: u64,
    pub message: String,
    pub icon: &'static str,
    pub action_title: String,
    pub undo: Option<Undo>,
}

/// One effect of a step in the undo history; applying it yields the effect that reverses it.
#[derive(Debug, Clone)]
pub enum Change {
    /// Restores deadlines' state. Assignment notes typed since then stay, because undo must not lose text.
    State(Box<StudyState>),
    Config(Box<StudyConfig>),
    Move { from: PathBuf, to: PathBuf },
    Trash(PathBuf),
    Restore(PathBuf),
}

/// What one undo or redo reverts.
#[derive(Debug, Clone)]
pub struct Step {
    pub label: String,
    pub changes: Vec<Change>,
}

const HISTORY_LIMIT: usize = 100;
const COALESCE: std::time::Duration = std::time::Duration::from_secs(2);

/// A class the user had recently; `base` is the folder its files live in (e.g. the week's course folder).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassContext {
    pub module: Module,
    pub day: NaiveDate,
    pub days_ago: i64,
    pub base: PathBuf,
}

impl ClassContext {
    pub fn day_label(&self) -> String {
        match self.days_ago {
            0 => tr("Heute"),
            1 => tr("Gestern"),
            _ => day::weekday_long(self.day),
        }
    }
}

/// A folder suggested for course material or notes.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialTarget {
    pub path: PathBuf,
    pub module: Module,
    pub context: Option<ClassContext>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewNoteRequest {
    pub module_id: Option<String>,
    pub folder: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilingRequest {
    pub paths: Vec<PathBuf>,
    pub assignment_id: Option<String>,
}

/// A due reminder to schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    pub assignment_id: String,
    pub upload: bool,
    pub id: String,
    pub title: String,
    pub body: String,
    pub fire: DateTime<Local>,
}

/// Folder listings and inbox contents gathered off the main thread.
pub struct ScanResult {
    pub generation: u64,
    pub listings: Listings,
    pub inbox: Vec<PathBuf>,
    pub missing: usize,
}

/// Work for a background scan.
pub struct ScanJob {
    pub generation: u64,
    pub requests: HashSet<scanner::FolderRequest>,
    pub inbox: PathBuf,
    pub expected: Vec<PathBuf>,
}

impl ScanJob {
    pub fn run(self) -> ScanResult {
        ScanResult {
            generation: self.generation,
            listings: scanner::listings(&self.requests),
            inbox: scanner::files(&self.inbox, "*"),
            missing: self.expected.iter().filter(|p| !p.exists()).count(),
        }
    }
}

/// Work for a background index update.
pub struct IndexJob {
    pub index: Arc<SearchIndex>,
    pub root: PathBuf,
    pub exclude: Vec<String>,
    pub config: StudyConfig,
}

impl IndexJob {
    pub fn run(self) -> (usize, Vec<CatalogFile>) {
        let count = self.index.update(&self.root, &self.exclude);
        let catalog = catalog::build(&self.index.all(), &self.root, &self.config);
        (count, catalog)
    }
}

const IN_APP_TYPES: [&str; 51] = [
    "md", "markdown", "mdown", "pdf", "txt", "rtf", "rtfd", "csv", "json", "html", "htm", "xml", "css", "js", "ts", "swift", "java", "py", "c", "cpp", "h", "yml", "yaml", "tex", "doc", "docx", "ppt",
    "pptx", "xls", "xlsx", "pages", "key", "numbers", "odt", "ods", "odp", "png", "jpg", "jpeg", "heic", "gif", "webp", "tiff", "svg", "bmp", "mp4", "mov", "m4v", "mp3", "m4a", "wav",
];
const MATERIAL_WORDS: [&str; 12] = ["unterlagen", "material", "folien", "skript", "slides", "handout", "literatur", "vorlesung", "lecture", "docs", "mitschrift", "notes"];
const NOTE_WORDS: [&str; 4] = ["mitschrift", "notiz", "notes", "note"];

/// Central app state: workspace, config, user state and everything derived from them.
pub struct Library {
    pub root: Option<PathBuf>,
    pub config: Option<StudyConfig>,
    pub state: StudyState,
    pub assignments: Vec<Assignment>,
    evaluations: HashMap<String, Evaluation>,
    pub inbox: Vec<PathBuf>,
    pub index_count: usize,
    pub index_version: u64,
    pub is_indexing: bool,
    pub catalog: Vec<CatalogFile>,
    pub search_index: Arc<SearchIndex>,
    naming_cache: std::sync::Mutex<Option<(String, Arc<Vec<Naming>>)>>,
    pub now: DateTime<Local>,
    pub selected_semester_id: Option<String>,
    selection: Option<String>,
    pub show_inspector: bool,
    pub show_quick_add: bool,
    pub show_quick_open: bool,
    /// A PDF about to open and the text to look for in it, set when coming from the file search.
    pub find_in_file: Option<(PathBuf, String, usize)>,
    pub show_palette: bool,
    pub show_overdue: bool,
    pub show_unconfirmed: bool,
    pub show_shortcuts: bool,
    pub new_note: Option<NewNoteRequest>,
    pub import_path: Option<PathBuf>,
    pub reschedule_id: Option<String>,
    pub is_exporting: bool,
    pub exporting_pdfs: HashSet<PathBuf>,
    /// Notes shown in their own window; their buffers outlive the tab.
    pub note_windows: HashSet<PathBuf>,
    route: Route,
    closed_documents: Vec<PathBuf>,
    /// The open document shown beside the current one.
    pub split_document: Option<PathBuf>,
    back_routes: Vec<Route>,
    forward_routes: Vec<Route>,
    pub toast_hovered: bool,
    pub open_documents: Vec<PathBuf>,
    pub active_document: Option<PathBuf>,
    pub search_query: String,
    pub module_query: String,
    pub upload_assignment_id: Option<String>,
    pub last_error: Option<String>,
    pub toast: Option<Toast>,
    pub config_broken: bool,
    read_only: bool,
    pub needs_setup: bool,
    pub filing_request: Option<FilingRequest>,
    pub missing_folder_count: usize,
    action_items: Vec<Assignment>,
    listings: Listings,
    scan_generation: u64,
    toast_counter: u64,
    /// A step ran since the last check, so lists may have changed order.
    rows_moved: bool,
    undo_steps: Vec<Step>,
    redo_steps: Vec<Step>,
    /// The kind and time of the last edit that merges with the following ones, e.g. typing in a settings field.
    coalescing: Option<(&'static str, std::time::Instant)>,
    pub buffers: HashMap<PathBuf, NoteBuffer>,
    weeks_cache: Option<(Semester, Vec<LectureWeek>)>,
    /// Bumped on every change, so views can skip work when nothing changed.
    pub revision: u64,
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn is_dir(p: &Path) -> bool {
    p.is_dir()
}

fn subdirs(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(p)
        .map(|d| d.filter_map(Result::ok).map(|e| e.path()).filter(|p| is_dir(p) && !name_of(p).starts_with('.')).collect())
        .unwrap_or_default();
    v.sort_by(|a, b| super::text::natural_cmp(&name_of(a), &name_of(b)));
    v
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

impl Library {
    pub fn new() -> Self {
        Self {
            root: None,
            config: None,
            state: StudyState::default(),
            assignments: Vec::new(),
            evaluations: HashMap::new(),
            inbox: Vec::new(),
            index_count: 0,
            index_version: 0,
            is_indexing: false,
            catalog: Vec::new(),
            search_index: Arc::new(SearchIndex::open()),
            naming_cache: Default::default(),
            now: Local::now(),
            selected_semester_id: None,
            selection: None,
            show_inspector: false,
            show_quick_add: false,
            show_quick_open: false,
            find_in_file: None,
            show_palette: false,
            show_overdue: false,
            show_unconfirmed: false,
            show_shortcuts: false,
            new_note: None,
            import_path: None,
            reschedule_id: None,
            is_exporting: false,
            exporting_pdfs: HashSet::new(),
            note_windows: HashSet::new(),
            route: Route::Today,
            closed_documents: Vec::new(),
            split_document: None,
            back_routes: Vec::new(),
            forward_routes: Vec::new(),
            toast_hovered: false,
            open_documents: Vec::new(),
            active_document: None,
            search_query: String::new(),
            module_query: String::new(),
            upload_assignment_id: None,
            last_error: None,
            toast: None,
            config_broken: false,
            read_only: false,
            needs_setup: false,
            filing_request: None,
            missing_folder_count: 0,
            action_items: Vec::new(),
            listings: HashMap::new(),
            scan_generation: 0,
            toast_counter: 0,
            rows_moved: false,
            undo_steps: Vec::new(),
            redo_steps: Vec::new(),
            coalescing: None,
            buffers: HashMap::new(),
            weeks_cache: None,
            revision: 0,
        }
    }

    fn changed(&mut self) {
        self.revision += 1;
    }

    // MARK: Workspace

    /// Opens a workspace folder and remembers it.
    pub fn open(&mut self, root: &Path) {
        self.close();
        workspace::migrate_legacy(root);
        self.root = Some(root.to_path_buf());
        prefs::remember_workspace(root);
        self.load_config();
        self.load_state();
        self.load_documents();
        self.refresh();
        if prefs::get().indexed_root.as_deref() != Some(root) {
            self.search_index.reset();
            prefs::update(|p| p.indexed_root = Some(root.to_path_buf()));
        }
    }

    pub fn close(&mut self) {
        self.undo_steps.clear();
        self.redo_steps.clear();
        self.coalescing = None;
        self.root = None;
        self.config = None;
        self.state = StudyState::default();
        self.assignments.clear();
        self.evaluations.clear();
        self.inbox.clear();
        self.index_count = 0;
        self.catalog.clear();
        self.listings.clear();
        self.action_items.clear();
        self.buffers.clear();
        self.open_documents.clear();
        self.active_document = None;
        self.selection = None;
        self.route = Route::Today;
        self.back_routes.clear();
        self.forward_routes.clear();
        self.changed();
    }

    pub fn config_path(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join(CONFIG_NAME))
    }

    fn state_path(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join(STATE_NAME))
    }

    pub fn load_config(&mut self) {
        let (Some(root), Some(path)) = (self.root.clone(), self.config_path()) else { return };
        match workspace::load::<StudyConfig>(&path) {
            Ok(Some(c)) => {
                self.read_only = c.version > CURRENT_VERSION;
                self.last_error = self.read_only.then(|| tr("schrumbi.json stammt aus einer neueren App-Version. Änderungen werden nicht gespeichert, bis du die App aktualisierst."));
                self.config = Some(c);
                self.config_broken = false;
            }
            Ok(None) => {
                let c = workspace::starter_config(&root);
                let _ = workspace::save(&c, &path);
                self.config = Some(c);
                self.needs_setup = true;
                self.last_error = None;
            }
            Err(e) => {
                self.config_broken = true;
                self.last_error = Some(trf("schrumbi.json ist fehlerhaft: %@", &[&format!("{e:#}")]));
            }
        }
        self.changed();
    }

    fn load_state(&mut self) {
        let Some(path) = self.state_path() else { return };
        match workspace::load::<StudyState>(&path) {
            Ok(s) => self.state = s.unwrap_or_default(),
            Err(_) => {
                if workspace::restore_latest::<StudyState>(&path)
                    && let Ok(Some(s)) = workspace::load::<StudyState>(&path)
                {
                    self.state = s;
                    self.last_error = Some(tr("Der Status war beschädigt und wurde aus der letzten Sicherung wiederhergestellt."));
                } else {
                    self.read_only = true;
                    self.last_error = Some(tr("Der Status (.schrumbi-state.json) ist beschädigt. Änderungen werden nicht gespeichert, damit nichts überschrieben wird."));
                }
            }
        }
    }

    pub fn restore_config_backup(&mut self) {
        let Some(path) = self.config_path() else { return };
        if workspace::restore_latest::<StudyConfig>(&path) {
            self.load_config();
            self.refresh();
            self.notify(tr("Letzte Sicherung wiederhergestellt"), "history", None, None);
        } else {
            self.last_error = Some(tr("Keine lesbare Sicherung gefunden."));
        }
    }

    /// Writes the config right away.
    pub fn save_config(&mut self, mut c: StudyConfig) {
        let Some(path) = self.config_path() else { return };
        if self.read_only {
            return;
        }
        c.version = CURRENT_VERSION;
        match workspace::save(&c, &path) {
            Ok(()) => {
                self.config = Some(c);
                self.refresh();
            }
            Err(e) => self.last_error = Some(trf("Speichern fehlgeschlagen: %@", &[&e])),
        }
    }

    /// Applies a config change in memory; the caller debounces `write_config`.
    pub fn update_config(&mut self, c: StudyConfig) -> bool {
        if self.config.as_ref() == Some(&c) {
            return false;
        }
        self.config = Some(c);
        self.changed();
        true
    }

    pub fn write_config(&mut self) {
        let (Some(path), Some(mut c)) = (self.config_path(), self.config.clone()) else { return };
        if self.read_only {
            return;
        }
        c.version = CURRENT_VERSION;
        if let Err(e) = workspace::save(&c, &path) {
            self.last_error = Some(trf("Speichern fehlgeschlagen: %@", &[&e]));
        }
    }

    fn save_state(&mut self) {
        let Some(path) = self.state_path() else { return };
        if self.read_only {
            return;
        }
        if let Err(e) = workspace::save(&self.state, &path) {
            self.last_error = Some(trf("Speichern fehlgeschlagen: %@", &[&e]));
        }
    }

    /// Picks up `schrumbi.json` / state edits made outside the app; true when something changed.
    pub fn reload_external(&mut self) -> bool {
        let mut changed = false;
        if let Some(p) = self.config_path()
            && let Ok(Some(c)) = workspace::load::<StudyConfig>(&p)
            && Some(&c) != self.config.as_ref()
        {
            self.config = Some(c);
            changed = true;
        }
        if let Some(p) = self.state_path()
            && let Ok(Some(s)) = workspace::load::<StudyState>(&p)
            && s != self.state
        {
            self.state = s;
            changed = true;
        }
        if changed {
            self.refresh();
        }
        changed
    }

    // MARK: Refresh

    /// Re-evaluates with the current time; called every 30 seconds.
    pub fn tick(&mut self) {
        self.now = Local::now();
        self.reevaluate();
    }

    /// Regenerates assignments and re-evaluates with cached listings; follow with a background scan.
    pub fn refresh(&mut self) {
        let (Some(root), Some(config)) = (self.root.clone(), self.config.clone()) else { return };
        day::set_time_zone(&config.time_zone);
        self.now = Local::now();
        let fresh = schedule::assignments(&config, &self.state, &root);
        if fresh != self.assignments {
            self.assignments = fresh;
        }
        if self.selected_semester_id.as_ref().is_none_or(|id| config.semester(id).is_none()) {
            self.selected_semester_id = config.current_semester(self.now).map(|s| s.id.clone());
        }
        self.weeks_cache = None;
        self.reevaluate();
    }

    /// Cheap: status from cached folder listings, no disk access.
    pub fn reevaluate(&mut self) {
        let Some(config) = self.config.as_ref() else { return };
        self.evaluations = scanner::evaluate(&self.assignments, &self.listings, config, &self.state, self.now, self.root.as_deref());
        let mut actions: Vec<Assignment> = self.assignments.iter().filter(|a| self.needs_action(a)).cloned().collect();
        actions.sort_by(|a, b| b.due.cmp(&a.due));
        self.action_items = actions;
        self.changed();
    }

    /// Folder listings to gather in the background; apply them with `apply_scan`.
    pub fn scan_job(&mut self) -> Option<ScanJob> {
        let (root, config) = (self.root.as_ref()?, self.config.as_ref()?);
        self.scan_generation += 1;
        Some(ScanJob {
            generation: self.scan_generation,
            requests: scanner::requests(&self.assignments),
            inbox: root.join(&config.inbox),
            expected: self.semester().map(|s| schedule::expected_folders(config, s, root)).unwrap_or_default(),
        })
    }

    pub fn apply_scan(&mut self, r: ScanResult) {
        if r.generation != self.scan_generation {
            return;
        }
        self.missing_folder_count = r.missing;
        self.listings = r.listings;
        self.inbox = r.inbox;
        self.reevaluate();
    }

    pub fn index_job(&mut self) -> Option<IndexJob> {
        if self.is_indexing {
            return None;
        }
        let root = self.root.clone()?;
        self.is_indexing = true;
        let config = self.config.clone().unwrap_or_default();
        Some(IndexJob { index: self.search_index.clone(), root, exclude: config.search_exclude.clone(), config })
    }

    pub fn apply_index(&mut self, count: usize, catalog: Vec<CatalogFile>) {
        if catalog != self.catalog {
            self.catalog = catalog;
            self.index_version += 1;
        }
        self.index_count = count;
        self.is_indexing = false;
        self.changed();
    }

    // MARK: Derived

    pub fn route(&self) -> &Route {
        &self.route
    }

    pub fn set_route(&mut self, r: Route) {
        if self.route != r {
            self.back_routes.push(std::mem::replace(&mut self.route, r));
            if self.back_routes.len() > ROUTE_HISTORY {
                self.back_routes.remove(0);
            }
            self.forward_routes.clear();
        }
        self.changed();
    }

    /// Whether a route can still be shown; closed documents drop out of the history.
    fn route_exists(&self, r: &Route) -> bool {
        !matches!(r, Route::Doc(p) if !self.open_documents.contains(p))
    }

    pub fn can_go_back(&self) -> bool {
        self.back_routes.iter().any(|r| self.route_exists(r))
    }

    pub fn can_go_forward(&self) -> bool {
        self.forward_routes.iter().any(|r| self.route_exists(r))
    }

    pub fn go_forward(&mut self) {
        while let Some(r) = self.forward_routes.pop() {
            if self.route_exists(&r) {
                self.back_routes.push(std::mem::replace(&mut self.route, r));
                self.changed();
                return;
            }
        }
    }

    pub fn selection(&self) -> Option<&str> {
        self.selection.as_deref()
    }

    pub fn select(&mut self, id: Option<String>) {
        if id.is_some() {
            self.show_inspector = true;
        }
        self.selection = id;
        self.changed();
    }

    pub fn semester(&self) -> Option<&Semester> {
        let config = self.config.as_ref()?;
        self.selected_semester_id.as_ref().and_then(|id| config.semester(id)).or_else(|| config.current_semester(self.now))
    }

    pub fn semester_modules(&self) -> Vec<&Module> {
        let (Some(c), Some(s)) = (self.config.as_ref(), self.semester()) else { return Vec::new() };
        c.modules.iter().filter(|m| m.semester == s.id).collect()
    }

    pub fn semester_assignments(&self) -> Vec<&Assignment> {
        let Some(s) = self.semester() else { return Vec::new() };
        self.assignments.iter().filter(|a| a.semester_id == s.id).collect()
    }

    pub fn module(&self, a: &Assignment) -> Option<&Module> {
        self.config.as_ref()?.module(&a.module_id)
    }

    pub fn module_by_id(&self, id: &str) -> Option<&Module> {
        self.config.as_ref()?.module(id)
    }

    pub fn assignment(&self, id: &str) -> Option<&Assignment> {
        self.assignments.iter().find(|a| a.id == id)
    }

    pub fn selected(&self) -> Option<&Assignment> {
        self.selection.as_deref().and_then(|id| self.assignment(id))
    }

    pub fn evaluation(&self, a: &Assignment) -> Evaluation {
        self.evaluations.get(&a.id).cloned().unwrap_or_else(Evaluation::empty)
    }

    pub fn status(&self, a: &Assignment) -> Status {
        self.evaluations.get(&a.id).map_or(Status::Open, |e| e.status)
    }

    pub fn is_done(&self, a: &Assignment) -> bool {
        self.status(a).is_closed()
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.show_inspector && self.selection.as_deref() == Some(id)
    }

    pub fn today(&self) -> NaiveDate {
        day::local_date(self.now)
    }

    /// Lecture weeks of the shown semester, computed once per semester change.
    pub fn lecture_weeks(&mut self) -> Vec<LectureWeek> {
        let Some(s) = self.semester().cloned() else { return Vec::new() };
        if let Some((cs, w)) = &self.weeks_cache
            && *cs == s
        {
            return w.clone();
        }
        let w = schedule::lecture_weeks(&s);
        self.weeks_cache = Some((s, w.clone()));
        w
    }

    pub fn weeks(&self) -> Vec<LectureWeek> {
        match (&self.weeks_cache, self.semester()) {
            (Some((cs, w)), Some(s)) if cs == s => w.clone(),
            (_, Some(s)) => schedule::lecture_weeks(s),
            _ => Vec::new(),
        }
    }

    pub fn current_week(&self) -> Option<LectureWeek> {
        let monday = day::monday(self.today());
        self.weeks().into_iter().find(|w| w.monday == monday)
    }

    pub fn lecture_week_containing(&self, d: NaiveDate) -> Option<LectureWeek> {
        let monday = day::monday(d);
        self.weeks().into_iter().find(|w| w.monday == monday)
    }

    pub fn assignments_on(&self, d: NaiveDate) -> Vec<&Assignment> {
        self.assignments.iter().filter(|a| day::local_date(a.due) == d).collect()
    }

    pub fn sessions_on(&self, d: NaiveDate) -> Vec<(&Module, &Session)> {
        let Some(s) = self.semester().filter(|s| s.contains(d) && s.break_at(d).is_none()) else { return Vec::new() };
        let weekday = d.weekday().number_from_monday();
        let mut out: Vec<_> = self.config.iter().flat_map(|c| c.modules.iter()).filter(|m| m.semester == s.id).flat_map(|m| m.sessions.iter().filter(move |x| x.weekday == weekday).map(move |x| (m, x))).collect();
        out.sort_by(|a, b| a.1.start.cmp(&b.1.start));
        out
    }

    pub fn has_sessions(&self) -> bool {
        self.semester_modules().iter().any(|m| !m.sessions.is_empty())
    }

    pub fn upcoming_exams(&self) -> Vec<&Assignment> {
        self.assignments.iter().filter(|a| a.kind == Kind::Exam && a.due >= self.now).collect()
    }

    /// Past deadlines that still need something, newest first.
    pub fn action_items(&self) -> &[Assignment] {
        &self.action_items
    }

    /// Ready submissions past their deadline that only need an upload confirmation.
    pub fn unconfirmed(&self) -> Vec<&Assignment> {
        self.action_items.iter().filter(|a| self.status(a) == Status::Ready).collect()
    }

    pub fn week_progress(&self) -> (usize, usize) {
        let monday = day::monday(self.today());
        let next = monday + Days::new(7);
        let items: Vec<_> = self
            .assignments
            .iter()
            .filter(|a| {
                let d = day::local_date(a.due);
                d >= monday && d < next && !a.optional && self.status(a) != Status::Skipped
            })
            .collect();
        (items.iter().filter(|a| self.is_done(a)).count(), items.len())
    }

    pub fn module_progress(&self, id: &str) -> (usize, usize) {
        let items: Vec<_> = self.assignments.iter().filter(|a| a.module_id == id && self.counts(a)).collect();
        (items.iter().filter(|a| self.is_done(a)).count(), items.len())
    }

    /// Past deadlines that still need something from the user.
    pub fn needs_action(&self, a: &Assignment) -> bool {
        self.now > a.due && !self.is_done(a)
    }

    pub fn upcoming(&self) -> Vec<&Assignment> {
        self.assignments.iter().filter(|a| a.due >= self.now && !self.is_done(a)).collect()
    }

    /// What the overview recommends doing now.
    pub fn next_action(&self) -> NextAction<&Assignment> {
        let overdue = self.action_items.iter().filter(|a| self.status(a) != Status::Ready).max_by_key(|a| a.due);
        let unconfirmed = self.unconfirmed().into_iter().max_by_key(|a| a.due);
        let next = self.next_deadline();
        let due_soon = next.is_some_and(|a| a.due - self.now < Duration::days(1));
        next_action::choose(overdue, unconfirmed, next, due_soon, self.inbox.len())
    }

    pub fn next_deadline(&self) -> Option<&Assignment> {
        let upcoming = self.upcoming();
        upcoming.iter().find(|a| !a.optional).or(upcoming.first()).copied()
    }

    pub fn upcoming_days(&self, days: i64) -> Vec<&Assignment> {
        let limit = self.now + Duration::days(days);
        self.assignments.iter().filter(|a| a.due >= self.now && a.due <= limit).collect()
    }

    /// Counts mandatory entries plus optional ones that were actually completed.
    fn counts(&self, a: &Assignment) -> bool {
        self.status(a) != Status::Skipped && (!a.optional || self.is_done(a)) && a.kind != Kind::Exam
    }

    /// (done, due so far, total) for the shown semester.
    pub fn progress(&self) -> (usize, usize, usize) {
        let relevant: Vec<_> = self.semester_assignments().into_iter().filter(|a| self.counts(a)).collect();
        (relevant.iter().filter(|a| self.is_done(a)).count(), relevant.iter().filter(|a| a.due <= self.now).count(), relevant.len())
    }

    /// Consecutive past deadlines met on time, counting back from the latest.
    pub fn streak(&self) -> usize {
        let mut past: Vec<_> = self.assignments.iter().filter(|a| a.due <= self.now).collect();
        past.sort_by(|a, b| b.due.cmp(&a.due));
        let mut count = 0;
        for a in past {
            let e = self.evaluations.get(&a.id);
            match e.map(|e| (e.status, e.completed_on_time)) {
                Some((Status::Skipped, _)) => continue,
                Some((s, on_time)) if s.is_closed() && on_time != Some(false) => count += 1,
                _ => break,
            }
        }
        count
    }

    pub fn misnamed(&self) -> Vec<(&Assignment, PathBuf)> {
        self.assignments.iter().flat_map(|a| self.evaluations.get(&a.id).into_iter().flat_map(|e| e.misnamed.iter().cloned()).map(move |f| (a, f))).collect()
    }

    pub fn naming(&self, module_id: &str) -> Option<Naming> {
        let index = self.config.as_ref()?.modules.iter().position(|m| m.id == module_id)?;
        self.namings().get(index).cloned()
    }

    /// The naming convention of every module. Their regexes are slow to compile and titles ask for them on every frame, so they stay cached until a convention changes.
    pub fn namings(&self) -> Arc<Vec<Naming>> {
        let Some(c) = self.config.as_ref() else { return Arc::default() };
        let key = format!("{}|{}|{:?}|{}", c.author, c.matrikel, c.coauthor_style, c.modules.iter().map(|m| format!("{}|{}|{:?}", m.code, m.naming, c.semester(&m.semester).map(|s| &s.start))).collect::<Vec<_>>().join("|"));
        let mut cache = self.naming_cache.lock().unwrap();
        if let Some((known, namings)) = cache.as_ref()
            && *known == key
        {
            return namings.clone();
        }
        let namings: Arc<Vec<Naming>> = Arc::new(c.modules.iter().map(|m| Naming::new(m, c.semester(&m.semester), c)).collect());
        *cache = Some((key, namings.clone()));
        namings
    }

    /// Words that alone never make a document title.
    pub fn title_ignore_words(&self) -> Vec<String> {
        let Some(c) = self.config.as_ref() else { return Vec::new() };
        let author = c.author.split('-').next().unwrap_or("").to_string();
        [c.name.clone(), author].into_iter().chain(c.modules.iter().flat_map(|m| [m.code.clone(), m.name.clone()])).chain(["Name", "Seite", "Lehrwoche"].map(String::from)).collect()
    }

    /// Convention name for an existing file, keeping its title, co-authors and date where recognizable.
    pub fn suggested_name(&self, file: &Path, a: &Assignment) -> Option<String> {
        let naming = self.naming(&a.module_id)?;
        let facts = FileFacts::read(file);
        let namings = self.namings();
        let parsed = namings.iter().find_map(|n| n.parse(&name_of(file)));
        let title = parsed.as_ref().map(|p| p.title.clone()).or_else(|| filing::title_suggestions(&facts, &namings, &self.title_ignore_words()).first().map(|s| s.title.clone())).unwrap_or_else(|| "Titel".into());
        let date = parsed.as_ref().and_then(|p| p.date).or(facts.name_date).unwrap_or_else(|| day::local_date(facts.created.min(a.due)));
        Some(naming.render(&title, date, extension(&name_of(file)), &parsed.map(|p| p.coauthors).unwrap_or_default(), a.week, a.iso_week, None))
    }

    pub fn module_containing(&self, path: &Path) -> Option<&Module> {
        let root = self.root.as_ref()?;
        let rel = super::text::slash_path(path.strip_prefix(root).ok()?);
        self.config.as_ref()?.modules.iter().filter(|m| rel == m.folder || rel.starts_with(&format!("{}/", m.folder))).max_by_key(|m| m.folder.len())
    }

    /// Folders worth offering as manual targets: those of the current lecture week plus top-level module folders.
    pub fn suggested_folders(&self, module_id: Option<&str>) -> Vec<PathBuf> {
        let (Some(root), Some(config)) = (self.root.as_ref(), self.config.as_ref()) else { return Vec::new() };
        let semester_id = self.semester().map(|s| s.id.clone());
        let week_token = self.current_week().map(|w| format!("W{:02}", w.index));
        let week_re = regex::Regex::new(r"^W\d{2}").unwrap();
        let mut out = Vec::new();
        for m in config.modules.iter().filter(|m| module_id.map_or(Some(&m.semester) == semester_id.as_ref(), |id| m.id == id)) {
            let base = root.join(&m.folder);
            let walker = walkdir::WalkDir::new(&base).min_depth(1).max_depth(4).into_iter().filter_entry(|e| {
                let n = e.file_name().to_string_lossy();
                !n.starts_with('.') && ![".git", "node_modules", "assets"].contains(&n.as_ref())
            });
            for e in walker.filter_map(Result::ok).filter(|e| e.file_type().is_dir()) {
                let rel = e.path().strip_prefix(&base).map(super::text::slash_path).unwrap_or_default();
                let top = !rel.contains('/');
                let this_week = week_token.as_ref().is_some_and(|t| rel.starts_with(t.as_str()));
                if (top && !week_re.is_match(&rel)) || (this_week && rel.split('/').count() >= 2) {
                    out.push(e.path().to_path_buf());
                }
            }
        }
        out.sort();
        out
    }

    /// Classes today and in the two days before, most recent first: timetable sessions, or, without a timetable,
    /// days on which a rule has a deadline in the same week (e.g. a daily exercise), which marks a class day.
    pub fn recent_classes(&self, days: i64) -> Vec<ClassContext> {
        let (Some(root), Some(s)) = (self.root.as_ref(), self.semester()) else { return Vec::new() };
        let weeks = self.weeks();
        let mut out: Vec<ClassContext> = Vec::new();
        for offset in 0..days {
            let d = self.today() - Days::new(offset as u64);
            let weekday = d.weekday().number_from_monday();
            let week = weeks.iter().rev().find(|w| w.monday <= d);
            for m in self.semester_modules() {
                let module_root = root.join(&m.folder);
                let mut bases: Vec<PathBuf> = Vec::new();
                for rule in m.rules.iter().filter(|r| r.week_offset == 0 && r.weekday == weekday) {
                    let Some(w) = week else { continue };
                    if let Some(a) = self.assignment(&format!("{}.{}.w{}", m.id, rule.id, w.index)) {
                        bases.push(if a.per_week_folder { a.folder.parent().map(Path::to_path_buf).unwrap_or(module_root.clone()) } else { module_root.clone() });
                    }
                }
                if bases.is_empty() && s.break_at(d).is_none() && m.sessions.iter().any(|x| x.weekday == weekday) {
                    let week_folder = week.and_then(|w| subdirs(&module_root).into_iter().find(|f| catalog::week_index(&name_of(f)) == Some(w.index)));
                    bases.push(week_folder.unwrap_or(module_root.clone()));
                }
                for base in bases {
                    if !out.iter().any(|c| c.base == base) {
                        out.push(ClassContext { module: m.clone(), day: d, days_ago: offset, base });
                    }
                }
            }
        }
        out
    }

    fn material_subfolders(base: &Path) -> Vec<PathBuf> {
        subdirs(base).into_iter().filter(|u| MATERIAL_WORDS.iter().any(|w| fold(&name_of(u)).contains(w))).collect()
    }

    /// Where course material most likely belongs right now: material folders of today's and recent classes first,
    /// then those of the other modules this semester.
    pub fn material_targets(&self) -> Vec<MaterialTarget> {
        let Some(root) = self.root.as_ref() else { return Vec::new() };
        let mut out: Vec<MaterialTarget> = Vec::new();
        let mut add = |path: PathBuf, module: &Module, context: Option<ClassContext>| {
            if !out.iter().any(|t| t.path == path) {
                out.push(MaterialTarget { path, module: module.clone(), context });
            }
        };
        for c in self.recent_classes(3) {
            let subs = Self::material_subfolders(&c.base);
            if subs.is_empty() {
                add(c.base.clone(), &c.module, Some(c.clone()));
            }
            for s in subs {
                add(s, &c.module, Some(c.clone()));
            }
        }
        let week = self.current_week();
        for m in self.semester_modules() {
            let module_root = root.join(&m.folder);
            let subs = Self::material_subfolders(&module_root);
            if subs.is_empty() {
                let week_folder = week.and_then(|w| subdirs(&module_root).into_iter().find(|f| catalog::week_index(&name_of(f)) == Some(w.index)));
                add(week_folder.unwrap_or(module_root), m, None);
            } else {
                for s in subs {
                    add(s, m, None);
                }
            }
        }
        out
    }

    /// Folders for a new note: note folders of recent classes, then of other modules; falls back to material folders.
    pub fn note_targets(&self, preferring: Option<&str>) -> Vec<MaterialTarget> {
        let Some(root) = self.root.as_ref() else { return Vec::new() };
        let note_folders = |base: &Path| subdirs(base).into_iter().filter(|u| NOTE_WORDS.iter().any(|w| fold(&name_of(u)).contains(w))).collect::<Vec<_>>();
        let mut out: Vec<MaterialTarget> = Vec::new();
        let mut add = |path: PathBuf, m: &Module, c: Option<ClassContext>| {
            if !out.iter().any(|t| t.path == path) {
                out.push(MaterialTarget { path, module: m.clone(), context: c });
            }
        };
        let materials = self.material_targets();
        for c in self.recent_classes(3) {
            let notes = note_folders(&c.base);
            if notes.is_empty() {
                for t in materials.iter().filter(|t| t.context.as_ref().is_some_and(|x| x.base == c.base)) {
                    add(t.path.clone(), &t.module, Some(c.clone()));
                }
            } else {
                for n in notes {
                    add(n, &c.module, Some(c.clone()));
                }
            }
        }
        for m in self.semester_modules() {
            for n in note_folders(&root.join(&m.folder)) {
                add(n, m, None);
            }
        }
        for t in &materials {
            add(t.path.clone(), &t.module, t.context.clone());
        }
        if let Some(id) = preferring {
            let (mut first, rest): (Vec<_>, Vec<_>) = out.into_iter().partition(|t| t.module.id == id);
            first.extend(rest);
            out = first;
        }
        out
    }

    /// Convention name for a new note in `folder`, or a plain name outside modules.
    pub fn note_name(&self, title: &str, folder: &Path, ext: &str) -> String {
        let Some(naming) = self.module_containing(folder).and_then(|m| self.naming(&m.id)) else { return format!("{}.{ext}", Naming::clean(title)) };
        naming.render(title, self.today(), ext, &[], self.lecture_week_containing(self.today()).map(|w| w.index), None, None)
    }

    /// Writes an empty note with the title as heading; returns its path.
    pub fn create_note(&mut self, title: &str, folder: &Path, ext: &str) -> Option<PathBuf> {
        let result: Result<PathBuf> = (|| {
            std::fs::create_dir_all(folder)?;
            let path = unique_path(&folder.join(self.note_name(title, folder, ext)));
            std::fs::write(&path, if ext == "md" { format!("# {}\n\n", title.trim()) } else { String::new() })?;
            Ok(path)
        })();
        match result {
            Ok(path) => {
                self.refresh();
                self.commit(trf("Angelegt: %@", &[&name_of(&path)]), "square-pen", vec![Change::Trash(path.clone())]);
                Some(path)
            }
            Err(e) => {
                self.last_error = Some(trf("Notiz konnte nicht angelegt werden: %@", &[&e]));
                None
            }
        }
    }

    /// Creates a subfolder; returns its path.
    pub fn create_folder(&mut self, name: &str, folder: &Path) -> Option<PathBuf> {
        let path = unique_path(&folder.join(name));
        match std::fs::create_dir_all(&path) {
            Ok(()) => {
                self.refresh();
                let p = path.clone();
                self.record(trf("Ordner angelegt: %@", &[&name_of(&path)]), vec![Change::Trash(path.clone())]);
                self.notify(trf("Ordner angelegt: %@", &[&name_of(&path)]), "folder-plus", Some(tr("Zeigen")), Some(Box::new(move |_: &mut Library| open_path(&p))));
                Some(path)
            }
            Err(e) => {
                self.last_error = Some(trf("Ordner konnte nicht angelegt werden: %@", &[&e]));
                None
            }
        }
    }

    /// Bylines (e.g. a lecturer like `Beispiel-A`) used by convention-named files in the same place of a module, most frequent first.
    pub fn bylines_near(&self, folder: &Path) -> Vec<String> {
        let (Some(root), Some(module), Some(author)) = (self.root.as_ref(), self.module_containing(folder), self.config.as_ref().map(|c| c.author.clone())) else { return Vec::new() };
        let base = root.join(&module.folder);
        let mut place: Vec<String> = folder.strip_prefix(&base).map(|r| r.iter().map(|c| c.to_string_lossy().to_string()).collect()).unwrap_or_default();
        if let Some(i) = place.iter().position(|p| catalog::week_index(p).is_some()) {
            place.remove(i);
        }
        let mut counts: HashMap<String, usize> = HashMap::new();
        for f in self.catalog.iter().filter(|f| f.module_id.as_deref() == Some(&module.id) && f.place == place) {
            let parts: Vec<&str> = super::naming::stem(f.name()).split('_').collect();
            let Some(last) = parts.last() else { continue };
            if parts.len() < 3 || last.len() != 6 || !last.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let byline = parts[parts.len() - 2];
            if byline != author && byline != BRIEF_TAG && byline.contains('-') && byline.chars().next().is_some_and(char::is_uppercase) {
                *counts.entry(byline.to_string()).or_default() += 1;
            }
        }
        let mut v: Vec<_> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v.into_iter().map(|(k, _)| k).collect()
    }

    /// Ranking boosts for filing targets from recent classes, keyed by assignment folder parent and module.
    pub fn class_boosts(&self) -> (HashMap<PathBuf, f64>, HashMap<String, f64>) {
        let (mut bases, mut modules): (HashMap<PathBuf, f64>, HashMap<String, f64>) = (HashMap::new(), HashMap::new());
        for c in self.recent_classes(3) {
            let weight = [1.0, 0.6, 0.35][c.days_ago.min(2) as usize];
            let b = bases.entry(c.base.clone()).or_default();
            *b = b.max(30. * weight);
            let m = modules.entry(c.module.id.clone()).or_default();
            *m = m.max(15. * weight);
        }
        (bases, modules)
    }

    pub fn rank_targets(&self, facts: &FileFacts) -> Vec<Assignment> {
        let (bases, modules) = self.class_boosts();
        let all = self.config.as_ref().map(|c| c.modules.clone()).unwrap_or_default();
        filing::rank_targets(facts, &self.assignments, &all, |a| self.is_done(a), self.now, &bases, &modules).into_iter().cloned().collect()
    }

    pub fn relative_path(&self, path: &Path) -> String {
        self.root.as_ref().and_then(|r| path.strip_prefix(r).ok()).map_or_else(|| path.to_string_lossy().to_string(), super::text::slash_path)
    }

    pub fn missing_folders(&self) -> Vec<PathBuf> {
        let (Some(root), Some(c), Some(s)) = (self.root.as_ref(), self.config.as_ref(), self.semester()) else { return Vec::new() };
        schedule::expected_folders(c, s, root).into_iter().filter(|p| !p.exists()).collect()
    }

    // MARK: Feedback

    pub fn notify(&mut self, message: String, icon: &'static str, action_title: Option<String>, undo: Option<Undo>) {
        self.toast_counter += 1;
        self.toast = Some(Toast { id: self.toast_counter, message, icon, action_title: action_title.unwrap_or_else(|| tr("Rückgängig")), undo });
        self.changed();
    }

    /// Runs the action offered by the current toast.
    pub fn perform_toast_action(&mut self) {
        if let Some(action) = self.toast.take().and_then(|t| t.undo) {
            action(self);
        }
        self.changed();
    }

    // MARK: History

    /// Adds a step to the undo history without announcing it.
    fn record(&mut self, label: String, changes: Vec<Change>) {
        self.rows_moved = true;
        self.redo_steps.clear();
        self.coalescing = None;
        self.undo_steps.push(Step { label, changes });
        if self.undo_steps.len() > HISTORY_LIMIT {
            self.undo_steps.remove(0);
        }
    }

    /// Records a step and shows it as a toast that offers to take it back.
    fn commit(&mut self, message: String, icon: &'static str, changes: Vec<Change>) {
        self.record(message.clone(), changes);
        self.notify(message, icon, None, Some(Box::new(|lib: &mut Library| lib.undo())));
    }

    /// Records an edit made by continuous input; edits of the same `kind` in quick succession become one step.
    pub fn record_edit(&mut self, kind: &'static str, label: String, before: Change) {
        let now = std::time::Instant::now();
        if self.coalescing.is_some_and(|(k, at)| k == kind && now.duration_since(at) < COALESCE) && !self.undo_steps.is_empty() {
            self.redo_steps.clear();
        } else {
            self.record(label, vec![before]);
        }
        self.coalescing = Some((kind, now));
    }

    pub fn state_snapshot(&self) -> Change {
        Change::State(Box::new(self.state.clone()))
    }

    pub fn config_snapshot(&self) -> Option<Change> {
        self.config.clone().map(|c| Change::Config(Box::new(c)))
    }

    /// Whether a step ran since the last call.
    pub fn take_rows_moved(&mut self) -> bool {
        std::mem::take(&mut self.rows_moved)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_steps.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_steps.is_empty()
    }

    /// Takes back the latest step and offers to repeat it.
    pub fn undo(&mut self) {
        let Some(step) = self.undo_steps.pop() else { return };
        let label = step.label.clone();
        let inverse = self.apply_step(step);
        self.redo_steps.push(inverse);
        self.coalescing = None;
        self.notify(trf("Rückgängig: %@", &[&label]), "undo-2", Some(tr("Wiederholen")), Some(Box::new(|lib: &mut Library| lib.redo())));
    }

    /// Repeats the step taken back last and offers to take it back again.
    pub fn redo(&mut self) {
        let Some(step) = self.redo_steps.pop() else { return };
        let label = step.label.clone();
        let inverse = self.apply_step(step);
        self.undo_steps.push(inverse);
        self.coalescing = None;
        self.notify(trf("Wiederholt: %@", &[&label]), "redo-2", None, Some(Box::new(|lib: &mut Library| lib.undo())));
    }

    fn apply_step(&mut self, step: Step) -> Step {
        self.rows_moved = true;
        let mut inverse: Vec<Change> = step.changes.into_iter().rev().filter_map(|c| self.apply_change(c)).collect();
        inverse.reverse();
        self.refresh();
        Step { label: step.label, changes: inverse }
    }

    fn apply_change(&mut self, change: Change) -> Option<Change> {
        let result: Result<Change> = (|| {
            Ok(match change {
                Change::State(state) => {
                    let inverse = self.state_snapshot();
                    let mut state = *state;
                    let notes = std::mem::take(&mut self.state.notes);
                    for (id, text) in notes {
                        state.notes.insert(id, text);
                    }
                    self.state = state;
                    self.save_state();
                    inverse
                }
                Change::Config(config) => {
                    let inverse = self.config_snapshot().unwrap_or_else(|| Change::Config(config.clone()));
                    self.config = Some(*config);
                    self.write_config();
                    inverse
                }
                Change::Move { from, to } => {
                    if let Some(parent) = to.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let target = unique_path(&to);
                    move_path(&from, &target)?;
                    Change::Move { from: target, to: from }
                }
                Change::Trash(path) => {
                    trash::delete(&path)?;
                    Change::Restore(path)
                }
                Change::Restore(path) => {
                    restore_from_trash(&path)?;
                    Change::Trash(path)
                }
            })
        })();
        match result {
            Ok(inverse) => Some(inverse),
            Err(e) => {
                self.last_error = Some(trf("Rückgängig fehlgeschlagen: %@", &[&e]));
                None
            }
        }
    }

    pub fn dismiss_toast(&mut self, id: u64) {
        if self.toast.as_ref().is_some_and(|t| t.id == id) {
            self.toast = None;
            self.toast_hovered = false;
            self.changed();
        }
    }

    // MARK: Mutations

    pub fn set_status(&mut self, status: Option<Status>, id: &str, announce: bool) {
        let Some(a) = self.assignment(id).cloned() else { return };
        let before = self.state_snapshot();
        match status {
            Some(s) => self.state.overrides.insert(id.into(), Override { status: s, date: Utc::now() }),
            None => self.state.overrides.remove(id),
        };
        self.save_state();
        self.refresh();
        if !announce {
            return;
        }
        let label = status.map_or_else(|| tr("Automatisch"), Status::label);
        self.commit(trf("%@: %@", &[&a.title, &label]), status.map_or("undo-2", Status::icon), vec![before]);
    }

    pub fn toggle_done(&mut self, id: &str) {
        let Some(a) = self.assignment(id) else { return };
        let target = if self.is_done(a) { None } else { Some(if a.upload { Status::Uploaded } else { Status::Done }) };
        self.set_status(target, id, true);
    }

    pub fn confirm_all(&mut self, ids: &[String]) {
        let before = self.state_snapshot();
        for id in ids {
            self.state.overrides.insert(id.clone(), Override { status: Status::Uploaded, date: Utc::now() });
        }
        self.save_state();
        self.refresh();
        self.commit(trf("%lld als hochgeladen markiert", &[&ids.len()]), "circle-check", vec![before]);
    }

    /// Opens the filing sheet for files headed to an assignment (or anywhere when `None`).
    pub fn request_filing(&mut self, paths: Vec<PathBuf>, assignment_id: Option<String>) {
        if paths.is_empty() || assignment_id.as_deref().and_then(|id| self.assignment(id)).is_some_and(|a| !a.tracks_files()) {
            return;
        }
        self.filing_request = Some(FilingRequest { paths, assignment_id });
        self.changed();
    }

    fn inside(&self, p: &Path) -> bool {
        self.root.as_ref().is_some_and(|r| p.starts_with(r))
    }

    /// Moves files inside the workspace, copies files from outside. Returns the new path.
    pub fn file(&mut self, path: &Path, folder: &Path, name: &str, announce: bool) -> Option<PathBuf> {
        let inside = self.inside(path);
        let result: Result<PathBuf> = (|| {
            std::fs::create_dir_all(folder)?;
            let target = unique_path(&folder.join(name));
            if target == path {
                return Ok(target);
            }
            if inside { move_path(path, &target)? } else { copy_path(path, &target)? }
            Ok(target)
        })();
        match result {
            Ok(target) => {
                self.refresh();
                if announce {
                    let change = filing_change(path, &target, inside);
                    self.commit(trf("Abgelegt: %@", &[&name_of(&target)]), "inbox", vec![change]);
                }
                Some(target)
            }
            Err(e) => {
                self.last_error = Some(trf("Ablegen fehlgeschlagen: %@", &[&explain(&e)]));
                None
            }
        }
    }

    /// Files several items and shows one undoable toast.
    pub fn file_batch(&mut self, items: Vec<(PathBuf, PathBuf, String)>) {
        let mut done: Vec<Change> = Vec::new();
        let mut last = None;
        for (path, folder, name) in items {
            let inside = self.inside(&path);
            if let Some(t) = self.file(&path, &folder, &name, false) {
                done.push(filing_change(&path, &t, inside));
                last = Some(t);
            }
        }
        let Some(last) = last else { return };
        let message = if done.len() == 1 { trf("Abgelegt: %@", &[&name_of(&last)]) } else { trf("%lld Dateien abgelegt", &[&done.len()]) };
        self.commit(message, "inbox", done);
    }

    /// Renames a file in an assignment folder so it counts as task description (or back as own work).
    pub fn set_role(&mut self, role: FileRole, path: &Path, assignment_id: &str) {
        let Some(a) = self.assignment(assignment_id).cloned() else { return };
        let Some(naming) = self.naming(&a.module_id) else { return };
        let facts = FileFacts::read(path);
        let namings = self.namings();
        let name = name_of(path);
        let parsed: Option<Parsed> = namings.iter().find_map(|n| n.parse(&name)).or_else(|| self.parse_brief(&name, &a.module_id));
        let title = parsed.as_ref().map(|p| p.title.clone()).or_else(|| filing::title_suggestions(&facts, &namings, &self.title_ignore_words()).first().map(|s| s.title.clone())).unwrap_or_else(|| "Titel".into());
        let date = parsed.as_ref().and_then(|p| p.date).or(facts.name_date).unwrap_or_else(|| day::local_date(facts.created.min(a.due)));
        let ext = extension(&name);
        let new = if role == FileRole::Brief {
            naming.render(&title, date, ext, &[], a.week, a.iso_week, Some(BRIEF_TAG))
        } else {
            naming.render(&title, date, ext, &parsed.map(|p| p.coauthors).unwrap_or_default(), a.week, a.iso_week, None)
        };
        self.rename(path, &new);
    }

    fn parse_brief(&self, file_name: &str, module_id: &str) -> Option<Parsed> {
        let c = self.config.as_ref()?;
        let m = c.module(module_id)?;
        Naming::with_author(m, c.semester(&m.semester), BRIEF_TAG, "", CoauthorStyle::Initial).parse(file_name)
    }

    pub fn rename(&mut self, file: &Path, name: &str) {
        if name == name_of(file) {
            return;
        }
        let target = unique_path(&file.with_file_name(name));
        match move_path(file, &target) {
            Ok(()) => {
                self.refresh();
                let change = Change::Move { from: target.clone(), to: file.to_path_buf() };
                self.commit(trf("Umbenannt: %@", &[&name_of(&target)]), "pencil", vec![change]);
            }
            Err(e) => self.last_error = Some(trf("Umbenennen fehlgeschlagen: %@", &[&explain(&e.into())])),
        }
    }

    pub fn rename_all(&mut self, items: Vec<(PathBuf, String)>) {
        let mut done: Vec<Change> = Vec::new();
        for (file, name) in items.into_iter().filter(|(f, n)| *n != name_of(f)) {
            let target = unique_path(&file.with_file_name(&name));
            if move_path(&file, &target).is_ok() {
                done.push(Change::Move { from: target, to: file });
            }
        }
        self.refresh();
        if !done.is_empty() {
            self.commit(if done.len() == 1 { tr("1 Datei umbenannt") } else { trf("%lld Dateien umbenannt", &[&done.len()]) }, "pencil", done);
        }
    }

    pub fn trash(&mut self, path: &Path) {
        match trash::delete(path) {
            Ok(()) => {
                self.refresh();
                self.commit(trf("In den Papierkorb: %@", &[&name_of(path)]), "trash-2", vec![Change::Restore(path.to_path_buf())]);
            }
            Err(_) => self.last_error = Some(tr("Konnte nicht in den Papierkorb gelegt werden. Prüfe, ob die Datei noch existiert und nicht gerade in einer anderen App offen ist.")),
        }
    }

    /// Moves several files to the trash as one undoable step.
    pub fn trash_all(&mut self, paths: &[PathBuf]) {
        let mut done: Vec<Change> = Vec::new();
        for path in paths {
            match trash::delete(path) {
                Ok(()) => done.push(Change::Restore(path.clone())),
                Err(_) => self.last_error = Some(tr("Konnte nicht in den Papierkorb gelegt werden. Prüfe, ob die Datei noch existiert und nicht gerade in einer anderen App offen ist.")),
            }
        }
        self.refresh();
        if !done.is_empty() {
            self.commit(if done.len() == 1 { tr("1 Datei in den Papierkorb") } else { trf("%lld Dateien in den Papierkorb", &[&done.len()]) }, "trash-2", done);
        }
    }

    pub fn note(&self, id: &str) -> String {
        self.state.notes.get(id).cloned().unwrap_or_default()
    }

    /// Stores the assignment note in memory; the caller debounces `save_notes`.
    pub fn set_note(&mut self, id: &str, text: String) {
        if text.is_empty() {
            self.state.notes.remove(id);
        } else {
            self.state.notes.insert(id.into(), text);
        }
    }

    pub fn save_notes(&mut self) {
        self.save_state();
    }

    pub fn add_manual(&mut self, m: ManualAssignment) {
        let before = self.state_snapshot();
        let message = trf("%@ hinzugefügt: %@", &[&m.kind.label(), &m.title]);
        self.state.manual.push(m);
        self.save_state();
        self.refresh();
        self.commit(message, "circle-plus", vec![before]);
    }

    /// Adds or updates calendar events; returns how many were added and updated.
    pub fn import_events(&mut self, items: Vec<(CalendarEvent, String, Kind)>) -> (usize, usize) {
        let (Some(root), Some(config)) = (self.root.clone(), self.config.clone()) else { return (0, 0) };
        let (mut added, mut updated) = (0, 0);
        let before = self.state_snapshot();
        for (event, module_id, kind) in items {
            let Some(module) = config.module(&module_id) else { continue };
            let folder = if kind == Kind::Submission && root.join(&module.folder).join("Abgaben").exists() { "Abgaben".to_string() } else { String::new() };
            if let Some(m) = self.state.manual.iter_mut().find(|m| m.external_id.as_deref() == Some(event.uid.as_str())) {
                m.title = event.title();
                m.due = event.due;
                m.module = module.id.clone();
                m.kind = kind;
                m.upload = kind == Kind::Submission;
                updated += 1;
            } else {
                self.state.manual.push(ManualAssignment {
                    module: module.id.clone(),
                    title: event.title(),
                    due: event.due,
                    kind,
                    upload: kind == Kind::Submission,
                    folder,
                    external_id: Some(event.uid.clone()),
                    ..Default::default()
                });
                added += 1;
            }
        }
        self.save_state();
        self.refresh();
        self.commit(trf("%lld neu, %lld aktualisiert", &[&added, &updated]), "calendar-plus", vec![before]);
        (added, updated)
    }

    /// Moves one generated occurrence; `None` restores the rule's date.
    pub fn move_occurrence(&mut self, id: &str, date: Option<DateTime<Utc>>) {
        let before = self.state_snapshot();
        match date {
            Some(d) => self.state.moved.insert(id.into(), d),
            None => self.state.moved.remove(id),
        };
        self.save_state();
        self.refresh();
        let message = date.map_or_else(|| tr("Ursprünglicher Termin"), |d| trf("Verschoben auf %@", &[&day::due(d.with_timezone(&Local), self.now)]));
        self.commit(message, "calendar", vec![before]);
    }

    /// Counts existing files inside the workspace towards an assignment without moving them.
    pub fn link(&mut self, paths: &[PathBuf], id: &str) {
        let rels: Vec<String> = paths.iter().filter(|p| self.inside(p)).map(|p| self.relative_path(p)).collect();
        if rels.is_empty() {
            self.last_error = Some(tr("Nur Dateien im Studienordner können zugeordnet werden."));
            return;
        }
        let before = self.state_snapshot();
        let list = self.state.linked_files.entry(id.into()).or_default();
        for r in &rels {
            if !list.contains(r) {
                list.push(r.clone());
            }
        }
        self.save_state();
        self.refresh();
        self.commit(if rels.len() == 1 { tr("1 Datei zugeordnet") } else { trf("%lld Dateien zugeordnet", &[&rels.len()]) }, "link", vec![before]);
    }

    pub fn is_linked(&self, path: &Path, id: &str) -> bool {
        self.state.linked_files.get(id).is_some_and(|l| l.contains(&self.relative_path(path)))
    }

    pub fn unlink(&mut self, path: &Path, id: &str) {
        let before = self.state_snapshot();
        let rel = self.relative_path(path);
        if let Some(l) = self.state.linked_files.get_mut(id) {
            l.retain(|r| *r != rel);
            if l.is_empty() {
                self.state.linked_files.remove(id);
            }
        }
        self.save_state();
        self.refresh();
        self.commit(tr("Zuordnung entfernt"), "link", vec![before]);
    }

    pub fn update_manual(&mut self, m: ManualAssignment) {
        let before = self.state_snapshot();
        if let Some(x) = self.state.manual.iter_mut().find(|x| x.id == m.id) {
            *x = m;
            self.record_edit("manual", tr("Eintrag geändert"), before);
            self.save_state();
            self.refresh();
        }
    }

    pub fn manual_for(&self, a: &Assignment) -> Option<&ManualAssignment> {
        let raw = a.id.strip_prefix("manual.")?;
        self.state.manual.iter().find(|m| m.id == raw)
    }

    pub fn remove_manual(&mut self, id: &str) {
        let Some(m) = self.assignment(id).and_then(|a| self.manual_for(a)).cloned() else { return };
        let before = self.state_snapshot();
        self.state.overrides.remove(id);
        self.state.notes.remove(id);
        self.state.manual.retain(|x| x.id != m.id);
        if self.selection.as_deref() == Some(id) {
            self.selection = None;
        }
        self.save_state();
        self.refresh();
        self.commit(trf("Gelöscht: %@", &[&m.title]), "trash-2", vec![before]);
    }

    pub fn import_to_inbox(&mut self, paths: &[PathBuf]) {
        let (Some(root), Some(c)) = (self.root.clone(), self.config.clone()) else { return };
        let inbox = root.join(&c.inbox);
        let _ = std::fs::create_dir_all(&inbox);
        let mut count = 0;
        for p in paths {
            match copy_path(p, &unique_path(&inbox.join(name_of(p)))) {
                Ok(()) => count += 1,
                Err(e) => self.last_error = Some(trf("Import fehlgeschlagen: %@", &[&explain(&e.into())])),
            }
        }
        self.refresh();
        if count > 0 {
            let msg = if count == 1 { tr("1 Datei in der Inbox") } else { trf("%lld Dateien in der Inbox", &[&count]) };
            self.notify(msg, "inbox", None, None);
        }
    }

    pub fn create_missing_folders(&mut self) -> usize {
        let missing = self.missing_folders();
        for p in &missing {
            let _ = std::fs::create_dir_all(p);
        }
        self.refresh();
        if !missing.is_empty() {
            self.notify(trf("%lld Ordner angelegt", &[&missing.len()]), "folder-plus", None, None);
        }
        missing.len()
    }

    // MARK: Documents

    pub fn toggle_inspector(&mut self) {
        self.show_inspector = !self.show_inspector;
        self.changed();
    }

    pub fn ics(&self) -> Option<String> {
        let c = self.config.as_ref()?;
        let items: Vec<&Assignment> = self.semester_assignments().into_iter().filter(|a| self.status(a) != Status::Skipped).collect();
        Some(ics::export(&items, c, prefs::get().reminder_hours))
    }

    pub fn ics_file_name(&self) -> String {
        format!("{}-{}.ics", self.config.as_ref().map_or("Schrumbi", |c| c.name.as_str()), self.semester().map_or("", |s| s.name.as_str())).replace('/', "-")
    }

    pub fn begin_upload(&mut self, id: &str) {
        self.upload_assignment_id = Some(id.into());
        if let Some(u) = self.assignment(id).and_then(|a| self.module(a)).and_then(|m| m.upload_link()).and_then(|l| l.url()) {
            let _ = opener::open(u.as_str());
        }
        self.changed();
    }

    pub fn upload_assignment(&self) -> Option<&Assignment> {
        self.upload_assignment_id.as_deref().and_then(|id| self.assignment(id))
    }

    fn documents_key(&self) -> String {
        self.root.as_ref().map(|r| r.to_string_lossy().to_string()).unwrap_or_default()
    }

    fn load_documents(&mut self) {
        let Some(root) = self.root.clone() else { return };
        let rels = prefs::get().open_documents.get(&self.documents_key()).cloned().unwrap_or_default();
        self.open_documents = rels.iter().map(|r| root.join(r)).filter(|p| p.exists()).collect();
        self.active_document = self.open_documents.first().cloned();
        let split = prefs::get().split_documents.get(&self.documents_key()).map(|r| root.join(r));
        self.split_document = split.filter(|p| self.open_documents.contains(p) && self.active_document.as_ref() != Some(p));
    }

    fn save_documents(&self) {
        let key = self.documents_key();
        let rels: Vec<String> = self.open_documents.iter().map(|p| self.relative_path(p)).collect();
        let split = self.split_document.as_ref().map(|p| self.relative_path(p));
        prefs::update(|p| {
            p.open_documents.insert(key.clone(), rels);
            match split {
                Some(split) => p.split_documents.insert(key, split),
                None => p.split_documents.remove(&key),
            };
        });
    }

    /// Files Schrumbi shows itself instead of handing them to another app.
    pub fn opens_in_app(path: &Path) -> bool {
        !path.is_dir() && IN_APP_TYPES.contains(&extension(&name_of(path)).to_lowercase().as_str())
    }

    /// Opens a file as a tab in the main window.
    pub fn open_document(&mut self, path: &Path) {
        let path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !self.open_documents.contains(&path) {
            match self.active_document.as_ref().and_then(|a| self.open_documents.iter().position(|d| d == a)) {
                Some(i) => self.open_documents.insert(i + 1, path.clone()),
                None => self.open_documents.push(path.clone()),
            }
            if self.open_documents.len() > 15 {
                let extra = self.open_documents.len() - 15;
                self.open_documents.drain(..extra);
            }
            self.save_documents();
        }
        if self.split_document.as_ref() == Some(&path) {
            self.split_document = None;
            self.save_documents();
        }
        self.active_document = Some(path.clone());
        self.set_route(Route::Doc(path));
    }

    /// Shows an open document next to the current one, or ends that when `None`.
    pub fn set_split(&mut self, path: Option<PathBuf>) {
        self.split_document = path.filter(|p| self.open_documents.contains(p) && self.active_document.as_ref() != Some(p));
        self.save_documents();
        self.changed();
    }

    /// The buffer for a note, created on first use.
    pub fn buffer(&mut self, path: &Path) -> &mut NoteBuffer {
        self.buffers.entry(path.to_path_buf()).or_insert_with(|| NoteBuffer::open(path))
    }

    /// Removes a document tab; the caller saves or asks about unsaved changes first.
    pub fn close_document(&mut self, path: &Path) {
        let Some(i) = self.open_documents.iter().position(|d| d == path) else { return };
        if !self.note_windows.contains(path) {
            self.buffers.remove(path);
        }
        self.closed_documents.push(self.open_documents.remove(i));
        if self.split_document.as_deref() == Some(path) {
            self.split_document = None;
        }
        self.save_documents();
        let was_shown = self.route == Route::Doc(path.to_path_buf());
        if self.active_document.as_deref() == Some(path) {
            self.active_document = if self.open_documents.is_empty() { None } else { Some(self.open_documents[i.min(self.open_documents.len() - 1)].clone()) };
        }
        if was_shown {
            match self.active_document.clone() {
                Some(next) => self.route = Route::Doc(next),
                None => self.go_back(),
            }
        }
        self.changed();
    }

    /// Opens the most recently closed tab whose file still exists.
    pub fn reopen_document(&mut self) {
        while let Some(path) = self.closed_documents.pop() {
            if path.exists() {
                self.open_document(&path);
                return;
            }
        }
    }

    /// Moves a tab in front of `before`, or to the end when `before` is the tab itself or unknown.
    pub fn move_document(&mut self, path: &Path, before: &Path) {
        if path == before {
            return;
        }
        let Some(from) = self.open_documents.iter().position(|d| d == path) else { return };
        let doc = self.open_documents.remove(from);
        let to = self.open_documents.iter().position(|d| d == before).unwrap_or(self.open_documents.len());
        self.open_documents.insert(to, doc);
        self.save_documents();
        self.changed();
    }

    pub fn close_other_documents(&mut self, path: &Path) {
        self.closed_documents.extend(self.open_documents.iter().filter(|d| *d != path).cloned());
        self.split_document = None;
        let windows = self.note_windows.clone();
        self.buffers.retain(|p, _| p == path || windows.contains(p));
        self.open_documents.retain(|p| p == path);
        self.save_documents();
        self.open_document(path);
    }

    pub fn close_all_documents(&mut self) {
        self.closed_documents.extend(self.open_documents.iter().cloned());
        self.split_document = None;
        let windows = self.note_windows.clone();
        self.buffers.retain(|p, _| windows.contains(p));
        self.open_documents.clear();
        self.active_document = None;
        self.save_documents();
        if matches!(self.route, Route::Doc(_)) {
            self.go_back();
        }
        self.changed();
    }

    /// Next (`+1`) or previous (`-1`) tab, wrapping around.
    pub fn cycle_document(&mut self, step: i64) {
        if self.open_documents.is_empty() {
            return;
        }
        let n = self.open_documents.len() as i64;
        let i = self.active_document.as_ref().and_then(|a| self.open_documents.iter().position(|d| d == a)).unwrap_or(0) as i64;
        let next = self.open_documents[((i + step + n) % n) as usize].clone();
        self.open_document(&next);
    }

    pub fn show_documents(&mut self) {
        if let Some(d) = self.active_document.clone().or_else(|| self.open_documents.first().cloned()) {
            self.set_route(Route::Doc(d));
        }
    }

    pub fn go_back(&mut self) {
        while let Some(r) = self.back_routes.pop() {
            if self.route_exists(&r) {
                self.forward_routes.push(std::mem::replace(&mut self.route, r));
                self.changed();
                return;
            }
        }
        if !self.route_exists(&self.route) {
            self.route = Route::Today;
            self.changed();
        }
    }

    /// Readable name for a note: the convention title, else the file name.
    pub fn note_title(&self, path: &Path) -> String {
        let name = name_of(path);
        self.namings().iter().find_map(|n| n.parse(&name)).map(|p| p.title.replace('-', " ")).unwrap_or_else(|| super::naming::stem(&name).to_string())
    }

    /// Notes, PDFs, Office files, images and text open as tabs in the app; everything else in its default app.
    pub fn open_file(&mut self, path: &Path) {
        if Self::opens_in_app(path) {
            self.open_document(path);
        } else {
            open_path(path);
        }
    }

    // MARK: Reminders

    pub fn reminders(&self) -> Vec<Reminder> {
        let p = prefs::get();
        let Some(c) = self.config.as_ref() else { return Vec::new() };
        if !p.reminders_enabled {
            return Vec::new();
        }
        let mut out = Vec::new();
        for a in self.upcoming() {
            let code = c.module(&a.module_id).map_or("", |m| m.code.as_str());
            let title = format!("{code} · {}", a.title);
            let due = day::due(a.due, self.now);
            let soon = a.due - Duration::hours(p.reminder_hours);
            if p.reminder_hours > 0 && soon > self.now {
                out.push(Reminder { assignment_id: a.id.clone(), upload: a.upload, id: format!("{}.h", a.id), title: title.clone(), body: trf("Fällig in %lld Std. (%@)", &[&p.reminder_hours, &due]), fire: soon });
            }
            let early = a.due - Duration::days(1);
            if p.reminder_day_before && !a.optional && early > self.now {
                out.push(Reminder { assignment_id: a.id.clone(), upload: a.upload, id: format!("{}.d", a.id), title, body: trf("Fällig morgen (%@)", &[&due]), fire: early });
            }
        }
        out.sort_by_key(|r| r.fire);
        out.truncate(60);
        out
    }

    /// Snapshot for widgets and the tray: overdue first, then the next mandatory deadlines.
    pub fn tray_items(&self) -> Vec<&Assignment> {
        self.action_items.iter().take(2).chain(self.upcoming().into_iter().filter(|a| !a.optional).take(7)).take(8).collect()
    }
}

/// Avoids collisions; for convention names the counter goes before the date so the name stays valid.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let name = name_of(path);
    let re = regex::Regex::new(r"_\d{6}\.[^.]+$").unwrap();
    for i in 2.. {
        let candidate = match re.find(&name) {
            Some(m) => format!("{}-{i}{}", &name[..m.start()], &name[m.start()..]),
            None => {
                let (stem, ext) = (super::naming::stem(&name), extension(&name));
                if ext.is_empty() { format!("{stem}-{i}") } else { format!("{stem}-{i}.{ext}") }
            }
        };
        let p = path.with_file_name(candidate);
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to).or_else(|_| {
        copy_path(from, to)?;
        if from.is_dir() { std::fs::remove_dir_all(from) } else { std::fs::remove_file(from) }
    })
}

fn copy_path(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_path(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Opens a file or folder in its default app.
pub fn open_path(path: &Path) {
    if path.is_dir() || !path.exists() {
        let _ = std::fs::create_dir_all(path);
    }
    let _ = opener::open(path);
}

/// Shows a file in Finder / Explorer / the file manager.
pub fn reveal(path: &Path) {
    if path.exists() {
        let _ = opener::reveal(path);
    }
}

/// What takes back filing `original` as `target`: moving it back, or trashing the copy of an outside file.
fn filing_change(original: &Path, target: &Path, moved: bool) -> Change {
    if moved { Change::Move { from: target.to_path_buf(), to: original.to_path_buf() } } else { Change::Trash(target.to_path_buf()) }
}

#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
fn restore_from_trash(original: &Path) -> Result<()> {
    let item = trash::os_limited::list()?.into_iter().filter(|i| i.original_path() == original).max_by_key(|i| i.time_deleted).ok_or_else(|| anyhow::anyhow!("{} is not in the trash", original.display()))?;
    trash::os_limited::restore_all([item])?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn restore_from_trash(original: &Path) -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home folder"))?;
    let candidate = home.join(".Trash").join(name_of(original));
    if !candidate.exists() {
        anyhow::bail!("{} is not in the trash", name_of(original));
    }
    if let Some(parent) = original.parent() {
        std::fs::create_dir_all(parent)?;
    }
    move_path(&candidate, &unique_path(original))?;
    Ok(())
}

/// Zips the whole workspace into `destination`; run off the main thread.
pub fn export_archive(root: &Path, destination: &Path) -> Result<()> {
    use std::io::Write;
    let file = std::fs::File::create(destination)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let base = root.parent().unwrap_or(root);
    for e in walkdir::WalkDir::new(root).into_iter().filter_map(Result::ok) {
        let rel = e.path().strip_prefix(base)?.to_string_lossy().replace('\\', "/");
        if e.file_type().is_dir() {
            zip.add_directory(format!("{rel}/"), opts)?;
        } else if e.file_type().is_file() {
            zip.start_file(rel, opts)?;
            zip.write_all(&std::fs::read(e.path())?)?;
        }
    }
    zip.finish()?;
    Ok(())
}
