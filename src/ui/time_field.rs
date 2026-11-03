use std::rc::Rc;

use chrono::NaiveTime;
use gpui_kit::base::actions::{SelectDown, SelectUp};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::theme::{Theme, control, font, iconsize, space};
use super::widgets::{icon, menu_icon, tip};
use crate::core::clock;
use crate::core::i18n::{tr, trf};

type Commit = Rc<dyn Fn(String, &mut App)>;

const STEP: i64 = 15;
const HOUR: i64 = 60;

/// A quick choice: the stored time and what the menu says about it.
#[derive(Clone, PartialEq)]
pub struct Preset {
    pub time: String,
    pub label: String,
}

impl Preset {
    pub fn plain(time: &str) -> Self {
        Self { time: time.into(), label: time.into() }
    }
}

/// A time of day typed as `18`, `1830` or `6:30`. Up and Down step by a quarter hour, with Shift by an hour; Enter, Tab and leaving the field keep it, Escape takes the typing back.
pub struct TimeField {
    input: Entity<InputState>,
    value: String,
    presets: Vec<Preset>,
    after: Option<NaiveTime>,
    on_commit: Commit,
    _sub: Subscription,
}

impl TimeField {
    pub fn new(value: &str, presets: Vec<Preset>, window: &mut Window, cx: &mut Context<Self>, on_commit: impl Fn(String, &mut App) + 'static) -> Self {
        let initial = value.to_string();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr("HH:MM")).default_value(initial));
        let sub = cx.subscribe_in(&input, window, |f, _, e: &InputEvent, window, cx| match e {
            InputEvent::PressEnter { .. } | InputEvent::Blur => f.commit(window, cx),
            InputEvent::Focus => {
                let input = f.input.clone();
                window.defer(cx, move |window, cx| input.update(cx, |s, cx| s.select_all(window, cx)));
                cx.notify();
            }
            InputEvent::Change => cx.notify(),
        });
        Self { input, value: value.to_string(), presets, after: None, on_commit: Rc::new(on_commit), _sub: sub }
    }

    /// Only times later than `after` are taken, like the end of a class after its start.
    pub fn after(&mut self, after: Option<NaiveTime>, cx: &mut Context<Self>) {
        if after != self.after {
            self.after = after;
            cx.notify();
        }
    }

    fn read(&self, text: &str) -> Option<NaiveTime> {
        clock::parse(text).filter(|t| self.after.is_none_or(|a| *t > a))
    }

    /// Follows the stored value unless it is being typed in.
    pub fn show(&mut self, value: &str, presets: Vec<Preset>, window: &mut Window, cx: &mut Context<Self>) {
        if presets != self.presets {
            self.presets = presets;
            cx.notify();
        }
        if value == self.value {
            return;
        }
        self.value = value.to_string();
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            self.set_text(value.to_string(), window, cx);
        }
        cx.notify();
    }

    fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn set_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.text(cx) != text {
            self.input.update(cx, |s, cx| s.set_value(text, window, cx));
        }
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.read(&self.text(cx)).map(clock::format) {
            Some(v) => {
                self.set_text(v.clone(), window, cx);
                self.set(v, cx);
            }
            None => self.set_text(self.value.clone(), window, cx),
        }
        cx.notify();
    }

    fn set(&mut self, v: String, cx: &mut Context<Self>) {
        if v != self.value {
            self.value = v.clone();
            (self.on_commit)(v, cx);
        }
    }

    fn step(&mut self, minutes: i64, window: &mut Window, cx: &mut Context<Self>) {
        let from = clock::parse(&self.text(cx)).or_else(|| clock::parse(&self.value)).unwrap_or(NaiveTime::MIN);
        let to = clock::step(from, minutes);
        if self.after.is_some_and(|a| to <= a) {
            return;
        }
        let v = clock::format(to);
        self.set_text(v.clone(), window, cx);
        self.set(v, cx);
        cx.notify();
    }

    fn pick(&mut self, v: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_text(v.clone(), window, cx);
        self.set(v, cx);
        cx.notify();
    }
}

impl Render for TimeField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let text = self.text(cx);
        let typed = text.trim();
        let read = self.read(typed).map(clock::format);
        let invalid = !typed.is_empty() && read.is_none();
        let early = invalid && clock::parse(typed).is_some();
        let reading = read.filter(|r| r != typed);
        let suffix: Option<AnyElement> = if invalid {
            let why = match self.after.filter(|_| early) {
                Some(a) => trf("Muss nach %@ liegen. Esc stellt %@ wieder her.", &[&clock::format(a), &self.value]),
                None => trf("Keine Uhrzeit. Esc stellt %@ wieder her.", &[&self.value]),
            };
            Some(div().id("time-invalid").child(icon("circle-alert", iconsize::XS, t.red)).tooltip(tip(why)).into_any_element())
        } else {
            reading.map(|r| div().text_size(px(font::CAPTION)).text_color(t.secondary).child(r).into_any_element())
        };
        let me = cx.entity();
        let presets = self.presets.clone();
        let current = self.value.clone();
        div()
            .flex()
            .items_center()
            .gap(px(space::XXS))
            .child(
                div()
                    .id("time-input")
                    .capture_action(cx.listener(|f, _: &MoveUp, window, cx| {
                        cx.stop_propagation();
                        f.step(STEP, window, cx);
                    }))
                            .capture_action(cx.listener(|f, _: &MoveDown, window, cx| {
                        cx.stop_propagation();
                        f.step(-STEP, window, cx);
                    }))
                            .capture_action(cx.listener(|f, _: &SelectUp, window, cx| {
                        cx.stop_propagation();
                        f.step(HOUR, window, cx);
                    }))
                            .capture_action(cx.listener(|f, _: &SelectDown, window, cx| {
                        cx.stop_propagation();
                        f.step(-HOUR, window, cx);
                    }))
                            .capture_action(cx.listener(|f, _: &Escape, window, cx| {
                        if f.text(cx) != f.value {
                            cx.stop_propagation();
                            let v = f.value.clone();
                            f.set_text(v, window, cx);
                            cx.notify();
                        }
                    }))
                    .w(px(control::TIME))
                    .child(Input::new(&self.input).small().when(invalid, |i| i.border_color(t.red).text_color(t.red)).when_some(suffix, Input::suffix))
                    .tooltip(tip(tr("Tippe z. B. 18, 1830 oder 18:30. ↑↓ in Viertelstunden, mit ⇧ in vollen Stunden."))),
            )
            .child({
                let none = presets.is_empty();
                Button::new("time-presets").ghost().xsmall().icon(menu_icon("chevron-down")).disabled(none).tooltip(if none { tr("Keine Schnellauswahl für diese Uhrzeit") } else { tr("Schnellauswahl") }).dropdown_menu(move |mut menu, _, _| {
                    for p in &presets {
                        let (me, v) = (me.clone(), p.time.clone());
                        menu = menu.item(PopupMenuItem::new(p.label.clone()).checked(p.time == current).on_click(move |_, window, cx| {
                            let v = v.clone();
                            me.update(cx, |f, cx| f.pick(v, window, cx));
                        }));
                    }
                    menu
                })
            })
    }
}
