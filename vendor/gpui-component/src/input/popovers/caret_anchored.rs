//! Caret-anchored popups positioned from the input's current-frame geometry.
//!
//! The input stashes fresh caret geometry in its prepaint
//! ([`PrepaintCaretGeometry`]). Popups live inside `deferred()`, so their
//! prepaint runs after the input's prepaint but before its paint; resolving
//! the anchor there puts the popup at the current frame's caret instead of
//! the previous frame's.

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, Size, Style, WeakEntity, Window, px,
};

use crate::input::{EditorState, PrepaintCaretGeometry};

const POPUP_GAP: Pixels = px(4.);

/// Computes the caret-anchored popup origin from fresh prepaint geometry.
///
/// Shared by the completion and code-action menus so both anchor to the
/// same point: just below the caret, nudged left. The result is in window
/// coordinates, matching where the caret itself is painted.
pub(crate) fn caret_popup_anchor(geometry: PrepaintCaretGeometry) -> Point<Pixels> {
    Point::new(
        geometry.cursor_bounds().origin.x - POPUP_GAP,
        geometry.cursor_bounds().origin.y
            + geometry.scroll_offset().y
            + geometry.line_height()
            + POPUP_GAP,
    )
}

fn caret_popup_ceiling(geometry: PrepaintCaretGeometry) -> Pixels {
    geometry.cursor_bounds().origin.y + geometry.scroll_offset().y - POPUP_GAP
}

fn place_popup(
    below: Point<Pixels>,
    ceiling: Pixels,
    size: Size<Pixels>,
    planned: Pixels,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let room_below = viewport.height - below.y - POPUP_GAP;
    let room_above = ceiling - POPUP_GAP;
    let height = size.height.max(planned);
    let y = if height > room_below && room_above > room_below {
        (ceiling - size.height).max(POPUP_GAP)
    } else {
        below.y
    };
    let x = below
        .x
        .min(viewport.width - size.width - POPUP_GAP)
        .max(Pixels::ZERO);
    Point::new(x, y)
}

/// A popup positioned at the caret from the input's current-frame prepaint.
///
/// The content is built in prepaint (not render) from the resolved anchor,
/// so position, hitboxes, accessibility bounds, and width/layout decisions
/// are all consistent with the current frame's caret. Always use inside
/// `deferred()` so the prepaint runs after the input's prepaint.
///
/// The builder also gets the height available on the roomier side of the
/// caret line. The popup opens above the line when its planned height does
/// not fit below and there is more room above.
pub(crate) struct CaretAnchoredPopup {
    editor: WeakEntity<EditorState>,
    build: Box<dyn Fn(Point<Pixels>, Pixels, &mut Window, &mut App) -> AnyElement>,
    planned_height: Pixels,
    content: Option<AnyElement>,
}

impl CaretAnchoredPopup {
    pub(crate) fn new(
        editor: WeakEntity<EditorState>,
        build: impl Fn(Point<Pixels>, Pixels, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            editor,
            build: Box::new(build),
            planned_height: Pixels::ZERO,
            content: None,
        }
    }

    pub(crate) fn planned_height(mut self, height: Pixels) -> Self {
        self.planned_height = height;
        self
    }
}

impl Element for CaretAnchoredPopup {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // The real layout happens in prepaint, once the anchor is resolved.
        let layout_id = window.request_layout(Style::default(), [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let Some(geometry) = editor.read(cx).prepaint_caret_geometry() else {
            return;
        };
        let anchor = caret_popup_anchor(geometry);
        let ceiling = caret_popup_ceiling(geometry);
        let viewport = window.viewport_size();
        let room = (viewport.height - anchor.y)
            .max(ceiling)
            .max(POPUP_GAP * 2.)
            - POPUP_GAP;
        let mut content = (self.build)(anchor, room, window, cx);
        let size = content.layout_as_root(
            Size {
                width: AvailableSpace::Definite(viewport.width),
                height: AvailableSpace::Definite(viewport.height),
            },
            window,
            cx,
        );
        let origin = place_popup(anchor, ceiling, size, self.planned_height, viewport);
        content.prepaint_at(origin, window, cx);
        self.content = Some(content);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(content) = self.content.as_mut() {
            content.paint(window, cx);
        }
    }
}

impl IntoElement for CaretAnchoredPopup {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};

    #[test]
    fn a_popup_opens_above_when_it_does_not_fit_below() {
        let viewport = size(px(800.), px(600.));
        let menu = size(px(200.), px(250.));
        assert_eq!(
            place_popup(point(px(100.), px(100.)), px(76.), menu, px(250.), viewport),
            point(px(100.), px(100.))
        );
        assert_eq!(
            place_popup(point(px(100.), px(500.)), px(476.), menu, px(250.), viewport),
            point(px(100.), px(226.))
        );
        let short = size(px(200.), px(60.));
        assert_eq!(
            place_popup(point(px(100.), px(500.)), px(476.), short, px(250.), viewport),
            point(px(100.), px(416.)),
            "a list that shrank stays on the side its full height chose"
        );
        assert_eq!(
            place_popup(point(px(700.), px(100.)), px(76.), menu, px(0.), viewport).x,
            px(596.)
        );
    }
}
