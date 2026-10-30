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

/// Removes accents, so "é" becomes "e".
pub fn strip_diacritics(s: &str) -> String {
    s.nfd().filter(|c| !is_combining_mark(*c)).collect()
}

/// Lowercased, umlauts transliterated, diacritics removed, so "Übung" matches "uebung".
pub fn fold(s: &str) -> String {
    let lower = s.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss");
    strip_diacritics(&lower)
}

/// Splits on characters that are neither letters nor digits.
pub fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// Finder-like natural ordering: digit runs compare by value, letters case-insensitively.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
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
}
