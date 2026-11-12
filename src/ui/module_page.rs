use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

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
use super::theme::{Theme, control, font, hover, radius, space, iconsize};
use super::widgets::*;
use crate::core::day;
use crate::core::filing::FileRole;
use crate::core::i18n::{tr, trf};
use crate::core::library::{self, Library, NewNoteRequest};
use crate::core::model::{Assignment, Kind, Module};

/// How many later or done deadlines show before "Alle zeigen".
const FOLDED: usize = 5;
/// How long a deadline just done keeps folding out of the attention list.
pub const LEAVE: Duration = Duration::from_millis(300);
/// Strength of the light on the done list when a deadline just went there.
const DONE_TINT: f32 = 0.1;
/// From this page width the side column stays beside the deadlines, also while the inspector is open.
const TWO_COLUMNS: f32 = 740.;
/// From this page width the side column gets its full width.
const WIDE_RAIL: f32 = 1000.;
/// Side column width from `WIDE_RAIL` on.
const RAIL: f32 = 300.;
/// Side column width between `TWO_COLUMNS` and `WIDE_RAIL`.
const NARROW_RAIL: f32 = 248.;

/// View state of a module page.
#[derive(Default)]
pub struct ModuleState {
    pub show_past: bool,
    pub show_later: bool,
    pub all_past: bool,
    pub all_later: bool,
    pub expanded_files: std::collections::HashSet<String>,
    /// Takes the keyboard while the page is not searching its files.
    pub focus: Option<FocusHandle>,
    pub scroll: ScrollHandle,
    /// The module and the deadlines that needed attention when last looked at, to notice one being done.
    pub attention: Option<(String, Vec<String>)>,
    /// Deadlines just done, folding out of the attention list.
    pub leaving: Vec<(String, Instant)>,
    /// When a deadline was last done here, to light up the list it went to.
    pub done_at: Option<Instant>,
    /// The selected row and its place, so the cursor stays put when the row leaves the list.
    pub cursor: Option<(String, usize)>,
    /// Set by keyboard moves: the selected row scrolls into view when it is painted next.
    pub reveal: Rc<Cell<bool>>,
}

/// A module's deadlines as the page groups them.
struct Buckets<'a> {
    attention: Vec<&'a Assignment>,
    weeks: BTreeMap<NaiveDate, Vec<&'a Assignment>>,
    later: Vec<&'a Assignment>,
    past: Vec<&'a Assignment>,
}

fn buckets<'a>(lib: &'a Library, m: &Module) -> Buckets<'a> {
    let monday = day::monday(lib.today());
    let horizon = monday + Days::new(14);
    let mut b = Buckets { attention: Vec::new(), weeks: BTreeMap::new(), later: Vec::new(), past: Vec::new() };
    for a in lib.assignments.iter().filter(|a| a.module_id == m.id) {
        let d = day::local_date(a.due);
        if lib.needs_action(a) {
            b.attention.push(a);
        } else if d >= horizon {
            b.later.push(a);
        } else if d >= monday {
            b.weeks.entry(day::monday(d)).or_default().push(a);
        } else {
            b.past.push(a);
        }
    }
    b.attention.sort_by(|x, y| y.due.cmp(&x.due));
    b.past.sort_by(|x, y| y.due.cmp(&x.due));
    b
}

fn shown(all: bool, items: &[&Assignment]) -> usize {
    if all { items.len() } else { items.len().min(FOLDED) }
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
        .soft(t.ink.opacity(hover::ROW), radius::INNER)
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
    let (fill, over) = (t.ink.opacity(0.05), t.ink.opacity(hover::CONTROL));
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
                .min_h(px(control::S))
                .px(px(space::S))
                .py(px(space::XS))
                .rounded(px(radius::CONTROL))
                .bg(fill)
                .soft(over, radius::CONTROL)
                .cursor_pointer().key_nav()
                .text_size(px(font::CAPTION))
                .tip(if brief { trf("Aufgabenstellung: %@", &[&name]) } else { name.clone() })
                .on_click(move |_, _, cx| super::preview::show(&preview, cx))
                .when(brief, |d| d.child(icon("clipboard-list", iconsize::XS, t.accent)).child(div().font_weight(FontWeight::MEDIUM).child(tr("Aufgabe"))))
                .when(!brief, |d| d.child(file_icon(path, iconsize::S)).child(div().truncate().child(crate::core::text::middle_ellipsis(&name, 26))))
                .context_menu(move |menu, _, cx| {
                    let (p1, p2, p3, id) = (menu_path.clone(), menu_path.clone(), menu_path.clone(), id.clone());
                    let rename = super::convention_rename::menu_item(&menu_path, cx);
                    let role = if brief { FileRole::Work } else { FileRole::Brief };
                    menu.item(PopupMenuItem::new(tr("Öffnen")).on_click(move |_, _, cx| act(cx, |l| l.open_file(&p1))))
                        .item(PopupMenuItem::new(super::platform::reveal_label()).on_click(move |_, _, _| library::reveal(&p2)))
                        .when_some(rename, |m, item| m.item(item))
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
                .min_h(px(control::S))
                .flex()
                .items_center()
                .px(px(space::S))
                .py(px(space::XS))
                .rounded(px(radius::CONTROL))
                .bg(fill)
                .soft(over, radius::CONTROL)
                .cursor_pointer().key_nav()
                .text_size(px(font::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .child(if expanded { tr("Weniger") } else { format!("+{more}") })
                .tip(if expanded { tr("Weniger zeigen") } else { trf("Alle %lld Dateien zeigen", &[&all.len()]) })
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
    let watch = lib.is_selected(&a.id).then(|| keep_visible(st.scroll.clone(), st.reveal.clone()));
    motion::flip(format!("entry-{}", a.id), div().relative().flex().flex_col().gap(px(space::XS)).py(px(space::XXS)).child(assignment_row(lib, a, false, true, false, t)).children(file_pills(lib, a, st, view, t)).children(watch))
}

/// Scrolls the page so the element it covers is in view, once after a keyboard move asked for it.
fn keep_visible(scroll: ScrollHandle, pending: Rc<Cell<bool>>) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            if !pending.replace(false) {
                return;
            }
            let view = scroll.bounds();
            let margin = px(space::XL);
            let mut offset = scroll.offset();
            if bounds.top() < view.top() + margin {
                offset.y += view.top() + margin - bounds.top();
            } else if bounds.bottom() > view.bottom() - margin {
                offset.y -= bounds.bottom() - (view.bottom() - margin);
            } else {
                return;
            }
            offset.y = offset.y.clamp(-scroll.max_offset().y, px(0.));
            scroll.set_offset(offset);
            window.refresh();
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

fn show_all(id: &'static str, total: usize, all: bool, view: Entity<Pages>, toggle: fn(&mut ModuleState), t: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(px(control::S))
        .px(px(space::S))
        .ml(px(space::XXL - space::S))
        .rounded(px(radius::INNER))
        .cursor_pointer()
        .key_nav()
        .soft(t.ink.opacity(hover::CONTROL), radius::INNER)
        .text_size(px(font::CALLOUT))
        .text_color(t.accent_text)
        .child(if all { tr("Weniger zeigen") } else { trf("Alle %lld zeigen", &[&total]) })
        .on_click(move |_, _, cx| view.update(cx, |p, cx| {
            toggle(&mut p.module);
            cx.notify();
        }))
}

/// Keeps the cursor and notices deadlines being done; call it whenever the library changed while the page shows.
/// Returns the row to select when the selected one just left the list.
pub fn follow(lib: &Library, m: &Module, st: &mut ModuleState, now: Instant, reduce_motion: bool) -> Option<String> {
    let ids: Vec<String> = buckets(lib, m).attention.iter().map(|a| a.id.clone()).collect();
    let mut done: Vec<String> = Vec::new();
    if st.attention.as_ref().is_some_and(|(module, _)| *module != m.id) {
        st.cursor = None;
        st.leaving.clear();
        st.done_at = None;
    }
    if let Some((module, before)) = &st.attention
        && *module == m.id
    {
        done = before.iter().filter(|id| !ids.contains(id) && lib.assignment(id).is_some_and(|a| lib.is_done(a))).cloned().collect();
    }
    if !done.is_empty() {
        st.done_at = Some(now);
        if !reduce_motion {
            st.leaving.extend(done.iter().map(|id| (id.clone(), now)));
        }
    }
    st.leaving.retain(|(_, at)| now.duration_since(*at) < LEAVE);
    st.attention = Some((m.id.clone(), ids));
    let rows = deadline_order(lib, m, st);
    let selected = lib.selected().filter(|a| a.module_id == m.id)?.id.clone();
    let at = rows.iter().position(|r| *r == selected);
    let left = done.contains(&selected);
    match (at, &st.cursor) {
        (Some(i), _) if !left => {
            st.cursor = Some((selected, i));
            None
        }
        (_, Some((id, i))) if left && *id == selected && !rows.is_empty() => {
            let last = rows.len() - 1;
            let i = (*i).min(last);
            let i = if rows[i] != selected { i } else if i < last { i + 1 } else { i.saturating_sub(1) };
            st.cursor = Some((rows[i].clone(), i));
            st.reveal.set(true);
            Some(rows[i].clone())
        }
        _ => None,
    }
}

/// The deadlines in the order the list shows them, for walking through by keyboard.
pub fn deadline_order(lib: &Library, m: &Module, st: &ModuleState) -> Vec<String> {
    let b = buckets(lib, m);
    let mut out: Vec<String> = b.attention.iter().map(|a| a.id.clone()).collect();
    out.extend(b.weeks.into_values().flatten().map(|a| a.id.clone()));
    if st.show_later {
        out.extend(b.later.iter().take(shown(st.all_later, &b.later)).map(|a| a.id.clone()));
    }
    if st.show_past {
        out.extend(b.past.iter().take(shown(st.all_past, &b.past)).map(|a| a.id.clone()));
    }
    out
}

/// Deadlines with their files: what needs attention, the next two weeks by lecture week, later and done.
pub fn deadlines(lib: &Library, m: &Module, st: &ModuleState, view: Entity<Pages>, t: &Theme) -> Div {
    let b = buckets(lib, m);
    let now = Instant::now();
    let visible = deadline_order(lib, m, st);
    let mut attention: Vec<(&Assignment, bool)> = b.attention.iter().map(|a| (*a, false)).collect();
    for (id, at) in &st.leaving {
        if now.duration_since(*at) < LEAVE
            && !visible.contains(id)
            && let Some(a) = lib.assignment(id).filter(|a| a.module_id == m.id && lib.is_done(a))
        {
            attention.push((a, true));
        }
    }
    attention.sort_by(|x, y| y.0.due.cmp(&x.0.due));
    let weeks = lib.weeks();
    let mut col = div().flex().flex_col().px(px(space::L)).py(px(space::S));
    if attention.is_empty() && b.weeks.is_empty() {
        col = col.child(div().flex().items_center().gap(px(space::S)).py(px(space::S)).text_color(t.secondary).child(icon("circle-check", iconsize::S, t.green)).child(tr("Nichts offen in den nächsten zwei Wochen")));
    }
    if !attention.is_empty() {
        col = col.child(div().flex().items_center().gap(px(space::S)).mt(px(space::S)).py(px(space::XS)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(tr("Noch offen")));
        for (a, leaving) in attention {
            col = col.child(div().mx(px(-space::S)).child(motion::reveal(SharedString::from(format!("attention-{}", a.id)), !leaving, div().w_full().relative().px(px(space::S)).child(entry(lib, a, st, view.clone(), t)).when(leaving, |d| d.child(div().absolute().inset_0().occlude())))));
        }
    }
    for (week, entries) in b.weeks {
        let lecture = weeks.iter().find(|w| w.monday == week).map(|w| w.index);
        let header = div().flex().items_center().gap(px(space::S)).mt(px(space::S)).py(px(space::XS)).child(week_title(week, lecture, lib.today(), t));
        col = col.child(motion::flip(format!("week-{week}"), header));
        for a in entries {
            col = col.child(entry(lib, a, st, view.clone(), t));
        }
    }
    if !b.later.is_empty() {
        let mut rows = div().w_full().px(px(space::S)).flex().flex_col().children(b.later.iter().take(shown(st.all_later, &b.later)).map(|a| entry(lib, a, st, view.clone(), t)));
        if b.later.len() > FOLDED {
            rows = rows.child(show_all("later-all", b.later.len(), st.all_later, view.clone(), |s| s.all_later = !s.all_later, t));
        }
        col = col.child(motion::flip("later", disclosure("later", trf("Später (%lld)", &[&b.later.len()]), st.show_later, view.clone(), |s| s.show_later = !s.show_later, t)));
        col = col.child(div().mx(px(-space::S)).child(motion::reveal("later-rows", st.show_later, rows)));
    }
    if !b.past.is_empty() {
        let lit = st.done_at.is_some_and(|at| now.duration_since(at) < motion::FLASH);
        let mut rows = div().w_full().px(px(space::S)).flex().flex_col().children(b.past.iter().take(shown(st.all_past, &b.past)).map(|a| entry(lib, a, st, view.clone(), t)));
        if b.past.len() > FOLDED {
            rows = rows.child(show_all("past-all", b.past.len(), st.all_past, view.clone(), |s| s.all_past = !s.all_past, t));
        }
        col = col.child(motion::flip("past", motion::glow("past-glow", lit, t.green.opacity(DONE_TINT), disclosure("past", trf("Erledigt (%lld)", &[&b.past.len()]), st.show_past, view.clone(), |s| s.show_past = !s.show_past, t))));
        col = col.child(div().mx(px(-space::S)).child(motion::reveal("past-rows", st.show_past, rows)));
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
            menu.item(PopupMenuItem::new(format!("{}{}", tr("Neuer Eintrag…"), crate::core::shortcuts::ShortcutAction::NewEntry.hint())).icon(menu_icon("calendar-plus")).on_click(|_, _, cx| act(cx, |l| {
                l.quick_add_kind = Some(Kind::Submission);
                l.show_quick_add = true;
            })))
                .item(PopupMenuItem::new(tr("Modulordner öffnen")).icon(menu_icon("folder")).on_click(move |_, _, _| library::open_folder(&reveal)))
                .separator()
                .item(PopupMenuItem::new(tr("Modul bearbeiten")).icon(menu_icon("settings")).on_click(move |_, _, cx| super::settings::open_module(&edit_id, cx)))
        }));
    div()
        .flex_none()
        .flex()
        .flex_col()
        .border_b_1()
        .border_color(t.separator)
        .child(
            div()
                .w_full()
                .max_w(px(control::CONTENT_MAX))
                .mx_auto()
                .px(px(space::XL))
                .py(px(space::L))
                .flex()
                .items_center()
                .gap(px(space::L))
                .child(div().size(px(48.)).flex_none().rounded(px(radius::CARD)).bg(color).flex().items_center().justify_center().text_size(px(font::TITLE3)).font_weight(FontWeight::BOLD).text_color(white()).child(code))
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
    div().id(id).flex_none().flex().items_center().h(px(control::S)).px(px(space::S)).mr(px(-space::S)).cursor_pointer().key_nav().soft(t.ink.opacity(hover::CONTROL), radius::INNER).text_size(px(font::CALLOUT)).text_color(t.accent_text).child(label)
}

fn section(title: impl IntoElement, link: Option<Stateful<Div>>) -> Div {
    div().flex().items_center().justify_between().gap(px(space::M)).child(title).children(link)
}

/// The main column: every deadline with what belongs to it, and what was worked on last.
fn overview_main(lib: &Library, m: &Module, st: &ModuleState, view: Entity<Pages>, t: &Theme) -> Div {
    let all = lib.assignments.iter().filter(|a| a.module_id == m.id).count();
    let waiting = lib.action_items().iter().filter(|a| a.module_id == m.id).count();
    let edit_id = m.id.clone();
    let add_submission = |cx: &mut App| act(cx, |l| {
        l.quick_add_kind = Some(Kind::Submission);
        l.show_quick_add = true;
    });
    let list: AnyElement = if all == 0 {
        empty_card(
            "calendar-plus",
            tr("Noch keine Abgaben"),
            tr("Lege eine einzelne Abgabe an, oder eine Regel, die sich jede Vorlesungswoche wiederholt."),
            div()
                .flex()
                .gap(px(space::S))
                .child(Button::new("first-entry").small().label(tr("Neuer Eintrag…")).on_click(move |_, _, cx| add_submission(cx)))
                .child(Button::new("first-rule").small().ghost().label(tr("Regel anlegen")).on_click(move |_, _, cx| super::settings::open_module(&edit_id, cx))),
            t,
        )
    } else {
        grouped(t).child(deadlines(lib, m, st, view, t)).into_any_element()
    };
    let title = div().flex().items_center().gap(px(space::S)).child(section_title(tr("Abgaben"))).when(waiting > 0, |d| {
        d.child(div().id("module-attention").flex_none().min_w(px(iconsize::M)).px(px(space::XS)).rounded_full().bg(t.badge_fill(t.gray)).flex().items_center().justify_center().text_size(px(font::CAPTION2)).font_weight(FontWeight::BOLD).font_features(tabular()).text_color(white()).child(waiting.to_string()).tip(tr("Noch offen")))
    });
    let add = (all > 0).then(|| more_link("add-entry", tr("Neuer Eintrag…"), t).tip(format!("{}{}", tr("Abgabe, Aufgabe oder Prüfung eintragen"), crate::core::shortcuts::ShortcutAction::NewEntry.hint())).on_click(move |_, _, cx| add_submission(cx)));
    let folder = lib.root.as_ref().map(|r| r.join(&m.folder)).unwrap_or_default();
    let recent = super::today::recent_notes_in(Some(&folder), 6);
    let count = recent.len();
    let now = chrono::Local::now().timestamp() as f64;
    let hover_rows = recent.into_iter().enumerate().map(|(i, (path, at))| {
        let (open, menu) = (path.clone(), path.clone());
        inset_file_row(("module-recent", i), &path, path.file_stem().unwrap_or_default().to_string_lossy().to_string(), t).child(caption(super::notes_page::changed_ago(now - at), t).flex_none()).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open))).context_menu(move |menu_box, window, cx| {
            let m = file_menu(menu_box, menu.clone()).when_some(super::convention_rename::menu_item(&menu, cx), |m, item| m.item(item));
            super::ai::file_submenu(m, &menu, window, cx)
        })
    });
    let other = super::today::recent_files_in(&folder, 6);
    let other_rows = other.into_iter().enumerate().map(|(i, (path, at))| {
        let (open, menu) = (path.clone(), path.clone());
        inset_file_row(("module-file-row", i), &path, path.file_name().unwrap_or_default().to_string_lossy().to_string(), t).child(caption(super::notes_page::changed_ago(now - at), t).flex_none()).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open))).context_menu(move |menu_box, window, cx| {
            let m = file_menu(menu_box, menu.clone());
            super::ai::file_submenu(m, &menu, window, cx)
        })
    });
    let other_rows: Vec<_> = other_rows.collect();
    let new_id = m.id.clone();
    let module_folder = m.folder.clone();
    let all_files = more_link("to-files", tr("Alle Dateien"), t).tip(tr("Im Dateibaum zeigen")).on_click(move |_, _, cx| {
        let folder = module_folder.clone();
        crate::core::prefs::update(|p| p.notes_explorer_hidden = false);
        act(cx, |l| {
            l.reveal_folder = Some(folder);
            l.show_notes();
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
        .child(div().flex().flex_col().gap(px(space::S)).child(section(title, add)).child(list))
        .child(div().flex().flex_col().gap(px(space::S)).child(section(section_title(tr("Zuletzt geschrieben")), Some(all_files))).child(files))
        .when(!other_rows.is_empty(), |d| d.child(div().flex().flex_col().gap(px(space::S)).child(section(section_title(tr("Dateien")), None)).child(inset_list(t).children(other_rows))))
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
                .soft_row(t.ink.opacity(hover::ROW))
                .tip(link.address.clone())
                .child(icon("external-link", iconsize::S, t.secondary))
                .child(div().flex_1().min_w_0().truncate().child(if link.title.is_empty() { link.address.clone() } else { link.title.clone() }))
                .on_click(move |_, _, _| {
                    crate::core::library::open_path(&url);
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
    let wide = width >= TWO_COLUMNS;
    let edit_id = m.id.clone();
    let hint = rail.is_none().then(|| {
        div().id("module-add-details").min_h(px(control::S)).flex().items_center().px(px(space::S)).mx(px(-space::S)).rounded(px(radius::INNER)).cursor_pointer().key_nav().soft(t.ink.opacity(hover::CONTROL), radius::INNER).text_size(px(font::CALLOUT)).text_color(t.secondary).child(tr("Termine, Links und Noten in den Moduleinstellungen ergänzen")).on_click(move |_, _, cx| super::settings::open_module(&edit_id, cx))
    });
    let body = if wide {
        let (rail_width, gap) = if width >= WIDE_RAIL { (RAIL, space::XXL) } else { (NARROW_RAIL, space::XL) };
        div().flex().items_start().gap(px(gap)).child(div().flex_1().min_w_0().child(main)).children(rail.map(|r| div().w(px(rail_width)).flex_none().child(r)))
    } else {
        div().flex().flex_col().gap(px(space::XL)).child(main).children(rail)
    };
    div().w_full().max_w(px(control::CONTENT_MAX)).mx_auto().px(px(space::XL)).py(px(space::L)).flex().flex_col().gap(px(space::XL)).child(body).children(hint)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use chrono::Days;

    use super::{FOLDED, ModuleState, buckets, deadline_order, follow};
    use crate::core::library::Library;

    fn module_lib(name: &str) -> Library {
        let today = chrono::Local::now().date_naive();
        let root = std::env::temp_dir().join(format!("schrumbi-module-page-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let (start, end) = ((today - Days::new(50)).format("%Y-%m-%d"), (today + Days::new(120)).format("%Y-%m-%d"));
        let rule = r#"{ "id": "r", "title": "Blatt", "short": "B", "weekday": 3, "time": "18:00", "weekOffset": 0, "interval": 1, "upload": true, "folder": "W{w}/B", "filePattern": "*" }"#;
        let config = format!(r#"{{ "version": 2, "name": "Test", "author": "Muster-M", "inbox": "_Inbox", "semesters": [{{ "id": "s", "name": "WS", "start": "{start}", "end": "{end}", "breaks": [], "holidays": [] }}], "modules": [{{ "id": "m0", "code": "M0", "name": "Modul", "semester": "s", "folder": "M0", "color": "blue", "naming": "{{code}}_{{yy}}_{{title}}_{{author}}_{{date}}", "rules": [{rule}] }}] }}"#);
        std::fs::write(root.join("schrumbi.json"), config).unwrap();
        let mut lib = Library::new();
        lib.open(&root);
        if let Some(job) = lib.scan_job() {
            lib.apply_scan(job.run());
        }
        lib
    }

    #[test]
    fn doing_the_selected_deadline_moves_the_cursor_to_the_next() {
        let mut lib = module_lib("cursor");
        let m = lib.module_by_id("m0").cloned().unwrap();
        let mut st = ModuleState::default();
        let now = Instant::now();
        assert_eq!(follow(&lib, &m, &mut st, now, false), None);
        let rows = deadline_order(&lib, &m, &st);
        assert!(rows.len() >= 3, "{rows:?}");
        lib.select(Some(rows[1].clone()));
        assert_eq!(follow(&lib, &m, &mut st, now, false), None);
        lib.toggle_done(&rows[1]);
        assert_eq!(follow(&lib, &m, &mut st, now, false), Some(rows[2].clone()));
        assert!(st.leaving.iter().any(|(id, _)| *id == rows[1]));
        assert!(st.done_at.is_some());
        assert!(!deadline_order(&lib, &m, &st).contains(&rows[1]));
    }

    #[test]
    fn reduced_motion_skips_the_fold_but_keeps_the_cursor() {
        let mut lib = module_lib("reduced");
        let m = lib.module_by_id("m0").cloned().unwrap();
        let mut st = ModuleState::default();
        let now = Instant::now();
        follow(&lib, &m, &mut st, now, true);
        let rows = deadline_order(&lib, &m, &st);
        lib.select(Some(rows[0].clone()));
        follow(&lib, &m, &mut st, now, true);
        lib.toggle_done(&rows[0]);
        assert_eq!(follow(&lib, &m, &mut st, now, true), Some(rows[1].clone()));
        assert!(st.leaving.is_empty());
    }

    #[test]
    fn a_done_row_out_of_view_keeps_its_selection() {
        let mut lib = module_lib("folded");
        let m = lib.module_by_id("m0").cloned().unwrap();
        let mut st = ModuleState { show_past: true, ..Default::default() };
        let now = Instant::now();
        let first = deadline_order(&lib, &m, &st)[0].clone();
        lib.toggle_done(&first);
        follow(&lib, &m, &mut st, now, true);
        assert!(deadline_order(&lib, &m, &st).contains(&first));
        lib.select(Some(first.clone()));
        follow(&lib, &m, &mut st, now, true);
        st.show_past = false;
        assert_eq!(follow(&lib, &m, &mut st, now, true), None);
        assert_eq!(lib.selected().map(|a| a.id.clone()), Some(first));
    }

    #[test]
    fn later_shows_five_until_all_are_asked_for() {
        let lib = module_lib("later");
        let m = lib.module_by_id("m0").cloned().unwrap();
        let mut st = ModuleState { show_later: true, ..Default::default() };
        let later = buckets(&lib, &m).later.len();
        assert!(later > FOLDED, "{later}");
        let folded = deadline_order(&lib, &m, &st).len();
        st.all_later = true;
        assert_eq!(deadline_order(&lib, &m, &st).len(), folded + later - FOLDED);
    }
}
