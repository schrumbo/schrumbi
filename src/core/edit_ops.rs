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

/// Whether the selection, or the caret, sits directly between a pair of `token`; a single `*` is not one half of a `**`.
fn wrapped_in(s: &str, sel: &Range<usize>, token: &str) -> bool {
    if !s[..sel.start].ends_with(token) || !s[sel.end..].starts_with(token) {
        return false;
    }
    if token != "*" {
        return true;
    }
    let before = s[..sel.start].chars().rev().take_while(|c| *c == '*').count();
    let after = s[sel.end..].chars().take_while(|c| *c == '*').count();
    before % 2 == 1 && after % 2 == 1
}

/// `**sel**`, `*sel*` or `` `sel` ``; with no selection the caret lands between the tokens.
pub fn wrap(s: &str, sel: Range<usize>, token: &str) -> Edit {
    let inner = &s[sel.clone()];
    if wrapped_in(s, &sel, token) {
        let range = sel.start - token.len()..sel.end + token.len();
        let start = range.start;
        return Edit { range, text: inner.to_string(), select: start..start + inner.len() };
    }
    if !sel.is_empty() {
        if inner.len() >= token.len() * 2 && inner.starts_with(token) && inner.ends_with(token) && !inner.contains('\n') {
            let bare = &inner[token.len()..inner.len() - token.len()];
            return Edit { range: sel.clone(), text: bare.to_string(), select: sel.start..sel.start + bare.len() };
        }
        if inner.contains('\n') {
            let text = inner
                .split('\n')
                .map(|line| {
                    let marks = super::live_text::prefix_len(line);
                    if line[marks..].trim().is_empty() { line.to_string() } else { format!("{}{token}{}{token}", &line[..marks], &line[marks..]) }
                })
                .collect::<Vec<_>>()
                .join("\n");
            let select = sel.start..sel.start + text.len();
            return Edit { range: sel, text, select };
        }
    }
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

/// Wraps the selected lines in `<!-- -->`, or unwraps them; the front matter is never touched, so `None` when only it is selected.
pub fn toggle_comment(s: &str, sel: Range<usize>) -> Option<Edit> {
    let mut r = selected_lines(s, &sel);
    let body = super::vault::frontmatter::body_offset(s);
    if r.start < body {
        if r.end <= body {
            return None;
        }
        r.start = body;
    }
    let block = &s[r.clone()];
    let trimmed = block.trim();
    let text = if let Some(inner) = trimmed.strip_prefix("<!--").and_then(|t| t.strip_suffix("-->")) {
        inner.strip_prefix(' ').unwrap_or(inner).strip_suffix(' ').unwrap_or_else(|| inner.strip_prefix(' ').unwrap_or(inner)).to_string()
    } else {
        format!("<!-- {block} -->")
    };
    let end = r.start + text.len();
    Some(Edit { range: r.clone(), text, select: r.start..end })
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
    if sel.is_empty() {
        let first = s[r.clone()].split('\n').next().unwrap_or_default();
        let moved = if direction > 0 { 2 } else { first.len().saturating_sub(first.strip_prefix("  ").or_else(|| first.strip_prefix(' ')).unwrap_or(first).len()) };
        let caret = if direction > 0 { sel.start + moved } else { sel.start.saturating_sub(moved).max(r.start) };
        return Edit { range: r.clone(), text, select: caret..caret };
    }
    Edit { range: r.clone(), text, select: r.start..end }
}

/// What typing a character around a selection puts on both sides, as in Obsidian.
pub fn surround_pair(typed: &str) -> Option<(&'static str, &'static str)> {
    Some(match typed {
        "*" => ("*", "*"),
        "_" => ("_", "_"),
        "`" => ("`", "`"),
        "=" => ("=", "="),
        "~" => ("~", "~"),
        "$" => ("$", "$"),
        "%" => ("%", "%"),
        "\"" => ("\"", "\""),
        "'" => ("'", "'"),
        "[" => ("[", "]"),
        "(" => ("(", ")"),
        "{" => ("{", "}"),
        _ => return None,
    })
}

/// Puts a pair around the selection and keeps the selection on the text, so a second `*` makes it bold.
pub fn surround(s: &str, sel: Range<usize>, open: &str, close: &str) -> Edit {
    let inner = &s[sel.clone()];
    let start = sel.start + open.len();
    let select = start..start + inner.len();
    Edit { text: format!("{open}{inner}{close}"), range: sel, select }
}

/// In a table row, Tab selects the text of the next cell and Shift+Tab the one before, across rows.
pub fn table_cell(s: &str, caret: usize, back: bool) -> Option<Range<usize>> {
    let start = line_start(s, caret);
    if !s[start..line_end(s, caret)].trim_start().starts_with('|') {
        return None;
    }
    let mut cells: Vec<Range<usize>> = Vec::new();
    let mut at = 0;
    for line in s.split_inclusive('\n') {
        let bare = line.trim_end_matches('\n');
        let rule = bare.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
        if bare.trim_start().starts_with('|') && !rule {
            let bars: Vec<usize> = bare.char_indices().filter(|(i, c)| *c == '|' && (*i == 0 || bare.as_bytes()[i - 1] != b'\\')).map(|(i, _)| i).collect();
            for pair in bars.windows(2) {
                let cell = &bare[pair[0] + 1..pair[1]];
                let lead = cell.len() - cell.trim_start().len();
                let inner = at + pair[0] + 1 + lead;
                cells.push(inner..inner + cell.trim().len());
            }
        }
        at += line.len();
    }
    let here = cells.iter().position(|c| caret >= c.start && caret <= c.end).or_else(|| cells.iter().rposition(|c| c.end <= caret))?;
    let next = if back { here.checked_sub(1)? } else { here + 1 };
    cells.get(next).cloned()
}

/// Tab behind the last cell of a table starts a new row with as many cells.
pub fn new_table_row(s: &str, caret: usize) -> Option<Edit> {
    let end = line_end(s, caret);
    let line = &s[line_start(s, caret)..end];
    if !line.trim_start().starts_with('|') {
        return None;
    }
    let cells = line.trim().matches('|').count().saturating_sub(1).max(1);
    let row = format!("\n|{}", "  |".repeat(cells));
    let at = end + 3;
    Some(Edit { range: end..end, text: row, select: at..at })
}

/// Whether Tab should indent: a selection, or a caret on a list line.
pub fn tab_indents(s: &str, sel: &Range<usize>) -> bool {
    !sel.is_empty() || LIST_PREFIX.is_match(&s[line_start(s, sel.start)..line_end(s, sel.start)])
}

/// Return in a list continues it (`- `, `1.` → `2.`, `- [ ] `); on an empty item it ends the list.
/// Outside lists the new line keeps the indentation. `None` means a plain newline.
/// The `> ` marks a line of a quote or callout starts with.
fn quote_prefix(line: &str) -> Option<&str> {
    let end = line.char_indices().find(|(_, c)| !matches!(c, '>' | ' ')).map_or(line.len(), |(i, _)| i);
    let prefix = &line[..end];
    prefix.trim_start().starts_with('>').then_some(prefix)
}

/// Puts a mark in front of each line, or takes it away if all have it.
pub fn toggle_line_mark(s: &str, sel: Range<usize>, mark: &str) -> Edit {
    let r = selected_lines(s, &sel);
    let all = s[r.clone()].split('\n').all(|l| l.trim_start().starts_with(mark));
    edit_lines(s, sel, |line| {
        let bare = line.trim_start();
        let indent = &line[..line.len() - bare.len()];
        if all {
            format!("{indent}{}", bare.strip_prefix(mark).unwrap_or(bare))
        } else if line.trim().is_empty() {
            line.to_string()
        } else {
            format!("{indent}{mark}{bare}")
        }
    })
}

/// Sets the heading level of the lines; the same level again takes it away.
pub fn set_heading(s: &str, sel: Range<usize>, level: usize) -> Edit {
    let mark = format!("{} ", "#".repeat(level));
    edit_lines(s, sel, |line| {
        let bare = line.trim_start_matches('#').trim_start();
        if line.starts_with(&mark) { bare.to_string() } else { format!("{mark}{bare}") }
    })
}

/// Whether pasted text is a web address that can become the link of a selection.
pub fn pasted_link(s: &str, sel: Range<usize>, pasted: &str) -> Option<Edit> {
    let url = pasted.trim();
    let is_url = (url.starts_with("http://") || url.starts_with("https://")) && !url.contains(char::is_whitespace);
    if sel.is_empty() || !is_url {
        return None;
    }
    let text = format!("[{}]({url})", &s[sel.clone()]);
    let end = sel.start + text.len();
    Some(Edit { range: sel, text, select: end..end })
}

pub fn newline(s: &str, caret: usize) -> Option<Edit> {
    let start = line_start(s, caret);
    let current = &s[start..caret];
    if let Some(quote) = quote_prefix(current) {
        if quote.len() == current.len() {
            return Some(Edit { range: start..caret, text: String::new(), select: start..start });
        }
        let text = format!("\n{quote}");
        let end = caret + text.len();
        return Some(Edit { range: caret..caret, text, select: end..end });
    }
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

/// What closes a code fence or formula just opened on this line, when nothing below closes it yet.
pub fn closing_mark(note: &str, line: usize) -> Option<String> {
    let lines: Vec<&str> = note.split('\n').collect();
    let here = lines.get(line)?.trim_end();
    let bare = here.trim_start();
    let indent = &here[..here.len() - bare.len()];
    let mark = if bare == "$$" {
        "$$"
    } else if bare.starts_with("```") && !bare[3..].contains('`') {
        "```"
    } else if bare.starts_with("~~~") {
        "~~~"
    } else {
        return None;
    };
    let is_mark = |l: &&&str| l.trim_start().starts_with(mark);
    let above = lines[..line].iter().filter(is_mark).count();
    let below = lines[line + 1..].iter().filter(is_mark).count();
    (above % 2 == 0 && below % 2 == 0).then(|| format!("{indent}{mark}"))
}

/// Typing an opening bracket adds its partner; a closing one steps over the partner that is already there.
pub fn typed_pair(s: &str, caret: usize, typed: &str) -> Option<Edit> {
    let next = s[caret..].chars().next();
    let closes = |t: &str| match t {
        "(" => Some(")"),
        "{" => Some("}"),
        "$" => Some("$"),
        _ => None,
    };
    if matches!(typed, ")" | "}" | "$") && next.is_some_and(|c| c.to_string() == typed) {
        let after = caret + typed.len();
        return Some(Edit { range: caret..after, text: typed.to_string(), select: after..after });
    }
    let close = closes(typed)?;
    if next.is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    if typed == "$" && s[..caret].chars().next_back().is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    let after = caret + typed.len();
    Some(Edit { range: caret..caret, text: format!("{typed}{close}"), select: after..after })
}

/// Backspace between an empty pair of brackets takes both.
pub fn empty_pair(s: &str, caret: usize) -> Option<Edit> {
    let before = s[..caret].chars().next_back()?;
    let after = s[caret..].chars().next()?;
    matches!((before, after), ('(', ')') | ('{', '}') | ('$', '$')).then(|| {
        let start = caret - before.len_utf8();
        Edit { range: start..caret + after.len_utf8(), text: String::new(), select: start..start }
    })
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
    fn tab_at_the_end_of_a_table_adds_a_row() {
        let s = "| 1 | 2 |";
        let e = new_table_row(s, s.len()).unwrap();
        assert_eq!(applied(s, &e), "| 1 | 2 |\n|  |  |");
        assert_eq!(e.select.start, s.len() + 3);
        assert!(new_table_row("text", 0).is_none());
    }

    #[test]
    fn tab_walks_table_cells() {
        let s = "| A | B |\n|---|---|\n| 1 | 2 |";
        let a = s.find('A').unwrap();
        assert_eq!(&s[table_cell(s, a, false).unwrap()], "B");
        let b = s.find('B').unwrap();
        assert_eq!(&s[table_cell(s, b, false).unwrap()], "1");
        let one = s.find('1').unwrap();
        assert_eq!(&s[table_cell(s, one, true).unwrap()], "B");
        assert!(table_cell(s, s.find('2').unwrap(), false).is_none());
        assert!(table_cell("kein tisch", 0, false).is_none());
    }

    #[test]
    fn quotes_and_callouts_continue() {
        let s = "> [!tip] Titel";
        let e = newline(s, s.len()).unwrap();
        assert_eq!(e.text, "\n> ");
        let empty = "> a\n> ";
        let cleared = newline(empty, empty.len()).unwrap();
        assert_eq!(cleared.text, "");
        assert_eq!(cleared.range, 4..6);
    }

    #[test]
    fn marks_headings_and_pasted_links() {
        assert_eq!(toggle_line_mark("a\nb", 0..3, "> ").text, "> a\n> b");
        assert_eq!(toggle_line_mark("> a", 0..0, "> ").text, "a");
        assert_eq!(set_heading("## a", 0..0, 1).text, "# a");
        assert_eq!(set_heading("# a", 0..0, 1).text, "a");
        assert_eq!(pasted_link("wort", 0..4, "https://x.org").unwrap().text, "[wort](https://x.org)");
        assert!(pasted_link("wort", 0..0, "https://x.org").is_none());
        assert!(pasted_link("wort", 0..4, "kein link").is_none());
    }

    fn applied(s: &str, e: &Edit) -> String {
        format!("{}{}{}", &s[..e.range.start], e.text, &s[e.range.end..])
    }

    #[test]
    fn tab_in_a_list_keeps_the_caret_and_no_selection() {
        let s = "- a\n- b";
        let e = shift_lines(s, 7..7, 1);
        assert_eq!(applied(s, &e), "- a\n  - b");
        assert_eq!(e.select, 9..9);
        let t = applied(s, &e);
        let back = shift_lines(&t, 9..9, -1);
        assert_eq!(applied(&t, &back), s);
        assert_eq!(back.select, 7..7);
        let sel = shift_lines(s, 0..7, 1);
        assert_eq!(sel.select.start, 0);
        assert!(sel.select.end > 7, "a selection stays a selection");
        let at_start = shift_lines("  - a", 1..1, -1);
        assert_eq!(at_start.select, 0..0, "the caret never leaves its line");
    }

    #[test]
    fn typing_around_a_selection_keeps_it_on_the_text() {
        let s = "hallo welt";
        let (o, c) = surround_pair("*").unwrap();
        let e = surround(s, 6..10, o, c);
        let once = applied(s, &e);
        assert_eq!(once, "hallo *welt*");
        assert_eq!(&once[e.select.clone()], "welt");
        let twice = applied(&once, &surround(&once, e.select.clone(), o, c));
        assert_eq!(twice, "hallo **welt**");
        assert_eq!(surround_pair("["), Some(("[", "]")));
        assert_eq!(surround_pair("a"), None);
    }

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
        let e = toggle_comment(s, 3..3).unwrap();
        let once = apply(s, &e);
        assert_eq!(once, "x\n<!-- hello -->\ny");
        assert_eq!(apply(&once, &toggle_comment(&once, 3..3).unwrap()), s);
    }

    #[test]
    fn commenting_leaves_the_front_matter_alone() {
        let s = "---
title: a
---
hello
world";
        assert_eq!(toggle_comment(s, 0..0), None);
        assert_eq!(toggle_comment(s, 6..10), None);
        assert_eq!(toggle_comment(s, 14..16), None);
        let e = toggle_comment(s, 0..s.len()).unwrap();
        assert_eq!(apply(s, &e), "---
title: a
---
<!-- hello
world -->");
        let e = toggle_comment(s, 18..18).unwrap();
        assert_eq!(apply(s, &e), "---
title: a
---
<!-- hello -->
world");
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

    #[test]
    fn wrapping_toggles_and_keeps_lines_apart() {
        assert_eq!(apply("ab", &wrap("ab", 0..2, "**")), "**ab**");
        assert_eq!(apply("**ab**", &wrap("**ab**", 2..4, "**")), "ab");
        assert_eq!(apply("**ab**", &wrap("**ab**", 0..6, "**")), "ab");
        assert_eq!(apply("a\n\n- b", &wrap("a\n\n- b", 0..6, "**")), "**a**\n\n- **b**");
    }

    #[test]
    fn fences_and_formulas_close_themselves_once() {
        assert_eq!(closing_mark("```js", 0).as_deref(), Some("```"));
        assert_eq!(closing_mark("$$", 0).as_deref(), Some("$$"));
        assert_eq!(closing_mark("```\nx\n```", 2), None);
        assert_eq!(closing_mark("```\n\n```", 0), None);
        assert_eq!(closing_mark("text", 0), None);
    }

    #[test]
    fn brackets_come_in_pairs() {
        assert_eq!(apply("", &typed_pair("", 0, "(").unwrap()), "()");
        assert_eq!(typed_pair("ab", 0, "("), None);
        assert_eq!(apply("()", &typed_pair("()", 1, ")").unwrap()), "()");
        assert_eq!(apply("()", &empty_pair("()", 1).unwrap()), "");
        assert_eq!(typed_pair("5", 1, "$"), None);
    }
}
