use std::collections::BTreeSet;

use super::i18n::tr;
use super::prefs::{self, Shortcut};

/// Every action with a configurable shortcut. Defaults follow VS Code where it has an equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutAction {
    GoToday,
    GoGrid,
    GoRecord,
    GoInbox,
    GoNotes,
    GoCards,
    GoForward,
    ShowGraph,
    GoModule1,
    GoModule2,
    GoModule3,
    OpenOtherWorkspace,
    ToggleFullscreen,
    ReopenTab,
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
    Navigation,
    Documents,
    Editor,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::General, Group::Navigation, Group::Documents, Group::Editor];

    fn is_global(self) -> bool {
        matches!(self, Group::General | Group::Navigation)
    }

    pub fn label(self) -> String {
        tr(match self {
            Group::General => "Allgemein",
            Group::Navigation => "Navigation",
            Group::Documents => "Dokumente & Notizen",
            Group::Editor => "Editor",
        })
    }
}

fn s(key: &str, command: bool, shift: bool, option: bool, control: bool) -> Shortcut {
    Shortcut { key: key.into(), command, shift, option, control }
}

impl ShortcutAction {
    pub const ALL: [ShortcutAction; 47] = [
        Self::GoToday,
        Self::GoGrid,
        Self::GoRecord,
        Self::GoInbox,
        Self::GoNotes,
        Self::GoCards,
        Self::GoForward,
        Self::ShowGraph,
        Self::GoModule1,
        Self::GoModule2,
        Self::GoModule3,
        Self::OpenOtherWorkspace,
        Self::ToggleFullscreen,
        Self::ReopenTab,
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
            Self::GoToday => "goToday",
            Self::GoGrid => "goGrid",
            Self::GoRecord => "goRecord",
            Self::GoInbox => "goInbox",
            Self::GoNotes => "goNotes",
            Self::GoCards => "goCards",
            Self::GoForward => "goForward",
            Self::ShowGraph => "showGraph",
            Self::GoModule1 => "goModule1",
            Self::GoModule2 => "goModule2",
            Self::GoModule3 => "goModule3",
            Self::OpenOtherWorkspace => "openOtherWorkspace",
            Self::ToggleFullscreen => "toggleFullscreen",
            Self::ReopenTab => "reopenTab",
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

    /// The actions for the first modules in sidebar order.
    pub const MODULES: [ShortcutAction; 3] = [Self::GoModule1, Self::GoModule2, Self::GoModule3];

    /// The action for the module at `index` in sidebar order; later modules have none.
    pub fn for_module(index: usize) -> Option<ShortcutAction> {
        Self::MODULES.get(index).copied()
    }

    pub fn group(self) -> Group {
        match self {
            Self::GoToday | Self::GoGrid | Self::GoRecord | Self::GoInbox | Self::GoNotes | Self::GoCards | Self::GoForward | Self::ShowGraph | Self::GoModule1 | Self::GoModule2 | Self::GoModule3 | Self::OpenOtherWorkspace | Self::ToggleFullscreen | Self::GoBack | Self::SearchFiles => Group::Navigation,
            Self::Sync | Self::DailyNote | Self::OpenInObsidian | Self::QuickOpen | Self::NewEntry | Self::NewFile | Self::ToggleInspector | Self::Refresh => Group::General,
            Self::CloseTab | Self::ReopenTab | Self::NextTab | Self::PreviousTab | Self::TogglePreview | Self::SplitPreview | Self::ExportPdf => Group::Documents,
            _ => Group::Editor,
        }
    }

    pub fn label(self) -> String {
        tr(match self {
            Self::GoToday => "Übersicht",
            Self::GoGrid => "Semester",
            Self::GoRecord => "Leistungen",
            Self::GoInbox => "Inbox",
            Self::GoNotes => "Notizen",
            Self::GoCards => "Karten",
            Self::GoForward => "Vorwärts",
            Self::ShowGraph => "Graph zeigen",
            Self::GoModule1 => "Modul 1",
            Self::GoModule2 => "Modul 2",
            Self::GoModule3 => "Modul 3",
            Self::OpenOtherWorkspace => "Anderen Ordner öffnen…",
            Self::ToggleFullscreen => "Vollbild",
            Self::ReopenTab => "Geschlossenen Tab wieder öffnen",
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
                Self::GoForward => return s("right", false, false, true, false),
                Self::Refresh => return s("f5", false, false, false, false),
                Self::ToggleFullscreen => return s("f11", false, false, false, false),
                Self::ToggleInspector => return s("i", true, true, false, false),
                _ => {}
            }
        }
        match self {
            Self::GoToday => s("1", true, false, false, false),
            Self::GoGrid => s("2", true, false, false, false),
            Self::GoRecord => s("3", true, false, false, false),
            Self::GoInbox => s("4", true, false, false, false),
            Self::GoNotes => s("5", true, false, false, false),
            Self::GoCards => s("6", true, false, false, false),
            Self::GoForward => s("-", false, true, false, true),
            Self::ShowGraph => s("g", true, false, false, false),
            Self::GoModule1 => s("7", true, false, false, false),
            Self::GoModule2 => s("8", true, false, false, false),
            Self::GoModule3 => s("9", true, false, false, false),
            Self::OpenOtherWorkspace => s("o", true, true, false, false),
            Self::ToggleFullscreen => s("f", true, false, false, true),
            Self::ReopenTab => s("t", true, true, false, false),
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
            .filter(|o| *o != self && o.shortcut().as_ref() == Some(&mine) && (o.group() == self.group() || o.group().is_global() || self.group().is_global()))
            .collect()
    }
}

/// The same keystroke with modifiers in one fixed order, so `shift-cmd-f` and `cmd-shift-f` compare equal.
fn canonical(keystroke: &str) -> String {
    let (modifiers, key) = match keystroke.strip_suffix('-') {
        Some(rest) => (rest.strip_suffix('-').unwrap_or(rest), "-"),
        None => keystroke.rsplit_once('-').unwrap_or(("", keystroke)),
    };
    let mut modifiers: Vec<&str> = modifiers.split('-').filter(|m| !m.is_empty()).collect();
    modifiers.sort_unstable();
    modifiers.dedup();
    modifiers.into_iter().chain([key]).collect::<Vec<_>>().join("-")
}

impl ShortcutAction {
    /// The keystrokes of every effective app shortcut outside the editor.
    pub fn bound_keystrokes() -> BTreeSet<String> {
        Self::ALL.into_iter().filter(|a| a.group() != Group::Editor).filter_map(|a| a.shortcut()).map(|s| canonical(&s.keystroke())).collect()
    }
}

/// Keystrokes bound before but not now and not in `reserved`; they need an explicit unbinding because key bindings only accumulate.
pub fn released_keystrokes(before: &BTreeSet<String>, now: &BTreeSet<String>, reserved: &[String]) -> Vec<String> {
    let reserved: BTreeSet<String> = reserved.iter().map(|k| canonical(k)).collect();
    before.iter().filter(|k| !now.contains(*k) && !reserved.contains(*k)).cloned().collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn set(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|k| canonical(k)).collect()
    }

    #[test]
    fn only_keys_that_were_bound_and_are_free_now_are_released() {
        let before = set(&["cmd-1", "cmd-2", "cmd-shift-f", "ctrl-shift--"]);
        let now = set(&["cmd-2", "cmd-x"]);
        let released = released_keystrokes(&before, &now, &["shift-cmd-f".to_string()]);
        assert_eq!(released.len(), 2);
        assert!(released.contains(&canonical("cmd-1")) && released.contains(&canonical("ctrl-shift--")));
        assert!(released_keystrokes(&BTreeSet::new(), &now, &[]).is_empty(), "the first call releases nothing");
        assert!(released_keystrokes(&now, &now, &[]).is_empty());
    }

    #[test]
    fn modifier_order_does_not_matter() {
        assert_eq!(canonical("shift-cmd-f"), canonical("cmd-shift-f"));
        assert_eq!(canonical("f11"), "f11");
        assert_eq!(canonical("ctrl--"), "ctrl--");
        assert_eq!(canonical("-"), "-");
    }

    #[test]
    fn rebinding_a_page_to_a_module_key_is_a_conflict() {
        assert_eq!(ShortcutAction::for_module(0), Some(ShortcutAction::GoModule1));
        assert_eq!(ShortcutAction::for_module(3), None);
        for a in ShortcutAction::MODULES.into_iter().chain([ShortcutAction::OpenOtherWorkspace, ShortcutAction::ToggleFullscreen]) {
            assert_eq!(a.group(), Group::Navigation);
        }
        let seven = ShortcutAction::GoModule1.default_shortcut();
        ShortcutAction::GoNotes.set(Some(seven));
        let conflicts = ShortcutAction::GoNotes.conflicts();
        ShortcutAction::GoNotes.reset();
        assert_eq!(conflicts, vec![ShortcutAction::GoModule1]);
        assert!(ShortcutAction::GoNotes.conflicts().is_empty());
    }
}
