use chrono::{Datelike, NaiveDate};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::act;
use super::motion::Soft;
use super::theme::{Theme, font, radius, space, iconsize};
use super::widgets::{KeyNav, icon, status_menu, status_toggle, tip};
use crate::core::i18n::tr;
use crate::core::library::Library;
use crate::core::model::{Assignment, Module, Session, Status};
use crate::core::day;

/// Column-per-day board showing timetable sessions and deadlines; one row per day when narrow.
pub fn render(lib: &Library, days: &[NaiveDate], narrow: bool, t: &Theme) -> Div {
    if narrow {
        return div().flex().flex_col().gap(px(space::S)).children(days.iter().map(|d| agenda_row(lib, *d, t)));
    }
    div().flex().gap(px(space::S)).children(days.iter().map(|d| column(lib, *d, t)))
}

fn day_label(d: NaiveDate, today: NaiveDate) -> String {
    if d == today { tr("Heute").to_uppercase() } else { day::weekday_short(d).to_uppercase() }
}

fn column(lib: &Library, d: NaiveDate, t: &Theme) -> AnyElement {
    let today = lib.today();
    let is_today = d == today;
    let weekend = day::is_weekend(d);
    let items = lib.assignments_on(d);
    let sessions = lib.sessions_on(d);
    let break_name = lib.semester().and_then(|s| s.break_at(d)).map(|b| b.name.clone());
    let open = items.iter().filter(|a| !lib.is_done(a)).count();
    let ink = t.ink;

    let header = div()
        .flex()
        .items_baseline()
        .gap(px(space::S))
        .pb(px(space::XXS))
        .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(if is_today { t.accent } else { t.secondary }).child(day_label(d, today)))
        .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).text_color(if is_today { t.accent } else { t.primary }).child(d.day().to_string()))
        .child(div().flex_1())
        .when(open > 0, |h| h.child(div().text_size(px(font::CAPTION2)).font_weight(FontWeight::BOLD).text_color(t.secondary).child(open.to_string())));

    div()
        .flex_1()
        .min_w_0()
        .min_h(px(230.))
        .p(px(space::S))
        .flex()
        .flex_col()
        .gap(px(space::S))
        .rounded(px(radius::CARD))
        .bg(if is_today {
            t.content.blend(t.accent.opacity(0.12))
        } else if weekend {
            t.content.blend(ink.opacity(0.02))
        } else {
            t.column
        })
        .border(px(if is_today { 1.5 } else { 1. }))
        .border_color(if is_today { t.accent.opacity(0.7) } else { ink.opacity(0.06) })
        .when(d < today, |c| c.opacity(0.6))
        .child(header)
        .when_some(break_name.clone(), |c, name| {
            c.child(div().flex().items_center().gap(px(space::XS)).py(px(space::XXS)).text_size(px(font::CAPTION)).text_color(t.secondary).child(icon("sun", iconsize::XS, t.secondary)).child(name))
        })
        .children(sessions.iter().map(|(m, s)| session_chip(m, s, t)))
        .children(items.iter().map(|a| deadline_card(lib, a, t)))
        .when(items.is_empty() && sessions.is_empty() && break_name.is_none(), |c| {
            c.child(div().w_full().flex().justify_center().py(px(space::M)).text_size(px(font::CAPTION)).text_color(t.tertiary).child(if weekend { tr("Wochenende") } else { tr("Frei") }))
        })
        .into_any_element()
}

fn agenda_row(lib: &Library, d: NaiveDate, t: &Theme) -> AnyElement {
    let today = lib.today();
    let is_today = d == today;
    let items = lib.assignments_on(d);
    let sessions = lib.sessions_on(d);
    div()
        .flex()
        .items_start()
        .gap(px(space::M))
        .p(px(space::M))
        .rounded(px(radius::CARD))
        .bg(if is_today { t.content.blend(t.accent.opacity(0.07)) } else { t.secondary_bg })
        .border(px(if is_today { 1.5 } else { 1. }))
        .border_color(if is_today { t.accent.opacity(0.7) } else { t.ink.opacity(0.06) })
        .child(
            div()
                .w(px(52.))
                .flex_none()
                .flex()
                .flex_col()
                .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(if is_today { t.accent } else { t.secondary }).child(day_label(d, today)))
                .child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::SEMIBOLD).text_color(if is_today { t.accent } else { t.primary }).child(d.day().to_string())),
        )
        .child(if items.is_empty() && sessions.is_empty() {
            div().pt(px(space::S)).text_size(px(font::CALLOUT)).text_color(t.tertiary).child(if day::is_weekend(d) { tr("Wochenende") } else { tr("Frei") }).into_any_element()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap(px(space::S))
                .children(sessions.iter().map(|(m, s)| div().w(px(200.)).child(session_chip(m, s, t))))
                .children(items.iter().map(|a| div().w(px(200.)).child(deadline_card(lib, a, t))))
                .into_any_element()
        })
        .into_any_element()
}

fn session_chip(m: &Module, s: &Session, t: &Theme) -> Stateful<Div> {
    let title = if s.title.is_empty() { m.code.clone() } else { s.title.clone() };
    let place = if s.location.is_empty() { String::new() } else { format!(" · {}", s.location) };
    div()
        .id(SharedString::from(format!("session-{}", s.id)))
        .flex()
        .gap(px(space::S))
        .py(px(space::XXS))
        .tooltip(tip(format!("{} · {} {}–{}", m.code, s.title, s.start, s.end)))
        .child(div().w(px(3.)).flex_none().rounded_full().bg(t.palette(&m.color).opacity(0.6)))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).truncate().child(title))
                .child(div().text_size(px(font::CAPTION2)).text_color(t.secondary).truncate().child(format!("{}–{}{place}", s.start, s.end))),
        )
}

pub fn deadline_card(lib: &Library, a: &Assignment, t: &Theme) -> AnyElement {
    let e = lib.evaluation(a);
    let module = lib.module(a);
    let tint = t.palette(module.map_or("blue", |m| m.color.as_str()));
    let closed = e.status.is_closed();
    let selected = lib.is_selected(&a.id);
    let attention = lib.needs_action(a);
    let border = if selected {
        Some(t.accent)
    } else if attention {
        Some(if e.status == Status::Ready { t.blue.opacity(0.7) } else { t.red.opacity(0.7) })
    } else if a.optional && !closed {
        Some(t.secondary.opacity(0.5))
    } else {
        None
    };
    let base = t.column;
    let fill = base.blend(tint.opacity(if closed { 0.05 } else { 0.12 }));
    let hover_fill = base.blend(tint.opacity(if closed { 0.05 } else { 0.2 }));
    let (select_id, menu_id, drop_id) = (a.id.clone(), a.id.clone(), a.id.clone());
    let tracks = a.tracks_files();
    let help = format!("{} · {}", a.title, day::due(a.due, lib.now));
    let accent = t.accent;

    let mut card = div()
        .id(SharedString::from(a.id.clone()))
        .relative()
        .w_full()
        .py(px(space::S))
        .pl(px(space::S))
        .pr(px(space::S))
        .flex()
        .flex_col()
        .gap(px(space::XS))
        .rounded(px(radius::CONTROL))
        .bg(if selected { hover_fill } else { fill })
        .soft(hover_fill, radius::CONTROL)
        .cursor_pointer().key_nav()
        .tooltip(tip(help))
        .on_click(move |_, _, cx| act(cx, |l| l.select(Some(select_id.clone()))))
        .when(tracks, |c| {
            c.drag_over::<ExternalPaths>(move |s, _, _, _| s.border_color(accent).border(px(1.5))).on_drop(move |paths: &ExternalPaths, _, cx| {
                let paths = paths.paths().to_vec();
                act(cx, |l| l.request_filing(paths, Some(drop_id.clone())));
            })
        })
        .child(div().absolute().top_0().bottom_0().left_0().w(px(3.)).rounded_l(px(radius::CONTROL)).bg(tint.opacity(if closed { 0.35 } else { 1. })))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::XS))
                .text_size(px(font::CAPTION))
                .child(status_toggle(a, lib, iconsize::M, t))
                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(tint).child(module.map_or(String::new(), |m| m.code.clone())))
                .child(div().text_color(t.secondary).child(day::hm(a.due))),
        )
        .child({
            let title = div().text_size(px(font::BODY)).font_weight(FontWeight::MEDIUM).line_clamp(3).text_color(if closed { t.secondary } else { t.primary }).child(a.title.clone());
            if e.status == Status::Skipped { title.line_through() } else { title }
        })
        .when(attention, |c| {
            let (label, color) = if e.status == Status::Ready { (tr("Upload bestätigen"), t.blue) } else { (tr("Überfällig"), t.red) };
            c.child(div().text_size(px(font::CAPTION2)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(label))
        });
    if let Some(color) = border {
        card = card.border(px(if selected || attention { 1.5 } else { 1. })).border_color(color);
        if a.optional && !closed && !selected {
            card = card.border_dashed();
        }
    }
    super::motion::flip(format!("card-{}", a.id), card.context_menu(move |menu, _, cx| status_menu(menu, &menu_id, cx))).into_any_element()
}
