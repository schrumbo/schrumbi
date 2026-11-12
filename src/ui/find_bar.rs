use std::ops::Range;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, TextDecorationCollection};
use gpui_kit::component::{Disableable, Selectable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::theme::{Theme, control, font, iconsize, space};
use super::widgets::*;
use crate::core::i18n::{Lang, lang, shift_name, tr, trf};
use crate::core::shortcuts::ShortcutAction;

const CASE_LABEL: &str = "Aa";
const SEARCH_SHOWN_AFTER: Duration = Duration::from_millis(200);

actions!(schrumbi_find, [FindInNote, ReplaceInNote, CloseFind]);

/// A note's search: the bar, the matches in the note's text and which one is current.
#[derive(Default)]
pub struct FindState {
    pub bar: Option<Entity<FindBar>>,
    pub open: bool,
    pub matches: Vec<Range<usize>>,
    pub current: usize,
    /// Where the next search starts, once, instead of at the current match.
    pub anchor: Option<usize>,
    /// The text, query and case mode the matches were found for.
    pub signature: Option<(u64, String, bool)>,
    pub marks: Option<TextDecorationCollection>,
    pub mode: Option<std::mem::Discriminant<super::documents::Mode>>,
    /// The search being computed in the background, and the task doing it.
    pub pending: Option<(u64, String, bool)>,
    pub task: Option<Task<()>>,
    /// Frames left to centre the current match once the editor has scrolled it into view.
    pub center: u8,
}

/// What the bar asks of the note it belongs to.
pub enum FindEvent {
    Query,
    Step(bool),
    Replace,
    ReplaceAll,
    Close,
}

/// The find and replace row above a note's text.
pub struct FindBar {
    query: Entity<InputState>,
    replacement: Entity<InputState>,
    exact: bool,
    replacing: bool,
    can_replace: bool,
    count: usize,
    current: usize,
    searching: bool,
    searching_shown: bool,
    search_epoch: u64,
    /// The buttons that take part in Tab: case, previous, next, replace toggle, close, replace one, replace all.
    stops: [FocusHandle; 7],
    _subs: Vec<Subscription>,
}

/// The shortcut for stepping to the next or previous match, as this platform writes keys.
fn step_hint(forward: bool) -> String {
    if cfg!(target_os = "macos") {
        return if forward { "↩" } else { "⇧↩" }.to_string();
    }
    let enter = if lang() == Lang::De { "Eingabe" } else { "Enter" };
    if forward { enter.to_string() } else { format!("{}+{enter}", shift_name()) }
}

impl EventEmitter<FindEvent> for FindBar {}

impl FindBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("In dieser Notiz suchen")));
        let replacement = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Ersetzen durch")));
        let subs = vec![
            cx.subscribe_in(&query, window, |_, _, e: &InputEvent, _, cx| match e {
                InputEvent::Change => cx.emit(FindEvent::Query),
                InputEvent::PressEnter { shift, .. } => cx.emit(FindEvent::Step(!*shift)),
                _ => {}
            }),
            cx.subscribe_in(&replacement, window, |_, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::PressEnter { .. }) {
                    cx.emit(FindEvent::Replace);
                }
            }),
        ];
        let stops = std::array::from_fn(|_| cx.focus_handle());
        Self { query, replacement, exact: false, replacing: false, can_replace: false, count: 0, current: 0, searching: false, searching_shown: false, search_epoch: 0, stops, _subs: subs }
    }

    /// Moves the keyboard through the bar's own field and buttons, so Tab never lands in the note behind it.
    fn step_focus(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let has = steppable(self.count, self.searching && self.searching_shown);
        let [exact, prev, next, toggle, close, one, all] = self.stops.clone();
        let query = self.query.read(cx).focus_handle(cx);
        let replacement = self.replacement.read(cx).focus_handle(cx);
        let mut order = Vec::new();
        if self.can_replace {
            order.push(toggle);
        }
        order.extend([query, exact]);
        if has {
            order.extend([prev, next]);
        }
        order.push(close);
        if self.can_replace && self.replacing {
            order.push(replacement);
            if has {
                order.extend([one, all]);
            }
        }
        let at = order.iter().position(|f| f.contains_focused(window, cx)).unwrap_or(0);
        let to = if back { (at + order.len() - 1) % order.len() } else { (at + 1) % order.len() };
        window.focus(&order[to], cx);
    }

    pub fn text(&self, cx: &App) -> String {
        self.query.read(cx).value().to_string()
    }

    pub fn replacement(&self, cx: &App) -> String {
        self.replacement.read(cx).value().to_string()
    }

    pub fn exact(&self) -> bool {
        self.exact
    }

    /// Shows the bar's state: how many matches there are, which one is current, and whether the note can be changed.
    pub fn show(&mut self, count: usize, current: usize, can_replace: bool, cx: &mut Context<Self>) {
        if (self.count, self.current, self.can_replace, self.searching) != (count, current, can_replace, false) {
            (self.count, self.current, self.can_replace, self.searching) = (count, current, can_replace, false);
            self.replacing &= can_replace;
            cx.notify();
        }
        self.searching_shown = false;
    }

    /// Marks a search as running; only one that takes longer than a moment says so, so a quick one does not flicker.
    pub fn searching(&mut self, cx: &mut Context<Self>) {
        if self.searching {
            return;
        }
        self.searching = true;
        self.search_epoch += 1;
        let epoch = self.search_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_SHOWN_AFTER).await;
            let _ = this.update(cx, |b, cx| {
                if b.searching && b.search_epoch == epoch {
                    b.searching_shown = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Puts the caret in the search field with its text selected; `seed` replaces the text, `replacing` opens the replace row.
    pub fn open(&mut self, seed: Option<String>, replacing: bool, window: &mut Window, cx: &mut Context<Self>) {
        if replacing && self.can_replace {
            self.replacing = true;
        }
        if let Some(seed) = seed {
            self.query.update(cx, |s, cx| s.set_value(seed, window, cx));
        }
        let field = if replacing && self.can_replace && !self.query.read(cx).value().is_empty() { self.replacement.clone() } else { self.query.clone() };
        let after = field.clone();
        field.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        window.on_next_frame(move |window, cx| {
            after.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            });
        });
        cx.notify();
    }

    pub fn focus_query(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |s, cx| s.focus(window, cx));
    }

    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.query.read(cx).focus_handle(cx).contains_focused(window, cx) || self.replacement.read(cx).focus_handle(cx).contains_focused(window, cx)
    }
}

impl Render for FindBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let empty = self.query.read(cx).value().is_empty();
        let waiting = self.searching && self.searching_shown;
        let summary = if empty {
            String::new()
        } else if waiting {
            tr("Suche…")
        } else if self.count == 0 {
            tr("Keine Treffer")
        } else {
            trf("%lld von %lld", &[&(self.current + 1), &self.count])
        };
        let has = steppable(self.count, waiting);
        let (prev, next, close, exact, toggle) = (cx.entity(), cx.entity(), cx.entity(), cx.entity(), cx.entity());
        let (replace_one, replace_all) = (cx.entity(), cx.entity());
        let [exact_stop, prev_stop, next_stop, toggle_stop, close_stop, one_stop, all_stop] = self.stops.clone();
        let first = div()
            .flex()
            .items_center()
            .gap(px(space::XS))
            .when(self.can_replace, |d| {
                d.child(Button::new("find-replace-toggle").ghost().size(px(control::S)).flex_none().track_focus(&toggle_stop).accessibility_label(tr("Ersetzen ein-/ausblenden")).icon(menu_icon(if self.replacing { "chevron-down" } else { "chevron-right" })).tooltip(tr("Ersetzen ein-/ausblenden")).on_click(move |_, _, cx| {
                    toggle.update(cx, |b, cx| {
                        b.replacing = !b.replacing;
                        cx.notify();
                    })
                }))
            })
            .child(div().w(px(260.)).flex_none().child(Input::new(&self.query).small().prefix(icon("search", iconsize::XS, t.secondary))))
            .child(Button::new("find-exact").ghost().xsmall().h(px(control::S)).track_focus(&exact_stop).accessibility_label(tr("Groß-/Kleinschreibung beachten")).label(CASE_LABEL).selected(self.exact).tooltip(tr("Groß-/Kleinschreibung beachten")).on_click(move |_, _, cx| {
                exact.update(cx, |b, cx| {
                    b.exact = !b.exact;
                    cx.emit(FindEvent::Query);
                    cx.notify();
                })
            }))
            .child(div().min_w(px(76.)).flex_none().font_features(tabular()).text_size(px(font::CAPTION)).text_color(t.secondary).child(summary))
            .child(Button::new("find-prev").ghost().size(px(control::S)).flex_none().track_focus(&prev_stop).accessibility_label(tr("Vorheriger Treffer")).icon(menu_icon("chevron-up")).disabled(!has).tooltip(format!("{} ({})", tr("Vorheriger Treffer"), step_hint(false))).on_click(move |_, _, cx| prev.update(cx, |_, cx| cx.emit(FindEvent::Step(false)))))
            .child(Button::new("find-next").ghost().size(px(control::S)).flex_none().track_focus(&next_stop).accessibility_label(tr("Nächster Treffer")).icon(menu_icon("chevron-down")).disabled(!has).tooltip(format!("{} ({})", tr("Nächster Treffer"), step_hint(true))).on_click(move |_, _, cx| next.update(cx, |_, cx| cx.emit(FindEvent::Step(true)))))
            .child(div().flex_1())
            .child(Button::new("find-close").ghost().size(px(control::S)).flex_none().track_focus(&close_stop).accessibility_label(tr("Schließen")).icon(menu_icon("x")).tooltip(format!("{} (Esc)", tr("Schließen"))).on_click(move |_, _, cx| close.update(cx, |_, cx| cx.emit(FindEvent::Close))));
        div()
            .id("find-bar")
            .key_context("FindBar")
            .on_action(cx.listener(|_, _: &gpui_kit::component::input::Enter, _, _| {}))
            .capture_key_down(cx.listener(|b, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                if k.key == "tab" && !k.modifiers.platform && !k.modifiers.control && !k.modifiers.alt {
                    cx.stop_propagation();
                    b.step_focus(k.modifiers.shift, window, cx);
                }
            }))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .px(px(space::M))
            .py(px(space::XS))
            .border_b_1()
            .border_color(t.separator)
            .bg(t.content)
            .child(first)
            .when(self.replacing && self.can_replace, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::XS))
                        .child(div().w(px(control::S)).flex_none())
                        .child(div().w(px(260.)).flex_none().child(Input::new(&self.replacement).small().prefix(icon("replace", iconsize::XS, t.secondary))))
                        .child(Button::new("find-replace-one").ghost().small().h(px(control::S)).track_focus(&one_stop).label(tr("Ersetzen")).disabled(!has).on_click(move |_, _, cx| replace_one.update(cx, |_, cx| cx.emit(FindEvent::Replace))))
                        .child(Button::new("find-replace-all").ghost().small().h(px(control::S)).track_focus(&all_stop).label(tr("Alle ersetzen")).disabled(!has).on_click(move |_, _, cx| replace_all.update(cx, |_, cx| cx.emit(FindEvent::ReplaceAll)))),
                )
            })
    }
}

/// The shortcut for opening the bar as a tooltip suffix.
pub fn hint() -> String {
    ShortcutAction::FindInNote.hint()
}

fn steppable(count: usize, waiting: bool) -> bool {
    count > 0 && !waiting
}

#[cfg(test)]
mod tests {
    use super::steppable;

    #[test]
    fn buttons_are_skipped_without_matches_and_while_waiting() {
        assert!(!steppable(0, false));
        assert!(!steppable(3, true));
        assert!(steppable(3, false));
    }
}
