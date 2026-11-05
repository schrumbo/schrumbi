use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use regex::Regex;
use serde::Deserialize;

/// A translation: one string, or plural forms keyed by CLDR category.
#[derive(Deserialize)]
#[serde(untagged)]
enum Entry {
    One(String),
    Plural(HashMap<String, String>),
}

static EN: LazyLock<HashMap<String, Entry>> = LazyLock::new(|| serde_json::from_str(include_str!("../../assets/i18n/en.json")).unwrap_or_default());
static LANG: RwLock<Lang> = RwLock::new(Lang::De);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
}

/// Picks the UI language: the setting, else the system language, falling back to German.
pub fn init(setting: &str) {
    let code = if setting.is_empty() { sys_locale::get_locale().unwrap_or_default() } else { setting.to_string() };
    *LANG.write().unwrap() = if code.starts_with("en") { Lang::En } else { Lang::De };
}

/// Whether the app speaks English.
pub fn is_english() -> bool {
    tr("Abbrechen") != "Abbrechen"
}

pub fn lang() -> Lang {
    *LANG.read().unwrap()
}

/// A whole number with thousands separators of the UI language: `111.234` in German, `111,234` in English.
pub fn grouped(n: usize) -> String {
    group_digits(n, if lang() == Lang::De { '.' } else { ',' })
}

fn group_digits(n: usize, separator: char) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(separator);
        }
        out.push(c);
    }
    out
}

/// Translates a German source string, keyed like the Swift string catalog.
pub fn tr(key: &str) -> String {
    trf(key, &[])
}

/// Translates and fills `%@` / `%lld` / `%1$@` placeholders in order.
pub fn trf(key: &str, args: &[&dyn std::fmt::Display]) -> String {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let template = match (lang(), EN.get(key)) {
        (Lang::En, Some(Entry::One(s))) => s.clone(),
        (Lang::En, Some(Entry::Plural(forms))) => {
            let one = args.iter().find_map(|a| a.parse::<i64>().ok()) == Some(1);
            forms.get(if one { "one" } else { "other" }).or_else(|| forms.get("other")).cloned().unwrap_or_else(|| key.to_string())
        }
        _ => key.to_string(),
    };
    for_platform(fill(&template, &args))
}

/// Texts are written for macOS; elsewhere they name the local shortcuts and file manager.
fn for_platform(text: String) -> String {
    if cfg!(target_os = "macos") {
        return text;
    }
    let text = if text.contains(['⌃', '⌥', '⇧', '⌘']) { portable_shortcuts(&text, ctrl_name(), shift_name(), backspace_name()) } else { text };
    let text = if text == "Im Dock ablegen" { "Minimieren".to_string() } else { text };
    if !text.contains("Finder") {
        return text;
    }
    let manager = match (cfg!(windows), lang()) {
        (true, _) => "Explorer",
        (false, Lang::De) => "Dateimanager",
        (false, Lang::En) => "File Manager",
    };
    text.replace("Finder", manager)
}

/// The label of the Ctrl key off macOS: `Strg` on a German keyboard.
pub fn ctrl_name() -> &'static str {
    if lang() == Lang::De { "Strg" } else { "Ctrl" }
}

/// The label of the Shift key off macOS: `Umschalt` on a German keyboard.
pub fn shift_name() -> &'static str {
    if lang() == Lang::De { "Umschalt" } else { "Shift" }
}

/// The label of the key that deletes backwards off macOS.
pub fn backspace_name() -> &'static str {
    if lang() == Lang::De { "Rücktaste" } else { "Backspace" }
}

/// The label a key shows off macOS in the language of the app: `Ctrl` → `Strg`, `Left` → `Links`.
pub fn key_name(label: &str) -> String {
    let german = lang() == Lang::De;
    match label {
        "Ctrl" => ctrl_name(),
        "Shift" => shift_name(),
        "Backspace" => backspace_name(),
        "Left" if german => "Links",
        "Right" if german => "Rechts",
        "Up" if german => "Oben",
        "Down" if german => "Unten",
        other => other,
    }
    .to_string()
}

/// `⇧⌘E` → `Ctrl+Shift+E`, `⌥⌘→` → `Ctrl+Alt+→`.
fn portable_shortcuts(text: &str, ctrl: &str, shift: &str, backspace: &str) -> String {
    static SHORTCUT: LazyLock<Regex> = LazyLock::new(|| Regex::new("([⌃⌥⇧⌘]+)(.)").unwrap());
    SHORTCUT
        .replace_all(text, |c: &regex::Captures| {
            let glyphs = &c[1];
            let mut parts: Vec<&str> = Vec::new();
            if glyphs.contains('⌘') || glyphs.contains('⌃') {
                parts.push(ctrl);
            }
            if glyphs.contains('⌥') {
                parts.push("Alt");
            }
            if glyphs.contains('⇧') {
                parts.push(shift);
            }
            let key = match &c[2] {
                "↩" => "Enter",
                "⇥" => "Tab",
                "⌫" => backspace,
                "⎋" => "Esc",
                k => k,
            };
            parts.push(key);
            parts.join("+")
        })
        .into_owned()
}

fn fill(template: &str, args: &[String]) -> String {
    let mut out = String::new();
    let mut next = 0;
    let mut rest = template;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        if let Some(r) = rest.strip_prefix('%') {
            out.push('%');
            rest = r;
            continue;
        }
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let positional = rest[digits.len()..].strip_prefix('$').filter(|_| !digits.is_empty());
        let after = positional.unwrap_or(rest);
        let Some(spec) = ["@", "lld", "ld", "d", ".1f", ".2f", "f"].iter().find(|s| after.starts_with(**s)) else {
            out.push('%');
            continue;
        };
        let index = if positional.is_some() {
            digits.parse::<usize>().unwrap_or(1) - 1
        } else {
            next += 1;
            next - 1
        };
        out.push_str(args.get(index).map(String::as_str).unwrap_or(""));
        rest = &after[spec.len()..];
    }
    out + rest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_shortcuts_for_other_platforms() {
        assert_eq!(portable_shortcuts("Sichern (⇧⌘E im Fenster), ⌘S und ⌥⌘→", "Ctrl", "Shift", "Backspace"), "Sichern (Ctrl+Shift+E im Fenster), Ctrl+S und Ctrl+Alt+→");
        assert_eq!(portable_shortcuts("⌘↩ erledigt, ⇧⌘, öffnet", "Ctrl", "Shift", "Backspace"), "Ctrl+Enter erledigt, Ctrl+Shift+, öffnet");
        assert_eq!(portable_shortcuts("⇧⌘E", "Strg", "Umschalt", "Rücktaste"), "Strg+Umschalt+E");
        assert_eq!(portable_shortcuts("⇧⌘⌫", "Strg", "Umschalt", "Rücktaste"), "Strg+Umschalt+Rücktaste");
        assert_eq!(portable_shortcuts("ohne Kürzel", "Ctrl", "Shift", "Backspace"), "ohne Kürzel");
    }

    #[test]
    fn keeps_plain_percent_signs() {
        assert_eq!(fill("Klausur 60 %, Projekt 40 %.", &[]), "Klausur 60 %, Projekt 40 %.");
        assert_eq!(fill("%@ von 60 %", &["5".into()]), "5 von 60 %");
        assert_eq!(fill("100%", &[]), "100%");
        assert_eq!(fill("%%", &[]), "%");
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(group_digits(0, '.'), "0");
        assert_eq!(group_digits(999, '.'), "999");
        assert_eq!(group_digits(1000, '.'), "1.000");
        assert_eq!(group_digits(111234, ','), "111,234");
        assert_eq!(group_digits(1234567, '.'), "1.234.567");
    }

    #[test]
    fn fills_placeholders() {
        assert_eq!(fill("in %@", &["2 Tagen".into()]), "in 2 Tagen");
        assert_eq!(fill("%2$@ / %1$@", &["a".into(), "b".into()]), "b / a");
        assert_eq!(fill("%lld von %lld", &["1".into(), "3".into()]), "1 von 3");
    }
}
