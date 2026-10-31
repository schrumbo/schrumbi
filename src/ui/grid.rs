use std::collections::HashSet;

use chrono::{Days, NaiveDate};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::act;
use super::motion::{self, Soft};
use super::theme::{Theme, font, radius, space, iconsize};
use super::widgets::*;
use crate::core::day;
use crate::core::i18n::{tr, trf};
use crate::core::library::{Library, Route};
use crate::core::model::{Assignment, LectureWeek, Status};
use crate::core::schedule;

/// View state of the semester page.
#[derive(Default)]
pub struct GridState {
    pub expanded: HashSet<NaiveDate>,
    pub show_past: bool,
    pub reveal_optional: HashSet<NaiveDate>,
    pub scroll: ScrollHandle,
    target: Option<(NaiveDate, ScrollAnchor)>,
}

impl GridState {
    /// Opens a week and scrolls it into view.
    fn jump_to(&mut self, monday: NaiveDate, current: NaiveDate, window: &mut Window, cx: &mut App) {
        self.expanded.insert(monday);
        self.show_past |= monday < current;
        let anchor = ScrollAnchor::for_handle(self.scroll.clone());
        self.target = Some((monday, anchor.clone()));
        window.on_next_frame(move |window, cx| anchor.scroll_to(window, cx));
    }
}

#[derive(Clone)]
struct Slot {
    monday: NaiveDate,
    lecture: Option<LectureWeek>,
    break_name: Option<String>,
}

fn slots(lib: &Library) -> Vec<Slot> {
    let Some(s) = lib.semester() else { return Vec::new() };
    let (Some(start), Some(end)) = (s.start_date(), s.end_date()) else { return Vec::new() };
    let lectures = schedule::lecture_weeks(s);
    let limit = lib.semester_assignments().iter().map(|a| day::local_date(a.due)).max().map_or(end, |m| m.max(end));
    let mut out = Vec::new();
    let mut monday = day::monday(start);
    while monday <= limit {
        let lecture = lectures.iter().find(|w| w.monday == monday).copied();
        let break_name = lecture.is_none().then(|| s.break_at(monday).map(|b| b.name.clone()).unwrap_or_else(|| tr("Vorlesungsfrei")));
        out.push(Slot { monday, lecture, break_name });
        monday = monday + Days::new(7);
    }
    out
}

fn items<'a>(lib: &'a Library, slot: &Slot) -> Vec<&'a Assignment> {
    let next = slot.monday + Days::new(7);
    lib.semester_assignments().into_iter().filter(|a| {
        let d = day::local_date(a.due);
        d >= slot.monday && d < next
    }).collect()
}

fn progress(lib: &Library, items: &[&Assignment]) -> Option<f32> {
    let relevant: Vec<_> = items.iter().filter(|a| lib.status(a) != Status::Skipped && !a.optional).collect();
    (!relevant.is_empty()).then(|| relevant.iter().filter(|a| lib.is_done(a)).count() as f32 / relevant.len() as f32)
}

pub fn render(lib: &Library, st: &GridState, view: Entity<super::pages::Pages>, t: &Theme) -> Div {
    let slots = slots(lib);
    let current = day::monday(lib.today());
    let past: Vec<&Slot> = slots.iter().filter(|s| s.monday < current).collect();
    let this_week = slots.iter().find(|s| s.monday == current);
    let future: Vec<&Slot> = slots.iter().filter(|s| s.monday > current).collect();

    let strip_view = view.clone();
    let mut weeks = grouped(t);
    if !past.is_empty() {
        let v = view.clone();
        weeks = weeks.child(
            row_button("past-weeks", t)
                .flex()
                .items_center()
                .gap(px(space::S))
                .px(px(space::L))
                .py(px(space::M))
                .child(div().w(px(16.)).flex().justify_center().child(chevron(st.show_past, t)))
                .child(div().font_weight(FontWeight::MEDIUM).child(tr("Vergangene Wochen")))
                .child(div().text_color(t.secondary).child(past_summary(lib, &past)))
                .on_click(move |_, _, cx| v.update(cx, |p, cx| {
                    p.grid.show_past = !p.grid.show_past;
                    cx.notify();
                })),
        );
        if st.show_past {
            for s in &past {
                weeks = weeks.child(divider(t)).child(week_row(lib, st, s, current, false, view.clone(), t));
            }
        }
        weeks = weeks.child(divider(t));
    }
    if let Some(s) = this_week {
        weeks = weeks.child(week_row(lib, st, s, current, true, view.clone(), t));
    }
    for (i, s) in future.iter().enumerate() {
        if i > 0 || this_week.is_some() {
            weeks = weeks.child(divider(t));
        }
        weeks = weeks.child(week_row(lib, st, s, current, false, view.clone(), t));
    }

    let view_for_strip = strip_view;
    let strip: Vec<(Slot, Option<f32>)> = slots.iter().map(|s| (s.clone(), progress(lib, &items(lib, s)))).collect();
    div().w_full().flex().justify_center().child(
        div()
            .w_full()
            .max_w(px(980.))
            .px(px(space::XXL))
            .py(px(space::XL))
            .flex()
            .flex_col()
            .gap(px(space::XXL))
            .child(progress_card(lib, &strip, current, view_for_strip, t))
            .child(div().flex().flex_col().gap(px(space::M)).child(section_title(tr("Module"))).child(module_list(lib, t)))
            .child(div().flex().flex_col().gap(px(space::M)).child(section_title(tr("Wochen"))).child(weeks)),
    )
}

fn chevron(open: bool, t: &Theme) -> motion::Chevron {
    motion::chevron("chevron", open, t.secondary)
}

fn past_summary(lib: &Library, past: &[&Slot]) -> String {
    let all: Vec<&Assignment> = past.iter().flat_map(|s| items(lib, s)).filter(|a| !a.optional && lib.status(a) != Status::Skipped).collect();
    let open = all.iter().filter(|a| !lib.is_done(a)).count();
    let weeks = past.iter().filter(|s| s.lecture.is_some()).count();
    if open == 0 { trf("%lld Wochen · alles erledigt", &[&weeks]) } else { trf("%lld Wochen · %lld offen", &[&weeks, &open]) }
}

fn progress_card(lib: &Library, strip: &[(Slot, Option<f32>)], current: NaiveDate, view: Entity<super::pages::Pages>, t: &Theme) -> Div {
    let (done, _, total) = lib.progress();
    let weeks = strip.iter().filter(|s| s.0.lecture.is_some()).count();
    let summary = match lib.current_week() {
        Some(w) => trf("Vorlesungswoche %lld von %lld · %lld von %lld Pflichtabgaben erledigt", &[&w.index, &weeks, &done, &total]),
        None => trf("%lld von %lld Pflichtabgaben erledigt", &[&done, &total]),
    };
    let end = lib.semester().and_then(|s| s.end_date());
    let first = strip.first().map(|s| day::short_date(s.0.monday)).unwrap_or_default();
    let last = strip.last().map(|s| day::short_date(s.0.monday)).unwrap_or_default();
    grouped(t)
        .p(px(space::XL))
        .gap(px(space::L))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::XL))
                .child(progress_ring(if total > 0 { done as f32 / total as f32 } else { 0. }, 56., t))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(space::XS))
                        .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(lib.semester().map_or_else(|| tr("Semester"), |s| s.name.clone())))
                        .child(div().text_color(t.secondary).child(summary)),
                )
                .when_some(end, |d, end| {
                    d.child(div().flex().flex_col().items_end().gap(px(space::XXS)).child(caption(tr("Vorlesungsende"), t)).child(div().font_weight(FontWeight::BOLD).child(day::short_date(end))))
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(space::S))
                .child(div().flex().items_end().gap(px(space::XS)).h(px(14.)).children(strip.iter().map(|(s, p)| {
                    let is_current = s.monday == current;
                    let is_past = s.monday < current;
                    let color = if s.lecture.is_none() {
                        t.ink.opacity(0.05)
                    } else if is_current {
                        t.accent
                    } else if let (true, Some(p)) = (is_past, p) {
                        if *p >= 1. { t.green.opacity(0.7) } else { t.red.opacity(0.7) }
                    } else {
                        t.ink.opacity(0.12)
                    };
                    let label = s.lecture.map(|w| w.label()).or(s.break_name.clone()).unwrap_or_default();
                    let help = format!("{label} · {}{}", day::short_date(s.monday), p.map(|p| format!(" · {} %", (p * 100.).round())).unwrap_or_default());
                    let (monday, view) = (s.monday, view.clone());
                    div()
                        .id(SharedString::from(format!("strip-{}", s.monday)))
                        .flex_1()
                        .h(px(if is_current { 14. } else { 8. }))
                        .rounded(px(radius::SMALL))
                        .bg(color)
                        .tooltip(tip(help))
                        .when(s.lecture.is_some(), |d| {
                            d.cursor_pointer().on_click(move |_, window, cx| view.update(cx, |p, cx| {
                                p.grid.jump_to(monday, current, window, cx);
                                cx.notify();
                            }))
                        })
                })))
                .child(
                    div()
                        .flex()
                        .text_size(px(font::CAPTION2))
                        .text_color(t.tertiary)
                        .child(first)
                        .child(div().flex_1().text_center().child(tr("Grün: Woche erledigt · Rot: offen geblieben · Blau: jetzt · Grau: Pause oder kommend")))
                        .child(last),
                ),
        )
}

fn module_list(lib: &Library, t: &Theme) -> Div {
    let mut list = grouped(t);
    for (i, m) in lib.semester_modules().into_iter().enumerate() {
        if i > 0 {
            list = list.child(divider(t));
        }
        let (done, total) = lib.module_progress(&m.id);
        let overdue = lib.action_items().iter().filter(|a| a.module_id == m.id).count();
        let next = lib.upcoming().into_iter().find(|a| a.module_id == m.id);
        let id = m.id.clone();
        let color = t.palette(&m.color);
        list = list.child(
            row_button(SharedString::from(format!("module-row-{}", m.id)), t)
                .flex()
                .items_center()
                .gap(px(space::M))
                .px(px(space::L))
                .py(px(space::M))
                .on_click(move |_, _, cx| act(cx, |l| l.set_route(Route::Module(id.clone()))))
                .child(div().size(px(10.)).rounded_full().bg(color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(space::XXS))
                        .child(div().font_weight(FontWeight::MEDIUM).child(m.display_name()))
                        .child(caption(next.map_or_else(|| tr("Nichts offen"), |a| trf("Nächste: %@, %@", &[&a.title, &day::due(a.due, lib.now)])), t).truncate()),
                )
                .when(overdue > 0, |d| d.child(div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).text_color(t.red).child(icon("circle-alert", iconsize::XS, t.red)).child(trf("%lld offen", &[&overdue]))))
                .when(total > 0, |d| {
                    d.child(div().w(px(120.)).child(progress_bar(done as f32 / total as f32, color, t))).child(div().font_features(tabular()).w(px(44.)).text_right().text_size(px(font::CALLOUT)).text_color(t.secondary).child(format!("{done}/{total}")))
                })
                .child(icon("chevron-right", iconsize::XS, t.tertiary)),
        );
    }
    list
}

fn week_row(lib: &Library, st: &GridState, slot: &Slot, current: NaiveDate, force_open: bool, view: Entity<super::pages::Pages>, t: &Theme) -> AnyElement {
    let entries = items(lib, slot);
    if let (Some(name), true) = (&slot.break_name, entries.is_empty()) {
        return div()
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::L))
            .py(px(space::S))
            .child(div().w(px(16.)).child(icon("sun", iconsize::S, t.secondary)))
            .child(div().text_color(t.secondary).child(name.clone()))
            .child(div().text_size(px(font::CAPTION)).text_color(t.tertiary).child(trf("ab %@", &[&day::short_date(slot.monday)])))
            .into_any_element();
    }
    let is_current = slot.monday == current;
    let open = force_open || st.expanded.contains(&slot.monday);
    let mandatory: Vec<_> = entries.iter().filter(|a| !a.optional && lib.status(a) != Status::Skipped).collect();
    let done = mandatory.iter().filter(|a| lib.is_done(a)).count();
    let attention = entries.iter().filter(|a| lib.needs_action(a)).count();
    let reveal = st.reveal_optional.contains(&slot.monday);
    let hidden = entries.iter().filter(|a| a.optional && !lib.is_done(a)).count();
    let shown: Vec<&&Assignment> = entries.iter().filter(|a| reveal || !a.optional || lib.is_done(a)).collect();
    let monday = slot.monday;
    let (v1, v2) = (view.clone(), view);

    let header = div()
        .id(SharedString::from(format!("week-{}", slot.monday)))
        .flex()
        .items_center()
        .gap(px(space::S))
        .px(px(space::L))
        .py(px(space::M))
        .when(!force_open, |d| {
            let hover = t.ink.opacity(0.06);
            d.cursor_pointer().key_nav().soft(hover, 0.).on_click(move |_, _, cx| v1.update(cx, |p, cx| {
                if !p.grid.expanded.remove(&monday) {
                    p.grid.expanded.insert(monday);
                }
                cx.notify();
            }))
        })
        .child(div().w(px(16.)).flex().justify_center().child(if force_open { div().size(px(8.)).rounded_full().bg(t.accent).into_any_element() } else { chevron(open, t).into_any_element() }))
        .child(div().w(px(36.)).font_weight(FontWeight::SEMIBOLD).child(slot.lecture.map_or("–".into(), |w| w.label())))
        .child(div().text_color(t.secondary).child(format!("{} – {}", day::short_date(slot.monday), day::short_date(slot.monday + Days::new(6)))))
        .when(is_current, |d| d.child(div().text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).text_color(t.accent).child(tr("Diese Woche"))))
        .child(div().flex_1())
        .when(!open, |d| {
            d.child(div().flex().gap(px(space::XS)).children(entries.iter().filter(|a| !a.optional).take(6).map(|a| {
                div().size(px(space::S)).rounded_full().bg(if lib.is_done(a) { t.ink.opacity(0.2) } else { t.palette(lib.module(a).map_or("blue", |m| m.color.as_str())) })
            })))
        })
        .when(attention > 0, |d| {
            d.child(div().id(SharedString::from(format!("att-{}", slot.monday))).flex().items_center().gap(px(space::XS)).text_size(px(font::CALLOUT)).text_color(t.red).child(icon("circle-alert", iconsize::XS, t.red)).child(attention.to_string()).tooltip(tip(trf("%lld brauchen Aufmerksamkeit", &[&attention]))))
        })
        .child(if mandatory.is_empty() {
            div().text_size(px(font::CALLOUT)).text_color(t.tertiary).child(if entries.is_empty() { tr("frei") } else { tr("nur optional") })
        } else {
            div().w(px(40.)).text_right().font_features(tabular()).text_size(px(font::CALLOUT)).text_color(if done == mandatory.len() { t.green } else { t.secondary }).child(format!("{done}/{}", mandatory.len()))
        });

    let anchor = st.target.as_ref().filter(|(m, _)| *m == slot.monday).map(|(_, a)| a.clone());
    div()
        .id(SharedString::from(format!("week-row-{}", slot.monday)))
        .flex()
        .flex_col()
        .when(is_current, |d| d.bg(t.accent.opacity(0.06)))
        .when_some(anchor, |d, a| d.anchor_scroll(Some(a)))
        .child(header)
        .when(!entries.is_empty(), |d| {
            d.child(motion::reveal(
                SharedString::from(format!("week-body-{monday}")),
                open,
                div()
                    .pl(px(space::XL + space::L))
                    .pr(px(space::L))
                    .pb(px(space::M))
                    .flex()
                    .flex_wrap()
                    .gap(px(space::S))
                    .children(shown.into_iter().map(|a| deadline_chip(lib, a, t)))
                    .when(!reveal && hidden > 0, |d| {
                        d.child(link_button(SharedString::from(format!("more-{}", monday)), format!("+ {hidden} {}", tr("optional")), t).py(px(space::S)).text_size(px(font::CALLOUT)).on_click(move |_, _, cx| v2.update(cx, |p, cx| {
                            p.grid.reveal_optional.insert(monday);
                            cx.notify();
                        })))
                    }),
            ))
        })
        .into_any_element()
}

/// Neutral chip with a module-colored edge; status reads from the leading control and the subtitle.
fn deadline_chip(lib: &Library, a: &Assignment, t: &Theme) -> AnyElement {
    let e = lib.evaluation(a);
    let module = lib.module(a);
    let code = module.map_or(String::new(), |m| m.code.clone());
    let tint = t.palette(module.map_or("blue", |m| m.color.as_str()));
    let selected = lib.is_selected(&a.id);
    let attention = lib.needs_action(a);
    let when = day::due(a.due, lib.now);
    let subtitle = if attention {
        format!("{code} · {}", if e.status == Status::Ready { tr("Upload bestätigen") } else { tr("Überfällig") })
    } else if e.status.is_closed() {
        format!("{code} · {}", e.status.label())
    } else if a.optional {
        trf("%@ · %@ · optional", &[&code, &when])
    } else {
        format!("{code} · {when}")
    };
    let (sel, menu_id, drop_id) = (a.id.clone(), a.id.clone(), a.id.clone());
    let (fill, hover, accent) = (t.ink.opacity(0.05), t.ink.opacity(0.1), t.accent);
    let tracks = a.tracks_files();
    let title = div().text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).truncate().text_color(if e.status.is_closed() { t.secondary } else { t.primary }).child(a.title.clone());
    let chip = div()
        .id(SharedString::from(format!("chip-{}", a.id)))
        .relative()
        .flex()
        .items_center()
        .gap(px(space::S))
        .pl(px(space::M))
        .pr(px(space::M))
        .py(px(space::S))
        .rounded(px(radius::CONTROL))
        .bg(if selected { hover } else { fill })
        .soft(hover, radius::CONTROL)
        .when(selected, |d| d.border(px(2.)).border_color(accent))
        .cursor_pointer().key_nav()
        .tooltip(tip(format!("{} · {} · {}", a.title, when, e.status.label())))
        .on_click(move |_, _, cx| act(cx, |l| l.select(Some(sel.clone()))))
        .when(tracks, |c| {
            c.drag_over::<ExternalPaths>(move |s, _, _, _| s.border(px(1.5)).border_color(accent)).on_drop(move |p: &ExternalPaths, _, cx| {
                let paths = p.paths().to_vec();
                act(cx, |l| l.request_filing(paths, Some(drop_id.clone())));
            })
        })
        .child(div().absolute().top_0().bottom_0().left_0().w(px(3.)).rounded_l(px(radius::CONTROL)).bg(tint.opacity(if e.status.is_closed() { 0.35 } else { 1. })))
        .child(status_toggle(a, lib, iconsize::M, t))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(space::XXS))
                .max_w(px(260.))
                .child(if e.status == Status::Skipped { title.line_through() } else { title })
                .child(div().text_size(px(font::CAPTION)).truncate().text_color(if attention { if e.status == Status::Ready { t.blue } else { t.red } } else { t.secondary }).child(subtitle)),
        )
        .context_menu(move |menu, _, cx| status_menu(menu, &menu_id, cx));
    motion::flip(format!("chip-{}", a.id), chip).into_any_element()
}
