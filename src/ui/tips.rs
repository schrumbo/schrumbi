use std::collections::BTreeSet;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::{self, Soft};
use super::state::{self, act};
use super::theme::{Theme, font, hints, iconsize, radius, space};
use super::widgets::*;
use super::widgets::Tip as _;
use crate::core::i18n::tr;
use crate::core::library::Route;
use crate::core::prefs;
use crate::core::tips::{self, Needs, Tip};

/// The tip on screen, if any.
#[derive(Default)]
struct Tips {
    shown: Option<&'static Tip>,
}

impl Global for Tips {}

/// How "Zeig's mir" shows a tip's feature.
enum Show {
    /// Goes to a page and marks the element with this id, or only says the label when the id is empty.
    Mark { route: Option<Route>, id: String, label: String },
    /// Runs the feature itself, e.g. opens the search it describes.
    Run(Box<dyn Fn(&mut Window, &mut App)>),
}

fn today() -> String {
    crate::core::day::format(chrono::Local::now().date_naive())
}

/// Counts the start and, from the second start on, shows one tip a day once the window has settled.
pub fn on_launch(cx: &mut App) {
    let launches = prefs::with(|p| p.launches) + 1;
    prefs::update(|p| p.launches = launches);
    if launches < 2 {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(motion::TIP_DELAY).await;
        for _ in 0..8 {
            let ready = cx.update(|cx| state::read(cx, |l| l.root.is_some() && l.last_error.is_none()) && !super::coach::active());
            if ready {
                cx.update(offer);
                return;
            }
            cx.background_executor().timer(motion::TIP_RETRY).await;
        }
    })
    .detach();
}

/// Shows today's tip unless tips are off or one already showed today.
fn offer(cx: &mut App) {
    let due = prefs::with(|p| p.tips_enabled && p.tips_last != today());
    if due {
        show_next(cx);
    }
}

fn available(cx: &App) -> Vec<&'static Tip> {
    let (modules, deadlines) = state::read(cx, |l| (!l.semester_modules().is_empty(), !l.assignments.is_empty()));
    tips::TIPS
        .iter()
        .filter(|t| match t.needs {
            Needs::Nothing => true,
            Needs::Modules => modules,
            Needs::Deadlines => deadlines,
        })
        .collect()
}

fn used(cx: &App) -> BTreeSet<String> {
    let mut used = prefs::with(|p| p.used_features.clone());
    if !crate::core::cards::all_cards().is_empty() {
        used.insert("cards".into());
    }
    if state::calendar_source(cx).is_some() {
        used.insert("calendar".into());
    }
    used
}

/// Picks and shows the next tip; with none left the card goes away.
pub fn show_next(cx: &mut App) {
    let seen = prefs::with(|p| p.tips_seen.clone());
    let next = tips::pick(&available(cx), &used(cx), &seen);
    cx.default_global::<Tips>().shown = next;
    if let Some(tip) = next {
        prefs::update(|p| {
            tips::remember(&mut p.tips_seen, tip.id);
            p.tips_last = today();
        });
    }
    cx.refresh_windows();
}

/// Closes the card for today.
pub fn dismiss(cx: &mut App) {
    cx.default_global::<Tips>().shown = None;
    cx.refresh_windows();
}

/// Switches tips off; the settings bring them back.
pub fn disable(cx: &mut App) {
    prefs::update(|p| p.tips_enabled = false);
    dismiss(cx);
    act(cx, |l| l.notify(tr("Keine Tipps mehr. Einstellungen → Allgemein → Hinweise schaltet sie wieder ein."), "lightbulb-off", Some(tr("Rückgängig")), Some(Box::new(|_| prefs::update(|p| p.tips_enabled = true)))));
}

/// Whether a tip is on screen.
pub fn showing(cx: &App) -> bool {
    cx.try_global::<Tips>().is_some_and(|t| t.shown.is_some())
}

/// `{key:…}` names in a tip text become the keys of this platform and the user's own shortcuts.
fn fill(text: &str, cx: &App) -> String {
    let mut out = tr(text);
    for name in tips::key_names(&out) {
        let keys = super::help::key_text(&name, cx).unwrap_or_else(|| tr("ohne Kürzel"));
        out = out.replace(&format!("{{key:{name}}}"), &keys);
    }
    out
}

fn first_module_tab(cx: &App) -> String {
    state::read(cx, |l| l.semester_modules().first().map(|m| format!("tab-module-{}", m.id)).unwrap_or_default())
}

fn mark(route: Option<Route>, id: String, label: String) -> Show {
    Show::Mark { route, id, label }
}

fn run(f: impl Fn(&mut Window, &mut App) + 'static) -> Show {
    Show::Run(Box::new(f))
}

fn next_toggle(cx: &App) -> String {
    state::read(cx, |l| match l.next_action() {
        crate::core::next_action::NextAction::Overdue(a) | crate::core::next_action::NextAction::Unconfirmed(a) | crate::core::next_action::NextAction::Due(a) => format!("toggle-{}", a.id),
        _ => l.semester_assignments().first().map(|a| format!("toggle-{}", a.id)).unwrap_or_default(),
    })
}

/// What "Zeig's mir" does for each tip; `None` for tips that are only words.
fn show(tip: &Tip, cx: &App) -> Option<Show> {
    Some(match tip.id {
        "quick-open" => run(|_, cx| act(cx, |l| l.show_quick_open = true)),
        "palette" => run(|_, cx| act(cx, |l| l.show_palette = true)),
        "drop-filing" => mark(None, first_module_tab(cx), tr("Zieh eine Datei hierher")),
        "status-circle" => mark(Some(Route::Today), next_toggle(cx), tr("Der Kreis setzt den Status")),
        "cards-syntax" => mark(Some(Route::Cards), "tab-Cards".into(), tr("Hier erscheinen deine Karten")),
        "new-entry" => mark(None, "coach-new".into(), tr("Neuer Eintrag: Abgabe, Aufgabe oder Prüfung")),
        "daily-note" => run(|window, cx| window.dispatch_action(Box::new(super::app::DailyNote), cx)),
        "split-view" => mark(Some(Route::Notes), "tab-Notes".into(), tr("Rechtsklick auf einen Tab, dann „Daneben öffnen“")),
        "pdf-embed" | "wikilink" | "math" | "tasks" | "export-pdf" => mark(Some(Route::Notes), "tab-Notes".into(), tr("Das geht in jeder Notiz")),
        "search-slash" => mark(None, "coach-search".into(), tr("Die Suche: / springt hierher")),
        "module-keys" => mark(None, first_module_tab(cx), tr("Die ersten drei Module haben eigene Kürzel")),
        "reschedule" => mark(Some(Route::Today), next_toggle(cx), tr("Rechtsklick auf den Eintrag")),
        "day-entry" => mark(Some(Route::Today), "tab-Today".into(), tr("Rechtsklick auf einen Tag unter „Nächste 7 Tage“")),
        "rename" => mark(Some(Route::Notes), "tab-Notes".into(), tr("Öffne eine Notiz, dann umbenennen")),
        "shortcuts" => run(|window, cx| window.dispatch_action(Box::new(super::app::ShowShortcuts), cx)),
        _ => return None,
    })
}

fn show_me(tip: &'static Tip, window: &mut Window, cx: &mut App) {
    dismiss(cx);
    match show(tip, cx) {
        Some(Show::Run(f)) => f(window, cx),
        Some(Show::Mark { route, id, label }) => {
            if let Some(route) = route {
                act(cx, |l| match route {
                    Route::Notes => l.show_notes(),
                    r => l.set_route(r),
                });
            }
            super::coach::start(&id, label, cx);
        }
        None => {}
    }
}

/// The card at the bottom right; `right` and `bottom` keep it clear of the details column and bars.
pub fn card(right: f32, bottom: f32, t: &Theme, cx: &App) -> Option<AnyElement> {
    let shown = cx.try_global::<Tips>()?.shown?;
    let showable = show(shown, cx).is_some();
    let travel = motion::timed(motion::SLOW, cx);
    let body = div()
        .id("tip-card")
        .occlude()
        .w(px(hints::TIP_WIDTH))
        .flex()
        .flex_col()
        .gap(px(space::M))
        .p(px(space::L))
        .rounded(px(radius::CARD))
        .bg(t.material)
        .border_1()
        .border_color(t.separator)
        .shadow_lg()
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(space::M))
                .child(div().flex_none().size(px(space::XXL)).rounded(px(radius::INNER)).bg(t.accent.opacity(hints::ICON_TINT)).flex().items_center().justify_center().child(icon(shown.icon, iconsize::M, t.accent_text)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(space::XS))
                        .child(div().flex().items_center().gap(px(space::S)).child(div().flex_1().min_w_0().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(tr("Tipp des Tages"))).child(
                            div()
                                .id("tip-close")
                                .flex_none()
                                .size(px(space::L))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .key_nav()
                                .soft(t.ink.opacity(0.1), radius::FULL)
                                .tip(tr("Für heute schließen"))
                                .on_click(|_, _, cx| dismiss(cx))
                                .child(icon("x", iconsize::XS, t.secondary)),
                        ))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(tr(shown.title)))
                        .child(div().text_size(px(font::CALLOUT)).text_color(t.secondary).child(fill(shown.text, cx))),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::S))
                .when(showable, |d| d.child(Button::new("tip-show").primary().small().label(tr("Zeig's mir")).on_click(move |_, window, cx| show_me(shown, window, cx))))
                .child(Button::new("tip-next").small().label(tr("Nächster Tipp")).on_click(|_, _, cx| show_next(cx)))
                .child(div().flex_1())
                .child(link_button("tip-off", tr("Keine Tipps mehr"), t).text_size(px(font::CAPTION)).text_color(t.secondary).on_click(|_, _, cx| disable(cx))),
        );
    Some(
        div()
            .absolute()
            .right(px(right))
            .child(body)
            .with_animation(SharedString::from(format!("tip-in-{}", shown.id)), Animation::new(travel).with_easing(ease_out_quint()), move |d, p| d.bottom(px(bottom + space::M * p)).opacity(p))
            .into_any_element(),
    )
}
