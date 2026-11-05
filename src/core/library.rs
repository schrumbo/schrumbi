use super::errors::explain;
use super::next_action::{self, NextAction};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
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
use super::highlights::{self, Highlight, Highlights};
use super::search_index::SearchIndex;
use super::text::fold;
use super::workspace::{self, CONFIG_NAME, STATE_NAME};
use super::{day, prefs, schedule};

/// Where a dragged document lands in the document area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocDrop {
    Here,
    Beside,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Route {
    Today,
    Grid,
    Inbox,
    Search,
    Record,
    Module(String),
    Doc(PathBuf),
    Notes,
    Graph,
    Cards,
}

/// What the details column beside a page is about. Each kind remembers on its own whether it is kept open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectorKind {
    Document,
    Assignments,
    Graph,
}

impl InspectorKind {
    const ALL: [Self; 3] = [Self::Document, Self::Assignments, Self::Graph];

    fn key(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Assignments => "assignments",
            Self::Graph => "graph",
        }
    }
}

impl Route {
    /// The details this page offers beside it; `None` for pages without any.
    pub fn inspector_kind(&self) -> Option<InspectorKind> {
        match self {
            Route::Doc(_) => Some(InspectorKind::Document),
            Route::Graph => Some(InspectorKind::Graph),
            Route::Today | Route::Grid | Route::Module(_) => Some(InspectorKind::Assignments),
            Route::Inbox | Route::Search | Route::Record | Route::Notes | Route::Cards => None,
        }
    }
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
    /// Brings a trashed file back from the copy kept for this purpose; macOS does not let apps read the trash.
    Restore { path: PathBuf, backup: PathBuf },
    /// Puts back the version of a file from before it was replaced.
    Replaced { path: PathBuf, backup: PathBuf },
    Highlights { key: String, list: Vec<Highlight> },
    /// Puts a note's text back, as chosen when another app changed the file.
    Note { path: PathBuf, text: String },
    /// Puts a file's bytes back, for restoring an earlier version.
    Bytes { path: PathBuf, bytes: Vec<u8> },
    /// Puts cards' progress back, in the vault whose cards they are.
    Cards { root: Option<PathBuf>, swaps: Vec<super::cards::Swap> },
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
    pub gone: HashSet<PathBuf>,
}

/// Work for a background scan.
pub struct ScanJob {
    pub generation: u64,
    pub requests: HashSet<scanner::FolderRequest>,
    pub inbox: PathBuf,
    pub linked: Vec<PathBuf>,
}

impl ScanJob {
    pub fn run(self) -> ScanResult {
        ScanResult {
            generation: self.generation,
            listings: scanner::listings(&self.requests),
            inbox: scanner::files(&self.inbox, "*"),
            gone: self.linked.into_iter().filter(|p| !p.exists()).collect(),
        }
    }
}

/// Work for a background index update.
pub struct IndexJob {
    pub index: Arc<SearchIndex>,
    pub root: PathBuf,
    pub exclude: Vec<String>,
    pub config: StudyConfig,
    /// A vault outside the workspace.
    pub vault: Option<PathBuf>,
}

impl IndexJob {
    pub fn run(self) -> (usize, Vec<CatalogFile>) {
        let mut roots = vec![self.root.clone()];
        roots.extend(self.vault.clone());
        let count = self.index.update_all(&roots, &self.exclude);
        let all = self.index.all();
        let mut catalog = catalog::build(&all, &self.root, &self.config);
        if let Some(vault) = &self.vault {
            let mut mapped = self.config.clone();
            for module in &mut mapped.modules {
                module.folder = module.notes_folder.clone().unwrap_or_else(|| module.folder.clone());
            }
            catalog.extend(catalog::build(&all, vault, &mapped));
        }
        let vault_root = self.vault.as_ref().unwrap_or(&self.root);
        super::vault::index::rebuild(vault_root, &self.exclude);
        super::cards::open(&dunce::canonicalize(vault_root).unwrap_or_else(|_| vault_root.clone()));
        super::cards::reconcile();
        super::cards::settle(super::cards::now());
        (count, catalog)
    }
}

const IN_APP_TYPES: [&str; 57] = [
    "base", "canvas", "md", "markdown", "mdown", "pdf", "txt", "rtf", "rtfd", "csv", "json", "html", "htm", "xml", "css", "js", "ts", "swift", "java", "py", "c", "cpp", "h", "yml", "yaml", "tex", "doc", "docx", "docm", "ppt",
    "pptx", "pptm", "xls", "xlsx", "xlsm", "xlsb", "pages", "key", "numbers", "odt", "ods", "odp", "png", "jpg", "jpeg", "heic", "gif", "webp", "tiff", "svg", "bmp", "mp4", "mov", "m4v", "mp3", "m4a", "wav",
];
const MATERIAL_WORDS: [&str; 12] = ["unterlagen", "material", "folien", "skript", "slides", "handout", "literatur", "vorlesung", "lecture", "docs", "mitschrift", "notes"];
const NOTE_WORDS: [&str; 4] = ["mitschrift", "notiz", "notes", "note"];

/// Set while a link is clicked with the command key, so it opens a new tab.
pub static LINK_IN_NEW_TAB: AtomicBool = AtomicBool::new(false);

/// Central app state: workspace, config, user state and everything derived from them.
pub struct Library {
    pub root: Option<PathBuf>,
    /// `root` with symlinks, short names and `..` resolved, as editor tabs and the notes tree spell paths.
    settled_root: Option<PathBuf>,
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
    highlights: Highlights,
    naming_cache: std::sync::Mutex<Option<(String, Arc<Vec<Naming>>)>>,
    pub now: DateTime<Local>,
    pub selected_semester_id: Option<String>,
    selection: Option<String>,
    /// Bumped whenever something is selected, so a click can tell whether it selected anything.
    selection_stamp: u64,
    /// The selection left behind on a page, given back when coming back to it.
    parked_selection: Option<(Route, String)>,
    /// Details kept open by the user, per kind.
    pinned_inspectors: HashSet<InspectorKind>,
    /// The document details were opened for one look, e.g. at its versions; leaving the page closes them.
    inspector_peek: bool,
    pub show_quick_add: bool,
    /// The kind the next quick add starts on, when the place it was opened from implies one.
    pub quick_add_kind: Option<Kind>,
    /// The day the next quick add starts on, when it was opened from a day.
    pub quick_add_day: Option<chrono::NaiveDate>,
    pub show_quick_open: bool,
    /// A deadline waiting for a file to be picked and linked to it.
    pub link_pick: Option<String>,
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
    closed_documents: Vec<(PathBuf, usize)>,
    /// The open document shown beside the current one.
    pub split_document: Option<PathBuf>,
    /// Where the last rename ended up, for the list that started it.
    pub last_renamed: Option<PathBuf>,
    /// Notes just made here, which open for writing whatever was shown last.
    fresh_notes: std::collections::HashSet<PathBuf>,
    leaving: HashSet<PathBuf>,
    leaving_forms: HashMap<PathBuf, PathBuf>,
    leaving_folder: bool,
    /// A heading whose name is taken was reported once.
    heading_clash: Option<(PathBuf, String)>,
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
    config_created: bool,
    /// A deck the flashcards page should open and start.
    pub cards_request: Option<String>,
    /// The dialog that names a set of tabs is wanted.
    pub show_save_layout: bool,
    pub filing_request: Option<FilingRequest>,
    /// Linked files the last scan did not find.
    gone_links: HashSet<PathBuf>,
    action_items: Vec<Assignment>,
    listings: Listings,
    scan_generation: u64,
    toast_counter: u64,
    /// A step ran since the last check, so lists may have changed order.
    rows_moved: bool,
    undo_steps: Vec<Step>,
    redo_steps: Vec<Step>,
    /// The kind and time of the last edit that merges with the following ones, e.g. typing in a settings field.
    coalescing: Option<(String, std::time::Instant)>,
    pub buffers: HashMap<PathBuf, NoteBuffer>,
    pending_link_changes: Vec<Change>,
    /// Whether the workspace is an Obsidian vault.
    pub is_vault: bool,
    /// Asks for the note picker: `(merge, note)`; merge chooses where a note goes, otherwise which template to insert.
    pub note_pick: Option<(bool, PathBuf)>,
    /// The note shown as slides.
    pub slides: Option<PathBuf>,
    /// Size and time of open files, to recognize them after another app moved them.
    document_stamps: HashMap<PathBuf, (u64, std::time::SystemTime)>,
    /// Asks the details column to show a note's versions.
    pub show_versions: bool,
    pub show_properties: bool,
    /// The note the graph centers on and how far it reaches; `None` shows the whole vault.
    pub graph_focus: Option<(PathBuf, usize)>,
    /// The tag the notes page filters by.
    pub notes_tag: Option<String>,
    /// The tab that the next opened note replaces, until it is edited or opened for good.
    pub preview_document: Option<PathBuf>,
    /// The note picked in the graph, shown beside it.
    pub graph_selection: Option<PathBuf>,
    /// Where each tab came from and went to while following links.
    tab_trails: HashMap<PathBuf, (Vec<PathBuf>, Vec<PathBuf>)>,
    /// A file whose name the explorer should turn into a field.
    pub rename_request: Option<PathBuf>,
    /// A folder the explorer should open and select.
    pub reveal_folder: Option<String>,
    pub focus_explorer: bool,
    pub focus_document: bool,
    /// Asks the main window to put the focus on the note on the right.
    pub focus_beside: bool,
    /// The last click went into the note on the right; links from there open on the right.
    pub beside_active: bool,
    /// Opens the next document without moving the keyboard to it.
    pub quiet_open: bool,
    /// Asks the shown note to switch between reading and writing, when the keyboard is elsewhere.
    pub toggle_mode: bool,
    pub show_sync_setup: bool,
    pub show_conflicts: bool,
    /// The toast asked to take back the last conflict resolution.
    pub sync_undo_requested: bool,
    /// A file was put where it belongs; automatic sync takes it up right away.
    pub sync_soon: bool,
    /// Undo or redo changed files; the index catches up right away.
    pub notes_touched: bool,
    /// A sync is rewriting the workspace; writes wait until `finish_sync`.
    sync_gate: bool,
    deferred_config: bool,
    deferred_state: bool,
    weeks_cache: Option<(Semester, Vec<LectureWeek>)>,
    /// Bumped on every change, so views can skip work when nothing changed.
    pub revision: u64,
}

fn pinned_from_prefs() -> HashSet<InspectorKind> {
    let keys = prefs::get().inspector_pinned;
    InspectorKind::ALL.into_iter().filter(|k| keys.contains(k.key())).collect()
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
            settled_root: None,
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
            highlights: Highlights::new(),
            naming_cache: Default::default(),
            now: Local::now(),
            selected_semester_id: None,
            selection: None,
            selection_stamp: 0,
            parked_selection: None,
            pinned_inspectors: if cfg!(test) { HashSet::new() } else { pinned_from_prefs() },
            inspector_peek: false,
            show_quick_add: false,
            quick_add_kind: None,
            quick_add_day: None,
            show_quick_open: false,
            link_pick: None,
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
            last_renamed: None,
            fresh_notes: Default::default(),
            leaving: HashSet::new(),
            leaving_forms: HashMap::new(),
            leaving_folder: false,
            heading_clash: None,
            preview_document: None,
            graph_selection: None,
            tab_trails: HashMap::new(),
            rename_request: None,
            reveal_folder: None,
            focus_explorer: false,
            focus_document: false,
            focus_beside: false,
            beside_active: false,
            quiet_open: false,
            toggle_mode: false,
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
            config_created: false,
            cards_request: None,
            show_save_layout: false,
            filing_request: None,
            gone_links: HashSet::new(),
            action_items: Vec::new(),
            listings: HashMap::new(),
            scan_generation: 0,
            toast_counter: 0,
            rows_moved: false,
            undo_steps: Vec::new(),
            redo_steps: Vec::new(),
            coalescing: None,
            buffers: HashMap::new(),
            pending_link_changes: Vec::new(),
            is_vault: false,
            note_pick: None,
            slides: None,
            document_stamps: HashMap::new(),
            show_versions: false,
            show_properties: false,
            graph_focus: None,
            notes_tag: None,
            show_sync_setup: false,
            show_conflicts: false,
            sync_undo_requested: false,
            sync_soon: false,
            notes_touched: false,
            sync_gate: false,
            deferred_config: false,
            deferred_state: false,
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
        self.settled_root = Some(dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()));
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

    /// Keeps the previous version of a file about to be replaced, for undo.
    pub fn keep_for_undo(&self, path: &Path) -> Option<PathBuf> {
        path.exists().then(|| stash(path).ok()).flatten()
    }

    /// Records an exported file as a step: undo removes it, or brings back the version it replaced. The export belongs to the deadlines its source belongs to.
    pub fn record_export(&mut self, target: &Path, replaced: Option<PathBuf>, source: &Path) {
        let mut changes = vec![match replaced {
            Some(backup) => Change::Replaced { path: target.to_path_buf(), backup },
            None => Change::Trash(target.to_path_buf()),
        }];
        let ids: Vec<String> = self.deadlines_of(source).into_iter().map(|a| a.id).collect();
        let rel = self.relative_path(target);
        if !ids.is_empty() && self.inside(target) && ids.iter().any(|id| !self.state.linked_files.get(id).is_some_and(|l| l.contains(&rel))) {
            changes.push(self.state_snapshot());
            for id in ids {
                let list = self.state.linked_files.entry(id).or_default();
                if !list.contains(&rel) {
                    list.push(rel.clone());
                }
            }
            self.gone_links.remove(target);
            self.save_state();
            self.reevaluate();
        }
        self.record(trf("PDF erstellt: %@", &[&name_of(target)]), changes);
    }

    pub fn close(&mut self) {
        self.undo_steps.clear();
        self.redo_steps.clear();
        self.coalescing = None;
        self.root = None;
        self.settled_root = None;
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
        self.parked_selection = None;
        self.inspector_peek = false;
        self.graph_selection = None;
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
        self.config_created = false;
        match workspace::load::<StudyConfig>(&path) {
            Ok(Some(c)) => {
                self.read_only = c.version > CURRENT_VERSION;
                self.last_error = self.read_only.then(|| tr("schrumbi.json stammt aus einer neueren App-Version. Änderungen werden nicht gespeichert, bis du die App aktualisierst."));
                self.config = Some(c);
                self.config_broken = false;
            }
            Ok(None) => {
                let mut c = workspace::starter_config(&root);
                if root.join(".obsidian").is_dir() {
                    c.modules.clear();
                }
                let _ = workspace::save(&c, &path);
                self.config = Some(c);
                self.needs_setup = true;
                self.config_created = true;
                self.last_error = None;
            }
            Err(e) => {
                self.config_broken = true;
                self.last_error = Some(trf("schrumbi.json ist fehlerhaft: %@", &[&format!("{e:#}")]));
            }
        }
        self.changed();
    }

    fn highlights_path(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join(highlights::FILE_NAME))
    }

    /// The highlights of a document, by its content key.
    pub fn highlights_of(&self, key: &str) -> Vec<Highlight> {
        self.highlights.get(key).cloned().unwrap_or_default()
    }

    /// Replaces a document's highlights as one step of the undo history.
    pub fn set_highlights(&mut self, key: &str, list: Vec<Highlight>, message: String) {
        let before = self.store_highlights(key, list);
        self.commit(message, "highlighter", vec![Change::Highlights { key: key.to_string(), list: before }]);
    }

    fn store_highlights(&mut self, key: &str, list: Vec<Highlight>) -> Vec<Highlight> {
        let before = if list.is_empty() { self.highlights.remove(key) } else { self.highlights.insert(key.to_string(), list) };
        if let Some(path) = self.highlights_path().filter(|_| !self.read_only)
            && let Err(e) = workspace::save(&self.highlights, &path)
        {
            self.last_error = Some(trf("Speichern fehlgeschlagen: %@", &[&e]));
        }
        before.unwrap_or_default()
    }

    fn load_state(&mut self) {
        prune_undo_copies();
        self.highlights = self.highlights_path().and_then(|p| workspace::load::<Highlights>(&p).ok().flatten()).unwrap_or_default();
        let Some(path) = self.state_path() else { return };
        match workspace::load::<StudyState>(&path) {
            Ok(s) => self.state = s.unwrap_or_else(|| StudyState { links_adopted: self.config_created, ..Default::default() }),
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
        if self.sync_gate {
            self.deferred_config = true;
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
        if self.sync_gate {
            self.deferred_state = true;
            return;
        }
        if let Err(e) = workspace::save(&self.state, &path) {
            self.last_error = Some(trf("Speichern fehlgeschlagen: %@", &[&e]));
        }
    }

    /// Picks up `schrumbi.json` / state edits made outside the app; true when something changed.
    pub fn reload_external(&mut self) -> bool {
        if self.sync_gate {
            return false;
        }
        let mut changed = false;
        if let Some(p) = self.config_path()
            && let Ok(Some(c)) = workspace::load::<StudyConfig>(&p)
            && Some(&c) != self.config.as_ref()
        {
            if c.version > CURRENT_VERSION {
                self.read_only = true;
                self.last_error = Some(tr("schrumbi.json stammt aus einer neueren App-Version. Änderungen werden nicht gespeichert, bis du die App aktualisierst."));
            }
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

    // MARK: Links

    /// The vault connected from outside the workspace, if any still exists.
    pub fn external_vault(&self) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        prefs::get().vaults.get(&root.to_string_lossy().to_string()).filter(|p| p.is_dir()).cloned()
    }

    /// Where the vault's files live: outside if one is connected, otherwise the workspace.
    pub fn vault_root(&self) -> Option<PathBuf> {
        let root = self.external_vault().or_else(|| self.root.clone())?;
        Some(dunce::canonicalize(&root).unwrap_or(root))
    }

    /// Connects or removes the vault outside this workspace; modules find their folder by name.
    pub fn connect_vault(&mut self, vault: Option<PathBuf>) {
        let Some(root) = self.root.clone() else { return };
        let key = root.to_string_lossy().to_string();
        prefs::update(|p| {
            match &vault {
                Some(v) => {
                    p.vaults.insert(key.clone(), v.clone());
                }
                None => {
                    p.vaults.remove(&key);
                }
            }
        });
        if let (Some(vault), Some(config)) = (&vault, self.config.clone()) {
            let mut updated = config.clone();
            for module in &mut updated.modules {
                if module.notes_folder.is_none() {
                    module.notes_folder = [module.folder.as_str(), module.code.as_str(), module.name.as_str()].into_iter().find(|name| !name.is_empty() && vault.join(name).is_dir()).map(String::from);
                }
            }
            if updated != config {
                self.save_config(updated);
            }
        }
        self.refresh();
    }

    /// Shows the graph, around a note or of the whole vault.
    pub fn show_graph(&mut self, around: Option<PathBuf>) {
        self.graph_focus = around.map(|p| (p, 1));
        self.set_route(Route::Graph);
    }

    /// Shows the notes with a tag.
    pub fn show_tag(&mut self, tag: &str) {
        self.notes_tag = Some(tag.to_lowercase());
        self.show_notes();
    }

    /// The notes room, with the last open note if there is one.
    pub fn show_notes(&mut self) {
        match self.active_document.clone().or_else(|| self.open_documents.first().cloned()) {
            Some(d) => self.set_route(Route::Doc(d)),
            None => self.set_route(Route::Notes),
        }
    }

    /// Opens a file in the preview tab, which the next one replaces.
    pub fn open_preview(&mut self, path: &Path) {
        self.preview_in(path, false);
    }

    /// Shows a file as the preview tab in the left pane or, with `right`, in the split.
    /// Only an untouched preview that is not on screen in the other pane makes room for it.
    fn preview_in(&mut self, path: &Path, right: bool) {
        let path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let right = right && self.split_document.is_some() && matches!(self.route, Route::Doc(_));
        if self.open_documents.contains(&path) || !Self::opens_in_app(&path) {
            if right && self.open_documents.contains(&path) && self.active_document.as_ref() != Some(&path) {
                self.show_in(path, true);
            } else {
                self.open_file(&path);
            }
            return;
        }
        let shown = self.shown_document().cloned();
        let (here, other) = if right { (self.split_document.clone(), shown) } else { (shown, self.split_document.clone()) };
        let replaced = self.preview_document.clone().filter(|p| self.open_documents.contains(p) && other.as_ref() != Some(p) && !self.buffers.get(p).is_some_and(|b| b.dirty || b.touched));
        match replaced {
            Some(old) => {
                let at = self.open_documents.iter().position(|d| *d == old).unwrap_or(0);
                self.open_documents[at] = path.clone();
                if !self.note_windows.contains(&old) {
                    self.buffers.remove(&old);
                }
                let trail = self.tab_trails.remove(&old);
                if here.as_ref() == Some(&old) {
                    let (mut back, _) = trail.unwrap_or_default();
                    back.push(old);
                    self.tab_trails.insert(path.clone(), (back, Vec::new()));
                }
            }
            None => {
                let at = here.as_ref().and_then(|h| self.open_documents.iter().position(|d| d == h)).map_or(self.open_documents.len(), |i| i + 1);
                self.open_documents.insert(at, path.clone());
            }
        }
        self.preview_document = Some(path.clone());
        self.follow_outside_moves();
        self.show_in(path, right);
    }

    /// Shows an open document in the left pane or, with `right`, in the split, and remembers the tabs.
    fn show_in(&mut self, path: PathBuf, right: bool) {
        self.beside_active = right;
        if right {
            self.split_document = Some(path);
            self.focus_beside = true;
            self.save_documents();
            self.changed();
        } else {
            self.active_document = Some(path.clone());
            self.save_documents();
            self.set_route(Route::Doc(path));
        }
    }

    /// A module's deadlines that are still open, soonest first.
    pub fn module_deadlines(&self, module: &str) -> Vec<Assignment> {
        let mut open: Vec<Assignment> = self.semester_assignments().into_iter().filter(|a| a.module_id == module && !self.status(a).is_closed()).cloned().collect();
        open.sort_by_key(|a| a.due);
        open
    }

    /// A tab the next opened file may still replace: the preview, as long as nothing was written in it.
    pub fn is_preview(&self, path: &Path) -> bool {
        self.preview_document.as_deref() == Some(path) && !self.buffers.get(path).is_some_and(|b| b.touched)
    }

    /// Keeps a preview tab open for good; Back no longer swaps it out.
    pub fn pin_document(&mut self, path: &Path) {
        if self.preview_document.as_deref() == Some(path) {
            self.preview_document = None;
            self.tab_trails.remove(path);
            self.changed();
        }
    }

    /// A new, empty note beside the open one, named `Unbenannt`, ready to be named.
    pub fn new_untitled_note(&mut self, folder: Option<&Path>) -> Option<PathBuf> {
        let vault = super::vault::index::current();
        let beside = self.active_document.as_deref().and_then(Path::parent).filter(|p| p.starts_with(&vault.root)).map(Path::to_path_buf);
        let folder = folder.map(Path::to_path_buf).or(beside).or_else(|| self.vault_root().map(|r| vault.config.new_note_folder(&r, None)))?;
        let target = unique_path(&folder.join(format!("{}.md", tr("Unbenannt"))));
        let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&target, ""));
        if let Err(e) = written {
            self.last_error = Some(trf("Notiz konnte nicht angelegt werden: %@", &[&explain(&e.into())]));
            return None;
        }
        self.mark_made_here(&target);
        self.refresh();
        self.commit(trf("Angelegt: %@", &[&name_of(&target)]), "square-pen", vec![Change::Trash(target.clone())]);
        self.open_document(&target);
        Some(target)
    }

    /// Swaps the shown preview tab's document, as Back and Forward do.
    fn replace_tab(&mut self, old: &Path, new: &Path) {
        if let Some(at) = self.open_documents.iter().position(|d| d == old) {
            self.open_documents[at] = new.to_path_buf();
        }
        if !self.note_windows.contains(old) {
            self.buffers.remove(old);
        }
        if self.preview_document.as_deref() == Some(old) {
            self.preview_document = Some(new.to_path_buf());
        }
        self.follow_outside_moves();
        self.active_document = Some(new.to_path_buf());
        let from = std::mem::replace(&mut self.route, Route::Doc(new.to_path_buf()));
        self.arrived(&from);
        self.save_documents();
        self.changed();
    }

    /// Walks to a note as a link in `from` does.
    pub fn follow_path(&mut self, path: &Path, from: Option<&Path>) {
        self.follow(path, from);
    }

    /// Follows a link as a preview in the pane of the note it sits in, or as a kept tab when asked.
    /// A kept tab is never swapped out.
    fn follow(&mut self, path: &Path, from: Option<&Path>) {
        let path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let new_tab = LINK_IN_NEW_TAB.swap(false, Ordering::Relaxed);
        if new_tab || !path.exists() || !Self::opens_in_app(&path) {
            self.open_file(&path);
            return;
        }
        let from = from.map(|f| dunce::canonicalize(f).unwrap_or_else(|_| f.to_path_buf()));
        let in_pane = matches!(self.route, Route::Doc(_)) && from.as_ref().is_some_and(|f| self.beside_active && !self.open_documents.contains(f));
        let right = in_pane || (from.is_some() && from == self.split_document);
        self.preview_in(&path, right);
    }

    fn save_tab(&mut self, path: &Path) -> bool {
        match self.buffers.get_mut(path) {
            Some(b) if b.conflict.is_some() => false,
            Some(b) if b.dirty => b.save().is_ok(),
            _ => true,
        }
    }

    fn step_tab(&mut self, backwards: bool) -> bool {
        let Route::Doc(current) = self.route.clone() else { return false };
        if !self.is_preview(&current) {
            return false;
        }
        let Some((mut back, mut forward)) = self.tab_trails.remove(&current) else { return false };
        let target = if backwards { back.pop() } else { forward.pop() };
        let usable = target.filter(|t| t.exists() && !self.open_documents.contains(t));
        let Some(target) = usable.filter(|_| self.save_tab(&current)) else {
            self.tab_trails.insert(current, (back, forward));
            return false;
        };
        if backwards { forward.push(current.clone()) } else { back.push(current.clone()) }
        self.tab_trails.insert(target.clone(), (back, forward));
        self.replace_tab(&current, &target);
        true
    }

    fn tab_can_step(&self, backwards: bool) -> bool {
        let Route::Doc(current) = &self.route else { return false };
        self.is_preview(current) && self.tab_trails.get(current).is_some_and(|(b, f)| !if backwards { b } else { f }.is_empty())
    }

    /// Today's daily note from the vault's settings, created from its template if needed.
    pub fn open_daily_note(&mut self) {
        let Some(root) = self.vault_root() else { return };
        let settings = super::vault::config::VaultConfig::read(&root);
        let now = Local::now();
        let english = super::i18n::is_english();
        let format = if settings.daily.format.is_empty() { "YYYY-MM-DD" } else { settings.daily.format.as_str() };
        let name = super::vault::moment::format(format, now, english);
        let path = root.join(settings.daily.folder.trim_matches('/')).join(format!("{name}.md"));
        if !path.exists() {
            let template = (!settings.daily.template.is_empty()).then(|| {
                let t = settings.daily.template.trim_start_matches('/');
                root.join(if t.ends_with(".md") { t.to_string() } else { format!("{t}.md") })
            });
            let text = template.and_then(|t| std::fs::read_to_string(t).ok()).map_or(String::new(), |t| {
                let title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                super::vault::moment::fill_template(&t, &title, now, &settings.templates.date_format, &settings.templates.time_format, english)
            });
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&path, text) {
                self.last_error = Some(trf("Notiz konnte nicht angelegt werden: %@", &[&explain(&e.into())]));
                return;
            }
            self.refresh();
            self.commit(trf("Angelegt: %@", &[&name_of(&path)]), "square-pen", vec![Change::Trash(path.clone())]);
        }
        self.open_document(&path);
    }

    /// A new note named by the unique note creator's format.
    pub fn open_unique_note(&mut self) {
        let Some(root) = self.vault_root() else { return };
        let settings = super::vault::config::unique_notes(&root);
        let now = Local::now();
        let english = super::i18n::is_english();
        let format = if settings.format.is_empty() { "YYYYMMDDHHmm" } else { settings.format.as_str() };
        let name = super::vault::moment::format(format, now, english);
        let path = unique_path(&root.join(settings.folder.trim_matches('/')).join(format!("{name}.md")));
        let vault = super::vault::config::VaultConfig::read(&root);
        let text = (!settings.template.is_empty()).then(|| {
            let t = settings.template.trim_start_matches('/');
            root.join(if t.ends_with(".md") { t.to_string() } else { format!("{t}.md") })
        }).and_then(|t| std::fs::read_to_string(t).ok()).map_or(String::new(), |t| super::vault::moment::fill_template(&t, &name, now, &vault.templates.date_format, &vault.templates.time_format, english));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&path, text) {
            Ok(()) => {
                self.refresh();
                self.commit(trf("Angelegt: %@", &[&name_of(&path)]), "square-pen", vec![Change::Trash(path.clone())]);
                self.open_document(&path);
            }
            Err(e) => self.last_error = Some(trf("Notiz konnte nicht angelegt werden: %@", &[&explain(&e.into())])),
        }
    }

    /// Changes a note in one step. Reads the file first, so what another app just saved is never overwritten.
    pub fn edit_note(&mut self, path: &Path, kind: &str, label: String, edit: impl FnOnce(&str) -> String) -> bool {
        let buffer = self.buffer(path);
        buffer.reload_if_changed();
        if buffer.conflict.is_some() {
            self.last_error = Some(trf("„%@“ wurde auch in einer anderen App geändert. Entscheide zuerst, welche Version bleibt.", &[&name_of(path)]));
            return false;
        }
        let before = buffer.text.clone();
        let updated = edit(&before);
        if updated == before {
            return false;
        }
        match self.buffer(path).restore(updated) {
            Ok(()) => {
                self.pin_document(path);
                self.record_edit(kind, label, Change::Note { path: path.to_path_buf(), text: before });
                true
            }
            Err(e) => {
                self.last_error = Some(explain(&e.into()));
                false
            }
        }
    }

    /// Ticks off a task from the overview. The line comes from the last index, so the note may have moved on:
    /// the task is looked up again by its text, and if that is not clear nothing changes.
    pub fn finish_task(&mut self, path: &Path, line: usize, task: &str) -> bool {
        let buffer = self.buffer(path);
        buffer.reload_if_changed();
        if buffer.conflict.is_some() {
            self.last_error = Some(trf("„%@“ wurde auch in einer anderen App geändert. Entscheide zuerst, welche Version bleibt.", &[&name_of(path)]));
            return false;
        }
        let before = buffer.text.clone();
        let open = super::vault::index::open_tasks(&before);
        let same: Vec<usize> = open.iter().filter(|(_, t)| t == task).map(|(l, _)| *l).collect();
        let Some(at) = same.iter().copied().find(|l| *l == line).or(if same.len() == 1 { Some(same[0]) } else { None }) else { return false };
        match self.buffer(path).restore(super::markdown::toggle_task(&before, at)) {
            Ok(()) => {
                self.commit(trf("Erledigt: %@", &[&task.to_string()]), "check", vec![Change::Note { path: path.to_path_buf(), text: before }]);
                true
            }
            Err(e) => {
                self.last_error = Some(explain(&e.into()));
                false
            }
        }
    }

    /// Appends a template to a note, filled for this moment.
    pub fn insert_template(&mut self, note: &Path, template: &Path) {
        let Some(root) = self.vault_root() else { return };
        let settings = super::vault::config::VaultConfig::read(&root);
        let Ok(raw) = std::fs::read_to_string(template) else { return };
        let title = note.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let filled = super::vault::moment::fill_template(&raw, &title, Local::now(), &settings.templates.date_format, &settings.templates.time_format, super::i18n::is_english());
        self.edit_note(note, "template", tr("Vorlage eingefügt"), |before| {
            let separator = if before.is_empty() || before.ends_with('\n') { "" } else { "\n" };
            format!("{before}{separator}{filled}")
        });
    }

    /// Moves part of a note into a new note beside it and leaves a link in its place.
    pub fn extract_to_note(&mut self, note: &Path, range: std::ops::Range<usize>) -> Option<PathBuf> {
        let buffer = self.buffer(note);
        buffer.reload_if_changed();
        if buffer.conflict.is_some() {
            return None;
        }
        let before = buffer.text.clone();
        let selected = before.get(range.clone())?.to_string();
        if selected.trim().is_empty() {
            return None;
        }
        let title: String = selected.lines().find(|l| !l.trim().is_empty()).unwrap_or_default().trim_start_matches(['#', ' ', '-', '>']).chars().filter(|c| !"\\/:*?\"<>|#^[]".contains(*c)).take(60).collect();
        let title = if title.trim().is_empty() { tr("Neue Notiz") } else { title.trim().to_string() };
        let folder = note.parent()?.to_path_buf();
        let path = unique_path(&folder.join(format!("{title}.md")));
        if let Err(e) = std::fs::write(&path, &selected) {
            self.last_error = Some(trf("Notiz konnte nicht angelegt werden: %@", &[&explain(&e.into())]));
            return None;
        }
        let vault = super::vault::index::current();
        let link = vault.new_link(Some(note), &path, "");
        let updated = format!("{}{link}{}", &before[..range.start], &before[range.end..]);
        if let Err(e) = self.buffer(note).restore(updated) {
            let _ = std::fs::remove_file(&path);
            self.last_error = Some(explain(&e.into()));
            return None;
        }
        self.refresh();
        self.commit(trf("Ausgelagert: %@", &[&name_of(&path)]), "scissors", vec![Change::Trash(path.clone()), Change::Note { path: note.to_path_buf(), text: before }]);
        Some(path)
    }

    /// Adds a note to the end of another one, points its links there and puts the original in the trash.
    pub fn merge_note(&mut self, source: &Path, target: &Path) {
        let text = self.buffers.get(source).map(|b| b.text.clone()).or_else(|| std::fs::read_to_string(source).ok()).unwrap_or_default();
        let body = text[super::vault::frontmatter::body_offset(&text)..].trim_start().to_string();
        let buffer = self.buffer(target);
        buffer.reload_if_changed();
        if buffer.conflict.is_some() {
            self.last_error = Some(trf("„%@“ wurde auch in einer anderen App geändert. Entscheide zuerst, welche Version bleibt.", &[&name_of(target)]));
            return;
        }
        let before = buffer.text.clone();
        let separator = if before.is_empty() { "" } else if before.ends_with("\n\n") { "" } else if before.ends_with('\n') { "\n" } else { "\n\n" };
        if let Err(e) = buffer.restore(format!("{before}{separator}{body}")) {
            self.last_error = Some(explain(&e.into()));
            return;
        }
        let (mut changes, _) = self.relink(source, target);
        changes.push(Change::Note { path: target.to_path_buf(), text: before });
        match trash_with_copy(source) {
            Ok(change) => changes.push(change),
            Err(_) => self.last_error = Some(tr("Konnte nicht in den Papierkorb gelegt werden. Prüfe, ob die Datei noch in einer anderen App geöffnet ist.")),
        }
        self.close_document(source);
        self.refresh();
        self.commit(trf("Zusammengeführt mit %@", &[&name_of(target)]), "merge", changes);
        self.open_document(target);
    }

    /// Opens any note of the vault by chance.
    pub fn open_random_note(&mut self) {
        let vault = super::vault::index::current();
        let notes: Vec<&PathBuf> = vault.notes.keys().filter(|p| super::markdown::is_markdown(p)).collect();
        if notes.is_empty() {
            return;
        }
        let pick = uuid::Uuid::new_v4().as_u128() as usize % notes.len();
        let path = notes[pick].clone();
        self.open_file(&path);
    }

    /// Follows an internal link from the note `from`: opens the file and lands on its heading or block.
    /// Returns false when the target does not exist yet.
    pub fn open_link(&mut self, destination: &str, from: Option<&Path>) -> bool {
        use super::vault::{index, links};
        let target = links::LinkTarget::parse(destination);
        let vault = index::current();
        let beside = from.and_then(Path::parent).map(|d| d.join(&target.path)).filter(|p| !target.path.is_empty() && p.is_file());
        let Some(path) = beside.or_else(|| vault.resolver.resolve(&target, from)).or_else(|| from.filter(|_| target.path.is_empty()).map(Path::to_path_buf)) else { return false };
        self.follow(&path, from);
        let text = if target.headings.is_empty() && target.block.is_none() { None } else { std::fs::read_to_string(&path).ok() };
        if let Some(text) = text {
            let spot = match (&target.block, target.headings.last()) {
                (Some(id), _) => links::block_ids(&text).into_iter().find(|(b, _)| b == id).map(|(_, line)| (format!("^{id}"), line)),
                (None, Some(h)) => links::headings(&text).into_iter().find(|(_, t, _)| t.trim().eq_ignore_ascii_case(h.trim())).map(|(_, t, line)| (t, line)),
                _ => None,
            };
            if let Some((term, line)) = spot {
                let nth = links::occurrences_before(&text, &term, line);
                self.find_in_file = Some((path, term, nth));
            }
        }
        true
    }

    /// Creates the note a link points at, where the vault wants new notes, and opens it.
    pub fn create_link_target(&mut self, destination: &str, from: Option<&Path>) {
        use super::vault::{index, links};
        let target = links::LinkTarget::parse(destination);
        let vault = index::current();
        let Some(root) = self.vault_root() else { return };
        let relative = if target.path.ends_with(".md") { target.path.clone() } else { format!("{}.md", target.path) };
        let path = if relative.contains('/') { root.join(relative.trim_start_matches('/')) } else { vault.config.new_note_folder(&root, from).join(&relative) };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if path.exists() {
            self.open_document(&path);
            return;
        }
        match std::fs::write(&path, "") {
            Ok(()) => {
                self.mark_made_here(&path);
                self.refresh();
                self.commit(trf("Angelegt: %@", &[&name_of(&path)]), "square-pen", vec![Change::Trash(path.clone())]);
                self.open_document(&path);
            }
            Err(e) => self.last_error = Some(trf("Notiz konnte nicht angelegt werden: %@", &[&explain(&e.into())])),
        }
    }

    /// Points every link at a file's new place. The returned changes take that back, and how many links moved.
    pub fn relink(&mut self, old: &Path, new: &Path) -> (Vec<Change>, usize) {
        use super::vault::{canvas, index};
        let settled = |p: &Path| p.parent().and_then(|d| dunce::canonicalize(d).ok()).zip(p.file_name()).map_or_else(|| p.to_path_buf(), |(d, n)| d.join(n));
        let (old, new) = (settled(old), settled(new));
        let before = index::current();
        if before.root.as_os_str().is_empty() || !old.starts_with(&before.root) || !new.starts_with(&before.root) {
            return (Vec::new(), 0);
        }
        let after = index::moved(&old, &new);
        if !after.config.always_update_links {
            return (Vec::new(), 0);
        }
        let mut sources: Vec<PathBuf> = before.backlinks.get(&old).map(|l| l.iter().map(|m| m.source.clone()).collect()).unwrap_or_default();
        sources.sort();
        sources.dedup();
        let (mut changes, mut count) = (Vec::new(), 0);
        let relative = |p: &Path| p.strip_prefix(&before.root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        for source in sources {
            if let Some(buffer) = self.buffers.get_mut(&source) {
                buffer.reload_if_changed();
            }
            let open = self.buffers.get(&source).map(|b| b.text.clone());
            let Some(text) = open.clone().or_else(|| std::fs::read_to_string(&source).ok()) else { continue };
            let rewritten = if source.extension().is_some_and(|e| e.eq_ignore_ascii_case("canvas")) {
                canvas::rewrite_file(&text, &relative(&old), &relative(&new)).map(|t| (t, 1))
            } else {
                index::rewrite_links(&text, &source, &old, &new, &before.resolver, &after)
            };
            let Some((updated, links)) = rewritten else { continue };
            let written = match self.buffers.get_mut(&source) {
                Some(buffer) => buffer.restore(updated).is_ok(),
                None => super::note_buffer::write_atomic(&source, updated.as_bytes()).is_ok(),
            };
            if written {
                count += links;
                changes.push(if open.is_some() { Change::Note { path: source, text } } else { Change::Bytes { path: source, bytes: text.into_bytes() } });
            }
        }
        (changes, count)
    }

    // MARK: Notes changed by other apps

    /// Replaces the version another app wrote with the text here; the other version stays in the undo history.
    pub fn keep_note_mine(&mut self, path: &Path) {
        let Some(buffer) = self.buffers.get_mut(path) else { return };
        let Some(theirs) = buffer.conflict.clone() else { return };
        buffer.keep_mine();
        if let Err(e) = buffer.save() {
            self.last_error = Some(trf("Notiz konnte nicht gesichert werden: %@", &[&e]));
            return;
        }
        self.commit(tr("Deine Version behalten"), "check", vec![Change::Note { path: path.to_path_buf(), text: theirs }]);
    }

    /// Shows the version another app wrote; the text here stays in the undo history.
    pub fn load_note_disk(&mut self, path: &Path) {
        let Some(mine) = self.buffers.get_mut(path).and_then(NoteBuffer::take_disk) else { return };
        self.commit(tr("Neu geladen"), "refresh-cw", vec![Change::Note { path: path.to_path_buf(), text: mine }]);
    }

    /// Makes an earlier version current; what was current stays one undo away.
    pub fn restore_version(&mut self, path: &Path, bytes: Vec<u8>) {
        let message = tr("Version wiederhergestellt");
        if super::markdown::is_markdown(path) || self.buffers.contains_key(path) {
            let buffer = self.buffer(path);
            let before = buffer.text.clone();
            match buffer.restore(String::from_utf8_lossy(&bytes).into_owned()) {
                Ok(()) => self.commit(message, "history", vec![Change::Note { path: path.to_path_buf(), text: before }]),
                Err(e) => self.last_error = Some(trf("Notiz konnte nicht gesichert werden: %@", &[&e])),
            }
            return;
        }
        let result = std::fs::read(path).and_then(|before| super::note_buffer::write_atomic(path, &bytes).map(|_| before));
        match result {
            Ok(before) => self.commit(message, "history", vec![Change::Bytes { path: path.to_path_buf(), bytes: before }]),
            Err(e) => self.last_error = Some(explain(&e.into())),
        }
    }

    // MARK: Sync

    /// Writes everything pending, then holds all further writes while a sync rewrites the workspace.
    pub fn begin_sync(&mut self) {
        self.write_config();
        self.save_state();
        let mut failed = None;
        for b in self.buffers.values_mut() {
            if b.dirty
                && let Err(e) = b.save()
            {
                failed = Some(e);
            }
            b.hold();
        }
        if let Some(e) = failed {
            self.last_error = Some(trf("Notiz konnte nicht gesichert werden: %@", &[&e]));
        }
        self.sync_gate = true;
    }

    /// Ends `begin_sync`: reloads what `changed` touched and writes what was held back.
    pub fn finish_sync(&mut self, changed: &[PathBuf]) {
        self.sync_gate = false;
        let changed: HashSet<&Path> = changed.iter().map(PathBuf::as_path).collect();
        let mut failed = None;
        for (path, b) in &mut self.buffers {
            let touched = changed.contains(path.as_path()) || dunce::canonicalize(path).is_ok_and(|p| changed.contains(p.as_path()));
            if let Err(e) = b.release(touched) {
                failed = Some(e);
            }
        }
        if let Some(e) = failed {
            self.last_error = Some(trf("Notiz konnte nicht gesichert werden: %@", &[&e]));
        }
        let held_config = std::mem::take(&mut self.deferred_config).then(|| self.config.clone()).flatten();
        let held_state = std::mem::take(&mut self.deferred_state).then(|| self.state.clone());
        self.reload_external();
        if let Some(c) = held_config {
            self.config = Some(c);
            self.write_config();
        }
        if let Some(s) = held_state {
            self.state = s;
            self.save_state();
        }
        self.refresh();
        self.changed();
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
        self.is_vault = root.join(".obsidian").is_dir() || self.external_vault().is_some();
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
        self.evaluations = scanner::evaluate(&self.assignments, &self.listings, config, &self.state, self.now, self.root.as_deref(), &self.gone_links);
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
            linked: self.state.linked_files.values().flatten().map(|rel| root.join(rel)).collect(),
        })
    }

    pub fn apply_scan(&mut self, r: ScanResult) {
        if r.generation != self.scan_generation {
            return;
        }
        self.listings = r.listings;
        self.inbox = r.inbox;
        self.gone_links = r.gone;
        self.hide_leaving();
        self.adopt_found_files();
        self.reevaluate();
    }

    /// Older versions counted files by date in shared folders; turns what they counted into links once, so no status changes on update.
    fn adopt_found_files(&mut self) {
        if self.state.links_adopted || self.read_only || self.sync_gate {
            return;
        }
        let Some(config) = self.config.as_ref() else { return };
        for (id, path) in scanner::adoptable(&self.assignments, &self.listings, config) {
            let rel = self.relative_path(&path);
            let list = self.state.linked_files.entry(id).or_default();
            if !list.contains(&rel) {
                list.push(rel);
            }
        }
        self.state.links_adopted = true;
        self.save_state();
    }

    pub fn index_job(&mut self) -> Option<IndexJob> {
        if self.is_indexing {
            return None;
        }
        let root = self.root.clone()?;
        self.is_indexing = true;
        let config = self.config.clone().unwrap_or_default();
        Some(IndexJob { vault: self.external_vault(), index: self.search_index.clone(), root, exclude: config.search_exclude.clone(), config })
    }

    pub fn apply_index(&mut self, count: usize, catalog: Vec<CatalogFile>) {
        if catalog != self.catalog {
            self.catalog = catalog;
            self.index_version += 1;
        }
        self.hide_leaving();
        self.index_count = count;
        self.is_indexing = false;
        self.follow_outside_moves();
        if let Some(adopted) = super::cards::take_notice() {
            if self.undo_steps.is_empty() && self.redo_steps.is_empty() {
                self.cards_known(adopted);
            } else {
                let message = adopted.message();
                let (root, swaps) = (adopted.root, adopted.changes);
                self.notify(message, "calendar-check", None, Some(Box::new(move |_: &mut Library| {
                    super::cards::swap(root.as_deref(), swaps);
                })));
            }
        }
        self.changed();
    }

    // MARK: Derived

    pub fn route(&self) -> &Route {
        &self.route
    }

    pub fn set_route(&mut self, r: Route) {
        if self.route != r {
            if let Route::Doc(left) = self.route.clone() {
                self.follow_heading(&left);
            }
            let from = std::mem::replace(&mut self.route, r);
            self.arrived(&from);
            self.back_routes.push(from);
            if self.back_routes.len() > ROUTE_HISTORY {
                self.back_routes.remove(0);
            }
            self.forward_routes.clear();
        }
        self.changed();
    }

    /// Whether a route can still be shown; closed documents drop out of the history.
    fn route_exists(&self, r: &Route) -> bool {
        match r {
            Route::Doc(p) => self.open_documents.contains(p),
            Route::Notes => self.open_documents.is_empty(),
            _ => true,
        }
    }

    fn reachable(&self, r: &Route) -> bool {
        *r != self.route && self.route_exists(r)
    }

    pub fn can_go_back(&self) -> bool {
        self.tab_can_step(true) || self.back_routes.iter().any(|r| self.reachable(r))
    }

    /// A note reached through the history becomes the shown tab; the note it leaves goes right when it was the right one.
    fn land(&mut self, r: &Route) {
        let Route::Doc(p) = r else { return };
        if self.split_document.as_ref() == Some(p) {
            self.split_document = self.active_document.clone().filter(|a| a != p);
        }
        self.active_document = Some(p.clone());
        self.beside_active = false;
        self.save_documents();
    }

    pub fn can_go_forward(&self) -> bool {
        self.tab_can_step(false) || self.forward_routes.iter().any(|r| self.reachable(r))
    }

    pub fn go_forward(&mut self) {
        if self.step_tab(false) {
            return;
        }
        while let Some(r) = self.forward_routes.pop() {
            if self.reachable(&r) {
                self.land(&r);
                let from = std::mem::replace(&mut self.route, r);
                self.arrived(&from);
                self.back_routes.push(from);
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
            self.selection_stamp += 1;
        }
        self.selection = id;
        self.changed();
    }

    /// Changes with every selection, so a click can tell whether something inside it selected anything.
    pub fn selection_stamp(&self) -> u64 {
        self.selection_stamp
    }

    /// Shows an assignment selected on its module page.
    pub fn reveal(&mut self, id: &str) {
        let Some(module) = self.assignment(id).map(|a| a.module_id.clone()) else { return };
        self.set_route(Route::Module(module));
        self.select(Some(id.to_string()));
    }

    /// A new page lets go of what was selected on the old one and gets back what was left on it.
    fn arrived(&mut self, from: &Route) {
        if *from == self.route {
            return;
        }
        self.inspector_peek = false;
        let back = self.parked_selection.take_if(|(r, _)| *r == self.route).map(|(_, id)| id).filter(|id| self.assignment(id).is_some());
        if let Some(id) = self.selection.take() {
            self.parked_selection = Some((from.clone(), id));
        }
        self.selection = back;
    }

    /// The details the shown page offers.
    pub fn inspector_kind(&self) -> Option<InspectorKind> {
        self.route.inspector_kind()
    }

    /// Whether the details column is shown: kept open for this kind of page, or opened by what is selected.
    pub fn show_inspector(&self) -> bool {
        let Some(kind) = self.inspector_kind() else { return false };
        self.pinned_inspectors.contains(&kind)
            || match kind {
                InspectorKind::Document => self.inspector_peek,
                InspectorKind::Assignments => self.selected().is_some(),
                InspectorKind::Graph => self.graph_selection.is_some(),
            }
    }

    /// Opens or closes the details for this kind of page and remembers it. Closing also lets go of the selection.
    pub fn toggle_inspector(&mut self) {
        let Some(kind) = self.inspector_kind() else { return };
        let open = !self.show_inspector();
        if open {
            self.pinned_inspectors.insert(kind);
        } else {
            self.pinned_inspectors.remove(&kind);
            self.release_inspector(kind);
        }
        let keys: std::collections::BTreeSet<String> = InspectorKind::ALL.into_iter().filter(|k| self.pinned_inspectors.contains(k)).map(|k| k.key().to_string()).collect();
        prefs::update(|p| p.inspector_pinned = keys);
        self.changed();
    }

    /// Opens the document details for this visit only, e.g. to show the versions.
    pub fn peek_inspector(&mut self) {
        self.inspector_peek = true;
        self.changed();
    }

    /// Takes the module page's search to all files.
    pub fn widen_module_search(&mut self) {
        self.search_query = std::mem::take(&mut self.module_query);
        self.set_route(Route::Search);
    }

    /// Lets go of what opened the details, as Esc does. Details the user keeps open stay.
    pub fn dismiss_inspector(&mut self) {
        if let Some(kind) = self.inspector_kind() {
            self.release_inspector(kind);
        }
        self.changed();
    }

    fn release_inspector(&mut self, kind: InspectorKind) {
        match kind {
            InspectorKind::Document => self.inspector_peek = false,
            InspectorKind::Assignments => self.selection = None,
            InspectorKind::Graph => self.graph_selection = None,
        }
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
        self.selection.as_deref() == Some(id) && self.inspector_kind() == Some(InspectorKind::Assignments)
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
                d >= monday && d < next && self.counts(a)
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
        let due_soon = next.is_some_and(|a| a.due - self.now < Duration::days(3));
        next_action::choose(overdue, unconfirmed, next, due_soon, self.inbox.len())
    }

    pub fn next_deadline(&self) -> Option<&Assignment> {
        let upcoming = self.upcoming();
        upcoming.iter().find(|a| !a.optional).or(upcoming.first()).copied()
    }

    /// The lecture week a deadline falls into, so every view sorts it by the same date.
    pub fn due_week(&self, a: &Assignment) -> Option<u32> {
        let date = day::local_date(a.due);
        self.weeks().iter().find(|w| w.monday <= date && date < w.monday + Days::new(7)).map(|w| w.index).or(a.week)
    }

    /// The module's next open entry that takes files, as the target for files dropped on the module.
    pub fn next_in_module(&self, module_id: &str) -> Option<String> {
        self.upcoming().into_iter().find(|a| a.tracks_files() && self.module(a).is_some_and(|m| m.id == module_id)).map(|a| a.id.clone())
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

    pub fn module_containing(&self, path: &Path) -> Option<&Module> {
        let rel = super::text::slash_path(&self.below_root(path)?);
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
        if self.config.as_ref().is_some_and(|c| !c.create_folders) {
            out.retain(|t| t.path.is_dir());
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
        if let (Some(vault), Some(config)) = (self.external_vault(), self.config.as_ref()) {
            for module in &config.modules {
                if let Some(folder) = &module.notes_folder {
                    add(vault.join(folder), module, None);
                }
            }
        }
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

    fn typed_name(title: &str) -> String {
        title.trim().chars().filter(|c| !"*?\"<>|".contains(*c)).map(|c| if "\\/:".contains(c) { '-' } else { c }).collect::<String>().trim_end_matches(['.', ' ']).to_string()
    }

    /// The file name of something new: what you call it. A convention is applied only when asked for, see `convention_plan`.
    pub fn note_name(&self, title: &str, ext: &str) -> String {
        format!("{}.{ext}", Self::typed_name(title))
    }

    /// Writes an empty note with the title as heading; returns its path.
    pub fn create_note(&mut self, title: &str, folder: &Path, ext: &str) -> Option<PathBuf> {
        let result: Result<PathBuf> = (|| {
            std::fs::create_dir_all(folder)?;
            let path = unique_path(&folder.join(self.note_name(title, ext)));
            std::fs::write(&path, if ext == "md" { self.new_note_text(title) } else { String::new() })?;
            Ok(path)
        })();
        match result {
            Ok(path) => {
                if ext == "md" {
                    self.mark_made_here(&path);
                }
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

    fn mark_made_here(&mut self, path: &Path) {
        let settled = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let key = settled.to_string_lossy().to_string();
        super::prefs::update(|p| {
            p.heading_titled.insert(key);
        });
        self.fresh_notes.insert(settled);
    }

    /// Whether this note was just made here; asks only once.
    pub fn take_fresh_note(&mut self, path: &Path) -> bool {
        self.fresh_notes.remove(path)
    }

    /// Gives a note made here the name of its first heading.
    pub fn follow_heading(&mut self, path: &Path) {
        if !heading_titled(path) || self.is_leaving(path) {
            return;
        }
        let text = match self.buffers.get_mut(path) {
            Some(buffer) => {
                if buffer.conflict.is_some() || (buffer.dirty && buffer.save().is_err()) {
                    return;
                }
                buffer.text.clone()
            }
            None => std::fs::read_to_string(path).unwrap_or_default(),
        };
        let Some(title) = super::markdown::first_heading(&text) else { return };
        if self.has_convention_name(path) {
            return;
        }
        let name = format!("{}.md", super::text::safe_file_stem(&title));
        if name == name_of(path) || name == ".md" {
            return;
        }
        let wanted = path.with_file_name(&name);
        if wanted.exists() && rename_target(path, &name) != wanted {
            let clash = (path.to_path_buf(), name.clone());
            if self.heading_clash.as_ref() != Some(&clash) {
                self.notify(trf("Name bleibt: „%@“ gibt es hier schon", &[&name]), "triangle-alert", None, None);
                self.heading_clash = Some(clash);
            }
            return;
        }
        self.rename_file(path, &name);
    }

    /// What a new note starts with: the vault's `Mitschrift` template if there is one, otherwise its title.
    fn new_note_text(&self, title: &str) -> String {
        let template = self.vault_root().and_then(|root| {
            let settings = super::vault::config::VaultConfig::read(&root);
            let folder = settings.templates.folder.trim_matches('/');
            if !settings.is_vault || folder.is_empty() {
                return None;
            }
            let raw = ["Mitschrift", "Notiz", "Note"].iter().find_map(|n| std::fs::read_to_string(root.join(folder).join(format!("{n}.md"))).ok())?;
            Some(super::vault::moment::fill_template(&raw, title.trim(), Local::now(), &settings.templates.date_format, &settings.templates.time_format, super::i18n::is_english()))
        });
        template.unwrap_or_else(|| format!("# {}\n\n", title.trim()))
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
        for f in self.catalog.iter().filter(|f| f.module_id.as_deref() == Some(module.id.as_str()) && f.place == place) {
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
        self.below_root(path).map_or_else(|| path.to_string_lossy().to_string(), |rest| super::text::slash_path(&rest))
    }

    /// `path` relative to the workspace, spelled from the root as opened or from the resolved root.
    fn below_root(&self, path: &Path) -> Option<PathBuf> {
        let (root, settled) = (self.root.as_ref()?, self.settled_root.as_ref()?);
        path.strip_prefix(root).or_else(|_| path.strip_prefix(settled)).map(Path::to_path_buf).ok()
    }

    /// Where filing into an assignment puts the file: its folder, or when Schrumbi may not create folders,
    /// the nearest existing folder inside the module; `None` when not even the module folder exists.
    pub fn filing_folder(&self, a: &Assignment) -> Option<PathBuf> {
        let (Some(root), Some(c)) = (self.root.as_ref(), self.config.as_ref()) else { return Some(a.folder.clone()) };
        if c.create_folders || a.folder.is_dir() {
            return Some(a.folder.clone());
        }
        let module = root.join(&c.module(&a.module_id)?.folder);
        a.folder.ancestors().take_while(|p| p.starts_with(&module)).find(|p| p.is_dir()).map(Path::to_path_buf)
    }

    // MARK: Feedback

    pub fn notify(&mut self, message: String, icon: &'static str, action_title: Option<String>, undo: Option<Undo>) {
        self.toast_counter += 1;
        self.toast_hovered = false;
        self.toast = Some(Toast { id: self.toast_counter, message, icon, action_title: action_title.unwrap_or_else(|| tr("Rückgängig")), undo });
        self.changed();
    }

    /// Runs the action offered by the current toast.
    pub fn perform_toast_action(&mut self) {
        self.toast_hovered = false;
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
        let undo = self.undo_latest();
        self.notify(message, icon, None, Some(undo));
    }

    /// Takes back the latest step, but only while it still is the latest; a toast must not undo a later edit.
    pub fn undo_latest(&self) -> Undo {
        let (count, label) = (self.undo_steps.len(), self.undo_steps.last().map(|s| s.label.clone()));
        Box::new(move |lib: &mut Library| {
            if lib.undo_steps.len() == count && lib.undo_steps.last().map(|s| s.label.clone()) == label {
                lib.undo();
            }
        })
    }

    /// Records an edit made by continuous input; edits of the same `kind` in quick succession become one step.
    pub fn record_edit(&mut self, kind: &str, label: String, before: Change) {
        let now = std::time::Instant::now();
        if self.coalescing.as_ref().is_some_and(|(k, at)| k == kind && now.duration_since(*at) < COALESCE) && !self.undo_steps.is_empty() {
            self.redo_steps.clear();
        } else {
            self.record(label, vec![before]);
        }
        self.coalescing = Some((kind.to_string(), now));
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
        if !self.leaving.is_empty() {
            return;
        }
        let Some(step) = self.undo_steps.pop() else { return };
        let label = step.label.clone();
        let (inverse, failed) = self.apply_step(step);
        let changed = !inverse.changes.is_empty();
        if changed {
            self.redo_steps.push(inverse);
        }
        self.coalescing = None;
        if !failed.is_empty() {
            self.undo_steps.push(Step { label, changes: failed });
            return;
        }
        if !changed {
            return;
        }
        self.notify(trf("Rückgängig: %@", &[&label]), "undo-2", Some(tr("Wiederholen")), Some(Box::new(|lib: &mut Library| lib.redo())));
    }

    /// Repeats the step taken back last and offers to take it back again.
    pub fn redo(&mut self) {
        if !self.leaving.is_empty() {
            return;
        }
        let Some(step) = self.redo_steps.pop() else { return };
        let label = step.label.clone();
        let (inverse, failed) = self.apply_step(step);
        let changed = !inverse.changes.is_empty();
        if changed {
            self.undo_steps.push(inverse);
        }
        self.coalescing = None;
        if !failed.is_empty() {
            self.redo_steps.push(Step { label, changes: failed });
            return;
        }
        if !changed {
            return;
        }
        let undo = self.undo_latest();
        self.notify(trf("Wiederholt: %@", &[&label]), "redo-2", None, Some(undo));
    }

    /// Applies a step; returns its reverse and the changes that failed, so they can be tried again.
    fn apply_step(&mut self, step: Step) -> (Step, Vec<Change>) {
        self.rows_moved = true;
        let (mut inverse, mut failed) = (Vec::new(), Vec::new());
        for change in step.changes.into_iter().rev() {
            if !matches!(change, Change::State(_) | Change::Config(_) | Change::Highlights { .. } | Change::Cards { .. }) {
                self.notes_touched = true;
            }
            match self.apply_change(change.clone()) {
                Some(done) => inverse.push(done),
                None => failed.push(change),
            }
        }
        inverse.retain(|c| !matches!(c, Change::Cards { swaps, .. } if swaps.is_empty()));
        inverse.reverse();
        failed.reverse();
        self.refresh();
        (Step { label: step.label, changes: inverse }, failed)
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
                    self.path_moved(&from, &target);
                    Change::Move { from: target, to: from }
                }
                Change::Trash(path) => trash_with_copy(&path)?,
                Change::Restore { path, backup } => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    move_path(&backup, &unique_path(&path))?;
                    Change::Trash(path)
                }
                Change::Replaced { path, backup } => {
                    let current = stash(&path)?;
                    move_path(&backup, &path)?;
                    Change::Replaced { path, backup: current }
                }
                Change::Highlights { key, list } => {
                    let before = self.store_highlights(&key, list);
                    Change::Highlights { key, list: before }
                }
                Change::Bytes { path, bytes } => {
                    let before = std::fs::read(&path)?;
                    super::note_buffer::write_atomic(&path, &bytes)?;
                    Change::Bytes { path, bytes: before }
                }
                Change::Note { path, text } => {
                    let buffer = self.buffer(&path);
                    let before = buffer.text.clone();
                    buffer.restore(text)?;
                    Change::Note { path, text: before }
                }
                Change::Cards { root, swaps } => {
                    let back = super::cards::swap(root.as_deref(), swaps).unwrap_or_else(|| {
                        self.last_error = Some(tr("Die Karten gehören zu einem anderen Ordner und bleiben, wie sie sind."));
                        Vec::new()
                    });
                    if back.is_empty() && self.last_error.is_none() {
                        self.last_error = Some(tr("Die Karten wurden seither geändert und bleiben, wie sie sind."));
                    }
                    Change::Cards { root, swaps: back }
                }
            })
        })();
        match result {
            Ok(inverse) => Some(inverse),
            Err(e) => {
                self.last_error = Some(trf("Rückgängig fehlgeschlagen: %@", &[&explain(&e)]));
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
        let label = status.or_else(|| self.assignment(id).map(|a| self.status(a))).map_or_else(|| tr("Automatisch"), Status::label);
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

    /// Whether `p` lies in the workspace, spelled as given or resolved.
    pub(crate) fn inside(&self, p: &Path) -> bool {
        self.below_root(p).is_some()
    }

    /// Moves files inside the workspace, copies files from outside. Returns the new path.
    pub fn file(&mut self, path: &Path, folder: &Path, name: &str, announce: bool) -> Option<PathBuf> {
        self.file_for(path, folder, name, None, announce)
    }

    /// Like `file`; a file filed for an assignment is linked to it, so it counts wherever it goes.
    pub fn file_for(&mut self, path: &Path, folder: &Path, name: &str, assignment: Option<&str>, announce: bool) -> Option<PathBuf> {
        let inside = self.inside(path);
        let link_to = assignment.and_then(|id| self.assignment(id)).filter(|a| a.tracks_files()).map(|a| a.id.clone());
        let result: Result<PathBuf> = (|| {
            std::fs::create_dir_all(folder)?;
            let wanted = folder.join(name);
            if wanted == path || settled_path(&wanted) == settled_path(path) {
                return Ok(wanted);
            }
            let target = unique_path(&wanted);
            if inside { move_path(path, &target)? } else { copy_path(path, &target)? }
            Ok(target)
        })();
        match result {
            Ok(target) => {
                let mut links = Vec::new();
                if inside {
                    self.path_moved(path, &target);
                    links = self.relink(path, &target).0;
                }
                if let Some(id) = link_to {
                    let before = self.state_snapshot();
                    let rel = self.relative_path(&target);
                    let list = self.state.linked_files.entry(id).or_default();
                    if !list.contains(&rel) {
                        list.push(rel);
                        self.save_state();
                        links.push(before);
                    }
                }
                self.refresh();
                self.sync_soon = true;
                if announce {
                    let mut changes = vec![filing_change(path, &target, inside)];
                    changes.extend(links);
                    self.commit(trf("Abgelegt: %@", &[&name_of(&target)]), "inbox", changes);
                } else {
                    self.pending_link_changes.extend(links);
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
    pub fn file_batch(&mut self, items: Vec<(PathBuf, PathBuf, String)>, assignment: Option<&str>) {
        let mut done: Vec<Change> = Vec::new();
        let mut last = None;
        for (path, folder, name) in items {
            let inside = self.inside(&path);
            if let Some(t) = self.file_for(&path, &folder, &name, assignment, false) {
                done.push(filing_change(&path, &t, inside));
                last = Some(t);
            }
        }
        let filed = done.len();
        done.append(&mut self.pending_link_changes);
        let Some(last) = last else { return };
        let message = if filed == 1 { trf("Abgelegt: %@", &[&name_of(&last)]) } else { trf("%lld Dateien abgelegt", &[&filed]) };
        self.commit(message, "inbox", done);
    }

    /// Renames a file in an assignment folder so it counts as task description (or back as own work).
    pub fn set_role(&mut self, role: FileRole, path: &Path, assignment_id: &str) {
        let Some(a) = self.assignment(assignment_id).cloned() else { return };
        if !self.config.as_ref().is_some_and(StudyConfig::naming_convention) {
            let author = self.config.as_ref().map(|c| c.author.clone()).unwrap_or_default();
            let new = plain_role_name(&name_of(path), role, &author);
            if heading_titled(path) {
                self.rename_titled(path, &new, super::naming::stem(&new), false);
            } else {
                self.rename_file(path, &new);
            }
            return;
        }
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
        if heading_titled(path) {
            self.rename_titled(path, &new, &title.replace('-', " "), true);
        } else {
            self.rename_file(path, &new);
        }
    }

    fn parse_brief(&self, file_name: &str, module_id: &str) -> Option<Parsed> {
        self.brief_naming(module_id)?.parse(file_name)
    }

    pub(crate) fn brief_naming(&self, module_id: &str) -> Option<Naming> {
        let c = self.config.as_ref()?;
        let m = c.module(module_id)?;
        Some(Naming::with_author(m, c.semester(&m.semester), BRIEF_TAG, "", CoauthorStyle::Initial))
    }

    /// Renames a file to a name picked by hand; the note's heading stays as it is.
    pub fn rename_keeping_heading(&mut self, file: &Path, name: &str) {
        self.rename_file(file, name);
    }

    /// Records cards taken as known and offers to take that back.
    pub fn cards_known(&mut self, adopted: super::cards::Adopted) {
        let message = adopted.message();
        self.record(adopted.label(), vec![Change::Cards { root: adopted.root, swaps: adopted.changes }]);
        let undo = self.undo_latest();
        self.notify(message, "calendar-check", None, Some(undo));
    }

    /// Records a change to cards' progress; `back` are the swaps that take it back. Without any, it only tells.
    pub fn cards_changed(&mut self, message: String, icon: &'static str, back: Vec<super::cards::Swap>) {
        if back.is_empty() {
            self.notify(message, icon, None, None);
            return;
        }
        self.commit(message, icon, vec![Change::Cards { root: super::cards::store_root(), swaps: back }]);
    }

    /// Reports an import of flashcards; taking it back removes the new files and what was taken as known.
    pub fn cards_imported(&mut self, created: Vec<PathBuf>, added: usize, duplicates: usize, spread: Option<super::cards::Adopted>) {
        self.refresh();
        let message = match (added, duplicates) {
            (1, 0) => tr("1 Karte importiert"),
            (n, 0) => trf("%@ Karten importiert", &[&n.to_string()]),
            (n, d) => trf("%@ Karten importiert, %@ waren schon da", &[&n.to_string(), &d.to_string()]),
        };
        let message = match &spread {
            Some(adopted) => format!("{message} · {}", adopted.spread_words()),
            None => message,
        };
        let mut changes: Vec<Change> = created.into_iter().map(Change::Trash).collect();
        if let Some(adopted) = spread {
            changes.push(Change::Cards { root: adopted.root, swaps: adopted.changes });
        }
        self.commit(message, "layers", changes);
    }

    /// Remembers the open tabs under a name.
    pub fn save_layout(&mut self, name: &str) {
        let Some(root) = self.root.clone() else { return };
        let tabs: Vec<String> = self.open_documents.iter().map(|p| p.to_string_lossy().to_string()).collect();
        prefs::update(|p| {
            p.layouts.entry(root.to_string_lossy().to_string()).or_default().insert(name.to_string(), tabs);
        });
        self.notify(trf("Arbeitsbereich „%@“ gesichert", &[&name]), "layout-grid", None, None);
    }

    /// The saved sets of tabs of this workspace.
    pub fn layouts(&self) -> Vec<String> {
        self.root.as_ref().and_then(|r| prefs::get().layouts.get(&r.to_string_lossy().to_string()).map(|m| m.keys().cloned().collect())).unwrap_or_default()
    }

    /// Opens a saved set of tabs instead of the open ones.
    pub fn open_layout(&mut self, name: &str) {
        let Some(root) = self.root.clone() else { return };
        let Some(tabs) = prefs::get().layouts.get(&root.to_string_lossy().to_string()).and_then(|m| m.get(name).cloned()) else { return };
        self.close_all_documents();
        for tab in tabs.iter().map(PathBuf::from).filter(|p| p.exists()) {
            self.open_file(&tab);
        }
    }

    /// Makes a folder at any depth a module, or takes that away again. Its files stay where they are.
    pub fn toggle_module_folder(&mut self, folder: &str) {
        let Some(mut config) = self.config.clone() else { return };
        if let Some(at) = config.modules.iter().position(|m| m.folder == folder) {
            let removed = config.modules.remove(at);
            let name = removed.name.clone();
            self.save_config(config);
            self.notify(trf("%@ ist kein Modul mehr", &[&name]), "library", Some(tr("Rückgängig")), Some(Box::new(move |l: &mut Library| {
                if let Some(mut config) = l.config.clone() {
                    config.modules.push(removed.clone());
                    l.save_config(config);
                }
            })));
        } else {
            let semester = self.now_semester_id().unwrap_or_default();
            let mut taken: Vec<String> = config.modules.iter().map(|m| m.code.clone()).collect();
            let at = config.modules.len();
            let module = super::workspace::module_for_folder(folder, &semester, at, &mut taken);
            let (name, id) = (module.name.clone(), module.id.clone());
            config.modules.push(module);
            self.save_config(config);
            self.notify(trf("%@ ist jetzt ein Modul", &[&name]), "library", Some(tr("Öffnen")), Some(Box::new(move |l: &mut Library| l.set_route(Route::Module(id.clone())))));
        }
    }

    fn now_semester_id(&self) -> Option<String> {
        let config = self.config.as_ref()?;
        config.current_semester(self.now).or(config.semesters.last()).map(|s| s.id.clone())
    }

    /// Gives a file the name typed. A note named after its heading keeps that heading; a convention name only has to say the same title.
    pub fn rename(&mut self, file: &Path, name: &str) {
        if !heading_titled(file) {
            self.rename_file(file, name);
            return;
        }
        match self.convention_title(file, name) {
            Some(title) => self.rename_titled(file, name, &title, true),
            None => self.rename_titled(file, name, super::naming::stem(name), false),
        }
    }

    /// The title a convention name says, for a file of this module; also as a task sheet.
    fn convention_title(&self, file: &Path, name: &str) -> Option<String> {
        let module = self.deadlines_of(file).into_iter().next().map(|a| a.module_id).or_else(|| self.module_containing(file).map(|m| m.id.clone()));
        let own = module.as_deref().and_then(|m| self.naming(m)).and_then(|n| n.parse(name));
        let parsed = own.or_else(|| self.namings().iter().find_map(|n| n.parse(name))).or_else(|| module.and_then(|m| self.parse_brief(name, &m)));
        parsed.map(|p| p.title.replace('-', " "))
    }

    /// Whether the file already carries a convention name of its module, also as a task sheet.
    fn has_convention_name(&self, file: &Path) -> bool {
        self.convention_title(file, &name_of(file)).is_some()
    }

    /// What renaming `file` by its module's convention needs, or `None` when no convention applies. Cheap; `ConventionPlan::name` reads the file.
    pub fn convention_plan(&self, file: &Path) -> Option<ConventionPlan> {
        let (deadline, module_id) = self.convention_target(file)?;
        let naming = self.naming(&module_id)?;
        let module = self.config.as_ref()?.module(&module_id)?;
        let name = name_of(file);
        let brief = self.parse_brief(&name, &module_id);
        let parsed = brief.clone().or_else(|| self.namings().iter().find_map(|n| n.parse(&name)));
        let heading = self.buffers.get(file).and_then(|b| super::markdown::first_heading(&b.text));
        Some(ConventionPlan {
            naming,
            namings: self.namings(),
            ignore: self.title_ignore_words(),
            weeks: self.weeks(),
            module: module.code.clone(),
            deadline: deadline_label(&deadline),
            due: deadline.due,
            week: deadline.week,
            iso_week: deadline.iso_week,
            brief: brief.is_some().then(|| self.brief_naming(&module_id)).flatten(),
            parsed,
            heading,
        })
    }

    /// Whether naming a file by its module's convention is offered, and if not yet possible why; cheap enough for opening a menu.
    pub fn convention_offer(&self, file: &Path) -> ConventionOffer {
        if !self.config.as_ref().is_some_and(StudyConfig::naming_convention) {
            return ConventionOffer::Hidden;
        }
        let name = name_of(file);
        let target = self.convention_target(file);
        let Some(module_id) = target.as_ref().map(|(_, m)| m.clone()).or_else(|| self.module_containing(file).map(|m| m.id.clone())) else { return ConventionOffer::Hidden };
        let Some(naming) = self.naming(&module_id) else { return ConventionOffer::Hidden };
        if naming.is_valid(&name) {
            return ConventionOffer::Hidden;
        }
        if target.is_some() {
            ConventionOffer::Ready
        } else if self.assignments.iter().any(|a| a.module_id == module_id && a.upload && self.submission_holds(a, file)) {
            ConventionOffer::NeedsDeadline
        } else {
            ConventionOffer::Hidden
        }
    }

    fn convention_target(&self, file: &Path) -> Option<(Assignment, String)> {
        self.config.as_ref().filter(|c| c.naming_convention())?;
        let linked = self.deadlines_of(file);
        let deadline = linked.iter().find(|a| a.upload).or(linked.first()).cloned().or_else(|| self.fitting_deadline(file))?;
        let module_id = deadline.module_id.clone();
        Some((deadline, module_id))
    }

    /// Renames a note named after its heading and brings the heading along; a convention name only has to say the same title.
    fn rename_titled(&mut self, file: &Path, name: &str, title: &str, convention: bool) {
        self.rename_file(file, name);
        let Some(target) = self.last_renamed.clone().filter(|t| t.file_name().is_some_and(|n| n.to_string_lossy() == name)) else { return };
        self.edit_note(&target, "title", tr("Überschrift angepasst"), |text| {
            let same = super::markdown::first_heading(text).is_some_and(|h| if convention { Naming::clean(&h) == Naming::clean(title) } else { h == title });
            if same { text.to_string() } else { super::markdown::with_first_heading(text, title) }
        });
    }

    /// Whether the folder of `a` holds `path`, directly or, for a folder of its own that looks into subfolders, below it.
    fn submission_holds(&self, a: &Assignment, path: &Path) -> bool {
        let (Some(raw), Some(root), Some(settled_root)) = (path.parent().map(Path::to_path_buf), self.root.as_ref(), self.settled_root.as_ref()) else { return false };
        let settled = dunce::canonicalize(&raw).unwrap_or_else(|_| raw.clone());
        let other = a.folder.strip_prefix(root).map_or_else(|_| a.folder.clone(), |rest| settled_root.join(rest));
        [&a.folder, &other].iter().any(|f| [&raw, &settled].iter().any(|d| *d == *f || (a.recursive && a.owns_folder() && d.starts_with(f))))
    }

    /// The upload whose folder holds `path` and whose pattern fits its name, the next one due first.
    fn fitting_deadline(&self, path: &Path) -> Option<Assignment> {
        let name = name_of(path);
        let fitting: Vec<&Assignment> = self.assignments.iter().filter(|a| a.upload && a.tracks_files() && scanner::matches(&name, &a.file_pattern) && self.submission_holds(a, path)).collect();
        let today = self.today();
        fitting.iter().filter(|a| day::local_date(a.due) >= today).min_by_key(|a| a.due).or_else(|| fitting.iter().max_by_key(|a| a.due)).map(|a| (*a).clone())
    }

    fn rename_file(&mut self, file: &Path, name: &str) {
        if name == name_of(file) {
            return;
        }
        let target = rename_target(file, name);
        match move_path(file, &target) {
            Ok(()) => {
                self.folder_moved(file, &target);
                self.path_moved(file, &target);
                let (mut changes, links) = self.relink(file, &target);
                self.refresh();
                changes.insert(0, Change::Move { from: target.clone(), to: file.to_path_buf() });
                let message = match links {
                    0 => trf("Umbenannt: %@", &[&name_of(&target)]),
                    1 => tr("Umbenannt, 1 Link angepasst"),
                    n => trf("Umbenannt, %@ Links angepasst", &[&n.to_string()]),
                };
                self.commit(message, "pencil", changes);
                self.last_renamed = Some(target);
            }
            Err(e) => self.last_error = Some(trf("Umbenennen fehlgeschlagen: %@", &[&explain(&e.into())])),
        }
    }

    /// Modules keep following their folder when it, or a folder above it, gets a new name.
    fn folder_moved(&mut self, old: &Path, new: &Path) {
        let Some(mut config) = self.config.clone() else { return };
        let (Some(old), Some(new)) = (self.below_root(old), self.below_root(new)) else { return };
        let (old, new) = (old.to_string_lossy().replace('\\', "/"), new.to_string_lossy().replace('\\', "/"));
        let mut changed = false;
        for m in &mut config.modules {
            if m.folder == old {
                m.folder = new.clone();
                changed = true;
            } else if let Some(rest) = m.folder.strip_prefix(&format!("{old}/")) {
                m.folder = format!("{new}/{rest}");
                changed = true;
            }
        }
        if changed {
            self.save_config(config);
        }
    }

    /// Notices when another app renamed or moved an open file: same size and time means the same file.
    /// Returns whether a tab followed. Files that are simply gone stay as they are, shown as missing.
    pub fn follow_outside_moves(&mut self) -> bool {
        let stamp = |p: &Path| std::fs::metadata(p).ok().and_then(|m| Some((m.len(), m.modified().ok()?)));
        let open: Vec<PathBuf> = self.open_documents.iter().chain(self.split_document.iter()).cloned().collect();
        self.document_stamps.retain(|p, _| open.contains(p));
        let mut gone = Vec::new();
        for path in &open {
            match stamp(path) {
                Some(now) => {
                    self.document_stamps.insert(path.clone(), now);
                }
                None => gone.extend(self.document_stamps.remove(path).map(|s| (path.clone(), s))),
            }
        }
        if gone.is_empty() {
            return false;
        }
        let roots: Vec<PathBuf> = self.root.iter().cloned().chain(self.external_vault()).collect();
        let mut files: Vec<PathBuf> = Vec::new();
        for root in &roots {
            let root = dunce::canonicalize(root).unwrap_or_else(|_| root.clone());
            files.extend(walkdir::WalkDir::new(&root).min_depth(1).into_iter().filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.')).filter_map(Result::ok).filter(|e| e.file_type().is_file()).map(|e| e.into_path()));
        }
        let mut followed = false;
        for (from, known) in gone {
            let matches: Vec<&PathBuf> = files.iter().filter(|f| !open.contains(f) && stamp(f) == Some(known)).collect();
            if let [to] = matches[..] {
                let to = to.clone();
                self.document_stamps.insert(to.clone(), known);
                self.path_moved(&from, &to);
                followed = true;
            }
        }
        followed
    }

    /// Keeps tabs, open notes, links and remembered positions on a file that moved.
    fn path_moved(&mut self, from: &Path, to: &Path) {
        let (old, new) = (self.relative_path(from), self.relative_path(to));
        let settled = |p: &Path| p.parent().and_then(|d| dunce::canonicalize(d).ok()).zip(p.file_name()).map_or_else(|| p.to_path_buf(), |(d, n)| d.join(n));
        let (from, to) = (settled(from), settled(to));
        let swap = |p: &mut PathBuf| {
            if *p == from {
                *p = to.clone();
            }
        };
        self.open_documents.iter_mut().for_each(swap);
        self.closed_documents.iter_mut().for_each(|(p, _)| swap(p));
        if self.preview_document.as_ref() == Some(&from) {
            self.preview_document = None;
        }
        if let Some(trail) = self.tab_trails.remove(&from) {
            self.tab_trails.insert(to.clone(), trail);
        }
        self.active_document.iter_mut().for_each(swap);
        self.split_document.iter_mut().for_each(swap);
        let (old_key, new_key) = (from.to_string_lossy().to_string(), to.to_string_lossy().to_string());
        if super::prefs::get().heading_titled.contains(&old_key) {
            super::prefs::update(|p| {
                p.heading_titled.remove(&old_key);
                p.heading_titled.insert(new_key);
            });
        }
        if self.note_windows.remove(&from) {
            self.note_windows.insert(to.clone());
        }
        if let Route::Doc(p) = &mut self.route {
            swap(p);
        }
        if let Some(mut buffer) = self.buffers.remove(&from) {
            buffer.moved_to(&to);
            self.buffers.insert(to.clone(), buffer);
        }
        let mut relinked = false;
        let inside = format!("{old}/");
        for rel in self.state.linked_files.values_mut().flatten() {
            if *rel == old {
                rel.clone_from(&new);
            } else if let Some(rest) = rel.strip_prefix(&inside) {
                *rel = format!("{new}/{rest}");
            } else {
                continue;
            }
            relinked = true;
        }
        if relinked {
            self.save_state();
        }
        let (key, moved) = (from.to_string_lossy().to_string(), to.to_string_lossy().to_string());
        if prefs::get().pdf_positions.contains_key(&key) {
            prefs::update(|p| {
                if let Some(position) = p.pdf_positions.remove(&key) {
                    p.pdf_positions.insert(moved, position);
                }
            });
        }
        self.save_documents();
    }

    /// Drops files on their way to the trash from every list at once; scans until `trashed` keep them out too.
    /// Returns the paths that may go; a note with unsaved edits that also changed elsewhere stays.
    pub fn trashing(&mut self, paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut going = Vec::new();
        for path in paths {
            let settled = settled_path(path);
            let within = |p: &Path| p.starts_with(path) || p.starts_with(&settled);
            if let Some((stuck, _)) = self.buffers.iter().find(|(p, b)| b.dirty && b.conflict.is_some() && within(p)) {
                self.last_error = Some(trf("„%@“ wurde auch in einer anderen App geändert. Entscheide zuerst, welche Version bleibt.", &[&name_of(stuck)]));
                continue;
            }
            for (p, buffer) in self.buffers.iter_mut() {
                if buffer.dirty && within(p) {
                    let _ = buffer.save();
                }
            }
            super::vault::index::hide(&[path.clone(), settled.clone()]);
            self.leaving.insert(path.clone());
            self.leaving.insert(settled.clone());
            self.leaving_folder |= path.is_dir();
            self.leaving_forms.insert(path.clone(), settled);
            going.push(path.clone());
        }
        let open: Vec<PathBuf> = self.open_documents.iter().filter(|d| self.is_leaving(d)).cloned().collect();
        for d in open {
            self.close_tab(&d);
        }
        self.hide_leaving();
        self.reevaluate();
        going
    }

    fn is_leaving(&self, path: &Path) -> bool {
        !self.leaving.is_empty() && if self.leaving_folder { path.ancestors().any(|a| self.leaving.contains(a)) } else { self.leaving.contains(path) }
    }

    fn hide_leaving(&mut self) {
        if self.leaving.is_empty() {
            return;
        }
        let leaving = std::mem::take(&mut self.leaving);
        let folders = self.leaving_folder;
        let gone = |p: &Path| if folders { p.ancestors().any(|a| leaving.contains(a)) } else { leaving.contains(p) };
        self.inbox.retain(|p| !gone(p));
        for files in self.listings.values_mut() {
            files.retain(|f| !gone(&f.path));
        }
        let count = self.catalog.len();
        self.catalog.retain(|c| !gone(&c.file.path));
        if self.catalog.len() != count {
            self.index_version += 1;
        }
        self.leaving = leaving;
    }

    /// Records what `trash_files` did as one undoable step.
    pub fn trashed(&mut self, results: Vec<(PathBuf, Result<Change>)>) {
        let mut done: Vec<(PathBuf, Change)> = Vec::new();
        let (mut gone, mut kept) = (Vec::new(), Vec::new());
        for (path, result) in results {
            let settled = self.leaving_forms.remove(&path).unwrap_or_else(|| settled_path(&path));
            let both = [settled, path.clone()];
            for p in &both {
                self.leaving.remove(p);
            }
            match result {
                Ok(change) => {
                    gone.extend(both);
                    done.push((path, change));
                }
                Err(_) => {
                    kept.extend(both);
                    self.last_error = Some(tr("Konnte nicht in den Papierkorb gelegt werden. Prüfe, ob die Datei noch existiert und nicht gerade in einer anderen App offen ist."));
                }
            }
        }
        self.leaving_folder &= !self.leaving.is_empty();
        super::vault::index::trash_ended(&gone, &kept);
        let trashed: Vec<PathBuf> = done.iter().map(|(p, _)| p.clone()).collect();
        let unlinked = self.drop_links_to(&trashed);
        self.refresh();
        let message = match &done[..] {
            [] => return,
            [(path, _)] => trf("In den Papierkorb: %@", &[&name_of(path)]),
            _ => trf("%lld Dateien in den Papierkorb", &[&done.len()]),
        };
        self.commit(message, "trash-2", done.into_iter().map(|(_, c)| c).chain(unlinked).collect());
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
    /// Takes over what a subscribed calendar says, quietly when nothing changed.
    pub fn refresh_events(&mut self, text: &str) -> usize {
        let events = ics::parse(text);
        let modules: Vec<Module> = self.semester_modules().into_iter().cloned().collect();
        let now = Utc::now();
        let items: Vec<(CalendarEvent, String, Kind)> = events
            .into_iter()
            .filter_map(|event| {
                let known = self.state.manual.iter().find(|m| m.external_id.as_deref() == Some(event.uid.as_str()));
                if known.is_some_and(|m| m.title == event.title() && m.due == event.due) || (known.is_none() && event.due < now) {
                    return None;
                }
                let module = known.map(|m| m.module.clone()).or_else(|| ics::module_for(&event.course, &modules).map(|m| m.id.clone()))?;
                let kind = known.map_or_else(|| event.guessed_kind(), |m| m.kind);
                Some((event, module, kind))
            })
            .collect();
        let count = items.len();
        if count > 0 {
            self.import_events(items);
        }
        count
    }

    pub fn import_events(&mut self, items: Vec<(CalendarEvent, String, Kind)>) -> (usize, usize) {
        let (Some(_), Some(config)) = (self.root.as_ref(), self.config.clone()) else { return (0, 0) };
        let (mut added, mut updated) = (0, 0);
        let before = self.state_snapshot();
        for (event, module_id, kind) in items {
            let Some(module) = config.module(&module_id) else { continue };
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
        let Some(label) = self.assignment(id).filter(|a| a.tracks_files()).map(deadline_label) else { return };
        let inside: Vec<&PathBuf> = paths.iter().filter(|p| self.inside(p)).collect();
        if inside.is_empty() {
            self.last_error = Some(tr("Nur Dateien im Studienordner können zugeordnet werden."));
            return;
        }
        let rels: Vec<String> = inside.iter().map(|p| self.relative_path(p)).collect();
        let before = self.state_snapshot();
        let list = self.state.linked_files.entry(id.into()).or_default();
        for r in &rels {
            if !list.contains(r) {
                list.push(r.clone());
            }
        }
        for p in inside {
            self.gone_links.remove(p);
        }
        self.save_state();
        self.refresh();
        let message = if let [one] = &paths[..] { trf("„%@“ gehört jetzt zu %@", &[&name_of(one), &label]) } else { trf("%lld Dateien gehören jetzt zu %@", &[&rels.len(), &label]) };
        self.commit(message, "link", vec![before]);
    }

    /// Links a file to a deadline, or takes the link back when it already is linked.
    pub fn toggle_link(&mut self, path: &Path, id: &str) {
        if self.is_linked(path, id) { self.unlink(path, id) } else { self.link(&[path.to_path_buf()], id) }
    }

    /// The deadlines a file belongs to, the next one due first.
    pub fn deadlines_of(&self, path: &Path) -> Vec<Assignment> {
        let rel = self.relative_path(path);
        let mut found: Vec<Assignment> = self.state.linked_files.iter().filter(|(_, l)| l.contains(&rel)).filter_map(|(id, _)| self.assignment(id).cloned()).collect();
        let today = self.today();
        found.sort_by_key(|a| (day::local_date(a.due) < today, if day::local_date(a.due) < today { -a.due.timestamp() } else { a.due.timestamp() }));
        found
    }

    /// Deadlines a file could belong to: those of its module still open, nearest first, and the ones it already belongs to.
    pub fn link_choices(&self, path: &Path) -> Vec<(Assignment, bool)> {
        let module = self.module_containing(path).map(|m| m.id.clone());
        let linked: Vec<String> = self.deadlines_of(path).into_iter().map(|a| a.id).collect();
        let now = self.now;
        let mut choices: Vec<(Assignment, bool)> = self
            .semester_assignments()
            .into_iter()
            .filter(|a| a.tracks_files() && module.as_ref().is_none_or(|m| *m == a.module_id))
            .filter(|a| linked.contains(&a.id) || !self.status(a).is_closed())
            .map(|a| (a.clone(), linked.contains(&a.id)))
            .collect();
        choices.sort_by_key(|(a, on)| (!on, (a.due - now).num_minutes().abs()));
        choices.truncate(8);
        choices.sort_by_key(|(a, _)| a.due);
        choices
    }

    /// A note that belongs to the deadline from the start, in its module's submissions folder; one undo takes both back.
    pub fn create_assignment_note(&mut self, id: &str) -> Option<PathBuf> {
        let a = self.assignment(id).filter(|a| a.tracks_files()).cloned()?;
        let Some(folder) = self.filing_folder(&a) else {
            self.last_error = Some(tr("Der Modulordner fehlt. Lege ihn an oder schalte „Ordner anlegen“ ein."));
            return None;
        };
        let plain = match (a.rule_id.is_some(), a.week) {
            (true, Some(w)) => format!("{} W{w:02}", a.title),
            _ => a.title.clone(),
        };
        let name = format!("{}.md", Self::typed_name(&plain));
        let result: Result<PathBuf> = (|| {
            std::fs::create_dir_all(&folder)?;
            let path = unique_path(&folder.join(name));
            std::fs::write(&path, self.new_note_text(&plain))?;
            Ok(path)
        })();
        match result {
            Ok(path) => {
                self.mark_made_here(&path);
                let before = self.state_snapshot();
                let rel = self.relative_path(&path);
                self.state.linked_files.entry(a.id.clone()).or_default().push(rel);
                self.save_state();
                self.refresh();
                self.sync_soon = true;
                self.commit(trf("Angelegt für %@: %@", &[&deadline_label(&a), &name_of(&path)]), "square-pen", vec![Change::Trash(path.clone()), before]);
                Some(path)
            }
            Err(e) => {
                self.last_error = Some(trf("Notiz konnte nicht angelegt werden: %@", &[&explain(&e)]));
                None
            }
        }
    }

    /// Forgets links to files that went to the trash; returns the state from before, for the same undo step.
    fn drop_links_to(&mut self, gone: &[PathBuf]) -> Option<Change> {
        let rels: Vec<String> = gone.iter().filter(|p| self.inside(p)).map(|p| self.relative_path(p)).collect();
        let hit = |r: &String| rels.iter().any(|g| r == g || r.starts_with(&format!("{g}/")));
        if !self.state.linked_files.values().flatten().any(hit) {
            return None;
        }
        let before = self.state_snapshot();
        for list in self.state.linked_files.values_mut() {
            list.retain(|r| !hit(r));
        }
        self.state.linked_files.retain(|_, l| !l.is_empty());
        self.save_state();
        Some(before)
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
        for file in paths.iter().flat_map(|p| files_within(p)) {
            match copy_path(&file, &unique_path(&inbox.join(name_of(&file)))) {
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

    // MARK: Documents

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
            open_path(u.as_str());
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
        let canonical = |p: PathBuf| dunce::canonicalize(&p).unwrap_or(p);
        let mut seen = HashSet::new();
        self.open_documents = rels.iter().map(|r| root.join(r)).filter(|p| p.exists()).map(canonical).filter(|p| seen.insert(p.clone())).collect();
        let active = prefs::get().active_documents.get(&self.documents_key()).map(|r| canonical(root.join(r)));
        self.active_document = active.filter(|p| self.open_documents.contains(p)).or_else(|| self.open_documents.first().cloned());
        let split = prefs::get().split_documents.get(&self.documents_key()).map(|r| canonical(root.join(r)));
        self.split_document = split.filter(|p| self.open_documents.contains(p) && self.active_document.as_ref() != Some(p));
    }

    fn save_documents(&self) {
        let key = self.documents_key();
        let rels: Vec<String> = self.open_documents.iter().map(|p| self.relative_path(p)).collect();
        let split = self.split_document.as_ref().map(|p| self.relative_path(p));
        let active = self.active_document.as_ref().map(|p| self.relative_path(p));
        prefs::update(|p| {
            p.open_documents.insert(key.clone(), rels);
            match active {
                Some(active) => p.active_documents.insert(key.clone(), active),
                None => p.active_documents.remove(&key),
            };
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
        if self.split_document.as_ref() == Some(&path)
            && let Some(left) = self.active_document.clone()
        {
            self.focus_beside = true;
            self.beside_active = true;
            self.set_route(Route::Doc(left));
            return;
        }
        self.beside_active = false;
        if !self.open_documents.contains(&path) {
            match self.active_document.as_ref().and_then(|a| self.open_documents.iter().position(|d| d == a)) {
                Some(i) => self.open_documents.insert(i + 1, path.clone()),
                None => self.open_documents.push(path.clone()),
            }
        }
        self.follow_outside_moves();
        self.active_document = Some(path.clone());
        self.save_documents();
        self.set_route(Route::Doc(path));
    }

    /// Shows an open document next to the current one, or ends that when `None`.
    pub fn set_split(&mut self, path: Option<PathBuf>) {
        self.split_document = path.filter(|p| self.open_documents.contains(p) && self.active_document.as_ref() != Some(p));
        if let Some(p) = self.split_document.clone() {
            self.pin_document(&p);
        }
        self.save_documents();
        self.changed();
    }

    fn shown_document(&self) -> Option<&PathBuf> {
        match &self.route {
            Route::Doc(p) => Some(p),
            _ => self.active_document.as_ref(),
        }
    }

    fn dropped_path(&self, path: &Path) -> PathBuf {
        if self.open_documents.iter().any(|d| d == path) {
            return path.to_path_buf();
        }
        dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    fn left_after_move(&self, path: &Path, fallback: Option<&Path>) -> Option<PathBuf> {
        let open = |f: &Path| f != path && self.open_documents.iter().any(|d| d == f);
        self.split_document.clone().filter(|s| s != path).or_else(|| fallback.filter(|f| open(f)).map(Path::to_path_buf)).or_else(|| {
            let i = self.open_documents.iter().position(|d| d == path)?;
            self.open_documents.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| self.open_documents.get(j))).filter(|d| *d != path).cloned()
        })
    }

    /// Whether dropping a dragged document there changes anything. `fallback` takes the shown pane when the shown document itself moves beside.
    pub fn can_drop_document(&self, path: &Path, to: DocDrop, fallback: Option<&Path>) -> bool {
        let path = self.dropped_path(path);
        if !self.open_documents.contains(&path) && !(path.exists() && Self::opens_in_app(&path)) {
            return false;
        }
        let shown = self.shown_document() == Some(&path);
        match to {
            DocDrop::Here => !shown,
            DocDrop::Beside => self.shown_document().is_some() && self.split_document.as_ref() != Some(&path) && (!shown || self.left_after_move(&path, fallback).is_some()),
        }
    }

    /// Shows a dragged document in the shown pane or beside it, opening it as a tab first.
    pub fn drop_document(&mut self, path: &Path, to: DocDrop, fallback: Option<&Path>) {
        if !self.can_drop_document(path, to, fallback) {
            return;
        }
        let path = self.dropped_path(path);
        let shown = self.shown_document().cloned();
        match to {
            DocDrop::Here => {
                if self.split_document.as_ref() == Some(&path)
                    && let Some(shown) = shown
                {
                    self.split_document = Some(shown);
                    self.active_document = Some(path.clone());
                    self.save_documents();
                    self.set_route(Route::Doc(path));
                } else if self.open_documents.contains(&path) {
                    self.open_document(&path);
                    self.pin_document(&path);
                } else {
                    self.open_file(&path);
                    self.pin_document(&path);
                }
            }
            DocDrop::Beside => {
                if !self.open_documents.contains(&path) {
                    let at = shown.as_ref().and_then(|a| self.open_documents.iter().position(|d| d == a)).map_or(self.open_documents.len(), |i| i + 1);
                    self.open_documents.insert(at, path.clone());
                }
                self.pin_document(&path);
                let left = match shown {
                    Some(s) if s == path => self.left_after_move(&path, fallback),
                    Some(s) if self.active_document.as_ref() != Some(&s) => Some(s),
                    _ => None,
                };
                self.split_document = Some(path);
                self.follow_outside_moves();
                self.save_documents();
                match left {
                    Some(left) => {
                        self.active_document = Some(left.clone());
                        self.set_route(Route::Doc(left));
                    }
                    None => self.changed(),
                }
            }
        }
    }

    /// The buffer for a note, created on first use.
    pub fn buffer(&mut self, path: &Path) -> &mut NoteBuffer {
        let gate = self.sync_gate;
        self.buffers.entry(path.to_path_buf()).or_insert_with(|| {
            let mut b = NoteBuffer::open(path);
            if gate {
                b.hold();
            }
            b
        })
    }

    /// Removes a document tab; the caller saves or asks about unsaved changes first.
    pub fn close_document(&mut self, path: &Path) {
        self.follow_heading(path);
        let settled = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let renamed = self.last_renamed.clone().filter(|_| !self.open_documents.iter().any(|d| d == path));
        let path = renamed.as_deref().unwrap_or(&settled).to_path_buf();
        self.close_tab(&path);
    }

    fn close_tab(&mut self, path: &Path) {
        let Some(i) = self.open_documents.iter().position(|d| d == path) else { return };
        if !self.note_windows.contains(path) {
            self.buffers.remove(path);
        }
        self.closed_documents.push((self.open_documents.remove(i), i));
        self.tab_trails.remove(path);
        if self.preview_document.as_deref() == Some(path) {
            self.preview_document = None;
        }
        if self.split_document.as_deref() == Some(path) {
            self.split_document = None;
        }
        let was_shown = self.route == Route::Doc(path.to_path_buf());
        if self.active_document.as_deref() == Some(path) {
            let split = self.split_document.clone();
            let distance = |j: usize| if j >= i { 2 * (j - i) } else { 2 * (i - 1 - j) + 1 };
            let next = (0..self.open_documents.len()).filter(|&j| split.as_ref() != Some(&self.open_documents[j])).min_by_key(|&j| distance(j));
            self.active_document = match next {
                Some(j) => Some(self.open_documents[j].clone()),
                None => self.split_document.take(),
            };
        }
        self.save_documents();
        if was_shown {
            match self.active_document.clone() {
                Some(next) => {
                    let from = std::mem::replace(&mut self.route, Route::Doc(next));
                    self.arrived(&from);
                }
                None => self.go_back(),
            }
        }
        self.changed();
    }

    /// Whether a closed tab is there to bring back.
    pub fn can_reopen_document(&self) -> bool {
        !self.closed_documents.is_empty()
    }

    /// Opens the most recently closed tab whose file still exists.
    pub fn reopen_document(&mut self) {
        while let Some((path, at)) = self.closed_documents.pop() {
            if path.exists() {
                if !self.open_documents.contains(&path) {
                    self.open_documents.insert(at.min(self.open_documents.len()), path.clone());
                }
                self.open_document(&path);
                return;
            }
        }
    }

    /// Moves a tab to the place of `before`, or to the end when `before` is unknown.
    pub fn move_document(&mut self, path: &Path, before: &Path) {
        if path == before {
            return;
        }
        let Some(from) = self.open_documents.iter().position(|d| d == path) else { return };
        let to = self.open_documents.iter().position(|d| d == before).unwrap_or(self.open_documents.len() - 1);
        let doc = self.open_documents.remove(from);
        self.open_documents.insert(to, doc);
        self.save_documents();
        self.changed();
    }

    pub fn close_other_documents(&mut self, path: &Path) {
        self.closed_documents.extend(self.open_documents.iter().enumerate().rev().filter(|(_, d)| *d != path).map(|(i, d)| (d.clone(), i)));
        self.split_document = None;
        let windows = self.note_windows.clone();
        self.buffers.retain(|p, _| p == path || windows.contains(p));
        self.open_documents.retain(|p| p == path);
        self.save_documents();
        self.open_document(path);
    }

    pub fn close_all_documents(&mut self) {
        self.closed_documents.extend(self.open_documents.iter().cloned().enumerate().rev().map(|(i, d)| (d, i)));
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
    /// The note on the right is on screen already and is skipped.
    pub fn cycle_document(&mut self, step: i64) {
        let docs: Vec<PathBuf> = self.open_documents.iter().filter(|d| self.split_document.as_ref() != Some(*d)).cloned().collect();
        if docs.is_empty() {
            return;
        }
        let n = docs.len() as i64;
        let i = self.active_document.as_ref().and_then(|a| docs.iter().position(|d| d == a)).unwrap_or(0) as i64;
        let next = docs[((i + step + n) % n) as usize].clone();
        self.open_document(&next);
    }

    pub fn go_back(&mut self) {
        if self.step_tab(true) {
            return;
        }
        while let Some(r) = self.back_routes.pop() {
            if self.reachable(&r) {
                self.land(&r);
                let from = std::mem::replace(&mut self.route, r);
                self.arrived(&from);
                self.forward_routes.push(from);
                self.changed();
                return;
            }
        }
        if !self.route_exists(&self.route) {
            let from = std::mem::replace(&mut self.route, Route::Today);
            self.arrived(&from);
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
        if !path.exists() {
            self.last_error = Some(trf("„%@“ gibt es nicht mehr. Vielleicht wurde die Datei verschoben oder gelöscht.", &[&name_of(path)]));
            self.refresh();
        } else if Self::opens_in_app(path) {
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

/// The folder for copies kept only for undo. It lies outside the workspace, so syncing never sees it.
fn undo_folder() -> Option<PathBuf> {
    super::prefs::data_dir().map(|d| d.join("undo"))
}

/// Keeps a copy of a file for undo; on APFS the copy takes no extra space.
fn stash(path: &Path) -> Result<PathBuf> {
    let dir = undo_folder().ok_or_else(|| anyhow::anyhow!("no data folder"))?.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir)?;
    let copy = dir.join(name_of(path));
    copy_path(path, &copy)?;
    Ok(copy)
}

/// Removes undo copies older than a week; the undo history only lasts one session anyway.
fn prune_undo_copies() {
    let Some(entries) = undo_folder().and_then(|d| std::fs::read_dir(d).ok()) else { return };
    let week = std::time::Duration::from_secs(7 * 24 * 3600);
    for entry in entries.flatten() {
        if entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age > week)) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The files a dropped item stands for: the file itself, or everything visible inside a folder.
fn files_within(path: &Path) -> Vec<PathBuf> {
    if !path.is_dir() {
        return vec![path.to_path_buf()];
    }
    walkdir::WalkDir::new(path).into_iter().filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.')).filter_map(Result::ok).filter(|e| e.file_type().is_file()).map(|e| e.into_path()).collect()
}

/// Puts files in the system trash, each with a copy kept for undo. Slow on Windows, so the app runs it off the main thread.
pub fn trash_files(paths: &[PathBuf]) -> Vec<(PathBuf, Result<Change>)> {
    paths.iter().map(|p| (p.clone(), trash_with_copy(p))).collect()
}

fn trash_with_copy(path: &Path) -> Result<Change> {
    let backup = stash(path)?;
    #[cfg(not(test))]
    let moved = match super::drive::trash_slot(path) {
        Some(slot) => slot.and_then(|slot| move_path(path, &slot)).map_err(anyhow::Error::from),
        None => trash::delete(path).map_err(anyhow::Error::from),
    };
    #[cfg(test)]
    let moved = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }.map_err(anyhow::Error::from);
    if let Err(e) = moved {
        let _ = backup.parent().map(std::fs::remove_dir_all);
        return Err(e);
    }
    Ok(Change::Restore { path: path.to_path_buf(), backup })
}

/// The path with its folder resolved, so it compares with open documents; works after the file itself is gone.
fn settled_path(p: &Path) -> PathBuf {
    p.parent().and_then(|d| dunce::canonicalize(d).ok()).zip(p.file_name()).map_or_else(|| p.to_path_buf(), |(d, n)| d.join(n))
}

/// The name a file gets for `role` without a convention: its own, with only the task-sheet word added or taken out.
pub(crate) fn plain_role_name(name: &str, role: FileRole, author: &str) -> String {
    let brief = FileRole::is_brief(name, author);
    let ext = super::naming::extension(name);
    let stem = super::naming::stem(name);
    let dotted = |stem: &str| if ext.is_empty() { stem.to_string() } else { format!("{stem}.{ext}") };
    let word = |token: &str| FileRole::is_brief(token, "");
    let separator = |c: char| matches!(c, ' ' | '_' | '-');
    match role {
        FileRole::Brief if !brief => {
            let own = without_author(stem, author).unwrap_or_else(|| stem.to_string());
            let own = own.as_str();
            let new = if own.is_empty() { dotted(BRIEF_TAG) } else if word(own) { dotted(own) } else { dotted(&format!("{own} {BRIEF_TAG}")) };
            if FileRole::is_brief(&new, author) { new } else { name.to_string() }
        }
        FileRole::Work if brief => {
            let kept = match stem.strip_suffix(BRIEF_TAG) {
                Some("") => Some(""),
                Some(rest) if rest.ends_with(separator) => Some(rest.trim_end_matches(separator)),
                _ => None,
            };
            match kept {
                Some("") => dotted(&tr("Abgabe")),
                Some(rest) if !FileRole::is_brief(rest, "") => dotted(rest),
                _ => {
                    let tagged = dotted(&format!("{stem} {author}"));
                    if !author.trim().is_empty() && !FileRole::is_brief(&tagged, author) { tagged } else { name.to_string() }
                }
            }
        }
        _ => name.to_string(),
    }
}

fn without_author(stem: &str, author: &str) -> Option<String> {
    let separator = |c: char| matches!(c, ' ' | '_' | '-');
    let wanted: Vec<String> = fold(author).split(separator).filter(|t| !t.is_empty()).map(str::to_string).collect();
    if wanted.is_empty() {
        return None;
    }
    let mut runs: Vec<(bool, String)> = Vec::new();
    for c in stem.chars() {
        match runs.last_mut() {
            Some((gap, text)) if *gap == separator(c) => text.push(c),
            _ => runs.push((separator(c), c.to_string())),
        }
    }
    let words: Vec<usize> = (0..runs.len()).filter(|&i| !runs[i].0).collect();
    let first = (0..words.len()).find(|&i| i + wanted.len() <= words.len() && words[i..i + wanted.len()].iter().zip(&wanted).all(|(&r, w)| fold(&runs[r].1) == *w))?;
    let (mut from, mut to) = (words[first], words[first + wanted.len() - 1]);
    if to + 1 < runs.len() {
        to += 1;
    } else {
        from = from.saturating_sub(1);
    }
    Some(runs.iter().enumerate().filter(|(i, _)| *i < from || *i > to).map(|(_, r)| r.1.as_str()).collect())
}

fn heading_titled(path: &Path) -> bool {
    let settled = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    super::prefs::get().heading_titled.contains(settled.to_string_lossy().as_ref())
}

/// Where a rename goes; changing only the case of a name on a case-insensitive disk is not a collision.
fn rename_target(file: &Path, name: &str) -> PathBuf {
    let wanted = file.with_file_name(name);
    if same_file(file, &wanted) { wanted } else { unique_path(&wanted) }
}

/// Whether two different spellings name one file, as a case-only rename does on a case-insensitive disk.
#[cfg(unix)]
pub fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    a != b && std::fs::metadata(a).ok().zip(std::fs::metadata(b).ok()).is_some_and(|(x, y)| x.ino() == y.ino() && x.dev() == y.dev())
}

/// Whether two different spellings name one file, as a case-only rename does on a case-insensitive disk.
#[cfg(not(unix))]
pub fn same_file(a: &Path, b: &Path) -> bool {
    a != b && dunce::canonicalize(a).ok().zip(dunce::canonicalize(b).ok()).is_some_and(|(x, y)| x == y)
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

/// Opens a file, folder or web address in its default app.
pub fn open_path(path: impl AsRef<std::ffi::OsStr>) {
    let target = path.as_ref();
    if !super::drive::fenced("open", Path::new(target).display()) {
        let _ = opener::open(target);
    }
}

/// Opens a folder in Finder / Explorer / the file manager, creating it first if needed.
pub fn open_folder(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    open_path(path);
}

/// Shows a file in Finder / Explorer / the file manager.
pub fn reveal(path: &Path) {
    if path.exists() && !super::drive::fenced("reveal", path.display()) {
        let _ = opener::reveal(path);
    }
}

/// What takes back filing `original` as `target`: moving it back, or trashing the copy of an outside file.
/// Whether a file can be named by its module's convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConventionOffer {
    /// Not offered: no convention, no submission folder, or the name already fits.
    Hidden,
    /// In a submission folder, but no deadline is linked yet.
    NeedsDeadline,
    Ready,
}

/// How a deadline is named in short messages, e.g. "ÜB W03".
/// What renaming one file by convention knows before reading the file.
#[derive(Clone)]
pub struct ConventionPlan {
    naming: Naming,
    namings: Arc<Vec<Naming>>,
    ignore: Vec<String>,
    weeks: Vec<LectureWeek>,
    pub module: String,
    pub deadline: String,
    due: DateTime<Local>,
    week: Option<u32>,
    iso_week: Option<u32>,
    brief: Option<Naming>,
    parsed: Option<Parsed>,
    heading: Option<String>,
}

impl ConventionPlan {
    /// The suggested name: the note's heading, else the title its name or contents give; its date and co-authors are kept. Reads the file, so call it off the UI thread.
    pub fn name(&self, file: &Path) -> String {
        let facts = FileFacts::read(file);
        let heading = self.heading.clone().or_else(|| super::markdown::is_markdown(file).then(|| std::fs::read_to_string(file).ok()).flatten().and_then(|t| super::markdown::first_heading(&t)));
        let title = heading.or_else(|| self.parsed.as_ref().map(|p| p.title.replace('-', " "))).or_else(|| filing::title_suggestions(&facts, &self.namings, &self.ignore).first().map(|s| s.title.clone())).unwrap_or_else(|| super::naming::stem(&name_of(file)).to_string());
        let created = facts.created.min(self.due);
        let date = self.parsed.as_ref().and_then(|p| p.date).or(facts.name_date).unwrap_or_else(|| created.date_naive());
        let ext = extension(&name_of(file)).to_string();
        let week = self.week.or_else(|| self.weeks.iter().find(|w| w.monday == day::monday(date)).map(|w| w.index));
        if self.brief.is_some() {
            return self.naming.render(&title, date, &ext, &[], week, self.iso_week, Some(BRIEF_TAG));
        }
        self.naming.render(&title, date, &ext, &self.parsed.as_ref().map(|p| p.coauthors.clone()).unwrap_or_default(), week, self.iso_week, None)
    }

    /// Whether `name` follows the convention.
    pub fn fits(&self, name: &str) -> bool {
        self.naming.is_valid(name) || self.naming.parse(name).is_some() || self.brief.as_ref().is_some_and(|b| b.parse(name).is_some())
    }
}

pub fn deadline_label(a: &Assignment) -> String {
    match (a.rule_id.is_some(), a.week) {
        (true, Some(w)) => format!("{} W{w:02}", a.short),
        _ => a.short.clone(),
    }
}

fn filing_change(original: &Path, target: &Path, moved: bool) -> Change {
    if moved { Change::Move { from: target.to_path_buf(), to: original.to_path_buf() } } else { Change::Trash(target.to_path_buf()) }
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
