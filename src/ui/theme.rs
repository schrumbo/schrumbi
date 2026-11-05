use gpui_kit::component::{Theme as ComponentTheme, ThemeMode};
use gpui_kit::{App, Global, Hsla, Window, WindowAppearance, px, rgb, Rgba, black, white};

use crate::core::prefs::{self, ThemeChoice};

/// The palettes the app can be drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Light,
    Dark,
    Nord,
    Dracula,
}

/// Spacing scale for padding, margins and gaps.
pub mod space {
    /// Optical inset inside chips and between stacked text lines.
    pub const XXS: f32 = 2.;
    pub const XS: f32 = 4.;
    pub const S: f32 = 8.;
    pub const M: f32 = 12.;
    pub const L: f32 = 16.;
    pub const XL: f32 = 24.;
    pub const XXL: f32 = 32.;
}

/// Icon sizes.
pub mod iconsize {
    pub const XS: f32 = 12.;
    pub const S: f32 = 14.;
    pub const M: f32 = 16.;
    pub const L: f32 = 20.;
    /// Empty states.
    pub const HERO: f32 = 32.;
    /// File previews.
    pub const PREVIEW: f32 = 64.;
}

/// Control heights.
pub mod control {
    /// Where the macOS window buttons sit in secondary windows.
    pub const TRAFFIC_INSET: f32 = 14.;
    pub const S: f32 = 24.;
    pub const M: f32 = 28.;
    /// The toolbar band.
    pub const BAR: f32 = 52.;
    /// The band under the toolbar. Every pane uses it, so the lines below them meet.
    pub const SUBBAR: f32 = 40.;
    /// The head of a pane that shows a file beside a list.
    pub const PANE_HEAD: f32 = 67.;
    /// The band with a pane's main actions at its bottom.
    pub const FOOTBAR: f32 = 60.;
    /// A button that shows a count, wide enough for two digits so its row does not move.
    pub const COUNTED: f32 = 104.;
    /// A settings row with a label and a line of explanation.
    pub const ROW: f32 = 40.;
    /// A time field, wide enough for a typed time and how it is read.
    pub const TIME: f32 = 92.;
    /// The widest a control at the end of a settings row gets.
    pub const ROW_CONTROL: f32 = 460.;
    /// The shortcut column of the settings: four key chips, or the reason that replaces them.
    pub const KEYS: f32 = 216.;
    /// The most a row of the shortcut list gives to naming what else uses its keys.
    pub const CLASH: f32 = 240.;
    /// A small icon button, like the reset button at the end of a shortcut row.
    pub const ICON_BUTTON: f32 = M;
}

/// Widths of the document tab row.
pub mod tab {
    /// Narrowest a tab gets before the strip scrolls.
    pub const MIN: f32 = 100.;
    /// Narrowest the shown tab gets while the row has room for it.
    pub const ACTIVE_MIN: f32 = 168.;
    /// Widest a tab title is drawn.
    pub const TITLE_MAX: f32 = 180.;
    /// Narrowest row that still shows the list of all tabs; below, the row keeps its room for the shown tab.
    pub const LIST_MIN_BAR: f32 = 360.;
    /// Title width a split pane's row keeps while its actions are comfortable.
    pub const SPLIT_TITLE: f32 = 120.;
    /// Title width a split pane's row keeps at the least.
    pub const SPLIT_TITLE_TIGHT: f32 = 64.;
    /// Width of the split grip; it lies right of the divider so the left pane's scrollbar stays free.
    pub const SPLIT_GRIP: f32 = 12.;
}

/// Widths in the toolbar's row for the open note: folders, name and deadline.
pub mod crumb {
    /// Narrowest the note's name gets before the row gives up showing it.
    pub const NAME_MIN: f32 = 64.;
    /// Narrowest the deadline gets: its icon, an ellipsis and the menu caret.
    pub const PILL_MIN: f32 = 56.;
    /// Widest a folder in the row gets.
    pub const FOLDER_MAX: f32 = 160.;
    /// Widest the deadline gets.
    pub const PILL_MAX: f32 = 240.;
    /// Room the name's bold glyphs and rounded hover need beyond its text and padding.
    pub const NAME_SLACK: f32 = 2.;
    /// Round buttons and gaps in the toolbar row that share its width with the title.
    pub const TOOLBAR_BUTTONS: f32 = 4.;
    pub const TOOLBAR_GAPS: f32 = 5.;
    /// The border between the sidebar and the toolbar.
    pub const BORDER: f32 = 1.;
}

/// Corner radii used across the app.
pub mod radius {
    /// Keyboard chips, small marks.
    pub const SMALL: f32 = 4.;
    /// Buttons, inputs, nested rows.
    pub const INNER: f32 = 6.;
    /// Chips, deadline cards, inline controls.
    pub const CONTROL: f32 = 8.;
    /// Cards, tiles, grouped containers.
    pub const CARD: f32 = 12.;
    /// Capsules and circles.
    pub const FULL: f32 = 999.;
}

/// The ink opacity the small controls floating over a note light up with under the pointer.
pub mod hover {
    pub const FLOAT: f32 = 0.1;
}

/// The fill a control gets while the keyboard has the focus.
pub mod ring {
    use gpui_kit::Hsla;
    use std::sync::RwLock;

    const OPACITY: f32 = 0.28;
    /// The tint of a shortcut chip that is waiting for keys.
    pub const ARMED: f32 = 0.08;
    /// The edge of a shortcut chip, clear while it has no state to show.
    pub const BORDER: f32 = 1.5;

    static ACCENT: RwLock<Hsla> = RwLock::new(Hsla { h: 0.58, s: 1., l: 0.5, a: 1. });

    /// The focus fill for a theme with this accent.
    pub fn of(accent: Hsla) -> Hsla {
        accent.opacity(OPACITY)
    }

    /// Remembers the accent of the theme now drawn, so `fill` follows it.
    pub fn set(accent: Hsla) {
        *ACCENT.write().unwrap_or_else(|e| e.into_inner()) = accent;
    }

    pub fn fill() -> Hsla {
        of(*ACCENT.read().unwrap_or_else(|e| e.into_inner()))
    }
}

/// macOS text styles in points.
pub mod font {
    pub const LARGE_TITLE: f32 = 26.;
    pub const TITLE: f32 = 22.;
    pub const TITLE2: f32 = 17.;
    pub const TITLE3: f32 = 15.;
    pub const BODY: f32 = 13.;
    pub const CALLOUT: f32 = 12.;
    pub const CAPTION: f32 = 11.;
    pub const CAPTION2: f32 = 10.;
    pub const LINE: f32 = 1.22;
    /// Line height of small running text, like a search snippet.
    pub const LINE_LOOSE: f32 = 1.45;
    /// The label of a kit small button, 0.875 rem.
    pub const BUTTON: f32 = 14.;
}

static NOTE_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1065353216);

/// How much bigger than the default the text of notes is, in all ways of showing them.
pub fn note_scale() -> f32 {
    f32::from_bits(NOTE_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

/// Remembers the note font size chosen in the settings.
pub fn set_note_scale(size: f32) {
    NOTE_SCALE.store((size / font::BODY).to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// Draws something at a larger note size for a moment, like the text of a flashcard.
pub fn with_note_scale<R>(factor: f32, draw: impl FnOnce() -> R) -> R {
    let before = NOTE_SCALE.load(std::sync::atomic::Ordering::Relaxed);
    NOTE_SCALE.store((f32::from_bits(before) * factor).to_bits(), std::sync::atomic::Ordering::Relaxed);
    let out = draw();
    NOTE_SCALE.store(before, std::sync::atomic::Ordering::Relaxed);
    out
}

/// A size from the type scale, as written in notes.
pub fn note_size(base: f32) -> f32 {
    (base * note_scale() * 2.).round() / 2.
}

/// Colors resolved for light or dark appearance, mirroring the SwiftUI semantic colors.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub kind: Kind,
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
    /// The accent for text, which needs more contrast than fills do.
    pub accent_text: Hsla,
    /// Green that is readable as text on the content background.
    pub green_text: Hsla,
    /// Text and icons on a solid accent fill.
    pub on_accent: Hsla,
    pub selection: Hsla,
    pub green: Hsla,
    pub blue: Hsla,
    pub orange: Hsla,
    /// Orange that is readable as text on the content background.
    pub orange_text: Hsla,
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
    pub fn new(kind: Kind) -> Self {
        match kind {
            Kind::Light | Kind::Dark => Self::plain(kind == Kind::Dark),
            Kind::Nord => Self::nord(),
            Kind::Dracula => Self::dracula(),
        }
    }

    fn plain(dark: bool) -> Self {
        let ink = if dark { c(0xFFFFFF) } else { c(0x000000) };
        Self {
            kind: if dark { Kind::Dark } else { Kind::Light },
            dark,
            sidebar: if dark { c(0x1D1D1D) } else { c(0xF3F3F5) },
            sidebar_border: ink.opacity(if dark { 0.08 } else { 0.06 }),
            content: if dark { c(0x222222) } else { c(0xFFFFFF) },
            secondary_bg: if dark { c(0x2A2A2A) } else { c(0xF5F5F7) },
            column: if dark { c(0x262626) } else { c(0xF7F7F9) },
            primary: ink.opacity(0.88),
            secondary: ink.opacity(if dark { 0.6 } else { 0.62 }),
            tertiary: ink.opacity(if dark { 0.47 } else { 0.56 }),
            separator: ink.opacity(if dark { 0.12 } else { 0.1 }),
            accent: if dark { c(0x0A84FF) } else { c(0x007AFF) },
            accent_text: if dark { c(0x6CB3FF) } else { c(0x0053B4) },
            green_text: if dark { c(0x30D158) } else { c(0x248A3D) },
            on_accent: c(0xFFFFFF),
            selection: if dark { c(0x3172D4) } else { c(0x0A6CF0) },
            green: if dark { c(0x30D158) } else { c(0x34C759) },
            blue: if dark { c(0x0A84FF) } else { c(0x007AFF) },
            orange: if dark { c(0xFF9F0A) } else { c(0xFF9500) },
            orange_text: if dark { c(0xFF9F0A) } else { c(0x824800) },
            red: if dark { c(0xFF453A) } else { c(0xFF3B30) },
            gray: c(0x8E8E93),
            ink,
            material: if dark { c(0x303030) } else { c(0xFBFBFC) },
        }
    }

    /// Nord: Polar Night for the surfaces, Snow Storm for the text, Frost for what can be used, Aurora for states.
    fn nord() -> Self {
        let ink = c(0xECEFF4);
        Self {
            kind: Kind::Nord,
            dark: true,
            sidebar: c(0x292E39),
            sidebar_border: ink.opacity(0.07),
            content: c(0x2E3440),
            secondary_bg: c(0x3B4252),
            column: c(0x2B313C),
            primary: ink.opacity(0.92),
            secondary: ink.opacity(0.66),
            tertiary: ink.opacity(0.5),
            separator: ink.opacity(0.11),
            accent: c(0x5E81AC),
            accent_text: c(0xA4CFDC),
            green_text: c(0xA3BE8C),
            on_accent: c(0xECEFF4),
            selection: c(0x4C6A92),
            green: c(0xA3BE8C),
            blue: c(0x81A1C1),
            orange: c(0xD08770),
            orange_text: c(0xF5BCA8),
            red: c(0xBF616A),
            gray: c(0x8591A8),
            ink,
            material: c(0x3B4252),
        }
    }

    /// Dracula: its own background family, purple as the accent, the signature colours for states.
    fn dracula() -> Self {
        let ink = c(0xF8F8F2);
        Self {
            kind: Kind::Dracula,
            dark: true,
            sidebar: c(0x21222C),
            sidebar_border: ink.opacity(0.07),
            content: c(0x282A36),
            secondary_bg: c(0x343746),
            column: c(0x252733),
            primary: ink.opacity(0.94),
            secondary: ink.opacity(0.68),
            tertiary: ink.opacity(0.5),
            separator: ink.opacity(0.11),
            accent: c(0xBD93F9),
            accent_text: c(0xDBC4FC),
            green_text: c(0x50FA7B),
            on_accent: c(0x21222C),
            selection: c(0x6A5A9C),
            green: c(0x50FA7B),
            blue: c(0x8BE9FD),
            orange: c(0xFFB86C),
            orange_text: c(0xFFC282),
            red: c(0xFF5555),
            gray: c(0x8A94B8),
            ink,
            material: c(0x3A3D4E),
        }
    }

    pub fn get(cx: &App) -> Self {
        *cx.global::<Theme>()
    }

    /// A color moved towards the text color until it reads as text on this theme's backgrounds.
    pub fn readable(&self, color: Hsla) -> Hsla {
        let mut text = color;
        for step in 1..=10 {
            if contrast(text, self.column.blend(color.opacity(0.12))) >= 4.5 {
                break;
            }
            text = color.blend(self.ink.opacity(step as f32 * 0.1));
        }
        text
    }

    /// Black or white, whichever reads on a solid fill of `color`.
    pub fn on_fill(&self, color: Hsla) -> Hsla {
        if luminance(over(color, color)) > 0.4 { black() } else { white() }
    }

    /// A count badge's fill, deepened until white digits on it read.
    pub fn badge_fill(&self, color: Hsla) -> Hsla {
        let mut fill = color;
        for step in 1..=10 {
            if contrast(white(), fill) >= 4.5 {
                break;
            }
            fill = color.blend(black().opacity(step as f32 * 0.05));
        }
        fill
    }

    /// A color stored in a document, shown as it is in both themes.
    pub fn document(color: u32) -> Hsla {
        rgb(color).into()
    }

    /// Module color by palette name, as in the Swift `Palette`.
    pub fn palette(&self, name: &str) -> Hsla {
        if let Some(color) = self.palette_of_theme(name) {
            return c(color);
        }
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

    fn palette_of_theme(&self, name: &str) -> Option<u32> {
        match self.kind {
            Kind::Light | Kind::Dark => None,
            Kind::Nord => Some(match name {
                "green" => 0xA3BE8C,
                "orange" => 0xD08770,
                "purple" => 0xB48EAD,
                "pink" => 0xC98BA6,
                "teal" => 0x8FBCBB,
                "red" => 0xBF616A,
                "yellow" => 0xEBCB8B,
                "indigo" => 0x7E8FC9,
                "mint" => 0x9FD1C7,
                "brown" => 0xA58D76,
                "gray" => 0x8591A8,
                _ => 0x81A1C1,
            }),
            Kind::Dracula => Some(match name {
                "green" => 0x50FA7B,
                "orange" => 0xFFB86C,
                "purple" => 0xBD93F9,
                "pink" => 0xFF79C6,
                "teal" => 0x8BE9FD,
                "red" => 0xFF5555,
                "yellow" => 0xF1FA8C,
                "indigo" => 0x9A8CFF,
                "mint" => 0x7AF0C8,
                "brown" => 0xC9A58A,
                "gray" => 0x8A94B8,
                _ => 0x6CB6FF,
            }),
        }
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

/// The palette the choice in the settings means right now.
pub fn resolve(choice: ThemeChoice, system_dark: bool) -> Kind {
    match choice {
        ThemeChoice::System => if system_dark { Kind::Dark } else { Kind::Light },
        ThemeChoice::Light => Kind::Light,
        ThemeChoice::Dark => Kind::Dark,
        ThemeChoice::Nord => Kind::Nord,
        ThemeChoice::Dracula => Kind::Dracula,
    }
}

/// Draws the app in what the settings say, following the system where they ask for that.
pub fn refresh(window: &mut Window, cx: &mut App) {
    let kind = resolve(prefs::get().theme, is_dark(window.appearance()));
    apply(kind, Some(window), cx);
}

/// Switches the theme now, in every window. Native parts of the window follow.
pub fn choose(choice: ThemeChoice, window: &mut Window, cx: &mut App) {
    prefs::update(|p| p.theme = choice);
    super::platform::set_window_appearance(match choice {
        ThemeChoice::System => None,
        ThemeChoice::Light => Some(false),
        _ => Some(true),
    });
    refresh(window, cx);
    cx.refresh_windows();
}

/// Sets the app palette and maps it onto the component library so its controls match.
pub fn apply(kind: Kind, window: Option<&mut Window>, cx: &mut App) {
    let t = Theme::new(kind);
    let dark = t.dark;
    cx.set_global(t);
    ring::set(t.accent);
    set_note_scale(crate::core::prefs::get().editor_font_size);
    ComponentTheme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, window, cx);
    let ct = ComponentTheme::global_mut(cx);
    ct.font_family = Theme::font_family().into();
    ct.font_size = px(font::BODY);
    ct.mono_font_family = Theme::mono_family().into();
    ct.mono_font_size = px(crate::core::prefs::get().editor_font_size);
    let mut highlight = (*ct.highlight_theme).clone();
    highlight.style.editor_background = Some(t.content);
    highlight.style.editor_active_line = Some(t.ink.opacity(0.05));
    apply_syntax_colors(&mut highlight, kind);
    ct.highlight_theme = std::sync::Arc::new(highlight);
    ct.motion.duration_slow = super::motion::SLOW;
    ct.scrollbar_mode = gpui_kit::base::ScrollbarMode::Hover;
    ct.radius = px(radius::INNER);
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
    k.primary_foreground = t.on_accent;
    k.button_primary = t.accent;
    k.button_primary_hover = t.accent.blend(gpui_kit::white().opacity(0.08));
    k.button_primary_active = t.accent.blend(gpui_kit::black().opacity(0.12));
    k.button_primary_foreground = t.on_accent;
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
    k.switch = t.ink.opacity(if dark { 0.22 } else { 0.15 });
    k.switch_thumb = gpui_kit::white();
    k.list_active = t.accent.opacity(0.15);
    k.list_hover = t.ink.opacity(0.06);
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

fn over(top: Hsla, bottom: Hsla) -> Rgba {
    let (t, b) = (Rgba::from(top), Rgba::from(bottom));
    let mix = |a: f32, z: f32| a * t.a + z * (1. - t.a);
    Rgba { r: mix(t.r, b.r), g: mix(t.g, b.g), b: mix(t.b, b.b), a: 1. }
}

fn luminance(c: Rgba) -> f32 {
    let f = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b)
}

fn contrast(text: Hsla, background: Hsla) -> f32 {
    let (a, b) = (luminance(over(text, background)), luminance(over(background, background)));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::Rgba;

    #[test]
    fn badge_digits_read_on_every_theme() {
        for kind in [Kind::Light, Kind::Dark, Kind::Nord, Kind::Dracula] {
            let t = Theme::new(kind);
            for color in [t.red, t.accent] {
                assert!(contrast(white(), t.badge_fill(color)) >= 4.5, "{kind:?}");
            }
        }
    }

    #[test]
    fn text_is_readable_on_every_background() {
        for kind in [Kind::Light, Kind::Dark, Kind::Nord, Kind::Dracula] {
            let dark = kind;
            let t = Theme::new(kind);
            let ring = ring::of(t.accent);
            for background in [t.content, t.sidebar, t.secondary_bg, t.column] {
                let focused = background.blend(ring);
                let armed = background.blend(t.accent.opacity(ring::ARMED));
                assert!(contrast(t.orange_text, armed) >= 4.5, "orange text on the armed chip, {dark:?}: {}", contrast(t.orange_text, armed));
                assert!(contrast(t.accent_text, armed) >= 4.5, "accent text on the armed chip, {dark:?}: {}", contrast(t.accent_text, armed));
                assert!(contrast(t.orange_text, focused) >= 4.5, "orange text on the focus fill, {dark:?}: {}", contrast(t.orange_text, focused));
                assert!(contrast(t.accent_text, focused) >= 4.5, "accent text on the focus fill, {dark:?}: {}", contrast(t.accent_text, focused));
                assert!(contrast(t.primary, focused) >= 4.5, "text on the focus fill, {dark:?}: {}", contrast(t.primary, focused));
                assert!(contrast(focused, background) >= 1.25, "the focus fill shows on the background, {dark:?}: {}", contrast(focused, background));
                assert!(contrast(t.primary, background) >= 7., "primary, {dark:?}");
                assert!(contrast(t.secondary, background) >= 4.5, "secondary, {dark:?}: {}", contrast(t.secondary, background));
                assert!(contrast(t.accent_text, background) >= 4.5, "accent, {dark:?}: {}", contrast(t.accent_text, background));
                assert!(contrast(t.orange_text, background) >= 4.5, "orange text, {dark:?}: {}", contrast(t.orange_text, background));
                for name in ["blue", "green", "orange", "purple", "pink", "teal", "red", "yellow", "indigo", "mint", "brown", "gray"] {
                    let color = t.palette(name);
                    assert!(contrast(t.readable(color), t.column.blend(color.opacity(0.12))) >= 4.5, "{name}, {dark:?}");
                }
                assert!(contrast(t.tertiary, background) >= 3., "tertiary, {dark:?}: {}", contrast(t.tertiary, background));
                let p = syntax_palette(kind);
                for color in [p.keyword, p.string, p.comment, p.number, p.kind, p.function] {
                    if background == t.content {
                        assert!(contrast(c(color), background) >= 4.5, "syntax {color:06x}, {dark:?}");
                    }
                }
            }
            assert!(contrast(t.on_accent, t.accent) >= 3., "on accent, {dark:?}: {}", contrast(t.on_accent, t.accent));
        }
    }
}

/// The colours of written code, in light and dark. Each stays readable on the note's background.
pub struct SyntaxPalette {
    pub keyword: u32,
    pub string: u32,
    pub comment: u32,
    pub number: u32,
    pub kind: u32,
    pub function: u32,
}

impl SyntaxPalette {
    /// The colour of a token, none for plain text.
    pub fn of(&self, kind: crate::core::markdown::TokenKind) -> Option<Hsla> {
        use crate::core::markdown::TokenKind;
        let c = match kind {
            TokenKind::Plain => return None,
            TokenKind::Keyword => self.keyword,
            TokenKind::String => self.string,
            TokenKind::Comment => self.comment,
            TokenKind::Number => self.number,
            TokenKind::Type => self.kind,
            TokenKind::Attribute => self.function,
        };
        Some(rgb(c).into())
    }
}

pub fn syntax_palette(kind: Kind) -> SyntaxPalette {
    match kind {
        Kind::Dark => SyntaxPalette { keyword: 0xB7A8F8, string: 0x8FCB9B, comment: 0x8E939B, number: 0xE5A76B, kind: 0x7CC7D6, function: 0x82AAF5 },
        Kind::Light => SyntaxPalette { keyword: 0x6A4BC4, string: 0x2B7A4B, comment: 0x68707C, number: 0xA3560F, kind: 0x0B7285, function: 0x1F5FCC },
        Kind::Nord => SyntaxPalette { keyword: 0x81A1C1, string: 0xA3BE8C, comment: 0x98A3B8, number: 0xC9A8C2, kind: 0x8FBCBB, function: 0x88C0D0 },
        Kind::Dracula => SyntaxPalette { keyword: 0xFF79C6, string: 0xF1FA8C, comment: 0x8D9AD0, number: 0xBD93F9, kind: 0x8BE9FD, function: 0x50FA7B },
    }
}

fn apply_syntax_colors(theme: &mut gpui_kit::component::highlighter::HighlightTheme, kind: Kind) {
    use gpui_kit::component::highlighter::ThemeStyle;
    let p = syntax_palette(kind);
    let paint = |color: u32| serde_json::from_value::<ThemeStyle>(serde_json::json!({ "color": format!("#{color:06x}") })).ok();
    let s = &mut theme.style.syntax;
    s.keyword = paint(p.keyword);
    s.preproc = paint(p.keyword);
    s.string = paint(p.string);
    s.string_escape = paint(p.number);
    s.string_regex = paint(p.string);
    s.string_special = paint(p.string);
    s.string_special_symbol = paint(p.string);
    s.comment = paint(p.comment);
    s.comment_doc = paint(p.comment);
    s.number = paint(p.number);
    s.boolean = paint(p.number);
    s.constant = paint(p.number);
    s.function = paint(p.function);
    s.constructor = paint(p.kind);
    s.enum_ = paint(p.kind);
    s.tag = paint(p.keyword);
    s.attribute = paint(p.function);
    s.property = paint(p.function);
    s.link_text = paint(p.function);
    s.link_uri = paint(p.comment);
    s.text_code_span = paint(p.string);
    s.punctuation = paint(p.comment);
    s.punctuation_list_marker = paint(p.comment);
    s.punctuation_special = paint(p.comment);
    s.punctuation_delimiter = paint(p.comment);
    s.title = paint(p.function);
    s.text_literal = paint(p.string);
    s.type_ = paint(p.kind);
    s.variant = paint(p.kind);
    s.label = paint(p.number);
    s.operator = paint(p.comment);
    s.emphasis = None;
    s.emphasis_strong = None;
    s.primary = None;
}
