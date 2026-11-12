use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use gpui_kit::component::{Theme as ComponentTheme, ThemeMode};
use gpui_kit::{AnyWindowHandle, App, AppContext, Global, Hsla, Window, WindowAppearance, px, rgb, Rgba, black, white};

use crate::core::i18n::trf;
use crate::core::prefs::{self, ThemeChoice};
use crate::core::themes::{self, MODULE_DEFAULTS, MODULE_NAMES, Palette};

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
    /// The widest a page's content grows before it is centred.
    pub const CONTENT_MAX: f32 = 1120.;
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
    /// Below this toolbar width the empty search field folds to its icon.
    pub const SEARCH_FOLD: f32 = 700.;
    /// The height of a one-line save notice, used until the real one is measured; toasts stand above it.
    pub const SAVE_NOTICE: f32 = 64.;
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

/// The calendar on the overview.
pub mod calendar {
    /// The column with the week numbers of the month grid.
    pub const KW: f32 = 36.;
    /// A row of the month grid.
    pub const ROW: f32 = 104.;
    /// A row of the month grid in a narrow window, where days show dots instead of chips.
    pub const ROW_NARROW: f32 = 60.;
    /// The inset of a day cell.
    pub const CELL_PAD: f32 = 4.;
    /// The day number of a cell; today's sits in a disc of this size.
    pub const DAY: f32 = 22.;
    /// A deadline chip in the month grid.
    pub const CHIP: f32 = 22.;
    /// The module colour at the left edge of a card or chip.
    pub const EDGE: f32 = 3.;
    /// A dot standing for one entry in a narrow month grid.
    pub const DOT: f32 = 6.;
    /// One half of the Woche/Monat switch.
    pub const SEGMENT: f32 = 76.;
    /// How far a page slides in when it turns.
    pub const SLIDE: f32 = 20.;
    /// The least height of a week column, so paging keeps the board's height.
    pub const COLUMN: f32 = 230.;
    /// The width of the list a crowded day opens.
    pub const MORE: f32 = 300.;
    /// The most that list grows before it scrolls.
    pub const MORE_HEIGHT: f32 = 440.;
}

/// The notes explorer beside the open note.
pub mod explorer {
    pub const MIN: f32 = 180.;
    pub const MAX: f32 = 420.;
    /// The room the explorer leaves for the note when the window is narrow.
    pub const NOTE_MIN: f32 = 360.;
}

/// How strongly the pointer lights up what it rests on, as ink opacity; every hover takes one of these and fades in over `motion::FAST`.
pub mod hover {
    /// Rows, list entries, tabs, menu entries and fields.
    pub const ROW: f32 = 0.06;
    /// Icon and text buttons in bars, chips, links and small controls floating over a note.
    pub const CONTROL: f32 = 0.1;
    /// Controls on a surface that is tinted itself, such as a code block or a grouped bar.
    pub const RAISED: f32 = 0.14;
    /// How far a row hover and a row selection stay from the edges of their list.
    pub const INSET: f32 = super::space::XS;
}

/// The orange tints of the notice about a note that did not save.
pub mod tint {
    pub const SAVE_NOTICE: f32 = 0.08;
    pub const SAVE_FLASH: f32 = 0.16;
    /// The fill of a selected row in a list of pages.
    pub const SELECTED: f32 = 0.14;
    /// The edge of a selected row in a list of pages, drawn in the readable accent.
    pub const SELECTED_EDGE: f32 = 0.7;
    /// What the pointer adds over a selected row of the page list.
    pub const SELECTED_HOVER: f32 = 0.14;
    /// The opacity of a control that waits for something.
    pub const DISABLED: f32 = 0.4;
    /// The opacity of a page while it is being dragged.
    pub const DRAGGED: f32 = 0.4;
    /// The selected button of a segmented control in the dark theme.
    pub const NAV_DARK: f32 = 0.16;
    /// The ground the pages of a document lie on.
    pub const CANVAS: f32 = 0.04;
    /// The accent wash over a place that takes what is being dragged.
    pub const DROP_ZONE: f32 = 0.08;
}

/// Pins, note badges, the note popover and the pen controls over a PDF page.
pub mod annot {
    /// A pin at zoom 100 %, in page points.
    pub const PIN: f32 = 20.;
    pub const PIN_ICON: f32 = 14.;
    /// The badge of a note on a passage at zoom 100 %.
    pub const NOTE_BADGE: f32 = 12.;
    /// The smallest a passage badge gets, as a share of its size.
    pub const NOTE_BADGE_MIN: f32 = 0.75;
    /// The icon in a passage badge, as a share of the badge.
    pub const BADGE_GLYPH: f32 = 0.75;
    /// The edge of a badge that is not selected.
    pub const BADGE_EDGE: f32 = 0.3;
    /// The area a badge answers to, in screen pixels whatever the zoom.
    pub const HIT: f32 = 24.;
    pub const NOTE_WIDTH: f32 = 272.;
    pub const NOTE_HEIGHT: f32 = 150.;
    /// The length of the line that shows a pen width.
    pub const WIDTH_SAMPLE: f32 = 14.;
    /// How much thicker the sample of a width is drawn than the stroke, so the thin one can be told from the thick one.
    pub const WIDTH_SAMPLE_GROW: f32 = 1.5;
    /// The edge of a pen color swatch.
    pub const SWATCH_EDGE: f32 = 0.25;
    /// A search match.
    pub const MATCH: f32 = 0.45;
    /// The match the view stands on.
    pub const MATCH_CURRENT: f32 = 0.6;
    /// The text a drag has selected.
    pub const SELECTED_TEXT: f32 = 0.3;
    /// The glow around a stroke that was clicked.
    pub const HALO: f32 = 0.35;
    /// The row of a list entry that was revealed.
    pub const ROW_ACTIVE: f32 = 0.08;
    /// The row of a search match the view stands on.
    pub const HIT_ROW: f32 = 0.15;
    /// A row of the marker list, with its slot.
    pub const MARK_ROW: f32 = 88.;
    /// The space between a passage badge and the text it belongs to.
    pub const BADGE_GAP: f32 = 2.;
    /// A pin whose note is still being written, before it is placed.
    pub const GHOST: f32 = 0.55;
    /// How far the flash around a revealed mark reaches past it, and the least it measures, in screen pixels.
    pub const FLASH_PAD: f32 = 12.;
    pub const FLASH_MIN: f32 = 32.;
    /// The fill of that flash.
    pub const FLASH_TINT: f32 = 0.2;
    /// The edge of a pen color swatch in the dark theme, where black needs to be told from the ground.
    pub const SWATCH_EDGE_DARK: f32 = 0.5;
    /// The least a stroke is drawn wide, in screen pixels.
    pub const INK_MIN: f32 = 2.;
}

/// The mark a tip puts on an element.
pub mod coach {
    /// How dark the rest of the window gets.
    pub const DIM: f32 = 0.3;
    pub const DIM_DARK: f32 = 0.5;
    /// How far the pulsing ring travels outward.
    pub const PULSE_REACH: f32 = 10.;
    /// How often the ring pulses before it rests.
    pub const PULSES: u32 = 3;
    pub const LABEL_WIDTH: f32 = 280.;
    /// The height a label needs below its element before it moves above it.
    pub const LABEL_ROOM: f32 = 96.;
}

/// The suggestion list under the caret.
pub mod suggestions {
    /// The widest the list gets; longer names end in an ellipsis.
    pub const WIDTH: f32 = 400.;
}

/// The tip of the day and the setup list.
pub mod hints {
    pub const TIP_WIDTH: f32 = 340.;
    /// The strength of the tint behind the tip's icon.
    pub const ICON_TINT: f32 = 0.14;
    /// How long the completed setup list stays before it leaves.
    pub const DONE_FOR: std::time::Duration = std::time::Duration::from_millis(3200);
    /// The label column of an inline form in the setup list.
    pub const FORM_LABEL: f32 = 150.;
    pub const DATE_WIDTH: f32 = 180.;
    pub const URL_WIDTH: f32 = 420.;
    /// The progress bar in the head of the setup list.
    pub const PROGRESS_WIDTH: f32 = 160.;
}

/// Measures of the page list of a PDF.
pub mod pages {
    /// The line that shows where dragged pages land.
    pub const DROP_LINE: f32 = 2.;
    /// The widest a notice over the pages grows.
    pub const NOTICE_WIDTH: f32 = 360.;
    /// Half the height of the chip that follows dragged pages, so the pointer is level with its middle.
    pub const CHIP_HALF: f32 = 16.;
    /// The height of the page picture that follows dragged pages.
    pub const DRAG_THUMB: f32 = 64.;
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

    /// The solid line drawn around a keyboard-focused element.
    pub fn line() -> Hsla {
        *ACCENT.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Width of the line around a keyboard-focused element.
    pub const WIDTH: f32 = 2.;
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
    /// Tells two palettes apart for caches; changes when a design file changes.
    pub id: u64,
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
    /// Red that is readable as text on the surfaces and on its own tinted fill.
    pub red_text: Hsla,
    /// Blue that is readable as text on the surfaces and on its own tinted fill.
    pub blue_text: Hsla,
    pub gray: Hsla,
    pub ink: Hsla,
    pub material: Hsla,
    /// The module colours of a design, in the order of `MODULE_NAMES`; `None` uses the standard table.
    pub modules: Option<[u32; 12]>,
    pub syntax: SyntaxPalette,
}

impl Global for Theme {}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Theme {
    /// The built-in light or dark palette.
    pub fn plain(dark: bool) -> Self {
        let ink = if dark { c(0xFFFFFF) } else { c(0x000000) };
        Self {
            id: if dark { 2 } else { 1 },
            dark,
            sidebar: if dark { c(0x1D1D1D) } else { c(0xF3F3F5) },
            sidebar_border: ink.opacity(if dark { 0.08 } else { 0.06 }),
            content: if dark { c(0x222222) } else { c(0xFFFFFF) },
            secondary_bg: if dark { c(0x2A2A2A) } else { c(0xF5F5F7) },
            column: if dark { c(0x262626) } else { c(0xF7F7F9) },
            primary: ink.opacity(0.88),
            secondary: ink.opacity(if dark { 0.6 } else { 0.62 }),
            tertiary: ink.opacity(if dark { 0.55 } else { 0.6 }),
            separator: ink.opacity(if dark { 0.12 } else { 0.1 }),
            accent: if dark { c(0x0A84FF) } else { c(0x007AFF) },
            accent_text: if dark { c(0x6CB3FF) } else { c(0x0053B4) },
            green_text: if dark { c(0x30D158) } else { c(0x248A3D) },
            on_accent: c(0xFFFFFF),
            selection: if dark { c(0x3172D4) } else { c(0x0A6CF0) },
            green: if dark { c(0x30D158) } else { c(0x34C759) },
            blue: if dark { c(0x0A84FF) } else { c(0x007AFF) },
            orange: if dark { c(0xFF9F0A) } else { c(0xFF9500) },
            orange_text: if dark { c(0xFF9F0A) } else { c(0xB45309) },
            red: if dark { c(0xFF453A) } else { c(0xFF3B30) },
            red_text: if dark { c(0xFF6961) } else { c(0xB3261E) },
            blue_text: if dark { c(0x6CB3FF) } else { c(0x0050B3) },
            gray: c(0x8E8E93),
            ink,
            material: if dark { c(0x303030) } else { c(0xFBFBFC) },
            modules: None,
            syntax: SyntaxPalette::from_keys(if dark { &themes::SYNTAX_DARK } else { &themes::SYNTAX_LIGHT }),
        }
        .hardened()
    }

    /// The theme of a checked design; `id` tells it apart from the others.
    pub fn from_palette(p: &Palette, id: u64) -> Self {
        let col = |v: u32| -> Hsla {
            let byte = |shift: u32| ((v >> shift) & 0xFF) as f32 / 255.;
            Rgba { r: byte(24), g: byte(16), b: byte(8), a: byte(0) }.into()
        };
        Self {
            id,
            dark: p.dark,
            sidebar: col(p.sidebar),
            sidebar_border: col(p.sidebar_border),
            content: col(p.content),
            secondary_bg: col(p.secondary_bg),
            column: col(p.column),
            primary: col(p.primary),
            secondary: col(p.secondary),
            tertiary: col(p.tertiary),
            separator: col(p.separator),
            accent: col(p.accent),
            accent_text: col(p.accent_text),
            green_text: col(p.green_text),
            on_accent: col(p.on_accent),
            selection: col(p.selection),
            green: col(p.green),
            blue: col(p.blue),
            orange: col(p.orange),
            orange_text: col(p.orange_text),
            red: col(p.red),
            red_text: col(p.red),
            blue_text: col(p.blue),
            gray: col(p.gray),
            ink: col(p.ink),
            material: col(p.material),
            modules: Some(std::array::from_fn(|i| p.modules[i] >> 8)),
            syntax: SyntaxPalette::from_keys(&std::array::from_fn::<u32, 6, _>(|i| p.syntax[i] >> 8)),
        }
        .hardened()
    }

    fn surfaces(&self) -> [Hsla; 4] {
        [self.content, self.secondary_bg, self.column, self.sidebar]
    }

    fn hardened(mut self) -> Self {
        let surfaces = self.surfaces();
        let ink = self.ink;
        self.accent_text = deepen(self.accent_text, self.accent, self.accent, ink, surfaces);
        self.green_text = deepen(self.green_text, self.green, self.accent, ink, surfaces);
        self.orange_text = deepen(self.orange_text, self.orange, self.accent, ink, surfaces);
        self.red_text = deepen(self.red_text, self.red, self.accent, ink, surfaces);
        self.blue_text = deepen(self.blue_text, self.blue, self.accent, ink, surfaces);
        self.tertiary = firm(self.tertiary, ink, surfaces);
        self
    }

    /// The border of an input or checkbox, at 3:1 on the content and the card colour.
    pub fn control_border(&self) -> Hsla {
        for step in 10..=100 {
            let border = self.ink.opacity(step as f32 * 0.01);
            if contrast(border, self.content) >= 3. && contrast(border, self.secondary_bg) >= 3. {
                return border;
            }
        }
        self.ink
    }

    pub fn get(cx: &App) -> Self {
        *cx.global::<Theme>()
    }

    /// A color moved towards the text color until it reads as text on this theme's backgrounds.
    pub fn readable(&self, color: Hsla) -> Hsla {
        self.readable_on(color, self.column)
    }

    /// `color` deepened until it reads at 4.5:1 on `background` tinted with itself.
    pub fn readable_on(&self, color: Hsla, background: Hsla) -> Hsla {
        let mut text = color;
        for step in 1..=10 {
            if contrast(text, background.blend(color.opacity(0.12))) >= 4.5 {
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

    /// The glyph on the destructive red, such as the close button of a window under the pointer: white while it reads at 3:1, else the better of black and white.
    pub fn on_danger(&self) -> Hsla {
        let on_white = contrast(white(), self.red);
        if on_white >= 3. || on_white >= contrast(black(), self.red) { white() } else { black() }
    }

    /// A highlight color deepened until it reads at 3:1 on the white page, for the line under the selected highlight.
    pub fn mark_underline(&self, name: &str) -> Hsla {
        let color = self.palette(name);
        let mut line = color;
        for step in 1..=20 {
            if contrast(line, white()) >= 3. {
                break;
            }
            line = color.blend(black().opacity(step as f32 * 0.05));
        }
        line
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
        let index = MODULE_NAMES.iter().position(|n| *n == name).unwrap_or(0);
        match self.modules {
            Some(colors) => c(colors[index]),
            None => {
                let (light, dark) = MODULE_DEFAULTS[index];
                c(if self.dark { dark } else { light })
            }
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

fn identity(tag: &str, bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    (tag, bytes).hash(&mut hasher);
    hasher.finish()
}

fn bundled(json: &str, tag: &str) -> Theme {
    themes::parse(json.as_bytes()).map(|d| Theme::from_palette(&d.palette, identity(tag, json.as_bytes()))).unwrap_or_else(|_| Theme::plain(true))
}

static NORD: LazyLock<Theme> = LazyLock::new(|| bundled(themes::NORD, "nord"));
static DRACULA: LazyLock<Theme> = LazyLock::new(|| bundled(themes::DRACULA, "dracula"));

struct FileState {
    id: String,
    name: String,
    modified: SystemTime,
    theme: Option<Theme>,
    problem: Option<String>,
    reported: bool,
}

static FILE: Mutex<Option<FileState>> = Mutex::new(None);

fn file_state() -> std::sync::MutexGuard<'static, Option<FileState>> {
    FILE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The theme of a loaded design file, read again only when the file changed; a broken edit keeps the colours from before.
fn file_theme(id: &str) -> Option<Theme> {
    let Some(modified) = themes::modified(id) else {
        *file_state() = None;
        return None;
    };
    {
        let state = file_state();
        if let Some(s) = state.as_ref().filter(|s| s.id == id && s.modified == modified) {
            return s.theme;
        }
    }
    let Some((modified, bytes)) = themes::read(id) else {
        return file_state().as_ref().filter(|s| s.id == id).and_then(|s| s.theme);
    };
    let mut state = file_state();
    let before = state.take().filter(|s| s.id == id);
    *state = Some(match themes::parse(&bytes) {
        Ok(design) => FileState { id: id.to_string(), name: design.name, modified, theme: Some(Theme::from_palette(&design.palette, identity(id, &bytes))), problem: None, reported: false },
        Err(problem) => FileState {
            id: id.to_string(),
            name: before.as_ref().map_or_else(|| id.to_string(), |s| s.name.clone()),
            modified,
            theme: before.and_then(|s| s.theme),
            problem: Some(problem),
            reported: false,
        },
    });
    state.as_ref().and_then(|s| s.theme)
}

/// The name of a built-in choice in the settings.
pub fn choice_label(choice: ThemeChoice) -> String {
    use crate::core::i18n::tr;
    match choice {
        ThemeChoice::System => tr("System"),
        ThemeChoice::Light => tr("Hell"),
        ThemeChoice::Dark => tr("Dunkel"),
        ThemeChoice::Nord => "Nord".to_string(),
        ThemeChoice::Dracula => "Dracula".to_string(),
    }
}

/// The palette the choice in the settings means right now; a loaded design wins while its file is there.
pub fn resolve(choice: ThemeChoice, file: &str, system_dark: bool) -> Theme {
    if !file.is_empty()
        && let Some(t) = file_theme(file)
    {
        return t;
    }
    match choice {
        ThemeChoice::System => Theme::plain(system_dark),
        ThemeChoice::Light => Theme::plain(false),
        ThemeChoice::Dark => Theme::plain(true),
        ThemeChoice::Nord => *NORD,
        ThemeChoice::Dracula => *DRACULA,
    }
}

/// Whether native window parts are forced light or dark by the theme in use; `None` follows the system.
pub fn native_appearance() -> Option<bool> {
    let (choice, file) = prefs::with(|p| (p.theme, p.theme_file.clone()));
    if !file.is_empty()
        && let Some(t) = file_theme(&file)
    {
        return Some(t.dark);
    }
    match choice {
        ThemeChoice::System => None,
        ThemeChoice::Light => Some(false),
        _ => Some(true),
    }
}

fn current(window: &Window) -> Theme {
    let (choice, file) = prefs::with(|p| (p.theme, p.theme_file.clone()));
    resolve(choice, &file, is_dark(window.appearance()))
}

/// Draws the app in what the settings say, following the system where they ask for that.
pub fn refresh(window: &mut Window, cx: &mut App) {
    apply(current(window), Some(window), cx);
}

/// Switches the theme now, in every window. Native parts of the window follow.
pub fn choose(choice: ThemeChoice, window: &mut Window, cx: &mut App) {
    prefs::update(|p| {
        p.theme = choice;
        p.theme_file.clear();
    });
    super::platform::set_window_appearance(native_appearance());
    refresh(window, cx);
    cx.refresh_windows();
}

fn select_file(p: &mut prefs::Prefs, id: &str) {
    p.theme_file = id.to_string();
}

/// Switches to a loaded design by its id. The built-in choice from before stays, so it shows again if the file goes.
pub fn choose_file(id: &str, window: &mut Window, cx: &mut App) {
    prefs::update(|p| select_file(p, id));
    super::platform::set_window_appearance(native_appearance());
    refresh(window, cx);
    cx.refresh_windows();
}

/// A design that was loaded: its id, its name, the earlier copy it took the place of and the design chosen before.
pub struct Loaded {
    pub id: String,
    pub name: String,
    pub replaced: Option<Vec<u8>>,
    pub previous: String,
}

static REVERTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Takes back a design that replaced an earlier copy: the old file returns and the design chosen before is chosen again.
/// Windows follow on their next frame through `follow_undo`.
pub fn undo_load(id: &str, bytes: &[u8], previous: &str) -> Result<(), String> {
    themes::revert(id, bytes).map_err(|e| trf("Design konnte nicht gespeichert werden: %@", &[&e]))?;
    prefs::update(|p| select_file(p, previous));
    REVERTED.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Draws in the design chosen again by `undo_load`.
pub fn follow_undo(window: &mut Window, cx: &mut App) {
    if REVERTED.swap(false, std::sync::atomic::Ordering::Relaxed) {
        super::platform::set_window_appearance(native_appearance());
        refresh(window, cx);
        cx.refresh_windows();
    }
}

fn load(path: &std::path::Path) -> Result<Loaded, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(65 * 1024).read_to_end(&mut bytes)).map_err(|_| crate::core::i18n::tr("Die Datei ließ sich nicht lesen."))?;
    let design = themes::parse(&bytes)?;
    if let Some(id) = themes::id_of(path) {
        themes::rescan();
        return Ok(Loaded { id, name: design.name, replaced: None, previous: String::new() });
    }
    let placed = themes::install(&bytes, &design).map_err(|e| trf("Design konnte nicht gespeichert werden: %@", &[&e]))?;
    Ok(Loaded { id: placed.id, name: design.name, replaced: placed.replaced, previous: String::new() })
}

/// Asks for a design file, checks it, keeps a copy and switches to it. `done` gets what was loaded or the German reason; a copy it replaced is announced in the main window as well.
pub fn pick_and_load(handle: AnyWindowHandle, cx: &mut App, done: impl FnOnce(Result<Loaded, String>, &mut App) + 'static) {
    super::platform::choose_file(cx, move |path, cx| {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let task = cx.background_spawn(async move { load(&path) });
        cx.spawn(async move |cx| {
            let mut result = task.await.map_err(|m| trf("%@ wurde nicht geladen: %@", &[&name, &m]));
            cx.update(|cx| {
                if let Ok(loaded) = &mut result {
                    loaded.previous = prefs::with(|p| p.theme_file.clone());
                    let _ = handle.update(cx, |_, window, cx| choose_file(&loaded.id, window, cx));
                    if let Some(bytes) = loaded.replaced.clone() {
                        let text = trf("Design „%@“ ersetzt", &[&loaded.name]);
                        let (id, previous) = (loaded.id.clone(), loaded.previous.clone());
                        let undo: crate::core::library::Undo = Box::new(move |l| {
                            if let Err(m) = undo_load(&id, &bytes, &previous) {
                                l.notify(m, "circle-alert", None, None);
                            }
                        });
                        super::state::act(cx, |l| l.notify(text, "check", Some(crate::core::i18n::tr("Rückgängig")), Some(undo)));
                    }
                }
                done(result, cx);
            });
        })
        .detach();
    });
}

/// Deletes a loaded design and returns its bytes for undo; if it was in use the app goes back to the built-in choice from before.
pub fn remove_file(id: &str, window: &mut Window, cx: &mut App) -> Result<Vec<u8>, String> {
    let bytes = themes::remove(id).map_err(|e| trf("Design konnte nicht entfernt werden: %@", &[&e]))?;
    if prefs::with(|p| p.theme_file == id) {
        choose(prefs::with(|p| p.theme), window, cx);
    }
    Ok(bytes)
}

/// Looks at the design in use again, for when the window comes to the front. Returns what is wrong with its file, once per change.
pub fn recheck(window: &mut Window, cx: &mut App) -> Option<String> {
    let id = prefs::with(|p| p.theme_file.clone());
    if id.is_empty() {
        return None;
    }
    let next = current(window);
    if next.id != Theme::get(cx).id {
        super::platform::set_window_appearance(native_appearance());
        apply(next, Some(window), cx);
        cx.refresh_windows();
    }
    let mut state = file_state();
    let s = state.as_mut().filter(|s| s.id == id && !s.reported)?;
    let problem = s.problem.clone()?;
    s.reported = true;
    Some(trf("Design „%@“ hat einen Fehler, die Farben bleiben wie vorher: %@", &[&s.name, &problem]))
}

/// Sets the app palette and maps it onto the component library so its controls match.
pub fn apply(t: Theme, window: Option<&mut Window>, cx: &mut App) {
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
    apply_syntax_colors(&mut highlight, &t.syntax);
    ct.highlight_theme = std::sync::Arc::new(highlight);
    ct.motion.duration_slow = super::motion::SLOW;
    ct.scrollbar_mode = gpui_kit::base::ScrollbarMode::Hover;
    ct.radius = px(radius::INNER);
    ct.radius_lg = px(radius::CARD);
    let k = &mut ct.colors;
    k.background = t.content;
    k.foreground = t.primary;
    k.border = t.separator;
    k.input = t.control_border();
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
    k.switch = if dark { t.ink.opacity(0.22) } else { t.control_border() };
    k.switch_thumb = gpui_kit::white();
    k.list_active = t.accent.opacity(0.15);
    k.list_hover = t.ink.opacity(0.06);
    k.tab_bar = t.secondary_bg;
    k.tab_active = t.content;
    k.danger = t.red;
    k.danger_foreground = t.on_danger();
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

fn reads(text: Hsla, base: Hsla, accent: Hsla, surfaces: [Hsla; 4]) -> bool {
    surfaces.iter().all(|&bg| {
        contrast(text, bg) >= 4.5
            && [0.12, 0.18].iter().all(|&a| contrast(text, bg.blend(base.opacity(a))) >= 4.5)
            && contrast(text, bg.blend(ring::of(accent))) >= 4.5
            && contrast(text, bg.blend(accent.opacity(ring::ARMED))) >= 4.5
    })
}

fn deepen(text: Hsla, base: Hsla, accent: Hsla, ink: Hsla, surfaces: [Hsla; 4]) -> Hsla {
    for step in 0..=20 {
        let x = text.blend(ink.opacity(step as f32 * 0.05));
        if reads(x, base, accent, surfaces) {
            return x;
        }
    }
    ink
}

fn firm(text: Hsla, ink: Hsla, surfaces: [Hsla; 4]) -> Hsla {
    let reads = |x: Hsla| surfaces.iter().all(|&bg| contrast(x, bg) >= 4.8);
    for step in 0..=25 {
        let x = Hsla { a: (text.a + step as f32 * 0.02).min(1.), ..text };
        if reads(x) {
            return x;
        }
    }
    let solid = Hsla { a: 1., ..text };
    for step in 1..=20 {
        let x = solid.blend(ink.opacity(step as f32 * 0.05));
        if reads(x) {
            return x;
        }
    }
    ink
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

    fn all() -> Vec<(String, Theme)> {
        let mut out = vec![("light".to_string(), Theme::plain(false)), ("dark".to_string(), Theme::plain(true)), ("nord".to_string(), *NORD), ("dracula".to_string(), *DRACULA)];
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/themes");
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let design = themes::parse(&std::fs::read(&path).unwrap()).unwrap();
            out.push((design.name, Theme::from_palette(&design.palette, 7)));
        }
        out
    }

    #[test]
    fn the_close_button_glyph_reads_on_every_theme() {
        for (name, t) in all() {
            assert!(contrast(t.on_danger(), t.red) >= 3., "{name}: {}", contrast(t.on_danger(), t.red));
        }
        assert_eq!(Theme::plain(true).on_danger(), white());
        assert_eq!(Theme::plain(false).on_danger(), white());
    }

    #[test]
    fn bundled_nord_and_dracula_keep_their_colours() {
        let nord = *NORD;
        assert!(nord.dark);
        assert_eq!(nord.content, c(0x2E3440));
        assert_eq!(nord.accent, c(0x5E81AC));
        assert_eq!(nord.accent_text, c(0xA4CFDC));
        assert_eq!(nord.palette("purple"), c(0xB48EAD));
        assert_eq!(nord.palette("unknown"), c(0x81A1C1));
        assert_eq!(nord.syntax.keyword, 0x81A1C1);
        let dracula = *DRACULA;
        assert_eq!(dracula.content, c(0x282A36));
        assert_eq!(dracula.accent_text, c(0xDBC4FC));
        assert_eq!(dracula.palette("blue"), c(0x6CB6FF));
        assert_eq!(dracula.syntax.function, 0x50FA7B);
        assert_ne!(nord.id, dracula.id);
    }

    #[test]
    fn a_design_that_goes_leaves_the_choice_from_before() {
        for before in [ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::Nord, ThemeChoice::Dracula, ThemeChoice::System] {
            let mut p = prefs::Prefs { theme: before, ..prefs::Prefs::default() };
            select_file(&mut p, "no-such-design");
            assert_eq!(p.theme, before);
            assert_eq!(p.theme_file, "no-such-design");
            assert_eq!(resolve(p.theme, &p.theme_file, true).id, resolve(before, "", true).id);
            assert_eq!(resolve(p.theme, &p.theme_file, false).id, resolve(before, "", false).id);
        }
    }

    #[test]
    fn a_missing_file_falls_back_to_the_system() {
        assert_eq!(resolve(ThemeChoice::System, "no-such-design", true).id, Theme::plain(true).id);
        assert_eq!(resolve(ThemeChoice::System, "no-such-design", false).id, Theme::plain(false).id);
        assert_eq!(resolve(ThemeChoice::Nord, "no-such-design", false).id, NORD.id);
        assert_eq!(resolve(ThemeChoice::Dark, "", false).id, Theme::plain(true).id);
    }

    #[test]
    fn badge_digits_read_on_every_theme() {
        for (kind, t) in all() {
            for color in [t.red, t.accent] {
                assert!(contrast(white(), t.badge_fill(color)) >= 4.5, "{kind:?}");
            }
        }
    }

    #[test]
    fn a_selected_page_shows_against_every_panel() {
        for (kind, t) in all() {
            for background in [t.content, t.sidebar, t.secondary_bg] {
                let row = background.blend(t.accent.opacity(tint::SELECTED));
                assert!(contrast(t.accent_text.opacity(tint::SELECTED_EDGE), background) >= 3., "edge on the panel, {kind:?}: {}", contrast(t.accent_text.opacity(tint::SELECTED_EDGE), background));
                assert!(contrast(row, background) >= 1.1, "fill on the panel, {kind:?}: {}", contrast(row, background));
                assert!(contrast(row.blend(t.accent.opacity(tint::SELECTED_HOVER)), row) >= 1.1, "hover on a selected row, {kind:?}");
            }
        }
    }

    #[test]
    fn study_counts_read_on_the_study_header() {
        for (kind, t) in all() {
            for color in [t.blue, t.orange, t.green] {
                let chip = t.content.blend(color.opacity(0.12));
                assert!(contrast(t.readable_on(color, t.content), chip) >= 4.5, "count on its chip, {kind:?}: {}", contrast(t.readable_on(color, t.content), chip));
                assert!(contrast(t.readable_on(color, t.content), t.content) >= 4.5, "count on the header, {kind:?}: {}", contrast(t.readable_on(color, t.content), t.content));
            }
            assert!(contrast(t.secondary, t.content) >= 4.5, "zero count, {kind:?}");
        }
    }

    #[test]
    fn selected_highlight_underline_shows_on_white_paper() {
        for (kind, t) in all() {
            for name in ["yellow", "green", "blue", "pink"] {
                assert!(contrast(t.mark_underline(name), white()) >= 3., "{name}, {kind:?}: {}", contrast(t.mark_underline(name), white()));
            }
        }
    }

    #[test]
    fn every_text_tint_reads_on_every_surface() {
        for (name, t) in all() {
            let tints = [("primary", t.primary), ("secondary", t.secondary), ("tertiary", t.tertiary), ("accent_text", t.accent_text), ("green_text", t.green_text), ("orange_text", t.orange_text), ("red_text", t.red_text), ("blue_text", t.blue_text)];
            let surfaces = [("content", t.content), ("secondary_bg", t.secondary_bg), ("column", t.column), ("sidebar", t.sidebar)];
            for (tint, color) in tints {
                for (surface, background) in surfaces {
                    assert!(contrast(color, background) >= 4.5, "{tint} on {surface}, {name}: {}", contrast(color, background));
                }
            }
            for (tint, color, fill) in [("accent_text", t.accent_text, t.accent), ("green_text", t.green_text, t.green), ("orange_text", t.orange_text, t.orange), ("red_text", t.red_text, t.red), ("blue_text", t.blue_text, t.blue)] {
                for (surface, background) in surfaces {
                    for alpha in [0.12, 0.18] {
                        let badge = background.blend(fill.opacity(alpha));
                        assert!(contrast(color, badge) >= 4.5, "{tint} on its {alpha} badge over {surface}, {name}: {}", contrast(color, badge));
                    }
                }
            }
        }
    }

    #[test]
    fn control_borders_show_on_the_surfaces_they_sit_on() {
        for (name, t) in all() {
            for (surface, background) in [("content", t.content), ("secondary_bg", t.secondary_bg)] {
                assert!(contrast(t.control_border(), background) >= 3., "border on {surface}, {name}: {}", contrast(t.control_border(), background));
            }
        }
    }

    #[test]
    fn text_is_readable_on_every_background() {
        for (dark, t) in all() {
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
                let p = t.syntax;
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

/// The colours of written code. Each stays readable on the note's background.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntaxPalette {
    pub keyword: u32,
    pub string: u32,
    pub comment: u32,
    pub number: u32,
    pub kind: u32,
    pub function: u32,
}

impl SyntaxPalette {
    /// The colours in the order of `themes::SYNTAX_KEYS`.
    pub fn from_keys(k: &[u32; 6]) -> Self {
        Self { keyword: k[0], string: k[1], comment: k[2], number: k[3], kind: k[4], function: k[5] }
    }

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

fn apply_syntax_colors(theme: &mut gpui_kit::component::highlighter::HighlightTheme, p: &SyntaxPalette) {
    use gpui_kit::component::highlighter::ThemeStyle;
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
