use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::board;
use super::state::act;
use super::motion::Soft;
use super::theme::{Theme, control, font, hover, space, iconsize, radius};
use super::widgets::*;
use crate::core::day;
use crate::core::i18n::{tr, trf};
use crate::core::library::{Library, Route};
use crate::core::model::{Assignment, Kind};
use crate::core::next_action::NextAction;

/// The overview: date header, the setup list while something is open, the next action with two tiles, bundled attention rows, the calendar and upcoming exams.
/// `setup` is the list; `covers_calendar` says it already offers the Moodle calendar, so the empty state need not.
/// `moodle` asks for the calendar address inside the empty state.
pub fn render(lib: &Library, width: f32, setup: Option<AnyElement>, covers_calendar: bool, moodle: AnyElement, t: &Theme) -> Div {
    let empty = lib.semester_assignments().is_empty();
    div().w_full().flex().justify_center().child(
        div()
            .w_full()
            .max_w(px(control::CONTENT_MAX))
            .flex()
            .flex_col()
            .gap(px(space::XL))
            .p(px(space::XL))
            .child(header(lib, t))
            .children(setup)
            .when(!empty, |d| d.child(top_row(lib, width < 860., t)))
            .when(empty && !covers_calendar, |d| d.child(getting_started(lib, moodle, t)))
            .children(open_row(lib, t))
            .child(board::render(width < 7. * 100. + 36. + 48.))
            .children(inbox_hint(lib, t))
            .children(cards_hint(t))
            .children(exams(lib, t))
            .children(recent_notes(t)),
    )
}

/// Notes newest first, optionally only below `within`; computed once per vault scan and folder.
pub(super) fn recent_notes_in(within: Option<&std::path::Path>, limit: usize) -> Vec<(std::path::PathBuf, f64)> {
    type Recent = std::collections::HashMap<(Option<std::path::PathBuf>, usize), Vec<(std::path::PathBuf, f64)>>;
    thread_local! {
        static CACHE: std::cell::RefCell<(u64, std::path::PathBuf, Recent)> = std::cell::RefCell::new(Default::default());
    }
    let vault = crate::core::vault::index::current();
    CACHE.with_borrow_mut(|(generation, root, recent)| {
        if *generation != vault.generation || *root != vault.root {
            (*generation, *root) = (vault.generation, vault.root.clone());
            recent.clear();
        }
        recent
            .entry((within.map(std::path::Path::to_path_buf), limit))
            .or_insert_with(|| {
                let within = within.map(|w| dunce::canonicalize(w).unwrap_or_else(|_| w.to_path_buf()));
                let within = within.as_deref();
                let mut notes: Vec<(std::path::PathBuf, f64)> = vault.notes.iter().filter(|(p, _)| !vault.is_template(p) && crate::core::markdown::is_markdown(p) && within.is_none_or(|w| p.starts_with(w))).map(|(p, f)| (p.clone(), f.modified)).collect();
                notes.sort_by(|a, b| b.1.total_cmp(&a.1));
                notes.truncate(limit);
                notes
            })
            .clone()
    })
}

/// Files that are not notes, newest first, below `within`, from the scan's own dates; cached per vault generation and folder.
pub(super) fn recent_files_in(within: &std::path::Path, limit: usize) -> Vec<(std::path::PathBuf, f64)> {
    type Recent = std::collections::HashMap<(std::path::PathBuf, usize), Vec<(std::path::PathBuf, f64)>>;
    thread_local! {
        static CACHE: std::cell::RefCell<((u64, u64), std::path::PathBuf, Recent)> = std::cell::RefCell::new(Default::default());
    }
    let vault = crate::core::vault::index::current();
    let hidden = crate::core::vault::index::hidden();
    CACHE.with_borrow_mut(|(generation, root, recent)| {
        if *generation != (vault.generation, hidden.generation) || *root != vault.root {
            (*generation, *root) = ((vault.generation, hidden.generation), vault.root.clone());
            recent.clear();
        }
        recent
            .entry((within.to_path_buf(), limit))
            .or_insert_with(|| {
                let hidden = crate::core::vault::index::hidden();
                let mut files: Vec<(std::path::PathBuf, f64)> = vault
                    .files
                    .iter()
                    .filter(|p| p.starts_with(&within) && !crate::core::markdown::is_markdown(p) && !hidden.hides(p))
                    .filter_map(|p| Some((p.clone(), *vault.file_times.get(p)?)))
                    .collect();
                files.sort_by(|a, b| b.1.total_cmp(&a.1));
                files.truncate(limit);
                files
            })
            .clone()
    })
}

/// What was written last, one click away.
fn recent_notes(t: &Theme) -> Option<Div> {
    let notes = recent_notes_in(None, 5);
    if notes.is_empty() {
        return None;
    }
    let now = chrono::Local::now().timestamp() as f64;
    let rows = notes.into_iter().enumerate().map(|(i, (path, modified))| {
        let (open, menu) = (path.clone(), path.clone());
        inset_file_row(("recent-note", i), &path, path.file_stem().unwrap_or_default().to_string_lossy().to_string(), t)
            .child(caption(super::notes_page::changed_ago(now - modified), t).flex_none())
            .on_click(move |_, _, cx| act(cx, |l| l.open_file(&open)))
            .context_menu(move |m, window, cx| {
                let m = deadline_submenu(file_menu(m, menu.clone()).when_some(super::convention_rename::menu_item_or_hint(&menu, cx), |m, item| m.item(item)).separator(), &menu, window, cx);
                super::ai::file_submenu(m, &menu, window, cx)
            })
    });
    Some(div().flex().flex_col().gap(px(space::S)).child(section_title(tr("Zuletzt geschrieben"))).child(inset_list(t).children(rows)))
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
    let guessed = lib.semester().is_some_and(crate::core::workspace::is_guessed) && lib.root.as_ref().is_none_or(|r| !crate::core::prefs::setup_of(r).confirmed.contains("semester"));
    if guessed {
        sub.push(tr("Vorlesungszeit geschätzt"));
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
    let streak = lib.streak() > 0;
    let tiles = div().flex().gap(px(space::M)).child(week_tile(lib, t)).when(streak, |d| d.child(streak_tile(lib, t)));
    if narrow {
        div().flex().flex_col().gap(px(space::M)).child(hero).child(tiles)
    } else {
        div().flex().items_stretch().gap(px(space::M)).child(hero.flex_1().min_w_0()).child(tiles.flex_none().w(px(if streak { 380. } else { 190. })))
    }
}

/// The card for the one thing to do now: neutral, the urgency is the order in which things are offered.
fn next_action(lib: &Library, t: &Theme) -> Stateful<Div> {
    let card = || div().id("next-card").p(px(space::L)).rounded(px(radius::CARD)).bg(t.secondary_bg).border_1().border_color(t.ink.opacity(0.1)).flex().items_center().gap(px(space::L)).min_h(px(112.));
    let open = |a: &Assignment| card().soft(t.ink.opacity(hover::ROW), radius::CARD).cursor_pointer().on_click(select(a.id.clone()));
    let label = || div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.accent_text).child(tr("Jetzt"));
    let assignment = |a: &Assignment, when: String| {
        let m = lib.module(a);
        let module_tint = t.palette(m.map_or("blue", |m| m.color.as_str()));
        div()
            .id("next-action")
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .cursor_pointer()
            .key_nav()
            .on_click({
                let id = a.id.clone();
                move |_, _, cx| {
                    cx.stop_propagation();
                    act(cx, |l| l.select(Some(id.clone())));
                }
            })
            .child(label())
            .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::BOLD).truncate().child(a.title.clone()))
            .child(div().flex().items_center().gap(px(space::S)).text_size(px(font::CALLOUT)).text_color(t.secondary).child(div().font_weight(FontWeight::SEMIBOLD).child(module_tag(&m.map_or("?".to_string(), |m| m.badge()), module_tint))).child(when))
    };
    let own = |id: &'static str, button: Button| div().id(id).flex_none().block_mouse_except_scroll().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(|_, _, cx| cx.stop_propagation()).child(button);
    let file_button = |a: &Assignment| {
        let id = a.id.clone();
        let has_files = !lib.evaluation(a).files.is_empty();
        (a.kind == Kind::Submission).then(|| {
            let button = Button::new("next-file").primary();
            own("next-file-wrap", if has_files {
                button.label(tr("Hochladen")).on_click(move |_, _, cx| super::upload::begin(&id, cx))
            } else {
                button.label(tr("Datei ablegen…")).on_click(move |_, _, cx| super::platform::pick_files_for_filing(Some(id.clone()), cx))
            })
        })
    };
    match lib.next_action() {
        NextAction::Overdue(a) => open(a).child(status_toggle(a, lib, iconsize::L, t)).child(assignment(a, day::due(a.due, lib.now))).children(file_button(a)).child(div().flex_1()),
        NextAction::Due(a) => open(a).child(status_toggle(a, lib, iconsize::L, t)).child(assignment(a, format!("{} · {}", day::countdown(a.due, lib.now), day::due(a.due, lib.now)))).children(file_button(a)).child(div().flex_1()),
        NextAction::Unconfirmed(a) => {
            let id = a.id.clone();
            open(a)
                .child(status_toggle(a, lib, iconsize::L, t))
                .child(assignment(a, tr("Fertig, aber Upload nicht bestätigt")))
                .child(own("next-confirm-wrap", Button::new("next-confirm").primary().label(tr("Als hochgeladen markieren")).on_click(move |_, _, cx| act(cx, |l| l.confirm_all(std::slice::from_ref(&id))))))
                .child(div().flex_1())
        }
        NextAction::Inbox(count) => card()
            .child(icon("inbox", iconsize::L, t.accent))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .child(label())
                    .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::BOLD).child(inbox_text(count)))
                    .child(div().text_size(px(font::CALLOUT)).text_color(t.secondary).child(tr("Abgabe, Titel und Name werden vorgeschlagen."))),
            )
            .child(Button::new("file-now").primary().label(tr("Jetzt ablegen")).on_click(|_, _, cx| act(cx, |l| l.set_route(Route::Inbox))))
            .child(div().flex_1()),
        NextAction::Clear => card().child(icon("circle-check", iconsize::L, t.green)).child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::BOLD).child(tr("Nichts offen"))),
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
        div().flex().flex_col().gap(px(space::XS)).child(div().text_size(px(font::TITLE)).font_weight(FontWeight::BOLD).child(streak.to_string())).child(caption(if streak == 1 { tr("Abgabe") } else { tr("Abgaben") }, t)),
    )
}

fn chips(lib: &Library, items: &[&Assignment], t: &Theme) -> Div {
    div().flex().flex_wrap().gap(px(space::S)).children(items.iter().map(|a| {
        let m = lib.module(a);
        let tint = t.palette(m.map_or("blue", |m| m.color.as_str()));
        let selected = lib.is_selected(&a.id);
        let (over, menu_id) = (t.content.blend(t.ink.opacity(hover::CONTROL)), a.id.clone());
        let chip = div()
            .id(SharedString::from(format!("chip-{}", a.id)))
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::M))
            .py(px(space::S))
            .rounded_full()
            .bg(t.content)
            .soft(over, radius::FULL)
            .cursor_pointer().key_nav()
            .when(selected, |c| c.border(px(2.)).border_color(t.accent))
            .on_click(select(a.id.clone()))
            .child(status_toggle(a, lib, iconsize::M, t))
            .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).child(module_tag(&m.map_or("?".to_string(), |m| m.badge()), tint)))
            .child(div().text_size(px(font::CALLOUT)).child(a.title.clone()))
            .child(caption(day::day_label(a.due, lib.now), t))
            .context_menu(move |menu, window, cx| super::ai::deadline_submenu(status_menu(menu, &menu_id, cx), &menu_id, window, cx));
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
fn bundle(id: &'static str, title: String, open: bool, toggle: fn(&mut Library), accessory: Option<AnyElement>, lib: &Library, items: &[&Assignment], t: &Theme) -> Div {
    let fixed = items.len() <= 3;
    let open = open || fixed;
    let summary = per_module(lib, items);
    let hidden = items.len().saturating_sub(CHIP_LIMIT);
    let shown = &items[..items.len().min(CHIP_LIMIT)];
    let body = div().flex().flex_col().gap(px(space::S)).pt(px(space::S)).child(chips(lib, shown, t)).when(hidden > 0, |d| d.child(caption(trf("und %lld weitere", &[&hidden]), t)));
    div()
        .px(px(space::L))
        .py(px(space::S))
        .rounded(px(radius::CARD))
        .bg(t.secondary_bg)
        .border_1()
        .border_color(t.ink.opacity(0.1))
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
                        .when(!fixed, |d| d.mx(px(-space::S)).px(px(space::S)).my(px(-space::XS)).py(px(space::XS)).min_h(px(control::S)).rounded(px(radius::CONTROL)))
                        .when(!fixed, |d| {
                            d.soft(t.ink.opacity(hover::ROW), radius::CONTROL).cursor_pointer().key_nav().on_click(move |_, _, cx| {
                                super::state::lib(cx).update(cx, |l, cx| {
                                    toggle(l);
                                    cx.notify();
                                })
                            })
                        })
                        .when(!fixed, |d| d.child(super::motion::chevron(SharedString::from(format!("{id}-chevron")), open, t.secondary)))
                        .child(icon("list-todo", iconsize::S, t.secondary))
                        .child(div().flex_none().font_weight(FontWeight::BOLD).child(title))
                        .child(div().min_w_0().truncate().text_size(px(font::CALLOUT)).text_color(t.secondary).child(summary)),
                )
                .children(accessory),
        )
        .child(super::motion::reveal(SharedString::from(format!("{id}-body")), open, body))
}

/// Everything still open besides the card above, folded to one row with a way to settle it all at once.
fn open_row(lib: &Library, t: &Theme) -> Option<Div> {
    let hero = match lib.next_action() {
        NextAction::Overdue(a) | NextAction::Unconfirmed(a) => Some(a.id.as_str()),
        _ => None,
    };
    let items: Vec<&Assignment> = lib.action_items().iter().filter(|a| Some(a.id.as_str()) != hero).collect();
    if items.is_empty() {
        return None;
    }
    let ids: Vec<String> = items.iter().map(|a| a.id.clone()).collect();
    let today = day::local_date(lib.now);
    let (tomorrow, next_week) = (today + chrono::Days::new(1), day::monday(today) + chrono::Days::new(7));
    let moving = ids.clone();
    let shift = Button::new("move-all").small().ghost().label(tr("Alle verschieben")).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
        let (a, b) = (moving.clone(), moving.clone());
        menu.item(PopupMenuItem::new(tr("Auf morgen")).on_click(move |_, _, cx| act(cx, |l| l.move_all(&a, tomorrow))))
            .item(PopupMenuItem::new(tr("Auf nächsten Montag")).on_click(move |_, _, cx| act(cx, |l| l.move_all(&b, next_week))))
    });
    let settle = div().flex().items_center().gap(px(space::XS)).child(shift).child(Button::new("complete-all").small().label(tr("Alle erledigt")).on_click(move |_, _, cx| act(cx, |l| l.complete_all(&ids)))).into_any_element();
    Some(bundle("overdue", format!("{} ({})", tr("Offen"), items.len()), lib.show_overdue, |l| l.show_overdue = !l.show_overdue, Some(settle), lib, &items, t))
}

fn inbox_text(count: usize) -> String {
    if count == 1 { tr("1 Datei wartet in der Inbox") } else { trf("%lld Dateien warten in der Inbox", &[&count]) }
}

fn hint_banner(id: &'static str, t: &Theme, go: impl Fn(&mut App) + 'static) -> Stateful<Div> {
    banner(t.accent).id(id).py(px(space::S)).flex().items_center().gap(px(space::S)).soft(t.ink.opacity(hover::ROW), radius::CARD).cursor_pointer().on_click(move |_, _, cx| go(cx))
}

fn hint_button(id: &'static str, label: String, go: impl Fn(&mut App) + 'static) -> Button {
    Button::new(id).small().label(label).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(move |_, _, cx| {
        cx.stop_propagation();
        go(cx);
    })
}

fn inbox_hint(lib: &Library, t: &Theme) -> Option<Stateful<Div>> {
    let count = lib.inbox.len();
    let go = |cx: &mut App| act(cx, |l| l.set_route(Route::Inbox));
    (count > 0 && !matches!(lib.next_action(), NextAction::Inbox(_))).then(|| {
        hint_banner("inbox-hint", t, go)
            .child(icon("inbox", iconsize::S, t.accent))
            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(inbox_text(count)))
            .child(hint_button("file-now", tr("Jetzt ablegen"), go))
    })
}

/// Cards waiting today, one key away.
fn cards_hint(t: &Theme) -> Option<Stateful<Div>> {
    let count = crate::core::cards::counts("", crate::core::cards::now()).total();
    let go = |cx: &mut App| {
        act(cx, |l| {
            l.cards_request = Some(String::new());
            l.set_route(Route::Cards);
        });
    };
    (count > 0).then(|| {
        hint_banner("cards-hint", t, go)
            .child(icon("layers", iconsize::S, t.accent))
            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(if count == 1 { tr("1 Karte wartet") } else { trf("%@ Karten warten", &[&count.to_string()]) }))
            .child(hint_button("cards-now", tr("Lernen"), go))
    })
}

/// The overview without a single deadline: the three ways to get some, each one click away.
fn getting_started(lib: &Library, moodle: AnyElement, t: &Theme) -> Div {
    let step = |icon_name: &str, title: String, body: String, action: AnyElement| {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .child(icon(icon_name, iconsize::M, t.accent))
            .child(div().font_weight(FontWeight::MEDIUM).child(title))
            .child(div().text_size(px(font::CALLOUT)).text_color(t.secondary).child(body))
            .child(div().flex_1())
            .child(div().pt(px(space::XS)).flex().child(action))
    };
    let button = |id: &'static str, label: String, go: Box<dyn Fn(&mut App)>| Button::new(id).small().label(label).on_click(move |_, _, cx| go(cx)).into_any_element();
    let first_module = lib.semester_modules().first().map(|m| m.id.clone());
    let weekly: Box<dyn Fn(&mut App)> = match first_module {
        Some(id) => Box::new(move |cx| super::settings::open_module(&id, cx)),
        None => Box::new(|cx| {
            act(cx, |l| {
                l.setup_only = Some(super::wizards::SETUP_MODULES);
                l.needs_setup = true;
            })
        }),
    };
    banner(t.accent)
        .p(px(space::L))
        .flex()
        .flex_col()
        .gap(px(space::M))
        .child(div().font_weight(FontWeight::BOLD).child(tr("So kommen deine Abgaben in die App")))
        .child(
            div()
                .flex()
                .gap(px(space::L))
                .child(step("calendar-plus", tr("Aus Moodle"), tr("Die Kalenderadresse aus Moodle eintragen, Abgaben kommen dann von selbst."), moodle))
                .child(step("repeat", tr("Jede Woche"), tr("Ein Übungsblatt jede Vorlesungswoche, einmal angelegt beim Modul."), button("start-weekly", tr("Wöchentliche Abgabe anlegen"), weekly)))
                .child(step("circle-plus", tr("Einzeln"), trf("Eine Abgabe, Aufgabe oder Prüfung mit Datum, auch mit %@.", &[&crate::core::shortcuts::ShortcutAction::NewEntry.active_shortcut().map_or_else(|| tr("dem Plus oben"), |s| s.display())]), button("start-single", tr("Abgabe eintragen"), Box::new(|cx| {
                    act(cx, |l| {
                        l.quick_add_kind = Some(Kind::Submission);
                        l.show_quick_add = true;
                    })
                })))),
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
        tile(SharedString::from(format!("exam-{}", a.id)), m.map_or(String::new(), |m| m.badge()), "graduation-cap", tint, true, t).on_click(select(a.id.clone())).child(
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
