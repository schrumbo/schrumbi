use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

/// One undoable edit: replace `range` with `text`, then select `select` (offsets in the new text).
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
    pub select: Range<usize>,
}

static LIST_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)([-*+]|(\d{1,3})([.)]))(\s+)(\[[ xX]\]\s+)?").unwrap());

fn line_start(s: &str, at: usize) -> usize {
    s[..at].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(s: &str, at: usize) -> usize {
    s[at..].find('\n').map_or(s.len(), |i| at + i)
}

/// Full lines touched by the selection, without the final newline.
pub fn selected_lines(s: &str, sel: &Range<usize>) -> Range<usize> {
    let end = if sel.end > sel.start && s[..sel.end].ends_with('\n') { sel.end - 1 } else { sel.end };
    line_start(s, sel.start)..line_end(s, end.max(sel.start))
}

/// `**sel**`, `*sel*` or `` `sel` ``; with no selection the caret lands between the tokens.
pub fn wrap(s: &str, sel: Range<usize>, token: &str) -> Edit {
    let inner = &s[sel.clone()];
    let text = format!("{token}{inner}{token}");
    let start = sel.start + token.len();
    let select = if sel.is_empty() { start..start } else { sel.start..sel.start + text.len() };
    Edit { range: sel, text, select }
}

/// `[sel]()` with the caret in the parentheses, or `[](url)` when the selection is a URL.
/// Wraps the selection into a wikilink, or starts an empty one.
pub fn wikilink(s: &str, sel: Range<usize>) -> Edit {
    let inner = &s[sel.clone()];
    let text = format!("[[{inner}]]");
    let caret = sel.start + 2 + inner.len();
    Edit { range: sel, text, select: caret..caret }
}

pub fn link(s: &str, sel: Range<usize>) -> Edit {
    let label = &s[sel.clone()];
    let is_url = label.starts_with("http://") || label.starts_with("https://");
    let text = if is_url { format!("[]({label})") } else { format!("[{label}]()") };
    let caret = if is_url { sel.start + 1 } else { sel.start + label.len() + 3 };
    Edit { range: sel, text, select: caret..caret }
}

/// `[ ]` ↔ `[x]`, list item → task, plain line → `- [ ] line`.
pub fn toggled_task(line: &str) -> String {
    if let Some(i) = line.find("[ ]") {
        return format!("{}[x]{}", &line[..i], &line[i + 3..]);
    }
    if let Some(i) = line.find("[x]").or_else(|| line.find("[X]")) {
        return format!("{}[ ]{}", &line[..i], &line[i + 3..]);
    }
    static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)([-*+]|\d{1,3}[.)])\s+").unwrap());
    if let Some(m) = ITEM.find(line) {
        return format!("{}[ ] {}", m.as_str(), &line[m.end()..]);
    }
    let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    format!("{indent}- [ ] {}", &line[indent.len()..])
}

/// Applies `transform` to every selected line; a caret stays at the same distance from the line end.
pub fn edit_lines(s: &str, sel: Range<usize>, transform: impl Fn(&str) -> String) -> Edit {
    let r = selected_lines(s, &sel);
    let text = s[r.clone()].split('\n').map(transform).collect::<Vec<_>>().join("\n");
    let select = if sel.is_empty() {
        let caret = (sel.start as isize + text.len() as isize - r.len() as isize).clamp(r.start as isize, (r.start + text.len()) as isize) as usize;
        caret..caret
    } else {
        r.start..r.start + text.len()
    };
    Edit { range: r, text, select }
}

/// Wraps the selected lines in `<!-- -->`, or unwraps them.
pub fn toggle_comment(s: &str, sel: Range<usize>) -> Edit {
    let r = selected_lines(s, &sel);
    let block = &s[r.clone()];
    let trimmed = block.trim();
    let text = if let Some(inner) = trimmed.strip_prefix("<!--").and_then(|t| t.strip_suffix("-->")) {
        inner.strip_prefix(' ').unwrap_or(inner).strip_suffix(' ').unwrap_or_else(|| inner.strip_prefix(' ').unwrap_or(inner)).to_string()
    } else {
        format!("<!-- {block} -->")
    };
    let end = r.start + text.len();
    Edit { range: r.clone(), text, select: r.start..end }
}

/// Swaps the selected lines with the line above or below; `None` at the edges.
pub fn move_lines(s: &str, sel: Range<usize>, up: bool) -> Option<Edit> {
    let r = selected_lines(s, &sel);
    let cur = &s[r.clone()];
    if up {
        if r.start == 0 {
            return None;
        }
        let prev = line_start(s, r.start - 1);
        let prev_text = &s[prev..r.start - 1];
        let shift = r.start - prev;
        Some(Edit { range: prev..r.end, text: format!("{cur}\n{prev_text}"), select: sel.start - shift..sel.end - shift })
    } else {
        if r.end >= s.len() {
            return None;
        }
        let next_end = line_end(s, r.end + 1);
        let next_text = &s[r.end + 1..next_end];
        let shift = next_text.len() + 1;
        Some(Edit { range: r.start..next_end, text: format!("{next_text}\n{cur}"), select: sel.start + shift..sel.end + shift })
    }
}

/// Duplicates the selected lines above or below, keeping the selection on the original or the copy.
pub fn copy_lines(s: &str, sel: Range<usize>, up: bool) -> Edit {
    let r = selected_lines(s, &sel);
    let cur = &s[r.clone()];
    let shift = if up { 0 } else { cur.len() + 1 };
    Edit { range: r.clone(), text: format!("{cur}\n{cur}"), select: sel.start + shift..sel.end + shift }
}

/// Removes the selected lines including their newline.
pub fn delete_lines(s: &str, sel: Range<usize>) -> Edit {
    let r = selected_lines(s, &sel);
    let range = if r.end < s.len() {
        r.start..r.end + 1
    } else if r.start > 0 {
        r.start - 1..r.end
    } else {
        r.clone()
    };
    let caret = range.start.min(s.len() - range.len());
    Edit { range, text: String::new(), select: caret..caret }
}

/// Selects the current line including its newline; repeated, extends by the next line.
pub fn select_lines(s: &str, sel: Range<usize>) -> Range<usize> {
    let full = |r: Range<usize>| r.start..(r.end + 1).min(s.len());
    let r = full(selected_lines(s, &sel));
    if !sel.is_empty() && sel == r && r.end < s.len() {
        return r.start..full(line_start(s, r.end)..line_end(s, r.end)).end;
    }
    r
}

/// Indents (`+1`) or outdents (`-1`) the selected lines by two spaces.
pub fn shift_lines(s: &str, sel: Range<usize>, direction: i32) -> Edit {
    let r = selected_lines(s, &sel);
    let text = s[r.clone()]
        .split('\n')
        .map(|row| {
            if direction > 0 {
                format!("  {row}")
            } else {
                row.strip_prefix("  ").or_else(|| row.strip_prefix(' ')).unwrap_or(row).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let end = r.start + text.len();
    Edit { range: r.clone(), text, select: r.start..end }
}

/// Whether Tab should indent: a selection, or a caret on a list line.
pub fn tab_indents(s: &str, sel: &Range<usize>) -> bool {
    !sel.is_empty() || LIST_PREFIX.is_match(&s[line_start(s, sel.start)..line_end(s, sel.start)])
}

/// Return in a list continues it (`- `, `1.` → `2.`, `- [ ] `); on an empty item it ends the list.
/// Outside lists the new line keeps the indentation. `None` means a plain newline.
pub fn newline(s: &str, caret: usize) -> Option<Edit> {
    let start = line_start(s, caret);
    let current = &s[start..caret];
    let Some(c) = LIST_PREFIX.captures(current) else {
        let indent: String = current.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        if indent.is_empty() {
            return None;
        }
        let text = format!("\n{indent}");
        let end = caret + text.len();
        return Some(Edit { range: caret..caret, text, select: end..end });
    };
    if c[0].len() == current.len() {
        return Some(Edit { range: start..caret, text: String::new(), select: start..start });
    }
    let indent = &c[1];
    let marker = match (c.get(3), c.get(4)) {
        (Some(n), Some(p)) => format!("{}{}", n.as_str().parse::<u32>().unwrap_or(0) + 1, p.as_str()),
        _ => c[2].to_string(),
    };
    let task = if c.get(6).is_some() { "[ ] " } else { "" };
    let text = format!("\n{indent}{marker} {task}");
    let end = caret + text.len();
    Some(Edit { range: caret..caret, text, select: end..end })
}

/// Applies an edit to a string, for tests and for buffers without an editor.
pub fn apply(s: &str, e: &Edit) -> String {
    format!("{}{}{}", &s[..e.range.start], e.text, &s[e.range.end..])
}

/// The Markdown for an image file next to the note; angle brackets keep spaces in the name working.
pub fn image_markdown(file_name: &str) -> String {
    let stem = file_name.rsplit_once('.').map_or(file_name, |(stem, _)| stem);
    let target = if file_name.contains(' ') { format!("<{file_name}>") } else { file_name.to_string() };
    format!("![{stem}]({target})")
}

/// The smallest edit that turns `old` into `new`: the changed range of `old` and what replaces it.
pub fn minimal_change<'a>(old: &str, new: &'a str) -> (Range<usize>, &'a str) {
    let prefix: usize = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a.len_utf8()).sum();
    let suffix: usize = old[prefix..].chars().rev().zip(new[prefix..].chars().rev()).take_while(|(a, b)| a == b).map(|(a, _)| a.len_utf8()).sum();
    (prefix..old.len() - suffix, &new[prefix..new.len() - suffix])
}

/// Where a position in the old text ends up after `minimal_change` replaced `changed` with `inserted` bytes.
pub fn shift_offset(offset: usize, changed: &Range<usize>, inserted: usize) -> usize {
    if offset <= changed.start {
        offset
    } else if offset >= changed.end {
        offset - changed.len() + inserted
    } else {
        changed.start + inserted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_change_finds_only_what_differs() {
        assert_eq!(minimal_change("a\nb\n", "neu\na\nb\n"), (0..0, "neu\n"));
        assert_eq!(minimal_change("a\nb", "a\nb\nc"), (3..3, "\nc"));
        assert_eq!(minimal_change("Übung", "Üben"), (3..6, "en"));
        assert_eq!(minimal_change("gleich", "gleich"), (6..6, ""));
        assert_eq!(minimal_change("aa", "a"), (1..2, ""));
        assert_eq!(minimal_change("€€", "€"), (3..6, ""));
    }

    #[test]
    fn offsets_follow_the_change() {
        assert_eq!(shift_offset(5, &(0..0), 3), 8);
        assert_eq!(shift_offset(1, &(4..6), 9), 1);
        assert_eq!(shift_offset(5, &(4..6), 9), 13);
        assert_eq!(shift_offset(8, &(4..6), 9), 15);
    }

    #[test]
    fn image_markdown_keeps_spaces_working() {
        assert_eq!(image_markdown("a.png"), "![a](a.png)");
        assert_eq!(image_markdown("Foto 1.png"), "![Foto 1](<Foto 1.png>)");
    }

    #[test]
    fn wraps_and_links() {
        let e = wrap("a word b", 2..6, "**");
        assert_eq!(apply("a word b", &e), "a **word** b");
        let e = wrap("ab", 1..1, "*");
        assert_eq!((apply("ab", &e), e.select), ("a**b".into(), 2..2));
        let e = link("see x", 4..5);
        assert_eq!((apply("see x", &e), e.select), ("see [x]()".into(), 8..8));
        let e = link("https://a.b", 0..11);
        assert_eq!(apply("https://a.b", &e), "[](https://a.b)");
    }

    #[test]
    fn toggles_tasks_and_comments() {
        assert_eq!(toggled_task("- [ ] a"), "- [x] a");
        assert_eq!(toggled_task("  - [X] a"), "  - [ ] a");
        assert_eq!(toggled_task("1. a"), "1. [ ] a");
        assert_eq!(toggled_task("  plain"), "  - [ ] plain");
        let s = "x\nhello\ny";
        let e = toggle_comment(s, 3..3);
        let once = apply(s, &e);
        assert_eq!(once, "x\n<!-- hello -->\ny");
        assert_eq!(apply(&once, &toggle_comment(&once, 3..3)), s);
    }

    #[test]
    fn moves_copies_and_deletes_lines() {
        let s = "one\ntwo\nthree";
        assert_eq!(apply(s, &move_lines(s, 5..5, true).unwrap()), "two\none\nthree");
        assert_eq!(apply(s, &move_lines(s, 5..5, false).unwrap()), "one\nthree\ntwo");
        assert!(move_lines(s, 0..0, true).is_none());
        assert!(move_lines(s, 10..10, false).is_none());
        assert_eq!(apply(s, &copy_lines(s, 5..5, false)), "one\ntwo\ntwo\nthree");
        assert_eq!(apply(s, &delete_lines(s, 5..5)), "one\nthree");
        assert_eq!(apply(s, &delete_lines(s, 10..10)), "one\ntwo");
        assert_eq!(apply("solo", &delete_lines("solo", 1..1)), "");
    }

    #[test]
    fn selects_and_shifts_lines() {
        let s = "one\ntwo\nthree";
        assert_eq!(select_lines(s, 1..1), 0..4);
        assert_eq!(select_lines(s, 0..4), 0..8);
        assert_eq!(apply(s, &shift_lines(s, 0..5, 1)), "  one\n  two\nthree");
        assert_eq!(apply("  a\n b", &shift_lines("  a\n b", 0..6, -1)), "a\nb");
    }

    #[test]
    fn continues_lists() {
        let s = "- item";
        assert_eq!(apply(s, &newline(s, 6).unwrap()), "- item\n- ");
        let s = "9. item";
        assert_eq!(apply(s, &newline(s, 7).unwrap()), "9. item\n10. ");
        let s = "  - [x] done";
        assert_eq!(apply(s, &newline(s, 12).unwrap()), "  - [x] done\n  - [ ] ");
        let s = "a\n- ";
        assert_eq!(apply(s, &newline(s, 4).unwrap()), "a\n");
        assert_eq!(newline("plain", 5), None);
        assert_eq!(apply("  code", &newline("  code", 6).unwrap()), "  code\n  ");
    }

    #[test]
    fn handles_umlauts_and_emoji() {
        let s = "Übung 🐌\nzwei";
        let e = move_lines(s, s.len() - 1..s.len() - 1, true).unwrap();
        assert_eq!(apply(s, &e), "zwei\nÜbung 🐌");
    }
}
