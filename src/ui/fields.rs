use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use gpui_kit::component::date_picker::{DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

/// Text and date fields created on demand and kept by key, so forms over changing data keep focus and cursor.
#[derive(Default)]
pub struct Fields {
    map: HashMap<String, (Entity<InputState>, Subscription)>,
    dates: HashMap<String, (Entity<DatePickerState>, Subscription)>,
    used: HashSet<String>,
}

impl Fields {
    /// The field for `key`, showing `value` unless the user is typing in it; `on_change` gets every edit.
    pub fn input<V: 'static>(&mut self, key: &str, value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<V>, on_change: impl Fn(String, &mut App) + 'static) -> Entity<InputState> {
        self.used.insert(key.to_string());
        if let Some((state, _)) = self.map.get(key) {
            let state = state.clone();
            let focused = state.read(cx).focus_handle(cx).is_focused(window);
            if !focused && state.read(cx).value().as_ref() != value {
                let v = value.to_string();
                state.update(cx, |s, cx| s.set_value(v, window, cx));
            }
            return state;
        }
        let placeholder = placeholder.to_string();
        let initial = value.to_string();
        let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(initial));
        let sub = cx.subscribe(&state, move |_, s, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                on_change(s.read(cx).value().to_string(), cx);
            }
        });
        self.map.insert(key.to_string(), (state.clone(), sub));
        state
    }

    /// A date-only picker for `key`, kept in sync with `value`.
    pub fn date<V: 'static>(&mut self, key: &str, value: Option<NaiveDate>, window: &mut Window, cx: &mut Context<V>, on_change: impl Fn(NaiveDate, &mut App) + 'static) -> Entity<DatePickerState> {
        self.used.insert(key.to_string());
        if let Some((state, _)) = self.dates.get(key) {
            let state = state.clone();
            let current = state.read(cx).date_time().start().map(|d| d.date());
            if let Some(v) = value.filter(|v| Some(*v) != current) {
                state.update(cx, |s, cx| s.set_date(v, window, cx));
            }
            return state;
        }
        let format = if crate::core::i18n::lang() == crate::core::i18n::Lang::En { "%m/%d/%Y" } else { "%d.%m.%Y" };
        let state = cx.new(|cx| DatePickerState::new(window, cx).date_format(format));
        if let Some(v) = value {
            state.update(cx, |s, cx| s.set_date(v, window, cx));
        }
        let sub = cx.subscribe(&state, move |_, _, e: &DatePickerEvent, cx| {
            let DatePickerEvent::Change(v) = e;
            if let Some(d) = v.start() {
                on_change(d.date(), cx);
            }
        });
        self.dates.insert(key.to_string(), (state.clone(), sub));
        state
    }

    /// Drops fields that were not rendered since the last call.
    pub fn prune(&mut self) {
        let used = std::mem::take(&mut self.used);
        self.map.retain(|k, _| used.contains(k));
        self.dates.retain(|k, _| used.contains(k));
    }
}

/// Parses "1,3", "1.3" or "" (none).
pub fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim().replace(',', ".");
    if t.is_empty() { None } else { t.parse().ok() }
}

/// German-style number with up to `digits` decimals, trailing zeros removed.
pub fn format_number(v: f64, digits: usize) -> String {
    let s = format!("{v:.digits$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if crate::core::i18n::lang() == crate::core::i18n::Lang::De { s.replace('.', ",") } else { s }
}

/// Grade with two decimals, or percent with up to one.
pub fn format_grade(v: f64, system: crate::core::model::GradingSystem) -> String {
    use crate::core::model::GradingSystem;
    match system {
        GradingSystem::Percent => format!("{} %", format_number(v, 1)),
        _ => {
            let s = format!("{v:.2}");
            if crate::core::i18n::lang() == crate::core::i18n::Lang::De { s.replace('.', ",") } else { s }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_number, parse_number};

    #[test]
    fn parses_and_formats_numbers() {
        assert_eq!(parse_number("1,3"), Some(1.3));
        assert_eq!(parse_number(" "), None);
        assert_eq!(parse_number("x"), None);
        crate::core::i18n::init("de");
        assert_eq!(format_number(20., 1), "20");
        assert_eq!(format_number(7.5, 1), "7,5");
    }
}
