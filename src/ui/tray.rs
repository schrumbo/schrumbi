use chrono::Duration;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::{self, act};
use super::theme::{Theme, font, space, iconsize};
use super::widgets::*;
use crate::core::day;
use crate::core::i18n::{tr, trf};
use crate::core::library::Library;
use crate::core::model::Assignment;
use crate::core::prefs;

/// Whether this platform shows a tray icon. Linux would need a GTK main loop next to GPUI's.
pub const AVAILABLE: bool = cfg!(any(target_os = "macos", target_os = "windows"));

fn label(l: &Library) -> String {
    if !l.action_items().is_empty() {
        return format!("{}", l.action_items().len());
    }
    l.next_deadline().map_or(String::new(), |a| format!("{} {}", l.module(a).map_or("", |m| m.code.as_str()), day::menu_bar(a.due, l.now)))
}

#[cfg(target_os = "macos")]
fn set_badge(count: usize) {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    let Some(app_class) = AnyClass::get(c"NSApplication") else { return };
    let Some(string_class) = AnyClass::get(c"NSString") else { return };
    let text = if count > 0 { format!("{count}\0") } else { String::new() };
    unsafe {
        let app: *mut AnyObject = msg_send![app_class, sharedApplication];
        let tile: *mut AnyObject = msg_send![app, dockTile];
        let label: *mut AnyObject = if count > 0 { msg_send![string_class, stringWithUTF8String: text.as_ptr() as *const std::ffi::c_char] } else { std::ptr::null_mut() };
        let _: () = msg_send![tile, setBadgeLabel: label];
    }
}

#[cfg(not(target_os = "macos"))]
fn set_badge(_: usize) {}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native {
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    #[derive(Default)]
    pub struct Native {
        icon: Option<TrayIcon>,
        title: String,
    }

    fn image() -> Option<Icon> {
        let decoded = image::load_from_memory(include_bytes!("../../assets/brand/tray.png")).ok()?.into_rgba8();
        let (w, h) = decoded.dimensions();
        Icon::from_rgba(decoded.into_raw(), w, h).ok()
    }

    impl Native {
        pub fn sync(&mut self, enabled: bool, title: &str) {
            if !enabled {
                self.icon = None;
                self.title.clear();
                return;
            }
            if self.icon.is_none() {
                self.icon = image().and_then(|i| TrayIconBuilder::new().with_icon(i).with_icon_as_template(true).with_tooltip("Schrumbi").build().ok());
            }
            if title != self.title
                && let Some(icon) = &self.icon
            {
                icon.set_title(Some(title));
                self.title = title.to_string();
            }
        }
    }

    /// Sends the icon's bottom-center position in physical pixels on every left click.
    pub fn listen(tx: async_channel::Sender<(f64, f64)>) {
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
                let _ = tx.try_send((rect.position.x + rect.size.width as f64 / 2., rect.position.y + rect.size.height as f64));
            }
        }));
    }
}

#[derive(Default)]
struct Tray {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native: native::Native,
    popup: Option<AnyWindowHandle>,
    closed_at: Option<std::time::Instant>,
}

impl Global for Tray {}

/// Brings the icon, its title and the dock badge in line with the library.
pub fn sync(cx: &mut App) {
    let (title, count) = state::read(cx, |l| (label(l), l.action_items().len()));
    set_badge(count);
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let enabled = prefs::get().tray;
        cx.global_mut::<Tray>().native.sync(enabled, &title);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = title;
}

/// Creates the tray and keeps it up to date.
pub fn init(cx: &mut App) {
    cx.set_global(Tray::default());
    sync(cx);
    let lib = state::lib(cx);
    cx.observe(&lib, |_, cx| sync(cx)).detach();
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let (tx, rx) = async_channel::unbounded();
        native::listen(tx);
        cx.spawn(async move |cx| {
            while let Ok(position) = rx.recv().await {
                cx.update(|cx| toggle(position, cx));
            }
        })
        .detach();
    }
}

/// The popover's height for its current content, capped so it stays a popover.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn content_height(l: &Library) -> f32 {
    const FOOTER: f32 = 56.;
    const DEADLINE: f32 = 106.;
    const SECTION: f32 = 40.;
    const ROW: f32 = 28.;
    const FREE: f32 = 46.;
    let today = day::local_date(l.now);
    let days: Vec<usize> = (0..3).map(|i| l.assignments_on(today + Duration::days(i)).len()).filter(|n| *n > 0).collect();
    let actions = l.action_items().len().min(4);
    let mut height = FOOTER;
    if l.root.is_none() {
        height += FREE;
    } else {
        if l.next_deadline().is_some() {
            height += DEADLINE;
        }
        if actions > 0 {
            height += SECTION + ROW * actions as f32;
        }
        height += days.iter().map(|n| SECTION + ROW * *n as f32).sum::<f32>();
        if days.is_empty() {
            height += FREE;
        }
    }
    height.clamp(170., 560.)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn toggle((x, y): (f64, f64), cx: &mut App) {
    let tray = cx.global::<Tray>();
    let recently_closed = tray.closed_at.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(300));
    if let Some(handle) = tray.popup {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        cx.global_mut::<Tray>().popup = None;
        return;
    }
    if recently_closed {
        return;
    }
    let scale = cx.windows().iter().find_map(|w| w.update(cx, |_, window, _| window.scale_factor()).ok()).unwrap_or(if cfg!(target_os = "macos") { 2. } else { 1. });
    let width = 340.;
    let height = state::read(cx, content_height);
    let origin = point(px((x as f32 / scale - width / 2.).max(0.)), px(y as f32 / scale + 4.));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, size(px(width), px(height))))),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: true,
        ..Default::default()
    };
    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| TrayPopup::new(window, cx))) {
        cx.global_mut::<Tray>().popup = Some(handle);
    }
    cx.activate(true);
}

/// The tray popover: next deadline, what needs action, the next three days.
struct TrayPopup {
    _subs: Vec<Subscription>,
}

impl TrayPopup {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let lib = state::lib(cx);
        Self {
            _subs: vec![
                cx.observe(&lib, |_, _, cx| cx.notify()),
                cx.observe_window_activation(window, |_, window, cx| {
                    if !window.is_window_active() {
                        let tray = cx.global_mut::<Tray>();
                        tray.popup = None;
                        tray.closed_at = Some(std::time::Instant::now());
                        window.remove_window();
                    }
                }),
            ],
        }
    }
}

fn close_popup(window: &mut Window, cx: &mut App) {
    cx.global_mut::<Tray>().popup = None;
    window.remove_window();
}

fn section(title: String, tint: Hsla, items: &[&Assignment], lib: &Library, t: &Theme) -> Div {
    div().flex().flex_col().gap(px(space::S)).px(px(space::L)).py(px(space::S)).child(div().font_weight(FontWeight::SEMIBOLD).child(text(title, font::CAPTION, tint))).children(items.iter().map(|a| {
        let module = lib.module(a);
        div()
            .flex()
            .items_center()
            .gap(px(space::S))
            .child(status_toggle(a, lib, iconsize::M, t))
            .when_some(module, |d, m| d.child(div().flex_none().font_weight(FontWeight::SEMIBOLD).text_size(px(font::CAPTION)).child(module_tag(&m.code, t.palette(&m.color)))))
            .child(div().flex_1().min_w_0().truncate().child(a.title.clone()))
            .child(div().flex_none().text_size(px(font::CAPTION)).child(due_label(lib, a, t)))
    }))
}

impl Render for TrayPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let lib = state::lib(cx);
        let l = lib.read(cx);
        let mut body = div().flex().flex_col();
        if l.root.is_none() {
            body = body.child(div().p(px(space::L)).text_color(t.secondary).child(tr("Kein Ordner gewählt")));
        } else {
            if let Some(a) = l.next_deadline() {
                body = body
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::XS))
                            .p(px(space::L))
                            .child(caption(tr("Nächste Deadline"), &t))
                            .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(font::TITLE2)).child(day::countdown(a.due, l.now)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S))
                                    .text_size(px(font::CALLOUT))
                                    .when_some(l.module(a), |d, m| d.child(module_tag(&m.code, t.palette(&m.color))))
                                    .child(a.title.clone()),
                            ),
                    )
                    .child(divider(&t));
            }
            let actions: Vec<&Assignment> = l.action_items().iter().take(4).collect();
            if !actions.is_empty() {
                body = body.child(section(tr("Erfordert Aktion"), t.palette("red"), &actions, l, &t)).child(divider(&t));
            }
            let today = day::local_date(l.now);
            let days: Vec<_> = (0..3).map(|i| today + Duration::days(i)).collect();
            let mut any = false;
            for d in days {
                let items = l.assignments_on(d);
                if !items.is_empty() {
                    any = true;
                    body = body.child(section(day::day_header(d, today), t.secondary, &items, l, &t));
                }
            }
            if !any {
                body = body.child(div().p(px(space::L)).text_color(t.secondary).child(tr("Die nächsten 3 Tage sind frei.")));
            }
        }
        let inbox = l.inbox.len();
        let accent = t.accent;
        let appear = super::motion::timed(super::motion::MEDIUM, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.content)
            .border_1()
            .border_color(t.separator)
            .text_size(px(font::BODY))
            .text_color(t.primary)
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_1().border_color(accent))
            .on_drop(|paths: &ExternalPaths, _, cx| {
                let paths = paths.paths().to_vec();
                act(cx, |l| l.import_to_inbox(&paths));
            })
            .child(div().id("tray-scroll").flex_1().min_h_0().overflow_y_scrollbar().child(body))
            .child(divider(&t))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .p(px(space::M))
                    .child(Button::new("tray-open").primary().small().label(tr("Schrumbi öffnen")).on_click(|_, window, cx| {
                        close_popup(window, cx);
                        cx.defer(super::main_window::show);
                    }))
                    .when(inbox > 0, |d| d.child(div().id("tray-inbox").flex().items_center().gap(px(space::XS)).text_color(t.secondary).tooltip(tip(tr("Dateien in der Inbox"))).child(icon("inbox", iconsize::S, t.secondary)).child(trf("%lld", &[&inbox]))))
                    .child(div().flex_1())
                    .child(Button::new("tray-quit").ghost().small().label(tr("Beenden")).on_click(|_, _, cx| super::app::quit(cx))),
            )
            .with_animation("tray-appear", Animation::new(appear).with_easing(ease_out_quint()), |d, p| d.opacity(p))
    }
}
