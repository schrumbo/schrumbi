use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::*;

use super::theme::{Theme, font};

/// Narrowest a tab gets before the strip scrolls.
pub const TAB_MIN: f32 = 116.;
/// Narrowest the shown tab gets while the row has room for it.
pub const ACTIVE_MIN: f32 = 168.;
/// Widest a tab title is drawn.
pub const TITLE_MAX: f32 = 180.;
/// A tab without its title: padding, file icon, close button and the gaps between them.
pub const TAB_CHROME: f32 = 68.;

/// How wide a document's top row and its tab strip were when last drawn, so the next frame fits them.
#[derive(Clone, Default)]
pub struct BarFit {
    bar: Rc<Cell<f32>>,
    strip: Rc<Cell<f32>>,
    scrolled: Rc<Cell<f32>>,
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

    /// Whether the strip changed width since the last call, so the shown tab is brought back into view.
    pub fn strip_moved(&self) -> bool {
        let now = self.strip.get();
        now > 0. && self.scrolled.replace(now) != now
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
    TAB_CHROME + text_width(title, font::CALLOUT, FontWeight::NORMAL, window).min(TITLE_MAX)
}

/// Widths for tabs sharing `room`. Wide tabs give way first, down to `TAB_MIN`;
/// the shown tab keeps up to `ACTIVE_MIN`. What still does not fit scrolls.
pub fn tab_widths(natural: &[f32], active: Option<usize>, room: f32) -> Vec<f32> {
    let floor = |i: usize| natural[i].min(if Some(i) == active { ACTIVE_MIN.min(room.max(0.)) } else { TAB_MIN });
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

/// Space left for a document's actions: `comfortable` keeps the shown tab wide, `tight` only readable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub comfortable: f32,
    pub tight: f32,
}

impl Room {
    /// Room before the row was measured: everything fits.
    pub const OPEN: Room = Room { comfortable: f32::INFINITY, tight: f32::INFINITY };
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
    use super::{ACTIVE_MIN, Room, TAB_MIN, fold, fold_room, row, tab_widths};

    #[test]
    fn tabs_keep_natural_width_when_they_fit() {
        assert_eq!(tab_widths(&[120., 200.], Some(0), 400.), vec![120., 200.]);
    }

    #[test]
    fn wide_tabs_give_way_first() {
        let w = tab_widths(&[120., 240., 240.], Some(0), 500.);
        assert_eq!(w[0], 120.);
        assert!(w[1] == w[2] && w[1] < 240. && w[1] >= TAB_MIN);
        assert!(w.iter().sum::<f32>() <= 500.);
    }

    #[test]
    fn shown_tab_stays_readable_while_others_hit_the_floor() {
        let w = tab_widths(&[240., 240., 240., 240.], Some(2), 300.);
        assert_eq!(w[2], ACTIVE_MIN);
        assert!(w.iter().enumerate().all(|(i, x)| i == 2 || *x == TAB_MIN));
    }

    #[test]
    fn shown_tab_never_wider_than_the_strip() {
        let w = tab_widths(&[240., 240.], Some(1), 90.);
        assert_eq!(w[1], 90.);
        assert_eq!(w[0], TAB_MIN);
    }

    #[test]
    fn short_titles_are_never_widened() {
        let w = tab_widths(&[80., 240., 240.], Some(1), 300.);
        assert_eq!(w[0], 80.);
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
