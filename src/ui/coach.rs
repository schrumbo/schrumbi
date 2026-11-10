use std::cell::RefCell;
use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion;
use super::theme::{Theme, coach, font, radius, space};
use crate::core::i18n::tr;

/// The element a tip points at: dimmed surroundings, a pulsing ring and a short label.
struct Coach {
    wanted: Option<SharedString>,
    label: String,
    found: Option<Bounds<Pixels>>,
    origin: Point<Pixels>,
    since: Instant,
    generation: u64,
}

thread_local! {
    static COACH: RefCell<Option<Coach>> = const { RefCell::new(None) };
}

/// Points at the element with `id`, or only says `label` when `id` is empty, until a click or Esc.
pub fn start(id: &str, label: String, cx: &mut App) {
    COACH.with_borrow_mut(|c| {
        let generation = c.as_ref().map_or(0, |c| c.generation) + 1;
        *c = Some(Coach { wanted: (!id.is_empty()).then(|| SharedString::from(id.to_string())), label, found: None, origin: point(px(0.), px(0.)), since: Instant::now(), generation });
    });
    cx.refresh_windows();
}

/// Takes the mark away; `true` when one was showing.
pub fn stop(cx: &mut App) -> bool {
    let was = COACH.with_borrow_mut(Option::take).is_some();
    if was {
        cx.refresh_windows();
    }
    was
}

pub fn active() -> bool {
    COACH.with_borrow(Option::is_some)
}

fn wants(id: &ElementId) -> bool {
    COACH.with_borrow(|c| c.as_ref().and_then(|c| c.wanted.as_ref()).is_some_and(|w| *w == SharedString::from(id.to_string())))
}

fn record(bounds: Bounds<Pixels>, window: &mut Window) {
    let changed = COACH.with_borrow_mut(|c| {
        let Some(c) = c.as_mut() else { return false };
        let changed = c.found != Some(bounds);
        c.found = Some(bounds);
        changed
    });
    if changed {
        window.refresh();
    }
}

/// Lets the element be found when a tip points at it; adds nothing otherwise.
pub fn anchor(mut element: Stateful<Div>) -> Stateful<Div> {
    if !active() {
        return element;
    }
    let Some(id) = element.interactivity().element_id.clone() else { return element };
    if !wants(&id) {
        return element;
    }
    element.child(canvas(move |bounds, window, _| record(bounds, window), |_, _, _, _| {}).absolute().inset_0())
}

/// Marks the layer the overlay is drawn in, so the ring lands on the element whatever sits above the layer.
pub fn root(element: Stateful<Div>) -> Stateful<Div> {
    if !active() {
        return element;
    }
    element.child(
        canvas(
            move |bounds, window, _| {
                let changed = COACH.with_borrow_mut(|c| {
                    let Some(c) = c.as_mut() else { return false };
                    let changed = c.origin != bounds.origin;
                    c.origin = bounds.origin;
                    changed
                });
                if changed {
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0(),
    )
}

fn shade(color: Hsla, left: f32, top: f32, width: f32, height: f32) -> Div {
    div().absolute().left(px(left)).top(px(top)).w(px(width.max(0.))).h(px(height.max(0.))).bg(color)
}

/// The layer over the window while a mark shows; nothing when none does.
pub fn overlay(window: &mut Window, cx: &App, t: &Theme) -> Option<AnyElement> {
    let (label, target, generation, waited) = COACH.with_borrow(|c| {
        let c = c.as_ref()?;
        let target = c.found.map(|b| Bounds { origin: b.origin - c.origin, size: b.size });
        Some((c.label.clone(), target, c.generation, c.since.elapsed() > motion::MEDIUM))
    })?;
    let size = window.viewport_size();
    let (w, h) = (f32::from(size.width), f32::from(size.height));
    let dim = black().opacity(if t.dark { coach::DIM_DARK } else { coach::DIM });
    let mut layer = div().id("coach-overlay").absolute().inset_0().occlude().cursor_pointer().on_click(|_, _, cx| {
        stop(cx);
    });
    let hint = div().text_size(px(font::CAPTION)).text_color(t.secondary).child(tr("Klick oder Esc schließt"));
    let bubble = |text: String| {
        div()
            .max_w(px(coach::LABEL_WIDTH))
            .p(px(space::M))
            .rounded(px(radius::CARD))
            .bg(t.material)
            .border_1()
            .border_color(t.separator)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .child(div().font_weight(FontWeight::MEDIUM).child(text))
            .child(hint)
    };
    match target {
        Some(b) => {
            let pad = space::XS;
            let (x, y) = (f32::from(b.origin.x) - pad, f32::from(b.origin.y) - pad);
            let (bw, bh) = (f32::from(b.size.width) + 2. * pad, f32::from(b.size.height) + 2. * pad);
            layer = layer.child(shade(dim, 0., 0., w, y)).child(shade(dim, 0., y + bh, w, h - y - bh)).child(shade(dim, 0., y, x, bh)).child(shade(dim, x + bw, y, w - x - bw, bh));
            let accent = t.accent;
            let ring = div().absolute().left(px(x)).top(px(y)).w(px(bw)).h(px(bh)).rounded(px(radius::CONTROL)).border_2().border_color(accent);
            let pulse = div().absolute().left(px(x)).top(px(y)).w(px(bw)).h(px(bh)).rounded(px(radius::CONTROL)).border_2().border_color(accent);
            let pulse = if cx.reduce_motion() {
                pulse.opacity(0.).into_any_element()
            } else {
                pulse
                    .with_animation(("coach-pulse", generation), Animation::new(motion::PULSE).repeat(), move |d, p| {
                        let grow = coach::PULSE_REACH * p;
                        d.left(px(x - grow)).top(px(y - grow)).w(px(bw + 2. * grow)).h(px(bh + 2. * grow)).border_color(accent.opacity(1. - p))
                    })
                    .into_any_element()
            };
            let below = y + bh + space::M;
            let fits_below = below + coach::LABEL_ROOM < h;
            let left = (x + bw / 2. - coach::LABEL_WIDTH / 2.).clamp(space::L, (w - coach::LABEL_WIDTH - space::L).max(space::L));
            let mut tag = div().absolute().left(px(left)).w(px(coach::LABEL_WIDTH));
            tag = if fits_below { tag.top(px(below)) } else { tag.bottom(px(h - y + space::M)) };
            layer = layer.child(ring).child(pulse).child(tag.child(bubble(label)));
        }
        None if waited => {
            layer = layer.child(shade(dim, 0., 0., w, h)).child(div().absolute().inset_0().flex().items_center().justify_center().child(bubble(label)));
        }
        None => window.request_animation_frame(),
    }
    Some(layer.with_animation(("coach-in", generation), Animation::new(motion::timed(motion::MEDIUM, cx)).with_easing(ease_out_quint()), |d, p| d.opacity(p)).into_any_element())
}
