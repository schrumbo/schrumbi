use std::path::{Path, PathBuf};

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::{self, act};
use crate::core::i18n::{key_name, tr};
use crate::core::library::{Library, NewNoteRequest, Route};
use crate::core::prefs;
use crate::core::text::end_ellipsis;
use crate::core::shortcuts::{Bound, ShortcutAction, released_bindings};

actions!(
    schrumbi,
    [
        Quit,
        About,
        CheckForUpdates,
        OpenSettings,
        Undo,
        Redo,
        CommandPalette,
        ShowGraph,
        ShowShortcuts,
        ShowHelp,
        NewEntry,
        NewFile,
        QuickOpen,
        ImportCalendar,
        ExportCalendar,
        ExportArchive,
        RevealWorkspace,
        OpenOtherWorkspace,
        Save,
        CloseTab,
        ReopenTab,
        CloseWindow,
        TogglePreview,
        SplitPreview,
        ExportPdf,
        ToggleInspector,
        Refresh,
        SyncNow,
        DailyNote,
        OpenInObsidian,
        SyncSetup,
        ResolveConflicts,
        GoBack,
        GoForward,
        GoToday,
        GoGrid,
        GoRecord,
        GoInbox,
        GoCards,
        GoNotes,
        SearchFiles,
        NextTab,
        PreviousTab,
        Minimize,
        ToggleFullscreen,
        Zoom,
        Hide,
        HideOthers,
        ShowAll,
        RenameNote
    ]
);

/// Opens a recent workspace by its index in the settings list.
#[derive(Clone, PartialEq, Debug, serde::Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = schrumbi)]
pub struct OpenRecent(pub usize);

/// Shows the module at this position of the sidebar.
#[derive(Clone, PartialEq, Debug, serde::Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = schrumbi)]
pub struct GoModuleAt(pub usize);

/// Shows a module page by id.
#[derive(Clone, PartialEq, Debug, serde::Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = schrumbi)]
pub struct GoModule(pub String);

fn route(r: Route) -> impl Fn(&mut App) {
    move |cx| act(cx, |l| l.set_route(r.clone()))
}

pub fn init(cx: &mut App) {
    gpui_kit::component::kbd::set_relabel(key_name);
    cx.on_action(|_: &Quit, cx| quit(cx));
    cx.on_action(|_: &OpenSettings, cx| super::settings::open(cx));
    cx.on_action(|_: &About, cx| super::settings::open_about(cx));
    cx.on_action(|_: &CheckForUpdates, cx| super::updates::check(true, cx));
    cx.on_action(|_: &OpenInObsidian, cx| {
        let open = state::read(cx, |l| l.active_document.clone().filter(|_| matches!(l.route(), crate::core::library::Route::Doc(_))));
        if let Some(path) = open.filter(|p| super::vault_tools::in_vault(p, cx)) {
            super::vault_tools::open_in_obsidian(&path, cx);
        }
    });
    cx.on_action(|_: &DailyNote, cx| {
        act(cx, |l| l.open_daily_note());
        state::reindex(cx);
    });
    cx.on_action(|_: &SyncNow, cx| {
        if super::sync::get(cx).git.is_some() {
            let text = super::sync::get(cx).message.clone();
            super::sync::run(Some(text), true, cx);
        } else if state::read(cx, |l| l.root.is_some()) {
            act(cx, |l| l.show_sync_setup = true);
        }
    });
    cx.on_action(|_: &SyncSetup, cx| act(cx, |l| if l.root.is_some() { l.show_sync_setup = true }));
    cx.on_action(|_: &ResolveConflicts, cx| {
        if !super::sync::get(cx).conflicts.is_empty() {
            act(cx, |l| l.show_conflicts = true);
        }
    });
    quit_on_terminate(cx);
    cx.on_action(|_: &Undo, cx| act(cx, |l| l.undo()));
    cx.on_action(|_: &Redo, cx| act(cx, |l| l.redo()));
    cx.on_action(|_: &ShowGraph, cx| {
        act(cx, |l| if l.root.is_some() { l.show_graph(None) })
    });
    cx.on_action(|_: &CommandPalette, cx| act(cx, |l| if l.root.is_some() { l.show_palette = true }));
    cx.on_action(|_: &ShowShortcuts, cx| act(cx, |l| if l.root.is_some() { l.show_shortcuts = true }));
    cx.on_action(|_: &ShowHelp, cx| cx.defer(|cx| super::help::open(None, cx)));
    cx.on_action(|_: &NewEntry, cx| act(cx, |l| if l.root.is_some() { l.show_quick_add = true }));
    cx.on_action(|_: &NewFile, cx| {
        act(cx, |l| {
            if l.root.is_some() {
                let module_id = if let Route::Module(id) = l.route() { Some(id.clone()) } else { None };
                if module_id.is_none() {
                    if let Some(made) = l.new_untitled_note(None) {
                        l.rename_request = Some(made);
                        l.quiet_open = true;
                    }
                    return;
                }
                l.new_note = Some(NewNoteRequest { module_id, folder: None });
            }
        })
    });
    cx.on_action(|_: &QuickOpen, cx| act(cx, |l| if l.root.is_some() { l.show_quick_open = true }));
    cx.on_action(|_: &ImportCalendar, cx| super::platform::pick_calendar_import(cx));
    cx.on_action(|_: &ExportCalendar, cx| super::platform::export_calendar(cx));
    cx.on_action(|_: &ExportArchive, cx| super::platform::export_archive(cx));
    cx.on_action(|_: &RevealWorkspace, cx| {
        if let Some(r) = state::read(cx, |l| l.root.clone()) {
            crate::core::library::open_path(&r);
        }
    });
    cx.on_action(|_: &OpenOtherWorkspace, cx| {
        super::platform::choose_folder(cx, |path, cx| {
            state::close_workspace(cx);
            state::open_workspace(&path, cx);
        })
    });
    cx.on_action(|a: &OpenRecent, cx| {
        if let Some(p) = prefs::get().recent_workspaces.get(a.0).cloned() {
            if p.is_dir() {
                state::close_workspace(cx);
                state::open_workspace(&p, cx);
            } else {
                prefs::forget_workspace(&p);
                act(cx, |l| l.last_error = Some(crate::core::i18n::trf("%@ ist nicht mehr erreichbar.", &[&p.file_name().unwrap_or_default().to_string_lossy()])));
                rebuild_menus(cx);
            }
        }
    });
    cx.on_action(|_: &ToggleInspector, cx| act(cx, |l| l.toggle_inspector()));
    cx.on_action(|_: &Refresh, cx| {
        act(cx, |l| l.refresh());
        state::reindex(cx);
    });
    cx.on_action(|_: &GoBack, cx| act(cx, |l| l.go_back()));
    cx.on_action(|_: &ReopenTab, cx| act(cx, |l| l.reopen_document()));
    cx.on_action(|_: &GoForward, cx| act(cx, |l| l.go_forward()));
    cx.on_action(|_: &GoToday, cx| route(Route::Today)(cx));
    cx.on_action(|_: &GoGrid, cx| route(Route::Grid)(cx));
    cx.on_action(|_: &GoRecord, cx| route(Route::Record)(cx));
    cx.on_action(|_: &GoInbox, cx| route(Route::Inbox)(cx));
    cx.on_action(|_: &GoCards, cx| route(Route::Cards)(cx));
    cx.on_action(|_: &GoNotes, cx| act(cx, |l| l.show_notes()));
    cx.on_action(|_: &SearchFiles, cx| route(Route::Search)(cx));
    cx.on_action(|a: &GoModule, cx| route(Route::Module(a.0.clone()))(cx));
    cx.on_action(|a: &GoModuleAt, cx| {
        if let Some(id) = state::read(cx, |l| l.semester_modules().get(a.0).map(|m| m.id.clone())) {
            route(Route::Module(id))(cx);
        }
    });
    cx.on_action(|_: &CloseTab, cx| {
        let shown = state::read(cx, |l| if let Route::Doc(p) = l.route() { Some(p.clone()) } else { None });
        let main = super::main_window::is_active(cx);
        match shown.filter(|_| main) {
            Some(path) => super::documents::close_tab(&path, cx),
            None => close_active_window(cx),
        }
    });
    cx.on_action(|_: &CloseWindow, cx| close_active_window(cx));
    cx.on_action(|_: &NextTab, cx| act(cx, |l| l.cycle_document(1)));
    cx.on_action(|_: &PreviousTab, cx| act(cx, |l| l.cycle_document(-1)));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_window_closed(|cx, closed| {
        note_window_front(closed, false, cx);
        let keep_running = cfg!(target_os = "macos") && !super::main_window::is_main(closed, cx);
        if cx.windows().is_empty() && !keep_running && !(super::tray::AVAILABLE && prefs::get().tray) {
            quit(cx);
        }
    })
    .detach();
    bind_keys(cx);
    rebuild_menus(cx);
    let lib = state::lib(cx);
    let mut modules: Vec<String> = Vec::new();
    let mut note = (note_is_active(cx), find_is_available(cx), rename_is_available(cx));
    cx.observe(&lib, move |lib, cx| {
        if let Some(path) = lib.update(cx, |l, _| l.raise_note_window.take()) {
            cx.defer(move |cx| super::note_window::open(&path, cx));
        }
        let now: Vec<String> = lib.read(cx).semester_modules().iter().map(|m| format!("{}{}", m.id, m.display_name())).collect();
        let open = (note_is_active(cx), find_is_available(cx), rename_is_available(cx));
        if now != modules || open != note {
            modules = now;
            note = open;
            rebuild_menus(cx);
        }
    })
    .detach();
}

const MENU_NAME: usize = 28;

#[derive(Default)]
struct NoteWindowFront(Option<WindowId>);

impl Global for NoteWindowFront {}

/// Remembers which separate note window is frontmost, so the Edit menu offers find and replace for it.
pub fn note_window_front(id: WindowId, front: bool, cx: &mut App) {
    let now = cx.default_global::<NoteWindowFront>();
    let next = match (front, now.0) {
        (true, _) => Some(id),
        (false, Some(open)) if open == id => None,
        (false, open) => open,
    };
    if now.0 != next {
        now.0 = next;
        rebuild_menus(cx);
    }
}

/// Whether a separate note window or the pane of the main window that has the focus shows a note, which find and replace then act on.
pub fn note_is_active(cx: &App) -> bool {
    cx.try_global::<NoteWindowFront>().is_some_and(|f| f.0.is_some()) || state::read(cx, |l| matches!(l.route(), Route::Doc(_)) && l.focused_document().is_some_and(crate::core::markdown::is_markdown))
}

/// What Rename acts on: the shown note or file, else the row the explorer has selected.
pub fn rename_target(l: &Library) -> Option<PathBuf> {
    match l.route() {
        Route::Doc(_) => l.focused_document().map(Path::to_path_buf),
        Route::Notes => l.explorer_selection.clone(),
        _ => None,
    }
}

/// Whether there is something Rename acts on.
pub fn rename_is_available(cx: &App) -> bool {
    state::read(cx, |l| rename_target(l).is_some())
}

/// Asks the explorer to rename `path`, showing it first when it was hidden.
pub fn rename_file(path: PathBuf, cx: &mut App) {
    prefs::update(|p| p.notes_explorer_hidden = false);
    act(cx, |l| {
        l.rename_request = Some(path);
        l.quiet_open = true;
    });
}

/// Hands an action meant for the open note to it when the focus is elsewhere in the main window, like the explorer or the tab strip.
/// Returns false when the focused pane is no note, so the action can go on to a PDF or sheet.
pub fn in_active_note(action: impl Action, cx: &mut App) -> bool {
    if !note_is_active(cx) {
        return false;
    }
    cx.default_global::<super::palette::PendingNoteAction>().0 = Some(Box::new(action));
    act(cx, |l| if l.beside_active && l.split_document.is_some() { l.focus_beside = true } else { l.focus_document = true });
    true
}

fn searchable(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) || crate::core::office::kind(path).is_some()
}

fn find_panes(l: &crate::core::library::Library) -> Vec<(bool, bool)> {
    if !matches!(l.route(), Route::Doc(_)) {
        return Vec::new();
    }
    let left = l.active_document.clone();
    let right = l.split_document.clone().filter(|r| Some(r) != left.as_ref());
    let findable = |p: &Option<PathBuf>| p.as_deref().is_some_and(|p| crate::core::markdown::is_markdown(p) || searchable(p));
    let panes = [(findable(&left), false), (findable(&right), true)];
    if l.beside_active && right.is_some() { vec![panes[1], panes[0]] } else { panes.to_vec() }
}

/// Whether Find has a note, PDF or sheet to go to in the main window, in the focused pane or else the other one.
pub fn find_is_available(cx: &App) -> bool {
    note_is_active(cx) || state::read(cx, |l| find_panes(l).into_iter().any(|(findable, _)| findable))
}

/// Opens find in the focused PDF or sheet pane, or in the other pane when the focused one has nothing to search.
/// Returns false when no pane offers it.
pub fn find_in_pane(cx: &mut App) -> bool {
    let Some((_, beside)) = state::read(cx, find_panes).into_iter().find(|(findable, _)| *findable) else { return false };
    cx.default_global::<super::palette::PendingNoteAction>().0 = Some(Box::new(super::find_bar::FindInNote));
    act(cx, |l| {
        l.beside_active = beside;
        if beside { l.focus_beside = true } else { l.focus_document = true }
    });
    true
}

fn close_active_window(cx: &mut App) {
    if let Some(w) = cx.active_window() {
        cx.defer(move |cx| {
            let _ = w.update(cx, |_, window, _| window.remove_window());
        });
    }
}

/// Saves or asks about unsaved notes, then quits.
/// Being told to end by the system saves like quitting does, so what was just typed stays.
#[cfg(unix)]
fn quit_on_terminate(cx: &mut App) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static ASKED: AtomicBool = AtomicBool::new(false);
    extern "C" fn asked(_: libc::c_int) {
        ASKED.store(true, Ordering::Relaxed);
    }
    let handler: extern "C" fn(libc::c_int) = asked;
    unsafe {
        libc::signal(libc::SIGTERM, handler as libc::sighandler_t);
    }
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
            if ASKED.swap(false, Ordering::Relaxed) {
                cx.update(quit);
            }
        }
    })
    .detach();
}

#[cfg(not(unix))]
fn quit_on_terminate(_: &mut App) {}

pub fn quit(cx: &mut App) {
    super::documents::prepare_to_quit(cx);
}

/// Key bindings need static strings; they are rebuilt only when shortcuts change.
pub fn kb<A: Action>(keys: String, a: A, context: Option<&str>) -> KeyBinding {
    KeyBinding::new(Box::leak(keys.into_boxed_str()), a, context)
}

fn binding<A: Action>(action: ShortcutAction, a: A, context: Option<&str>) -> Option<KeyBinding> {
    action.active_shortcut().map(|s| kb(s.keystroke(), a, context))
}

/// `cmd-…` on macOS, `ctrl-…` elsewhere.
pub fn cmd(k: &str) -> String {
    format!("{}-{k}", if cfg!(target_os = "macos") { "cmd" } else { "ctrl" })
}

static BOUND: std::sync::Mutex<std::collections::BTreeSet<Bound>> = std::sync::Mutex::new(std::collections::BTreeSet::new());

fn bound_pairs(bindings: &[KeyBinding]) -> std::collections::BTreeSet<Bound> {
    let os = if cfg!(target_os = "macos") {
        OperatingSystem::Mac
    } else if cfg!(windows) {
        OperatingSystem::Windows
    } else {
        OperatingSystem::Linux
    };
    bindings
        .iter()
        .map(|b| (b.keystrokes().iter().map(|k| k.inner().unparse(os)).collect::<Vec<_>>().join(" "), b.action().name().to_string()))
        .collect()
}

/// Binds the app's keys on top of the component library's, using the user's custom shortcuts.
/// Later bindings win, so rebinding after a settings change overrides the earlier ones.
pub fn bind_keys(cx: &mut App) {
    let mut b: Vec<KeyBinding> = Vec::new();
    b.extend([
        kb(cmd("q"), Quit, None),
        kb(cmd("k"), CommandPalette, None),
        kb(cmd("p"), CommandPalette, None),
        kb(cmd("shift-p"), CommandPalette, None),
        kb(cmd("shift-7"), ShowShortcuts, None),
        kb(cmd("/"), ShowShortcuts, None),
        kb("f1".into(), ShowHelp, None),
        kb(cmd(","), OpenSettings, None),
        kb(cmd("x"), gpui_kit::component::input::Cut, Some("Root")),
        kb(cmd("v"), gpui_kit::component::input::Paste, Some("Root")),
        kb(cmd("a"), gpui_kit::component::input::SelectAll, Some("Root")),
        kb(cmd("z"), Undo, Some("MainWindow")),
        kb(cmd("shift-z"), Redo, Some("MainWindow")),
        kb(cmd("s"), Save, None),
        kb(cmd("shift-w"), CloseWindow, None),
        kb(cmd("shift-r"), RevealWorkspace, None),
        kb(cmd("m"), Minimize, None),
        kb(cmd("enter"), super::filing_form::CommitFiling, Some("FilingForm")),
        kb(cmd("b"), super::main_window::ToggleSidebar, Some("MainWindow")),
        kb("down".into(), super::quick::QuickDown, Some("QuickOpen")),
        kb("up".into(), super::quick::QuickUp, Some("QuickOpen")),
        kb("down".into(), super::main_window::SearchDown, Some("SearchField > Input")),
        kb("up".into(), super::main_window::SearchUp, Some("SearchField > Input")),
        KeyBinding::new("escape", super::main_window::Dismiss, Some("MainWindow")),
    ]);
    #[cfg(not(target_os = "macos"))]
    b.push(kb(cmd("y"), Redo, Some("MainWindow")));
    #[cfg(target_os = "macos")]
    b.extend([KeyBinding::new("cmd-h", Hide, None), KeyBinding::new("cmd-alt-h", HideOthers, None)]);
    b.extend(
        [
            binding(ShortcutAction::GoModule1, GoModuleAt(0), None),
            binding(ShortcutAction::GoModule2, GoModuleAt(1), None),
            binding(ShortcutAction::GoModule3, GoModuleAt(2), None),
            binding(ShortcutAction::OpenOtherWorkspace, OpenOtherWorkspace, None),
            binding(ShortcutAction::ToggleFullscreen, ToggleFullscreen, None),
            binding(ShortcutAction::GoToday, GoToday, None),
            binding(ShortcutAction::GoGrid, GoGrid, None),
            binding(ShortcutAction::GoRecord, GoRecord, None),
            binding(ShortcutAction::GoInbox, GoInbox, None),
            binding(ShortcutAction::GoNotes, GoNotes, None),
            binding(ShortcutAction::GoCards, GoCards, None),
            binding(ShortcutAction::GoForward, GoForward, None),
            binding(ShortcutAction::ShowGraph, ShowGraph, None),
            binding(ShortcutAction::ReopenTab, ReopenTab, None),
            binding(ShortcutAction::QuickOpen, QuickOpen, None),
            binding(ShortcutAction::SearchFiles, SearchFiles, None),
            binding(ShortcutAction::NewEntry, NewEntry, None),
            binding(ShortcutAction::NewFile, NewFile, None),
            binding(ShortcutAction::GoBack, GoBack, None),
            binding(ShortcutAction::ToggleInspector, ToggleInspector, None),
            binding(ShortcutAction::Refresh, Refresh, None),
            binding(ShortcutAction::Sync, SyncNow, None),
            binding(ShortcutAction::DailyNote, DailyNote, None),
            binding(ShortcutAction::OpenInObsidian, OpenInObsidian, None),
            Some(gpui_kit::KeyBinding::new("tab", super::canvas_view::NextItem, Some("WalkItems"))),
            Some(gpui_kit::KeyBinding::new("shift-tab", super::canvas_view::PreviousItem, Some("WalkItems"))),
            binding(ShortcutAction::CloseTab, CloseTab, None),
            binding(ShortcutAction::NextTab, NextTab, None),
            binding(ShortcutAction::PreviousTab, PreviousTab, None),
            binding(ShortcutAction::TogglePreview, TogglePreview, None),
            binding(ShortcutAction::SplitPreview, SplitPreview, None),
            binding(ShortcutAction::ExportPdf, ExportPdf, None),
            binding(ShortcutAction::FindInNote, super::find_bar::FindInNote, None),
            binding(ShortcutAction::RenameNote, RenameNote, None),
            Some(kb(cmd("alt-f"), super::find_bar::ReplaceInNote, None)),
            #[cfg(not(target_os = "macos"))]
            Some(kb("ctrl-h".into(), super::find_bar::ReplaceInNote, None)),
        ]
        .into_iter()
        .flatten(),
    );
    b.extend(super::editor::key_bindings());
    b.extend(super::pdf_view::key_bindings());
    b.extend(super::sheet_view::key_bindings());
    let now = bound_pairs(&b);
    let mut bound = BOUND.lock().unwrap_or_else(|e| e.into_inner());
    let released: Vec<KeyBinding> = released_bindings(&bound, &now).into_iter().map(|(keys, name)| kb(keys, Unbind(name.into()), None)).collect();
    *bound = now;
    drop(bound);
    cx.bind_keys(released.into_iter().chain(b));
    rebuild_menus(cx);
}

/// The menu bar: Schrumbi, Ablage, Bearbeiten, Darstellung, Gehe zu, Fenster.
pub fn rebuild_menus(cx: &mut App) {
    let (note, find) = (note_is_active(cx), find_is_available(cx));
    let renamable = rename_is_available(cx);
    let modules: Vec<(String, String)> = state::read(cx, |l| l.semester_modules().iter().map(|m| (m.id.clone(), m.display_name())).collect());
    let recents: Vec<MenuItem> = prefs::get()
        .recent_workspaces
        .iter()
        .enumerate()
        .map(|(i, p)| MenuItem::action(p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), OpenRecent(i)))
        .chain([MenuItem::separator(), MenuItem::action(tr("Anderen Ordner öffnen…"), OpenOtherWorkspace)])
        .collect();
    let mut go: Vec<MenuItem> = vec![
        MenuItem::action(tr("Zurück"), GoBack),
        MenuItem::action(tr("Vorwärts"), GoForward),
        MenuItem::separator(),
        MenuItem::action(tr("Übersicht"), GoToday),
        MenuItem::action(tr("Semester"), GoGrid),
        MenuItem::action(tr("Leistungen"), GoRecord),
        MenuItem::action(tr("Inbox"), GoInbox),
        MenuItem::action(tr("Notizen"), GoNotes),
        MenuItem::action(tr("Karten"), GoCards),
        MenuItem::separator(),
        MenuItem::action(tr("Tagesnotiz öffnen"), DailyNote),
        MenuItem::action(tr("Graph zeigen"), ShowGraph),
        MenuItem::action(tr("In allen Dateien suchen"), SearchFiles),
        MenuItem::action(tr("Datei schnell öffnen…"), QuickOpen),
        MenuItem::separator(),
        MenuItem::action(tr("Nächster Tab"), NextTab),
        MenuItem::action(tr("Vorheriger Tab"), PreviousTab),
        MenuItem::separator(),
    ];
    go.extend(modules.into_iter().enumerate().map(|(i, (id, name))| {
        let name = end_ellipsis(&name, MENU_NAME);
        if ShortcutAction::for_module(i).is_some() { MenuItem::action(name, GoModuleAt(i)) } else { MenuItem::action(name, GoModule(id)) }
    }));
    let mut app_menu = vec![
        MenuItem::action(tr("Über Schrumbi"), About),
        MenuItem::action(tr("Nach Updates suchen…"), CheckForUpdates),
        MenuItem::separator(),
        MenuItem::action(tr("Einstellungen…"), OpenSettings),
        MenuItem::separator(),
    ];
    if cfg!(target_os = "macos") {
        app_menu.extend([MenuItem::action(tr("Schrumbi ausblenden"), Hide), MenuItem::action(tr("Andere ausblenden"), HideOthers), MenuItem::action(tr("Alle einblenden"), ShowAll), MenuItem::separator()]);
    }
    app_menu.push(MenuItem::action(tr("Schrumbi beenden"), Quit));
    cx.set_menus([
        Menu::new("Schrumbi").items(app_menu),
        Menu::new(tr("Ablage")).items([
            MenuItem::action(tr("Befehl suchen…"), CommandPalette),
            MenuItem::separator(),
            MenuItem::action(tr("Neuer Eintrag…"), NewEntry),
            MenuItem::action(tr("Neue Notiz, Datei oder Ordner…"), NewFile),
            MenuItem::action(tr("Datei schnell öffnen…"), QuickOpen),
            MenuItem::action(tr("Umbenennen…"), RenameNote).disabled(!renamable),
            MenuItem::separator(),
            MenuItem::action(tr("Kalender importieren (.ics)…"), ImportCalendar),
            MenuItem::action(tr("Kalender exportieren (.ics)…"), ExportCalendar),
            MenuItem::action(tr("Studium als ZIP exportieren…"), ExportArchive),
            MenuItem::separator(),
            MenuItem::action(super::platform::reveal_workspace_label(), RevealWorkspace),
            MenuItem::submenu(Menu::new(tr("Arbeitsordner")).items(recents)),
            MenuItem::separator(),
            MenuItem::action(tr("Sichern"), Save),
            MenuItem::action(tr("Jetzt synchronisieren"), SyncNow),
            MenuItem::action(tr("Tab schließen"), CloseTab),
            MenuItem::action(tr("Geschlossenen Tab wieder öffnen"), ReopenTab),
            MenuItem::action(tr("Fenster schließen"), CloseWindow),
        ]),
        Menu::new(tr("Bearbeiten")).items([
            MenuItem::action(tr("Rückgängig"), Undo),
            MenuItem::action(tr("Wiederholen"), Redo),
            MenuItem::separator(),
            MenuItem::os_action(tr("Ausschneiden"), gpui_kit::component::input::Cut, OsAction::Cut),
            MenuItem::os_action(tr("Kopieren"), gpui_kit::component::input::Copy, OsAction::Copy),
            MenuItem::os_action(tr("Einsetzen"), gpui_kit::component::input::Paste, OsAction::Paste),
            MenuItem::os_action(tr("Alles auswählen"), gpui_kit::component::input::SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action(tr("In Notiz suchen"), super::find_bar::FindInNote).disabled(!find),
            MenuItem::action(tr("In Notiz ersetzen"), super::find_bar::ReplaceInNote).disabled(!note),
        ]),
        Menu::new(tr("Darstellung")).items([
            MenuItem::action(tr("Lesen / Bearbeiten"), TogglePreview),
            MenuItem::action(tr("Geteilte Ansicht"), SplitPreview),
            MenuItem::action(tr("Notiz als PDF exportieren"), ExportPdf),
            MenuItem::separator(),
            MenuItem::action(tr("Seitenleiste ein-/ausblenden"), super::main_window::ToggleSidebar),
            MenuItem::action(tr("Details ein-/ausblenden"), ToggleInspector),
            MenuItem::action(tr("Aktualisieren"), Refresh),
            MenuItem::separator(),
            MenuItem::action(tr("Vollbild"), ToggleFullscreen),
        ]),
        Menu::new(tr("Gehe zu")).items(go),
        Menu::new(tr("Fenster")).items([MenuItem::action(tr("Im Dock ablegen"), Minimize), MenuItem::action(tr("Zoomen"), Zoom)]),
        Menu::new(tr("Hilfe")).items([MenuItem::action(tr("Schrumbi-Anleitung"), ShowHelp), MenuItem::action(tr("Tastenkürzel anzeigen"), ShowShortcuts)]),
    ]);
    super::chrome::sync_menus(cx);
}
