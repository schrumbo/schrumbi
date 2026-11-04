use std::ops::Range;

const LONGEST: usize = 60;

/// A word as it is stored: without punctuation around it.
pub fn clean(word: &str) -> Option<String> {
    let word = word.trim_matches(|c: char| !c.is_alphanumeric());
    (!word.is_empty() && word.chars().count() <= LONGEST && !word.chars().any(char::is_whitespace)).then(|| word.to_string())
}

fn folded(word: &str) -> String {
    word.to_lowercase()
}

/// Whether the personal dictionary knows `word`, in any case.
pub fn contains(words: &[String], word: &str) -> bool {
    let key = folded(word);
    words.iter().any(|w| folded(w) == key)
}

/// `words` with `word` added, sorted and without duplicates; `None` when there is nothing to add.
pub fn added(words: &[String], word: &str) -> Option<Vec<String>> {
    let word = clean(word)?;
    if contains(words, &word) {
        return None;
    }
    let mut all = words.to_vec();
    all.push(word);
    all.sort_by_cached_key(|w| (folded(w), w.clone()));
    Some(all)
}

/// `words` without `word`, in any case.
pub fn removed(words: &[String], word: &str) -> Vec<String> {
    let key = folded(word);
    words.iter().filter(|w| folded(w) != key).cloned().collect()
}

/// The ranges of `text` whose word the dictionary does not know.
pub fn unknown(words: &[String], text: &str, ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    if words.is_empty() {
        return ranges;
    }
    ranges.into_iter().filter(|r| text.get(r.clone()).is_none_or(|word| !contains(words, word))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn a_word_is_kept_without_the_punctuation_around_it() {
        assert_eq!(clean("  „Schrumbi“, ").as_deref(), Some("Schrumbi"));
        assert_eq!(clean("Eltern-Kind").as_deref(), Some("Eltern-Kind"));
        assert_eq!(clean("..."), None);
        assert_eq!(clean("zwei Wörter"), None);
        assert_eq!(clean(&"a".repeat(61)), None);
    }

    #[test]
    fn words_stay_sorted_and_appear_once_whatever_the_case() {
        let all = added(&words(&["Zebra"]), "apfel").unwrap();
        let all = added(&all, "Mond").unwrap();
        assert_eq!(all, words(&["apfel", "Mond", "Zebra"]));
        assert_eq!(added(&all, "APFEL"), None);
        assert_eq!(added(&all, "  "), None);
    }

    #[test]
    fn removing_a_word_ignores_its_case_and_keeps_the_others() {
        assert_eq!(removed(&words(&["apfel", "Mond"]), "Apfel"), words(&["Mond"]));
        assert_eq!(removed(&words(&["Mond"]), "Sonne"), words(&["Mond"]));
    }

    #[test]
    fn known_words_lose_their_mark_in_every_case() {
        let text = "Quikc quikc brwon";
        let all = vec![0..5, 6..11, 12..17];
        assert_eq!(unknown(&words(&["quikc"]), text, all.clone()), vec![12..17]);
        assert_eq!(unknown(&[], text, all.clone()), all);
    }
}
