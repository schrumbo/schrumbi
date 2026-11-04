use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, TextDecorationCollection};
use gpui_kit::component::{Disableable, Selectable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::theme::{Theme, control, font, iconsize, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::shortcuts::ShortcutAction;

const CASE_LABEL: &str = "Aa";

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
    _subs: Vec<Subscription>,
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
        Self { query, replacement, exact: false, replacing: false, can_replace: false, count: 0, current: 0, _subs: subs }
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
        if (self.count, self.current, self.can_replace) != (count, current, can_replace) {
            (self.count, self.current, self.can_replace) = (count, current, can_replace);
            self.replacing &= can_replace;
            cx.notify();
        }
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
        let summary = if empty {
            String::new()
        } else if self.count == 0 {
            tr("Keine Treffer")
        } else {
            trf("%lld von %lld", &[&(self.current + 1), &self.count])
        };
        let has = self.count > 0;
        let (prev, next, close, exact, toggle) = (cx.entity(), cx.entity(), cx.entity(), cx.entity(), cx.entity());
        let (replace_one, replace_all) = (cx.entity(), cx.entity());
        let first = div()
            .flex()
            .items_center()
            .gap(px(space::XS))
            .when(self.can_replace, |d| {
                d.child(Button::new("find-replace-toggle").ghost().xsmall().icon(menu_icon(if self.replacing { "chevron-down" } else { "chevron-right" })).tooltip(tr("Ersetzen ein-/ausblenden")).on_click(move |_, _, cx| {
                    toggle.update(cx, |b, cx| {
                        b.replacing = !b.replacing;
                        cx.notify();
                    })
                }))
            })
            .child(div().w(px(260.)).flex_none().child(Input::new(&self.query).small().prefix(icon("search", iconsize::XS, t.secondary))))
            .child(Button::new("find-exact").ghost().xsmall().label(CASE_LABEL).selected(self.exact).tooltip(tr("Groß-/Kleinschreibung beachten")).on_click(move |_, _, cx| {
                exact.update(cx, |b, cx| {
                    b.exact = !b.exact;
                    cx.emit(FindEvent::Query);
                    cx.notify();
                })
            }))
            .child(div().min_w(px(76.)).flex_none().font_features(tabular()).text_size(px(font::CAPTION)).text_color(t.secondary).child(summary))
            .child(Button::new("find-prev").ghost().xsmall().icon(menu_icon("chevron-up")).disabled(!has).tooltip(format!("{} ({})", tr("Vorheriger Treffer"), if cfg!(target_os = "macos") { "⇧↩" } else { "Shift+Enter" })).on_click(move |_, _, cx| prev.update(cx, |_, cx| cx.emit(FindEvent::Step(false)))))
            .child(Button::new("find-next").ghost().xsmall().icon(menu_icon("chevron-down")).disabled(!has).tooltip(format!("{} ({})", tr("Nächster Treffer"), if cfg!(target_os = "macos") { "↩" } else { "Enter" })).on_click(move |_, _, cx| next.update(cx, |_, cx| cx.emit(FindEvent::Step(true)))))
            .child(div().flex_1())
            .child(Button::new("find-close").ghost().xsmall().icon(menu_icon("x")).tooltip(format!("{} (Esc)", tr("Schließen"))).on_click(move |_, _, cx| close.update(cx, |_, cx| cx.emit(FindEvent::Close))));
        div()
            .id("find-bar")
            .key_context("FindBar")
            .on_action(cx.listener(|_, _: &gpui_kit::component::input::Enter, _, _| {}))
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
                        .child(Button::new("find-replace-one").ghost().small().label(tr("Ersetzen")).disabled(!has).on_click(move |_, _, cx| replace_one.update(cx, |_, cx| cx.emit(FindEvent::Replace))))
                        .child(Button::new("find-replace-all").ghost().small().label(tr("Alle ersetzen")).disabled(!has).on_click(move |_, _, cx| replace_all.update(cx, |_, cx| cx.emit(FindEvent::ReplaceAll)))),
                )
            })
    }
}

/// The shortcut for opening the bar as a tooltip suffix.
pub fn hint() -> String {
    ShortcutAction::FindInNote.hint()
}
