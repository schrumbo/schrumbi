use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::LazyLock;

use pulldown_cmark::{Event, LinkType, Parser, Tag};
use regex::Regex;

use super::markdown;

/// How a top-level block behaves in the live view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Every line is written on its own: paragraphs, lists, headings.
    Lines,
    /// Written cell by cell.
    Table,
    /// Opens as a whole: code, formulas, callouts, HTML, properties.
    Whole,
}

/// One thing the live view draws, always covering whole source lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Blank,
    /// A line drawn on its own, at this list depth.
    Line { depth: usize },
    /// A line with nothing to draw, like a footnote definition; it shows as written.
    Raw,
    Whole { block: usize },
    Table { block: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub line: usize,
    pub count: usize,
    pub kind: SegmentKind,
    pub hash: u64,
}

fn hash_of(lines: &[&str]) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    lines.hash(&mut hash);
    hash.finish()
}

fn columns(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

/// Cuts a note into what the live view draws. `blocks` holds first line, line after the end and shape of each top-level block.
pub fn segments(text: &str, blocks: &[(usize, usize, Shape)]) -> Vec<Segment> {
    let lines: Vec<&str> = text.split('\n').collect();
    let unit = lines.iter().map(|l| columns(l)).filter(|c| *c > 0).min().unwrap_or(2).max(1);
    let mut out = Vec::with_capacity(lines.len());
    let single = |line: usize, kind: SegmentKind| Segment { line, count: 1, kind, hash: hash_of(&lines[line..=line]) };
    let loose = |line: usize| if lines[line].trim().is_empty() { SegmentKind::Blank } else { SegmentKind::Raw };
    let mut at = 0;
    for (index, (first, end, shape)) in blocks.iter().enumerate() {
        if *first < at || *first >= lines.len() {
            continue;
        }
        while at < *first {
            out.push(single(at, loose(at)));
            at += 1;
        }
        let end = (*end).clamp(first + 1, lines.len());
        match shape {
            Shape::Lines => {
                for line in *first..end {
                    let kind = if lines[line].trim().is_empty() { SegmentKind::Blank } else { SegmentKind::Line { depth: columns(lines[line]) / unit } };
                    out.push(single(line, kind));
                }
            }
            Shape::Table => out.push(Segment { line: *first, count: end - first, kind: SegmentKind::Table { block: index }, hash: hash_of(&lines[*first..end]) }),
            Shape::Whole => out.push(Segment { line: *first, count: end - first, kind: SegmentKind::Whole { block: index }, hash: hash_of(&lines[*first..end]) }),
        }
        at = end;
    }
    while at < lines.len() {
        out.push(single(at, loose(at)));
        at += 1;
    }
    out
}

/// How many bytes a line may have for the live view to draw it whole.
pub const DRAW_MAX: usize = 32_768;

/// How many bytes of a longer line the live view draws.
const DRAW_PART: usize = 4096;

/// What of a line the live view draws, and how many bytes it leaves out: a line too long to lay out in one frame shows its start up to the next space.
pub fn drawn_part(line: &str) -> (&str, usize) {
    if line.len() <= DRAW_MAX {
        return (line, 0);
    }
    let from = (DRAW_PART..).find(|i| line.is_char_boundary(*i)).unwrap_or(line.len());
    let reach = (from + 256).min(line.len());
    let end = (from..=reach).rev().find(|i| line.is_char_boundary(*i)).unwrap_or(from);
    let cut = line[from..end].char_indices().find(|(_, c)| c.is_whitespace()).map_or(from, |(i, c)| from + i + c.len_utf8());
    (&line[..cut], line.len() - cut)
}

/// How deep a line stands while it is written, when Tab may just have moved it.
pub fn depth_while_writing(line: &str, drawn: usize) -> usize {
    let wide = columns(line);
    if wide == 0 { 0 } else { (wide / 2).max(usize::from(drawn > 0) * drawn.min(wide)).min(wide / 2 + drawn) }
}

/// What of a line is drawn when it stands alone: no indent, and no backslash that only breaks it from the next line of its paragraph.
pub fn drawn_line(line: &str, continues: bool) -> &str {
    let bare = line.trim_start();
    let slashes = bare.len() - bare.trim_end_matches('\\').len();
    if continues && slashes % 2 == 1 { bare.strip_suffix('\\').unwrap_or(bare) } else { bare }
}

/// Whether the line after `line` goes on with its paragraph, so a backslash ending `line` is a hard break.
pub fn continues_paragraph(line: &str, next: &str) -> bool {
    let bare = line.trim_start();
    let hashes = bare.chars().take_while(|c| *c == '#').count();
    let heading = (1..=6).contains(&hashes) && bare[hashes..].chars().next().is_none_or(char::is_whitespace);
    !heading && !next.trim().is_empty() && !next.trim_start().starts_with(['#', '>', '`', '~', '|']) && prefix_len(next) == 0
}

/// The marks a line starts with: indent, heading signs, list marker, task box, quote sign.
pub fn prefix_len(line: &str) -> usize {
    static PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:#{1,6}\s+|[-*+]\s+(?:\[.\]\s+)?|\d{1,9}[.)]\s+(?:\[.\]\s+)?|>\s?)").unwrap());
    PREFIX.find(line).map_or(0, |m| m.end())
}

/// How a piece of an open line looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveStyle {
    Marker,
    Bold,
    Italic,
    Strike,
    Code,
    Mark,
    Link,
}

fn inner(range: &Range<usize>, width: usize) -> Range<usize> {
    if range.len() >= width * 2 { range.start + width..range.end - width } else { range.clone() }
}

/// What each part of written Markdown should look like while it is being written.
pub fn styles(text: &str) -> Vec<(Range<usize>, LiveStyle)> {
    static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"==[^=\n]+==").unwrap());
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let prefix = prefix_len(line);
        if prefix > 0 {
            out.push((offset..offset + prefix, LiveStyle::Marker));
        }
        offset += line.len();
    }
    let both = |range: &Range<usize>, width: usize, out: &mut Vec<(Range<usize>, LiveStyle)>| {
        if range.len() >= width * 2 {
            out.push((range.start..range.start + width, LiveStyle::Marker));
            out.push((range.end - width..range.end, LiveStyle::Marker));
        }
    };
    for (event, range) in Parser::new_ext(text, markdown::options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Strong) => {
                out.push((inner(&range, 2), LiveStyle::Bold));
                both(&range, 2, &mut out);
            }
            Event::Start(Tag::Emphasis) => {
                out.push((inner(&range, 1), LiveStyle::Italic));
                both(&range, 1, &mut out);
            }
            Event::Start(Tag::Strikethrough) => {
                let width = if text[range.clone()].starts_with("~~") { 2 } else { 1 };
                out.push((inner(&range, width), LiveStyle::Strike));
                both(&range, width, &mut out);
            }
            Event::Code(_) => {
                let written = &text[range.clone()];
                let width = written.chars().take_while(|c| *c == '`').count();
                out.push((inner(&range, width), LiveStyle::Code));
                both(&range, width, &mut out);
            }
            Event::Start(Tag::Link { link_type, .. }) => {
                if matches!(link_type, LinkType::WikiLink { .. }) {
                    out.push((inner(&range, 2), LiveStyle::Link));
                    both(&range, 2, &mut out);
                } else if let Some(close) = text[range.clone()].rfind("](") {
                    out.push((range.start + 1..range.start + close, LiveStyle::Link));
                    out.push((range.start..range.start + 1, LiveStyle::Marker));
                    out.push((range.start + close..range.end, LiveStyle::Marker));
                } else {
                    out.push((range.clone(), LiveStyle::Link));
                }
            }
            _ => {}
        }
    }
    for hit in MARK.find_iter(text) {
        out.push((inner(&hit.range(), 2), LiveStyle::Mark));
        both(&hit.range(), 2, &mut out);
    }
    out
}

/// Byte ranges of a table row's cells, without the padding around their text.
pub fn table_cells(line: &str) -> Vec<Range<usize>> {
    let bytes = line.as_bytes();
    let bars: Vec<usize> = line.char_indices().filter(|(i, c)| *c == '|' && (*i == 0 || bytes[i - 1] != b'\\')).map(|(i, _)| i).collect();
    let trimmed = line.trim();
    let mut edges = bars;
    if !trimmed.starts_with('|') {
        edges.insert(0, 0_usize.wrapping_sub(1));
    }
    if !trimmed.ends_with('|') {
        edges.push(line.len());
    }
    edges
        .windows(2)
        .map(|pair| {
            let start = pair[0].wrapping_add(1);
            let cell = &line[start..pair[1]];
            let lead = cell.len() - cell.trim_start().len();
            start + lead..start + lead + cell.trim().len()
        })
        .collect()
}

/// Puts text into a cell, keeping one space of padding on both sides.
pub fn set_table_cell(line: &str, column: usize, value: &str) -> Option<(String, Range<usize>)> {
    let cell = table_cells(line).get(column)?.clone();
    let clean = value.replace('\n', " ").replace('|', "\\|");
    let before = line[..cell.start].trim_end_matches(' ');
    let after = line[cell.end..].trim_start_matches(' ');
    let mut out = String::with_capacity(line.len() + clean.len() + 2);
    out.push_str(before);
    out.push(' ');
    let start = out.len();
    out.push_str(&clean);
    let end = out.len();
    out.push(' ');
    out.push_str(after);
    Some((out, start..end))
}

/// Sets the width written with an image, or removes it.
pub fn set_image_width(text: &str, source: &str, width: Option<u32>) -> String {
    let name = regex::escape(source);
    let wiki = Regex::new(&format!(r"!\[\[{name}(\|[^\]]*)?\]\]")).unwrap();
    let suffix = width.map_or(String::new(), |w| format!("|{w}"));
    if wiki.is_match(text) {
        return wiki.replace(text, |_: &regex::Captures| format!("![[{source}{suffix}]]")).to_string();
    }
    let plain = Regex::new(&format!(r"!\[([^\]|]*)(?:\|[^\]]*)?\]\({name}\)")).unwrap();
    plain.replace(text, |c: &regex::Captures| format!("![{}{suffix}]({source})", &c[1])).to_string()
}

/// Changes the type of the first callout in a text, keeping fold mark and title.
pub fn set_callout_type(text: &str, kind: &str) -> String {
    static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[![^\]\s]+\]").unwrap());
    HEAD.replace(text, |_: &regex::Captures| format!("[!{kind}]")).to_string()
}

/// Joins three versions of a note when both sides changed different lines; `None` when they touched the same place.
pub fn merge(base: &str, mine: &str, theirs: &str) -> Option<String> {
    let (b, m, t): (Vec<&str>, Vec<&str>, Vec<&str>) = (base.split('\n').collect(), mine.split('\n').collect(), theirs.split('\n').collect());
    let changes = |other: &[&str]| -> Vec<(Range<usize>, Vec<String>)> {
        similar_blocks(&b, other)
    };
    let (ours, others) = (changes(&m), changes(&t));
    let mut out: Vec<String> = Vec::with_capacity(b.len());
    let (mut i, mut oi, mut ti) = (0, 0, 0);
    while i <= b.len() {
        let o = ours.get(oi).filter(|c| c.0.start == i);
        let th = others.get(ti).filter(|c| c.0.start == i);
        match (o, th) {
            (Some(a), Some(c)) => {
                if a != c {
                    return None;
                }
                out.extend(a.1.iter().cloned());
                i = a.0.end;
                oi += 1;
                ti += 1;
                continue;
            }
            (Some(change), None) | (None, Some(change)) => {
                let is_ours = o.is_some();
                let overlap = if is_ours { others.get(ti).is_some_and(|c| c.0.start < change.0.end.max(i + 1)) } else { ours.get(oi).is_some_and(|c| c.0.start < change.0.end.max(i + 1)) };
                if overlap && change.0.end > i {
                    return None;
                }
                out.extend(change.1.iter().cloned());
                let was_empty = change.0.is_empty();
                i = change.0.end;
                if is_ours {
                    oi += 1;
                } else {
                    ti += 1;
                }
                if was_empty {
                    if i < b.len() && ours.get(oi).is_none_or(|c| c.0.start != i) && others.get(ti).is_none_or(|c| c.0.start != i) {
                        out.push(b[i].to_string());
                        i += 1;
                    } else if i >= b.len() && ours.get(oi).is_none() && others.get(ti).is_none() {
                        break;
                    }
                }
                continue;
            }
            (None, None) => {}
        }
        if i < b.len() {
            out.push(b[i].to_string());
        }
        i += 1;
    }
    Some(out.join("\n"))
}

/// What changed from `base` to `other`, as replaced base lines. Common start and end stay out; the middle is compared by longest common lines.
fn similar_blocks(base: &[&str], other: &[&str]) -> Vec<(Range<usize>, Vec<String>)> {
    let prefix = base.iter().zip(other).take_while(|(a, b)| a == b).count();
    let suffix = base[prefix..].iter().rev().zip(other[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (a, b) = (&base[prefix..base.len() - suffix], &other[prefix..other.len() - suffix]);
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    if a.len() * b.len() > 4_000_000 {
        return vec![(prefix..prefix + a.len(), b.iter().map(|s| s.to_string()).collect())];
    }
    let mut table = vec![vec![0_u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i][j] = if a[i] == b[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out: Vec<(Range<usize>, Vec<String>)> = Vec::new();
    let mut open: Option<(usize, Vec<String>)> = None;
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            if let Some((start, text)) = open.take() {
                out.push((start + prefix..i + prefix, text));
            }
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || table[i][j + 1] >= table[i + 1][j]) {
            open.get_or_insert_with(|| (i, Vec::new())).1.push(b[j].to_string());
            j += 1;
        } else {
            open.get_or_insert_with(|| (i, Vec::new()));
            i += 1;
        }
    }
    if let Some((start, text)) = open.take() {
        out.push((start + prefix..i + prefix, text));
    }
    out
}

/// What a line can be turned into from its menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Text,
    Heading(usize),
    Bullet,
    Numbered,
    Task,
    Quote,
    Code,
    Callout,
    Formula,
}

/// The line without what makes it a heading, list item or quote; its indent stays out.
pub fn bare_text(line: &str) -> &str {
    let t = line.trim_start();
    let t = t.strip_prefix("> ").or_else(|| t.strip_prefix('>')).unwrap_or(t);
    let t = t.trim_start_matches('#').trim_start();
    let at = prefix_len(t);
    t[at..].trim_start()
}

/// The same text as another kind of block, in as many lines as that takes.
pub fn turn_into(lines: &[&str], kind: LineKind) -> Vec<String> {
    let indent: String = lines.first().map_or(String::new(), |l| l.chars().take_while(|c| c.is_whitespace()).collect());
    let texts: Vec<&str> = lines.iter().map(|l| bare_text(l)).collect();
    let prefixed = |prefix: &str| texts.iter().map(|t| format!("{indent}{prefix}{t}")).collect::<Vec<_>>();
    match kind {
        LineKind::Text => prefixed(""),
        LineKind::Heading(level) => prefixed(&format!("{} ", "#".repeat(level.clamp(1, 6)))),
        LineKind::Bullet => prefixed("- "),
        LineKind::Numbered => texts.iter().enumerate().map(|(i, t)| format!("{indent}{}. {t}", i + 1)).collect(),
        LineKind::Task => prefixed("- [ ] "),
        LineKind::Quote => prefixed("> "),
        LineKind::Code => std::iter::once("```".to_string()).chain(texts.iter().map(|t| t.to_string())).chain(std::iter::once("```".to_string())).collect(),
        LineKind::Callout => std::iter::once("> [!note]".to_string()).chain(texts.iter().map(|t| format!("> {t}"))).collect(),
        LineKind::Formula => std::iter::once("$$".to_string()).chain(texts.iter().map(|t| t.to_string())).chain(std::iter::once("$$".to_string())).collect(),
    }
}

/// A table with one more empty row at the end.
pub fn table_with_row(lines: &[&str]) -> Vec<String> {
    let columns = lines.first().map_or(1, |l| table_cells(l).len().max(1));
    let row = format!("|{}", "  |".repeat(columns));
    lines.iter().map(|l| l.to_string()).chain(std::iter::once(row)).collect()
}

/// A table with one more empty column on the right.
pub fn table_with_column(lines: &[&str]) -> Vec<String> {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let line = l.trim_end();
            let line = if line.ends_with('|') { line.to_string() } else { format!("{line} |") };
            if i == 1 { format!("{line}---|") } else { format!("{line}  |") }
        })
        .collect()
}

/// The whole `$$` block a line stands in, while it is being written line by line.
pub fn open_formula(lines: &[String], line: usize) -> Option<String> {
    let is_mark = |l: &String| l.trim() == "$$";
    let above = lines.iter().take(line).filter(|l| is_mark(l)).count();
    if above % 2 == 0 || lines.get(line).is_some_and(is_mark) {
        return None;
    }
    let first = (0..line).rev().find(|i| is_mark(&lines[*i]))?;
    let last = (line + 1..lines.len()).find(|i| is_mark(&lines[*i])).unwrap_or(lines.len());
    Some(format!("$$\n{}\n$$", lines[first + 1..last].join("\n")))
}

/// The formula around a position in a line, without its dollar signs.
pub fn formula_at(line: &str, caret: usize) -> Option<&str> {
    let mut start = None;
    let bytes = line.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b != b'$' || (i > 0 && bytes[i - 1] == b'\\') {
            continue;
        }
        match start {
            None if bytes.get(i + 1).is_some_and(|b| !b.is_ascii_whitespace()) => start = Some(i),
            None => {}
            Some(_) if bytes[i - 1].is_ascii_whitespace() || bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {}
            Some(open) => {
                if caret > open && caret <= i && i > open + 1 {
                    return line.get(open + 1..i);
                }
                start = None;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_is_cut_into_lines_and_wholes() {
        let text = "# T\n\nA\nB\n\n- a\n  - b\n\n```\nc\n```\n\n| x |\n|---|\n| y |\n\n[^1]: Fuß\n";
        let blocks = [(0, 1, Shape::Lines), (2, 4, Shape::Lines), (5, 7, Shape::Lines), (8, 11, Shape::Whole), (12, 15, Shape::Table)];
        let kinds: Vec<(usize, SegmentKind)> = segments(text, &blocks).into_iter().map(|s| (s.line, s.kind)).collect();
        assert_eq!(kinds[0], (0, SegmentKind::Line { depth: 0 }));
        assert_eq!(kinds[1], (1, SegmentKind::Blank));
        assert_eq!(kinds[3], (3, SegmentKind::Line { depth: 0 }));
        assert_eq!(kinds[6], (6, SegmentKind::Line { depth: 1 }));
        assert_eq!(kinds[8], (8, SegmentKind::Whole { block: 3 }));
        assert_eq!(kinds[10], (12, SegmentKind::Table { block: 4 }));
        assert_eq!(kinds[12], (16, SegmentKind::Raw));
        assert_eq!(kinds.last().unwrap().1, SegmentKind::Blank);
    }

    #[test]
    fn a_line_too_long_to_lay_out_shows_its_start_up_to_a_space() {
        let short = "Wort ".repeat(100);
        assert_eq!(drawn_part(&short), (short.as_str(), 0));
        let long = "Wörter ".repeat(20_000);
        let (shown, left) = drawn_part(&long);
        assert!(shown.len() >= DRAW_PART && shown.len() < DRAW_PART + 300 && shown.ends_with(' '));
        assert_eq!(shown.len() + left, long.len());
        let solid = "ä".repeat(DRAW_MAX);
        let (shown, left) = drawn_part(&solid);
        assert!(!shown.is_empty() && shown.len() + left == solid.len());
    }

    #[test]
    fn a_footnote_definition_and_a_raw_line_are_cut_like_any_long_line() {
        let note = format!("[^1]: {}", "Wörter ".repeat(6_000));
        let (shown, left) = drawn_part(&note);
        assert!(shown.starts_with("[^1]: ") && left > 0 && shown.len() + left == note.len());
    }

    #[test]
    fn prefixes_and_styles_find_the_marks() {
        assert_eq!(prefix_len("## T"), 3);
        assert_eq!(prefix_len("  - [ ] a"), 8);
        assert_eq!(prefix_len("12. a"), 4);
        assert_eq!(prefix_len("> a"), 2);
        assert_eq!(prefix_len("Text"), 0);
        let found = styles("a **b** [[c]]");
        assert!(found.contains(&(4..5, LiveStyle::Bold)));
        assert!(found.contains(&(2..4, LiveStyle::Marker)));
        assert!(found.contains(&(10..11, LiveStyle::Link)));
        assert!(found.contains(&(11..13, LiveStyle::Marker)));
    }

    #[test]
    fn cells_are_found_and_set() {
        let line = "| A | B c |";
        let cells = table_cells(line);
        assert_eq!(&line[cells[0].clone()], "A");
        assert_eq!(&line[cells[1].clone()], "B c");
        let (changed, range) = set_table_cell(line, 1, "neu").unwrap();
        assert_eq!(changed, "| A | neu |");
        assert_eq!(&changed[range], "neu");
        let (piped, _) = set_table_cell(line, 0, "a|b").unwrap();
        assert_eq!(piped, "| a\\|b | B c |");
        assert_eq!(set_table_cell("|  |  |", 1, "x").unwrap().0, "|  | x |");
        let bare = "a | b";
        assert_eq!(table_cells(bare).len(), 2);
    }

    #[test]
    fn image_width_and_callout_type_are_rewritten() {
        assert_eq!(set_image_width("a ![[b.png]] c", "b.png", Some(200)), "a ![[b.png|200]] c");
        assert_eq!(set_image_width("![[b.png|200]]", "b.png", Some(400)), "![[b.png|400]]");
        assert_eq!(set_image_width("![[b.png|200]]", "b.png", None), "![[b.png]]");
        assert_eq!(set_image_width("![alt](x/b.png)", "x/b.png", Some(300)), "![alt|300](x/b.png)");
        assert_eq!(set_image_width("![alt|300](x/b.png)", "x/b.png", None), "![alt](x/b.png)");
        assert_eq!(set_callout_type("> [!tip]+ Titel\n> a", "warning"), "> [!warning]+ Titel\n> a");
    }

    #[test]
    fn changes_in_different_places_merge() {
        let base = "a\nb\nc\nd\ne";
        assert_eq!(merge(base, "a!\nb\nc\nd\ne", "a\nb\nc\nd\ne!").unwrap(), "a!\nb\nc\nd\ne!");
        assert_eq!(merge(base, "x\na\nb\nc\nd\ne", "a\nb\nc\nd\ne\ny").unwrap(), "x\na\nb\nc\nd\ne\ny");
        assert_eq!(merge(base, "a\nb\nc\nd\ne", "a\nb!\nc\nd\ne").unwrap(), "a\nb!\nc\nd\ne");
        assert_eq!(merge(base, "a\nB\nc\nd\ne", "a\nb2\nc\nd\ne"), None);
        assert_eq!(merge(base, "a\nB\nc\nd\ne", "a\nB\nc\nd\ne").unwrap(), "a\nB\nc\nd\ne");
    }

    #[test]
    fn lines_turn_into_other_blocks() {
        assert_eq!(turn_into(&["# Titel"], LineKind::Bullet), vec!["- Titel"]);
        assert_eq!(turn_into(&["- [ ] Aufgabe"], LineKind::Heading(2)), vec!["## Aufgabe"]);
        assert_eq!(turn_into(&["> Zitat"], LineKind::Text), vec!["Zitat"]);
        assert_eq!(turn_into(&["a"], LineKind::Code), vec!["```", "a", "```"]);
        assert_eq!(turn_into(&["  - x"], LineKind::Task), vec!["  - [ ] x"]);
    }

    #[test]
    fn tables_grow() {
        let table = ["| a | b |", "|---|---|", "| 1 | 2 |"];
        assert_eq!(table_with_row(&table).last().unwrap(), "|  |  |");
        assert_eq!(table_with_column(&table), vec!["| a | b |  |", "|---|---|---|", "| 1 | 2 |  |"]);
    }

    #[test]
    fn a_line_drawn_alone_keeps_its_hard_break_hidden() {
        assert_eq!(drawn_line("Umbruch\\", true), "Umbruch");
        assert_eq!(drawn_line("Umbruch\\", false), "Umbruch\\");
        assert_eq!(drawn_line("zwei \\\\", true), "zwei \\\\");
        assert_eq!(drawn_line("  - eingerückt", false), "- eingerückt");
        assert_eq!(drawn_line("    eingerückter Code", true), "eingerückter Code");
    }

    #[test]
    fn only_a_line_that_goes_on_loses_its_backslash() {
        assert!(continues_paragraph("Zeile\\", "weiter"));
        assert!(!continues_paragraph("Zeile\\", ""));
        assert!(!continues_paragraph("Pfad C:\\Daten\\", "- Punkt"));
        assert!(!continues_paragraph("Pfad C:\\Daten\\", "1. Punkt"));
        for next in ["# Titel", "> Zitat", "```rust", "~~~", "| a | b |"] {
            assert!(!continues_paragraph("Pfad\\", next), "{next}");
        }
        assert!(!continues_paragraph("# Titel\\", "weiter"));
        assert!(continues_paragraph("#Tag\\", "weiter"));
    }

    #[test]
    fn finds_the_formula_under_the_caret() {
        assert_eq!(formula_at("a $x^2$ b $y$", 4), Some("x^2"));
        assert_eq!(formula_at("a $x^2$ b $y$", 1), None);
        assert_eq!(formula_at("preis 5\\$ und $y$", 15), Some("y"));
        assert_eq!(formula_at("Preis 5$ und 7$ gesamt", 10), None);
    }
}
