use std::{sync::Arc, time::Duration};

use gpui::{App, ElementId, Entity, Hsla, Window};

use crate::Colorize as _;

pub(crate) type HoverState = Entity<(bool, bool)>;

pub(crate) fn hover_level(
    id: &ElementId,
    interactive: bool,
    window: &mut Window,
    cx: &mut App,
) -> (HoverState, f32) {
    let key = ElementId::NamedChild(Arc::new(id.clone()), "hover".into());
    let state = window.use_keyed_state(key.clone(), cx, |_, _| (false, false));
    let (hovered, pressed) = *state.read(cx);
    let target = if !interactive {
        0.
    } else if pressed {
        2.
    } else if hovered {
        1.
    } else {
        0.
    };
    let level = gpui_base::transition(
        ElementId::NamedChild(Arc::new(key), "level".into()),
        target,
        gpui_base::Transition::new(Duration::from_millis(120)),
        window,
        cx,
    );
    (state, level)
}

pub(crate) fn set_hover(state: &HoverState, hovered: bool, cx: &mut App) {
    state.update(cx, |s, cx| {
        if s.0 != hovered {
            s.0 = hovered;
            cx.notify();
        }
    });
}

pub(crate) fn set_pressed(state: &HoverState, pressed: bool, cx: &mut App) {
    state.update(cx, |s, cx| {
        if s.1 != pressed {
            s.1 = pressed;
            cx.notify();
        }
    });
}

pub(crate) fn lift(base: Hsla, toward: Hsla, level: f32, strength: f32) -> Hsla {
    if level <= 0. {
        return base;
    }
    base.mix_oklab(toward, 1. - (level * strength).min(1.))
}
