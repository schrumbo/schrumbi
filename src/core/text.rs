use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// A relative path with `/` on every platform, as configs and convention paths use it.
pub fn slash_path(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) { text.replace('\\', "/") } else { text.into_owned() }
}

/// Shortens a file name in the middle, so its start, date and extension stay readable.
pub fn middle_ellipsis(name: &str, max: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max || max < 5 {
        return name.to_string();
    }
    let tail = (max - 1) / 2;
    let head = max - 1 - tail;
    format!("{}…{}", chars[..head].iter().collect::<String>(), chars[chars.len() - tail..].iter().collect::<String>())
}

/// Cuts a name at the end with `…` so it stays within `max` characters.
pub fn end_ellipsis(name: &str, max: usize) -> String {
    if name.chars().count() <= max || max < 2 {
        return name.to_string();
    }
    format!("{}…", name.chars().take(max - 1).collect::<String>().trim_end())
}

/// Removes accents, so "é" becomes "e".
pub fn strip_diacritics(s: &str) -> String {
    s.nfd().filter(|c| !is_combining_mark(*c)).collect()
}

/// Lowercased, umlauts transliterated, diacritics removed, so "Übung" matches "uebung".
pub fn fold(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    let lower = s.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss");
    strip_diacritics(&lower)
}

/// Splits on characters that are neither letters nor digits.
pub fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// Finder-like natural ordering: digit runs compare by value, letters case-insensitively.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a, b);
    loop {
        match (x.chars().next(), y.chars().next()) {
            (None, None) => return a.cmp(b),
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let (n, rest_x) = x.split_at(x.bytes().take_while(u8::is_ascii_digit).count());
                let (m, rest_y) = y.split_at(y.bytes().take_while(u8::is_ascii_digit).count());
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o.is_ne() {
                    return o;
                }
                (x, y) = (rest_x, rest_y);
            }
            (Some(c), Some(d)) => {
                let o = if c.is_ascii() && d.is_ascii() { c.to_ascii_lowercase().cmp(&d.to_ascii_lowercase()) } else { fold(&c.to_string()).cmp(&fold(&d.to_string())) };
                if o.is_ne() {
                    return o;
                }
                (x, y) = (&x[c.len_utf8()..], &y[d.len_utf8()..]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_umlauts_and_accents() {
        assert_eq!(fold("Übung Café"), "uebung cafe");
    }

    #[test]
    fn sorts_naturally() {
        let mut v = vec!["W10", "W2", "w1"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["w1", "W2", "W10"]);
    }

    fn slow_cmp(a: &str, b: &str) -> std::cmp::Ordering {
        let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
        loop {
            match (x.peek().copied(), y.peek().copied()) {
                (None, None) => return a.cmp(b),
                (None, _) => return std::cmp::Ordering::Less,
                (_, None) => return std::cmp::Ordering::Greater,
                (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                    let n: String = std::iter::from_fn(|| x.next_if(|c| c.is_ascii_digit())).collect();
                    let m: String = std::iter::from_fn(|| y.next_if(|c| c.is_ascii_digit())).collect();
                    let o = n.trim_start_matches('0').len().cmp(&m.trim_start_matches('0').len()).then_with(|| n.trim_start_matches('0').cmp(m.trim_start_matches('0')));
                    if o.is_ne() {
                        return o;
                    }
                }
                (Some(c), Some(d)) => {
                    let o = fold(&c.to_string()).cmp(&fold(&d.to_string()));
                    if o.is_ne() {
                        return o;
                    }
                    x.next();
                    y.next();
                }
            }
        }
    }

    #[test]
    fn folds_ascii_like_the_general_path() {
        let slow = |s: &str| strip_diacritics(&s.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss"));
        for s in ["", "Messung 19997", "Übung","A-b_C.md", "ABC xyz 123 \t\n"] {
            assert_eq!(fold(s), slow(s), "{s:?}");
        }
    }

    #[test]
    fn natural_order_matches_folding_for_umlauts_sharp_s_digits_and_case() {
        let samples = ["Übung 1", "uebung 1", "Ubung", "ä", "ae", "Z", "z", "ß", "ss", "é", "e", "E", "W02", "w2", "W10", "a_b", "A-B", "Straße", "strasse", "", "1", "01", "İ", "i", "a10b", "a9b", "a09b", "x007y", "x7", "x0", "x00", "Messung 19997", "Messung 2", "Ü1", "u01"];
        for a in samples {
            for b in samples {
                assert_eq!(natural_cmp(a, b), slow_cmp(a, b), "{a:?} vs {b:?}");
            }
        }
    }
}

/// A heading as a file name: nothing a disk or a link would trip over, and not endless.
pub fn safe_file_stem(title: &str) -> String {
    let clean: String = title.chars().map(|c| if "\\/:*?\"<>|#^[]".contains(c) { ' ' } else { c }).collect();
    let joined = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(120).collect::<String>().trim_matches(['.', ' ']).to_string()
}

const FORBIDDEN_IN_NAMES: &str = "\\/:*?\"<>|";

fn reserved_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    let numbered = |prefix: &str| stem.strip_prefix(prefix).is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'));
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT")
}

/// What keeps `name` from working as a file or folder name on every system a study folder travels to; `None` when it works.
pub fn file_name_problem(name: &str) -> Option<String> {
    use super::i18n::{tr, trf};
    if name.chars().all(|c| c == '.' || c.is_whitespace()) {
        Some(tr("Der Name ist leer."))
    } else if let Some(bad) = name.chars().find(|c| FORBIDDEN_IN_NAMES.contains(*c)) {
        Some(trf("Das Zeichen „%@“ geht in einem Namen nicht.", &[&bad.to_string()]))
    } else if name.chars().any(char::is_control) {
        Some(tr("Der Name enthält ein unsichtbares Steuerzeichen."))
    } else if name.ends_with('.') {
        Some(tr("Ein Name darf nicht mit einem Punkt enden."))
    } else if name.ends_with(char::is_whitespace) {
        Some(tr("Ein Name darf nicht mit einem Leerzeichen enden."))
    } else if reserved_device(name) {
        Some(trf("„%@“ ist unter Windows reserviert und geht als Name nicht.", &[&name.split('.').next().unwrap_or_default().trim_end().to_string()]))
    } else if name.len() > 255 {
        Some(tr("Der Name ist zu lang."))
    } else {
        None
    }
}

/// `name` changed just enough to pass `file_name_problem` and stay visible: forbidden and invisible characters become spaces,
/// leading dots go, a device name gets a leading `_`, and an empty name becomes `fallback`.
pub fn safe_file_name(name: &str, fallback: &str) -> String {
    let clean: String = name.chars().map(|c| if FORBIDDEN_IN_NAMES.contains(c) || c.is_control() { ' ' } else { c }).collect();
    let joined = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = joined.trim_matches(['.', ' ']).to_string();
    if out.is_empty() {
        out = fallback.to_string();
    }
    if reserved_device(&out) {
        out.insert(0, '_');
    }
    while out.len() > 255 {
        out.pop();
    }
    out.trim_end_matches(['.', ' ']).to_string()
}

#[cfg(test)]
mod name_tests {
    use super::{file_name_problem, safe_file_name};

    #[test]
    fn names_that_break_on_some_system_are_refused() {
        for bad in ["CON", "nul.md", "Aux.txt", "prn", "Com1.md", "LPT9", "NUL .md", "...", "  ", "", "a.", "a ", "x.md:ads", "..\\esc.md", "C:\\b.md", "a/b", "tab\there", &"x".repeat(256)] {
            assert!(file_name_problem(bad).is_some(), "{bad:?}");
        }
        for good in ["Notiz.md", "Übung ö.1", "COM10.md", "Console.md", "nullte.md", ".hidden", "Blatt 4 - Lösungen.pdf"] {
            assert_eq!(file_name_problem(good), None, "{good:?}");
        }
    }

    #[test]
    fn a_made_safe_name_always_passes() {
        for name in ["NUL", "x.md:ads", "Blatt 4: Lösungen.pdf", "...", "  ", "a.", "con.md", "a\u{0}b", "..", &"ü".repeat(200)] {
            let safe = safe_file_name(name, "Anki");
            assert_eq!(file_name_problem(&safe), None, "{name:?} -> {safe:?}");
        }
        assert_eq!(safe_file_name("Blatt 4: Lösungen.pdf", "x"), "Blatt 4 Lösungen.pdf");
        assert_eq!(safe_file_name("NUL", "x"), "_NUL");
        assert_eq!(safe_file_name("..", "Anki"), "Anki");
    }
}
