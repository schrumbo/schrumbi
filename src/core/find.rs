use std::ops::Range;

fn fold_into(c: char, out: &mut String) {
    match c {
        '\n' | '\r' | '\t' | '\u{a0}' => out.push(' '),
        'ß' => out.push_str("ss"),
        c => out.extend(c.to_lowercase()),
    }
}

/// Byte ranges of `query` in `text`, left to right and never overlapping. Unless `exact`, capitals do not matter and ß matches ss.
pub fn matches(text: &str, query: &str, exact: bool) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    if exact {
        return text.match_indices(query).map(|(at, m)| at..at + m.len()).collect();
    }
    let mut needle = String::with_capacity(query.len());
    query.chars().for_each(|c| fold_into(c, &mut needle));
    let mut folded = String::with_capacity(text.len());
    let mut origin: Vec<usize> = Vec::with_capacity(text.len());
    for (at, c) in text.char_indices() {
        fold_into(c, &mut folded);
        origin.resize(folded.len(), at);
    }
    let mut found: Vec<Range<usize>> = Vec::new();
    for (at, m) in folded.match_indices(&needle) {
        let last = origin[at + m.len() - 1];
        let range = origin[at]..last + text[last..].chars().next().map_or(0, char::len_utf8);
        if found.last().is_none_or(|prev| prev.end <= range.start) {
            found.push(range);
        }
    }
    found
}

/// The first match at or after `offset`, wrapping to the first one; `None` without matches.
pub fn first_from(found: &[Range<usize>], offset: usize) -> Option<usize> {
    if found.is_empty() {
        return None;
    }
    Some(found.iter().position(|m| m.start >= offset).unwrap_or(0))
}

/// The match after or before `current`, wrapping around the ends.
pub fn step(current: usize, count: usize, forward: bool) -> usize {
    match (count, forward) {
        (0, _) => 0,
        (_, true) => (current + 1) % count,
        (_, false) => (current + count - 1) % count,
    }
}

/// `text` with every range in `found` replaced by `replacement`.
pub fn replace_all(text: &str, found: &[Range<usize>], replacement: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for m in found {
        out.push_str(&text[at..m.start]);
        out.push_str(replacement);
        at = m.end;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found<'a>(text: &'a str, query: &str, exact: bool) -> Vec<&'a str> {
        matches(text, query, exact).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn a_part_of_a_folded_letter_never_gives_overlapping_matches() {
        assert_eq!(matches("Maß", "s", false), vec![2..4]);
        assert_eq!(matches("saßas", "sas", false), vec![0..4]);
        assert_eq!(replace_all("Maß", &matches("Maß", "s", false), "x"), "Max");
    }

    #[test]
    fn capitals_do_not_matter_unless_asked() {
        assert_eq!(found("Test test TEST", "test", false), ["Test", "test", "TEST"]);
        assert_eq!(found("Test test TEST", "test", true), ["test"]);
    }

    #[test]
    fn empty_query_finds_nothing() {
        assert!(matches("abc", "", false).is_empty());
        assert!(matches("abc", "", true).is_empty());
    }

    #[test]
    fn matches_never_overlap() {
        assert_eq!(matches("aaaa", "aa", false), [0..2, 2..4]);
        assert_eq!(matches("aaa", "aa", true), [0..2]);
    }

    #[test]
    fn umlauts_and_sharp_s_keep_their_byte_ranges() {
        let text = "Größe der Übung, GROSS";
        assert_eq!(found(text, "übung", false), ["Übung"]);
        assert_eq!(found(text, "größe", false), ["Größe"]);
        assert_eq!(found(text, "gross", false), ["GROSS"]);
        assert_eq!(found("Maß", "mass", false), ["Maß"]);
        assert_eq!(found("Maß", "ß", false), ["ß"]);
    }

    #[test]
    fn a_line_break_matches_a_space() {
        assert_eq!(found("ein\nWort", "ein wort", false), ["ein\nWort"]);
    }

    #[test]
    fn the_first_match_from_a_position_wraps() {
        let m = [2..4, 10..12, 20..22];
        assert_eq!(first_from(&m, 0), Some(0));
        assert_eq!(first_from(&m, 3), Some(1));
        assert_eq!(first_from(&m, 10), Some(1));
        assert_eq!(first_from(&m, 21), Some(0));
        assert_eq!(first_from(&[], 5), None);
    }

    #[test]
    fn stepping_wraps_both_ways() {
        assert_eq!(step(2, 3, true), 0);
        assert_eq!(step(0, 3, false), 2);
        assert_eq!(step(1, 3, true), 2);
        assert_eq!(step(0, 0, true), 0);
        assert_eq!(step(0, 1, false), 0);
    }

    #[test]
    fn replacing_all_keeps_the_rest() {
        let text = "ab Test cd test";
        assert_eq!(replace_all(text, &matches(text, "test", false), "X"), "ab X cd X");
        assert_eq!(replace_all(text, &[], "X"), text);
        assert_eq!(replace_all("aa", &matches("aa", "a", true), ""), "");
    }
}
