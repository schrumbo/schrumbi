use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::app;
use super::quick::{Dismissed, QuickDown, QuickUp};
use super::state::{self, act};
use super::theme::{Theme, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::library::Route;
use crate::core::model::Status;
use crate::core::shortcuts::{Group, ShortcutAction as S};

enum Run {
    Action(Box<dyn Action>),
    Call(Rc<dyn Fn(&mut App)>),
}

/// One entry of the command palette.
struct Command {
    label: String,
    group: String,
    icon: &'static str,
    keys: Option<String>,
    run: Run,
}

/// Labels of recently run commands, newest first; they lead the list while nothing is typed.
#[derive(Default)]
struct Recent(Vec<String>);

impl Global for Recent {}

fn fixed_keys(key: &str) -> String {
    if cfg!(target_os = "macos") { format!("⌘{key}") } else { format!("Ctrl+{key}") }
}

fn keys(action: S) -> Option<String> {
    action.shortcut().map(|s| s.display())
}

/// Everything the app can do right now.
fn commands(cx: &App) -> Vec<Command> {
    let mut list = Vec::new();
    let mut add = |group: &str, label: String, icon: &'static str, keys: Option<String>, run: Run| list.push(Command { label, group: group.to_string(), icon, keys, run });
    let (go, new, file, view, edit, general) = (tr("Gehe zu"), tr("Neu"), tr("Ablage"), tr("Darstellung"), tr("Bearbeiten"), tr("Schrumbi"));
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
            add(&group, tr("Vergleichen"), "file-text", None, Run::Call(Rc::new(move |cx| super::documents::compare_note(compare.clone(), cx))));
            add(&group, tr("Neu laden"), "refresh-cw", None, Run::Call(Rc::new(move |cx| act(cx, |l| l.load_note_disk(&reload)))));
            add(&group, tr("Meine behalten"), "check", None, Run::Call(Rc::new(move |cx| act(cx, |l| l.keep_note_mine(&keep)))));
        }
        add(&general, S::Sync.label(), "cloud", keys(S::Sync), Run::Action(Box::new(app::SyncNow)));
        add(&general, tr("Sync einrichten…"), "git-branch", None, Run::Action(Box::new(app::SyncSetup)));
        add(&general, tr("Konflikte lösen…"), "git-merge", None, Run::Action(Box::new(app::ResolveConflicts)));
        add(&go, tr("Übersicht"), "layout-grid", Some(fixed_keys("1")), Run::Action(Box::new(app::GoToday)));
        add(&go, tr("Semester"), "table", Some(fixed_keys("2")), Run::Action(Box::new(app::GoGrid)));
        add(&go, tr("Leistungen"), "award", Some(fixed_keys("3")), Run::Action(Box::new(app::GoRecord)));
        add(&go, tr("Inbox"), "inbox", Some(fixed_keys("4")), Run::Action(Box::new(app::GoInbox)));
        add(&go, tr("Aufräumen"), "list-checks", Some(fixed_keys("5")), Run::Action(Box::new(app::GoChecks)));
        add(&go, S::SearchFiles.label(), "search", keys(S::SearchFiles), Run::Action(Box::new(app::SearchFiles)));
        add(&go, S::GoBack.label(), "arrow-left", keys(S::GoBack), Run::Action(Box::new(app::GoBack)));
        add(&go, tr("Vorwärts"), "arrow-right", None, Run::Action(Box::new(app::GoForward)));
        add(&go, tr("Geschlossenen Tab wieder öffnen"), "rotate-ccw", None, Run::Action(Box::new(app::ReopenTab)));
        add(&go, tr("Alle Tabs schließen"), "x", None, Run::Call(Rc::new(|cx| super::documents::close_all_tabs(cx))));
        add(&go, tr("Einrichtung erneut starten"), "settings", None, Run::Call(Rc::new(|cx| act(cx, |l| l.needs_setup = true))));
        for m in l.semester_modules() {
            let route = Route::Module(m.id.clone());
            add(&go, m.display_name(), "book-open", None, Run::Call(Rc::new(move |cx| act(cx, |l| l.set_route(route.clone())))));
        }
    });
    add(&new, S::NewEntry.label(), "calendar-plus", keys(S::NewEntry), Run::Action(Box::new(app::NewEntry)));
    add(&new, S::NewFile.label(), "file-plus", keys(S::NewFile), Run::Action(Box::new(app::NewFile)));
    add(&file, S::QuickOpen.label(), "search", keys(S::QuickOpen), Run::Action(Box::new(app::QuickOpen)));
    add(&file, tr("Kalender importieren (.ics)…"), "calendar-plus", None, Run::Action(Box::new(app::ImportCalendar)));
    add(&file, tr("Kalender exportieren (.ics)…"), "calendar", None, Run::Action(Box::new(app::ExportCalendar)));
    add(&file, tr("Studium als ZIP exportieren…"), "archive", None, Run::Action(Box::new(app::ExportArchive)));
    add(&file, super::platform::reveal_workspace_label(), "folder", None, Run::Action(Box::new(app::RevealWorkspace)));
    add(&file, tr("Anderen Ordner öffnen…"), "folder-open", None, Run::Action(Box::new(app::OpenOtherWorkspace)));
    add(&view, S::ToggleInspector.label(), "panel-right", keys(S::ToggleInspector), Run::Action(Box::new(app::ToggleInspector)));
    add(&view, tr("Seitenleiste ein-/ausblenden"), "panel-left", Some(fixed_keys("B")), Run::Action(Box::new(super::main_window::ToggleSidebar)));
    add(&view, S::Refresh.label(), "refresh-cw", keys(S::Refresh), Run::Action(Box::new(app::Refresh)));
    if state::read(cx, |l| matches!(l.route(), Route::Doc(_))) {
        add(&view, S::TogglePreview.label(), "book-open", keys(S::TogglePreview), Run::Action(Box::new(app::TogglePreview)));
        add(&view, S::ExportPdf.label(), "share", keys(S::ExportPdf), Run::Action(Box::new(app::ExportPdf)));
        add(&view, S::CloseTab.label(), "x", keys(S::CloseTab), Run::Action(Box::new(app::CloseTab)));
    }
    if state::read(cx, |l| l.can_undo()) {
        add(&edit, tr("Widerrufen"), "undo-2", Some(fixed_keys("Z")), Run::Action(Box::new(app::Undo)));
    }
    if state::read(cx, |l| l.can_redo()) {
        add(&edit, tr("Wiederholen"), "redo-2", Some(if cfg!(target_os = "macos") { "⇧⌘Z".to_string() } else { "Ctrl+Shift+Z".to_string() }), Run::Action(Box::new(app::Redo)));
    }
    if state::read(cx, |l| matches!(l.route(), Route::Doc(_))) && super::sync::get(cx).git.is_some() {
        add(&file, tr("Verlauf dieser Datei"), "history", None, Run::Call(Rc::new(|cx| {
            act(cx, |l| {
                l.show_inspector = true;
                l.show_versions = true;
            });
        })));
    }
    if super::sync::enabled_here(cx) {
        add(&general, tr("Sync-Einstellungen"), "settings", None, Run::Call(Rc::new(|cx| super::settings::open_sync(cx))));
    }
    if state::read(cx, |l| l.is_vault) {
        let vault = tr("Obsidian");
        let active = state::read(cx, |l| l.active_document.clone().filter(|p| crate::core::markdown::is_markdown(p) && matches!(l.route(), Route::Doc(_))));
        add(&vault, tr("Tagesnotiz öffnen"), "calendar", keys(S::DailyNote), Run::Action(Box::new(app::DailyNote)));
        add(&vault, tr("Eindeutige Notiz anlegen"), "file-plus", None, Run::Call(Rc::new(|cx| {
            act(cx, |l| l.open_unique_note());
            state::reindex(cx);
        })));
        add(&vault, tr("Zufällige Notiz öffnen"), "shuffle", None, Run::Call(Rc::new(|cx| act(cx, |l| l.open_random_note()))));
        add(&vault, tr("Notizen zeigen"), "book-open", None, Run::Call(Rc::new(|cx| act(cx, |l| l.show_notes()))));
        add(&vault, tr("Dateien ein- oder ausblenden"), "panel-left", None, Run::Call(Rc::new(|cx| super::notes_page::set_hidden(!crate::core::prefs::get().notes_explorer_hidden, cx))));
        add(&vault, tr("Graph zeigen"), "waypoints", None, Run::Call(Rc::new(|cx| act(cx, |l| l.show_graph(None)))));
        if let Some(note) = active {
            let (a, b, c, d, e) = (note.clone(), note.clone(), note.clone(), note.clone(), note);
            add(&vault, tr("Graph um diese Notiz"), "waypoints", None, Run::Call(Rc::new(move |cx| {
                let p = a.clone();
                act(cx, |l| l.show_graph(Some(p)));
            })));
            add(&vault, tr("Vorlage einfügen"), "file-plus", None, Run::Call(Rc::new(move |cx| {
                let p = b.clone();
                act(cx, |l| l.note_pick = Some((false, p)));
            })));
            add(&vault, tr("Mit anderer Notiz zusammenführen"), "merge", None, Run::Call(Rc::new(move |cx| {
                let p = c.clone();
                act(cx, |l| l.note_pick = Some((true, p)));
            })));
            add(&vault, tr("Als Folien zeigen"), "presentation", None, Run::Call(Rc::new(move |cx| {
                let p = d.clone();
                act(cx, |l| l.slides = Some(p));
            })));
            add(&vault, tr("In Obsidian öffnen"), "external-link", keys(S::OpenInObsidian), Run::Call(Rc::new(move |cx| super::vault_tools::open_in_obsidian(&e, cx))));
            add(&vault, tr("Auswahl in neue Notiz auslagern"), "scissors", None, Run::Action(Box::new(super::documents::ExtractSelection)));
        }
    }
    add(&general, tr("Einstellungen…"), "settings", Some(fixed_keys(",")), Run::Action(Box::new(app::OpenSettings)));
    add(&general, tr("Tastenkürzel anzeigen"), "keyboard", Some(fixed_keys("/")), Run::Action(Box::new(app::ShowShortcuts)));
    add(&general, tr("Nach Updates suchen…"), "download", None, Run::Action(Box::new(app::CheckForUpdates)));
    add(&general, tr("Über Schrumbi"), "info", None, Run::Action(Box::new(app::About)));
    list
}

fn fold(s: &str) -> String {
    s.to_lowercase().replace('ä', "a").replace('ö', "o").replace('ü', "u").replace('ß', "ss")
}

/// How well `query` fits `label`: whole prefix first, then a word start, then anywhere. All words must occur.
fn score(label: &str, group: &str, query: &str) -> Option<u32> {
    let (label, group, query) = (fold(label), fold(group), fold(query));
    let mut total = 0;
    for term in query.split_whitespace() {
        total += if label.starts_with(term) {
            3
        } else if label.split([' ', '/', '-', '(']).any(|w| w.starts_with(term)) {
            2
        } else if label.contains(term) || group.starts_with(term) {
            1
        } else {
            return None;
        };
    }
    Some(total)
}

/// ⌘K: find and run any action of the app.
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
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Befehl suchen …")));
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
        let mut found: Vec<(usize, u32)> = self.commands.iter().enumerate().filter_map(|(i, c)| score(&c.label, &c.group, &q).map(|s| (i, s))).collect();
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
            Run::Call(call) => {
                let call = call.clone();
                window.defer(cx, move |_, cx| call(cx));
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

fn fixed_parts(key: &str) -> Vec<String> {
    if cfg!(target_os = "macos") { vec!["⌘".to_string(), key.to_string()] } else { vec!["Ctrl".to_string(), key.to_string()] }
}

/// ⌘/: every shortcut at a glance, always showing the user's current keys.
pub struct ShortcutSheet;

impl EventEmitter<Dismissed> for ShortcutSheet {}

impl Render for ShortcutSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let section = |title: String, rows: Vec<Div>| div().flex().flex_col().gap(px(space::XS)).child(div().pb(px(space::XS)).font_weight(FontWeight::SEMIBOLD).child(title)).children(rows);
        let group = |g: Group| -> Vec<Div> { S::ALL.into_iter().filter(|a| a.group() == g).filter_map(|a| a.shortcut().map(|s| shortcut_row(a.label(), s.parts(), &t))).collect() };
        let mut general = vec![
            shortcut_row(tr("Befehl suchen …"), fixed_parts("K"), &t),
            shortcut_row(tr("Tastenkürzel anzeigen"), fixed_parts("/"), &t),
            shortcut_row(tr("Suchen"), vec!["/".to_string()], &t),
            shortcut_row(tr("Seitenleiste ein-/ausblenden"), fixed_parts("B"), &t),
            shortcut_row(tr("Widerrufen"), fixed_parts("Z"), &t),
            shortcut_row(tr("Übersicht"), fixed_parts("1"), &t),
            shortcut_row(tr("Semester"), fixed_parts("2"), &t),
            shortcut_row(tr("Leistungen"), fixed_parts("3"), &t),
            shortcut_row(tr("Inbox"), fixed_parts("4"), &t),
            shortcut_row(tr("Aufräumen"), fixed_parts("5"), &t),
        ];
        general.extend(group(Group::General));
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
                                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XL)).child(section(Group::General.label(), general)))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(px(space::XL))
                                        .child(section(Group::Documents.label(), group(Group::Documents)))
                                        .child(section(Group::Editor.label(), group(Group::Editor))),
                                ),
                        ),
                ),
            )
    }
}
