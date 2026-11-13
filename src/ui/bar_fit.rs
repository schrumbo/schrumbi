use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::*;

use super::theme::{Theme, control, font, iconsize, space, tab};

/// A tab without title and close button: padding, file icon and the gap after it.
pub const TAB_BASE: f32 = space::M + iconsize::M + space::S + space::S;
/// The close button and the gap before it; a squeezed tab shows it only on hover, over its title.
pub const TAB_CLOSE: f32 = iconsize::M + space::S;
/// What a title gets beyond its measured width, so the face it is drawn in never cuts it short.
pub const TAB_TEXT_SLACK: f32 = 4.;
/// Width of an icon-only button.
pub const ICON_BUTTON: f32 = control::S;

/// How wide a document's top row and its tab strip were when last drawn, so the next frame fits them.
#[derive(Clone, Default)]
pub struct BarFit {
    bar: Rc<Cell<f32>>,
    strip: Rc<Cell<f32>>,
    revealed: Rc<RefCell<(Option<PathBuf>, f32)>>,
    decided: Rc<Cell<Option<bool>>>,
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

    /// Records whether the actions fitting just now left room for the list of all tabs.
    pub fn decide(&self, listed: bool) {
        self.decided.set(Some(listed));
    }

    /// The list of all tabs as the actions' fitting just decided it, once.
    pub fn decided(&self) -> Option<bool> {
        self.decided.take()
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
    shaped_width(text, size, weight, FontStyle::Normal, window)
}

fn shaped_width(text: &str, size: f32, weight: FontWeight, style: FontStyle, window: &Window) -> f32 {
    if text.is_empty() {
        return 0.;
    }
    let text: String = text.chars().map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c }).collect();
    let font = Font { weight, style, ..gpui_kit::font(Theme::font_family()) };
    let run = TextRun { len: text.len(), font, color: Hsla::default(), background_color: None, underline: None, strikethrough: None };
    f32::from(window.text_system().shape_line(text.into(), px(size), &[run], None).width).ceil()
}

/// How wide a tab with this title is when nothing squeezes it.
pub fn tab_natural(title: &str, window: &Window) -> f32 {
    let text = TAB_TEXT.with(|c| c.borrow().get(title).copied()).unwrap_or_else(|| {
        let width = text_width(title, font::CALLOUT, FontWeight::NORMAL, window).max(shaped_width(title, font::CALLOUT, FontWeight::NORMAL, FontStyle::Italic, window));
        TAB_TEXT.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 4096 {
                c.clear();
            }
            c.insert(title.to_string(), width);
        });
        width
    });
    TAB_BASE + TAB_CLOSE + (text + TAB_TEXT_SLACK).min(tab::TITLE_MAX)
}

/// How wide a small outlined button with an icon and this label is.
pub fn button_width(label: &str, window: &Window) -> f32 {
    2. * space::S + iconsize::S + space::XS + text_width(label, font::BUTTON, FontWeight::NORMAL, window) + space::XXS
}

/// Widths for tabs sharing a strip of width `strip` with `gap` between them. Wide tabs give way first,
/// down to `TAB_MIN`; the shown tab keeps up to `ACTIVE_MIN`, never more than the strip less `margin`
/// toward each side with more tabs. What still does not fit scrolls.
pub fn tab_widths(natural: &[f32], active: Option<usize>, strip: f32, gap: f32, margin: f32) -> Vec<f32> {
    let room = strip - gap * natural.len().saturating_sub(1) as f32;
    let margins = active.map_or(0., |a| margin * (f32::from(u8::from(a > 0)) + f32::from(u8::from(a + 1 < natural.len()))));
    let shown_max = (strip - margins).max(strip.min(tab::MIN)).max(0.);
    let floor = |i: usize| natural[i].min(if Some(i) == active { tab::ACTIVE_MIN.min(shown_max) } else { tab::MIN });
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

/// Whether tabs of these natural widths do not all fit in a strip of width `strip`.
pub fn overflows(natural: &[f32], active: Option<usize>, strip: f32) -> bool {
    let gaps = space::XXS * natural.len().saturating_sub(1) as f32;
    tab_widths(natural, active, strip, space::XXS, space::XL).iter().sum::<f32>() + gaps > strip + 0.5
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
/// The step just before `keep` is the last one a wide tab may cost.
pub fn fold_room(room: Room, widths: &[f32], keep: usize) -> usize {
    fold(room.comfortable, widths).min(fold(room.tight, widths).max(keep.saturating_sub(1)))
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

/// How the toolbar's row for the open note divides its room.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crumbs {
    pub folders: bool,
    pub name: f32,
    pub pill: f32,
}

/// Divides `room` between the folders, the note's name and the deadline: the folders collapse into one "…"
/// before anything shrinks, then the deadline gives way down to `pill_min` and vanishes below it, and the name last, down to `name_min`.
pub fn crumb_widths(room: f32, folders: f32, collapsed: f32, name: f32, pill: f32, pill_min: f32, name_min: f32) -> Crumbs {
    if folders + name + pill <= room {
        return Crumbs { folders: true, name, pill };
    }
    let rest = (room - collapsed).max(0.);
    if name + pill <= rest {
        return Crumbs { folders: false, name, pill };
    }
    let pill_min = pill.min(pill_min);
    let name_w = (rest - pill_min).max(name_min.min(rest)).min(name);
    let pill_w = (rest - name_w).clamp(0., pill);
    if pill_w < pill_min {
        return Crumbs { folders: false, name: rest.min(name), pill: 0. };
    }
    Crumbs { folders: false, name: name_w, pill: pill_w }
}

#[cfg(test)]
mod tests {
    use super::{Crumbs, Room, crumb_widths, fold, fold_room, overflows, reveal, row, tab_widths};
    use crate::ui::theme::tab::{ACTIVE_MIN, MIN as TAB_MIN};

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
    fn file_viewer_keeps_its_labeled_button_for_a_narrow_tab() {
        let widths = [152., 56., 24.];
        assert_eq!(fold_room(Room { comfortable: 20., tight: 88. }, &widths, 2), 1);
    }

    #[test]
    fn tabs_overflow_only_when_they_do_not_fit() {
        assert!(!overflows(&[120., 120.], Some(0), 300.));
        assert!(overflows(&[120., 120.], Some(0), 200.));
        assert!(!overflows(&[], None, 0.));
    }

    #[test]
    fn everything_stays_while_it_fits() {
        assert_eq!(crumb_widths(500., 100., 24., 200., 100., 40., 64.), Crumbs { folders: true, name: 200., pill: 100. });
    }

    #[test]
    fn folders_collapse_before_the_name_or_the_deadline_shrink() {
        assert_eq!(crumb_widths(350., 100., 24., 200., 100., 40., 64.), Crumbs { folders: false, name: 200., pill: 100. });
    }

    #[test]
    fn the_deadline_gives_way_before_the_name_and_keeps_its_minimum() {
        assert_eq!(crumb_widths(260., 100., 24., 200., 100., 40., 64.), Crumbs { folders: false, name: 196., pill: 40. });
        assert_eq!(crumb_widths(200., 100., 24., 200., 100., 40., 64.), Crumbs { folders: false, name: 136., pill: 40. });
    }

    #[test]
    fn the_name_shrinks_last_and_the_parts_never_exceed_the_room() {
        for room in [0., 30., 80., 120., 200.] {
            let c = crumb_widths(room, 100., 24., 200., 100., 40., 64.);
            assert!(!c.folders);
            assert!(c.name + c.pill + 24. <= room.max(24.) + 0.01, "{room}: {c:?}");
        }
        assert_eq!(crumb_widths(120., 100., 24., 200., 100., 40., 64.), Crumbs { folders: false, name: 96., pill: 0. });
    }

    #[test]
    fn the_deadline_is_either_whole_enough_or_gone_and_the_name_takes_its_width() {
        for room in (0..400).map(|r| r as f32) {
            let c = crumb_widths(room, 100., 24., 200., 100., 40., 64.);
            assert!(c.pill == 0. || c.pill >= 40., "{room}: {c:?}");
        }
        assert_eq!(crumb_widths(130., 100., 24., 200., 100., 40., 64.), Crumbs { folders: false, name: 66., pill: 40. });
        assert_eq!(crumb_widths(110., 100., 24., 200., 100., 40., 64.), Crumbs { folders: false, name: 86., pill: 0. });
    }

    #[test]
    fn no_deadline_leaves_all_of_it_to_the_name() {
        assert_eq!(crumb_widths(150., 100., 24., 200., 0., 40., 64.), Crumbs { folders: false, name: 126., pill: 0. });
    }

    #[test]
    fn row_skips_missing_items() {
        assert_eq!(row(&[24., 0., 24.], 8.), 56.);
        assert_eq!(row(&[], 8.), 0.);
    }
}
