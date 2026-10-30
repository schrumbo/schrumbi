use gpui_kit::component::{Theme as ComponentTheme, ThemeMode};
use gpui_kit::{App, Global, Hsla, Window, WindowAppearance, px, rgb};

/// Corner radii used across the app.
pub mod radius {
    /// Cards, tiles, grouped containers.
    pub const CARD: f32 = 12.;
    /// Chips, deadline cards, inline controls.
    pub const CONTROL: f32 = 8.;
}

/// macOS text styles in points.
pub mod font {
    pub const LARGE_TITLE: f32 = 26.;
    pub const TITLE: f32 = 22.;
    pub const TITLE2: f32 = 17.;
    pub const TITLE3: f32 = 15.;
    pub const HEADLINE: f32 = 13.;
    pub const BODY: f32 = 13.;
    pub const CALLOUT: f32 = 12.;
    pub const SUBHEADLINE: f32 = 11.;
    pub const CAPTION: f32 = 11.;
    pub const CAPTION2: f32 = 10.;
}

/// Colors resolved for light or dark appearance, mirroring the SwiftUI semantic colors.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub sidebar: Hsla,
    pub sidebar_border: Hsla,
    pub content: Hsla,
    pub secondary_bg: Hsla,
    pub column: Hsla,
    pub primary: Hsla,
    pub secondary: Hsla,
    pub tertiary: Hsla,
    pub separator: Hsla,
    pub accent: Hsla,
    pub selection: Hsla,
    pub green: Hsla,
    pub blue: Hsla,
    pub orange: Hsla,
    pub red: Hsla,
    pub gray: Hsla,
    pub ink: Hsla,
    pub material: Hsla,
}

impl Global for Theme {}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Theme {
    pub fn new(dark: bool) -> Self {
        let ink = if dark { c(0xFFFFFF) } else { c(0x000000) };
        Self {
            dark,
            sidebar: if dark { c(0x1D1D1D) } else { c(0xF3F3F5) },
            sidebar_border: ink.opacity(if dark { 0.08 } else { 0.06 }),
            content: if dark { c(0x222222) } else { c(0xFFFFFF) },
            secondary_bg: if dark { c(0x2A2A2A) } else { c(0xF5F5F7) },
            column: if dark { c(0x262626) } else { c(0xF7F7F9) },
            primary: ink.opacity(0.88),
            secondary: ink.opacity(if dark { 0.55 } else { 0.5 }),
            tertiary: ink.opacity(0.26),
            separator: ink.opacity(if dark { 0.12 } else { 0.1 }),
            accent: if dark { c(0x0A84FF) } else { c(0x007AFF) },
            selection: if dark { c(0x3172D4) } else { c(0x0A6CF0) },
            green: if dark { c(0x30D158) } else { c(0x34C759) },
            blue: if dark { c(0x0A84FF) } else { c(0x007AFF) },
            orange: if dark { c(0xFF9F0A) } else { c(0xFF9500) },
            red: if dark { c(0xFF453A) } else { c(0xFF3B30) },
            gray: c(0x8E8E93),
            ink,
            material: if dark { c(0x303030) } else { c(0xFBFBFC) },
        }
    }

    pub fn get(cx: &App) -> Self {
        *cx.global::<Theme>()
    }

    /// Module color by palette name, as in the Swift `Palette`.
    pub fn palette(&self, name: &str) -> Hsla {
        let (light, dark) = match name {
            "green" => (0x34C759, 0x30D158),
            "orange" => (0xFF9500, 0xFF9F0A),
            "purple" => (0xAF52DE, 0xBF5AF2),
            "pink" => (0xFF2D55, 0xFF375F),
            "teal" => (0x30B0C7, 0x40C8E0),
            "red" => (0xFF3B30, 0xFF453A),
            "yellow" => (0xFFCC00, 0xFFD60A),
            "indigo" => (0x5856D6, 0x5E5CE6),
            "mint" => (0x00C7BE, 0x63E6E2),
            "brown" => (0xA2845E, 0xAC8E68),
            "gray" => (0x8E8E93, 0x8E8E93),
            _ => (0x007AFF, 0x0A84FF),
        };
        c(if self.dark { dark } else { light })
    }

    /// The platform UI font.
    pub fn font_family() -> &'static str {
        if cfg!(target_os = "macos") {
            ".SystemUIFont"
        } else if cfg!(target_os = "windows") {
            "Segoe UI"
        } else {
            "Inter"
        }
    }

    pub fn mono_family() -> &'static str {
        if cfg!(target_os = "macos") {
            "Menlo"
        } else if cfg!(target_os = "windows") {
            "Consolas"
        } else {
            "DejaVu Sans Mono"
        }
    }
}

pub fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

/// Sets the app palette and maps it onto the component library so its controls match.
pub fn apply(dark: bool, window: Option<&mut Window>, cx: &mut App) {
    let t = Theme::new(dark);
    cx.set_global(t);
    ComponentTheme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, window, cx);
    let ct = ComponentTheme::global_mut(cx);
    ct.font_family = Theme::font_family().into();
    ct.font_size = px(font::BODY);
    ct.mono_font_family = Theme::mono_family().into();
    ct.mono_font_size = px(crate::core::prefs::get().editor_font_size);
    let mut highlight = (*ct.highlight_theme).clone();
    highlight.style.editor_background = Some(t.content);
    highlight.style.editor_active_line = Some(t.ink.opacity(0.05));
    ct.highlight_theme = std::sync::Arc::new(highlight);
    ct.radius = px(6.);
    ct.radius_lg = px(radius::CARD);
    let k = &mut ct.colors;
    k.background = t.content;
    k.foreground = t.primary;
    k.border = t.separator;
    k.input = t.ink.opacity(if dark { 0.14 } else { 0.16 });
    k.ring = t.accent.opacity(0.5);
    k.primary = t.accent;
    k.primary_hover = t.accent.opacity(0.9);
    k.primary_active = t.accent.opacity(0.8);
    k.primary_foreground = gpui_kit::white();
    k.button_primary = t.accent;
    k.button_primary_hover = t.accent.blend(gpui_kit::white().opacity(0.08));
    k.button_primary_active = t.accent.blend(gpui_kit::black().opacity(0.12));
    k.button_primary_foreground = gpui_kit::white();
    k.button = t.ink.opacity(if dark { 0.1 } else { 0.05 });
    k.button_hover = t.ink.opacity(if dark { 0.14 } else { 0.08 });
    k.button_active = t.ink.opacity(0.18);
    k.button_foreground = t.primary;
    k.button_danger = t.red;
    k.button_danger_foreground = gpui_kit::white();
    k.secondary = t.ink.opacity(if dark { 0.1 } else { 0.05 });
    k.secondary_hover = t.ink.opacity(if dark { 0.14 } else { 0.08 });
    k.secondary_active = t.ink.opacity(0.18);
    k.secondary_foreground = t.primary;
    k.muted = t.secondary_bg;
    k.muted_foreground = t.secondary;
    k.accent = t.ink.opacity(0.06);
    k.accent_foreground = t.primary;
    k.popover = t.material;
    k.popover_foreground = t.primary;
    k.selection = t.accent.opacity(0.3);
    k.caret = t.accent;
    k.link = t.accent;
    k.link_hover = t.accent.opacity(0.85);
    k.link_active = t.accent.opacity(0.7);
    k.switch = t.ink.opacity(0.15);
    k.list_active = t.accent.opacity(0.15);
    k.list_hover = t.ink.opacity(0.05);
    k.tab_bar = t.secondary_bg;
    k.tab_active = t.content;
    k.danger = t.red;
    k.success = t.green;
    k.warning = t.orange;
    k.info = t.blue;
    k.overlay = gpui_kit::black().opacity(if dark { 0.45 } else { 0.25 });
    k.scrollbar_thumb = t.ink.opacity(0.25);
    k.scrollbar_thumb_hover = t.ink.opacity(0.4);
    k.title_bar = t.sidebar;
    k.title_bar_border = t.separator;
    k.window_border = t.separator;
    ct.tokens = ct.colors.into();
}
