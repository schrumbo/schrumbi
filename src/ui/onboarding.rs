use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state;
use super::theme::{Theme, font, space};
use crate::core::i18n::tr;

/// The snail mark in the variant that reads on the current background.
pub fn snail(size: f32, t: &Theme) -> Img {
    img(if t.dark { "brand/snail-dark.svg" } else { "brand/snail.svg" }).w(px(size)).h(px(size * 300. / 340.)).flex_none()
}

/// Snail plus the lowercase wordmark, as in the Schrumbi logo.
pub fn logo(height: f32, t: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(height * 0.22))
        .child(snail(height * 1.13, t))
        .child(div().text_size(px(height * 0.78)).font_weight(FontWeight::EXTRA_BOLD).text_color(if t.dark { rgb(0xE8EEF4) } else { rgb(0x182535) }).child("schrumbi"))
}

/// Shown until a workspace folder is chosen.
pub fn render(error: Option<String>, t: &Theme) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(space::L))
        .p(px(space::XXL))
        .on_drop(|paths: &ExternalPaths, _, cx| {
            if let Some(folder) = paths.paths().iter().find(|p| p.is_dir()) {
                state::open_workspace(folder, cx);
            }
        })
        .child(logo(56., t))
        .child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::SEMIBOLD).child(tr("Deadlines, Dateien und Notizen fürs Studium an einem Ort")))
        .child(div().max_w(px(380.)).text_center().text_color(t.secondary).child(tr("Wähle den Ordner mit deinen Unterlagen. Die App liest nur diesen Ordner und legt dort `schrumbi.json` an.").replace('`', "")))
        .when_some(error, |d, e| d.child(div().max_w(px(480.)).text_center().text_color(t.red_text).child(e)))
        .child(Button::new("choose").primary().label(tr("Ordner wählen…")).on_click(|_, _, cx| {
            super::platform::choose_folder(cx, |path, cx| state::open_workspace(&path, cx));
        }))
}
