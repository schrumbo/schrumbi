use std::collections::{BTreeMap, BTreeSet};
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
    Focus,
    /// A few seconds after the last keystroke, plus everything `Focus` does.
    Delay,
    /// Only with ⌘S (and on close, if enabled).
    Manual,
}

/// How the file browser groups files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileGrouping {
    #[default]
    Week,
    Folder,
    Date,
}

/// A key plus modifiers, as stored for custom shortcuts.
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
    pub pdf_header: bool,
    pub pdf_page_numbers: bool,
    pub pdf_extra_css: String,
    pub editor_font_size: f32,
    pub note_save_mode: NoteSaveMode,
    pub note_save_delay: f64,
    pub note_save_on_close: bool,
    pub file_grouping: FileGrouping,
    pub file_kind: String,
    pub workspace: Option<PathBuf>,
    pub recent_workspaces: Vec<PathBuf>,
    pub recent_coauthors: Vec<String>,
    pub indexed_root: Option<PathBuf>,
    /// Open document tabs per workspace, as paths relative to it.
    pub open_documents: BTreeMap<String, Vec<String>>,
    pub shortcuts: BTreeMap<String, Shortcut>,
    pub disabled_shortcuts: BTreeSet<String>,
    pub tray: bool,
    /// Keeps `Schrumbi-Kalender.ics` in the workspace up to date for calendar subscriptions.
    pub calendar_feed: bool,
    pub sidebar_open: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            reminders_enabled: true,
            reminder_hours: 3,
            reminder_day_before: true,
            language: String::new(),
            note_accent: DEFAULT_ACCENT.into(),
            pdf_header: true,
            pdf_page_numbers: true,
            pdf_extra_css: String::new(),
            editor_font_size: 13.,
            note_save_mode: NoteSaveMode::Focus,
            note_save_delay: 5.,
            note_save_on_close: true,
            file_grouping: FileGrouping::Week,
            file_kind: "documents".into(),
            workspace: None,
            recent_workspaces: Vec::new(),
            recent_coauthors: Vec::new(),
            indexed_root: None,
            open_documents: BTreeMap::new(),
            shortcuts: BTreeMap::new(),
            disabled_shortcuts: BTreeSet::new(),
            tray: true,
            calendar_feed: false,
            sidebar_open: true,
        }
    }
}

/// Folder for settings; `SCHRUMBI_HOME` redirects settings and caches, e.g. for tests.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("SCHRUMBI_HOME").map(PathBuf::from).or_else(|| dirs::config_dir().map(|d| d.join("Schrumbi")))
}

/// Folder for caches such as the search index.
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("SCHRUMBI_HOME").map(|d| PathBuf::from(d).join("data")).or_else(|| dirs::data_dir().map(|d| d.join("Schrumbi")))
}

static PREFS: LazyLock<RwLock<Prefs>> = LazyLock::new(|| {
    let loaded = config_dir().and_then(|d| std::fs::read(d.join("settings.json")).ok()).and_then(|b| serde_json::from_slice(&b).ok());
    RwLock::new(loaded.unwrap_or_default())
});

/// A copy of the current settings.
pub fn get() -> Prefs {
    PREFS.read().unwrap().clone()
}

/// Changes settings and writes them to disk.
pub fn update(f: impl FnOnce(&mut Prefs)) {
    let snapshot = {
        let mut p = PREFS.write().unwrap();
        f(&mut p);
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
