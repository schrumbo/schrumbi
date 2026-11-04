use std::fmt::Write;

/// Families tried after the one a file names, so text keeps a similar look where its font is missing.
pub const FALLBACK: &str = r#""Carlito", "Calibri", "Segoe UI", "Helvetica Neue", "Helvetica", "Arial", "Liberation Sans", "Noto Sans", "DejaVu Sans""#;

/// What every converted file starts with: a stand-in for pictures that failed to load.
pub const PRELUDE: &str = "#let office-missing(src, width: auto, height: auto, fit: \"cover\") = box(width: width, height: height, fill: luma(235), stroke: 0.5pt + luma(210))\n";

/// A Typst string literal.
pub fn s(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn pt(v: f32) -> String {
    let v = if v.is_finite() { v } else { 0. };
    format!("{v:.2}pt")
}

/// A colour from six hex digits, with an opacity from 0 to 1.
pub fn rgb(hex: &str, alpha: f32) -> String {
    if alpha >= 0.999 {
        format!("rgb(\"#{hex}\")")
    } else {
        let mut out = String::new();
        let _ = write!(out, "rgb(\"#{hex}{:02x}\")", (alpha.clamp(0., 1.) * 255.).round() as u8);
        out
    }
}

/// Six hex digits, or nothing for `auto` and garbage.
pub fn hex(value: &str) -> Option<String> {
    let v = value.trim().trim_start_matches('#');
    let v = if v.len() == 8 { v.get(2..).unwrap_or(v) } else { v };
    (v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit())).then(|| v.to_ascii_lowercase())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

impl Align {
    pub fn typst(self) -> &'static str {
        match self {
            Self::Left | Self::Justify => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

/// HSL luminance changes as Office colours use them (`lumMod`, `lumOff`, `tint`, `shade`).
pub fn adjust(hex: &str, lum_mod: f32, lum_off: f32, tint: Option<f32>, shade: Option<f32>) -> String {
    let n = u32::from_str_radix(hex, 16).unwrap_or(0);
    let (mut r, mut g, mut b) = (((n >> 16) & 255) as f32 / 255., ((n >> 8) & 255) as f32 / 255., (n & 255) as f32 / 255.);
    if let Some(t) = tint {
        r += (1. - r) * (1. - t);
        g += (1. - g) * (1. - t);
        b += (1. - b) * (1. - t);
    }
    if let Some(s) = shade {
        r *= s;
        g *= s;
        b *= s;
    }
    if (lum_mod - 1.).abs() > 1e-4 || lum_off.abs() > 1e-4 {
        let (h, s, l) = to_hsl(r, g, b);
        let l = (l * lum_mod + lum_off).clamp(0., 1.);
        (r, g, b) = from_hsl(h, s, l);
    }
    let c = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    format!("{:06x}", (c(r) << 16) | (c(g) << 8) | c(b))
}

fn to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.;
    if (max - min).abs() < 1e-6 {
        return (0., 0., l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2. - max - min) } else { d / (max + min) };
    let h = if max == r { (g - b) / d + if g < b { 6. } else { 0. } } else if max == g { (b - r) / d + 2. } else { (r - g) / d + 4. };
    (h / 6., s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0. {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1. + s) } else { l + s - l * s };
    let p = 2. * l - q;
    let f = |mut t: f32| {
        if t < 0. {
            t += 1.;
        }
        if t > 1. {
            t -= 1.;
        }
        if t < 1. / 6. {
            p + (q - p) * 6. * t
        } else if t < 0.5 {
            q
        } else if t < 2. / 3. {
            p + (q - p) * (2. / 3. - t) * 6.
        } else {
            p
        }
    };
    (f(h + 1. / 3.), f(h), f(h - 1. / 3.))
}

/// Symbol fonts put their glyphs in the private use area; these are the ones lists use.
pub fn symbol_char(c: char, font: Option<&str>) -> char {
    let symbolic = font.is_some_and(|f| {
        let f = f.to_ascii_lowercase();
        f.contains("symbol") || f.contains("wingdings") || f.contains("webdings")
    });
    let code = c as u32;
    let base = if (0xF000..=0xF0FF).contains(&code) { code - 0xF000 } else { code };
    if !symbolic && code < 0xF000 {
        return c;
    }
    match base {
        0xB7 | 0x6C | 0x9F | 0xA1 => '•',
        0xA7 | 0x6E | 0xA8 => '▪',
        0x71 | 0x6F => '◦',
        0xD8 | 0xE0 => '➢',
        0xFC | 0xFB => '✓',
        0x76 => '❖',
        0x2D => '–',
        _ if symbolic => '•',
        _ => char::from_u32(base).unwrap_or('•'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_colours() {
        assert_eq!(s("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(hex("FF00aa"), Some("ff00aa".into()));
        assert_eq!(hex("auto"), None);
        assert_eq!(adjust("ffffff", 0.5, 0., None, None), "808080");
        assert_eq!(symbol_char('\u{F0B7}', Some("Symbol")), '•');
        assert_eq!(symbol_char('o', Some("Courier New")), 'o');
    }
}
