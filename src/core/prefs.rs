use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Serialize};

pub const DEFAULT_ACCENT: &str = "#0EA5E9";

/// When notes are written to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteSaveMode {
    /// When the editor loses focus, the window or app goes to the background, or the note is closed.
    #[default]
    #[serde(alias = "manual")]
    Focus,
    /// A few seconds after the last keystroke, plus everything `Focus` does.
    Delay,
}

/// How a note was shown last; the next one opens the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteMode {
    Read,
    #[default]
    Write,
    Split,
}

/// When a synced workspace saves its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitMode {
    #[default]
    Manual,
    Auto,
}

/// Sync settings of one workspace on this device. Not in the workspace: paths differ per device,
/// and a cloned repository must never be able to switch itself on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GitPrefs {
    pub enabled: bool,
    pub mode: GitMode,
    /// Minutes without changes before an automatic sync.
    pub idle_minutes: u32,
    pub sync_on_open: bool,
    pub sync_on_quit: bool,
    pub last_sync: Option<i64>,
    /// The sidebar no longer offers setup here.
    pub offer_hidden: bool,
}

impl Default for GitPrefs {
    fn default() -> Self {
        Self { enabled: false, mode: GitMode::Manual, idle_minutes: 5, sync_on_open: true, sync_on_quit: true, last_sync: None, offer_hidden: false }
    }
}

/// How the file browser groups files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileGrouping {
    Week,
    #[default]
    Folder,
    Date,
}

/// How notes are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteEditor {
    /// The note stays drawn; only the line with the caret shows its marks.
    Live,
    /// The whole note as source text with line numbers.
    #[default]
    Code,
}

/// The colours of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeChoice {
    /// Light or dark, as the system is.
    #[default]
    System,
    Light,
    Dark,
    Nord,
    Dracula,
}

/// A key plus modifiers, as stored for custom shortcuts.
/// A reading position in a PDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfPosition {
    pub page: usize,
    pub fraction: f32,
    pub scale: Option<f32>,
    pub fit_page: bool,
    pub used: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Shortcut {
    /// A lowercased character, or a special key name: `up`, `down`, `left`, `right`, `tab`, `return`, `space`, `delete`, `escape`.
    pub key: String,
    #[serde(default)]
    pub command: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub option: bool,
    #[serde(default)]
    pub control: bool,
}

/// App settings and per-user state, stored as JSON in the platform config folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub reminders_enabled: bool,
    pub reminder_hours: i64,
    pub reminder_day_before: bool,
    /// UI language code; empty follows the system.
    pub language: String,
    pub note_accent: String,
    /// Where each PDF was last read, by path.
    pub pdf_positions: std::collections::BTreeMap<String, PdfPosition>,
    /// Where reading a note stopped, by its path.
    pub note_places: std::collections::BTreeMap<String, f32>,
    pub pdf_header: bool,
    pub pdf_page_numbers: bool,
    pub pdf_properties: bool,
    pub editor_font_size: f32,
    pub theme: ThemeChoice,
    /// The chance of remembering a card that reviews aim for.
    pub card_retention: f64,
    pub card_new_per_day: u32,
    pub card_reviews_per_day: u32,
    /// The note new cards went to last, relative to the vault.
    pub card_target: Option<String>,
    /// The deck last open on the cards page.
    pub card_deck: String,
    /// The folders open in its deck tree.
    pub card_open: Vec<String>,
    /// Saved sets of open tabs, by workspace, by name.
    pub layouts: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// Notes save while typing since this was the default; older settings move over once.
    pub saves_while_typing: bool,
    pub note_save_mode: NoteSaveMode,
    pub note_save_delay: f64,
    pub file_grouping: FileGrouping,
    pub file_kind: String,
    pub workspace: Option<PathBuf>,
    pub recent_workspaces: Vec<PathBuf>,
    pub recent_coauthors: Vec<String>,
    pub indexed_root: Option<PathBuf>,
    /// Open document tabs per workspace, as paths relative to it.
    pub open_documents: BTreeMap<String, Vec<String>>,
    /// The document shown beside the current one, per workspace.
    pub split_documents: BTreeMap<String, String>,
    /// The tab shown last, per workspace.
    pub active_documents: BTreeMap<String, String>,
    /// Share of the width the left document gets in the split view.
    pub split_share: f32,
    /// Width of the folder tree on the notes page.
    pub notes_column: Option<f32>,
    pub notes_explorer_hidden: bool,
    pub spell_check: bool,
    /// Words the spell check should not mark, sorted.
    pub spell_words: Vec<String>,
    /// Notes use the whole pane width instead of a centered reading column.
    pub reader_full_width: bool,
    /// How notes are written. Stored under a new name, so everyone starts on the code editor once.
    #[serde(rename = "noteEditorMode")]
    pub note_editor: NoteEditor,
    /// How the last note was shown.
    pub note_mode: NoteMode,
    /// Notes made in the app: their file name follows their first heading.
    pub heading_titled: BTreeSet<String>,
    /// What was folded in each note's reading view.
    pub note_folds: HashMap<String, Vec<String>>,
    /// Notes whose line wrapping in the source editor differs from the default for their size, by path.
    pub note_wraps: BTreeMap<String, bool>,
    /// Width of the deadlines column on module pages.
    pub module_column: Option<f32>,
    pub shortcuts: BTreeMap<String, Shortcut>,
    pub disabled_shortcuts: BTreeSet<String>,
    pub git: BTreeMap<String, GitPrefs>,
    /// An Obsidian vault outside the workspace, by workspace path.
    pub vaults: BTreeMap<String, std::path::PathBuf>,
    /// A Git to use instead of the one found automatically.
    pub git_path: Option<PathBuf>,
    pub tray: bool,
    /// Looks for a new version once a day.
    pub update_check: bool,
    /// A version the user chose not to install.
    pub update_skip: String,
    /// Asks before a PDF export replaces an earlier export of the same note.
    pub pdf_replace_ask: bool,
    /// Keeps `Schrumbi-Kalender.ics` in the workspace up to date for calendar subscriptions.
    pub calendar_feed: bool,
    /// Part of the local subscription address, so only who was given it reads the calendar.
    pub calendar_token: String,
    /// Writes the semester into the calendar „Schrumbi“ in Apple Kalender (macOS).
    pub apple_calendar: bool,
    /// Calendar addresses to fetch deadlines from, by workspace.
    pub calendar_sources: BTreeMap<String, String>,
    /// Whether the calendar on the overview shows a week or a month.
    pub calendar_view: super::calendar::CalendarView,
    /// Kinds of pages whose details column the user keeps open: `document`, `assignments`, `graph`.
    pub inspector_pinned: BTreeSet<String>,
    /// Sidebar entries the user hid; they stay reachable by shortcut and palette.
    pub hidden_tabs: Vec<String>,
    /// Last position and size of the main window: x, y, width, height.
    pub main_window: Option<[f32; 4]>,
    /// Last position of the upload panel: x, y.
    pub upload_window: Option<[f32; 2]>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            reminders_enabled: true,
            reminder_hours: 3,
            reminder_day_before: true,
            language: String::new(),
            note_accent: DEFAULT_ACCENT.into(),
            pdf_positions: Default::default(),
            note_places: Default::default(),
            pdf_header: true,
            pdf_page_numbers: true,
            pdf_properties: false,
            editor_font_size: 13.,
            theme: ThemeChoice::System,
            card_retention: 0.9,
            card_new_per_day: 30,
            card_reviews_per_day: 200,
            card_target: None,
            note_mode: NoteMode::default(),
            heading_titled: BTreeSet::new(),
            card_deck: String::new(),
            card_open: Vec::new(),
            layouts: BTreeMap::new(),
            saves_while_typing: false,
            note_save_mode: NoteSaveMode::Delay,
            note_save_delay: 2.,
            file_grouping: FileGrouping::Week,
            file_kind: "documents".into(),
            workspace: None,
            recent_workspaces: Vec::new(),
            recent_coauthors: Vec::new(),
            indexed_root: None,
            open_documents: BTreeMap::new(),
            split_documents: BTreeMap::new(),
            active_documents: BTreeMap::new(),
            split_share: 0.5,
            module_column: None,
            notes_column: None,
            notes_explorer_hidden: false,
            spell_check: true,
            spell_words: Vec::new(),
            reader_full_width: true,
            note_editor: NoteEditor::default(),
            note_folds: HashMap::new(),
            note_wraps: BTreeMap::new(),
            shortcuts: BTreeMap::new(),
            disabled_shortcuts: BTreeSet::new(),
            git: BTreeMap::new(),
            vaults: BTreeMap::new(),
            git_path: None,
            tray: true,
            update_check: true,
            update_skip: String::new(),
            pdf_replace_ask: true,
            calendar_feed: false,
            calendar_token: String::new(),
            apple_calendar: false,
            calendar_sources: BTreeMap::new(),
            calendar_view: Default::default(),
            inspector_pinned: BTreeSet::new(),
            hidden_tabs: Vec::new(),
            main_window: None,
            upload_window: None,
        }
    }
}

/// Folder for settings; `SCHRUMBI_HOME` redirects settings and caches, e.g. for tests.
pub fn config_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return Some(std::env::temp_dir().join(format!("schrumbi-test-home-{}", std::process::id())));
    }
    std::env::var_os("SCHRUMBI_HOME").map(PathBuf::from).or_else(|| dirs::config_dir().map(|d| d.join("Schrumbi")))
}

/// Folder for caches such as the search index.
pub fn data_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return config_dir().map(|d| d.join("data"));
    }
    std::env::var_os("SCHRUMBI_HOME").map(|d| PathBuf::from(d).join("data")).or_else(|| dirs::data_dir().map(|d| d.join("Schrumbi")))
}

static PREFS: LazyLock<RwLock<Prefs>> = LazyLock::new(|| {
    let file = config_dir().map(|d| d.join("settings.json"));
    let bytes = file.as_ref().and_then(|f| std::fs::read(f).ok());
    let loaded = bytes.as_ref().and_then(|b| serde_json::from_slice(b).ok());
    if let (None, Some(_), Some(f)) = (&loaded, &bytes, &file) {
        let _ = std::fs::copy(f, f.with_extension("defekt.json"));
    }
    let mut prefs: Prefs = loaded.unwrap_or_default();
    if !prefs.saves_while_typing {
        prefs.saves_while_typing = true;
        prefs.note_save_mode = NoteSaveMode::Delay;
        prefs.note_save_delay = 2.;
        prefs.file_grouping = FileGrouping::Folder;
    }
    RwLock::new(prefs)
});

/// A copy of the current settings.
pub fn get() -> Prefs {
    PREFS.read().unwrap().clone()
}

/// Reads the current settings without copying them.
pub fn with<R>(f: impl FnOnce(&Prefs) -> R) -> R {
    f(&PREFS.read().unwrap())
}

/// Changes settings and writes them to disk.
pub fn update(f: impl FnOnce(&mut Prefs)) {
    let snapshot = {
        let mut p = PREFS.write().unwrap();
        let before = p.clone();
        f(&mut p);
        if *p == before {
            return;
        }
        p.clone()
    };
    if let Some(dir) = config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(json) = serde_json::to_vec_pretty(&snapshot) {
            let tmp = dir.join("settings.json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(tmp, dir.join("settings.json"));
            }
        }
    }
}

/// Recently opened workspaces, most recent first, keeping eight.
pub fn remember_workspace(path: &std::path::Path) {
    update(|p| {
        p.workspace = Some(path.to_path_buf());
        p.recent_workspaces.retain(|r| r != path);
        p.recent_workspaces.insert(0, path.to_path_buf());
        p.recent_workspaces.truncate(8);
    });
}

pub fn forget_workspace(path: &std::path::Path) {
    update(|p| {
        p.recent_workspaces.retain(|r| r != path);
        if p.workspace.as_deref() == Some(path) {
            p.workspace = None;
        }
    });
}

/// Stores `value` under `key` in a per-note map and drops other keys, in key order, past `cap`; the entry just stored stays.
pub fn remember_capped<V>(map: &mut BTreeMap<String, V>, key: String, value: V, cap: usize) {
    map.insert(key.clone(), value);
    while map.len() > cap {
        let Some(other) = map.keys().find(|k| **k != key).cloned() else { break };
        map.remove(&other);
    }
}

/// Recently used co-authors, most recent first.
pub fn remember_coauthors(names: &[String]) {
    if names.is_empty() {
        return;
    }
    update(|p| {
        let mut merged: Vec<String> = names.to_vec();
        merged.extend(p.recent_coauthors.iter().filter(|c| !names.contains(c)).cloned());
        merged.truncate(12);
        p.recent_coauthors = merged;
    });
}

#[cfg(test)]
mod tests {
    use super::remember_capped;
    use std::collections::BTreeMap;

    #[test]
    fn the_entry_just_stored_is_never_the_one_dropped() {
        let mut map: BTreeMap<String, u8> = BTreeMap::new();
        for k in ["b", "c", "d"] {
            remember_capped(&mut map, k.to_string(), 1, 3);
        }
        remember_capped(&mut map, "a".to_string(), 2, 3);
        assert_eq!(map.len(), 3);
        assert_eq!(map.get("a"), Some(&2));
        assert!(!map.contains_key("b"));
        remember_capped(&mut map, "a".to_string(), 3, 3);
        assert_eq!(map.get("a"), Some(&3));
        remember_capped(&mut map, "e".to_string(), 4, 0);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("e"), Some(&4));
    }
}
