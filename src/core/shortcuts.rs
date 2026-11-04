use super::i18n::tr;
use super::prefs::{self, Shortcut};

/// Every action with a configurable shortcut. Defaults follow VS Code where it has an equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutAction {
    QuickOpen,
    SearchFiles,
    NewEntry,
    NewFile,
    GoBack,
    CloseTab,
    NextTab,
    PreviousTab,
    TogglePreview,
    SplitPreview,
    ExportPdf,
    ToggleInspector,
    Refresh,
    Sync,
    DailyNote,
    OpenInObsidian,
    Bold,
    Italic,
    Link,
    InlineCode,
    ToggleTask,
    ToggleComment,
    MoveLineUp,
    MoveLineDown,
    CopyLineUp,
    CopyLineDown,
    DeleteLine,
    SelectLine,
    Indent,
    Outdent,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    General,
    Documents,
    Editor,
}

impl Group {
    pub const ALL: [Group; 3] = [Group::General, Group::Documents, Group::Editor];

    pub fn label(self) -> String {
        tr(match self {
            Group::General => "Allgemein",
            Group::Documents => "Dokumente & Notizen",
            Group::Editor => "Editor",
        })
    }
}

fn s(key: &str, command: bool, shift: bool, option: bool, control: bool) -> Shortcut {
    Shortcut { key: key.into(), command, shift, option, control }
}

impl ShortcutAction {
    pub const ALL: [ShortcutAction; 33] = [
        Self::QuickOpen,
        Self::SearchFiles,
        Self::NewEntry,
        Self::NewFile,
        Self::GoBack,
        Self::CloseTab,
        Self::NextTab,
        Self::PreviousTab,
        Self::TogglePreview,
        Self::SplitPreview,
        Self::ExportPdf,
        Self::ToggleInspector,
        Self::Refresh,
        Self::Sync,
        Self::DailyNote,
        Self::OpenInObsidian,
        Self::Bold,
        Self::Italic,
        Self::Link,
        Self::InlineCode,
        Self::ToggleTask,
        Self::ToggleComment,
        Self::MoveLineUp,
        Self::MoveLineDown,
        Self::CopyLineUp,
        Self::CopyLineDown,
        Self::DeleteLine,
        Self::SelectLine,
        Self::Indent,
        Self::Outdent,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ZoomReset,
    ];

    /// Storage key, matching the Swift app's raw values.
    pub fn id(self) -> &'static str {
        match self {
            Self::QuickOpen => "quickOpen",
            Self::SearchFiles => "searchFiles",
            Self::NewEntry => "newEntry",
            Self::NewFile => "newFile",
            Self::GoBack => "goBack",
            Self::CloseTab => "closeTab",
            Self::NextTab => "nextTab",
            Self::PreviousTab => "previousTab",
            Self::TogglePreview => "togglePreview",
            Self::SplitPreview => "splitPreview",
            Self::ExportPdf => "exportPDF",
            Self::ToggleInspector => "toggleInspector",
            Self::Refresh => "refresh",
            Self::Sync => "syncNow",
            Self::DailyNote => "dailyNote",
            Self::OpenInObsidian => "openInObsidian",
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Link => "link",
            Self::InlineCode => "inlineCode",
            Self::ToggleTask => "toggleTask",
            Self::ToggleComment => "toggleComment",
            Self::MoveLineUp => "moveLineUp",
            Self::MoveLineDown => "moveLineDown",
            Self::CopyLineUp => "copyLineUp",
            Self::CopyLineDown => "copyLineDown",
            Self::DeleteLine => "deleteLine",
            Self::SelectLine => "selectLine",
            Self::Indent => "indent",
            Self::Outdent => "outdent",
            Self::ZoomIn => "zoomIn",
            Self::ZoomOut => "zoomOut",
            Self::ZoomReset => "zoomReset",
        }
    }

    pub fn group(self) -> Group {
        match self {
            Self::Sync | Self::DailyNote | Self::OpenInObsidian | Self::QuickOpen | Self::SearchFiles | Self::NewEntry | Self::NewFile | Self::GoBack | Self::ToggleInspector | Self::Refresh => Group::General,
            Self::CloseTab | Self::NextTab | Self::PreviousTab | Self::TogglePreview | Self::SplitPreview | Self::ExportPdf => Group::Documents,
            _ => Group::Editor,
        }
    }

    pub fn label(self) -> String {
        tr(match self {
            Self::QuickOpen => "Datei schnell öffnen",
            Self::SearchFiles => "In allen Dateien suchen",
            Self::NewEntry => "Neuer Eintrag",
            Self::NewFile => "Neue Notiz, Datei oder Ordner",
            Self::GoBack => "Zurück",
            Self::ToggleInspector => "Details ein-/ausblenden",
            Self::Refresh => "Aktualisieren",
            Self::Sync => "Jetzt synchronisieren",
            Self::DailyNote => "Tagesnotiz öffnen",
            Self::OpenInObsidian => "In Obsidian öffnen",
            Self::CloseTab => "Tab schließen",
            Self::NextTab => "Nächster Tab",
            Self::PreviousTab => "Vorheriger Tab",
            Self::TogglePreview => "Lesen / Bearbeiten wechseln",
            Self::SplitPreview => "Geteilte Ansicht",
            Self::ExportPdf => "Als PDF exportieren",
            Self::Bold => "Fett",
            Self::Italic => "Kursiv",
            Self::Link => "Link einfügen",
            Self::InlineCode => "Code",
            Self::ToggleTask => "Aufgabe abhaken",
            Self::ToggleComment => "Auskommentieren",
            Self::MoveLineUp => "Zeile nach oben",
            Self::MoveLineDown => "Zeile nach unten",
            Self::CopyLineUp => "Zeile oben duplizieren",
            Self::CopyLineDown => "Zeile unten duplizieren",
            Self::DeleteLine => "Zeile löschen",
            Self::SelectLine => "Zeile auswählen",
            Self::Indent => "Einrücken",
            Self::Outdent => "Ausrücken",
            Self::ZoomIn => "Schrift größer",
            Self::ZoomOut => "Schrift kleiner",
            Self::ZoomReset => "Schriftgröße zurücksetzen",
        })
    }

    pub fn default_shortcut(self) -> Shortcut {
        if !cfg!(target_os = "macos") {
            match self {
                Self::GoBack => return s("left", false, false, true, false),
                Self::Refresh => return s("f5", false, false, false, false),
                Self::ToggleInspector => return s("i", true, true, false, false),
                _ => {}
            }
        }
        match self {
            Self::QuickOpen => s("o", true, false, false, false),
            Self::SearchFiles => s("f", true, true, false, false),
            Self::NewEntry => s("n", true, true, false, false),
            Self::NewFile => s("n", true, false, false, false),
            Self::GoBack => s("-", false, false, false, true),
            Self::ToggleInspector => s("i", true, false, true, false),
            Self::Refresh => s("r", true, false, false, false),
            Self::Sync => s("s", true, true, false, false),
            Self::DailyNote => s("t", true, false, true, false),
            Self::OpenInObsidian => s("o", true, false, true, false),
            Self::CloseTab => s("w", true, false, false, false),
            Self::NextTab => s("tab", false, false, false, true),
            Self::PreviousTab => s("tab", false, true, false, true),
            Self::TogglePreview => s("v", true, true, false, false),
            Self::SplitPreview => s("v", true, false, true, false),
            Self::ExportPdf => s("e", true, true, false, false),
            Self::Bold => s("b", true, false, false, false),
            Self::Italic => s("i", true, false, false, false),
            Self::Link => s("k", true, false, false, false),
            Self::InlineCode => s("c", true, true, false, false),
            Self::ToggleTask => s("return", true, false, false, false),
            Self::ToggleComment => s("/", true, false, false, false),
            Self::MoveLineUp => s("up", false, false, true, false),
            Self::MoveLineDown => s("down", false, false, true, false),
            Self::CopyLineUp => s("up", false, true, true, false),
            Self::CopyLineDown => s("down", false, true, true, false),
            Self::DeleteLine => s("k", true, true, false, false),
            Self::SelectLine => s("l", true, false, false, false),
            Self::Indent => s("]", true, false, false, false),
            Self::Outdent => s("[", true, false, false, false),
            Self::ZoomIn => s("+", true, false, false, false),
            Self::ZoomOut => s("-", true, false, false, false),
            Self::ZoomReset => s("0", true, false, false, false),
        }
    }

    /// The effective shortcut; `None` when the user cleared it.
    pub fn shortcut(self) -> Option<Shortcut> {
        let p = prefs::get();
        if p.disabled_shortcuts.contains(self.id()) {
            return None;
        }
        Some(p.shortcuts.get(self.id()).cloned().unwrap_or_else(|| self.default_shortcut()))
    }

    pub fn is_customized(self) -> bool {
        let p = prefs::get();
        p.shortcuts.contains_key(self.id()) || p.disabled_shortcuts.contains(self.id())
    }

    pub fn set(self, shortcut: Option<Shortcut>) {
        prefs::update(|p| match shortcut {
            Some(s) => {
                p.disabled_shortcuts.remove(self.id());
                if s == self.default_shortcut() {
                    p.shortcuts.remove(self.id());
                } else {
                    p.shortcuts.insert(self.id().into(), s);
                }
            }
            None => {
                p.shortcuts.remove(self.id());
                p.disabled_shortcuts.insert(self.id().into());
            }
        });
    }

    pub fn reset(self) {
        prefs::update(|p| {
            p.shortcuts.remove(self.id());
            p.disabled_shortcuts.remove(self.id());
        });
    }

    pub fn reset_all() {
        prefs::update(|p| {
            p.shortcuts.clear();
            p.disabled_shortcuts.clear();
        });
    }

    /// The keys as a tooltip suffix, e.g. ` (⌘N)`; empty when the user cleared the shortcut.
    pub fn hint(self) -> String {
        self.shortcut().map_or_else(String::new, |s| format!(" ({})", s.display()))
    }

    /// Other actions using the same keys in an overlapping scope.
    pub fn conflicts(self) -> Vec<ShortcutAction> {
        let Some(mine) = self.shortcut() else { return Vec::new() };
        Self::ALL
            .into_iter()
            .filter(|o| *o != self && o.shortcut().as_ref() == Some(&mine) && (o.group() == self.group() || o.group() == Group::General || self.group() == Group::General))
            .collect()
    }
}

impl Shortcut {
    /// The modifier and key labels in this platform's order, e.g. ⇧ ⌘ E or Ctrl Shift E.
    pub fn parts(&self) -> Vec<String> {
        let mac = cfg!(target_os = "macos");
        let mut parts: Vec<String> = Vec::new();
        if mac {
            for (on, glyph) in [(self.control, "⌃"), (self.option, "⌥"), (self.shift, "⇧"), (self.command, "⌘")] {
                if on {
                    parts.push(glyph.into());
                }
            }
        } else {
            for (on, label) in [(self.command || self.control, "Ctrl"), (self.option, "Alt"), (self.shift, "Shift")] {
                if on {
                    parts.push(label.into());
                }
            }
        }
        let key = match (self.key.as_str(), mac) {
            ("up", _) => "↑".to_string(),
            ("down", _) => "↓".into(),
            ("left", _) => "←".into(),
            ("right", _) => "→".into(),
            ("space", _) => "Space".into(),
            ("tab", true) => "⇥".into(),
            ("return", true) => "↩".into(),
            ("delete", true) => "⌫".into(),
            ("escape", true) => "⎋".into(),
            ("tab", false) => "Tab".into(),
            ("return", false) => "Enter".into(),
            ("delete", false) => "Backspace".into(),
            ("escape", false) => "Esc".into(),
            ("pageup", true) => "⇞".into(),
            ("pagedown", true) => "⇟".into(),
            ("pageup", false) => "PgUp".into(),
            ("pagedown", false) => "PgDn".into(),
            (k, _) => k.to_uppercase(),
        };
        parts.push(key);
        parts
    }

    /// Display as the platform writes shortcuts: `⇧⌘E` on macOS, `Ctrl+Shift+E` elsewhere.
    pub fn display(&self) -> String {
        self.parts().join(if cfg!(target_os = "macos") { "" } else { "+" })
    }

    /// GPUI keystroke syntax, e.g. `cmd-shift-n`; ⌘ maps to Ctrl off macOS.
    pub fn keystroke(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.control {
            parts.push("ctrl");
        }
        if self.option {
            parts.push("alt");
        }
        if self.shift {
            parts.push("shift");
        }
        if self.command {
            parts.push(if cfg!(target_os = "macos") { "cmd" } else { "ctrl" });
        }
        let key = match self.key.as_str() {
            "return" => "enter",
            "delete" => "backspace",
            k => k,
        };
        parts.push(key);
        parts.join("-")
    }
}
