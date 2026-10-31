use std::collections::BTreeMap;

use chrono::Days;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::browser::week_badge;
use super::pages::Pages;
use super::state::act;
use super::motion::{self, Soft};
use super::theme::{Theme, font, radius, space, iconsize};
use super::widgets::*;
use crate::core::day;
use crate::core::filing::FileRole;
use crate::core::i18n::{tr, trf};
use crate::core::library::{self, Library, NewNoteRequest};
use crate::core::model::{Assignment, Module};

/// View state of a module page.
#[derive(Default)]
pub struct ModuleState {
    pub show_past: bool,
    pub show_later: bool,
    pub files_pane: bool,
    pub expanded_files: std::collections::HashSet<String>,
    pub deadlines_width: Option<f32>,
}

/// The divider between deadlines and files being dragged.
#[derive(Clone)]
pub struct SplitDrag;

impl Render for SplitDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn disclosure(id: &'static str, title: String, open: bool, view: Entity<Pages>, toggle: fn(&mut ModuleState), t: &Theme) -> Stateful<Div> {
    let hover = t.ink.opacity(0.05);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(space::S))
        .py(px(space::XS))
        .px(px(space::S))
        .mt(px(space::S))
        .rounded(px(radius::INNER))
        .cursor_pointer().key_nav()
        .soft(hover, radius::INNER)
        .child(motion::chevron("chevron", open, t.tertiary))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
        .on_click(move |_, _, cx| view.update(cx, |p, cx| {
            toggle(&mut p.module);
            cx.notify();
        }))
}

fn file_pills(lib: &Library, a: &Assignment, st: &ModuleState, view: Entity<Pages>, t: &Theme) -> Option<Div> {
    let e = lib.evaluation(a);
    if !a.tracks_files() || (e.briefs.is_empty() && e.files.is_empty()) {
        return None;
    }
    let all: Vec<(std::path::PathBuf, bool)> = e.briefs.iter().map(|p| (p.clone(), true)).chain(e.files.iter().map(|p| (p.clone(), false))).collect();
    let expanded = st.expanded_files.contains(&a.id);
    let limit = if expanded { all.len() } else { 3 };
    let (fill, hover) = (t.ink.opacity(0.05), t.ink.opacity(0.1));
    let mut row = div().pl(px(space::XXL)).flex().flex_wrap().gap(px(space::XS));
    for (path, brief) in all.iter().take(limit) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let (preview, menu_path, id) = (path.clone(), path.clone(), a.id.clone());
        let brief = *brief;
        row = row.child(
            div()
                .id(SharedString::from(format!("pill-{}", path.display())))
                .flex()
                .items_center()
                .gap(px(space::XS))
                .max_w(px(180.))
                .px(px(space::S))
                .py(px(space::XS))
                .rounded(px(radius::CONTROL))
                .bg(fill)
                .soft(hover, radius::CONTROL)
                .cursor_pointer().key_nav()
                .text_size(px(font::CAPTION))
                .tooltip(tip(if brief { trf("Aufgabenstellung: %@", &[&name]) } else { name.clone() }))
                .on_click(move |_, _, cx| super::preview::show(&preview, cx))
                .when(brief, |d| d.child(icon("clipboard-list", iconsize::XS, t.accent)).child(div().font_weight(FontWeight::MEDIUM).child(tr("Aufgabe"))))
                .when(!brief, |d| d.child(file_icon(path, iconsize::S)).child(div().truncate().child(crate::core::text::middle_ellipsis(&name, 26))))
                .context_menu(move |menu, _, _| {
                    let (p1, p2, p3, id) = (menu_path.clone(), menu_path.clone(), menu_path.clone(), id.clone());
                    let role = if brief { FileRole::Work } else { FileRole::Brief };
                    menu.item(PopupMenuItem::new(tr("Öffnen")).on_click(move |_, _, cx| act(cx, |l| l.open_file(&p1))))
                        .item(PopupMenuItem::new(super::platform::reveal_label()).on_click(move |_, _, _| library::reveal(&p2)))
                        .separator()
                        .item(PopupMenuItem::new(if brief { tr("Ist meine Abgabe") } else { tr("Ist die Aufgabenstellung") }).on_click(move |_, _, cx| act(cx, |l| l.set_role(role, &p3, &id))))
                }),
        );
    }
    if all.len() > 3 {
        let id = a.id.clone();
        let more = all.len() - 3;
        row = row.child(
            div()
                .id(SharedString::from(format!("more-files-{}", a.id)))
                .px(px(space::S))
                .py(px(space::XS))
                .rounded(px(radius::CONTROL))
                .bg(fill)
                .soft(hover, radius::CONTROL)
                .cursor_pointer().key_nav()
                .text_size(px(font::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .child(if expanded { tr("Weniger") } else { format!("+{more}") })
                .tooltip(tip(if expanded { tr("Weniger zeigen") } else { trf("Alle %lld Dateien zeigen", &[&all.len()]) }))
                .on_click(move |_, _, cx| view.update(cx, |p, cx| {
                    if !p.module.expanded_files.remove(&id) {
                        p.module.expanded_files.insert(id.clone());
                    }
                    cx.notify();
                })),
        );
    }
    Some(row)
}

fn entry(lib: &Library, a: &Assignment, st: &ModuleState, view: Entity<Pages>, t: &Theme) -> motion::Flip {
    motion::flip(format!("entry-{}", a.id), div().flex().flex_col().gap(px(space::XS)).py(px(space::XXS)).child(assignment_row(lib, a, false, true, false, t)).children(file_pills(lib, a, st, view, t)))
}

/// Deadlines with their files: attention, the next two weeks by lecture week, later and done.
pub fn deadlines(lib: &Library, m: &Module, st: &ModuleState, view: Entity<Pages>, t: &Theme) -> Div {
    let items: Vec<&Assignment> = lib.assignments.iter().filter(|a| a.module_id == m.id).collect();
    let monday = day::monday(lib.today());
    let horizon = monday + Days::new(14);
    let d = |a: &Assignment| day::local_date(a.due);
    let open: Vec<_> = items.iter().filter(|a| d(a) < monday && !lib.is_done(a)).copied().collect();
    let soon: Vec<_> = items.iter().filter(|a| d(a) >= monday && d(a) < horizon).copied().collect();
    let later: Vec<_> = items.iter().filter(|a| d(a) >= horizon).copied().collect();
    let past: Vec<_> = items.iter().filter(|a| d(a) < monday && lib.is_done(a)).copied().collect();
    let mut groups: BTreeMap<u32, Vec<&Assignment>> = BTreeMap::new();
    for a in &soon {
        groups.entry(a.week.unwrap_or(0)).or_default().push(a);
    }
    let current = lib.current_week().map(|w| w.index);
    let mut col = div().flex().flex_col().px(px(space::L)).py(px(space::S));
    if items.is_empty() {
        col = col.child(div().flex().items_center().gap(px(space::S)).py(px(space::S)).text_color(t.secondary).child(icon("inbox", iconsize::S, t.secondary)).child(tr("Noch keine Einträge. Wiederkehrende Abgaben unter Einstellungen → Module anlegen oder mit + einen Eintrag hinzufügen.")));
    }
    if !open.is_empty() {
        col = col.child(div().flex().items_center().gap(px(space::S)).mt(px(space::S)).py(px(space::XS)).font_weight(FontWeight::SEMIBOLD).text_color(t.red).child(icon("circle-alert", iconsize::XS, t.red)).child(tr("Braucht Aufmerksamkeit")));
        for a in open.iter().rev() {
            col = col.child(entry(lib, a, st, view.clone(), t));
        }
    }
    for (week, entries) in groups {
        let mut header = div().flex().items_center().gap(px(space::S)).mt(px(space::S)).py(px(space::XS));
        if week > 0 {
            header = header.child(div().font_weight(FontWeight::SEMIBOLD).child(format!("W{week:02}")));
            if let Some(kw) = entries.first().and_then(|a| a.iso_week) {
                header = header.child(div().text_color(t.secondary).child(format!("KW{kw:02}")));
            }
            if current == Some(week) {
                header = header.child(week_badge(tr("Diese Woche"), true, t));
            } else if current.is_some_and(|c| c + 1 == week) {
                header = header.child(week_badge(tr("Nächste Woche"), false, t));
            }
        } else {
            header = header.child(div().font_weight(FontWeight::SEMIBOLD).child(tr("Ohne Vorlesungswoche")));
        }
        col = col.child(motion::flip(format!("week-{week}"), header));
        for a in entries {
            col = col.child(entry(lib, a, st, view.clone(), t));
        }
    }
    if !later.is_empty() {
        col = col.child(motion::flip("later", disclosure("later", trf("Später (%lld)", &[&later.len()]), st.show_later, view.clone(), |s| s.show_later = !s.show_later, t)));
        col = col.child(motion::reveal("later-rows", st.show_later, div().flex().flex_col().children(later.iter().map(|a| entry(lib, a, st, view.clone(), t)))));
    }
    if !past.is_empty() {
        col = col.child(motion::flip("past", disclosure("past", trf("Erledigt (%lld)", &[&past.len()]), st.show_past, view.clone(), |s| s.show_past = !s.show_past, t)));
        col = col.child(motion::reveal("past-rows", st.show_past, div().flex().flex_col().children(past.iter().rev().map(|a| entry(lib, a, st, view.clone(), t)))));
    }
    col
}

/// Colored code badge, name, progress, next deadline and the folder, filing, new-file and link buttons.
pub fn header(lib: &Library, m: &Module, split: bool, width: f32, st: &ModuleState, view: Entity<Pages>, t: &Theme) -> Div {
    let (done, total) = lib.module_progress(&m.id);
    let upcoming = lib.upcoming();
    let next = upcoming.iter().find(|a| a.module_id == m.id && !a.optional).or_else(|| upcoming.iter().find(|a| a.module_id == m.id)).copied();
    let color = t.palette(&m.color);
    let folder = lib.root.as_ref().map(|r| r.join(&m.folder)).unwrap_or_default();
    let mut stats = div().flex().items_center().gap(px(space::S)).text_size(px(font::CALLOUT)).text_color(t.secondary);
    if total > 0 {
        stats = stats.child(div().w(px(100.)).child(progress_bar(done as f32 / total as f32, color, t))).child(div().font_features(tabular()).child(format!("{done}/{total}")));
    }
    if let Some(a) = next.filter(|_| width >= 640.) {
        let due = day::due(a.due, lib.now);
        let text = if width >= 860. { trf("Als Nächstes: %@, %@", &[&a.title, &due]) } else { format!("{}, {due}", a.title) };
        stats = stats.child(div().text_color(t.tertiary).child("·")).child(div().truncate().child(text));
    }
    let icon_only = !split;
    let button = |id: &'static str, icon_name: &'static str, label: String, help: String| {
        let b = Button::new(id).small().icon(menu_icon(icon_name)).tooltip(help);
        if icon_only { b } else { b.label(label) }
    };
    let (f1, code, mid) = (folder.clone(), m.code.clone(), m.id.clone());
    let mut actions = div()
        .flex()
        .items_center()
        .gap(px(space::S))
        .child(button("module-folder", "folder", tr("Ordner"), tr("Modulordner im Finder öffnen")).on_click(move |_, _, _| library::open_path(&f1)))
        .child(button("module-file", "inbox", tr("Ablegen"), trf("Dateien in %@ ablegen", &[&code])).on_click(|_, _, cx| super::platform::pick_files_for_filing(None, cx)))
        .child(button("module-new", "file-plus", tr("Neue Datei"), trf("Notiz, Textdatei oder Ordner in %@ anlegen", &[&m.code])).on_click(move |_, _, cx| {
            let id = mid.clone();
            act(cx, |l| l.new_note = Some(NewNoteRequest { module_id: Some(id), folder: None }))
        }));
    for (i, link) in m.links.iter().filter(|k| k.url().is_some()).enumerate() {
        let url = link.url().map(|u| u.to_string()).unwrap_or_default();
        let label = if link.title.is_empty() { tr("Link") } else { link.title.clone() };
        let b = Button::new(("module-link", i)).small().icon(menu_icon("link")).tooltip(link.address.clone());
        actions = actions.child(if icon_only { b } else { b.label(label) }.on_click(move |_, _, _| {
            let _ = opener::open(&url);
        }));
    }
    let pane = (!split).then(|| {
        let (v1, v2) = (view.clone(), view);
        div()
            .flex()
            .rounded(px(radius::INNER))
            .bg(t.ink.opacity(0.06))
            .p(px(space::XXS))
            .child(segment("pane-deadlines", tr("Abgaben"), !st.files_pane, t).on_click(move |_, _, cx| v1.update(cx, |p, cx| {
                p.module.files_pane = false;
                cx.notify();
            })))
            .child(segment("pane-files", tr("Dateien"), st.files_pane, t).on_click(move |_, _, cx| v2.update(cx, |p, cx| {
                p.module.files_pane = true;
                cx.notify();
            })))
    });
    let code: String = m.code.clone();
    div()
        .flex()
        .items_center()
        .gap(px(space::L))
        .px(px(space::XL))
        .py(px(space::M))
        .border_b_1()
        .border_color(t.separator)
        .child(div().size(px(44.)).flex_none().rounded(px(radius::CARD)).bg(color).flex().items_center().justify_center().p(px(space::XS)).text_size(px(if code.chars().count() > 3 { 10. } else { 13. })).font_weight(FontWeight::BOLD).text_color(white()).child(code))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XS)).child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).truncate().child(m.name.clone())).child(stats))
        .children(pane)
        .child(actions)
}

pub fn segment(id: &'static str, label: String, active: bool, t: &Theme) -> Stateful<Div> {
    div().id(id).px(px(space::M)).py(px(space::XXS)).rounded(px(radius::INNER)).text_size(px(font::CALLOUT)).cursor_pointer().key_nav().when(active, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm().font_weight(FontWeight::MEDIUM)).child(label)
}
