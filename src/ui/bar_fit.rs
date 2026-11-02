use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::*;

use super::theme::{Theme, control, font, iconsize, space};

/// Narrowest a tab gets before the strip scrolls.
pub const TAB_MIN: f32 = 100.;
/// Narrowest the shown tab gets while the row has room for it.
pub const ACTIVE_MIN: f32 = 168.;
/// Widest a tab title is drawn.
pub const TITLE_MAX: f32 = 180.;
/// A tab without title and close button: padding, file icon and the gap after it.
pub const TAB_BASE: f32 = space::M + iconsize::M + space::S + space::S;
/// The close button and the gap before it; a squeezed tab shows it only on hover, over its title.
pub const TAB_CLOSE: f32 = iconsize::M + space::S;
/// Text size of a small button's label.
pub const BUTTON_TEXT: f32 = 14.;
/// Border on both sides of an outlined button.
pub const BUTTON_BORDER: f32 = 2.;
/// Width of an icon-only button.
pub const ICON_BUTTON: f32 = control::S;
/// Narrowest row that still shows the list of all tabs; below, the row keeps its room for the shown tab.
pub const LIST_MIN_BAR: f32 = 360.;

/// How wide a document's top row and its tab strip were when last drawn, so the next frame fits them.
#[derive(Clone, Default)]
pub struct BarFit {
    bar: Rc<Cell<f32>>,
    strip: Rc<Cell<f32>>,
    revealed: Rc<RefCell<(Option<PathBuf>, f32)>>,
    listed: Rc<Cell<bool>>,
}

thread_local! {
    static SLOTS: [BarFit; 2] = Default::default();
    static TAB_TEXT: RefCell<std::collections::HashMap<String, f32>> = RefCell::default();
}

/// The widths of the main document pane's row, or of the split pane's, shared by whatever document is shown there.
pub fn slot(split: bool) -> BarFit {
    SLOTS.with(|s| s[usize::from(split)].clone())
}

impl BarFit {
    /// The row's width, once it was drawn.
    pub fn bar(&self) -> Option<f32> {
        Some(self.bar.get()).filter(|w| *w > 0.)
    }

    /// The tab strip's width, once it was drawn.
    pub fn strip(&self) -> Option<f32> {
        Some(self.strip.get()).filter(|w| *w > 0.)
    }

    /// Whether `shown` or the strip width changed since the last call, so the shown tab is brought back into view.
    pub fn needs_reveal(&self, shown: &std::path::Path) -> bool {
        let strip = self.strip.get();
        let mut last = self.revealed.borrow_mut();
        if last.0.as_deref() == Some(shown) && last.1 == strip {
            return false;
        }
        *last = (Some(shown.to_path_buf()), strip);
        true
    }

    /// Whether the list of all tabs was shown last time; it takes room from the strip.
    pub fn listed(&self) -> bool {
        self.listed.get()
    }

    /// Records whether the list of all tabs is shown.
    pub fn set_listed(&self, on: bool) {
        self.listed.set(on);
    }

    /// Fills its parent and records the row's width.
    pub fn measure_bar(&self) -> impl IntoElement + use<> {
        measurer(self.bar.clone())
    }

    /// Fills its parent and records the strip's width.
    pub fn measure_strip(&self) -> impl IntoElement + use<> {
        measurer(self.strip.clone())
    }
}

fn measurer(cell: Rc<Cell<f32>>) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            let width = f32::from(bounds.size.width).floor();
            if cell.get() != width {
                cell.set(width);
                window.on_next_frame(|window, _| window.refresh());
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// How wide `text` is drawn in the interface font.
pub fn text_width(text: &str, size: f32, weight: FontWeight, window: &Window) -> f32 {
    if text.is_empty() {
        return 0.;
    }
    let font = Font { weight, ..gpui_kit::font(Theme::font_family()) };
    let run = TextRun { len: text.len(), font, color: Hsla::default(), background_color: None, underline: None, strikethrough: None };
    f32::from(window.text_system().shape_line(text.to_string().into(), px(size), &[run], None).width).ceil()
}

/// How wide a tab with this title is when nothing squeezes it.
pub fn tab_natural(title: &str, window: &Window) -> f32 {
    let text = TAB_TEXT.with(|c| c.borrow().get(title).copied()).unwrap_or_else(|| {
        let width = text_width(title, font::CALLOUT, FontWeight::NORMAL, window);
        TAB_TEXT.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 4096 {
                c.clear();
            }
            c.insert(title.to_string(), width);
        });
        width
    });
    TAB_BASE + TAB_CLOSE + text.min(TITLE_MAX)
}

/// How wide a small outlined button with an icon and this label is.
pub fn button_width(label: &str, window: &Window) -> f32 {
    2. * space::S + iconsize::S + space::XS + text_width(label, BUTTON_TEXT, FontWeight::NORMAL, window) + BUTTON_BORDER
}

/// Widths for tabs sharing a strip of width `strip` with `gap` between them. Wide tabs give way first,
/// down to `TAB_MIN`; the shown tab keeps up to `ACTIVE_MIN`, never more than the strip less `margin`
/// toward each side with more tabs. What still does not fit scrolls.
pub fn tab_widths(natural: &[f32], active: Option<usize>, strip: f32, gap: f32, margin: f32) -> Vec<f32> {
    let room = strip - gap * natural.len().saturating_sub(1) as f32;
    let margins = active.map_or(0., |a| margin * (f32::from(u8::from(a > 0)) + f32::from(u8::from(a + 1 < natural.len()))));
    let shown_max = (strip - margins).max(strip.min(TAB_MIN)).max(0.);
    let floor = |i: usize| natural[i].min(if Some(i) == active { ACTIVE_MIN.min(shown_max) } else { TAB_MIN });
    let total: f32 = natural.iter().sum();
    if total <= room {
        return natural.to_vec();
    }
    let floors: Vec<f32> = (0..natural.len()).map(floor).collect();
    if floors.iter().sum::<f32>() >= room {
        return floors.iter().map(|w| w.floor()).collect();
    }
    let fill = |level: f32| -> f32 { natural.iter().zip(&floors).map(|(n, f)| level.clamp(*f, *n)).sum() };
    let (mut low, mut high) = (0., natural.iter().copied().fold(0., f32::max));
    for _ in 0..24 {
        let mid = (low + high) / 2.;
        if fill(mid) > room { high = mid } else { low = mid }
    }
    natural.iter().zip(&floors).map(|(n, f)| low.clamp(*f, *n).floor()).collect()
}

/// Scroll offset (zero or negative) that shows tab `shown` whole, `margin` away from an edge that has more tabs behind it.
pub fn reveal(widths: &[f32], gap: f32, shown: usize, strip: f32, offset: f32, margin: f32) -> f32 {
    let total = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
    let max = (total - strip).max(0.);
    let left = widths[..shown].iter().map(|w| w + gap).sum::<f32>();
    let right = left + widths[shown];
    let sides = f32::from(u8::from(shown > 0)) + f32::from(u8::from(shown + 1 < widths.len()));
    let margin = if sides > 0. { margin.min(((strip - widths[shown]) / sides).max(0.)) } else { 0. };
    let before = if shown > 0 { margin } else { 0. };
    let after = if shown + 1 < widths.len() { margin } else { 0. };
    let mut offset = offset.clamp(-max, 0.);
    if right + offset > strip - after {
        offset = strip - after - right;
    }
    if left + offset < before {
        offset = before - left;
    }
    offset.clamp(-max, 0.)
}

/// Space left for a document's actions: `comfortable` keeps the shown tab wide, `tight` only readable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub comfortable: f32,
    pub tight: f32,
}

impl Room {
    /// Room before the row was measured: everything fits.
    pub const OPEN: Room = Room { comfortable: f32::INFINITY, tight: f32::INFINITY };

    /// The same room on both counts, for a row without tabs.
    pub fn exact(width: f32) -> Room {
        Room { comfortable: width, tight: width }
    }
}

/// Which of the ever more compact action rows to draw. The shown tab stays wide while
/// steps before `keep` are folded; steps from `keep` on are taken only when even a narrow tab leaves no room.
pub fn fold_room(room: Room, widths: &[f32], keep: usize) -> usize {
    fold(room.comfortable, widths).min(fold(room.tight, widths).max(keep))
}

/// The first of the ever more compact widths that fits in `room`, else the most compact.
pub fn fold(room: f32, widths: &[f32]) -> usize {
    widths.iter().position(|w| *w <= room).unwrap_or_else(|| widths.len().saturating_sub(1))
}

/// Width of a row of items with `gap` between them.
pub fn row(items: &[f32], gap: f32) -> f32 {
    let shown: Vec<f32> = items.iter().copied().filter(|w| *w > 0.).collect();
    shown.iter().sum::<f32>() + gap * shown.len().saturating_sub(1) as f32
}

#[cfg(test)]
mod tests {
    use super::{ACTIVE_MIN, Room, TAB_MIN, fold, fold_room, reveal, row, tab_widths};

    #[test]
    fn tabs_keep_natural_width_when_they_fit() {
        assert_eq!(tab_widths(&[120., 200.], Some(0), 400., 2., 24.), vec![120., 200.]);
    }

    #[test]
    fn wide_tabs_give_way_first() {
        let w = tab_widths(&[120., 240., 240.], Some(0), 504., 2., 24.);
        assert_eq!(w[0], 120.);
        assert!(w[1] == w[2] && w[1] < 240. && w[1] >= TAB_MIN);
        assert!(w.iter().sum::<f32>() <= 500.);
    }

    #[test]
    fn shown_tab_stays_readable_while_others_hit_the_floor() {
        let w = tab_widths(&[240., 240., 240., 240.], Some(2), 300., 2., 24.);
        assert_eq!(w[2], ACTIVE_MIN);
        assert!(w.iter().enumerate().all(|(i, x)| i == 2 || *x == TAB_MIN));
    }

    #[test]
    fn shown_tab_never_wider_than_the_strip() {
        let w = tab_widths(&[240., 240.], Some(1), 150., 2., 24.);
        assert_eq!(w[1], 126.);
        assert_eq!(w[0], TAB_MIN);
    }

    #[test]
    fn shown_tab_keeps_the_strip_with_many_tabs() {
        let w = tab_widths(&[240.; 80], Some(40), 200., 2., 24.);
        assert_eq!(w[40], 152.);
        let w = tab_widths(&[240.; 80], Some(40), 120., 2., 24.);
        assert_eq!(w[40], TAB_MIN);
        let w = tab_widths(&[240.; 3], Some(1), 60., 2., 24.);
        assert_eq!(w[1], 60.);
    }

    #[test]
    fn short_titles_are_never_widened() {
        let w = tab_widths(&[80., 240., 240.], Some(1), 300., 2., 24.);
        assert_eq!(w[0], 80.);
    }

    #[test]
    fn reveal_keeps_a_margin_where_more_tabs_follow() {
        let w = [100.; 5];
        assert_eq!(reveal(&w, 0., 0, 250., -200., 24.), 0.);
        assert_eq!(reveal(&w, 0., 2, 250., 0., 24.), -74.);
        assert_eq!(reveal(&w, 0., 4, 250., 0., 24.), -250.);
        assert_eq!(reveal(&w, 0., 2, 250., -120., 24.), -120.);
        assert_eq!(reveal(&w, 0., 1, 1000., -50., 24.), 0.);
        assert_eq!(reveal(&w, 0., 2, 100., 0., 24.), -200.);
        assert_eq!(reveal(&w, 0., 2, 120., 0., 24.), -190.);
    }

    #[test]
    fn fold_picks_first_that_fits_and_never_runs_out() {
        assert_eq!(fold(300., &[400., 250., 100.]), 1);
        assert_eq!(fold(500., &[400., 250., 100.]), 0);
        assert_eq!(fold(10., &[400., 250., 100.]), 2);
        assert_eq!(fold(f32::INFINITY, &[400.]), 0);
    }

    #[test]
    fn tab_comfort_never_costs_the_kept_steps() {
        let widths = [400., 250., 150., 100., 24.];
        assert_eq!(fold_room(Room { comfortable: 50., tight: 120. }, &widths, 3), 3);
        assert_eq!(fold_room(Room { comfortable: 50., tight: 30. }, &widths, 3), 4);
        assert_eq!(fold_room(Room { comfortable: 260., tight: 500. }, &widths, 3), 1);
        assert_eq!(fold_room(Room::OPEN, &widths, 3), 0);
    }

    #[test]
    fn row_skips_missing_items() {
        assert_eq!(row(&[24., 0., 24.], 8.), 56.);
        assert_eq!(row(&[], 8.), 0.);
    }
}
