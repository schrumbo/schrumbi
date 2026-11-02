use chrono::{Local, TimeZone};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::time_field::TimePrecision;
use gpui_kit::component::Sizable;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::{self, act};
use super::motion::Soft;
use super::theme::{Theme, font, radius, space, iconsize};
use super::widgets::*;
use crate::core::day;
use crate::core::filing::FileRole;
use crate::core::i18n::{tr, trf};
use crate::core::library::{self, Library};
use crate::core::model::{Assignment, Kind, Status};

/// Details of the selected assignment: status actions, files, drop zone, note and the editor for manual entries.
pub struct Inspector {
    graph_preview: Option<(std::path::PathBuf, std::rc::Rc<super::markdown_view::ParsedNote>)>,
    shown: Option<String>,
    /// The selection seen last; `shown` keeps the assignment while the column closes.
    seen: Option<String>,
    /// What the column showed while open, kept while it closes.
    kind: Option<library::InspectorKind>,
    /// Title and due as loaded into the fields; only what the user changed since is written back.
    loaded: Option<(String, Option<chrono::DateTime<chrono::Utc>>)>,
    note: Entity<TextareaState>,
    title: Entity<InputState>,
    due: Entity<DatePickerState>,
    syncing: bool,
    doc_side: Entity<super::doc_side::DocSide>,
    _subs: Vec<Subscription>,
}

pub fn datetime_picker(window: &mut Window, cx: &mut App) -> Entity<DatePickerState> {
    cx.new(|cx| DatePickerState::new(window, cx).time_precision(TimePrecision::Minute).date_format(if crate::core::i18n::lang() == crate::core::i18n::Lang::En { "%m/%d/%Y %I:%M %p" } else { "%d.%m.%Y %H:%M" }))
}

pub fn picked(state: &Entity<DatePickerState>, cx: &App) -> Option<chrono::DateTime<chrono::Utc>> {
    state.read(cx).date_time().start().and_then(|n| match crate::core::day::at(n.date(), n.time()) {
        Some(t) => Some(t.with_timezone(&chrono::Utc)),
        None => Local.from_local_datetime(&n).earliest().map(|t| t.with_timezone(&chrono::Utc)),
    })
}

impl Inspector {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let note = cx.new(|cx| TextareaState::new(window, cx).placeholder(tr("Aufgabenstellung, Links, Gedanken …")).auto_grow(4, 20));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Titel")));
        let due = datetime_picker(window, cx);
        let lib = state::lib(cx);
        let subs = vec![
            cx.observe(&lib, |_, _, cx| cx.notify()),
            cx.subscribe(&note, |this, note, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) && !this.syncing
                    && let Some(id) = this.shown.clone()
                {
                    let text = note.read(cx).value().to_string();
                    state::lib(cx).update(cx, |l, _| l.set_note(&id, text));
                    state::schedule_notes_save(cx);
                }
            }),
            cx.subscribe(&title, |this, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.save_manual(cx);
                }
            }),
            cx.subscribe(&due, |this, _, _: &DatePickerEvent, cx| this.save_manual(cx)),
        ];
        let doc_side = cx.new(|cx| super::doc_side::DocSide::new(window, cx));
        Self { graph_preview: None, shown: None, seen: None, kind: None, loaded: None, note, title, due, syncing: false, doc_side, _subs: subs }
    }

    fn save_manual(&mut self, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        let Some(id) = self.shown.clone() else { return };
        let Some((loaded_title, loaded_due)) = self.loaded.clone() else { return };
        let Some(mut m) = state::read(cx, |l| l.assignment(&id).and_then(|a| l.manual_for(a)).cloned()) else { return };
        let original = m.clone();
        let title = self.title.read(cx).value().trim().to_string();
        let due = picked(&self.due, cx);
        if !title.is_empty() && title != loaded_title {
            m.title = title.clone();
        }
        if let Some(d) = due.filter(|d| Some(*d) != loaded_due) {
            m.due = d;
        }
        self.loaded = Some((title, due));
        if m != original {
            act(cx, |l| l.update_manual(m));
        }
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = state::read(cx, |l| l.selection().map(String::from));
        if selected == self.seen {
            return;
        }
        if self.seen.is_some() {
            self.save_manual(cx);
        }
        self.seen = selected.clone();
        if selected.is_none() {
            return;
        }
        self.shown = selected.clone();
        self.syncing = true;
        let (note, manual) = state::read(cx, |l| {
            let a = selected.as_deref().and_then(|id| l.assignment(id));
            (selected.as_deref().map(|id| l.note(id)).unwrap_or_default(), a.and_then(|a| l.manual_for(a)).cloned())
        });
        self.note.update(cx, |s, cx| s.set_value(note, window, cx));
        self.loaded = None;
        if let Some(m) = manual {
            self.title.update(cx, |s, cx| s.set_value(m.title.clone(), window, cx));
            let local = day::wall(m.due.with_timezone(&Local));
            self.due.update(cx, |s, cx| s.set_date_time(local, window, cx));
            self.loaded = Some((m.title.trim().to_string(), picked(&self.due, cx)));
        }
        self.syncing = false;
    }
}

fn info_row(icon_name: &str, text: String, tint: Option<Hsla>, t: &Theme) -> Div {
    div().flex().items_center().gap(px(space::S)).child(div().w(px(18.)).flex().justify_center().child(icon(icon_name, iconsize::S, tint.unwrap_or(t.secondary)))).child(div().text_color(tint.unwrap_or(t.primary)).child(text))
}

fn status_color(s: Status, t: &Theme) -> Hsla {
    match s {
        Status::Open => t.secondary,
        Status::InProgress => t.orange,
        Status::Ready => t.blue,
        Status::Uploaded | Status::Done => t.green,
        Status::Skipped => t.gray,
        Status::Overdue => t.red,
    }
}

fn file_line(path: &std::path::Path, warning: bool, extra: Vec<(String, Box<dyn Fn(&mut App)>)>, t: &Theme) -> impl IntoElement {
    let (open, menu_path) = (path.to_path_buf(), path.to_path_buf());
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let extra = std::rc::Rc::new(extra);
    div()
        .id(SharedString::from(format!("file-{}", path.display())))
        .flex()
        .items_center()
        .gap(px(space::S))
        .py(px(space::XXS))
        .rounded(px(radius::SMALL))
        .soft(t.ink.opacity(0.05), radius::SMALL)
        .tooltip(tip(tr("Doppelklick zum Öffnen")))
        .on_click(move |e, _, cx| {
            if e.click_count() >= 2 {
                act(cx, |l| l.open_file(&open));
            }
        })
        .child(file_icon(path, iconsize::M))
        .child(div().flex_1().min_w_0().truncate().child(name))
        .when(warning, |d| d.child(icon("triangle-alert", iconsize::XS, t.orange)))
        .context_menu(move |menu, _, _| {
            let mut menu = file_menu(menu, menu_path.clone());
            if !extra.is_empty() {
                menu = menu.separator();
            }
            for (i, (label, _)) in extra.iter().enumerate() {
                let extra = extra.clone();
                menu = menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, _, cx| (extra[i].1)(cx)));
            }
            menu
        })
}

impl Render for Inspector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let t = Theme::get(cx);
        let lib = state::lib(cx);
        let l = lib.read(cx);
        let open = l.show_inspector();
        if open {
            self.kind = l.inspector_kind();
        }
        match self.kind {
            Some(library::InspectorKind::Document) => return self.doc_side.clone().into_any_element(),
            Some(library::InspectorKind::Graph) => {
                let path = if open { l.graph_selection.clone() } else { self.graph_preview.as_ref().map(|(p, _)| p.clone()) };
                return match path {
                    Some(path) => graph_card(&path, &t, &mut self.graph_preview).into_any_element(),
                    None => unavailable("waypoints", tr("Keine Notiz ausgewählt"), tr("Klicke einen Punkt an, um die Notiz hier zu sehen. Doppelklick öffnet sie."), &t).into_any_element(),
                };
            }
            Some(library::InspectorKind::Assignments) => {}
            None => return div().into_any_element(),
        }
        let shown = if open { l.selection() } else { self.shown.as_deref() };
        let Some(a) = shown.and_then(|id| l.assignment(id)).cloned() else {
            return unavailable("panel-right", tr("Kein Eintrag ausgewählt"), tr("Wähle eine Abgabe, um Details, Dateien und Notizen zu sehen."), &t).into_any_element();
        };
        let body = details(l, &a, &t, &self.note, self.title.clone(), self.due.clone());
        div().id("inspector-scroll").size_full().overflow_y_scrollbar().child(body).into_any_element()
    }
}

fn details(l: &Library, a: &Assignment, t: &Theme, note: &Entity<TextareaState>, title: Entity<InputState>, due: Entity<DatePickerState>) -> Div {
    let e = l.evaluation(a);
    let module = l.module(a);
    let tint = t.palette(module.map_or("blue", |m| m.color.as_str()));
    let id = a.id.clone();

    let mut head = div().flex().items_center().gap(px(space::S)).child(module_chip(module.map_or("?", |m| m.code.as_str()), tint));
    if let Some(w) = l.due_week(a) {
        let origin = a.week.filter(|o| *o != w).map(|o| format!(" · {}", trf("für W%@", &[&format!("{o:02}")]))).unwrap_or_default();
        head = head.child(caption(format!("W{w:02}{origin}"), t));
    }
    if a.kind != Kind::Submission {
        head = head.child(div().flex().items_center().gap(px(space::XS)).child(icon(a.kind.icon(), iconsize::XS, t.secondary)).child(caption(a.kind.label(), t)));
    }
    let mut links = div().flex().flex_wrap().items_center().gap(px(space::S));
    if let Some(m) = module.filter(|m| m.name != m.code && !m.name.is_empty()) {
        links = links.child(div().max_w_full().truncate().text_color(t.secondary).child(m.name.clone()));
    }
    for (i, link) in module.map(|m| m.links.clone()).unwrap_or_default().into_iter().filter(|k| k.url().is_some()).enumerate() {
        let url = link.url().map(|u| u.to_string()).unwrap_or_default();
        let label = if link.title.is_empty() { tr("Link") } else { link.title.clone() };
        links = links.child(link_button(("link", i), label, t).flex().items_center().gap(px(space::XS)).child(icon("link", iconsize::XS, t.accent)).tooltip(tip(link.address.clone())).on_click(move |_, _, _| {
            let _ = opener::open(&url);
        }));
    }

    let due_row: AnyElement = if a.is_manual() {
        info_row("calendar", day::due(a.due, l.now), None, t).into_any_element()
    } else {
        let move_id = id.clone();
        div().id("due-row").w_full().rounded(px(radius::INNER)).soft(t.ink.opacity(0.06), radius::INNER).cursor_pointer().key_nav().tooltip(tip(tr("Termin verschieben"))).on_click(move |_, _, cx| act(cx, |l| l.reschedule_id = Some(move_id.clone()))).child(info_row("calendar", day::due(a.due, l.now), None, t)).into_any_element()
    };
    let mut info = div().flex().flex_col().gap(px(space::S)).child(due_row).child(info_row(
        "clock",
        day::countdown(a.due, l.now),
        l.needs_action(a).then_some(t.red),
        t,
    ));
    let status_text = e.status.label() + &if e.is_override { tr(" · manuell") } else { String::new() };
    info = info.child(info_row(e.status.icon(), status_text, Some(status_color(e.status, t)), t));
    if a.optional {
        info = info.child(info_row("circle-help", tr("Optional – nicht jede Woche"), None, t));
    }
    if a.is_moved {
        info = info.child(info_row("calendar-clock", tr("Termin verschoben"), Some(t.orange), t));
    }

    let primary = if !e.status.is_closed() && a.upload && !e.files.is_empty() {
        let (up, mark) = (id.clone(), id.clone());
        let help = match module.and_then(|m| m.upload_link()) {
            Some(link) => trf("Öffnet %@ und stellt die Dateien zum Ziehen bereit", &[&link.title]),
            None => tr("Dateien zum Hochladen bereitstellen. Tipp: Moodle-Link in den Moduleinstellungen hinterlegen."),
        };
        div()
            .flex()
            .flex_1()
            .gap(px(space::S))
            .child(div().flex_1().child(Button::new("upload").primary().w_full().icon(menu_icon("file-up")).label(tr("Hochladen")).tooltip(help).on_click(move |_, _, cx| super::upload::begin(&up, cx))))
            .child(Button::new("mark-uploaded").icon(menu_icon("check")).tooltip(tr("Als hochgeladen markieren")).on_click(move |_, _, cx| act(cx, |l| l.set_status(Some(Status::Uploaded), &mark, true))))
    } else if !e.status.is_closed() {
        let done = id.clone();
        div().flex_1().child(Button::new("done").primary().w_full().icon(menu_icon("check")).label(tr("Erledigt")).on_click(move |_, _, cx| act(cx, |l| l.toggle_done(&done))))
    } else {
        let reset = id.clone();
        div().flex_1().child(Button::new("reset").w_full().icon(menu_icon("undo-2")).label(tr("Zurücksetzen")).on_click(move |_, _, cx| act(cx, |l| l.set_status(None, &reset, true))))
    };
    let menu_id = id.clone();
    let actions = div().flex().items_center().gap(px(space::S)).child(primary).child(Button::new("more").icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu(move |menu, _, cx| status_menu(menu, &menu_id, cx)));

    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(space::L))
        .p(px(space::L))
        .child(div().flex().flex_col().gap(px(space::S)).child(head).child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(a.title.clone())).child(links))
        .child(info)
        .child(actions);

    let mut notes: Vec<std::path::PathBuf> = crate::core::vault::index::current().notes.keys().filter(|p| p.starts_with(&a.folder)).cloned().collect();
    notes.sort();
    notes.truncate(20);
    if !notes.is_empty() {
        let rows = notes.into_iter().enumerate().map(|(i, path)| {
            let open = path.clone();
            file_row(("assignment-note", i), &path, path.file_stem().unwrap_or_default().to_string_lossy().to_string(), t).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open)))
        });
        col = col.child(div().flex().flex_col().gap(px(space::S)).child(div().font_weight(FontWeight::BOLD).child(tr("Notizen"))).child(div().flex().flex_col().children(rows)));
    }

    if a.tracks_files() {
        let folder = a.folder.clone();
        let mut files = div().flex().flex_col().gap(px(space::S)).child(
            div()
                .flex()
                .items_center()
                .child(div().font_weight(FontWeight::BOLD).child(tr("Dateien")))
                .child(div().flex_1())
                .child(Button::new("open-folder").ghost().small().icon(menu_icon("folder")).tooltip(folder.display().to_string()).on_click(move |_, _, _| library::open_path(&folder))),
        );
        if !e.briefs.is_empty() {
            let mut briefs = div()
                .flex()
                .flex_col()
                .gap(px(space::XS))
                .p(px(space::M))
                .rounded(px(radius::CONTROL))
                .bg(t.accent.opacity(0.06))
                .child(div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(icon("clipboard-list", iconsize::XS, t.secondary)).child(tr("Aufgabenstellung")));
            for p in &e.briefs {
                let (p2, id2) = (p.clone(), id.clone());
                briefs = briefs.child(file_line(p, false, vec![(tr("Ist meine Abgabe"), Box::new(move |cx: &mut App| act(cx, |l| l.set_role(FileRole::Work, &p2, &id2))))], t));
            }
            files = files.child(briefs);
        }
        for p in &e.files {
            let warn = e.misnamed.contains(p);
            let (p2, id2) = (p.clone(), id.clone());
            let mut extra: Vec<(String, Box<dyn Fn(&mut App)>)> = vec![(tr("Ist die Aufgabenstellung"), Box::new(move |cx: &mut App| act(cx, |l| l.set_role(FileRole::Brief, &p2, &id2))))];
            if l.is_linked(p, &id) {
                let (p3, id3) = (p.clone(), id.clone());
                extra.push((tr("Zuordnung lösen"), Box::new(move |cx: &mut App| act(cx, |l| l.unlink(&p3, &id3)))));
            }
            files = files.child(file_line(p, warn, extra, t));
            if warn && let Some(s) = l.suggested_name(p, a) {
                let (p4, s2) = (p.clone(), s.clone());
                files = files.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(space::S))
                        .pl(px(space::XL))
                        .child(div().flex_1().text_size(px(font::CAPTION)).font_family(Theme::mono_family()).text_color(t.secondary).line_clamp(2).child(s))
                        .child(Button::new(SharedString::from(format!("rename-{}", p.display()))).small().label(tr("Umbenennen")).on_click(move |_, _, cx| act(cx, |l| l.rename(&p4, &s2)))),
                );
            }
        }
        let (title_text, folder2) = (a.title.clone(), a.folder.clone());
        files = files.child(
            link_button("note-for-assignment", tr("Markdown-Datei anlegen"), t)
                .flex()
                .items_center()
                .gap(px(space::XS))
                .text_size(px(font::CALLOUT))
                .child(icon("square-pen", iconsize::XS, t.accent))
                .tooltip(tip(tr("Markdown-Notiz im Abgabeordner anlegen; zählt als „In Arbeit“, nicht als fertig")))
                .on_click(move |_, _, cx| {
                    if let Some(p) = act(cx, |l| l.create_note(&title_text, &folder2, "md")) {
                        act(cx, |l| l.open_document(&p));
                    }
                }),
        );
        files = files.child(drop_zone(id.clone(), !e.files.is_empty(), t));
        col = col.child(files);
    }

    col = col.child(div().flex().flex_col().gap(px(space::S)).child(div().flex().items_baseline().gap(px(space::S)).child(div().font_weight(FontWeight::BOLD).child(tr("Kurznotiz"))).child(caption(tr("wird automatisch gesichert"), t))).child(Textarea::new(note)));

    if l.manual_for(a).is_some() {
        let remove = id.clone();
        col = col.child(
            div()
                .flex()
                .flex_col()
                .gap(px(space::S))
                .child(div().font_weight(FontWeight::BOLD).child(tr("Bearbeiten")))
                .child(Input::new(&title))
                .child(div().flex().items_center().gap(px(space::S)).child(div().text_color(t.secondary).child(tr("Fällig"))).child(div().flex_1().child(DatePicker::new(&due))))
                .child(div().flex().child(div().flex_1()).child(Button::new("delete-manual").ghost().danger().icon(menu_icon("trash-2")).label(tr("Löschen")).on_click(move |_, _, cx| act(cx, |l| l.remove_manual(&remove))))),
        );
    }
    col
}

fn drop_zone(id: String, has_files: bool, t: &Theme) -> impl IntoElement {
    let (accent, secondary) = (t.accent, t.secondary);
    let (drop_id, list_id, pick_id) = (id.clone(), id.clone(), id);
    div()
        .id("drop-zone")
        .flex()
        .flex_col()
        .items_center()
        .gap(px(space::S))
        .py(px(space::L))
        .rounded(px(radius::CARD))
        .border(px(1.5))
        .border_dashed()
        .border_color(secondary.opacity(0.4))
        .text_color(secondary)
        .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_color(accent).bg(accent.opacity(0.08)).text_color(accent))
        .drag_over::<super::browser::DraggedFile>(move |s, _, _, _| s.border_color(accent).bg(accent.opacity(0.08)).text_color(accent))
        .on_drop(move |paths: &ExternalPaths, _, cx| {
            let paths = paths.paths().to_vec();
            act(cx, |l| l.request_filing(paths, Some(drop_id.clone())));
        })
        .on_drop(move |f: &super::browser::DraggedFile, _, cx| {
            let path = f.0.clone();
            act(cx, |l| l.request_filing(vec![path], Some(list_id.clone())));
        })
        .child(icon("arrow-down-to-line", iconsize::L, secondary))
        .child(div().text_size(px(font::CALLOUT)).child(if has_files { tr("Weitere Datei hierher ziehen") } else { tr("Datei hierher ziehen") }))
        .child(link_button("pick-files", tr("oder auswählen…"), t).text_size(px(font::CAPTION)).on_click(move |_, _, cx| super::platform::pick_files_for_filing(Some(pick_id.clone()), cx)))
}

/// A note picked in the graph: readable here, and one click from being opened.
fn graph_card(path: &std::path::Path, t: &Theme, kept: &mut Option<(std::path::PathBuf, std::rc::Rc<super::markdown_view::ParsedNote>)>) -> impl IntoElement {
    let note = match kept {
        Some((known, note)) if known == path => note.clone(),
        _ => {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            let body = &text[crate::core::vault::frontmatter::body_offset(&text).min(text.len())..];
            let cut = body.char_indices().nth(4000).map_or(body.len(), |(i, _)| i);
            let note = std::rc::Rc::new(super::markdown_view::parse(&body[..cut]));
            *kept = Some((path.to_path_buf(), note.clone()));
            note
        }
    };
    let (open, around) = (path.to_path_buf(), path.to_path_buf());
    div()
        .id("graph-card")
        .size_full()
        .overflow_y_scrollbar()
        .p(px(space::L))
        .flex()
        .flex_col()
        .gap(px(space::M))
        .child(div().font_weight(FontWeight::BOLD).text_size(px(font::TITLE3)).child(path.file_stem().unwrap_or_default().to_string_lossy().to_string()))
        .child(
            div()
                .flex()
                .gap(px(space::S))
                .child(Button::new("graph-open").primary().small().label(tr("Öffnen")).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open))))
                .child(Button::new("graph-around").small().label(tr("Graph um diese Notiz")).on_click(move |_, _, cx| act(cx, |l| l.show_graph(Some(around.clone()))))),
        )
        .child(super::markdown_view::render(&note, path.parent(), None, t))
}
