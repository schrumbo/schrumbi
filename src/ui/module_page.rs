use std::collections::BTreeMap;

use chrono::{Days, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::browser::week_title;
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
    pub expanded_files: std::collections::HashSet<String>,
    /// Takes the keyboard while the page is not searching its files.
    pub focus: Option<FocusHandle>,
    pub scroll: ScrollHandle,
}

/// The divider between two documents being dragged.
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

/// The deadlines in the order the list shows them, for walking through by keyboard.
pub fn deadline_order(lib: &Library, m: &Module, st: &ModuleState) -> Vec<String> {
    let items: Vec<&Assignment> = lib.assignments.iter().filter(|a| a.module_id == m.id).collect();
    let monday = day::monday(lib.today());
    let horizon = monday + Days::new(14);
    let d = |a: &Assignment| day::local_date(a.due);
    let mut out: Vec<String> = items.iter().rev().filter(|a| d(a) < monday && !lib.is_done(a)).map(|a| a.id.clone()).collect();
    let mut groups: BTreeMap<NaiveDate, Vec<&Assignment>> = BTreeMap::new();
    for a in items.iter().filter(|a| d(a) >= monday && d(a) < horizon) {
        groups.entry(day::monday(d(a))).or_default().push(a);
    }
    out.extend(groups.into_values().flatten().map(|a| a.id.clone()));
    if st.show_later {
        out.extend(items.iter().filter(|a| d(a) >= horizon).map(|a| a.id.clone()));
    }
    if st.show_past {
        out.extend(items.iter().rev().filter(|a| d(a) < monday && lib.is_done(a)).map(|a| a.id.clone()));
    }
    out
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
    let mut groups: BTreeMap<NaiveDate, Vec<&Assignment>> = BTreeMap::new();
    for a in &soon {
        groups.entry(day::monday(d(a))).or_default().push(a);
    }
    let weeks = lib.weeks();
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
        let lecture = weeks.iter().find(|w| w.monday == week).map(|w| w.index);
        let header = div().flex().items_center().gap(px(space::S)).mt(px(space::S)).py(px(space::XS)).child(week_title(week, lecture, lib.today(), t));
        col = col.child(motion::flip(format!("week-{week}"), header));
        for a in entries {
            col = col.child(entry(lib, a, st, view.clone(), t));
        }
    }
    if !later.is_empty() {
        col = col.child(motion::flip("later", disclosure("later", trf("Später (%lld)", &[&later.len()]), st.show_later, view.clone(), |s| s.show_later = !s.show_later, t)));
        col = col.child(div().mx(px(-space::S)).child(motion::reveal("later-rows", st.show_later, div().px(px(space::S)).flex().flex_col().children(later.iter().map(|a| entry(lib, a, st, view.clone(), t))))));
    }
    if !past.is_empty() {
        col = col.child(motion::flip("past", disclosure("past", trf("Erledigt (%lld)", &[&past.len()]), st.show_past, view.clone(), |s| s.show_past = !s.show_past, t)));
        col = col.child(div().mx(px(-space::S)).child(motion::reveal("past-rows", st.show_past, div().px(px(space::S)).flex().flex_col().children(past.iter().rev().map(|a| entry(lib, a, st, view.clone(), t))))));
    }
    col
}

/// Who the module is and what can always be done: name and state on the left, the two things done most on the right.
pub fn header(lib: &Library, m: &Module, width: f32, t: &Theme) -> Div {
    let (done, total) = lib.module_progress(&m.id);
    let color = t.palette(&m.color);
    let folder = lib.root.as_ref().map(|r| r.join(&m.folder)).unwrap_or_default();
    let mut facts: Vec<String> = Vec::new();
    if let Some(semester) = lib.config.as_ref().and_then(|c| c.semesters.iter().find(|s| s.id == m.semester)) {
        facts.push(semester.name.clone());
    }
    if let Some(ects) = m.ects {
        facts.push(format!("{} ECTS", super::fields::format_number(ects, 1)));
    }
    if total > 0 {
        facts.push(trf("%@ von %@ erledigt", &[&done.to_string(), &total.to_string()]));
    }
    let code = m.code.clone();
    let compact = width < 760.;
    let (file_code, note_id, edit_id, reveal) = (m.code.clone(), m.id.clone(), m.id.clone(), folder.clone());
    let next_entry = lib.next_in_module(&m.id);
    let actions = div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::S))
        .child(Button::new("module-new").small().icon(menu_icon("file-plus")).when(!compact, |b| b.label(tr("Neue Notiz"))).tooltip(trf("Notiz, Textdatei oder Ordner in %@ anlegen (N)", &[&m.code])).on_click(move |_, _, cx| {
            let id = note_id.clone();
            act(cx, |l| l.new_note = Some(NewNoteRequest { module_id: Some(id), folder: None }))
        }))
        .child(Button::new("module-file").small().primary().icon(menu_icon("inbox")).when(!compact, |b| b.label(tr("Ablegen"))).tooltip(trf("Dateien in %@ ablegen (F)", &[&file_code])).on_click(move |_, _, cx| super::platform::pick_files_for_filing(next_entry.clone(), cx)))
        .child(Button::new("module-more").small().ghost().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu(move |menu, _, _| {
            let (reveal, edit_id) = (reveal.clone(), edit_id.clone());
            menu.item(PopupMenuItem::new(tr("Eintrag hinzufügen")).icon(menu_icon("plus")).on_click(|_, _, cx| act(cx, |l| l.show_quick_add = true)))
                .item(PopupMenuItem::new(tr("Modulordner öffnen")).icon(menu_icon("folder")).on_click(move |_, _, _| library::open_folder(&reveal)))
                .separator()
                .item(PopupMenuItem::new(tr("Modul bearbeiten")).icon(menu_icon("settings")).on_click(move |_, _, cx| super::settings::open_module(&edit_id, cx)))
        }));
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(space::M))
        .px(px(space::XL))
        .py(px(space::L))
        .border_b_1()
        .border_color(t.separator)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::L))
                .child(div().size(px(48.)).flex_none().rounded(px(radius::CARD)).bg(color).flex().items_center().justify_center().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(white()).child(code))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(space::XXS))
                        .child(div().text_size(px(font::TITLE)).font_weight(FontWeight::BOLD).truncate().child(m.name.clone()))
                        .child(div().flex().items_center().gap(px(space::M)).text_size(px(font::CALLOUT)).text_color(t.secondary).when(!facts.is_empty(), |d| d.child(div().truncate().child(facts.join(" · ")))).when(total > 0 && width >= 900., |d| d.child(div().w(px(120.)).flex_none().child(progress_bar(done as f32 / total as f32, color, t))))),
                )
                .child(actions),
        )
}

fn more_link(id: &'static str, label: String, t: &Theme) -> Stateful<Div> {
    div().id(id).flex_none().cursor_pointer().key_nav().text_size(px(font::CALLOUT)).text_color(t.accent_text).hover({ let c = t.accent_text.opacity(0.8); move |s| s.text_color(c) }).child(label)
}

fn section(title: impl IntoElement, link: Option<Stateful<Div>>) -> Div {
    div().flex().items_baseline().justify_between().gap(px(space::M)).child(title).children(link)
}

/// The main column: every deadline with what belongs to it, and what was worked on last.
fn overview_main(lib: &Library, m: &Module, st: &ModuleState, view: Entity<Pages>, t: &Theme) -> Div {
    let all = lib.assignments.iter().filter(|a| a.module_id == m.id).count();
    let waiting = lib.assignments.iter().filter(|a| a.module_id == m.id && day::local_date(a.due) < lib.today() && !lib.is_done(a)).count();
    let edit_id = m.id.clone();
    let list: AnyElement = if all == 0 {
        empty_card(
            "calendar-plus",
            tr("Noch keine Abgaben"),
            tr("Lege eine einzelne Abgabe an, oder eine Regel, die sich jede Vorlesungswoche wiederholt."),
            div()
                .flex()
                .gap(px(space::S))
                .child(Button::new("first-entry").small().label(tr("Eintrag hinzufügen")).on_click(|_, _, cx| act(cx, |l| l.show_quick_add = true)))
                .child(Button::new("first-rule").small().ghost().label(tr("Regel anlegen")).on_click(move |_, _, cx| super::settings::open_module(&edit_id, cx))),
            t,
        )
    } else {
        grouped(t).child(deadlines(lib, m, st, view, t)).into_any_element()
    };
    let title = div().flex().items_center().gap(px(space::S)).child(section_title(tr("Abgaben"))).when(waiting > 0, |d| d.child(div().px(px(space::XS)).rounded_full().bg(t.red.opacity(0.16)).text_color(t.red).text_size(px(font::CAPTION)).font_features(tabular()).child(waiting.to_string())));
    let folder = lib.root.as_ref().map(|r| r.join(&m.folder)).unwrap_or_default();
    let recent = super::today::recent_notes_in(Some(&folder), 6);
    let count = recent.len();
    let now = chrono::Local::now().timestamp() as f64;
    let hover_rows = recent.into_iter().enumerate().map(|(i, (path, at))| {
        let (open, menu) = (path.clone(), path.clone());
        inset_file_row(("module-recent", i), &path, path.file_stem().unwrap_or_default().to_string_lossy().to_string(), t).child(caption(super::notes_page::changed_ago(now - at), t).flex_none()).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open))).context_menu(move |menu_box, _, _| file_menu(menu_box, menu.clone()))
    });
    let new_id = m.id.clone();
    let module_folder = m.folder.clone();
    let all_files = more_link("to-files", tr("Alle Dateien"), t).tooltip(tip(tr("Im Dateibaum zeigen"))).on_click(move |_, _, cx| {
        let folder = module_folder.clone();
        act(cx, |l| {
            l.reveal_folder = Some(folder);
            l.set_route(library::Route::Notes);
        })
    });
    let files: AnyElement = if count == 0 {
        empty_card(
            "file-plus",
            tr("Noch keine Notizen"),
            tr("Schreibe die erste Notiz oder ziehe Dateien hierher."),
            Button::new("first-note").small().label(tr("Neue Notiz")).on_click(move |_, _, cx| {
                let id = new_id.clone();
                act(cx, |l| l.new_note = Some(NewNoteRequest { module_id: Some(id), folder: None }))
            }),
            t,
        )
    } else {
        inset_list(t).children(hover_rows).into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .gap(px(space::XL))
        .child(div().flex().flex_col().gap(px(space::S)).child(section(title, None)).child(list))
        .child(div().flex().flex_col().gap(px(space::S)).child(section(section_title(tr("Zuletzt geschrieben")), Some(all_files))).child(files))
}

fn empty_card(icon_name: &str, title: String, detail: String, actions: impl IntoElement, t: &Theme) -> AnyElement {
    grouped(t)
        .p(px(space::L))
        .flex_row()
        .items_start()
        .gap(px(space::M))
        .child(icon(icon_name, iconsize::M, t.secondary))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::S)).child(div().flex().flex_col().gap(px(space::XXS)).child(div().font_weight(FontWeight::MEDIUM).child(title)).child(caption(detail, t))).child(div().flex().child(actions)))
        .into_any_element()
}

/// The side column: only what this module has; what it lacks takes no room.
fn overview_rail(lib: &Library, m: &Module, t: &Theme) -> Option<Div> {
    let mut rail = div().flex().flex_col().gap(px(space::XL));
    let mut any = false;
    let cards = crate::core::cards::all_cards();
    let total_cards = cards.iter().filter(|c| c.in_deck(&m.folder)).count();
    if total_cards > 0 {
        any = true;
        let waiting = crate::core::cards::counts(&m.folder, crate::core::cards::now()).total();
        let deck = m.folder.clone();
        rail = rail.child(
            div().flex().flex_col().gap(px(space::S)).child(section_title(tr("Karten"))).child(
                grouped(t).p(px(space::M)).flex_row().items_center().gap(px(space::M)).child(div().flex_1().min_w_0().flex().flex_col().child(div().font_weight(FontWeight::MEDIUM).child(if waiting > 0 { trf("%@ warten", &[&waiting.to_string()]) } else { tr("Alles gelernt") })).child(caption(if total_cards == 1 { tr("1 Karte") } else { trf("%@ Karten", &[&total_cards.to_string()]) }, t))).child(Button::new("module-learn").small().when(waiting > 0, |b| b.primary()).label(if waiting > 0 { tr("Lernen") } else { tr("Ansehen") }).on_click(move |_, _, cx| {
                    let deck = deck.clone();
                    act(cx, |l| {
                        l.cards_request = Some(deck);
                        l.set_route(library::Route::Cards);
                    })
                })),
            ),
        );
    }
    if !m.sessions.is_empty() {
        any = true;
        use chrono::Datelike;
        let today = lib.today().weekday().number_from_monday();
        let mut sessions = m.sessions.clone();
        sessions.sort_by_key(|s| ((s.weekday + 7 - today) % 7, s.start.clone()));
        let rows = sessions.into_iter().map(|s| {
            let is_today = s.weekday == today;
            let day_name = day::weekday_names().get(s.weekday.saturating_sub(1) as usize).copied().unwrap_or_default();
            div()
                .flex()
                .items_center()
                .gap(px(space::M))
                .px(px(space::M))
                .py(px(space::S))
                .child(div().w(px(28.)).flex_none().text_size(px(font::CALLOUT)).font_weight(FontWeight::SEMIBOLD).text_color(if is_today { t.accent_text } else { t.secondary }).child(day_name.chars().take(2).collect::<String>()))
                .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().child(if s.title.is_empty() { tr("Termin") } else { s.title.clone() })).child(caption(if s.location.is_empty() { format!("{}–{}", s.start, s.end) } else { format!("{}–{} · {}", s.start, s.end, s.location) }, t).truncate()))
        });
        rail = rail.child(div().flex().flex_col().gap(px(space::S)).child(section_title(tr("Termine"))).child(grouped(t).children(rows)));
    }
    let system = lib.config.as_ref().map(|c| c.grading);
    if system.is_none_or(|s| s.uses_grades()) && (!m.components.is_empty() || m.grade.is_some() || m.target_grade.is_some()) {
        any = true;
        let number = |g: f64| super::fields::format_number(g, 2);
        let mut rows: Vec<AnyElement> = Vec::new();
        let weight_total: f64 = m.components.iter().map(|c| c.weight).sum();
        for c in &m.components {
            let share = if weight_total > 0. { c.weight / weight_total * 100. } else { 0. };
            rows.push(div().flex().items_center().gap(px(space::S)).px(px(space::M)).py(px(space::S)).child(div().flex_1().min_w_0().truncate().child(if c.name.is_empty() { tr("Teilnote") } else { c.name.clone() })).child(caption(format!("{share:.0} %"), t).flex_none()).child(div().w(px(36.)).flex_none().text_right().font_features(tabular()).font_weight(FontWeight::MEDIUM).when(c.grade.is_none(), |d| d.text_color(t.tertiary)).child(c.grade.map_or("–".to_string(), number))).into_any_element());
        }
        let summary = match (m.effective_grade(), m.target_grade) {
            (Some(g), Some(goal)) => Some(trf("Aktuell %@, Ziel %@", &[&number(g), &number(goal)])),
            (Some(g), None) => Some(trf("Aktuell %@", &[&number(g)])),
            (None, Some(goal)) => Some(trf("Ziel %@", &[&number(goal)])),
            (None, None) => None,
        };
        if let Some(line) = summary {
            rows.push(div().px(px(space::M)).py(px(space::S)).border_t_1().border_color(t.separator).font_weight(FontWeight::MEDIUM).text_size(px(font::CALLOUT)).child(line).into_any_element());
        }
        rail = rail.child(div().flex().flex_col().gap(px(space::S)).child(section(section_title(tr("Note")), None)).child(grouped(t).children(rows)));
    }
    let links: Vec<_> = m.links.iter().filter(|k| k.url().is_some()).cloned().collect();
    if !links.is_empty() {
        any = true;
        let hover = t.ink.opacity(0.05);
        let rows = links.into_iter().enumerate().map(|(i, link)| {
            let url = link.url().map(|u| u.to_string()).unwrap_or_default();
            div()
                .id(("module-link", i))
                .flex()
                .items_center()
                .gap(px(space::S))
                .px(px(space::M))
                .h(px(super::theme::control::M))
                .cursor_pointer()
                .key_nav()
                .soft(hover, 0.)
                .tooltip(tip(link.address.clone()))
                .child(icon("external-link", iconsize::S, t.secondary))
                .child(div().flex_1().min_w_0().truncate().child(if link.title.is_empty() { link.address.clone() } else { link.title.clone() }))
                .on_click(move |_, _, _| {
                    let _ = opener::open(&url);
                })
        });
        rail = rail.child(div().flex().flex_col().gap(px(space::S)).child(section_title(tr("Links"))).child(grouped(t).children(rows)));
    }
    any.then_some(rail)
}

/// The overview: two columns while they fit, one after the other when not.
pub fn overview(lib: &Library, m: &Module, width: f32, st: &ModuleState, view: Entity<Pages>, t: &Theme) -> Div {
    let main = overview_main(lib, m, st, view, t);
    let rail = overview_rail(lib, m, t);
    let wide = width >= 900.;
    let edit_id = m.id.clone();
    let hint = rail.is_none().then(|| {
        div().id("module-add-details").cursor_pointer().key_nav().text_size(px(font::CALLOUT)).text_color(t.secondary).hover({ let c = t.primary; move |s| s.text_color(c) }).child(tr("Termine, Links und Noten in den Moduleinstellungen ergänzen")).on_click(move |_, _, cx| super::settings::open_module(&edit_id, cx))
    });
    let body = if wide {
        div().flex().items_start().gap(px(space::XXL)).child(div().flex_1().min_w_0().child(main)).children(rail.map(|r| div().w(px(300.)).flex_none().child(r)))
    } else {
        div().flex().flex_col().gap(px(space::XL)).child(main).children(rail)
    };
    div().w_full().max_w(px(1120.)).mx_auto().px(px(space::XL)).py(px(space::L)).flex().flex_col().gap(px(space::XL)).child(body).children(hint)
}
