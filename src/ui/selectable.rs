use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::base::{TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun};
use gpui_kit::*;

/// Styled text that takes part in the window's text selection, which can span several blocks.
pub struct SelectableStyled {
    id: ElementId,
    text: SharedString,
    styled: StyledText,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    /// Things that stand inside the sentence, waiting for their room.
    slots: Vec<InlineSlot>,
    placed: Vec<Placed>,
    pills: Vec<Range<usize>>,
    pill_color: Hsla,
    on_click: Option<TextClick>,
    order: u64,
    color: Hsla,
    links: Vec<(Range<usize>, LinkAction)>,
    /// Link targets for the preview card, with the folder they are relative to.
    hovers: Vec<(Range<usize>, String)>,
    hover_base: Option<std::path::PathBuf>,
    floor: Option<Arc<AtomicU64>>,
}

/// Something drawn inside a sentence, at a byte position of the text.
pub struct InlineSlot {
    pub at: usize,
    pub kind: InlineKind,
}

pub enum InlineKind {
    /// A formula and the note scale it is drawn at.
    Math(String, f32),
    Sized(AnyElement, f32, f32),
}

struct Placed {
    range: Range<usize>,
    element: AnyElement,
    size: Size<Pixels>,
    shown: bool,
    letters: usize,
}

/// Called with the drawn text and the byte a plain click landed on.
pub type TextClick = Rc<dyn Fn(&str, usize, &mut Window, &mut App)>;

/// A letter nothing wraps inside of, drawn invisibly to keep room in the line.
const ROOM: char = 'ı';

/// Removes the room kept for pictures from copied text.
pub fn without_room(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ROOM && chars.peek() == Some(&ROOM) {
            while chars.peek() == Some(&ROOM) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What clicking a link does.
pub type LinkAction = Rc<dyn Fn(&mut App)>;

/// Wraps `text` and its highlights; `order` is its place in reading order.
pub fn selectable(id: impl Into<ElementId>, text: String, highlights: Vec<(Range<usize>, HighlightStyle)>, order: u64, color: Hsla) -> SelectableStyled {
    let text = SharedString::from(text);
    SelectableStyled { id: id.into(), styled: StyledText::new(text.clone()).with_highlights(highlights.clone()), highlights, slots: Vec::new(), placed: Vec::new(), pills: Vec::new(), pill_color: transparent_black(), on_click: None, text, order, color, links: Vec::new(), hovers: Vec::new(), hover_base: None, floor: None }
}

impl SelectableStyled {
    /// Reports plain clicks on the text.
    pub fn on_text_click(mut self, on_click: Option<TextClick>) -> Self {
        self.on_click = on_click;
        self
    }

    /// Puts rounded backgrounds behind parts of the text, as for tags.
    pub fn pills(mut self, pills: Vec<Range<usize>>, color: Hsla) -> Self {
        self.pills = pills;
        self.pill_color = color;
        self
    }

    /// Shares the line height of the parts of one paragraph, so the parts keep one rhythm: the tallest formula any of them held, at this text size.
    pub fn line_floor(mut self, floor: Option<Arc<AtomicU64>>) -> Self {
        self.floor = floor;
        self
    }

    /// Draws pictures and formulas inside the sentence.
    pub fn inline(mut self, slots: Vec<InlineSlot>) -> Self {
        self.slots = slots;
        self
    }

    /// Asks each slot for its size, keeps room for it in the text and moves all ranges behind it.
    /// Returns the line height the tallest picture needs.
    fn make_room(&mut self, window: &mut Window, cx: &mut App) -> Option<Pixels> {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = style.line_height_in_pixels(window.rem_size());
        let kept = self.floor.as_ref().map_or(0., |floor| {
            let packed = floor.load(Ordering::Relaxed);
            if f32::from_bits((packed >> 32) as u32) == f32::from(line) { f32::from_bits(packed as u32) } else { 0. }
        });
        if self.slots.is_empty() {
            return (kept > f32::from(line)).then(|| px(kept));
        }
        let system = window.text_system();
        let letter = system.advance(system.resolve_font(&style.font()), font_size, ROOM).map_or(px(4.), |a| a.width).max(px(1.));
        let source = self.text.to_string();
        let mut text = String::with_capacity(source.len());
        let mut copied = 0;
        let mut moves: Vec<(usize, usize)> = Vec::new();
        let mut shifted = 0;
        let mut known = px(0.);
        let mut rooms: Vec<(Range<usize>, Option<AnyElement>, Size<Pixels>)> = Vec::new();
        let mut tallest = px(0.);
        let mut slots = std::mem::take(&mut self.slots);
        slots.sort_by_key(|slot| slot.at);
        for slot in slots {
            let (element, size) = match slot.kind {
                InlineKind::Math(formula, scale) => match super::embeds::inline_math(&formula, scale, cx) {
                    Some((image, w, h)) => (Some(img(image).w(px(w)).h(px(h)).into_any_element()), size(px(w), px(h))),
                    None => (None, size(px(formula.len() as f32 * 7.), line)),
                },
                InlineKind::Sized(element, w, h) => (Some(element), size(px(w), px(h))),
            };
            text.push_str(&source[copied..slot.at]);
            copied = slot.at;
            let count = ((size.width / letter).ceil() as usize).max(2);
            let start = text.len();
            text.extend(std::iter::repeat_n(ROOM, count));
            shifted += text.len() - start;
            moves.push((slot.at, shifted));
            if element.is_some() {
                known = known.max(size.height);
            }
            rooms.push((start..text.len(), element, size));
            tallest = tallest.max(size.height);
        }
        text.push_str(&source[copied..]);
        let before = |count: usize| if count == 0 { 0 } else { moves[count - 1].1 };
        let moved = |i: usize| i + before(moves.partition_point(|(at, _)| *at <= i));
        let shift_end = |i: usize| i + before(moves.partition_point(|(at, _)| *at < i));
        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = std::mem::take(&mut self.highlights).into_iter().map(|(r, h)| (moved(r.start)..shift_end(r.end), h)).collect();
        for (room, ..) in &rooms {
            highlights.push((room.clone(), HighlightStyle { fade_out: Some(1.), ..Default::default() }));
        }
        self.highlights = highlights.clone();
        highlights.sort_by_key(|(r, _)| r.start);
        for r in &mut self.pills {
            *r = moved(r.start)..shift_end(r.end);
        }
        for (r, _) in &mut self.links {
            *r = moved(r.start)..shift_end(r.end);
        }
        for (r, _) in &mut self.hovers {
            *r = moved(r.start)..shift_end(r.end);
        }
        self.text = SharedString::from(text);
        self.styled = StyledText::new(self.text.clone()).with_highlights(highlights);
        let letters = ROOM.len_utf8();
        self.placed = rooms.into_iter().filter_map(|(range, element, size)| element.map(|element| Placed { letters: range.len() / letters, range, element, size, shown: false })).collect();
        let mut height = line.max(tallest + px(4.));
        if let Some(floor) = &self.floor {
            height = height.max(px(kept));
            let shared = line.max(known + px(4.)).max(px(kept));
            floor.store((u64::from(f32::from(line).to_bits()) << 32) | u64::from(f32::from(shared).to_bits()), Ordering::Relaxed);
        }
        Some(height)
    }

    /// Makes byte ranges of the text clickable; a click that selected text does not count.
    pub fn hovers(mut self, hovers: Vec<(Range<usize>, String)>, base: Option<std::path::PathBuf>) -> Self {
        self.hovers = hovers;
        self.hover_base = base;
        self
    }

    pub fn links(mut self, links: Vec<(Range<usize>, LinkAction)>) -> Self {
        self.links = links;
        self
    }
}

fn selection_quads(start: Point<Pixels>, end: Point<Pixels>, bounds: Bounds<Pixels>, line_height: Pixels) -> Vec<Bounds<Pixels>> {
    if start.y == end.y {
        return vec![Bounds::from_corners(start, point(end.x, end.y + line_height))];
    }
    let mut quads = vec![Bounds::from_corners(start, point(bounds.right(), start.y + line_height))];
    if end.y > start.y + line_height {
        quads.push(Bounds::from_corners(point(bounds.left(), start.y + line_height), point(bounds.right(), end.y)));
    }
    quads.push(Bounds::from_corners(point(bounds.left(), end.y), point(end.x, end.y + line_height)));
    quads
}

impl IntoElement for SelectableStyled {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for SelectableStyled {
    type RequestLayoutState = TextSelectionHandle;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, global_id: Option<&GlobalElementId>, inspector_id: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, TextSelectionHandle) {
        let line_height = self.make_room(window, cx);
        let text = self.text.to_string();
        let handle = window.with_element_state(global_id.expect("selectable text has an id"), |kept: Option<TextSelectionHandle>, window| {
            let handle = kept.unwrap_or_else(|| {
                let made = TextSelectionHandle::new(text, cx);
                made.refresh_window_on_change(window, cx).detach();
                made
            });
            (handle.clone(), handle)
        });
        let refinement = line_height.map(|h| TextStyleRefinement { line_height: Some(h.into()), ..Default::default() });
        let (layout, ()) = window.with_text_style(refinement, |window| self.styled.request_layout(global_id, inspector_id, window, cx));
        (layout, handle)
    }

    fn prepaint(&mut self, global_id: Option<&GlobalElementId>, inspector_id: Option<&InspectorElementId>, bounds: Bounds<Pixels>, handle: &mut TextSelectionHandle, window: &mut Window, cx: &mut App) -> Hitbox {
        self.styled.prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        let text_layout = self.styled.layout().clone();
        let line = text_layout.line_height();
        let mask = window.content_mask().bounds;
        let (first, last) = if self.placed.len() > 16 {
            let first = self.placed.partition_point(|p| text_layout.position_for_index(p.range.start).is_some_and(|at| at.y + line.max(p.size.height) < mask.top()));
            let last = self.placed.partition_point(|p| text_layout.position_for_index(p.range.start).is_none_or(|at| at.y <= mask.bottom()));
            (first, last.max(first))
        } else {
            (0, self.placed.len())
        };
        for placed in &mut self.placed[first..last] {
            if let (Some(mut start), Some(end)) = (text_layout.position_for_index(placed.range.start), text_layout.position_for_index(placed.range.end)) {
                if end.y != start.y
                    && let Some(second) = text_layout.position_for_index(placed.range.start + ROOM.len_utf8()).filter(|second| second.y == end.y)
                {
                    let letter = (end.x - second.x) / placed.letters.saturating_sub(1).max(1) as f32;
                    start = point((second.x - letter).max(bounds.left()), end.y);
                }
                let room = if end.y == start.y { end.x - start.x } else { placed.size.width };
                let x = (start.x + (room - placed.size.width) / 2.).min(bounds.right() - placed.size.width).max(bounds.left());
                let origin = point(x, start.y + (line - placed.size.height) / 2.);
                placed.element.prepaint_as_root(origin, placed.size.map(Into::into), window, cx);
                placed.shown = true;
            }
        }
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds).with_document_order(self.order).with_text_bounds(vec![bounds]).with_rendered_element(handle, window, cx);
        handle.register(registration, window, cx);
        hitbox
    }

    fn paint(&mut self, global_id: Option<&GlobalElementId>, inspector_id: Option<&InspectorElementId>, bounds: Bounds<Pixels>, handle: &mut TextSelectionHandle, hitbox: &mut Hitbox, window: &mut Window, cx: &mut App) {
        let layout = self.styled.layout().clone();
        let projection = handle.update_runs(&[TextSelectionRun::new(self.text.clone(), layout.clone(), bounds).with_document_order(self.order)], cx);
        for range in projection.ranges().iter().flatten().cloned() {
            if let (Some(start), Some(end)) = (layout.position_for_index(range.start), layout.position_for_index(range.end)) {
                for quad in selection_quads(start, end, layout.bounds(), layout.line_height()) {
                    window.paint_quad(fill(quad, self.color));
                }
            }
        }
        let line = layout.line_height();
        for pill in &self.pills {
            if let (Some(start), Some(end)) = (layout.position_for_index(pill.start), layout.position_for_index(pill.end))
                && start.y == end.y
            {
                let inset = px(2.);
                let quad = Bounds::from_corners(point(start.x - px(3.), start.y + inset), point(end.x + px(3.), end.y + line - inset));
                window.paint_quad(fill(quad, self.pill_color).corner_radii((line - inset * 2.) / 2.));
            }
        }
        self.styled.paint(global_id, inspector_id, bounds, &mut (), &mut (), window, cx);
        for placed in self.placed.iter_mut().filter(|p| p.shown) {
            placed.element.paint(window, cx);
        }
        if !self.links.is_empty() {
            let ranges: Vec<Range<usize>> = self.links.iter().map(|(r, _)| r.clone()).collect();
            let hovered_link = {
                let (ranges, layout, hitbox) = (ranges.clone(), layout.clone(), hitbox.clone());
                move |position: Point<Pixels>, window: &Window| {
                    if !hitbox.is_hovered(window) {
                        return None;
                    }
                    let at = layout.index_for_position(position).ok()?;
                    ranges.iter().position(|r| r.contains(&at))
                }
            };
            let painted = hovered_link(window.mouse_position(), window);
            if let Some(range) = painted.and_then(|i| ranges.get(i)).cloned() {
                let color = self.highlights.iter().find_map(|(r, h)| if r.contains(&range.start) { h.color } else { None }).unwrap_or_else(|| window.text_style().color);
                let mut lines: Vec<(Pixels, Pixels, Pixels)> = Vec::new();
                let boundaries = self.text.get(range.clone()).into_iter().flat_map(|t| t.char_indices().map(|(i, _)| range.start + i)).chain(std::iter::once(range.end));
                for position in boundaries.filter_map(|i| layout.position_for_index(i)) {
                    match lines.last_mut() {
                        Some((y, min, max)) if *y == position.y => {
                            *min = (*min).min(position.x);
                            *max = (*max).max(position.x);
                        }
                        _ => lines.push((position.y, position.x, position.x)),
                    }
                }
                for (y, min, max) in lines {
                    let tint = Bounds::from_corners(point(min - px(2.), y + px(1.)), point(max + px(2.), y + line - px(1.)));
                    window.paint_quad(fill(tint, color.opacity(0.14)).corner_radii(px(super::theme::radius::SMALL)));
                }
            }
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, _| {
                if phase.bubble() && hovered_link(e.position, window) != painted {
                    window.refresh();
                }
            });
            window.on_mouse_event(move |_: &MouseExitEvent, phase, window, _| {
                if phase.bubble() && painted.is_some() {
                    window.refresh();
                }
            });
        }
        let link_at = |links: &[(Range<usize>, LinkAction)], position: Point<Pixels>| layout.index_for_position(position).ok().and_then(|i| links.iter().find(|(r, _)| r.contains(&i)).map(|(_, a)| a.clone()));
        if hitbox.is_hovered(window) && link_at(&self.links, window.mouse_position()).is_some() {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        if !self.hovers.is_empty() {
            let (hovers, base, hitbox, layout) = (std::mem::take(&mut self.hovers), self.hover_base.take(), hitbox.clone(), layout.clone());
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                if phase.bubble() {
                    let target = if hitbox.is_hovered(window) { layout.index_for_position(e.position).ok().and_then(|i| hovers.iter().find(|(r, _)| r.contains(&i)).map(|(_, u)| u.clone())) } else { None };
                    super::link_card::hover(target, base.clone(), e.position, cx);
                }
            });
        }
        let (links, hitbox, layout) = (std::mem::take(&mut self.links), hitbox.clone(), layout.clone());
        let clicked_text = self.text.clone();
        let on_click = self.on_click.take();
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if phase.bubble() && e.button == MouseButton::Left && hitbox.is_hovered(window) && TextSelection::selected_text(window, cx).is_empty() {
                let action = layout.index_for_position(e.position).ok().and_then(|i| links.iter().find(|(r, _)| r.contains(&i)).map(|(_, a)| a.clone()));
                if action.is_none()
                    && let Some(on_click) = &on_click
                {
                    let at = layout.index_for_position(e.position).unwrap_or_else(|i| i);
                    on_click(&clicked_text, at, window, cx);
                    cx.stop_propagation();
                }
                if let Some(action) = action {
                    cx.stop_propagation();
                    crate::core::library::LINK_IN_NEW_TAB.store(e.modifiers.platform || e.modifiers.control, std::sync::atomic::Ordering::Relaxed);
                    action(cx);
                    super::link_card::hide(cx);
                    crate::core::library::LINK_IN_NEW_TAB.store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });
    }
}
