use chrono::Days;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::board;
use super::state::act;
use super::motion::Soft;
use super::theme::{Theme, font, space, iconsize, radius};
use super::widgets::*;
use crate::core::day;
use crate::core::i18n::{tr, trf};
use crate::core::library::{Library, Route};
use crate::core::model::{Assignment, Kind, Status};
use crate::core::next_action::NextAction;

/// The overview: date header, the next action with two tiles, bundled attention rows, the 7-day board and upcoming exams.
pub fn render(lib: &Library, width: f32, t: &Theme) -> Div {
    let today = lib.today();
    let days: Vec<_> = (0..7).map(|i| today + Days::new(i)).collect();
    let empty = lib.semester_assignments().is_empty();
    div()
        .flex()
        .flex_col()
        .gap(px(space::XL))
        .p(px(space::XL))
        .child(header(lib, t))
        .when(!empty, |d| d.child(top_row(lib, width < 860., t)))
        .when(empty, |d| d.child(getting_started(t)))
        .children(attention(lib, t))
        .children(inbox_hint(lib, t))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(space::S))
                .child(section_title(tr("Nächste 7 Tage")))
                .child(board::render(lib, &days, width < 7. * 100. + 36. + 48., t)),
        )
        .children(exams(lib, t))
}

fn header(lib: &Library, t: &Theme) -> Div {
    let mut sub: Vec<String> = Vec::new();
    if let Some(s) = lib.semester() {
        sub.push(s.name.clone());
    }
    if let Some(w) = lib.current_week() {
        sub.push(trf("Vorlesungswoche %lld · KW%@", &[&w.index, &format!("{:02}", w.iso_week)]));
    } else if let Some(b) = lib.semester().and_then(|s| s.break_at(lib.today())) {
        sub.push(b.name.clone());
    }
    div()
        .flex()
        .flex_col()
        .gap(px(space::XS))
        .child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(day::long(lib.today())))
        .child(div().text_size(px(font::BODY)).text_color(t.secondary).child(sub.join("  ·  ")))
}

fn select(id: String) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, _, cx| act(cx, |l| l.select(Some(id.clone())))
}

fn top_row(lib: &Library, narrow: bool, t: &Theme) -> Div {
    let hero = next_action(lib, t);
    let tiles = div().flex().gap(px(space::M)).child(week_tile(lib, t)).child(streak_tile(lib, t));
    if narrow {
        div().flex().flex_col().gap(px(space::M)).child(hero).child(tiles)
    } else {
        div().flex().items_stretch().gap(px(space::M)).child(hero.flex_1().min_w_0()).child(tiles.flex_none().w(px(380.)))
    }
}

fn next_action(lib: &Library, t: &Theme) -> Div {
    let card = |tint: Hsla| banner(tint).flex().items_center().gap(px(space::L)).min_h(px(112.));
    let label = |tint: Hsla| div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(tint).child(tr("Jetzt dran"));
    let assignment = |a: &Assignment, tint: Hsla, when: String| {
        let m = lib.module(a);
        let module_tint = t.palette(m.map_or("blue", |m| m.color.as_str()));
        div()
            .id("next-action")
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .cursor_pointer()
            .key_nav()
            .on_click(select(a.id.clone()))
            .child(label(tint))
            .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::BOLD).truncate().child(a.title.clone()))
            .child(div().flex().items_center().gap(px(space::S)).text_size(px(font::CALLOUT)).text_color(t.secondary).child(div().font_weight(FontWeight::SEMIBOLD).child(module_tag(m.map_or("?", |m| m.code.as_str()), module_tint))).child(when))
    };
    let file_button = |a: &Assignment| {
        let id = a.id.clone();
        let has_files = !lib.evaluation(a).files.is_empty();
        (a.kind == Kind::Submission).then(|| {
            let button = Button::new("next-file").primary();
            if has_files {
                button.label(tr("Hochladen")).on_click(move |_, _, cx| super::upload::begin(&id, cx))
            } else {
                button.label(tr("Datei ablegen…")).on_click(move |_, _, cx| super::platform::pick_files_for_filing(Some(id.clone()), cx))
            }
        })
    };
    match lib.next_action() {
        NextAction::Overdue(a) => card(t.red).child(status_toggle(a, lib, iconsize::L, t)).child(assignment(a, t.red, day::due(a.due, lib.now))).children(file_button(a)),
        NextAction::Due(a) => card(t.accent).child(status_toggle(a, lib, iconsize::L, t)).child(assignment(a, t.accent_text, format!("{} · {}", day::countdown(a.due, lib.now), day::due(a.due, lib.now)))).children(file_button(a)),
        NextAction::Unconfirmed(a) => {
            let id = a.id.clone();
            card(t.blue)
                .child(status_toggle(a, lib, iconsize::L, t))
                .child(assignment(a, t.blue, tr("Fertig, aber Upload nicht bestätigt")))
                .child(Button::new("next-confirm").primary().label(tr("Als hochgeladen markieren")).on_click(move |_, _, cx| act(cx, |l| l.confirm_all(std::slice::from_ref(&id)))))
        }
        NextAction::Inbox(count) => card(t.accent)
            .child(icon("inbox", iconsize::L, t.accent))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .child(label(t.accent_text))
                    .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::BOLD).child(inbox_text(count)))
                    .child(div().text_size(px(font::CALLOUT)).text_color(t.secondary).child(tr("Abgabe, Titel und Name werden vorgeschlagen."))),
            )
            .child(Button::new("file-now").primary().label(tr("Jetzt ablegen")).on_click(|_, _, cx| act(cx, |l| l.set_route(Route::Inbox)))),
        NextAction::Clear => card(t.green).child(icon("circle-check", iconsize::L, t.green)).child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::BOLD).child(tr("Nichts offen"))),
    }
}

fn week_tile(lib: &Library, t: &Theme) -> Stateful<Div> {
    let (done, total) = lib.week_progress();
    tile("week", tr("Diese Woche"), "calendar", t.secondary, false, t).child(
        div()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(trf("%lld von %lld", &[&done, &total])))
            .child(progress_bar(done as f32 / total.max(1) as f32, if total > 0 && done == total { t.green } else { t.accent }, t))
            .child(caption(tr("Pflichtabgaben erledigt"), t)),
    )
}

fn streak_tile(lib: &Library, t: &Theme) -> Stateful<Div> {
    let streak = lib.streak();
    tile("streak", tr("Pünktlich in Folge"), "flame", t.orange, false, t).child(
        div().flex().flex_col().gap(px(space::XS)).child(div().text_size(px(font::TITLE)).font_weight(FontWeight::BOLD).child(streak.to_string())).child(caption(
            match streak {
                0 => tr("Nächste Abgabe startet die Serie"),
                1 => tr("Abgabe"),
                _ => tr("Abgaben"),
            },
            t,
        )),
    )
}

fn chips(lib: &Library, items: &[&Assignment], t: &Theme) -> Div {
    div().flex().flex_wrap().gap(px(space::S)).children(items.iter().map(|a| {
        let m = lib.module(a);
        let tint = t.palette(m.map_or("blue", |m| m.color.as_str()));
        let selected = lib.is_selected(&a.id);
        let (hover, menu_id) = (t.content.blend(t.ink.opacity(0.1)), a.id.clone());
        let chip = div()
            .id(SharedString::from(format!("chip-{}", a.id)))
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::M))
            .py(px(space::S))
            .rounded_full()
            .bg(t.content)
            .soft(hover, radius::FULL)
            .cursor_pointer().key_nav()
            .when(selected, |c| c.border(px(2.)).border_color(t.accent))
            .on_click(select(a.id.clone()))
            .child(status_toggle(a, lib, iconsize::M, t))
            .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).child(module_tag(m.map_or("?", |m| m.code.as_str()), tint)))
            .child(div().text_size(px(font::CALLOUT)).child(a.title.clone()))
            .child(caption(day::day_label(a.due, lib.now), t))
            .context_menu(move |menu, _, cx| status_menu(menu, &menu_id, cx));
        super::motion::flip(format!("chip-{}", a.id), chip)
    }))
}

/// Most chips a group shows; the rest are summed up.
const CHIP_LIMIT: usize = 30;

/// How many entries each module has, largest first: "P1 8 · SuM 4".
fn per_module(lib: &Library, items: &[&Assignment]) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for a in items {
        let code = lib.module(a).map_or("?", |m| m.code.as_str());
        match counts.iter_mut().find(|(c, _)| c == code) {
            Some((_, n)) => *n += 1,
            None => counts.push((code.to_string(), 1)),
        }
    }
    counts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    counts.iter().map(|(c, n)| format!("{c} {n}")).collect::<Vec<_>>().join(" · ")
}

/// A one-line sum of entries that unfolds into their chips; short groups stay open.
fn bundle(id: &'static str, title: String, icon_name: &str, tint: Hsla, open: bool, toggle: fn(&mut Library), accessory: Option<AnyElement>, lib: &Library, items: &[&Assignment], t: &Theme) -> Div {
    let fixed = items.len() <= 3;
    let open = open || fixed;
    let summary = per_module(lib, items);
    let hidden = items.len().saturating_sub(CHIP_LIMIT);
    let shown = &items[..items.len().min(CHIP_LIMIT)];
    let body = div().flex().flex_col().gap(px(space::S)).pt(px(space::S)).child(chips(lib, shown, t)).when(hidden > 0, |d| d.child(caption(trf("und %lld weitere", &[&hidden]), t)));
    banner(tint)
        .py(px(space::S))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::M))
                .child(
                    div()
                        .id(id)
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .when(!fixed, |d| {
                            d.cursor_pointer().key_nav().on_click(move |_, _, cx| {
                                super::state::lib(cx).update(cx, |l, cx| {
                                    toggle(l);
                                    cx.notify();
                                })
                            })
                        })
                        .when(!fixed, |d| d.child(super::motion::chevron(SharedString::from(format!("{id}-chevron")), open, tint)))
                        .child(icon(icon_name, iconsize::S, tint))
                        .child(div().flex_none().font_weight(FontWeight::BOLD).text_color(t.readable(tint)).child(title))
                        .child(div().min_w_0().truncate().text_size(px(font::CALLOUT)).text_color(t.secondary).child(summary)),
                )
                .children(accessory),
        )
        .child(super::motion::reveal(SharedString::from(format!("{id}-body")), open, body))
}

fn attention(lib: &Library, t: &Theme) -> Option<Div> {
    let unconfirmed = lib.unconfirmed();
    let overdue: Vec<&Assignment> = lib.action_items().iter().filter(|a| lib.status(a) != Status::Ready).collect();
    if unconfirmed.is_empty() && overdue.is_empty() {
        return None;
    }
    let mut col = div().flex().flex_col().gap(px(space::S));
    if !overdue.is_empty() {
        col = col.child(bundle("overdue", format!("{} ({})", tr("Überfällig"), overdue.len()), "circle-alert", t.red, lib.show_overdue, |l| l.show_overdue = !l.show_overdue, None, lib, &overdue, t));
    }
    if !unconfirmed.is_empty() {
        let ids: Vec<String> = unconfirmed.iter().map(|a| a.id.clone()).collect();
        let button = Button::new("confirm-all").small().label(tr("Alle als hochgeladen markieren")).on_click(move |_, _, cx| act(cx, |l| l.confirm_all(&ids))).into_any_element();
        col = col.child(bundle("unconfirmed", format!("{} ({})", tr("Fertig, aber Upload nicht bestätigt"), unconfirmed.len()), "circle-check", t.blue, lib.show_unconfirmed, |l| l.show_unconfirmed = !l.show_unconfirmed, Some(button), lib, &unconfirmed, t));
    }
    Some(col)
}

fn inbox_text(count: usize) -> String {
    if count == 1 { tr("1 Datei wartet in der Inbox") } else { trf("%lld Dateien warten in der Inbox", &[&count]) }
}

fn inbox_hint(lib: &Library, t: &Theme) -> Option<Div> {
    let count = lib.inbox.len();
    (count > 0 && !matches!(lib.next_action(), NextAction::Inbox(_))).then(|| {
        banner(t.accent)
            .py(px(space::S))
            .flex()
            .items_center()
            .gap(px(space::S))
            .child(icon("inbox", iconsize::S, t.accent))
            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(inbox_text(count)))
            .child(Button::new("file-now").small().label(tr("Jetzt ablegen")).on_click(|_, _, cx| act(cx, |l| l.set_route(Route::Inbox))))
    })
}

fn getting_started(t: &Theme) -> Div {
    let step = |icon_name: &str, title: String, body: String| {
        div().flex_1().flex().flex_col().gap(px(space::XS)).child(icon(icon_name, iconsize::M, t.accent)).child(div().font_weight(FontWeight::MEDIUM).child(title)).child(div().text_size(px(font::CALLOUT)).text_color(t.secondary).child(body))
    };
    banner(t.accent)
        .p(px(space::L))
        .flex()
        .flex_col()
        .gap(px(space::M))
        .child(div().font_weight(FontWeight::BOLD).child(tr("So kommen deine Deadlines in die App")))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(space::M))
                .child(step("calendar-plus", tr("Aus Moodle importieren"), tr("Kalender → Exportieren → .ics, dann „Ablage → Kalender importieren“.")))
                .child(step("repeat", tr("Wiederkehrende Abgaben"), tr("Einstellungen → Module → Regel hinzufügen, z. B. jeden Montag 18:00.")))
                .child(step("circle-plus", tr("Einzeln hinzufügen"), trf("%@ für Aufgaben, Abgaben und Prüfungen.", &[&crate::core::shortcuts::ShortcutAction::NewEntry.shortcut().map_or_else(|| tr("Plus"), |s| s.display())]))),
        )
}

fn exams(lib: &Library, t: &Theme) -> Option<Div> {
    let exams = lib.upcoming_exams();
    if exams.is_empty() {
        return None;
    }
    let row = div().flex().gap(px(space::M)).children(exams.into_iter().take(4).map(|a| {
        let m = lib.module(a);
        let tint = t.palette(m.map_or("blue", |m| m.color.as_str()));
        tile(SharedString::from(format!("exam-{}", a.id)), m.map_or(String::new(), |m| m.code.clone()), "graduation-cap", tint, true, t).on_click(select(a.id.clone())).child(
            div()
                .flex()
                .flex_col()
                .gap(px(space::XS))
                .child(div().font_weight(FontWeight::BOLD).line_clamp(2).child(a.title.clone()))
                .child(caption(day::due(a.due, lib.now), t))
                .child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::SEMIBOLD).child(day::countdown(a.due, lib.now))),
        )
    }));
    Some(div().flex().flex_col().gap(px(space::S)).child(section_title(tr("Prüfungen"))).child(row))
}
