use std::ops::Range;
use std::rc::Rc;

use gpui_kit::base::{TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun};
use gpui_kit::*;

/// Styled text that takes part in the window's text selection, which can span several blocks.
pub struct SelectableStyled {
    id: ElementId,
    text: SharedString,
    styled: StyledText,
    order: u64,
    color: Hsla,
    links: Vec<(Range<usize>, LinkAction)>,
}

/// What clicking a link does.
pub type LinkAction = Rc<dyn Fn(&mut App)>;

/// Wraps `text` and its highlights; `order` is its place in reading order.
pub fn selectable(id: impl Into<ElementId>, text: String, highlights: Vec<(Range<usize>, HighlightStyle)>, order: u64, color: Hsla) -> SelectableStyled {
    let text = SharedString::from(text);
    SelectableStyled { id: id.into(), styled: StyledText::new(text.clone()).with_highlights(highlights), text, order, color, links: Vec::new() }
}

impl SelectableStyled {
    /// Makes byte ranges of the text clickable; a click that selected text does not count.
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
        let text = self.text.to_string();
        let handle = window.with_element_state(global_id.expect("selectable text has an id"), |kept: Option<TextSelectionHandle>, _| {
            let handle = kept.unwrap_or_else(|| TextSelectionHandle::new(text, cx));
            (handle.clone(), handle)
        });
        let (layout, ()) = self.styled.request_layout(global_id, inspector_id, window, cx);
        (layout, handle)
    }

    fn prepaint(&mut self, global_id: Option<&GlobalElementId>, inspector_id: Option<&InspectorElementId>, bounds: Bounds<Pixels>, handle: &mut TextSelectionHandle, window: &mut Window, cx: &mut App) -> Hitbox {
        self.styled.prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
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
        self.styled.paint(global_id, inspector_id, bounds, &mut (), &mut (), window, cx);
        if self.links.is_empty() {
            return;
        }
        let link_at = |links: &[(Range<usize>, LinkAction)], position: Point<Pixels>| layout.index_for_position(position).ok().and_then(|i| links.iter().find(|(r, _)| r.contains(&i)).map(|(_, a)| a.clone()));
        if hitbox.is_hovered(window) && link_at(&self.links, window.mouse_position()).is_some() {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        let (links, hitbox, layout) = (std::mem::take(&mut self.links), hitbox.clone(), layout.clone());
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if phase.bubble() && e.button == MouseButton::Left && hitbox.is_hovered(window) && TextSelection::selected_text(window, cx).is_empty() {
                let action = layout.index_for_position(e.position).ok().and_then(|i| links.iter().find(|(r, _)| r.contains(&i)).map(|(_, a)| a.clone()));
                if let Some(action) = action {
                    action(cx);
                }
            }
        });
    }
}
