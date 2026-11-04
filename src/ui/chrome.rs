use gpui_kit::component::TitleBar;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::theme::{Theme, font};

/// Whether the app draws its own title bar with window buttons. macOS keeps its traffic lights.
pub const CUSTOM: bool = !cfg!(target_os = "macos");

/// Prepares a window for the custom title bar; on macOS the options stay as they are.
pub fn window_options(options: WindowOptions, title: impl Into<SharedString>) -> WindowOptions {
    let driven = crate::core::drive::on();
    let options = WindowOptions {
        focus: options.focus && !driven,
        inactive_frame_interval: if driven { None } else { options.inactive_frame_interval },
        ..options
    };
    if !CUSTOM {
        return options;
    }
    let bar = TitleBar::window_options();
    WindowOptions {
        titlebar: Some(TitlebarOptions { title: Some(title.into()), ..TitleBar::title_bar_options() }),
        app_owns_titlebar_drag: bar.app_owns_titlebar_drag,
        window_decorations: cfg!(target_os = "linux").then_some(WindowDecorations::Client),
        ..options
    }
}

/// The title bar of a secondary window: its title and the window buttons.
fn secondary_bar(title: SharedString, t: &Theme) -> TitleBar {
    TitleBar::new().child(div().truncate().text_size(px(font::CALLOUT)).text_color(t.secondary).child(title))
}

/// Puts a window's content below the custom title bar.
pub fn frame(title: impl Into<SharedString>, content: impl IntoElement, t: &Theme) -> AnyElement {
    let title = title.into();
    if !CUSTOM {
        return super::perf::window(content.into_any_element(), title);
    }
    super::perf::window(div().size_full().flex().flex_col().child(secondary_bar(title.clone(), t)).child(div().flex_1().min_h_0().child(content)).into_any_element(), title)
}

/// Like `frame`, but the close button runs `close` instead of just removing the window.
pub fn frame_with_close(title: impl Into<SharedString>, content: impl IntoElement, close: impl Fn(&mut Window, &mut App) + 'static, t: &Theme) -> AnyElement {
    let title = title.into();
    if !CUSTOM {
        return super::perf::window(content.into_any_element(), title);
    }
    let bar = secondary_bar(title.clone(), t).on_close_window(move |_, window, cx| close(window, cx));
    super::perf::window(div().size_full().flex().flex_col().child(bar).child(div().flex_1().min_h_0().child(content)).into_any_element(), title)
}

struct MainMenu(Entity<AppMenuBar>);

impl Global for MainMenu {}

/// The main window's title bar, with the in-window menu bar.
pub fn main_bar(cx: &mut App) -> TitleBar {
    let menu = match cx.try_global::<MainMenu>() {
        Some(m) => m.0.clone(),
        None => {
            let menu = AppMenuBar::new(cx);
            cx.set_global(MainMenu(menu.clone()));
            menu
        }
    };
    TitleBar::new().child(menu)
}

/// Opens a window like `gpui_kit::open_window`, with its content view out of the responder chain.
/// AccessKit swaps that view's class back when a window closes, and AppKit's Touch Bar search, which
/// observes every responder, then fails to stop observing it and aborts the app.
pub fn open_window<V: Render>(options: WindowOptions, cx: &mut App, build: impl FnOnce(&mut Window, &mut App) -> Entity<V>) -> Result<(AnyWindowHandle, Entity<V>)> {
    let opened = gpui_kit::open_window(options, cx, build)?;
    let _ = opened.0.update(cx, |_, window, _| skip_content_view(window));
    Ok(opened)
}

#[cfg(target_os = "macos")]
fn skip_content_view(window: &Window) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(RawWindowHandle::AppKit(h)) = HasWindowHandle::window_handle(window).map(|h| h.as_raw()) else { return };
    let view = h.ns_view.as_ptr().cast::<AnyObject>();
    unsafe {
        let ns_window: *mut AnyObject = msg_send![view, window];
        if !ns_window.is_null() {
            let _: () = msg_send![view, setNextResponder: ns_window];
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn skip_content_view(_: &Window) {}

/// Copies the app's menus into the in-window menu bar.
pub fn sync_menus(cx: &mut App) {
    if !CUSTOM {
        return;
    }
    if let Some(menus) = cx.get_menus() {
        gpui_kit::base::GlobalState::global_mut(cx).set_app_menus(menus);
    }
    if let Some(menu) = cx.try_global::<MainMenu>().map(|m| m.0.clone()) {
        menu.update(cx, |m, cx| m.reload(cx));
    }
}
