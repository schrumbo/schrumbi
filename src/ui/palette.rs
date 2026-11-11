use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::app;
use super::motion::Soft;
use super::quick::{Dismissed, QuickDown, QuickUp};
use super::state::{self, act};
use super::theme::{Theme, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::library::Route;
use crate::core::model::Status;
use crate::core::prefs::{Shortcut, ThemeChoice};
use crate::core::shortcuts::{Group, ShortcutAction as S};

enum Run {
    InWindow(fn(&mut Window, &mut App)),
    Action(Box<dyn Action>),
    /// Handled by the open note, which gets the keyboard back first.
    NoteAction(Box<dyn Action>),
    Call(Rc<dyn Fn(&mut App)>),
    WindowCall(Rc<dyn Fn(&mut Window, &mut App)>),
}

/// One entry of the command palette.
struct Command {
    label: String,
    group: String,
    icon: &'static str,
    keys: Option<String>,
    run: Run,
    aliases: String,
}

/// A command waiting for the open note to get the keyboard back.
#[derive(Default)]
pub struct PendingNoteAction(pub Option<Box<dyn Action>>);

impl Global for PendingNoteAction {}

/// Labels of recently run commands, newest first; they lead the list while nothing is typed.
#[derive(Default)]
struct Recent(Vec<String>);

impl Global for Recent {}

fn fixed_shortcut(key: &str, shift: bool) -> Shortcut {
    Shortcut { key: key.to_lowercase(), command: true, shift, option: false, control: false }
}

fn fixed_keys(key: &str) -> String {
    fixed_shortcut(key, false).display()
}

fn fixed_shift_keys(key: &str) -> String {
    fixed_shortcut(key, true).display()
}

fn keys(action: S) -> Option<String> {
    action.active_shortcut().map(|s| s.display())
}

/// Everything the app can do right now.
fn commands(cx: &App) -> Vec<Command> {
    let mut list = Vec::new();
    let mut extra: Vec<Command> = Vec::new();
    let mut add = |group: &str, label: String, icon: &'static str, keys: Option<String>, run: Run| list.push(Command { label, group: group.to_string(), icon, keys, run, aliases: String::new() });
    let (go, new, file, view, edit, general, cards) = (tr("Gehe zu"), tr("Neu"), tr("Ablage"), tr("Darstellung"), tr("Bearbeiten"), tr("Schrumbi"), tr("Karten"));
    let (git, conflicts) = (super::sync::enabled_here(cx), !super::sync::get(cx).conflicts.is_empty());
    let in_document = state::read(cx, |l| matches!(l.route(), Route::Doc(_)));
    let in_notes = in_document || state::read(cx, |l| matches!(l.route(), Route::Notes));
    let focused = state::read(cx, |l| l.focused_document().map(std::path::Path::to_path_buf).filter(|_| in_document));
    let active = focused.clone().filter(|p| crate::core::markdown::is_markdown(p));
    state::read(cx, |l| {
        if let Some(id) = l.selection().map(String::from)
            && let Some(a) = l.assignment(&id).cloned()
        {
            let current = tr("Auswahl");
            let closed = l.status(&a).is_closed();
            let (target, label) = if a.upload { (Status::Uploaded, tr("Hochgeladen")) } else { (Status::Done, tr("Erledigt")) };
            let toggle = id.clone();
            add(&current, trf("%@: %@", &[&a.title, &if closed { tr("Automatisch erkennen") } else { label }]), "circle-check", None, Run::Call(Rc::new(move |cx| {
                let status = if closed { None } else { Some(target) };
                act(cx, |l| l.set_status(status, &toggle, true));
            })));
            if !a.is_manual() {
                let move_id = id.clone();
                add(&current, trf("%@: %@", &[&a.title, &tr("Termin verschieben…")]), "calendar-clock", None, Run::Call(Rc::new(move |cx| act(cx, |l| l.reschedule_id = Some(move_id.clone())))));
            }
        }
        if let Some(path) = l.active_document.clone().filter(|p| l.buffers.get(p).is_some_and(|b| b.conflict.is_some())) {
            let group = tr("Notiz außerhalb geändert");
            let (compare, reload, keep) = (path.clone(), path.clone(), path);
            add(&group, tr("Vergleichen"), "file-text", None, Run::Call(Rc::new(move |cx| super::documents::compare_note(compare.clone(), None, cx))));
            add(&group, tr("Neu laden"), "refresh-cw", None, Run::Call(Rc::new(move |cx| act(cx, |l| l.load_note_disk(&reload)))));
            add(&group, tr("Meine behalten"), "check", None, Run::Call(Rc::new(move |cx| act(cx, |l| l.keep_note_mine(&keep)))));
        }
        if conflicts {
            add(&general, tr("Konflikte lösen…"), "git-merge", None, Run::Action(Box::new(app::ResolveConflicts)));
        }
        add(&go, S::GoToday.label(), "layout-grid", keys(S::GoToday), Run::Action(Box::new(app::GoToday)));
        if state::read(cx, |l| matches!(l.route(), Route::Today)) {
            let calendar = tr("Kalender");
            add(&calendar, tr("Heute"), "calendar-check", Some("T".into()), Run::Call(Rc::new(|cx| act(cx, crate::core::library::Library::calendar_today))));
            add(&calendar, tr("Woche"), "calendar", Some("W".into()), Run::Call(Rc::new(|cx| act(cx, |l| l.set_calendar_view(crate::core::calendar::CalendarView::Week)))));
            add(&calendar, tr("Monat"), "calendar-days", Some("M".into()), Run::Call(Rc::new(|cx| act(cx, |l| l.set_calendar_view(crate::core::calendar::CalendarView::Month)))));
        }
        add(&go, S::GoGrid.label(), "table", keys(S::GoGrid), Run::Action(Box::new(app::GoGrid)));
        add(&go, S::GoRecord.label(), "award", keys(S::GoRecord), Run::Action(Box::new(app::GoRecord)));
        add(&go, S::GoInbox.label(), "inbox", keys(S::GoInbox), Run::Action(Box::new(app::GoInbox)));
        add(&go, S::GoNotes.label(), "book-open", keys(S::GoNotes), Run::Action(Box::new(app::GoNotes)));
        add(&go, S::GoCards.label(), "layers", keys(S::GoCards), Run::Action(Box::new(app::GoCards)));
        add(&cards, tr("Neue Karte…"), "plus", None, Run::InWindow(super::cards_page::new_card));
        add(&cards, tr("Karten lernen"), "graduation-cap", None, Run::Call(Rc::new(|cx| {
            act(cx, |l| {
                l.cards_request = Some(crate::core::prefs::get().card_deck);
                l.set_route(Route::Cards);
            })
        })));
        add(&cards, tr("Aus Anki importieren…"), "download", None, Run::InWindow(super::cards_page::choose_import));
        add(&go, S::SearchFiles.label(), "search", keys(S::SearchFiles), Run::Action(Box::new(app::SearchFiles)));
        add(&go, S::GoBack.label(), "arrow-left", keys(S::GoBack), Run::Action(Box::new(app::GoBack)));
        add(&go, S::GoForward.label(), "arrow-right", keys(S::GoForward), Run::Action(Box::new(app::GoForward)));
        if l.open_documents.len() > 1 {
            add(&go, S::NextTab.label(), "arrow-right", keys(S::NextTab), Run::Action(Box::new(app::NextTab)));
            add(&go, S::PreviousTab.label(), "arrow-left", keys(S::PreviousTab), Run::Action(Box::new(app::PreviousTab)));
        }
        if l.can_reopen_document() {
            add(&go, S::ReopenTab.label(), "rotate-ccw", keys(S::ReopenTab), Run::Action(Box::new(app::ReopenTab)));
        }
        if !l.open_documents.is_empty() {
            add(&go, tr("Alle Tabs schließen"), "x", None, Run::Call(Rc::new(|cx| super::documents::close_all_tabs(cx))));
        }
        for (i, m) in l.semester_modules().into_iter().enumerate() {
            let route = Route::Module(m.id.clone());
            add(&go, m.display_name(), "book-open", S::for_module(i).and_then(keys), Run::Call(Rc::new(move |cx| act(cx, |l| l.set_route(route.clone())))));
        }
    });
    add(&new, S::NewEntry.label(), "calendar-plus", keys(S::NewEntry), Run::Action(Box::new(app::NewEntry)));
    add(&new, S::NewFile.label(), "file-plus", keys(S::NewFile), Run::Action(Box::new(app::NewFile)));
    add(&file, S::QuickOpen.label(), "search", keys(S::QuickOpen), Run::Action(Box::new(app::QuickOpen)));
    add(&file, tr("Kalender importieren (.ics)…"), "calendar-plus", None, Run::Action(Box::new(app::ImportCalendar)));
    add(&file, tr("Kalender exportieren (.ics)…"), "calendar", None, Run::Action(Box::new(app::ExportCalendar)));
    add(&file, tr("Studium als ZIP exportieren…"), "archive", None, Run::Action(Box::new(app::ExportArchive)));
    add(&file, super::platform::reveal_workspace_label(), "folder", None, Run::Action(Box::new(app::RevealWorkspace)));
    add(&file, S::OpenOtherWorkspace.label(), "folder-open", keys(S::OpenOtherWorkspace), Run::Action(Box::new(app::OpenOtherWorkspace)));
    if state::read(cx, |l| l.inspector_kind().is_some()) {
        add(&view, S::ToggleInspector.label(), "panel-right", keys(S::ToggleInspector), Run::Action(Box::new(app::ToggleInspector)));
    }
    add(&view, S::Refresh.label(), "refresh-cw", keys(S::Refresh), Run::Action(Box::new(app::Refresh)));
    add(&view, S::ToggleFullscreen.label(), "maximize", keys(S::ToggleFullscreen), Run::Action(Box::new(app::ToggleFullscreen)));
    if active.is_some() {
        add(&view, S::TogglePreview.label(), "book-open", keys(S::TogglePreview), Run::NoteAction(Box::new(app::TogglePreview)));
        add(&view, if app::live_is_on() { tr("Live-Ansicht ausschalten") } else { tr("Live-Ansicht einschalten") }, "sparkles", keys(S::ToggleLive), Run::Action(Box::new(app::ToggleLive)));
        if crate::core::prefs::get().note_editor == crate::core::prefs::NoteEditor::Code {
            add(&view, S::SplitPreview.label(), "columns-2", keys(S::SplitPreview), Run::NoteAction(Box::new(app::SplitPreview)));
        }
        if app::wrap_is_available(cx) {
            add(&view, S::ToggleWrap.label(), "text", keys(S::ToggleWrap), Run::NoteAction(Box::new(super::documents::ToggleWrap)));
        }
        add(&view, S::ExportPdf.label(), "share", keys(S::ExportPdf), Run::NoteAction(Box::new(app::ExportPdf)));
        add(&edit, S::FindInNote.label(), "search", keys(S::FindInNote), Run::NoteAction(Box::new(super::find_bar::FindInNote)));
        add(&edit, tr("Zum Wörterbuch hinzufügen"), "plus", None, Run::NoteAction(Box::new(super::spell_card::AddWordAtCaret)));
        add(&edit, tr("In Notiz ersetzen"), "replace", Some(if cfg!(target_os = "macos") { Shortcut { key: "f".into(), command: true, shift: false, option: true, control: false } } else { fixed_shortcut("h", false) }.display()), Run::NoteAction(Box::new(super::find_bar::ReplaceInNote)));
        use super::documents as d;
        let chord = |key: &str, shift: bool, option: bool| Shortcut { key: key.into(), command: true, shift, option, control: false }.display();
        let configurable: [(S, &'static str, Box<dyn Action>); 14] = [
            (S::Bold, "bold", Box::new(d::Bold)),
            (S::Italic, "italic", Box::new(d::Italic)),
            (S::Link, "link", Box::new(d::InsertLink)),
            (S::InlineCode, "code", Box::new(d::InlineCode)),
            (S::ToggleTask, "square-check", Box::new(d::ToggleTask)),
            (S::ToggleComment, "text", Box::new(d::ToggleComment)),
            (S::MoveLineUp, "arrow-up", Box::new(d::MoveLineUp)),
            (S::MoveLineDown, "arrow-down", Box::new(d::MoveLineDown)),
            (S::CopyLineUp, "copy", Box::new(d::CopyLineUp)),
            (S::CopyLineDown, "copy", Box::new(d::CopyLineDown)),
            (S::DeleteLine, "trash-2", Box::new(d::DeleteLine)),
            (S::SelectLine, "text-search", Box::new(d::SelectLine)),
            (S::Indent, "arrow-right", Box::new(d::IndentLines)),
            (S::Outdent, "arrow-left", Box::new(d::OutdentLines)),
        ];
        for (action, icon_name, run) in configurable {
            add(&edit, action.label(), icon_name, keys(action), Run::NoteAction(run));
        }
        let fixed: [(&str, &'static str, String, Box<dyn Action>); 9] = [
            ("Durchgestrichen", "strikethrough", chord("x", true, false), Box::new(d::Strike)),
            ("Markiert", "highlighter", chord("h", true, false), Box::new(d::Mark)),
            ("Formel", "sigma", chord("m", true, false), Box::new(d::InlineMath)),
            ("Zitat", "quote", chord(".", true, false), Box::new(d::Quote)),
            ("Liste", "list", chord("8", true, false), Box::new(d::BulletList)),
            ("Nummerierte Liste", "list-ordered", chord("7", true, false), Box::new(d::NumberedList)),
            ("Überschrift 1", "heading-1", chord("1", false, true), Box::new(d::Heading1)),
            ("Überschrift 2", "heading-2", chord("2", false, true), Box::new(d::Heading2)),
            ("Überschrift 3", "heading-3", chord("3", false, true), Box::new(d::Heading3)),
        ];
        for (label, icon_name, shown, run) in fixed {
            add(&edit, tr(label), icon_name, Some(shown), Run::NoteAction(run));
        }
        for (action, icon_name, run) in [(S::ZoomIn, "zoom-in", Box::new(d::ZoomIn) as Box<dyn Action>), (S::ZoomOut, "zoom-out", Box::new(d::ZoomOut)), (S::ZoomReset, "scan", Box::new(d::ZoomReset))] {
            add(&view, action.label(), icon_name, keys(action), Run::NoteAction(run));
        }
        let full = crate::core::prefs::get().reader_full_width;
        add(&view, if full { tr("Text in voller Breite ausschalten") } else { tr("Text in voller Breite einschalten") }, "move-horizontal", None, Run::Call(Rc::new(move |cx| super::documents::set_reader_full_width(!full, cx))));
        add(&file, tr("Sichern"), "save", Some(fixed_keys("S")), Run::NoteAction(Box::new(app::Save)));
    }
    if focused.as_deref().is_some_and(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))) {
        use super::pdf_view as p;
        add(&edit, S::MarkText.label(), "highlighter", keys(S::MarkText), Run::NoteAction(Box::new(p::MarkSelection)));
        let pages: [(S, &'static str, Box<dyn Action>); 6] = [
            (S::RotatePagesLeft, "rotate-ccw", Box::new(p::RotatePagesLeft)),
            (S::RotatePagesRight, "rotate-cw", Box::new(p::RotatePagesRight)),
            (S::MovePagesUp, "arrow-up", Box::new(p::MovePagesUp)),
            (S::MovePagesDown, "arrow-down", Box::new(p::MovePagesDown)),
            (S::DeletePages, "trash-2", Box::new(p::DeletePages)),
            (S::InsertPdf, "file-plus", Box::new(p::InsertPdf)),
        ];
        for (action, icon_name, run) in pages {
            add(&edit, action.label(), icon_name, keys(action), Run::NoteAction(run));
        }
        add(&edit, S::NoteOnSelection.label(), "sticky-note", keys(S::NoteOnSelection), Run::NoteAction(Box::new(p::NoteOnSelection)));
        let tools: [(S, &'static str, Box<dyn Action>, String); 3] = [
            (S::PenTool, "pencil", Box::new(p::PenTool), tr("zeichnen malen schreiben")),
            (S::EraserTool, "eraser", Box::new(p::EraserTool), tr("löschen wegmachen")),
            (S::NoteTool, "sticky-note", Box::new(p::NoteTool), tr("kommentar anmerkung")),
        ];
        for (action, icon_name, run, aliases) in tools {
            extra.push(Command { label: action.label(), group: edit.clone(), icon: icon_name, keys: keys(action), run: Run::NoteAction(run), aliases });
        }
        for (action, aliases, run) in p::pen_commands() {
            extra.push(Command { label: action.label(), group: edit.clone(), icon: "pencil", keys: keys(action), run: Run::NoteAction(run), aliases });
        }
        add(&file, S::ExportMarkedPdf.label(), "download", keys(S::ExportMarkedPdf), Run::NoteAction(Box::new(p::ExportMarkedPdf)));
        add(&view, S::MarkList.label(), "highlighter", keys(S::MarkList), Run::NoteAction(Box::new(p::ShowMarks)));
        add(&view, S::PageList.label(), "gallery-vertical-end", keys(S::PageList), Run::NoteAction(Box::new(p::TogglePageList)));
    }
    if in_document {
        add(&view, S::CloseTab.label(), "x", keys(S::CloseTab), Run::Action(Box::new(app::CloseTab)));
    }
    if let Some(doc) = focused.clone().filter(|p| super::convention_rename::available(p, cx)) {
        add(&file, tr("Nach Muster benennen…"), "wand-sparkles", None, Run::WindowCall(Rc::new(move |window, cx| super::convention_rename::open(doc.clone(), window, cx))));
    }
    if let Some(target) = state::read(cx, app::rename_target) {
        add(&file, tr("Umbenennen…"), "pencil", keys(S::RenameNote), Run::Call(Rc::new(move |cx| app::rename_file(target.clone(), cx))));
    }
    if in_document && state::read(cx, |l| l.split_document.is_some()) {
        add(&view, tr("Geteilte Ansicht schließen"), "columns-2", None, Run::Call(Rc::new(|cx| act(cx, |l| l.set_split(None)))));
    }
    if state::read(cx, |l| l.can_undo()) {
        add(&edit, tr("Rückgängig"), "undo-2", Some(fixed_keys("Z")), Run::Action(Box::new(app::Undo)));
    }
    if state::read(cx, |l| l.can_redo()) {
        add(&edit, tr("Wiederholen"), "redo-2", Some(fixed_shift_keys("Z")), Run::Action(Box::new(app::Redo)));
    }
    if in_document && git {
        add(&file, tr("Verlauf dieser Datei"), "history", None, Run::Call(Rc::new(|cx| {
            act(cx, |l| {
                l.peek_inspector();
                l.show_versions = true;
            });
        })));
    }
    add(&general, S::Sync.label(), "cloud", keys(S::Sync), Run::Action(Box::new(app::SyncNow)));
    if !git {
        add(&general, tr("Sync einrichten…"), "git-branch", None, Run::Action(Box::new(app::SyncSetup)));
    }
    if git {
        add(&general, tr("Sync-Einstellungen"), "settings", None, Run::Call(Rc::new(|cx| super::settings::open_sync(cx))));
    }
    let notes = tr("Notizen");
    let obsidian = state::read(cx, |l| l.is_vault);
    add(&notes, tr("Tagesnotiz öffnen"), "calendar", keys(S::DailyNote), Run::Action(Box::new(app::DailyNote)));
    add(&notes, tr("Eindeutige Notiz anlegen"), "file-plus", None, Run::Call(Rc::new(|cx| {
        act(cx, |l| l.open_unique_note());
        state::reindex(cx);
    })));
    add(&notes, tr("Zufällige Notiz öffnen"), "shuffle", None, Run::Call(Rc::new(|cx| act(cx, |l| l.open_random_note()))));
    add(&notes, tr("Arbeitsbereich sichern…"), "layout-grid", None, Run::Call(Rc::new(|cx| act(cx, |l| l.show_save_layout = true))));
    for name in state::read(cx, |l| l.layouts()) {
        let label = trf("Arbeitsbereich laden: %@", &[&name]);
        add(&notes, label, "layout-grid", None, Run::Call(Rc::new(move |cx| act(cx, |l| l.open_layout(&name)))));
    }
    if in_notes {
        add(&notes, tr("Dateien ein- oder ausblenden"), "panel-left", None, Run::Call(Rc::new(|cx| super::notes_page::set_hidden(!crate::core::prefs::get().notes_explorer_hidden, cx))));
    }
    add(&notes, S::ShowGraph.label(), "waypoints", keys(S::ShowGraph), Run::Action(Box::new(app::ShowGraph)));
    if let Some(note) = active {
        let (a, b, c, d, e) = (note.clone(), note.clone(), note.clone(), note.clone(), note);
        add(&notes, tr("Graph um diese Notiz"), "waypoints", None, Run::Call(Rc::new(move |cx| {
            let p = a.clone();
            act(cx, |l| l.show_graph(Some(p)));
        })));
        if obsidian {
            add(&notes, tr("Vorlage einfügen"), "file-plus", None, Run::Call(Rc::new(move |cx| {
                let p = b.clone();
                act(cx, |l| l.note_pick = Some((false, p)));
            })));
        }
        add(&notes, tr("Mit anderer Notiz zusammenführen"), "merge", None, Run::Call(Rc::new(move |cx| {
            let p = c.clone();
            act(cx, |l| l.note_pick = Some((true, p)));
        })));
        add(&notes, tr("Als Folien zeigen"), "presentation", None, Run::Call(Rc::new(move |cx| {
            let p = d.clone();
            act(cx, |l| l.slides = Some(p));
        })));
        if obsidian {
            let mark = e.clone();
            add(&notes, tr("Lesezeichen setzen oder entfernen"), "bookmark", None, Run::Call(Rc::new(move |cx| {
                let root = crate::core::vault::index::current().root.clone();
                if let Ok(rel) = mark.strip_prefix(&root) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    if crate::core::vault::config::toggle_bookmark(&root, &rel).is_ok() {
                        act(cx, |l| l.notify(if crate::core::vault::config::is_bookmarked(&root, &rel) { tr("Lesezeichen gesetzt") } else { tr("Lesezeichen entfernt") }, "bookmark", None, None));
                        state::reindex(cx);
                    }
                }
            })));
            add(&tr("Obsidian"), tr("In Obsidian öffnen"), "external-link", keys(S::OpenInObsidian), Run::Call(Rc::new(move |cx| super::vault_tools::open_in_obsidian(&e, cx))));
        }
        add(&notes, tr("Auswahl in neue Notiz auslagern"), "scissors", None, Run::NoteAction(Box::new(super::documents::ExtractSelection)));
        add(&notes, tr("Karte aus Auswahl…"), "layers", None, Run::NoteAction(Box::new(super::documents::CardFromSelection)));
    }
    add(&general, tr("Geführt einrichten…"), "list-checks", None, Run::Call(Rc::new(|cx| act(cx, |l| l.needs_setup = true))));
    add(&general, tr("Tipp des Tages zeigen"), "lightbulb", None, Run::Call(Rc::new(super::tips::show_next)));
    add(&general, tr("Einstellungen…"), "settings", Some(fixed_keys(",")), Run::Action(Box::new(app::OpenSettings)));
    add(&general, tr("Schrumbi-Anleitung"), "book-open", Some("F1".to_string()), Run::Action(Box::new(app::ShowHelp)));
    add(&general, tr("Tastenkürzel anzeigen"), "keyboard", Some(fixed_keys("/")), Run::Action(Box::new(app::ShowShortcuts)));
    add(&general, tr("Nach Updates suchen…"), "download", None, Run::Action(Box::new(app::CheckForUpdates)));
    add(&general, tr("Über Schrumbi"), "info", None, Run::Action(Box::new(app::About)));
    if super::ai::active() {
        let ki = tr("KI");
        add(&ki, tr("Frag die KI…"), "bot", keys(S::ToggleAi), Run::Call(Rc::new(super::ai::open_column)));
        for s in super::ai_panel::suggestions(cx) {
            let (flow, chips) = (s.flow, s.chips.clone());
            add(&ki, s.label, s.icon, None, Run::Call(Rc::new(move |cx| super::ai::start_flow(flow, chips.clone(), cx))));
        }
        let setup = crate::core::ai::prompts::Flow::Setup;
        add(&ki, setup.label(), setup.icon(), None, Run::Call(Rc::new(move |cx| super::ai::start_flow(setup, Vec::new(), cx))));
    }
    let (choice, file) = crate::core::prefs::with(|p| (p.theme, p.theme_file.clone()));
    let designs: Vec<_> = crate::core::themes::installed().into_iter().filter(|d| d.problem.is_none()).collect();
    let active_file = designs.iter().any(|d| d.id == file);
    let appearance = tr("theme farbe aussehen");
    let mut theme_command = |label: String, run: Run| extra.push(Command { label: trf("Design: %@", &[&label]), group: view.clone(), icon: "palette", keys: None, run, aliases: appearance.clone() });
    for built_in in [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::Nord, ThemeChoice::Dracula] {
        if !file.is_empty() || built_in != choice {
            theme_command(super::theme::choice_label(built_in), Run::WindowCall(Rc::new(move |window, cx| super::theme::choose(built_in, window, cx))));
        }
    }
    for design in designs {
        if !(active_file && design.id == file) {
            let id = design.id.clone();
            theme_command(design.name, Run::WindowCall(Rc::new(move |window, cx| super::theme::choose_file(&id, window, cx))));
        }
    }
    extra.push(Command {
        label: tr("Design laden…"),
        group: view.clone(),
        icon: "palette",
        keys: None,
        run: Run::WindowCall(Rc::new(|window, cx| {
            super::theme::pick_and_load(window.window_handle(), cx, |result, cx| {
                if let Err(m) = result {
                    act(cx, |l| l.notify(m, "circle-alert", None, None));
                }
            })
        })),
        aliases: appearance.clone(),
    });
    list.extend(extra);
    list
}

fn fold(s: &str) -> String {
    s.to_lowercase().replace('ä', "a").replace('ö', "o").replace('ü', "u").replace('ß', "ss")
}

/// How well `query` fits `label`: whole prefix first, then a word start, then anywhere. All words must occur.
fn score(label: &str, group: &str, aliases: &str, query: &str) -> Option<u32> {
    let (label, group, aliases, query) = (fold(label), fold(group), fold(aliases), fold(query));
    let mut total = 0;
    for term in query.split_whitespace() {
        total += if label.starts_with(term) {
            3
        } else if label.split([' ', '/', '-', '(']).any(|w| w.starts_with(term)) {
            2
        } else if label.contains(term) || group.starts_with(term) || aliases.split_whitespace().any(|w| w.starts_with(term)) {
            1
        } else {
            return None;
        };
    }
    Some(total)
}

/// ⇧⌘P: find and run any action of the app.
pub struct Palette {
    query: Entity<InputState>,
    commands: Vec<Command>,
    index: usize,
    focused: bool,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<Dismissed> for Palette {}

impl Palette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Befehl suchen…")));
        let subs = vec![cx.subscribe_in(&query, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::Change => {
                this.index = 0;
                this.scroll.scroll_to_item(0);
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.run(window, cx),
            _ => {}
        })];
        Self { query, commands: commands(cx), index: 0, focused: false, scroll: ScrollHandle::new(), _subs: subs }
    }

    fn matches(&self, cx: &App) -> Vec<usize> {
        let q = self.query.read(cx).value().to_string();
        let mut found: Vec<(usize, u32)> = self.commands.iter().enumerate().filter_map(|(i, c)| score(&c.label, &c.group, &c.aliases, &q).map(|s| (i, s))).collect();
        if q.trim().is_empty() {
            if let Some(recent) = cx.try_global::<Recent>() {
                found.sort_by_key(|(i, _)| recent.0.iter().position(|l| *l == self.commands[*i].label).unwrap_or(usize::MAX));
            }
        } else {
            found.sort_by(|a, b| b.1.cmp(&a.1));
        }
        found.into_iter().map(|(i, _)| i).collect()
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.matches(cx).len();
        if count > 0 {
            self.index = (self.index as isize + by).rem_euclid(count as isize) as usize;
            self.scroll.scroll_to_item(self.index);
        }
        cx.notify();
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.matches(cx).get(self.index).copied() else { return };
        let label = self.commands[i].label.clone();
        let recent = &mut cx.default_global::<Recent>().0;
        recent.retain(|l| *l != label);
        recent.insert(0, label);
        recent.truncate(5);
        cx.emit(Dismissed);
        match &self.commands[i].run {
            Run::Action(action) => {
                let action = action.boxed_clone();
                window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
            }
            Run::NoteAction(action) => {
                let action = action.boxed_clone();
                cx.default_global::<PendingNoteAction>().0 = Some(action);
                act(cx, |l| if l.beside_active && l.split_document.is_some() { l.focus_beside = true } else { l.focus_document = true });
            }
            Run::InWindow(call) => window.defer(cx, *call),
            Run::Call(call) => {
                let call = call.clone();
                window.defer(cx, move |_, cx| call(cx));
            }
            Run::WindowCall(call) => {
                let call = call.clone();
                window.defer(cx, move |window, cx| call(window, cx));
            }
        }
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            self.focused = true;
            self.query.update(cx, |s, cx| s.focus(window, cx));
        }
        let t = Theme::get(cx);
        let found = self.matches(cx);
        self.index = self.index.min(found.len().saturating_sub(1));
        let me = cx.entity();
        let rows = found.iter().enumerate().map(|(row, i)| {
            let c = &self.commands[*i];
            let selected = row == self.index;
            let (fg, sub) = if selected { (white().into(), Hsla::from(white()).opacity(0.85)) } else { (t.primary, t.secondary) };
            let e = me.clone();
            div()
                .id(("pal", row))
                .flex()
                .items_center()
                .gap(px(space::S))
                .px(px(space::M))
                .py(px(space::S))
                .rounded(px(radius::INNER))
                .cursor_pointer()
                .text_color(fg)
                .when(selected, |d| d.bg(t.accent))
                .when(!selected, |d| d.soft(t.ink.opacity(0.05), radius::INNER))
                .child(icon(c.icon, iconsize::M, sub))
                .child(div().flex_1().min_w_0().truncate().child(c.label.clone()))
                .child(div().text_size(px(font::CAPTION)).text_color(sub).child(c.group.clone()))
                .when_some(c.keys.clone(), |d, k| d.child(div().min_w(px(space::XXL + space::L)).flex().justify_end().text_size(px(font::CAPTION)).font_features(tabular()).text_color(sub).child(k)))
                .on_click(move |_, window, cx| e.update(cx, |p, cx| {
                    p.index = row;
                    p.run(window, cx);
                }))
                .into_any_element()
        });
        let list = div().flex().flex_col().gap(px(space::XXS)).p(px(space::S)).children(rows.collect::<Vec<_>>());
        div()
            .w_full()
            .flex()
            .flex_col()
            .key_context("QuickOpen")
            .on_action(cx.listener(|p, _: &QuickDown, _, cx| p.step(1, cx)))
            .on_action(cx.listener(|p, _: &QuickUp, _, cx| p.step(-1, cx)))
            .child(div().flex().items_center().gap(px(space::S)).p(px(space::L)).child(icon("search", iconsize::M, t.secondary)).child(div().flex_1().child(Input::new(&self.query).appearance(false))))
            .child(div().h(px(1.)).bg(t.separator))
            .child(if found.is_empty() {
                unavailable("search", tr("Keine Treffer"), "", &t).h(px(360.)).into_any_element()
            } else {
                div().relative().h(px(360.)).child(div().id("pal-list").size_full().overflow_y_scroll().track_scroll(&self.scroll).child(list)).vertical_scrollbar(&self.scroll).into_any_element()
            })
    }
}

fn shortcut_row(label: String, parts: Vec<String>, t: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(space::M))
        .py(px(space::XS))
        .child(div().min_w_0().truncate().child(label))
        .child(div().flex().flex_none().gap(px(space::XXS)).children(parts.into_iter().map(|p| kbd(p, t))))
}

fn fixed_parts(key: &str, shift: bool) -> Vec<String> {
    fixed_shortcut(key, shift).parts()
}

/// ⌘/: every shortcut at a glance, always showing the user's current keys.
pub struct ShortcutSheet;

impl EventEmitter<Dismissed> for ShortcutSheet {}

impl Render for ShortcutSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let section = |title: String, rows: Vec<Div>| div().flex().flex_col().gap(px(space::XS)).child(div().pb(px(space::XS)).font_weight(FontWeight::SEMIBOLD).child(title)).children(rows);
        let modules = state::read(cx, |l| l.semester_modules().iter().take(S::MODULES.len()).map(|m| m.display_name()).collect::<Vec<_>>());
        let group = |g: Group| -> Vec<Div> { S::ALL.into_iter().filter(|a| a.group() == g && a.module_index().is_none_or(|i| i < modules.len()) && (*a != S::ToggleAi || super::ai::active())).filter_map(|a| a.active_shortcut().map(|s| shortcut_row(a.named(&modules), s.parts(), &t))).collect() };
        let mut general = vec![
            shortcut_row(tr("Befehl suchen…"), fixed_parts("P", true), &t),
            shortcut_row(tr("Tastenkürzel anzeigen"), fixed_parts("/", false), &t),
            shortcut_row(tr("Schrumbi-Anleitung"), vec!["F1".to_string()], &t),
            shortcut_row(tr("Suchen"), vec!["/".to_string()], &t),
            shortcut_row(tr("Rückgängig"), fixed_parts("Z", false), &t),
        ];
        general.extend(group(Group::General));
        let navigation = group(Group::Navigation);
        let close = cx.entity();
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pl(px(space::XL))
                    .pr(px(space::XL + space::L))
                    .py(px(space::L))
                    .child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::SEMIBOLD).child(tr("Tastenkürzel")))
                    .child(link_button("shortcuts-edit", tr("In den Einstellungen ändern"), &t).on_click(move |_, _, cx| {
                        close.update(cx, |_, cx| cx.emit(Dismissed));
                        super::settings::open(cx);
                    })),
            )
            .child(div().h(px(1.)).bg(t.separator))
            .child(
                div().relative().h(px(520.)).child(
                    div()
                        .id("shortcuts-scroll")
                        .size_full()
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .flex()
                                .gap(px(space::XL))
                                .p(px(space::XL))
                                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XL)).child(section(Group::General.label(), general)).child(section(Group::Navigation.label(), navigation)))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(px(space::XL))
                                        .child(section(Group::View.label(), group(Group::View)))
                                        .child(section(Group::Documents.label(), group(Group::Documents)))
                                        .child(section(Group::Editor.label(), group(Group::Editor))),
                                ),
                        ),
                ),
            )
    }
}
