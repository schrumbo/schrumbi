use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::i18n::{backspace_name, ctrl_name, shift_name, tr, trf};
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
    FindInNote,
    RenameNote,
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
    ToggleWrap,
    ToggleLive,
    MarkText,
    RotatePagesLeft,
    RotatePagesRight,
    MovePagesUp,
    MovePagesDown,
    DeletePages,
    InsertPdf,
    PenTool,
    EraserTool,
    NoteTool,
    ExportMarkedPdf,
    PageList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    General,
    Navigation,
    View,
    Documents,
    Editor,
}

impl Group {
    pub const ALL: [Group; 5] = [Group::General, Group::Navigation, Group::View, Group::Documents, Group::Editor];

    fn is_global(self) -> bool {
        matches!(self, Group::General | Group::Navigation | Group::View)
    }

    pub fn label(self) -> String {
        tr(match self {
            Group::General => "Allgemein",
            Group::Navigation => "Navigation",
            Group::View => "Darstellung",
            Group::Documents => "Dokumente & Notizen",
            Group::Editor => "Editor",
        })
    }
}

fn s(key: &str, command: bool, shift: bool, option: bool, control: bool) -> Shortcut {
    Shortcut { key: key.into(), command, shift, option, control }
}

impl ShortcutAction {
    pub const ALL: [ShortcutAction; 63] = [
        Self::GoToday,
        Self::GoGrid,
        Self::GoRecord,
        Self::GoInbox,
        Self::GoNotes,
        Self::GoCards,
        Self::GoModule1,
        Self::GoModule2,
        Self::GoModule3,
        Self::GoBack,
        Self::GoForward,
        Self::ShowGraph,
        Self::ToggleFullscreen,
        Self::ReopenTab,
        Self::QuickOpen,
        Self::SearchFiles,
        Self::OpenOtherWorkspace,
        Self::NewEntry,
        Self::NewFile,
        Self::CloseTab,
        Self::NextTab,
        Self::PreviousTab,
        Self::TogglePreview,
        Self::SplitPreview,
        Self::ExportPdf,
        Self::FindInNote,
        Self::RenameNote,
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
        Self::ToggleWrap,
        Self::ToggleLive,
        Self::MarkText,
        Self::RotatePagesLeft,
        Self::RotatePagesRight,
        Self::MovePagesUp,
        Self::MovePagesDown,
        Self::DeletePages,
        Self::InsertPdf,
        Self::PenTool,
        Self::EraserTool,
        Self::NoteTool,
        Self::ExportMarkedPdf,
        Self::PageList,
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
            Self::FindInNote => "findInNote",
            Self::RenameNote => "renameNote",
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
            Self::ToggleWrap => "toggleWrap",
            Self::ToggleLive => "toggleLive",
            Self::MarkText => "markText",
            Self::RotatePagesLeft => "rotatePagesLeft",
            Self::RotatePagesRight => "rotatePagesRight",
            Self::MovePagesUp => "movePagesUp",
            Self::MovePagesDown => "movePagesDown",
            Self::DeletePages => "deletePages",
            Self::InsertPdf => "insertPdf",
            Self::PenTool => "penTool",
            Self::EraserTool => "eraserTool",
            Self::NoteTool => "noteTool",
            Self::ExportMarkedPdf => "exportMarkedPdf",
            Self::PageList => "pageList",
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
            Self::GoToday | Self::GoGrid | Self::GoRecord | Self::GoInbox | Self::GoNotes | Self::GoCards | Self::GoForward | Self::GoModule1 | Self::GoModule2 | Self::GoModule3 | Self::GoBack => Group::Navigation,
            Self::ShowGraph | Self::ToggleFullscreen => Group::View,
            Self::Sync | Self::DailyNote | Self::OpenInObsidian | Self::QuickOpen | Self::SearchFiles | Self::OpenOtherWorkspace | Self::NewEntry | Self::NewFile | Self::ToggleInspector | Self::Refresh => Group::General,
            Self::CloseTab | Self::ReopenTab | Self::NextTab | Self::PreviousTab | Self::TogglePreview | Self::SplitPreview | Self::ExportPdf | Self::FindInNote | Self::RenameNote | Self::ToggleLive | Self::MarkText | Self::RotatePagesLeft | Self::RotatePagesRight | Self::MovePagesUp | Self::MovePagesDown | Self::DeletePages | Self::InsertPdf | Self::PenTool | Self::EraserTool | Self::NoteTool | Self::ExportMarkedPdf | Self::PageList => Group::Documents,
            _ => Group::Editor,
        }
    }

    /// The sidebar position of the module this shortcut opens.
    pub fn module_index(self) -> Option<usize> {
        Self::MODULES.iter().position(|m| *m == self)
    }

    /// The label, or the real name of the module this shortcut opens when `modules` lists it in sidebar order.
    pub fn named(self, modules: &[String]) -> String {
        match self.module_index().and_then(|i| modules.get(i)) {
            Some(name) => name.clone(),
            None => self.label(),
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
            Self::FindInNote => "In Notiz suchen",
            Self::RenameNote => "Umbenennen…",
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
            Self::ToggleWrap => "Zeilenumbruch ein-/ausschalten",
            Self::ToggleLive => "Live-Ansicht ein-/ausschalten",
            Self::MarkText => "Markieren",
            Self::RotatePagesLeft => "Seiten links drehen",
            Self::RotatePagesRight => "Seiten rechts drehen",
            Self::MovePagesUp => "Seiten nach oben verschieben",
            Self::MovePagesDown => "Seiten nach unten verschieben",
            Self::DeletePages => "Seiten löschen",
            Self::InsertPdf => "Seiten aus PDF einfügen …",
            Self::PenTool => "Stift",
            Self::EraserTool => "Radierer",
            Self::NoteTool => "Notiz setzen",
            Self::ExportMarkedPdf => "Mit Markierungen exportieren",
            Self::PageList => "Seitenliste ein-/ausblenden",
        })
    }

    /// The keys the action has until the user changes them; `None` when it ships without any.
    pub fn default_shortcut(self) -> Option<Shortcut> {
        self.default_on(Os::here())
    }

    fn default_on(self, os: Os) -> Option<Shortcut> {
        if self == Self::ToggleComment {
            return None;
        }
        if os != Os::Mac {
            match self {
                Self::GoBack => return Some(s("left", false, false, true, false)),
                Self::GoForward => return Some(s("right", false, false, true, false)),
                Self::Refresh => return Some(s("f5", false, false, false, false)),
                Self::ToggleFullscreen => return Some(s("f11", false, false, false, false)),
                Self::ToggleInspector => return Some(s("i", true, true, false, false)),
                _ => {}
            }
        }
        Some(match self {
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
            Self::FindInNote => s("f", true, false, false, false),
            Self::RenameNote => s("f2", false, false, false, false),
            Self::Bold => s("b", true, false, false, false),
            Self::Italic => s("i", true, false, false, false),
            Self::Link => s("k", true, false, false, false),
            Self::InlineCode => s("c", true, true, false, false),
            Self::ToggleTask => s("return", true, false, false, false),
            Self::ToggleComment => return None,
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
            Self::ToggleWrap => s("z", true, false, true, false),
            Self::ToggleLive => s("l", true, false, true, false),
            Self::MarkText => s("h", false, false, false, false),
            Self::RotatePagesLeft => s("r", false, true, false, false),
            Self::RotatePagesRight => s("r", false, false, false, false),
            Self::MovePagesUp => s("up", false, false, true, false),
            Self::MovePagesDown => s("down", false, false, true, false),
            Self::DeletePages => s("delete", true, true, false, false),
            Self::InsertPdf => s("i", false, false, false, false),
            Self::PenTool => s("p", false, false, false, false),
            Self::EraserTool => s("e", false, false, false, false),
            Self::NoteTool => s("n", false, false, false, false),
            Self::ExportMarkedPdf => s("m", true, true, false, false),
            Self::PageList => s("b", true, true, false, false),
        })
    }

    /// The effective shortcut; `None` when the user cleared it.
    pub fn shortcut(self) -> Option<Shortcut> {
        prefs::with(|p| self.shortcut_in(p))
    }

    fn shortcut_in(self, p: &prefs::Prefs) -> Option<Shortcut> {
        if p.disabled_shortcuts.contains(self.id()) {
            return None;
        }
        p.shortcuts.get(self.id()).cloned().or_else(|| self.default_shortcut())
    }

    /// The shortcut that is really bound: the effective one unless it is reserved, e.g. in an old settings file.
    pub fn active_shortcut(self) -> Option<Shortcut> {
        self.shortcut().filter(|s| self.reserved_for(s).is_none())
    }

    /// Why the stored shortcut is not bound; `None` when it works.
    pub fn blocked(self) -> Option<String> {
        self.shortcut().and_then(|s| self.reserved_for(&s))
    }

    pub fn is_customized(self) -> bool {
        prefs::with(|p| p.shortcuts.contains_key(self.id()) || p.disabled_shortcuts.contains(self.id()))
    }

    /// Whether any shortcut differs from its default.
    pub fn any_customized() -> bool {
        prefs::with(|p| !p.shortcuts.is_empty() || !p.disabled_shortcuts.is_empty())
    }

    pub fn set(self, shortcut: Option<Shortcut>) {
        let default = self.default_shortcut();
        prefs::update(|p| match shortcut {
            Some(s) => {
                p.disabled_shortcuts.remove(self.id());
                if Some(&s) == default.as_ref() {
                    p.shortcuts.remove(self.id());
                } else {
                    p.shortcuts.insert(self.id().into(), s);
                }
            }
            None => {
                p.shortcuts.remove(self.id());
                if default.is_some() {
                    p.disabled_shortcuts.insert(self.id().into());
                } else {
                    p.disabled_shortcuts.remove(self.id());
                }
            }
        });
    }

    pub fn reset(self) {
        prefs::update(|p| {
            p.shortcuts.remove(self.id());
            p.disabled_shortcuts.remove(self.id());
        });
    }

    /// The custom and the cleared shortcuts as they are now.
    pub fn snapshot() -> Customized {
        prefs::with(|p| (p.shortcuts.clone(), p.disabled_shortcuts.clone()))
    }

    /// Drops every custom shortcut and returns them, so `restore` can bring them back.
    pub fn reset_all() -> Customized {
        let mut before = (BTreeMap::new(), BTreeSet::new());
        prefs::update(|p| {
            before = (std::mem::take(&mut p.shortcuts), std::mem::take(&mut p.disabled_shortcuts));
        });
        before
    }

    pub fn restore(custom: Customized) {
        prefs::update(|p| (p.shortcuts, p.disabled_shortcuts) = custom);
    }

    /// The keys as a tooltip suffix, e.g. ` (⌘N)`; empty when the user cleared the shortcut.
    pub fn hint(self) -> String {
        self.active_shortcut().map_or_else(String::new, |s| format!(" ({})", s.display()))
    }

    /// The names of what the app itself does with these keys, unless they are the action's own defaults.
    pub fn fixed_conflicts(self) -> Vec<String> {
        let Some(mine) = self.shortcut().filter(|m| Some(m) != self.default_shortcut().as_ref()) else { return Vec::new() };
        let mac = cfg!(target_os = "macos");
        let notes = matches!(self.group(), Group::Editor | Group::Documents);
        let mut names: Vec<String> = Vec::new();
        for f in FIXED.iter().filter(|f| f.key == mine.key && f.command == mine.command && f.shift == mine.shift && f.option == mine.option && !mine.control && (notes || !f.notes_only) && f.platform.here(mac)) {
            let name = tr(f.label);
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }

    /// Why these keys cannot be recorded for `self`: what they already do everywhere.
    pub fn reserved_for(self, keys: &Shortcut) -> Option<String> {
        self.reserved_on(keys, Os::here())
    }

    fn reserved_on(self, keys: &Shortcut, os: Os) -> Option<String> {
        if Some(keys) == self.default_on(os).as_ref() {
            return None;
        }
        keys.reserved_on(os)
    }

    /// Other actions using the same keys in an overlapping scope.
    pub fn conflicts(self) -> Vec<ShortcutAction> {
        Self::all_conflicts().remove(&self).unwrap_or_default()
    }

    /// The conflicts of every action that has one, from a single read of the settings.
    pub fn all_conflicts() -> HashMap<ShortcutAction, Vec<ShortcutAction>> {
        let keys: Vec<(ShortcutAction, Shortcut)> = prefs::with(|p| Self::ALL.into_iter().filter_map(|a| a.shortcut_in(p).map(|s| (a, s))).collect());
        let mut out: HashMap<ShortcutAction, Vec<ShortcutAction>> = HashMap::new();
        for (a, mine) in &keys {
            let others: Vec<ShortcutAction> = keys.iter().filter(|(o, s)| o != a && s == mine && (o.group() == a.group() || o.group().is_global() || a.group().is_global())).map(|(o, _)| *o).collect();
            if !others.is_empty() {
                out.insert(*a, others);
            }
        }
        out
    }
}

/// The desktop systems whose text fields bind different editing keys.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    fn here() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// Where a fixed key exists.
#[derive(Clone, Copy)]
enum Platform {
    All,
    Mac,
    Other,
}

impl Platform {
    fn here(self, mac: bool) -> bool {
        match self {
            Self::All => true,
            Self::Mac => mac,
            Self::Other => !mac,
        }
    }
}

/// A key the app binds on its own: the keys, the label of what it does, whether it only works in notes and where it exists.
struct Fixed {
    key: &'static str,
    command: bool,
    shift: bool,
    option: bool,
    label: &'static str,
    notes_only: bool,
    platform: Platform,
}

const fn fixed(key: &'static str, command: bool, shift: bool, option: bool, label: &'static str, notes_only: bool) -> Fixed {
    Fixed { key, command, shift, option, label, notes_only, platform: Platform::All }
}

const fn fixed_on(platform: Platform, key: &'static str, command: bool, shift: bool, option: bool, label: &'static str, notes_only: bool) -> Fixed {
    Fixed { key, command, shift, option, label, notes_only, platform }
}

const FIXED: [Fixed; 25] = [
    fixed("k", true, false, false, "Befehl suchen…", false),
    fixed("p", true, false, false, "Befehl suchen…", false),
    fixed("p", true, true, false, "Befehl suchen…", false),
    fixed("/", true, false, false, "Tastenkürzel anzeigen", false),
    fixed("7", true, true, false, "Tastenkürzel anzeigen", false),
    fixed("f1", false, false, false, "Schrumbi-Anleitung", false),
    fixed(",", true, false, false, "Einstellungen", false),
    fixed("s", true, false, false, "Sichern", false),
    fixed("w", true, true, false, "Fenster schließen", false),
    fixed("r", true, true, false, "Studienordner im Finder zeigen", false),
    fixed("m", true, false, false, "Im Dock ablegen", false),
    fixed("b", true, false, false, "Seitenleiste ein-/ausblenden", false),
    fixed("f", true, false, true, "In Notiz ersetzen", true),
    fixed_on(Platform::Other, "h", true, false, false, "In Notiz ersetzen", true),
    fixed_on(Platform::Mac, "h", true, false, false, "Schrumbi ausblenden", false),
    fixed_on(Platform::Mac, "h", true, false, true, "Andere ausblenden", false),
    fixed("h", true, true, false, "Markiert", true),
    fixed("m", true, true, false, "Formel", true),
    fixed("x", true, true, false, "Durchgestrichen", true),
    fixed(".", true, true, false, "Zitat", true),
    fixed("8", true, true, false, "Liste", true),
    fixed("7", true, true, false, "Nummerierte Liste", true),
    fixed("1", true, false, true, "Überschrift 1", true),
    fixed("2", true, false, true, "Überschrift 2", true),
    fixed("3", true, false, true, "Überschrift 3", true),
];

/// The custom and the cleared shortcuts, as `reset_all` took them away.
pub type Customized = (BTreeMap<String, Shortcut>, BTreeSet<String>);

/// A key binding as the keystroke and the name of the action it triggers.
pub type Bound = (String, String);

/// Bindings present before but not now; they need an explicit unbinding because key bindings only accumulate.
pub fn released_bindings(before: &BTreeSet<Bound>, now: &BTreeSet<Bound>) -> Vec<Bound> {
    before.difference(now).cloned().collect()
}

impl Shortcut {
    /// What the system and every text field do with these keys, e.g. "Reserviert: Kopieren".
    pub fn reserved_for(&self) -> Option<String> {
        self.reserved_on(Os::here())
    }

    fn reserved_on(&self, os: Os) -> Option<String> {
        let what = if self.key == "tab" && !self.command && !self.option && !self.control {
            "Tastaturnavigation"
        } else if self.edits_text(os) || (self.key == "return" && self.shift && !self.command && !self.option && !self.control) {
            "Textbearbeitung"
        } else if self.command && !self.option && !self.control {
            match (self.key.as_str(), self.shift) {
                ("c", false) => "Kopieren",
                ("v", false) => "Einsetzen",
                ("x", false) => "Ausschneiden",
                ("a", false) => "Alles auswählen",
                ("z", false) => "Rückgängig",
                ("z", true) => "Wiederholen",
                ("y", false) if os != Os::Mac => "Wiederholen",
                ("w", false) => "Tab schließen",
                ("q", false) => "Beenden",
                _ => return None,
            }
        } else {
            return None;
        };
        Some(trf("Reserviert: %@", &[&tr(what)]))
    }

    fn edits_text(&self, os: Os) -> bool {
        let key = self.key.as_str();
        if self.control {
            return os == Os::Mac && !self.command && !self.option && (matches!(key, "a" | "e") || (key == "delete" && !self.shift));
        }
        let horizontal = matches!(key, "left" | "right");
        let vertical = matches!(key, "up" | "down");
        let edge = matches!(key, "home" | "end");
        let delete = key == "delete" && !self.shift;
        match (self.command, self.option) {
            (false, false) => self.shift && (horizontal || vertical || edge || key == "delete"),
            (true, false) => match os {
                Os::Mac => horizontal || vertical || delete,
                Os::Windows | Os::Linux => horizontal || edge || delete,
            },
            (false, true) => match os {
                Os::Mac => horizontal || delete,
                Os::Windows => self.shift && horizontal,
                Os::Linux => self.shift && (horizontal || vertical),
            },
            (true, true) => os != Os::Linux && vertical && !self.shift,
        }
    }

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
            for (on, label) in [(self.command || self.control, ctrl_name()), (self.option, "Alt"), (self.shift, shift_name())] {
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
            ("home", _) => "Home".into(),
            ("end", _) => "End".into(),
            ("tab", true) => "⇥".into(),
            ("return", true) => "↩".into(),
            ("delete", true) => "⌫".into(),
            ("escape", true) => "⎋".into(),
            ("tab", false) => "Tab".into(),
            ("return", false) => "Enter".into(),
            ("delete", false) => backspace_name().into(),
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

    fn bound(pairs: &[(&str, &str)]) -> BTreeSet<Bound> {
        pairs.iter().map(|(k, a)| (k.to_string(), a.to_string())).collect()
    }

    #[test]
    fn only_bindings_that_are_gone_or_moved_are_released() {
        let before = bound(&[("ctrl-1", "GoToday"), ("ctrl-2", "GoGrid"), ("ctrl-b", "Bold"), ("ctrl-q", "Quit")]);
        let now = bound(&[("ctrl-2", "GoGrid"), ("ctrl-shift-b", "Bold"), ("ctrl-q", "Quit")]);
        let released = released_bindings(&before, &now);
        assert_eq!(released, vec![("ctrl-1".to_string(), "GoToday".to_string()), ("ctrl-b".to_string(), "Bold".to_string())]);
        assert!(released_bindings(&BTreeSet::new(), &now).is_empty(), "the first call releases nothing");
        assert!(released_bindings(&now, &now).is_empty());
    }

    #[test]
    fn a_key_taken_by_another_action_releases_only_the_old_one() {
        let before = bound(&[("ctrl-z", "SyncNow"), ("ctrl-z", "Undo")]);
        let now = bound(&[("ctrl-z", "Undo")]);
        assert_eq!(released_bindings(&before, &now), vec![("ctrl-z".to_string(), "SyncNow".to_string())]);
    }

    #[test]
    fn navigation_lists_pages_modules_then_history() {
        let nav: Vec<ShortcutAction> = ShortcutAction::ALL.into_iter().filter(|a| a.group() == Group::Navigation).collect();
        use ShortcutAction::*;
        assert_eq!(nav, vec![GoToday, GoGrid, GoRecord, GoInbox, GoNotes, GoCards, GoModule1, GoModule2, GoModule3, GoBack, GoForward]);
    }

    #[test]
    fn modules_show_their_real_name() {
        let names = vec!["Analysis".to_string(), "Physik".to_string()];
        assert_eq!(ShortcutAction::GoModule2.named(&names), "Physik");
        assert_eq!(ShortcutAction::GoModule3.named(&names), ShortcutAction::GoModule3.label());
        assert_eq!(ShortcutAction::GoToday.named(&names), ShortcutAction::GoToday.label());
    }

    #[test]
    fn rebinding_a_page_to_a_module_key_is_a_conflict() {
        assert_eq!(ShortcutAction::for_module(0), Some(ShortcutAction::GoModule1));
        assert_eq!(ShortcutAction::for_module(3), None);
        for a in ShortcutAction::MODULES {
            assert_eq!(a.group(), Group::Navigation);
        }
        assert_eq!(ShortcutAction::OpenOtherWorkspace.group(), Group::General);
        assert_eq!(ShortcutAction::ToggleFullscreen.group(), Group::View);
        let seven = ShortcutAction::GoModule1.default_shortcut();
        ShortcutAction::GoNotes.set(seven);
        let conflicts = ShortcutAction::GoNotes.conflicts();
        ShortcutAction::GoNotes.reset();
        assert_eq!(conflicts, vec![ShortcutAction::GoModule1]);
        assert!(ShortcutAction::GoNotes.conflicts().is_empty());
    }

    #[test]
    fn copy_paste_and_tab_cannot_be_taken() {
        let k = |key: &str, shift: bool| s(key, true, shift, false, false);
        for key in ["c", "v", "x", "a", "z", "w", "q"] {
            assert!(ShortcutAction::Bold.reserved_for(&k(key, false)).is_some(), "{key}");
        }
        assert!(ShortcutAction::Bold.reserved_for(&k("z", true)).is_some());
        assert_eq!(ShortcutAction::Bold.reserved_for(&k("y", false)).is_some(), !cfg!(target_os = "macos"));
        assert!(ShortcutAction::Bold.reserved_for(&s("tab", false, false, false, false)).is_some());
        assert!(ShortcutAction::Bold.reserved_for(&s("tab", false, true, false, false)).is_some());
        assert!(ShortcutAction::Bold.reserved_for(&s("tab", false, false, false, true)).is_none(), "Ctrl+Tab stays free");
        assert!(ShortcutAction::Bold.reserved_for(&k("c", true)).is_none(), "Ctrl+Shift+C is not copy");
        assert!(ShortcutAction::Bold.reserved_for(&k("b", false)).is_none());
        assert!(ShortcutAction::CloseTab.reserved_for(&k("w", false)).is_none(), "an action may keep its own default");
    }

    fn edits(os: Os, key: &str, command: bool, shift: bool, option: bool) -> bool {
        s(key, command, shift, option, false).edits_text(os)
    }

    #[test]
    fn text_editing_keys_are_what_the_input_binds_on_macos() {
        let mac = |key: &str, command, shift, option| edits(Os::Mac, key, command, shift, option);
        for shift in [false, true] {
            for key in ["left", "right"] {
                assert!(mac(key, true, shift, false), "cmd {key} {shift}");
                assert!(mac(key, false, shift, true), "alt {key} {shift}");
            }
            for key in ["up", "down"] {
                assert!(mac(key, true, shift, false), "cmd {key} {shift}");
                assert!(!mac(key, false, shift, true), "alt {key} {shift}");
                assert_eq!(mac(key, true, shift, true), !shift, "cmd alt {key} {shift}");
            }
        }
        for key in ["up", "down", "left", "right", "home", "end"] {
            assert!(mac(key, false, true, false), "shift {key}");
        }
        assert!(mac("delete", true, false, false) && mac("delete", false, false, true));
        assert!(!mac("home", true, false, false) && !mac("end", true, true, false));
        assert!(!mac("left", false, false, false));
        assert!(!mac("return", true, false, false) && !mac("b", false, true, false));
    }

    #[test]
    fn text_editing_keys_are_what_the_input_binds_on_windows() {
        let win = |key: &str, command, shift, option| edits(Os::Windows, key, command, shift, option);
        for shift in [false, true] {
            for key in ["left", "right", "home", "end"] {
                assert!(win(key, true, shift, false), "ctrl {key} {shift}");
            }
            for key in ["up", "down"] {
                assert!(!win(key, true, shift, false), "ctrl {key} {shift}");
                assert_eq!(win(key, true, shift, true), !shift, "ctrl alt {key} {shift}");
                assert!(!win(key, false, shift, true), "alt {key} {shift}");
            }
        }
        for key in ["up", "down", "left", "right", "home", "end"] {
            assert!(win(key, false, true, false), "shift {key}");
        }
        for key in ["left", "right"] {
            assert!(win(key, false, true, true), "alt shift {key}");
            assert!(!win(key, false, false, true), "alt {key}");
        }
        assert!(win("delete", true, false, false));
        assert!(!win("delete", false, false, true));
        assert!(!win("return", true, false, false) && !win("b", true, true, false));
    }

    #[test]
    fn text_editing_keys_are_what_the_input_binds_on_linux() {
        let linux = |key: &str, command, shift, option| edits(Os::Linux, key, command, shift, option);
        for shift in [false, true] {
            for key in ["left", "right", "home", "end"] {
                assert!(linux(key, true, shift, false), "ctrl {key} {shift}");
            }
            for key in ["up", "down"] {
                assert!(!linux(key, true, shift, false), "ctrl {key} {shift}");
                assert!(!linux(key, true, shift, true), "ctrl alt {key} {shift}");
            }
        }
        for key in ["left", "right"] {
            assert!(linux(key, false, true, true), "alt shift {key}");
            assert!(!linux(key, false, false, true), "alt {key}");
        }
        for key in ["up", "down", "left", "right", "home", "end"] {
            assert!(linux(key, false, true, false), "shift {key}");
        }
        assert!(linux("delete", true, false, false));
        assert!(!linux("delete", false, false, true));
    }

    #[test]
    fn the_kits_remaining_editing_keys_are_reserved_per_system() {
        let ctrl = |key: &str, shift: bool| s(key, false, shift, false, true).edits_text(Os::Mac);
        assert!(ctrl("a", false) && ctrl("e", false) && ctrl("a", true) && ctrl("e", true) && ctrl("delete", false));
        assert!(!ctrl("b", false) && !ctrl("delete", true));
        for os in [Os::Windows, Os::Linux] {
            assert!(!s("a", false, false, false, true).edits_text(os));
        }
        for os in [Os::Mac, Os::Windows, Os::Linux] {
            assert!(edits(os, "delete", false, true, false), "shift backspace and delete");
        }
        assert!(edits(Os::Linux, "up", false, true, true) && edits(Os::Linux, "down", false, true, true));
        assert!(!edits(Os::Windows, "up", false, true, true) && !edits(Os::Mac, "up", false, true, true));
    }

    #[test]
    fn no_default_shortcut_is_taken_by_a_text_field() {
        for os in [Os::Mac, Os::Windows, Os::Linux] {
            for action in ShortcutAction::ALL {
                let Some(keys) = action.default_on(os) else { continue };
                let reserved = keys.reserved_on(os);
                let text_editing = trf("Reserviert: %@", &[&tr("Textbearbeitung")]);
                let own_name = trf("Reserviert: %@", &[&action.label()]);
                let editor_override = action.group() == Group::Editor && reserved.as_deref() == Some(text_editing.as_str());
                let same_command = reserved.as_deref() == Some(own_name.as_str());
                assert!(reserved.is_none() || editor_override || same_command, "{} {} {:?}", action.label(), keys.display(), reserved);
                if action.group() != Group::Editor {
                    assert!(!keys.edits_text(os), "{} {}", action.label(), keys.display());
                }
            }
        }
    }

    #[test]
    fn shift_return_and_the_systems_own_keys_stay_reserved() {
        let key = |k: &str, command: bool, shift: bool, option: bool| s(k, command, shift, option, false);
        assert!(ShortcutAction::Bold.reserved_for(&key("return", false, true, false)).is_some());
        assert!(ShortcutAction::Bold.reserved_for(&key("return", true, false, false)).is_none());
        assert!(ShortcutAction::Bold.reserved_for(&key("left", false, true, false)).is_some());
        assert!(ShortcutAction::MoveLineUp.reserved_for(&key("up", false, false, true)).is_none(), "an action may keep its own default");
    }

    #[test]
    fn a_reserved_stored_shortcut_is_not_bound() {
        let keys = s("c", true, false, false, false);
        ShortcutAction::ZoomIn.set(Some(keys));
        let (active, blocked) = (ShortcutAction::ZoomIn.active_shortcut(), ShortcutAction::ZoomIn.blocked());
        ShortcutAction::ZoomIn.reset();
        assert!(active.is_none() && blocked.is_some());
        assert!(ShortcutAction::ZoomIn.active_shortcut().is_some() && ShortcutAction::ZoomIn.blocked().is_none());
    }

    #[test]
    fn the_editors_own_keys_are_named() {
        ShortcutAction::ZoomOut.set(Some(s("h", true, true, false, false)));
        let fixed = ShortcutAction::ZoomOut.fixed_conflicts();
        ShortcutAction::ZoomOut.reset();
        assert_eq!(fixed.len(), 1);
        let dock = s("m", true, true, false, false);
        ShortcutAction::ZoomOut.set(Some(dock));
        let fixed = ShortcutAction::ZoomOut.fixed_conflicts();
        ShortcutAction::ZoomOut.reset();
        assert_eq!(fixed.len(), 1);
    }

    #[test]
    fn f2_is_the_rename_actions_own_key_and_not_a_fixed_one() {
        let f2 = s("f2", false, false, false, false);
        assert_eq!(ShortcutAction::RenameNote.default_shortcut(), Some(f2.clone()));
        ShortcutAction::ZoomOut.set(Some(f2));
        let fixed = ShortcutAction::ZoomOut.fixed_conflicts();
        ShortcutAction::ZoomOut.reset();
        assert!(fixed.is_empty());
    }

    #[test]
    fn a_key_of_the_app_names_what_it_does() {
        let sidebar = s("b", true, false, false, false);
        ShortcutAction::Italic.set(Some(sidebar));
        let fixed = ShortcutAction::Italic.fixed_conflicts();
        ShortcutAction::Italic.reset();
        assert_eq!(fixed.len(), 1);
        assert!(ShortcutAction::Bold.fixed_conflicts().is_empty(), "the default of Bold is meant to win in notes");
        assert!(ShortcutAction::Italic.fixed_conflicts().is_empty());
    }

    #[test]
    fn commenting_ships_without_keys_but_can_be_bound() {
        assert_eq!(ShortcutAction::ToggleComment.default_shortcut(), None);
        assert_eq!(ShortcutAction::ToggleComment.shortcut(), None);
        assert!(!ShortcutAction::ToggleComment.is_customized());
        let keys = s("j", true, true, false, false);
        ShortcutAction::ToggleComment.set(Some(keys.clone()));
        assert_eq!(ShortcutAction::ToggleComment.shortcut(), Some(keys));
        assert!(ShortcutAction::ToggleComment.is_customized());
        ShortcutAction::ToggleComment.set(None);
        assert_eq!(ShortcutAction::ToggleComment.shortcut(), None);
        assert!(!ShortcutAction::ToggleComment.is_customized());
    }
}
