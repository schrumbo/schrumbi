use std::ops::Range;

/// Words the system spell checker does not know, as byte ranges. Code, links, tags and addresses are left alone.
#[cfg(target_os = "macos")]
pub fn misspelled(text: &str) -> Vec<Range<usize>> {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::{NSRange, NSString};
    let skipped = skipped_ranges(text);
    let mut starts: Vec<usize> = Vec::with_capacity(text.len());
    for (at, c) in text.char_indices() {
        starts.extend(std::iter::repeat_n(at, c.len_utf16()));
    }
    starts.push(text.len());
    let mut found = Vec::new();
    unsafe {
        let checker: *mut AnyObject = msg_send![class!(NSSpellChecker), sharedSpellChecker];
        if checker.is_null() {
            return found;
        }
        let _: () = msg_send![checker, setAutomaticallyIdentifiesLanguages: true];
        let source = NSString::from_str(text);
        let length = starts.len() - 1;
        let mut at = 0_usize;
        while at < length {
            let hit: NSRange = msg_send![checker, checkSpellingOfString: &*source, startingAt: at as isize];
            if hit.length == 0 || hit.location >= length || hit.location < at {
                break;
            }
            let end = hit.location + hit.length;
            let range = starts[hit.location]..starts[end.min(length)];
            if !skipped.iter().any(|s| s.start <= range.start && range.end <= s.end) {
                found.push(range);
            }
            at = end;
        }
    }
    found
}

/// Other systems bring no checker the app could ask.
#[cfg(not(target_os = "macos"))]
pub fn misspelled(_: &str) -> Vec<Range<usize>> {
    Vec::new()
}

/// Parts of a note that are not prose.
#[cfg(any(target_os = "macos", test))]
fn skipped_ranges(text: &str) -> Vec<Range<usize>> {
    static NOT_PROSE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?s)```.*?(?:```|$)|`[^`\n]*`|\$[^$\n]*\$|\[\[[^\]\n]*\]\]|\]\([^)\n]*\)|https?://\S+|(?:^|\s)#[^\s#]+|<[^>\n]+>").unwrap());
    NOT_PROSE.find_iter(text).map(|m| m.range()).collect()
}

#[cfg(test)]
mod tests {
    use super::skipped_ranges;

    #[test]
    fn code_links_and_tags_are_not_prose() {
        let text = "Wort `kode` [[Ziel]] #tag https://x.org Ende";
        let skipped: Vec<&str> = skipped_ranges(text).into_iter().map(|r| text[r].trim()).collect();
        assert_eq!(skipped, ["`kode`", "[[Ziel]]", "#tag", "https://x.org"]);
    }
}
