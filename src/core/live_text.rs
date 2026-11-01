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

/// How deep a line stands while it is written, when Tab may just have moved it.
pub fn depth_while_writing(line: &str, drawn: usize) -> usize {
    let wide = columns(line);
    if wide == 0 { 0 } else { (wide / 2).max(usize::from(drawn > 0) * drawn.min(wide)).min(wide / 2 + drawn) }
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
    fn changes_in_different_places_merge() {
        let base = "a\nb\nc\nd\ne";
        assert_eq!(merge(base, "a!\nb\nc\nd\ne", "a\nb\nc\nd\ne!").unwrap(), "a!\nb\nc\nd\ne!");
        assert_eq!(merge(base, "x\na\nb\nc\nd\ne", "a\nb\nc\nd\ne\ny").unwrap(), "x\na\nb\nc\nd\ne\ny");
        assert_eq!(merge(base, "a\nb\nc\nd\ne", "a\nb!\nc\nd\ne").unwrap(), "a\nb!\nc\nd\ne");
        assert_eq!(merge(base, "a\nB\nc\nd\ne", "a\nb2\nc\nd\ne"), None);
        assert_eq!(merge(base, "a\nB\nc\nd\ne", "a\nB\nc\nd\ne").unwrap(), "a\nB\nc\nd\ne");
    }
}
