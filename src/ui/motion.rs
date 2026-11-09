use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::base::{MotionReveal, Transition, transition};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::theme::{Theme, iconsize, radius, space};
use super::widgets::icon;

/// Hover, press, small confirmations.
pub const FAST: Duration = Duration::from_millis(120);
/// Reveals, sidebar, sheets.
pub const MEDIUM: Duration = Duration::from_millis(180);
/// Elements travelling across the window.
pub const SLOW: Duration = Duration::from_millis(240);

/// How long the offer to take back a settings change stays.
pub const NOTICE: Duration = Duration::from_secs(8);

/// How long the sign that a PDF is being written stays once it showed, so it does not flicker.
pub const STATUS_MIN: Duration = Duration::from_millis(400);

/// How long the page list waits for a frame after it moved a row into view, before it opens that row's menu.
pub const MENU_AFTER_MOVE: Duration = Duration::from_millis(40);

/// How often a page list scrolls while a dragged item rests at its edge.
pub const DRAG_TICK: Duration = Duration::from_millis(16);

/// How long a section found by search stays lit.
pub const FLASH: Duration = Duration::from_millis(1400);

/// How long a failed save waits before it tries once more.
pub const SAVE_RETRY_WAIT: Duration = Duration::from_millis(400);

/// How often a note whose folder is gone looks for the folder again.
pub const FOLDER_RETRY: Duration = Duration::from_secs(3);

/// How long the wrap switch confirms itself in the word count bar.
pub const WRAP_NOTICE_FOR: Duration = Duration::from_millis(1600);

/// How long list rows keep sliding to new positions after a change; covers the rescan that follows it.
const FLIP_ARMED: Duration = Duration::from_millis(500);

/// A duration for `with_animation`, which does not look at the reduced-motion setting by itself.
pub fn timed(duration: Duration, cx: &App) -> Duration {
    if cx.reduce_motion() { Duration::from_millis(1) } else { duration }
}

fn ease_out(p: f32) -> f32 {
    1. - (1. - p).powi(3)
}

/// An interruptible, ease-out value that follows `target`; it adopts its first value without animating.
pub fn follow(id: impl Into<ElementId>, target: f32, duration: Duration, window: &mut Window, cx: &mut App) -> f32 {
    transition(id.into(), target, Transition::new(duration), window, cx)
}

/// Hover and press feedback that fades instead of switching.
pub trait Soft {
    /// Paints `color` behind the children on hover and twice as strong on press. Call it before adding children.
    fn soft(self, color: Hsla, radius: f32) -> Self;
    /// Like `soft`, with a line of `width` at the left edge in `line` that fades with it.
    fn soft_edge(self, color: Hsla, radius: f32, line: Hsla, width: f32) -> Self;
}

impl Soft for Stateful<Div> {
    fn soft(self, color: Hsla, radius: f32) -> Self {
        self.soft_edge(color, radius, color, 0.)
    }

    fn soft_edge(mut self, color: Hsla, radius: f32, line: Hsla, width: f32) -> Self {
        if self.style().position.is_none() {
            self = self.relative();
        }
        super::drive::marked(self, "clickable").child(HoverLayer { color, radius, line, width })
    }
}

#[derive(IntoElement)]
struct HoverLayer {
    color: Hsla,
    radius: f32,
    line: Hsla,
    width: f32,
}

impl RenderOnce for HoverLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("soft-state", cx, |_, _| (false, false));
        let (hovered, pressed) = *state.read(cx);
        let target = if pressed { 2. } else if hovered { 1. } else { 0. };
        let level = follow("soft-level", target, FAST, window, cx);
        let (on_hover, on_down, on_up, on_up_out) = (state.clone(), state.clone(), state.clone(), state);
        div()
            .id("soft")
            .absolute()
            .inset_0()
            .rounded(px(self.radius))
            .when(level > 0., |d| d.bg(Hsla { a: (self.color.a * level).min(1.), ..self.color }))
            .when(level > 0. && self.width > 0., |d| d.child(div().absolute().top_0().bottom_0().left_0().w(px(self.width)).bg(Hsla { a: (self.line.a * level).min(1.), ..self.line })))
            .on_hover(move |hovered, _, cx| on_hover.update(cx, |s, cx| {
                s.0 = *hovered;
                s.1 &= *hovered;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| on_down.update(cx, |s, cx| {
                s.1 = true;
                cx.notify();
            }))
            .capture_any_mouse_up(move |_, _, cx| on_up.update(cx, |s, cx| {
                s.1 = false;
                cx.notify();
            }))
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| on_up_out.update(cx, |s, cx| {
                s.1 = false;
                cx.notify();
            }))
    }
}

/// Content that unfolds and folds in place.
#[derive(IntoElement)]
pub struct Reveal {
    id: ElementId,
    open: bool,
    content: AnyElement,
}

/// Unfolds `content` while `open` is true. It folds away smoothly as long as it is still passed in.
pub fn reveal(id: impl Into<ElementId>, open: bool, content: impl IntoElement) -> Reveal {
    Reveal { id: id.into(), open, content: content.into_any_element() }
}

impl RenderOnce for Reveal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let progress = follow(ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "reveal".into()), if self.open { 1. } else { 0. }, MEDIUM, window, cx);
        if progress <= 0. {
            return div().into_any_element();
        }
        MotionReveal::new(self.id, progress, self.content).into_any_element()
    }
}

/// A block that lights up in `color` while `lit` and fades in and out around it.
#[derive(IntoElement)]
pub struct Glow {
    id: ElementId,
    lit: bool,
    color: Hsla,
    content: AnyElement,
}

/// Wraps `content` in a tint that eases in when `lit` turns true and out when it turns false; `color` carries the strength.
pub fn glow(id: impl Into<ElementId>, lit: bool, color: Hsla, content: impl IntoElement) -> Glow {
    Glow { id: id.into(), lit, color, content: content.into_any_element() }
}

impl RenderOnce for Glow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let level = follow(ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "glow".into()), if self.lit { 1. } else { 0. }, timed(MEDIUM, cx), window, cx);
        div()
            .id(self.id)
            .mx(px(-space::M))
            .px(px(space::M))
            .rounded(px(radius::CONTROL))
            .when(level > 0., |d| d.bg(Hsla { a: self.color.a * level, ..self.color }))
            .child(self.content)
    }
}

/// A disclosure chevron that turns as its section opens.
#[derive(IntoElement)]
pub struct Chevron {
    id: ElementId,
    open: bool,
    color: Hsla,
}

/// Disclosure chevron; right when closed, down when open.
pub fn chevron(id: impl Into<ElementId>, open: bool, color: Hsla) -> Chevron {
    Chevron { id: id.into(), open, color }
}

impl RenderOnce for Chevron {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let angle = follow(ElementId::NamedChild(std::sync::Arc::new(self.id), "chevron".into()), if self.open { 90f32.to_radians() } else { 0. }, MEDIUM, window, cx);
        icon("chevron-right", iconsize::XS, self.color).with_transformation(Transformation::rotate(radians(angle)))
    }
}

/// A ring that grows and fades once when `done` turns true, confirming a checkmark.
#[derive(IntoElement)]
pub struct DonePulse {
    id: ElementId,
    done: bool,
    size: f32,
}

/// Confirmation ring around a status glyph of `size`; place it in a relatively positioned parent.
pub fn done_pulse(id: impl Into<ElementId>, done: bool, size: f32) -> DonePulse {
    DonePulse { id: id.into(), done, size }
}

impl RenderOnce for DonePulse {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let green = Theme::get(cx).green;
        let progress = follow(ElementId::NamedChild(std::sync::Arc::new(self.id), "pulse".into()), if self.done { 1. } else { 0. }, SLOW, window, cx);
        let running = self.done && progress > 0. && progress < 1.;
        div().when(running, |d| {
            let grow = self.size * 0.45 * progress;
            d.absolute()
                .top(px(-grow))
                .left(px(-grow))
                .size(px(self.size + grow * 2.))
                .rounded_full()
                .border_2()
                .border_color(green.opacity(0.7 * (1. - progress)))
        })
    }
}

#[derive(Default)]
struct FlipStore {
    armed_until: Option<Instant>,
    rows: HashMap<SharedString, FlipRow>,
}

struct FlipRow {
    position: Point<Pixels>,
    slide: Option<(Instant, Point<Pixels>)>,
}

impl Global for FlipStore {}

/// Lets rows slide to their new places for the next moments; call it when a change reorders lists.
pub fn arm_rows(cx: &mut App) {
    cx.default_global::<FlipStore>().armed_until = Some(Instant::now() + FLIP_ARMED);
}

/// Forgets row positions, so a list shown again does not slide from where it once was.
pub fn forget_rows(cx: &mut App) {
    cx.default_global::<FlipStore>().rows.clear();
}

/// A list row that slides from its previous position when a change moved it.
pub struct Flip {
    key: SharedString,
    child: AnyElement,
}

/// Wraps a row; `key` identifies it across lists, so it also slides from one group into another.
pub fn flip(key: impl Into<SharedString>, child: impl IntoElement) -> Flip {
    Flip { key: key.into(), child: child.into_any_element() }
}

impl IntoElement for Flip {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Flip {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) {
        let position = bounds.origin - window.element_offset();
        let reduce = cx.reduce_motion();
        let store = cx.default_global::<FlipStore>();
        let now = Instant::now();
        let armed = store.armed_until.is_some_and(|until| now < until) && !reduce;
        let row = store.rows.entry(self.key.clone()).or_insert(FlipRow { position, slide: None });
        let shown = row.slide.map_or(point(px(0.), px(0.)), |(start, from)| {
            let progress = (now - start).as_secs_f32() / SLOW.as_secs_f32();
            if progress >= 1. { point(px(0.), px(0.)) } else { from * (1. - ease_out(progress)) }
        });
        if row.position != position {
            row.slide = armed.then(|| (now, shown + (row.position - position)));
            row.position = position;
        }
        let offset = match row.slide {
            Some((start, from)) if (now - start) < SLOW => {
                window.request_animation_frame();
                from * (1. - ease_out((now - start).as_secs_f32() / SLOW.as_secs_f32()))
            }
            _ => {
                row.slide = None;
                point(px(0.), px(0.))
            }
        };
        window.with_element_offset(offset, |window| self.child.prepaint(window, cx));
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        self.child.paint(window, cx);
    }
}

/// A turning progress mark. With `delayed` it stays invisible for the first 300 ms, so quick work shows nothing.
#[derive(IntoElement)]
pub struct Spinner {
    size: f32,
    color: Hsla,
    delayed: bool,
}

/// Progress mark for work that is known to take a while.
pub fn spinner(size: f32, color: Hsla) -> Spinner {
    Spinner { size, color, delayed: false }
}

/// Progress mark that only appears once the work takes noticeably long.
pub fn loading(size: f32, color: Hsla) -> Spinner {
    Spinner { size, color, delayed: true }
}

impl RenderOnce for Spinner {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let glyph = icon("loader", self.size, self.color);
        let turning = if cx.reduce_motion() {
            glyph.into_any_element()
        } else {
            glyph.with_animation("spin", Animation::new(Duration::from_millis(1000)).repeat(), |s, p| s.with_transformation(Transformation::rotate(percentage(p)))).into_any_element()
        };
        let delay = if self.delayed { 300. } else { 0. };
        let total = delay + 120.;
        div().child(turning).with_animation("appear", Animation::new(timed(Duration::from_millis(total as u64), cx)), move |d, p| d.opacity(((p * total - delay) / 120.).clamp(0., 1.)))
    }
}
