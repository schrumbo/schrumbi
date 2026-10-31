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
    #[serde(alias = "manual")]
    Focus,
    /// A few seconds after the last keystroke, plus everything `Focus` does.
    Delay,
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
    pub pdf_header: bool,
    pub pdf_page_numbers: bool,
    pub editor_font_size: f32,
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
    /// Share of the width the left document gets in the split view.
    pub split_share: f32,
    /// Width of the deadlines column on module pages.
    pub module_column: Option<f32>,
    pub shortcuts: BTreeMap<String, Shortcut>,
    pub disabled_shortcuts: BTreeSet<String>,
    pub tray: bool,
    /// Looks for a new version once a day.
    pub update_check: bool,
    /// A version the user chose not to install.
    pub update_skip: String,
    /// Keeps `Schrumbi-Kalender.ics` in the workspace up to date for calendar subscriptions.
    pub calendar_feed: bool,
    pub sidebar_open: bool,
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
            pdf_header: true,
            pdf_page_numbers: true,
            editor_font_size: 13.,
            note_save_mode: NoteSaveMode::Focus,
            note_save_delay: 5.,
            file_grouping: FileGrouping::Week,
            file_kind: "documents".into(),
            workspace: None,
            recent_workspaces: Vec::new(),
            recent_coauthors: Vec::new(),
            indexed_root: None,
            open_documents: BTreeMap::new(),
            split_documents: BTreeMap::new(),
            split_share: 0.5,
            module_column: None,
            shortcuts: BTreeMap::new(),
            disabled_shortcuts: BTreeSet::new(),
            tray: true,
            update_check: true,
            update_skip: String::new(),
            calendar_feed: false,
            sidebar_open: true,
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
