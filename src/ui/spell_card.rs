use std::ops::Range;

use gpui_kit::component::input::TextDecorationCollection;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::Soft;
use super::theme::{Theme, font, hover, iconsize, radius, space};
use super::widgets::*;
use crate::core::dictionary;
use crate::core::i18n::tr;
use crate::core::prefs;

actions!(schrumbi_spelling, [AddWordUnderPointer, AddWordAtCaret]);

/// The marked word the pointer was last over, for the context menu.
#[derive(Default)]
pub struct WordUnderPointer(pub Option<String>);

impl Global for WordUnderPointer {}

/// The word the context menu was opened on.
#[derive(Default)]
pub struct MenuWord(pub Option<String>);

impl Global for MenuWord {}

/// A marked word and where its card hangs: the bottom left of the word, in window coordinates.
pub struct SpellHover {
    pub range: Range<usize>,
    pub word: String,
    pub at: Point<Pixels>,
}

/// What a note knows about its misspelled words.
#[derive(Default)]
pub struct SpellState {
    pub marks: Option<(EntityId, TextDecorationCollection)>,
    pub hover: Option<SpellHover>,
    pub hide: Option<Task<()>>,
    pub show: Option<Task<()>>,
    pub pending: Option<Range<usize>>,
    pub card_hot: bool,
    pub words_seen: Option<Vec<String>>,
}

/// Remembers the marked word under the pointer, so the context menu can offer to add it.
pub fn set_pointer_word(word: Option<String>, cx: &mut App) {
    if cx.try_global::<WordUnderPointer>().is_none_or(|w| w.0 != word) {
        cx.set_global(WordUnderPointer(word));
    }
}

/// The word under the pointer, taken for the one context menu that was opened on it.
pub fn menu_opened(cx: &mut App) -> Option<String> {
    let word = cx.try_global::<WordUnderPointer>().and_then(|w| w.0.clone());
    cx.set_global(MenuWord(word.clone()));
    word
}

/// The word the last context menu was opened on, once.
pub fn take_menu_word(cx: &mut App) -> Option<String> {
    cx.try_global::<MenuWord>().and_then(|w| w.0.clone()).inspect(|_| cx.set_global(MenuWord(None)))
}

/// Adds a word to the personal dictionary; every note marks it as known from its next look.
pub fn add_word(word: &str, cx: &mut App) {
    if let Some(all) = dictionary::added(&prefs::get().spell_words, word) {
        prefs::update(|p| p.spell_words = all);
        cx.refresh_windows();
    }
}

/// Takes a word out of the personal dictionary again.
pub fn remove_word(word: &str, cx: &mut App) {
    let word = word.to_string();
    prefs::update(|p| p.spell_words = dictionary::removed(&p.spell_words, &word));
    cx.refresh_windows();
}

/// The card that hangs under a marked word.
pub fn card(hover: &SpellHover, on_add: impl Fn(&mut Window, &mut App) + 'static, on_hot: impl Fn(bool, &mut Window, &mut App) + 'static, t: &Theme) -> AnyElement {
    deferred(
        anchored().position(hover.at + point(px(0.), px(space::XXS))).snap_to_window_with_margin(px(space::S)).child(
            div()
                .id("spell-card")
                .occlude()
                .flex()
                .flex_col()
                .min_w(px(200.))
                .p(px(space::XS))
                .rounded(px(radius::CONTROL))
                .bg(t.material)
                .border_1()
                .border_color(t.separator)
                .shadow_lg()
                .on_hover(move |hot, window, cx| on_hot(*hot, window, cx))
                .child(div().px(px(space::S)).py(px(space::XXS)).text_size(px(font::CAPTION)).text_color(t.secondary).child(tr("Unbekanntes Wort")))
                .child(
                    div()
                        .id("spell-add")
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .px(px(space::S))
                        .py(px(space::XS))
                        .rounded(px(radius::SMALL))
                        .cursor_pointer()
                        .key_nav()
                        .text_size(px(font::CALLOUT))
                        .text_color(t.primary)
                        .soft(t.ink.opacity(hover::ROW), radius::SMALL)
                        .on_click(move |_, window, cx| on_add(window, cx))
                        .child(icon("plus", iconsize::S, t.accent))
                        .child(tr("Zum Wörterbuch hinzufügen")),
                ),
        ),
    )
    .with_priority(3)
    .into_any_element()
}
