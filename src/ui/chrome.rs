use gpui_kit::component::TitleBar;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::theme::{Theme, font};

/// Whether the app draws its own title bar with window buttons. macOS keeps its traffic lights.
pub const CUSTOM: bool = !cfg!(target_os = "macos");

/// Prepares a window for the custom title bar; on macOS the options stay as they are.
pub fn window_options(options: WindowOptions, title: impl Into<SharedString>) -> WindowOptions {
    let options = WindowOptions { focus: options.focus && !crate::core::drive::on(), ..options };
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
    if !CUSTOM {
        return super::perf::window(content.into_any_element());
    }
    super::perf::window(div().size_full().flex().flex_col().child(secondary_bar(title.into(), t)).child(div().flex_1().min_h_0().child(content)).into_any_element())
}

/// Like `frame`, but the close button runs `close` instead of just removing the window.
pub fn frame_with_close(title: impl Into<SharedString>, content: impl IntoElement, close: impl Fn(&mut Window, &mut App) + 'static, t: &Theme) -> AnyElement {
    if !CUSTOM {
        return super::perf::window(content.into_any_element());
    }
    let bar = secondary_bar(title.into(), t).on_close_window(move |_, window, cx| close(window, cx));
    super::perf::window(div().size_full().flex().flex_col().child(bar).child(div().flex_1().min_h_0().child(content)).into_any_element())
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
