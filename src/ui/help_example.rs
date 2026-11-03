use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::markdown_view::{self, Folds, LinkFocus, ParsedNote};
use super::motion::{self, Soft};
use super::selectable::selectable;
use super::theme::{Theme, font, iconsize, radius, space, syntax_palette};
use super::widgets::*;
use crate::core::i18n::tr;
use crate::core::markdown::TokenKind;

/// How long the copy button shows its check mark.
pub const COPIED_FOR: Duration = Duration::from_millis(1400);

/// Height of the bar above an example.
const BAR: f32 = 36.;

/// Called with whether the source should show.
pub type OnShow = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// Called when the source should be copied.
pub type OnCopy = Rc<dyn Fn(&mut Window, &mut App)>;

/// A Markdown example: the rendered result, or its source with one click; copying works in both.
#[derive(IntoElement)]
pub struct Example {
    id: usize,
    title: String,
    source: String,
    note: Rc<ParsedNote>,
    folds: Folds,
    base: Option<std::path::PathBuf>,
    show_source: bool,
    copied: bool,
    animate: bool,
    on_show: OnShow,
    on_copy: OnCopy,
}

/// Builds an example block; `id` must be unique on the page, `animate` fades the body in after a switch.
#[allow(clippy::too_many_arguments)]
pub fn example(id: usize, title: String, source: String, note: Rc<ParsedNote>, folds: Folds, base: Option<&Path>, show_source: bool, copied: bool, animate: bool, on_show: OnShow, on_copy: OnCopy) -> Example {
    Example { id, title, source, note, folds, base: base.map(Path::to_path_buf), show_source, copied, animate, on_show, on_copy }
}

fn segment(id: (&'static str, usize), label: String, on: bool, t: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(BAR - space::S * 2. - 2.))
        .px(px(space::S + space::XXS))
        .flex()
        .items_center()
        .rounded(px(radius::SMALL))
        .text_size(px(font::CAPTION))
        .cursor_pointer()
        .key_nav()
        .when(on, |d| d.bg(t.content).text_color(t.primary).font_weight(FontWeight::MEDIUM).shadow(vec![BoxShadow { color: t.ink.opacity(0.12), offset: point(px(0.), px(1.)), blur_radius: px(2.), spread_radius: px(0.), inset: false }]))
        .when(!on, |d| d.text_color(t.secondary).soft(t.ink.opacity(0.06), radius::SMALL))
        .child(label)
}

fn highlighted(source: &str, t: &Theme) -> (String, Vec<(std::ops::Range<usize>, HighlightStyle)>) {
    let colors = syntax_palette(t.kind);
    let mut text = String::new();
    let mut marks = Vec::new();
    for (s, kind) in crate::core::markdown::highlight(source, Some("markdown")) {
        let start = text.len();
        text.push_str(&s);
        let color = match kind {
            TokenKind::Plain => None,
            TokenKind::Keyword => Some(colors.keyword),
            TokenKind::String => Some(colors.string),
            TokenKind::Comment => Some(colors.comment),
            TokenKind::Number => Some(colors.number),
            TokenKind::Type => Some(colors.kind),
            TokenKind::Attribute => Some(colors.function),
        };
        if let Some(c) = color {
            marks.push((start..text.len(), HighlightStyle { color: Some(rgb(c).into()), ..Default::default() }));
        }
    }
    (text, marks)
}

fn rendered(note: &ParsedNote, folds: &Folds, base: Option<&Path>, t: &Theme) -> Div {
    let focus = LinkFocus { index: None, note: None, list: None, text_click: None, edit_item: None, folds: folds.clone() };
    let hidden = note.hidden_blocks(folds);
    div()
        .flex()
        .flex_col()
        .gap(px(space::M))
        .children((0..hidden.len()).filter(|&i| !hidden[i]).map(|i| markdown_view::render_block(note, i, base, None, None, Some(&focus), t)))
        .children(markdown_view::render_footnotes(note, None, t))
}

impl RenderOnce for Example {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let id = self.id;
        let (show, hide) = (self.on_show.clone(), self.on_show);
        let copy = self.on_copy;
        let toggle = div()
            .flex()
            .items_center()
            .gap(px(space::XXS))
            .p(px(space::XXS))
            .rounded(px(radius::INNER))
            .bg(t.ink.opacity(0.06))
            .child(segment(("help-ex-result", id), tr("Ergebnis"), !self.show_source, &t).on_click(move |_, window, cx| hide(false, window, cx)))
            .child(segment(("help-ex-source", id), tr("Markdown"), self.show_source, &t).on_click(move |_, window, cx| show(true, window, cx)));
        let copied = self.copied;
        let copy_button = div()
            .id(("help-ex-copy", id))
            .size(px(BAR - space::S * 2.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(radius::SMALL))
            .cursor_pointer()
            .key_nav()
            .soft(t.ink.opacity(0.06), radius::SMALL)
            .tooltip(tip(if copied { tr("Kopiert") } else { tr("Markdown kopieren") }))
            .on_click(move |_, window, cx| copy(window, cx))
            .child(icon(if copied { "check" } else { "copy" }, iconsize::S, if copied { t.green } else { t.secondary }));
        let bar = div()
            .h(px(BAR))
            .pl(px(space::M))
            .pr(px(space::S))
            .flex()
            .items_center()
            .gap(px(space::S))
            .border_b_1()
            .border_color(t.separator)
            .child(div().flex_1().min_w_0().truncate().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(self.title))
            .child(toggle)
            .child(copy_button);
        let body = if self.show_source {
            let (text, marks) = highlighted(&self.source, &t);
            div()
                .id(("help-ex-code", id))
                .w_full()
                .overflow_x_scroll()
                .px(px(space::L))
                .py(px(space::M))
                .bg(t.secondary_bg)
                .font_family(Theme::mono_family())
                .text_size(px(font::CALLOUT))
                .line_height(relative(1.5))
                .child(selectable(("help-ex-text", id), text, marks, 9_000_000 + id as u64, t.accent.opacity(0.3)))
                .into_any_element()
        } else {
            div()
                .id(("help-ex-view", id))
                .w_full()
                .px(px(space::L))
                .py(px(space::M))
                .child(rendered(&self.note, &self.folds, self.base.as_deref(), &t))
                .into_any_element()
        };
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(t.separator)
            .bg(t.content)
            .overflow_hidden()
            .child(bar)
            .map(|d| {
                if self.animate {
                    d.child(div().id(("help-ex-body", id)).child(body).with_animation(("help-ex-fade", id * 2 + usize::from(self.show_source)), Animation::new(motion::timed(motion::FAST, cx)).with_easing(ease_out_quint()), |d, delta| d.opacity(0.4 + 0.6 * delta)).into_any_element())
                } else {
                    d.child(body)
                }
            })
    }
}
