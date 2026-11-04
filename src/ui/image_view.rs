use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::Soft;
use super::theme::{Theme, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::tr;
use crate::core::vault::render;

const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 32.;
/// Room left around an image that is fitted into its view.
const FIT_MARGIN: f32 = 16.;

/// One image, to look at closely: zooms around the pointer and moves by dragging.
pub struct ImageView {
    path: PathBuf,
    /// Screen points per image point; `None` fits the image into the view.
    zoom: Option<f32>,
    /// Where the image's top left corner stands within the view, while zoomed.
    origin: (f32, f32),
    view: Bounds<Pixels>,
    grab: Option<(Point<Pixels>, (f32, f32))>,
    focus: FocusHandle,
    /// A sharp drawing of a vector image for the current zoom.
    vector: Option<(f32, Arc<Image>)>,
    drawing: Option<(f32, Task<()>)>,
}

/// The factor a wheel or trackpad movement zooms by.
pub fn wheel_factor(delta: &ScrollDelta) -> f32 {
    match delta {
        ScrollDelta::Pixels(p) => (f32::from(p.y) * 0.004).exp(),
        ScrollDelta::Lines(l) => (l.y.clamp(-1., 1.) * 0.12).exp(),
    }
}

/// Whether the pressed keys turn the wheel into zoom: ⌘ or Ctrl.
pub fn zooms(modifiers: &Modifiers) -> bool {
    modifiers.platform || modifiers.control
}

/// Catches ⌘ or Ctrl plus wheel before anything scrolls; the rest of the wheel is left alone.
pub fn wheel_zoom(on: impl Fn(f32, Point<Pixels>, &mut Window, &mut App) + 'static) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            window.on_mouse_event(move |e: &ScrollEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture && zooms(&e.modifiers) && hitbox.is_hovered(window) {
                    on(wheel_factor(&e.delta), e.position, window, cx);
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn is_vector(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

impl ImageView {
    pub fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        Self { path, zoom: None, origin: (0., 0.), view: Bounds::default(), grab: None, focus: cx.focus_handle(), vector: None, drawing: None }
    }

    /// Takes the keyboard, so ⌘+, ⌘- and ⌘0 work right away.
    pub fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn view_size(&self) -> (f32, f32) {
        (f32::from(self.view.size.width), f32::from(self.view.size.height))
    }

    fn fit(&self, natural: (f32, f32)) -> f32 {
        let (w, h) = self.view_size();
        (((w - FIT_MARGIN * 2.).max(1.) / natural.0).min((h - FIT_MARGIN * 2.).max(1.) / natural.1)).min(1.)
    }

    fn scale(&self, natural: (f32, f32)) -> f32 {
        self.zoom.unwrap_or_else(|| self.fit(natural))
    }

    /// Keeps the image in the view: small sides stay centred, big ones cannot leave a gap.
    fn place(&self, natural: (f32, f32), scale: f32, origin: (f32, f32)) -> (f32, f32) {
        let (vw, vh) = self.view_size();
        let axis = |view: f32, size: f32, at: f32| if size <= view { (view - size) / 2. } else { at.clamp(view - size, 0.) };
        (axis(vw, natural.0 * scale, origin.0), axis(vh, natural.1 * scale, origin.1))
    }

    fn zoom_to(&mut self, next: f32, around: Option<Point<Pixels>>, natural: (f32, f32), cx: &mut Context<Self>) {
        let scale = self.scale(natural);
        let next = next.clamp(MIN_ZOOM, MAX_ZOOM);
        let (vw, vh) = self.view_size();
        let at = around.map_or((vw / 2., vh / 2.), |p| (f32::from(p.x - self.view.origin.x), f32::from(p.y - self.view.origin.y)));
        let now = self.place(natural, scale, self.origin);
        let image_point = ((at.0 - now.0) / scale, (at.1 - now.1) / scale);
        self.origin = self.place(natural, next, (at.0 - image_point.0 * next, at.1 - image_point.1 * next));
        self.zoom = Some(next);
        cx.notify();
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.zoom = None;
        cx.notify();
    }

    /// The image's own size in points, once loaded.
    fn natural(&self, window: &mut Window, cx: &mut App) -> Result<Option<(f32, f32)>, ()> {
        let resource = Resource::Path(Arc::from(self.path.as_path()));
        match window.use_asset::<ImgResourceLoader>(&resource, cx) {
            None => Ok(None),
            Some(Err(_)) => Err(()),
            Some(Ok(image)) => {
                let size = image.size(0);
                let density = window.scale_factor() * if is_vector(&self.path) { 2. } else { 1. };
                Ok(Some((size.width.0 as f32 / density, size.height.0 as f32 / density)))
            }
        }
    }

    /// Draws a vector image anew for what it is zoomed to; bitmaps cannot do this.
    fn sharpen(&mut self, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        if !is_vector(&self.path) || scale <= 2. {
            self.vector = None;
            self.drawing = None;
            return;
        }
        let wanted = (scale * window.scale_factor() * 2.).ceil() / 2.;
        if self.vector.as_ref().is_some_and(|(s, _)| *s == wanted) || self.drawing.as_ref().is_some_and(|(s, _)| *s == wanted) {
            return;
        }
        let path = self.path.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(120)).await;
            let drawn = cx.background_spawn(async move { render::svg_to_png(&std::fs::read(path).ok()?, wanted) }).await;
            let _ = this.update(cx, |v, cx| {
                v.drawing = None;
                if let Some(png) = drawn {
                    v.vector = Some((wanted, Arc::new(Image::from_bytes(ImageFormat::Png, png))));
                    cx.notify();
                }
            });
        });
        self.drawing = Some((wanted, task));
    }
}

impl Render for ImageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let me = cx.entity();
        let natural = match self.natural(window, cx) {
            Ok(n) => n,
            Err(()) => {
                return unavailable("image", tr("Bild kann nicht angezeigt werden"), tr("Dieses Format lässt sich hier nicht darstellen. Öffne es mit der Standard-App."), &t).into_any_element();
            }
        };
        let measure = {
            let me = me.clone();
            canvas(
                move |bounds, _, cx| {
                    me.update(cx, |v, cx| {
                        if v.view != bounds {
                            v.view = bounds;
                            cx.notify();
                        }
                    })
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        };
        let Some(natural) = natural else {
            return div().size_full().relative().child(measure).flex().items_center().justify_center().child(super::motion::loading(iconsize::L, t.secondary)).into_any_element();
        };
        let scale = self.scale(natural);
        self.sharpen(scale, window, cx);
        let (x, y) = self.place(natural, scale, self.origin);
        let (w, h) = (natural.0 * scale, natural.1 * scale);
        let (vw, vh) = self.view_size();
        let movable = w > vw || h > vh;
        let picture: AnyElement = match &self.vector {
            Some((_, drawn)) => img(drawn.clone()).w_full().h_full().into_any_element(),
            None => img(self.path.clone()).w_full().h_full().into_any_element(),
        };
        let (m1, m2, m3, m4, m5) = (me.clone(), me.clone(), me.clone(), me.clone(), me.clone());
        let bar = div()
            .absolute()
            .bottom(px(space::M))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::XXS))
                    .p(px(space::XS))
                    .rounded(px(radius::CONTROL))
                    .bg(t.material)
                    .border_1()
                    .border_color(t.separator)
                    .shadow_md()
                    .child(Button::new("image-out").ghost().small().icon(menu_icon("zoom-out")).tooltip(tr("Verkleinern")).on_click(move |_, _, cx| m1.update(cx, |v, cx| v.zoom_to(scale * 0.8, None, natural, cx))))
                    .child(div().id("image-percent").w(px(52.)).text_center().text_size(px(font::CALLOUT)).font_features(tabular()).rounded(px(radius::INNER)).cursor_pointer().soft(t.ink.opacity(0.06), radius::INNER).tooltip(tip(tr("Einpassen"))).child(format!("{:.0} %", scale * 100.)).on_click(move |_, _, cx| m2.update(cx, |v, cx| v.reset(cx))))
                    .child(Button::new("image-in").ghost().small().icon(menu_icon("zoom-in")).tooltip(tr("Vergrößern")).on_click(move |_, _, cx| m3.update(cx, |v, cx| v.zoom_to(scale * 1.25, None, natural, cx))))
                    .child(Button::new("image-fit").ghost().small().icon(menu_icon("maximize")).tooltip(tr("Einpassen")).on_click(move |_, _, cx| m4.update(cx, |v, cx| v.reset(cx)))),
            );
        div()
            .id("image-view")
            .track_focus(&self.focus)
            .key_context("ImageView")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(t.content)
            .when(movable, |d| d.cursor(if self.grab.is_some() { CursorStyle::ClosedHand } else { CursorStyle::OpenHand }))
            .child(measure)
            .child(div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h)).child(picture))
            .child(bar)
            .on_key_down(cx.listener(move |v, e: &KeyDownEvent, _, cx| {
                if !zooms(&e.keystroke.modifiers) {
                    return;
                }
                match e.keystroke.key.as_str() {
                    "=" | "+" => v.zoom_to(scale * 1.25, None, natural, cx),
                    "-" => v.zoom_to(scale * 0.8, None, natural, cx),
                    "0" => v.reset(cx),
                    "1" => v.zoom_to(1., None, natural, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(move |v, e: &MouseDownEvent, window, cx| {
                window.focus(&v.focus, cx);
                if e.click_count >= 2 {
                    if v.zoom.is_some() {
                        v.reset(cx);
                    } else {
                        v.zoom_to(1., Some(e.position), natural, cx);
                    }
                    return;
                }
                let origin = v.place(natural, v.scale(natural), v.origin);
                v.grab = Some((e.position, origin));
                cx.notify();
            }))
            .on_mouse_move(cx.listener(move |v, e: &MouseMoveEvent, _, cx| {
                if let Some((start, origin)) = v.grab {
                    if !e.dragging() {
                        v.grab = None;
                    } else if v.zoom.is_some() || v.fit(natural) < 1. {
                        let scale = v.scale(natural);
                        v.zoom = Some(scale);
                        v.origin = v.place(natural, scale, (origin.0 + f32::from(e.position.x - start.x), origin.1 + f32::from(e.position.y - start.y)));
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, cx| {
                v.grab = None;
                cx.notify();
            }))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|v, _, _, cx| {
                v.grab = None;
                cx.notify();
            }))
            .on_scroll(cx.listener(move |v, e: &ScrollEvent, _, cx| {
                let delta = e.delta.pixel_delta(px(20.));
                if v.zoom.is_some() {
                    let scale = v.scale(natural);
                    let now = v.place(natural, scale, v.origin);
                    v.origin = v.place(natural, scale, (now.0 + f32::from(delta.x), now.1 + f32::from(delta.y)));
                    cx.notify();
                }
            }))
            .on_pinch(cx.listener(move |v, e: &PinchEvent, _, cx| {
                let scale = v.scale(natural);
                v.zoom_to(scale * (1. + e.delta), Some(e.position), natural, cx);
            }))
            .child(wheel_zoom(move |factor, at, _, cx| {
                m5.update(cx, |v, cx| {
                    let scale = v.scale(natural);
                    v.zoom_to(scale * factor, Some(at), natural, cx);
                })
            }))
            .into_any_element()
    }
}
