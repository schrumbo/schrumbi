use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::markdown_view::{self, ParsedNote};
use super::theme::{Theme, font, radius, space};
use crate::core::markdown;
use crate::core::vault::{index, links};

/// How long the pointer rests on a link before its preview appears.
const DELAY: Duration = Duration::from_millis(400);

/// The link under the pointer and, once it rested, what it shows.
#[derive(Default)]
pub struct LinkCard {
    target: Option<(String, Option<PathBuf>)>,
    position: Point<Pixels>,
    shown: Option<(PathBuf, String, Rc<ParsedNote>)>,
    wait: Option<Task<()>>,
}

impl Global for LinkCard {}

pub fn init(cx: &mut App) {
    cx.set_global(LinkCard::default());
}

fn resolve(url: &str, base: Option<&PathBuf>) -> Option<(PathBuf, links::LinkTarget)> {
    if url.contains("://") || url.starts_with("mailto:") || markdown_view::is_sample_base(base.map(PathBuf::as_path)) {
        return None;
    }
    let decoded = markdown::percent_decode(url);
    let target = links::LinkTarget::parse(&decoded);
    let from = base.map(|b| b.join("."));
    let path = base.map(|b| b.join(&target.path)).filter(|p| !target.path.is_empty() && p.is_file()).or_else(|| index::current().resolver.resolve(&target, from.as_deref()))?;
    markdown::is_markdown(&path).then_some((path, target))
}

/// Reports what is under the pointer; the card follows after a short rest.
pub fn hover(target: Option<String>, base: Option<PathBuf>, position: Point<Pixels>, cx: &mut App) {
    let wanted = target.map(|t| (t, base));
    let card = cx.global_mut::<LinkCard>();
    if card.target == wanted {
        return;
    }
    let had = card.shown.take().is_some();
    card.target = wanted.clone();
    card.position = position;
    card.wait = None;
    if had {
        cx.refresh_windows();
    }
    show_after(wanted, DELAY, cx);
}

/// Shows the card for a link right away, for the keyboard.
pub fn show_now(url: String, base: Option<PathBuf>, position: Point<Pixels>, cx: &mut App) {
    let wanted = Some((url, base));
    let card = cx.global_mut::<LinkCard>();
    card.target = wanted.clone();
    card.position = position;
    show_after(wanted, Duration::ZERO, cx);
}

pub fn hide(cx: &mut App) {
    let card = cx.global_mut::<LinkCard>();
    card.target = None;
    card.wait = None;
    if card.shown.take().is_some() {
        cx.refresh_windows();
    }
}

fn show_after(wanted: Option<(String, Option<PathBuf>)>, delay: Duration, cx: &mut App) {
    let Some((url, base)) = wanted else { return };
    let Some((path, target)) = resolve(&url, base.as_ref()) else { return };
    let source = path.clone();
    let task = cx.background_spawn(async move {
        let text = std::fs::read_to_string(&source).unwrap_or_default();
        let part = links::section(&text, &target);
        let stem = source.file_stem().unwrap_or_default().to_string_lossy().to_string();
        without_title(part.split('\n').take(24).collect::<Vec<_>>().join("\n"), &stem)
    });
    let wait = cx.spawn(async move |cx| {
        cx.background_executor().timer(delay).await;
        let text = task.await;
        cx.update(|cx| {
            let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let card = cx.global_mut::<LinkCard>();
            card.shown = Some((path, name, Rc::new(markdown_view::parse(&text))));
            cx.refresh_windows();
        });
    });
    cx.global_mut::<LinkCard>().wait = Some(wait);
}

fn without_title(text: String, name: &str) -> String {
    let rest = text.trim_start();
    let (first, after) = rest.split_once('\n').unwrap_or((rest, ""));
    if first.trim_end().strip_prefix("# ").is_some_and(|h| h.trim() == name) { after.to_string() } else { text }
}

/// The card, drawn above the page; `None` while nothing is shown.
pub fn overlay(cx: &App) -> Option<AnyElement> {
    let card = cx.global::<LinkCard>();
    let (path, name, note) = card.shown.as_ref()?;
    let t = Theme::get(cx);
    let position = card.position + point(px(0.), px(space::L));
    let body = markdown_view::render(note, path.parent(), None, &t);
    Some(
        deferred(
            anchored().position(position).snap_to_window_with_margin(px(space::S)).child(
                div()
                    .w(px(320.))
                    .max_h(px(240.))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .rounded(px(radius::CONTROL))
                    .bg(t.material)
                    .border_1()
                    .border_color(t.separator)
                    .shadow_lg()
                    .child(div().flex_none().px(px(space::M)).py(px(space::S)).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).border_b_1().border_color(t.separator).truncate().child(name.clone()))
                    .child(div().p(px(space::M)).text_size(px(font::CALLOUT)).child(body)),
            ),
        )
        .with_priority(3)
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn the_card_does_not_repeat_the_name_as_its_first_heading() {
        assert_eq!(super::without_title("# Home\nText".into(), "Home"), "Text");
        assert_eq!(super::without_title("# Anders\nText".into(), "Home"), "# Anders\nText");
        assert_eq!(super::without_title("Text".into(), "Home"), "Text");
    }

    #[test]
    fn a_link_in_a_guide_example_resolves_to_nothing() {
        let base = crate::ui::help::sample_dir();
        assert!(resolve("Beispiel", Some(&base)).is_none());
    }
}
