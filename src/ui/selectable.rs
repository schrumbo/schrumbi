use std::ops::Range;
use std::rc::Rc;

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
    order: u64,
    color: Hsla,
    links: Vec<(Range<usize>, LinkAction)>,
    /// Link targets for the preview card, with the folder they are relative to.
    hovers: Vec<(Range<usize>, String)>,
    hover_base: Option<std::path::PathBuf>,
}

/// Something drawn inside a sentence, at a byte position of the text.
pub struct InlineSlot {
    pub at: usize,
    pub kind: InlineKind,
}

pub enum InlineKind {
    Math(String),
    Sized(AnyElement, f32, f32),
}

struct Placed {
    range: Range<usize>,
    element: AnyElement,
    size: Size<Pixels>,
}

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
    SelectableStyled { id: id.into(), styled: StyledText::new(text.clone()).with_highlights(highlights.clone()), highlights, slots: Vec::new(), placed: Vec::new(), pills: Vec::new(), pill_color: transparent_black(), text, order, color, links: Vec::new(), hovers: Vec::new(), hover_base: None }
}

impl SelectableStyled {
    /// Puts rounded backgrounds behind parts of the text, as for tags.
    pub fn pills(mut self, pills: Vec<Range<usize>>, color: Hsla) -> Self {
        self.pills = pills;
        self.pill_color = color;
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
        if self.slots.is_empty() {
            return None;
        }
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = style.line_height_in_pixels(window.rem_size());
        let system = window.text_system();
        let letter = system.advance(system.resolve_font(&style.font()), font_size, ROOM).map_or(px(4.), |a| a.width).max(px(1.));
        let source = self.text.to_string();
        let mut text = String::with_capacity(source.len());
        let mut copied = 0;
        let mut moves: Vec<(usize, usize)> = Vec::new();
        let mut rooms: Vec<(Range<usize>, Option<AnyElement>, Size<Pixels>)> = Vec::new();
        let mut tallest = px(0.);
        for slot in std::mem::take(&mut self.slots) {
            let (element, size) = match slot.kind {
                InlineKind::Math(formula) => match super::embeds::inline_math(&formula, cx) {
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
            moves.push((slot.at, text.len() - start));
            rooms.push((start..text.len(), element, size));
            tallest = tallest.max(size.height);
        }
        text.push_str(&source[copied..]);
        let moved = |i: usize| i + moves.iter().filter(|(at, _)| *at <= i).map(|(_, by)| by).sum::<usize>();
        let shift_end = |i: usize| i + moves.iter().filter(|(at, _)| *at < i).map(|(_, by)| by).sum::<usize>();
        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = std::mem::take(&mut self.highlights).into_iter().map(|(r, h)| (moved(r.start)..shift_end(r.end), h)).collect();
        for (room, ..) in &rooms {
            highlights.push((room.clone(), HighlightStyle { fade_out: Some(1.), ..Default::default() }));
        }
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
        self.placed = rooms.into_iter().filter_map(|(range, element, size)| element.map(|element| Placed { range, element, size })).collect();
        Some(line.max(tallest + px(4.)))
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
        let handle = window.with_element_state(global_id.expect("selectable text has an id"), |kept: Option<TextSelectionHandle>, _| {
            let handle = kept.unwrap_or_else(|| TextSelectionHandle::new(text, cx));
            (handle.clone(), handle)
        });
        let refinement = line_height.map(|h| TextStyleRefinement { line_height: Some(h.into()), ..Default::default() });
        let (layout, ()) = window.with_text_style(refinement, |window| self.styled.request_layout(global_id, inspector_id, window, cx));
        (layout, handle)
    }

    fn prepaint(&mut self, global_id: Option<&GlobalElementId>, inspector_id: Option<&InspectorElementId>, bounds: Bounds<Pixels>, handle: &mut TextSelectionHandle, window: &mut Window, cx: &mut App) -> Hitbox {
        self.styled.prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        let text_layout = self.styled.layout().clone();
        for placed in &mut self.placed {
            if let (Some(start), Some(end)) = (text_layout.position_for_index(placed.range.start), text_layout.position_for_index(placed.range.end)) {
                let room = if end.y == start.y { end.x - start.x } else { placed.size.width };
                let origin = point(start.x + (room - placed.size.width) / 2., start.y + (text_layout.line_height() - placed.size.height) / 2.);
                placed.element.prepaint_as_root(origin, placed.size.map(Into::into), window, cx);
            }
        }
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds).with_document_order(self.order).with_text_bounds(vec![bounds]).with_rendered_element(handle, window, cx);
        handle.register(registration, window, cx);
        hitbox
    }

    fn paint(&mut self, global_id: Option<&GlobalElementId>, inspector_id: Option<&InspectorElementId>, bounds: Bounds<Pixels>, handle: &mut TextSelectionHandle, hitbox: &mut Hitbox, window: &mut Window, cx: &mut App) {
        let layout = self.styled.layout().clone();
        let before = TextSelection::selected_text(window, cx);
        let projection = handle.update_runs(&[TextSelectionRun::new(self.text.clone(), layout.clone(), bounds).with_document_order(self.order)], cx);
        if before != TextSelection::selected_text(window, cx) {
            window.refresh();
        }
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
        for placed in &mut self.placed {
            placed.element.paint(window, cx);
        }
        if self.links.is_empty() {
            return;
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
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if phase.bubble() && e.button == MouseButton::Left && hitbox.is_hovered(window) && TextSelection::selected_text(window, cx).is_empty() {
                let action = layout.index_for_position(e.position).ok().and_then(|i| links.iter().find(|(r, _)| r.contains(&i)).map(|(_, a)| a.clone()));
                if let Some(action) = action {
                    crate::core::library::LINK_IN_NEW_TAB.store(e.modifiers.platform || e.modifiers.control, std::sync::atomic::Ordering::Relaxed);
                    action(cx);
                    crate::core::library::LINK_IN_NEW_TAB.store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });
    }
}
