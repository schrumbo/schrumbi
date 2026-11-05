use std::path::PathBuf;

use gpui_kit::base::actions::Confirm;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::quick::Dismissed;
use super::state::{self, act};
use super::theme::{Theme, font, iconsize, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};

const FORBIDDEN: &str = "\\/:*?\"<>|";

/// Asks for a new name in a small sheet, for when the explorer has no room to rename in its row.
pub fn open(path: PathBuf, window: &mut Window, cx: &mut App) {
    super::sheets::show_sheet(420., window, cx, move |window, cx| RenameSheet::new(path, window, cx));
}

struct RenameSheet {
    path: PathBuf,
    ext: String,
    input: Entity<InputState>,
    error: Option<String>,
    focused: bool,
}

impl EventEmitter<Dismissed> for RenameSheet {}

impl RenameSheet {
    fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let (stem, ext) = if path.is_dir() { (name, String::new()) } else { (crate::core::naming::stem(&name).to_string(), crate::core::naming::extension(&name).to_string()) };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(stem));
        Self { path, ext, input, error: None, focused: false }
    }

    fn name(&self, cx: &App) -> String {
        let stem = self.input.read(cx).value().trim().to_string();
        if self.ext.is_empty() { stem } else { format!("{stem}.{}", self.ext) }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let stem = self.input.read(cx).value().trim().to_string();
        let name = self.name(cx);
        let current = self.path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name == current {
            cx.emit(Dismissed);
            return;
        }
        let target = self.path.with_file_name(&name);
        self.error = if stem.is_empty() {
            Some(tr("Gib einen Namen ein."))
        } else if stem.contains(|c| FORBIDDEN.contains(c)) || stem.ends_with('.') {
            Some(trf("Diese Zeichen gehen nicht: %@", &[&"\\ / : * ? \" < > |".to_string()]))
        } else if target.exists() && !crate::core::library::same_file(&self.path, &target) {
            Some(tr("Diesen Namen gibt es hier schon. Wähle einen anderen."))
        } else {
            None
        };
        if self.error.is_some() {
            cx.notify();
            return;
        }
        let path = self.path.clone();
        act(cx, |l| l.rename(&path, &name));
        state::reindex(cx);
        cx.emit(Dismissed);
    }
}

impl Render for RenameSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !std::mem::replace(&mut self.focused, true) {
            self.input.update(cx, |s, cx| {
                let end = s.value().len();
                s.focus(window, cx);
                s.set_selected_range(0..end, cx);
            });
        }
        let t = Theme::get(cx);
        let field = Input::new(&self.input).when(!self.ext.is_empty(), |i| i.suffix(div().text_color(t.secondary).child(format!(".{}", self.ext))));
        let status = self.error.clone().map(|e| div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.red).child(icon("triangle-alert", iconsize::XS, t.red)).child(div().min_w_0().child(e)));
        div()
            .w_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|s, _: &Confirm, window, cx| if super::sheets::typing(window) { s.confirm(cx) } else { cx.propagate() }))
            .child(div().px(px(space::XL)).pt(px(space::L)).child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(tr("Umbenennen"))))
            .child(div().px(px(space::XL)).py(px(space::L)).flex().flex_col().gap(px(space::S)).child(field).children(status))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(space::S))
                    .p(px(space::L))
                    .border_t_1()
                    .border_color(t.separator)
                    .child(Button::new("rename-cancel").label(tr("Abbrechen")).on_click(cx.listener(|_, _, _, cx| cx.emit(Dismissed))))
                    .child(Button::new("rename-confirm").primary().label(tr("Umbenennen")).on_click(cx.listener(|s, _, _, cx| s.confirm(cx)))),
            )
    }
}
