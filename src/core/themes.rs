use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};
use std::time::SystemTime;

use serde_json::{Map, Value};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use super::i18n::{is_english, tr, trf};
use super::prefs;

/// A colour as 0xRRGGBBAA.
pub type Rgba = u32;

pub const NORD: &str = include_str!("../../assets/themes/nord.json");
pub const DRACULA: &str = include_str!("../../assets/themes/dracula.json");

pub const MODULE_NAMES: [&str; 12] = ["blue", "green", "orange", "purple", "pink", "teal", "red", "yellow", "indigo", "mint", "brown", "gray"];
pub const SYNTAX_KEYS: [&str; 6] = ["keyword", "string", "comment", "number", "type", "function"];

/// The module colours as (light, dark) in the order of `MODULE_NAMES`, 0xRRGGBB.
pub const MODULE_DEFAULTS: [(u32, u32); 12] = [
    (0x007AFF, 0x0A84FF),
    (0x34C759, 0x30D158),
    (0xFF9500, 0xFF9F0A),
    (0xAF52DE, 0xBF5AF2),
    (0xFF2D55, 0xFF375F),
    (0x30B0C7, 0x40C8E0),
    (0xFF3B30, 0xFF453A),
    (0xFFCC00, 0xFFD60A),
    (0x5856D6, 0x5E5CE6),
    (0x00C7BE, 0x63E6E2),
    (0xA2845E, 0xAC8E68),
    (0x8E8E93, 0x8E8E93),
];

/// The colours of written code in the light theme, in the order of `SYNTAX_KEYS`, 0xRRGGBB.
pub const SYNTAX_LIGHT: [u32; 6] = [0x6A4BC4, 0x2B7A4B, 0x68707C, 0xA3560F, 0x0B7285, 0x1F5FCC];
/// The colours of written code in the dark theme, in the order of `SYNTAX_KEYS`, 0xRRGGBB.
pub const SYNTAX_DARK: [u32; 6] = [0xB7A8F8, 0x8FCB9B, 0x8E939B, 0xE5A76B, 0x7CC7D6, 0x82AAF5];

const MAX_BYTES: usize = 64 * 1024;
const NAME_MAX: usize = 40;
const READABLE: f32 = 4.5;
const BLACK: Rgba = 0x000000FF;
const WHITE: Rgba = 0xFFFFFFFF;

const COLORS: [(&str, bool); 22] = [
    ("content", false),
    ("ink", false),
    ("accent", false),
    ("sidebar", false),
    ("secondaryBg", false),
    ("column", false),
    ("material", false),
    ("sidebarBorder", true),
    ("separator", true),
    ("primary", true),
    ("secondary", true),
    ("tertiary", true),
    ("accentText", false),
    ("onAccent", false),
    ("selection", false),
    ("green", false),
    ("blue", false),
    ("orange", false),
    ("red", false),
    ("gray", false),
    ("greenText", false),
    ("orangeText", false),
];

const TOP_KEYS: [&str; 8] = ["schemaVersion", "name", "dark", "moduleColors", "syntax", "author", "license", "source"];

/// Every colour of a design, resolved, as 0xRRGGBBAA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub sidebar: Rgba,
    pub sidebar_border: Rgba,
    pub content: Rgba,
    pub secondary_bg: Rgba,
    pub column: Rgba,
    pub primary: Rgba,
    pub secondary: Rgba,
    pub tertiary: Rgba,
    pub separator: Rgba,
    pub accent: Rgba,
    pub accent_text: Rgba,
    pub green_text: Rgba,
    pub on_accent: Rgba,
    pub selection: Rgba,
    pub green: Rgba,
    pub blue: Rgba,
    pub orange: Rgba,
    pub orange_text: Rgba,
    pub red: Rgba,
    pub gray: Rgba,
    pub ink: Rgba,
    pub material: Rgba,
    pub modules: [Rgba; 12],
    pub syntax: [Rgba; 6],
}

/// A checked design file.
#[derive(Clone, Debug)]
pub struct Design {
    pub name: String,
    pub palette: Palette,
}

fn parse_hex(text: &str) -> Option<Rgba> {
    let digits = text.strip_prefix('#')?;
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match digits.len() {
        6 => u32::from_str_radix(digits, 16).ok().map(|v| v << 8 | 0xFF),
        8 => u32::from_str_radix(digits, 16).ok(),
        _ => None,
    }
}

fn color_of(path: &str, value: &Value, alpha_ok: bool) -> Result<Rgba, String> {
    let bad = || trf("„%@“ muss eine Farbe wie #1E66F5 sein.", &[&path]);
    let color = value.as_str().and_then(parse_hex).ok_or_else(bad)?;
    if !alpha_ok && color & 0xFF != 0xFF {
        return Err(trf("„%@“ muss deckend sein, ohne Alpha-Anteil.", &[&path]));
    }
    Ok(color)
}

fn channel(c: Rgba, shift: u32) -> f32 {
    ((c >> shift) & 0xFF) as f32 / 255.
}

fn with_alpha(c: Rgba, opacity: f32) -> Rgba {
    (c & 0xFFFFFF00) | (opacity.clamp(0., 1.) * 255.).round() as u32
}

fn pack(r: f32, g: f32, b: f32, a: u32) -> Rgba {
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    byte(r) << 24 | byte(g) << 16 | byte(b) << 8 | a
}

fn mix(base: Rgba, top: Rgba) -> Rgba {
    let a = channel(top, 0);
    let m = |shift: u32| channel(base, shift) * (1. - a) + channel(top, shift) * a;
    pack(m(24), m(16), m(8), base & 0xFF)
}

fn lum(c: Rgba) -> f32 {
    let f = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * f(channel(c, 24)) + 0.7152 * f(channel(c, 16)) + 0.0722 * f(channel(c, 8))
}

fn contrast(text: Rgba, bg: Rgba) -> f32 {
    let a = lum(mix(bg | 0xFF, text));
    let b = lum(bg);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn lift(c: Rgba, content: Rgba, sidebar: Rgba, ink: Rgba) -> Rgba {
    for step in 0..=20 {
        let x = mix(c, with_alpha(ink, step as f32 * 0.05));
        if contrast(x, content) >= READABLE && contrast(x, sidebar) >= READABLE {
            return x;
        }
    }
    ink
}

fn selection_of(accent: Rgba) -> Rgba {
    let mut shade = accent;
    for n in 0..=20 {
        shade = mix(accent, with_alpha(BLACK, n as f32 * 0.05));
        if contrast(WHITE, shade) >= READABLE {
            break;
        }
    }
    shade
}

fn ratio(value: f32) -> String {
    let text = format!("{value:.1}");
    if is_english() { text } else { text.replace('.', ",") }
}

fn too_dim(key: &str, on: &str, value: f32) -> String {
    trf("„%@“ ist auf „%@“ schwer lesbar: Kontrast %@ : 1, nötig sind 4,5 : 1.", &[&key, &on, &ratio(value)])
}

fn missing(key: &str) -> String {
    trf("Im Design fehlt „%@“.", &[&key])
}

fn unknown(key: &str) -> String {
    trf("„%@“ kennt Schrumbi nicht. Vielleicht ein Tippfehler?", &[&key])
}

fn first_unknown(map: &Map<String, Value>, known: impl Fn(&str) -> bool, prefix: &str) -> Option<String> {
    let mut keys: Vec<&String> = map.keys().filter(|k| !known(k)).collect();
    keys.sort();
    keys.first().map(|k| unknown(&format!("{prefix}{k}")))
}

fn text_of(top: &Map<String, Value>, key: &str) -> Result<(), String> {
    match top.get(key) {
        Some(v) if !v.is_string() => Err(trf("„%@“ muss ein Text sein.", &[&key])),
        _ => Ok(()),
    }
}

/// Checks a design file and resolves every colour it leaves out.
pub fn parse(bytes: &[u8]) -> Result<Design, String> {
    if bytes.len() > MAX_BYTES {
        return Err(tr("Die Datei ist zu groß für ein Design, höchstens 64 KB."));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| trf("Die Datei ist kein gültiges JSON (Zeile %lld, Spalte %lld).", &[&e.line(), &e.column()]))?;
    let Value::Object(top) = value else {
        return Err(tr("Die Datei enthält kein Design. Erwartet wird ein JSON-Objekt mit „name“, „content“, „ink“ und „accent“."));
    };
    let known_top: HashSet<&str> = TOP_KEYS.iter().copied().chain(COLORS.iter().map(|(k, _)| *k)).collect();
    if let Some(m) = first_unknown(&top, |k| known_top.contains(k), "") {
        return Err(m);
    }
    if let Some(Value::Object(group)) = top.get("moduleColors") {
        if let Some(m) = first_unknown(group, |k| MODULE_NAMES.contains(&k), "moduleColors.") {
            return Err(m);
        }
    }
    if let Some(Value::Object(group)) = top.get("syntax") {
        if let Some(m) = first_unknown(group, |k| SYNTAX_KEYS.contains(&k), "syntax.") {
            return Err(m);
        }
    }
    match top.get("schemaVersion") {
        None => return Err(missing("schemaVersion")),
        Some(v) => match v.as_u64() {
            Some(1) => {}
            Some(n) if n > 1 => return Err(tr("Dieses Design braucht eine neuere Schrumbi-Version.")),
            _ => return Err(tr("„schemaVersion“ muss 1 sein.")),
        },
    }
    let name = match top.get("name") {
        None => return Err(missing("name")),
        Some(Value::String(s)) => s.trim().to_string(),
        Some(_) => return Err(trf("„%@“ muss ein Text sein.", &[&"name"])),
    };
    if name.is_empty() {
        return Err(tr("„name“ darf nicht leer sein."));
    }
    if name.chars().count() > NAME_MAX {
        return Err(tr("„name“ ist zu lang, höchstens 40 Zeichen."));
    }
    for key in ["content", "ink", "accent"] {
        if !top.contains_key(key) {
            return Err(missing(key));
        }
    }
    let mut given: [Option<Rgba>; 22] = [None; 22];
    for (i, (key, alpha_ok)) in COLORS.iter().enumerate() {
        if let Some(v) = top.get(*key) {
            given[i] = Some(color_of(key, v, *alpha_ok)?);
        }
    }
    let mut modules: [Option<Rgba>; 12] = [None; 12];
    if let Some(Value::Object(group)) = top.get("moduleColors") {
        for (i, key) in MODULE_NAMES.iter().enumerate() {
            if let Some(v) = group.get(*key) {
                modules[i] = Some(color_of(&format!("moduleColors.{key}"), v, false)?);
            }
        }
    }
    let mut syntax: [Option<Rgba>; 6] = [None; 6];
    if let Some(Value::Object(group)) = top.get("syntax") {
        for (i, key) in SYNTAX_KEYS.iter().enumerate() {
            if let Some(v) = group.get(*key) {
                syntax[i] = Some(color_of(&format!("syntax.{key}"), v, false)?);
            }
        }
    }
    let dark_flag = match top.get("dark") {
        None => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(trf("„%@“ muss true oder false sein.", &[&"dark"])),
    };
    for key in ["moduleColors", "syntax"] {
        if matches!(top.get(key), Some(v) if !v.is_object()) {
            return Err(trf("„%@“ muss eine Gruppe in { } sein.", &[&key]));
        }
    }
    for key in ["author", "license", "source"] {
        text_of(&top, key)?;
    }
    let [content, ink, accent, sidebar, secondary_bg, column, material, sidebar_border, separator, primary, secondary, tertiary, accent_text, on_accent, selection, green, blue, orange, red, gray, green_text, orange_text] = given;
    let (Some(content), Some(ink), Some(accent)) = (content, ink, accent) else {
        return Err(tr("Die Datei enthält kein Design. Erwartet wird ein JSON-Objekt mit „name“, „content“, „ink“ und „accent“."));
    };
    let dark = dark_flag.unwrap_or_else(|| lum(content) < lum(ink));
    let pick = |night: f32, day: f32| if dark { night } else { day };
    let sidebar = sidebar.unwrap_or_else(|| mix(content, with_alpha(BLACK, pick(0.15, 0.045))));
    let secondary_bg = secondary_bg.unwrap_or_else(|| mix(content, with_alpha(ink, pick(0.035, 0.04))));
    let column = column.unwrap_or_else(|| mix(content, with_alpha(ink, pick(0.02, 0.03))));
    let material = material.unwrap_or_else(|| if dark { mix(content, with_alpha(ink, 0.065)) } else { mix(content, with_alpha(WHITE, 0.6)) });
    let sidebar_border = sidebar_border.unwrap_or_else(|| with_alpha(ink, pick(0.08, 0.06)));
    let separator = separator.unwrap_or_else(|| with_alpha(ink, pick(0.12, 0.10)));
    let primary_named = if primary.is_some() { "primary" } else { "ink" };
    let secondary_named = if secondary.is_some() { "secondary" } else { "ink" };
    let primary = primary.unwrap_or_else(|| with_alpha(ink, 0.88));
    let secondary = secondary.unwrap_or_else(|| with_alpha(ink, pick(0.60, 0.62)));
    let tertiary = tertiary.unwrap_or_else(|| with_alpha(ink, pick(0.47, 0.56)));
    let accent_text_named = if accent_text.is_some() { "accentText" } else { "ink" };
    let accent_text = accent_text.unwrap_or_else(|| lift(accent, content, sidebar, ink));
    let on_accent = on_accent.unwrap_or_else(|| if lum(accent) > 0.4 { BLACK } else { WHITE });
    let selection = selection.unwrap_or_else(|| selection_of(accent));
    let pair = |night: u32, day: u32| (if dark { night } else { day } << 8) | 0xFF;
    let green = green.unwrap_or_else(|| pair(0x30D158, 0x34C759));
    let blue = blue.unwrap_or_else(|| pair(0x0A84FF, 0x007AFF));
    let orange = orange.unwrap_or_else(|| pair(0xFF9F0A, 0xFF9500));
    let red = red.unwrap_or_else(|| pair(0xFF453A, 0xFF3B30));
    let gray = gray.unwrap_or(0x8E8E93FF);
    let green_text = green_text.unwrap_or_else(|| lift(green, content, sidebar, ink));
    let orange_text = orange_text.unwrap_or_else(|| lift(orange, content, sidebar, ink));
    for (color, key) in [(primary, primary_named), (secondary, secondary_named)] {
        for (bg, on) in [(content, "content"), (sidebar, "sidebar")] {
            let r = contrast(color, bg);
            if r < READABLE {
                return Err(too_dim(key, on, r));
            }
        }
    }
    let r = contrast(accent_text, content);
    if r < READABLE {
        return Err(too_dim(accent_text_named, "content", r));
    }
    let module_colors = std::array::from_fn(|i| modules[i].unwrap_or_else(|| (if dark { MODULE_DEFAULTS[i].1 } else { MODULE_DEFAULTS[i].0 } << 8) | 0xFF));
    let syntax_colors = std::array::from_fn(|i| syntax[i].unwrap_or_else(|| (if dark { SYNTAX_DARK[i] } else { SYNTAX_LIGHT[i] } << 8) | 0xFF));
    Ok(Design {
        name,
        palette: Palette {
            dark,
            sidebar,
            sidebar_border,
            content,
            secondary_bg,
            column,
            primary,
            secondary,
            tertiary,
            separator,
            accent,
            accent_text,
            green_text,
            on_accent,
            selection,
            green,
            blue,
            orange,
            orange_text,
            red,
            gray,
            ink,
            material,
            modules: module_colors,
            syntax: syntax_colors,
        },
    })
}

/// The file stem a design of this name is saved under.
pub fn slug(name: &str) -> String {
    let spelled: String = name
        .chars()
        .flat_map(|c| match c {
            'ä' => "ae".chars().collect::<Vec<_>>(),
            'ö' => "oe".chars().collect(),
            'ü' => "ue".chars().collect(),
            'Ä' => "Ae".chars().collect(),
            'Ö' => "Oe".chars().collect(),
            'Ü' => "Ue".chars().collect(),
            'ß' => "ss".chars().collect(),
            c => vec![c],
        })
        .collect();
    let mut out = String::new();
    for c in spelled.nfd().filter(|c| !is_combining_mark(*c)).flat_map(|c| c.to_lowercase()) {
        let c = if c.is_ascii_alphanumeric() { c } else { '-' };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let trimmed: String = out.trim_matches('-').chars().take(NAME_MAX).collect();
    let trimmed = trimmed.trim_end_matches('-').to_string();
    if trimmed.is_empty() { "design".to_string() } else { trimmed }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= NAME_MAX + 4 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The folder loaded designs are kept in.
pub fn dir() -> Option<PathBuf> {
    prefs::config_dir().map(|d| d.join("themes"))
}

fn path_of(id: &str) -> Option<PathBuf> {
    if !valid_id(id) {
        return None;
    }
    dir().map(|d| d.join(format!("{id}.json")))
}

/// A design file in the folder: its id, its name, and what is wrong with it if it does not load.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub problem: Option<String>,
}

static CACHE: LazyLock<RwLock<Option<Vec<Installed>>>> = LazyLock::new(|| RwLock::new(None));

fn scan() -> Vec<Installed> {
    let Some(folder) = dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).filter(|s| valid_id(s)).map(str::to_string) else { continue };
        let item = match std::fs::read(&path) {
            Err(_) => Installed { name: id.clone(), id, dark: false, problem: Some(tr("Die Datei ließ sich nicht lesen.")) },
            Ok(bytes) => match parse(&bytes) {
                Ok(d) => Installed { id, name: d.name, dark: d.palette.dark, problem: None },
                Err(m) => Installed { name: id.clone(), id, dark: false, problem: Some(m) },
            },
        };
        out.push(item);
    }
    out.sort_by_key(|i| (i.name.to_lowercase(), i.id.clone()));
    out
}

/// The loaded designs, by name.
pub fn installed() -> Vec<Installed> {
    if let Some(list) = CACHE.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return list.clone();
    }
    let list = scan();
    *CACHE.write().unwrap_or_else(|e| e.into_inner()) = Some(list.clone());
    list
}

/// Looks at the folder again.
pub fn rescan() {
    let list = scan();
    *CACHE.write().unwrap_or_else(|e| e.into_inner()) = Some(list);
}

/// When the file of a loaded design last changed.
pub fn modified(id: &str) -> Option<SystemTime> {
    std::fs::metadata(path_of(id)?).ok()?.modified().ok()
}

/// The id of a design file that already lies in the folder, so choosing it needs no copy.
pub fn id_of(path: &std::path::Path) -> Option<String> {
    let folder = std::fs::canonicalize(dir()?).ok()?;
    let file = std::fs::canonicalize(path).ok()?;
    if file.parent()? != folder || file.extension()?.to_str()? != "json" {
        return None;
    }
    Some(file.file_stem()?.to_str()?.to_string()).filter(|id| valid_id(id))
}

/// The bytes of a loaded design and when its file last changed.
pub fn read(id: &str) -> Option<(SystemTime, Vec<u8>)> {
    let path = path_of(id)?;
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    let bytes = std::fs::read(&path).ok()?;
    Some((modified, bytes))
}

fn write_file(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}.{n}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
}

/// What `install` did with a design: where its copy lies and what it replaced.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub id: String,
    /// The bytes of the earlier copy of the same design this one took the place of, for undo.
    pub replaced: Option<Vec<u8>>,
}

/// Keeps a copy of a checked design in the folder. The same bytes keep their id, a changed design of the same name or a broken copy is replaced and its bytes handed back, another design with the same file name gets a number.
pub fn install(bytes: &[u8], design: &Design) -> io::Result<Placed> {
    let base = slug(&design.name);
    let mut n = 1;
    loop {
        let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let path = path_of(&id).ok_or_else(|| io::Error::other("no folder"))?;
        let replaced = match std::fs::read(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
            Ok(old) if old == bytes => return Ok(Placed { id, replaced: None }),
            Ok(old) => {
                let same = match parse(&old) {
                    Ok(d) => d.name == design.name,
                    Err(_) => true,
                };
                if !same {
                    n += 1;
                    continue;
                }
                Some(old)
            }
        };
        write_file(&path, bytes)?;
        rescan();
        return Ok(Placed { id, replaced });
    }
}

/// Writes the bytes back that a replacing `install` took the place of.
pub fn revert(id: &str, bytes: &[u8]) -> io::Result<()> {
    let path = path_of(id).ok_or_else(|| io::Error::other("no such design"))?;
    write_file(&path, bytes)?;
    rescan();
    Ok(())
}

/// Deletes a loaded design and gives back its bytes so the removal can be undone.
pub fn remove(id: &str) -> io::Result<Vec<u8>> {
    let path = path_of(id).ok_or_else(|| io::Error::other("no such design"))?;
    let bytes = std::fs::read(&path)?;
    std::fs::remove_file(&path)?;
    rescan();
    Ok(bytes)
}

/// Puts back what `remove` took.
pub fn restore(id: &str, bytes: &[u8]) -> io::Result<()> {
    let path = path_of(id).ok_or_else(|| io::Error::other("no such design"))?;
    if path.exists() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    write_file(&path, bytes)?;
    rescan();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal(extra: &str) -> String {
        format!(r##"{{"schemaVersion":1,"name":"Probe","content":"#FFFFFF","ink":"#000000","accent":"#1E66F5"{extra}}}"##)
    }

    #[test]
    fn every_gallery_theme_loads() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/themes");
        let mut seen = 0;
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let design = parse(&bytes).unwrap_or_else(|m| panic!("{}: {m}", path.display()));
            assert_eq!(path.file_stem().unwrap().to_str().unwrap(), slug(&design.name));
            seen += 1;
        }
        assert!(seen >= 8);
    }

    #[test]
    fn bundled_themes_load() {
        for (json, name) in [(NORD, "Nord"), (DRACULA, "Dracula")] {
            let d = parse(json.as_bytes()).unwrap();
            assert_eq!(d.name, name);
            assert!(d.palette.dark);
        }
    }

    #[test]
    fn a_minimal_theme_fills_the_rest() {
        let d = parse(minimal("").as_bytes()).unwrap();
        assert!(!d.palette.dark);
        assert!(contrast(d.palette.accent_text, d.palette.content) >= 4.5);
        assert!(contrast(d.palette.accent_text, d.palette.sidebar) >= 4.5);
        assert_eq!(d.palette.modules[0], 0x007AFFFF);
        assert_eq!(d.palette.syntax[0], 0x6A4BC4FF);
        let dark = parse(r##"{"schemaVersion":1,"name":"Nacht","content":"#101010","ink":"#F0F0F0","accent":"#5090F0"}"##.as_bytes()).unwrap();
        assert!(dark.palette.dark);
        assert_eq!(dark.palette.green, 0x30D158FF);
    }

    #[test]
    fn errors_name_the_key() {
        let long = "x".repeat(41);
        let cases: Vec<(String, &str)> = vec![
            ("{ nope".into(), "Zeile 1"),
            ("[]".into(), "Die Datei enthält kein Design"),
            (r##"{"schemaVersion":1,"name":"A","content":"#FFFFFF","ink":"#000000"}"##.into(), "„accent“"),
            (minimal(r##","acent":"#FF0000""##), "„acent“ kennt"),
            (minimal(r##","gray":"#12345""##), "„gray“ muss eine Farbe"),
            (minimal(r##","moduleColors":{"blue":"red"}"##), "„moduleColors.blue“"),
            (minimal(r##","syntax":{"comment":"#12"}"##), "„syntax.comment“"),
            (minimal(r##","moduleColors":{"blau":"#FF0000"}"##), "„moduleColors.blau“ kennt"),
            (minimal(r##","sidebar":"#FFFFFF80""##), "„sidebar“ muss deckend sein"),
            (minimal(r##","dark":"ja""##), "„dark“ muss true oder false"),
            (minimal(r##","author":3"##), "„author“ muss ein Text"),
            (minimal(r##","syntax":3"##), "„syntax“ muss eine Gruppe"),
            (minimal(r##","primary":"#E0E0E0""##), "„primary“ ist auf „content“ schwer lesbar"),
            (r##"{"schemaVersion":2,"name":"A","content":"#FFFFFF","ink":"#000000","accent":"#1E66F5"}"##.into(), "neuere Schrumbi-Version"),
            (r##"{"schemaVersion":"1","name":"A","content":"#FFFFFF","ink":"#000000","accent":"#1E66F5"}"##.into(), "„schemaVersion“ muss 1 sein"),
            (r##"{"name":"A","content":"#FFFFFF","ink":"#000000","accent":"#1E66F5"}"##.into(), "„schemaVersion“"),
            (r##"{"schemaVersion":1,"name":"  ","content":"#FFFFFF","ink":"#000000","accent":"#1E66F5"}"##.into(), "„name“ darf nicht leer"),
            (format!(r##"{{"schemaVersion":1,"name":"{long}","content":"#FFFFFF","ink":"#000000","accent":"#1E66F5"}}"##), "zu lang"),
            (r##"{"schemaVersion":1,"name":"Grau","content":"#808080","ink":"#909090","accent":"#1E66F5"}"##.into(), "„ink“ ist auf „content“ schwer lesbar"),
        ];
        for (json, expect) in cases {
            let err = parse(json.as_bytes()).err().unwrap_or_else(|| panic!("accepted: {json}"));
            assert!(err.contains(expect), "{json}\n  {err}\n  wanted {expect}");
        }
        assert!(parse(&vec![b' '; MAX_BYTES + 1]).err().unwrap().contains("zu groß"));
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Rosé Pine Dawn"), "rose-pine-dawn");
        assert_eq!(slug("Über Äpfel"), "ueber-aepfel");
        assert_eq!(slug("!!!"), "design");
        assert_eq!(slug("Straße"), "strasse");
        assert!(slug(&"ab ".repeat(30)).chars().count() <= NAME_MAX);
    }

    #[test]
    fn install_replace_suffix_remove_restore() {
        let a = minimal("").replace("Probe", "Install Probe");
        let da = parse(a.as_bytes()).unwrap();
        let first = install(a.as_bytes(), &da).unwrap();
        assert_eq!(first, Placed { id: "install-probe".into(), replaced: None });
        let id = first.id;
        assert_eq!(read(&id).unwrap().1, a.as_bytes());
        assert_eq!(install(a.as_bytes(), &da).unwrap(), Placed { id: id.clone(), replaced: None });
        let changed = a.replace("#1E66F5", "#2266F5");
        let swapped = install(changed.as_bytes(), &parse(changed.as_bytes()).unwrap()).unwrap();
        assert_eq!(swapped.id, id);
        assert_eq!(swapped.replaced.as_deref(), Some(a.as_bytes()));
        assert_eq!(read(&id).unwrap().1, changed.as_bytes());
        revert(&id, a.as_bytes()).unwrap();
        assert_eq!(read(&id).unwrap().1, a.as_bytes());
        let other = a.replace("Install Probe", "Install-Probe");
        let numbered = install(other.as_bytes(), &parse(other.as_bytes()).unwrap()).unwrap();
        assert_eq!(numbered, Placed { id: "install-probe-2".into(), replaced: None });
        let id2 = numbered.id;
        assert_eq!(read(&id).unwrap().1, a.as_bytes());
        assert_eq!(read(&id2).unwrap().1, other.as_bytes());
        assert!(installed().iter().any(|i| i.id == id2 && i.problem.is_none()));
        let bytes = remove(&id2).unwrap();
        assert!(read(&id2).is_none());
        restore(&id2, &bytes).unwrap();
        assert!(read(&id2).is_some());
        assert_eq!(restore(&id2, b"x").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read(&id2).unwrap().1, bytes);
        remove(&id2).unwrap();
        remove(&id).unwrap();
        assert!(!installed().iter().any(|i| i.id.starts_with("install-probe")));
        assert!(path_of("../x").is_none());
    }
}
