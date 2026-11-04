use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::*;
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use super::motion::Soft;
use super::selectable::{InlineKind, InlineSlot, LinkAction, selectable};
use super::theme::{Theme, control, font, note_size, radius, space, iconsize};
use crate::core::i18n::{tr, trf};
use crate::core::markdown::{self, Run, TokenKind};
use crate::core::vault::syntax::{self, CalloutHead, MarkColor};
use crate::core::vault::{frontmatter, index, links};

/// Called with the 0-based source line of a clicked task box.
pub type OnToggle = Rc<dyn Fn(usize, &mut App)>;

#[derive(Clone)]
enum Inline {
    Text(String, Style),
    Link(String, Style, String),
    /// Source, alternative text and the link around it.
    Image(String, String, Option<String>),
    Math(String),
    FootnoteRef(String),
    Tag(String),
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    mark: Option<MarkColor>,
    link: bool,
    underline: bool,
    small: bool,
    color: Option<u32>,
}

enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    Quote(Vec<Block>),
    Code(Option<String>, String),
    List { start: Option<u64>, items: Vec<Item> },
    Rule,
    Table { head: Vec<Vec<Inline>>, rows: Vec<Vec<Vec<Inline>>>, aligns: Vec<pulldown_cmark::Alignment> },
    Callout(CalloutHead, Vec<Inline>, Vec<Block>),
    Properties(Vec<frontmatter::Property>),
    Centered(Vec<Block>),
    Right(Vec<Block>),
    Details(Vec<Inline>, Vec<Block>, bool),
    Math(String),
    /// Takes its place in the count of blocks and shows nothing, like a `^id` on its own line.
    Nothing,
}

struct Item {
    /// The character in the box: ` `, `x`, or anything a theme gives its own meaning.
    task: Option<char>,
    line: usize,
    blocks: Vec<Block>,
}

/// Where a top-level block came from.
#[derive(Clone, Copy, PartialEq)]
pub struct BlockSource {
    pub line: usize,
    pub end_line: usize,
    pub hash: u64,
}

/// A parsed note, kept between frames so long notes don't re-parse while scrolling.
pub struct ParsedNote {
    /// Where each top-level block came from, in order.
    sources: Vec<BlockSource>,
    /// Every link in reading order, with the top-level block it sits in.
    links: Vec<(String, usize)>,
    blocks: Vec<Block>,
    footnotes: Vec<(String, Vec<Block>)>,
    /// The source line that first refers to a footnote.
    footnote_homes: HashMap<String, usize>,
    footnote_order: Vec<String>,
    rows: Vec<ReaderRow>,
}

/// How many list items or table rows one row of the reader holds, so a long list only builds what is on screen.
const PART: usize = 8;

/// One row of the reader: a top-level block, or a stretch of the items of a long list or the rows of a long table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReaderRow {
    pub block: usize,
    /// The list items or table rows drawn, when the block is drawn in parts.
    pub part: Option<(usize, usize)>,
    pub line: usize,
    pub end_line: usize,
    pub hash: u64,
    /// The links of the note before this row.
    links_before: usize,
    /// Where this row starts counting what it draws, growing in reading order.
    order: u64,
}

struct Breaks(Vec<usize>);

impl Breaks {
    fn of(src: &str) -> Self {
        Breaks(src.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i).collect())
    }

    fn line_of(&self, offset: usize) -> usize {
        self.0.partition_point(|at| *at < offset)
    }
}

/// Parses Markdown into blocks with source lines for task items.
pub fn parse(src: &str) -> ParsedNote {
    crate::core::vault::latex::learn_macros(src);
    let prepared = syntax::prepare(src);
    let src = prepared.as_str();
    let strict = index::current().config.strict_line_breaks;
    let mut metadata: Option<String> = None;
    let mut stack: Vec<(Option<Tag>, Vec<Block>, Vec<Inline>)> = vec![(None, Vec::new(), Vec::new())];
    let mut style = Style::default();
    let mut link: Option<String> = None;
    let mut items: Vec<Vec<Item>> = Vec::new();
    let mut item_meta: Vec<(Option<char>, usize)> = Vec::new();
    let mut table: Option<(Vec<Vec<Inline>>, Vec<Vec<Vec<Inline>>>, Vec<pulldown_cmark::Alignment>, bool)> = None;
    let mut row: Vec<Vec<Inline>> = Vec::new();
    let mut code: Option<(Option<String>, String)> = None;
    let mut image: Option<(String, String)> = None;
    let mut footnotes: Vec<(String, Vec<Block>)> = Vec::new();
    let mut footnote_order: Vec<String> = Vec::new();
    let mut html_block: Option<String> = None;
    let flush_inline = |stack: &mut Vec<(Option<Tag>, Vec<Block>, Vec<Inline>)>| {
        let top = stack.last_mut().unwrap();
        if !top.2.is_empty() {
            let inl = std::mem::take(&mut top.2);
            top.1.push(Block::Paragraph(inl));
        }
    };
    let mut sources: Vec<BlockSource> = Vec::new();
    let mut footnote_homes: HashMap<String, usize> = HashMap::new();
    let mut last_range = 0..0;
    let mut bare_wikilink = false;
    let mut raised: Option<fn(&str) -> String> = None;
    let breaks = Breaks::of(src);
    for (event, range) in Parser::new_ext(src, markdown::options()).into_offset_iter() {
        note_sources(src, &breaks, &stack, &last_range, &mut sources);
        last_range = range.clone();
        if let Some(buf) = metadata.as_mut() {
            match event {
                Event::Text(t) => buf.push_str(&t),
                Event::End(TagEnd::MetadataBlock(_)) => {
                    let text = format!("---\n{}\n---\n", metadata.take().unwrap_or_default());
                    let props = frontmatter::properties(&text).unwrap_or_default();
                    stack.last_mut().unwrap().1.push(Block::Properties(props));
                }
                _ => {}
            }
            continue;
        }
        if let Some((_, buf)) = code.as_mut() {
            match event {
                Event::Text(t) => buf.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, buf) = code.take().unwrap();
                    stack.last_mut().unwrap().1.push(Block::Code(lang, buf.strip_suffix('\n').unwrap_or(&buf).to_string()));
                }
                _ => {}
            }
            continue;
        }
        if let Some((_, alt)) = image.as_mut() {
            match event {
                Event::Text(t) => alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    let (src, alt) = image.take().unwrap();
                    stack.last_mut().unwrap().2.push(Inline::Image(src, alt, link.clone()));
                }
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::MetadataBlock(_) => metadata = Some(String::new()),
                Tag::Paragraph | Tag::Heading { .. } | Tag::BlockQuote(_) | Tag::FootnoteDefinition(_) => {
                    flush_inline(&mut stack);
                    stack.push((Some(tag), Vec::new(), Vec::new()));
                }
                Tag::HtmlBlock => {
                    flush_inline(&mut stack);
                    html_block = Some(String::new());
                }
                Tag::Item => {
                    flush_inline(&mut stack);
                    item_meta.push((None, breaks.line_of(range.start)));
                    stack.push((Some(Tag::Item), Vec::new(), Vec::new()));
                }
                Tag::List(_) => {
                    flush_inline(&mut stack);
                    items.push(Vec::new());
                    stack.push((Some(tag), Vec::new(), Vec::new()));
                }
                Tag::CodeBlock(kind) => {
                    flush_inline(&mut stack);
                    let lang = match kind {
                        CodeBlockKind::Fenced(l) if !l.is_empty() => l.split_whitespace().next().map(String::from),
                        _ => None,
                    };
                    code = Some((lang, String::new()));
                }
                Tag::Table(a) => {
                    flush_inline(&mut stack);
                    table = Some((Vec::new(), Vec::new(), a, false));
                }
                Tag::TableHead => {
                    if let Some(t) = table.as_mut() {
                        t.3 = true;
                    }
                    row.clear();
                }
                Tag::TableRow => row.clear(),
                Tag::TableCell => stack.push((Some(Tag::TableCell), Vec::new(), Vec::new())),
                Tag::Emphasis => style.italic = true,
                Tag::Strong => style.bold = true,
                Tag::Strikethrough => style.strike = true,
                Tag::Link { dest_url, link_type, .. } => {
                    style.link = true;
                    bare_wikilink = matches!(link_type, pulldown_cmark::LinkType::WikiLink { has_pothole: false });
                    link = Some(dest_url.to_string());
                }
                Tag::Image { dest_url, .. } => image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph => {
                    let (_, mut inner, mut inl) = stack.pop().unwrap();
                    join_marks(&mut inl);
                    let had_blocks = !inner.is_empty();
                    if had_blocks {
                        trim_beside_math(&mut inner);
                        if matches!(inner.last(), Some(Block::Math(_))) {
                            trim_inlines(&mut inl, true);
                        }
                    }
                    stack.last_mut().unwrap().1.extend(inner);
                    if !(had_blocks && inl.is_empty()) {
                        let block = if strip_block_id(&mut inl) && plain_text(&inl).trim().is_empty() { Block::Nothing } else { Block::Paragraph(inl) };
                        stack.last_mut().unwrap().1.push(block);
                    }
                }
                TagEnd::Heading(level) => {
                    let (_, mut inner, mut inl) = stack.pop().unwrap();
                    let mut head = match inner.first() {
                        None => std::mem::take(&mut inl),
                        Some(Block::Paragraph(_)) => match inner.remove(0) {
                            Block::Paragraph(first) => first,
                            _ => Vec::new(),
                        },
                        Some(_) => Vec::new(),
                    };
                    join_marks(&mut head);
                    join_marks(&mut inl);
                    if !inl.is_empty() {
                        inner.push(Block::Paragraph(inl));
                    }
                    if !inner.is_empty() {
                        trim_inlines(&mut head, false);
                        trim_beside_math(&mut inner);
                    }
                    stack.last_mut().unwrap().1.push(Block::Heading(markdown::heading_number(level), head));
                    stack.last_mut().unwrap().1.extend(inner);
                }
                TagEnd::BlockQuote(_) => {
                    flush_inline(&mut stack);
                    let (_, mut blocks, _) = stack.pop().unwrap();
                    trim_beside_math(&mut blocks);
                    stack.last_mut().unwrap().1.push(into_callout(blocks));
                }
                TagEnd::FootnoteDefinition => {
                    flush_inline(&mut stack);
                    if let (Some(Tag::FootnoteDefinition(id)), mut blocks, _) = stack.pop().unwrap() {
                        trim_beside_math(&mut blocks);
                        footnotes.push((id.to_string(), blocks));
                    }
                }
                TagEnd::Item => {
                    flush_inline(&mut stack);
                    let (_, mut blocks, _) = stack.pop().unwrap();
                    trim_beside_math(&mut blocks);
                    if let Some(Block::Paragraph(inl)) = blocks.iter_mut().find(|b| matches!(b, Block::Paragraph(_))) {
                        strip_block_id(inl);
                    }
                    let (mut task, line) = item_meta.pop().unwrap_or((None, 0));
                    if task.is_none() {
                        task = take_task_marker(&mut blocks);
                    }
                    if let Some(list) = items.last_mut() {
                        list.push(Item { task, line, blocks });
                    }
                }
                TagEnd::List(_) => {
                    let (tag, _, _) = stack.pop().unwrap();
                    let start = if let Some(Tag::List(s)) = tag { s } else { None };
                    let list = items.pop().unwrap_or_default();
                    stack.last_mut().unwrap().1.push(Block::List { start, items: list });
                }
                TagEnd::TableCell => {
                    let (_, _, inl) = stack.pop().unwrap();
                    row.push(inl);
                }
                TagEnd::TableHead => {
                    if let Some(t) = table.as_mut() {
                        t.0 = std::mem::take(&mut row);
                        t.3 = false;
                    }
                }
                TagEnd::TableRow => {
                    if let Some(t) = table.as_mut() {
                        t.1.push(std::mem::take(&mut row));
                    }
                }
                TagEnd::Table => {
                    if let Some((head, rows, aligns, _)) = table.take() {
                        stack.last_mut().unwrap().1.push(Block::Table { head, rows, aligns });
                    }
                }
                TagEnd::HtmlBlock => {
                    let found = html_blocks(&html_block.take().unwrap_or_default());
                    stack.last_mut().unwrap().1.extend(found);
                }
                TagEnd::Emphasis => style.italic = false,
                TagEnd::Strong => style.bold = false,
                TagEnd::Strikethrough => style.strike = false,
                TagEnd::Link => {
                    style.link = false;
                    link = None;
                }
                _ => {}
            },
            Event::Text(t) => {
                let lifted = raised.map(|lift| lift(&t));
                let t = lifted.as_deref().unwrap_or(&t);
                for r in markdown::runs(t) {
                    let target = &mut stack.last_mut().unwrap().2;
                    match r {
                        Run::Text(s) => match &link {
                            Some(url) => {
                                let shown = if bare_wikilink { wikilink_label(&s) } else { s };
                                target.push(Inline::Link(shown, style, url.clone()));
                            }
                            None => {
                                let spaced = collapse_spaces(&s);
                                let after_space = !style.code && matches!(target.last(), Some(Inline::Text(t, st)) if !st.code && t.ends_with(' '));
                                push_with_autolinks(target, if after_space { spaced.trim_start_matches(' ') } else { &spaced }, style);
                            }
                        },
                        Run::Mark(s, color) => target.push(Inline::Text(s, Style { mark: Some(color), ..style })),
                        Run::Tag(s) => match &link {
                            Some(url) => {
                                let written = format!("#{}", s.trim_start_matches('#'));
                                target.push(Inline::Link(if bare_wikilink { wikilink_label(&written) } else { written }, style, url.clone()));
                            }
                            None => target.push(Inline::Tag(s)),
                        },
                    }
                }
            }
            Event::Code(c) => stack.last_mut().unwrap().2.push(match &link {
                Some(url) => Inline::Link(c.to_string(), Style { code: true, ..style }, url.clone()),
                None => Inline::Text(c.to_string(), Style { code: true, ..style }),
            }),
            Event::SoftBreak => stack.last_mut().unwrap().2.push(Inline::Text(if strict { " " } else { "\n" }.into(), style)),
            Event::InlineMath(m) => stack.last_mut().unwrap().2.push(Inline::Math(m.to_string())),
            Event::DisplayMath(m) => {
                flush_inline(&mut stack);
                stack.last_mut().unwrap().1.push(Block::Math(m.to_string()));
            }
            Event::HardBreak => stack.last_mut().unwrap().2.push(Inline::Text("\n".into(), style)),
            Event::Rule => {
                flush_inline(&mut stack);
                stack.last_mut().unwrap().1.push(Block::Rule);
            }
            Event::Html(h) => {
                if let Some(buffer) = html_block.as_mut() {
                    buffer.push_str(&h);
                } else {
                    flush_inline(&mut stack);
                    let found = html_blocks(&h);
                    stack.last_mut().unwrap().1.extend(found);
                }
            }
            Event::InlineHtml(h) => match h.trim().to_lowercase().as_str() {
                "<br>" | "<br/>" | "<br />" => stack.last_mut().unwrap().2.push(Inline::Text("\n".into(), style)),
                "<b>" | "<strong>" => style.bold = true,
                "</b>" | "</strong>" => style.bold = false,
                "<i>" | "<em>" => style.italic = true,
                "</i>" | "</em>" => style.italic = false,
                "<s>" | "<del>" => style.strike = true,
                "</s>" | "</del>" => style.strike = false,
                "<u>" | "<ins>" | "<abbr>" => style.underline = true,
                "</u>" | "</ins>" | "</abbr>" => style.underline = false,
                "<small>" => style.small = true,
                "</small>" => style.small = false,
                "<sup>" => raised = Some(superscript),
                "<sub>" => raised = Some(subscript),
                "</sup>" | "</sub>" => raised = None,
                "</span>" | "</font>" => style.color = None,
                "</a>" => {
                    style.link = false;
                    link = None;
                }
                tag if tag.starts_with("<span") || tag.starts_with("<font") => style.color = css_color_of(tag),
                tag if tag.starts_with("<abbr") => style.underline = true,
                tag if tag.starts_with("<a ") => {
                    if let Some(href) = html_attribute(h.as_ref(), "href") {
                        style.link = true;
                        link = Some(href);
                    }
                }
                "<mark>" => style.mark = Some(MarkColor::Default),
                "</mark>" => style.mark = None,
                "<kbd>" | "<code>" => style.code = true,
                "</kbd>" | "</code>" => style.code = false,
                _ => {}
            },
            Event::TaskListMarker(done) => {
                if let Some(m) = item_meta.last_mut() {
                    m.0 = Some(if done { 'x' } else { ' ' });
                }
            }
            Event::FootnoteReference(id) => {
                if !footnote_order.iter().any(|f| f == id.as_ref()) {
                    footnote_order.push(id.to_string());
                    footnote_homes.insert(id.to_string(), breaks.line_of(range.start));
                }
                stack.last_mut().unwrap().2.push(Inline::FootnoteRef(id.to_string()));
            }
        }
    }
    flush_inline(&mut stack);
    note_sources(src, &breaks, &stack, &last_range, &mut sources);
    let blocks = stack.pop().map(|s| s.1).unwrap_or_default();
    let mut links = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        collect_links(std::slice::from_ref(block), i, &mut links);
    }
    let rows = reader_rows(src, &breaks, &blocks, &sources);
    ParsedNote { sources, links, blocks, footnotes, footnote_homes, footnote_order, rows }
}

/// Splits long lists and tables into rows of `PART` items, each knowing its lines, links and drawing order.
fn reader_rows(src: &str, breaks: &Breaks, blocks: &[Block], sources: &[BlockSource]) -> Vec<ReaderRow> {
    use std::hash::{Hash, Hasher};
    let line_start = |line: usize| if line == 0 { 0 } else { breaks.0.get(line - 1).map_or(src.len(), |b| b + 1) };
    let mut rows = Vec::new();
    let mut links_before = 0;
    for (block, (b, source)) in blocks.iter().zip(sources).enumerate() {
        let order = (block as u64 + 1) * 100_000;
        let links_in = |blocks: &[Block]| {
            let mut found = Vec::new();
            collect_links(blocks, block, &mut found);
            found.len()
        };
        let parts: Vec<(usize, usize, usize, usize, u64)> = match b {
            Block::List { items, .. } if items.len() > PART => (0..items.len())
                .step_by(PART)
                .map(|from| {
                    let part = &items[from..(from + PART).min(items.len())];
                    let line = if from == 0 { source.line } else { part[0].line };
                    let links = part.iter().map(|i| links_in(&i.blocks)).sum();
                    (from, from + part.len(), line, links, part.iter().map(|i| order_room(&i.blocks)).sum())
                })
                .collect(),
            Block::Table { head, rows: body, .. } if body.len() > PART => (0..body.len())
                .step_by(PART)
                .map(|from| {
                    let part = &body[from..(from + PART).min(body.len())];
                    let line = if from == 0 { source.line } else { source.line + 2 + from };
                    let count = |cells: &mut dyn Iterator<Item = &Vec<Inline>>| cells.flatten().filter(|i| matches!(i, Inline::Link(..))).count();
                    let links = count(&mut part.iter().flatten()) + if from == 0 { count(&mut head.iter()) } else { 0 };
                    let room = part.iter().flatten().chain(if from == 0 { head.iter() } else { [].iter() }).map(|c| 1 + inline_room(c)).sum::<u64>() + 1;
                    (from, from + part.len(), line, links, room)
                })
                .collect(),
            _ => Vec::new(),
        };
        if parts.is_empty() {
            rows.push(ReaderRow { block, part: None, line: source.line, end_line: source.end_line, hash: source.hash, links_before, order });
            links_before += links_in(std::slice::from_ref(b));
            continue;
        }
        let mut order = order;
        for (k, (from, to, line, links, room)) in parts.iter().copied().enumerate() {
            let end_line = parts.get(k + 1).map_or(source.end_line, |next| next.2).max(line);
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            src.get(line_start(line)..line_start(end_line)).unwrap_or_default().hash(&mut hash);
            (from, to, source.line, matches!(b, Block::Table { .. }).then_some(source.hash)).hash(&mut hash);
            if let Block::List { start, .. } = b {
                start.hash(&mut hash);
            }
            rows.push(ReaderRow { block, part: Some((from, to)), line, end_line, hash: hash.finish(), links_before, order });
            links_before += links;
            order += room + 1;
        }
    }
    rows
}

/// More than the things drawn for these inlines count, so the parts of one block number theirs without meeting.
fn inline_room(inlines: &[Inline]) -> u64 {
    let room = |src: &str| if Path::new(src.trim_end_matches('\\')).extension().is_some_and(|e| IMAGE_TYPES.contains(&e.to_string_lossy().to_lowercase().as_str())) { 2 } else { 1_000 };
    1 + inlines.iter().map(|i| if let Inline::Image(src, ..) = i { room(src) } else { 0 }).sum::<u64>()
}

/// More than the things drawn for these blocks count, as `inline_room` for blocks.
fn order_room(blocks: &[Block]) -> u64 {
    blocks
        .iter()
        .map(|b| match b {
            Block::Heading(_, inl) | Block::Paragraph(inl) => inline_room(inl),
            Block::Quote(inner) | Block::Centered(inner) | Block::Right(inner) => order_room(inner),
            Block::Callout(_, title, inner) | Block::Details(title, inner, _) => 1 + inline_room(title) + order_room(inner),
            Block::List { items, .. } => items.iter().map(|i| order_room(&i.blocks)).sum(),
            Block::Table { head, rows, .. } => 1 + head.iter().chain(rows.iter().flatten()).map(|c| 1 + inline_room(c)).sum::<u64>(),
            Block::Properties(props) => 1 + props.len() as u64,
            _ => 2,
        })
        .sum::<u64>()
        + 1
}

/// Drops the spaces and line breaks at the start or end of a paragraph's text.
fn trim_inlines(inlines: &mut Vec<Inline>, start: bool) {
    loop {
        let at = if start { 0 } else { inlines.len().wrapping_sub(1) };
        let Some(Inline::Text(text, style)) = inlines.get_mut(at) else { return };
        if style.code {
            return;
        }
        let trimmed = if start { text.trim_start() } else { text.trim_end() };
        if !trimmed.is_empty() {
            *text = trimmed.to_string();
            return;
        }
        inlines.remove(at);
    }
}

/// Text written around display math in one paragraph loses the space and break that join it to the formula.
fn trim_beside_math(blocks: &mut Vec<Block>) {
    for i in 0..blocks.len() {
        let (before, after) = (i > 0 && matches!(blocks[i - 1], Block::Math(_)), matches!(blocks.get(i + 1), Some(Block::Math(_))));
        if let Block::Paragraph(inl) = &mut blocks[i] {
            if before {
                trim_inlines(inl, true);
            }
            if after {
                trim_inlines(inl, false);
            }
        }
    }
    blocks.retain(|b| !matches!(b, Block::Paragraph(inl) if inl.is_empty()));
}

/// A note's source of `^id` at a paragraph's end disappears from what is shown.
/// Writes digits and common letters raised; what has no raised form stays as it is.
fn superscript(text: &str) -> String {
    text.chars().map(|c| match c {
        '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴', '5' => '⁵', '6' => '⁶', '7' => '⁷', '8' => '⁸', '9' => '⁹',
        '+' => '⁺', '-' => '⁻', '=' => '⁼', '(' => '⁽', ')' => '⁾', 'n' => 'ⁿ', 'i' => 'ⁱ',
        other => other,
    }).collect()
}

/// Writes digits lowered; what has no lowered form stays as it is.
fn subscript(text: &str) -> String {
    text.chars().map(|c| match c {
        '0' => '₀', '1' => '₁', '2' => '₂', '3' => '₃', '4' => '₄', '5' => '₅', '6' => '₆', '7' => '₇', '8' => '₈', '9' => '₉',
        '+' => '₊', '-' => '₋', '=' => '₌', '(' => '₍', ')' => '₎',
        other => other,
    }).collect()
}

/// What a link without its own label says: `Note#Heading` reads `Note > Heading`.
fn wikilink_label(target: &str) -> String {
    let (note, rest) = target.split_once('#').unwrap_or((target, ""));
    let parts: Vec<&str> = std::iter::once(note).chain(rest.split('#')).filter(|p| !p.is_empty()).collect();
    parts.join(" > ")
}

/// Runs of spaces in the source read as one, as in Obsidian's reading view.
fn collapse_spaces(text: &str) -> String {
    if !text.contains("  ") && !text.contains('\t') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        let space = c == ' ' || c == '\t';
        if !(space && last_space) {
            out.push(if space { ' ' } else { c });
        }
        last_space = space;
    }
    out
}

/// Bare web addresses in the text become links.
fn push_with_autolinks(target: &mut Vec<Inline>, text: &str, style: Style) {
    static SEPARATOR: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r" :::? ").unwrap());
    if !style.code && let Some(found) = SEPARATOR.find(text) {
        push_with_autolinks(target, &text[..found.start()], style);
        target.push(Inline::Text(found.as_str().to_string(), Style { color: Some(0x8E8E93), ..style }));
        push_with_autolinks(target, &text[found.end()..], style);
        return;
    }
    static URL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"https?://[^\s<>\]\)]+|\bwww\.[a-zA-Z0-9-]+\.[^\s<>\]\)]+|\b[\w.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9.-]+").unwrap());
    let mut copied = 0;
    let may_link = text.contains("://") || text.contains("www.") || text.contains('@');
    for hit in may_link.then(|| URL.find_iter(text)).into_iter().flatten() {
        let shown = hit.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"']);
        if hit.start() > copied {
            target.push(Inline::Text(text[copied..hit.start()].to_string(), style));
        }
        let address = if shown.starts_with("http") {
            shown.to_string()
        } else if shown.starts_with("www.") {
            format!("https://{shown}")
        } else {
            format!("mailto:{shown}")
        };
        target.push(Inline::Link(shown.to_string(), Style { link: true, ..style }, address));
        copied = hit.start() + shown.len();
    }
    if copied < text.len() {
        target.push(Inline::Text(text[copied..].to_string(), style));
    }
}

/// Marks whose `==` sit in different runs, as in `==**fett** Text==`.
fn join_marks(inlines: &mut Vec<Inline>) {
    loop {
        let found = |from: usize, inlines: &[Inline]| inlines.iter().enumerate().skip(from).find(|(_, i)| matches!(i, Inline::Text(t, st) if !st.code && t.contains("=="))).map(|(at, _)| at);
        let Some(open) = found(0, inlines) else { return };
        let Inline::Text(open_text, open_style) = inlines[open].clone() else { return };
        let split = open_text.find("==").unwrap_or(0);
        let after_open = &open_text[split + 2..];
        if after_open.contains("==") {
            return;
        }
        let Some(close) = found(open + 1, inlines) else { return };
        let Inline::Text(close_text, close_style) = inlines[close].clone() else { return };
        let end = close_text.find("==").unwrap_or(0);
        let mark = Some(MarkColor::Default);
        let mut replaced: Vec<Inline> = Vec::new();
        if split > 0 {
            replaced.push(Inline::Text(open_text[..split].to_string(), open_style));
        }
        if !after_open.is_empty() {
            replaced.push(Inline::Text(after_open.to_string(), Style { mark, ..open_style }));
        }
        for inner in &inlines[open + 1..close] {
            replaced.push(match inner {
                Inline::Text(t, st) => Inline::Text(t.clone(), Style { mark, ..*st }),
                Inline::Link(t, st, url) => Inline::Link(t.clone(), Style { mark, ..*st }, url.clone()),
                other => other.clone(),
            });
        }
        if end > 0 {
            replaced.push(Inline::Text(close_text[..end].to_string(), Style { mark, ..close_style }));
        }
        if end + 2 < close_text.len() {
            replaced.push(Inline::Text(close_text[end + 2..].to_string(), close_style));
        }
        inlines.splice(open..=close, replaced);
    }
}

/// Remembers where newly finished top-level blocks came from.
fn note_sources<T>(src: &str, breaks: &Breaks, stack: &[(T, Vec<Block>, Vec<Inline>)], range: &std::ops::Range<usize>, sources: &mut Vec<BlockSource>) {
    use std::hash::{Hash, Hasher};
    let [(_, blocks, _)] = stack else { return };
    while sources.len() < blocks.len() {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        src.get(range.clone()).unwrap_or_default().hash(&mut hash);
        let text = src.get(range.clone()).unwrap_or_default();
        let line = breaks.line_of(range.start);
        sources.push(BlockSource { line, end_line: line + text.trim_end_matches('\n').matches('\n').count() + 1, hash: hash.finish() });
    }
}

/// Finds where a place in drawn text sits in its source: the drawn text leaves out marks, the source has them.
pub fn source_offset(source: &str, drawn: &str, at: usize) -> usize {
    let cleaned = super::selectable::without_room(drawn.get(..at).unwrap_or(drawn));
    let first_words: String = drawn.chars().take_while(|c| !"\n*_`=~[]".contains(*c)).take(24).collect();
    let mut from = if first_words.trim().is_empty() { 0 } else { source.find(first_words.trim()).unwrap_or(0) };
    for c in cleaned.chars() {
        let window_end = (from + 400).min(source.len());
        let Some(tail) = source.get(from..window_end) else { break };
        if let Some(found) = tail.find(c) {
            from += found + c.len_utf8();
        }
    }
    from.min(source.len())
}

/// Lists a block's links in the order they are drawn.
fn collect_links(blocks: &[Block], top: usize, out: &mut Vec<(String, usize)>) {
    let mut inline = |inl: &[Inline], out: &mut Vec<(String, usize)>| {
        for i in inl {
            if let Inline::Link(_, _, url) = i {
                out.push((url.clone(), top));
            }
        }
    };
    for block in blocks {
        match block {
            Block::Heading(_, inl) | Block::Paragraph(inl) => inline(inl, out),
            Block::Quote(inner) | Block::Callout(_, _, inner) | Block::Centered(inner) | Block::Right(inner) | Block::Details(_, inner, _) => collect_links(inner, top, out),
            Block::List { items, .. } => {
                for item in items {
                    collect_links(&item.blocks, top, out);
                }
            }
            Block::Table { head, rows, .. } => {
                for cell in head.iter().chain(rows.iter().flatten()) {
                    inline(cell, out);
                }
            }
            _ => {}
        }
    }
}

fn strip_block_id(inlines: &mut Vec<Inline>) -> bool {
    static ID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?:^|\s)\^[A-Za-z0-9-]+\s*$").unwrap());
    let Some(Inline::Text(last, _)) = inlines.last_mut() else { return false };
    let Some(m) = ID.find(last) else { return false };
    last.truncate(m.start());
    true
}

/// Turns a quote starting with `[!type]` into a callout; its first line is the head.
fn into_callout(mut blocks: Vec<Block>) -> Block {
    let head = match blocks.first() {
        Some(Block::Paragraph(inl)) => {
            let text = plain_text(inl);
            syntax::callout_head(text.split('\n').next().unwrap_or_default())
        }
        _ => None,
    };
    let Some(head) = head else { return Block::Quote(blocks) };
    let mut title: Vec<Inline> = Vec::new();
    if let Block::Paragraph(inl) = &mut blocks[0] {
        let mut rest = Vec::new();
        let mut found = false;
        for item in inl.drain(..) {
            if found {
                rest.push(item);
            } else if let Inline::Text(t, st) = &item
                && let Some((before, after)) = t.split_once('\n')
            {
                found = true;
                if !before.is_empty() {
                    title.push(Inline::Text(before.to_string(), *st));
                }
                if !after.is_empty() {
                    rest.push(Inline::Text(after.to_string(), *st));
                }
            } else {
                title.push(item);
            }
        }
        *inl = rest;
        let mut skipped = false;
        let mut after_close = false;
        title.retain_mut(|piece| {
            if after_close {
                after_close = false;
                if let Inline::Text(text, _) = piece {
                    *text = text.trim_start_matches(['+', '-']).trim_start().to_string();
                    return !text.is_empty();
                }
                return true;
            }
            if skipped {
                return true;
            }
            let Inline::Text(text, _) = piece else { return true };
            match text.find(']') {
                Some(close) => {
                    skipped = true;
                    *text = text[close + 1..].trim_start_matches(['+', '-']).trim_start().to_string();
                    after_close = text.is_empty();
                    !text.is_empty()
                }
                None => false,
            }
        });
    }
    if matches!(&blocks[0], Block::Paragraph(inl) if inl.is_empty()) {
        blocks.remove(0);
    }
    Block::Callout(head, title, blocks)
}

/// A list item starting with `[c] `: any character but a space means done, as in Obsidian.
fn take_task_marker(blocks: &mut [Block]) -> Option<char> {
    let Some(Block::Paragraph(inl)) = blocks.first_mut() else { return None };
    let text = plain_text(inl);
    let mut chars = text.chars();
    let (Some('['), Some(mark), Some(']'), Some(' ')) = (chars.next(), chars.next(), chars.next(), chars.next()) else { return None };
    let mut skip = 3 + mark.len_utf8();
    let mut rest = Vec::new();
    for item in inl.drain(..) {
        match item {
            Inline::Text(t, st) if skip > 0 => {
                if t.len() <= skip {
                    skip -= t.len();
                } else {
                    rest.push(Inline::Text(t[skip..].to_string(), st));
                    skip = 0;
                }
            }
            other => rest.push(other),
        }
    }
    *inl = rest;
    Some(mark)
}

/// Text of an HTML block: comments and tags go, Markdown inside stays as it is.
/// One piece of HTML: a tag with its attributes and content, or text.
enum HtmlNode {
    Text(String),
    Element(String, String, Vec<HtmlNode>),
}

/// The value of an attribute in a tag.
fn html_attribute(tag: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"(?i)\b{name}\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#);
    let c = regex::Regex::new(&pattern).ok()?.captures(tag)?;
    c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)).map(|m| m.as_str().to_string())
}

/// The colour a tag's style or `color` attribute names.
fn css_color_of(tag: &str) -> Option<u32> {
    let style = html_attribute(tag, "style").unwrap_or_default();
    let named = style.split(';').find_map(|part| part.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("color")).map(|(_, v)| v.trim().to_string())).or_else(|| html_attribute(tag, "color"))?;
    let value = named.to_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        let full = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect() } else { hex.to_string() };
        return u32::from_str_radix(&full, 16).ok().filter(|_| full.len() == 6);
    }
    Some(match value.as_str() {
        "red" => 0xe5484d,
        "green" => 0x30a46c,
        "blue" => 0x3b82f6,
        "orange" => 0xf76b15,
        "yellow" => 0xeab308,
        "purple" => 0x8e4ec6,
        "pink" => 0xe93d82,
        "gray" | "grey" => 0x8b8d98,
        "white" => 0xffffff,
        "black" => 0x000000,
        _ => return None,
    })
}

/// Reads HTML as a tree. Broken HTML still gives its text: an unknown closing tag is skipped, open tags end with their parent.
fn html_tree(html: &str) -> Vec<HtmlNode> {
    static TOKEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r#"<(/?)([a-zA-Z][a-zA-Z0-9]*)((?:[^>"']|"[^"]*"|'[^']*')*)>"#).unwrap());
    const SINGLE: [&str; 5] = ["br", "hr", "img", "input", "wbr"];
    let mut frames: Vec<(String, String, Vec<HtmlNode>)> = vec![(String::new(), String::new(), Vec::new())];
    let mut copied = 0;
    for c in TOKEN.captures_iter(html) {
        let whole = c.get(0).unwrap();
        if whole.start() > copied {
            frames.last_mut().unwrap().2.push(HtmlNode::Text(html[copied..whole.start()].to_string()));
        }
        copied = whole.end();
        let name = c[2].to_lowercase();
        if &c[1] == "/" {
            if let Some(at) = frames.iter().rposition(|f| f.0 == name).filter(|at| *at > 0) {
                while frames.len() > at {
                    let (tag, attributes, children) = frames.pop().unwrap();
                    frames.last_mut().unwrap().2.push(HtmlNode::Element(tag, attributes, children));
                }
            }
        } else if SINGLE.contains(&name.as_str()) || c[3].trim_end().ends_with('/') {
            frames.last_mut().unwrap().2.push(HtmlNode::Element(name, c[3].to_string(), Vec::new()));
        } else {
            frames.push((name, c[3].to_string(), Vec::new()));
        }
    }
    if copied < html.len() {
        frames.last_mut().unwrap().2.push(HtmlNode::Text(html[copied..].to_string()));
    }
    while frames.len() > 1 {
        let (tag, attributes, children) = frames.pop().unwrap();
        frames.last_mut().unwrap().2.push(HtmlNode::Element(tag, attributes, children));
    }
    frames.pop().map(|f| f.2).unwrap_or_default()
}

fn html_decoded(text: &str) -> String {
    text.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'")
}

/// Text and inline tags as the reader's inline pieces.
fn html_inlines(nodes: &[HtmlNode], style: Style, link: Option<&str>, lift: Option<fn(&str) -> String>, out: &mut Vec<Inline>) {
    for node in nodes {
        match node {
            HtmlNode::Text(t) => {
                let spaced = collapse_spaces(&t.replace(['\n', '\r'], " "));
                let decoded = html_decoded(&spaced);
                let shown = lift.map_or(decoded.clone(), |f| f(&decoded));
                if !shown.is_empty() {
                    out.push(match link {
                        Some(url) => Inline::Link(shown, Style { link: true, ..style }, url.to_string()),
                        None => Inline::Text(shown, style),
                    });
                }
            }
            HtmlNode::Element(tag, attributes, children) => {
                let mut inner = style;
                let mut inner_link = link.map(str::to_string);
                let mut inner_lift = lift;
                match tag.as_str() {
                    "br" => out.push(Inline::Text("\n".into(), style)),
                    "img" => {
                        let src = html_attribute(attributes, "src").unwrap_or_default();
                        let (w, h) = (html_attribute(attributes, "width"), html_attribute(attributes, "height"));
                        let size = match (w, h) {
                            (Some(w), Some(h)) => format!("{w}x{h}"),
                            (Some(w), None) => w,
                            _ => String::new(),
                        };
                        out.push(Inline::Image(src, size, link.map(str::to_string)));
                    }
                    "b" | "strong" => inner.bold = true,
                    "i" | "em" | "cite" | "var" => inner.italic = true,
                    "s" | "del" | "strike" => inner.strike = true,
                    "u" | "ins" | "abbr" => inner.underline = true,
                    "code" | "kbd" | "samp" | "tt" => inner.code = true,
                    "mark" => inner.mark = Some(MarkColor::Default),
                    "small" => inner.small = true,
                    "sup" => inner_lift = Some(superscript),
                    "sub" => inner_lift = Some(subscript),
                    "a" => inner_link = html_attribute(attributes, "href").or(inner_link),
                    _ => {}
                }
                if let Some(color) = css_color_of(attributes) {
                    inner.color = Some(color);
                }
                html_inlines(children, inner, inner_link.as_deref(), inner_lift, out);
            }
        }
    }
}

fn html_is_block(tag: &str) -> bool {
    matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "section" | "article" | "aside" | "header" | "footer" | "main" | "nav" | "figure" | "figcaption" | "center" | "iframe" | "audio" | "video" | "embed" | "object" | "blockquote" | "ul" | "ol" | "table" | "hr" | "details" | "pre" | "dl" | "dt" | "dd")
}

/// HTML as the reader's blocks. Script, style and comments never show.
fn html_blocks(html: &str) -> Vec<Block> {
    static HIDDEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?is)<script\b.*?(?:</script\s*>|$)|<style\b.*?(?:</style\s*>|$)|<!--.*?(?:-->|$)").unwrap());
    let cleaned = HIDDEN.replace_all(html, "");
    html_nodes_to_blocks(&html_tree(&cleaned))
}

fn html_nodes_to_blocks(nodes: &[HtmlNode]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut run: Vec<Inline> = Vec::new();
    let flush = |run: &mut Vec<Inline>, blocks: &mut Vec<Block>| {
        let taken = std::mem::take(run);
        if !plain_text(&taken).trim().is_empty() || taken.iter().any(|i| matches!(i, Inline::Image(..))) {
            blocks.push(Block::Paragraph(taken));
        }
    };
    for node in nodes {
        let HtmlNode::Element(tag, attributes, children) = node else {
            html_inlines(std::slice::from_ref(node), Style::default(), None, None, &mut run);
            continue;
        };
        if !html_is_block(tag) {
            html_inlines(std::slice::from_ref(node), Style::default(), None, None, &mut run);
            continue;
        }
        flush(&mut run, &mut blocks);
        match tag.as_str() {
            "hr" => blocks.push(Block::Rule),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let mut inl = Vec::new();
                html_inlines(children, Style::default(), None, None, &mut inl);
                blocks.push(Block::Heading(tag[1..].parse().unwrap_or(1), inl));
            }
            "ul" | "ol" => {
                let items = children
                    .iter()
                    .filter_map(|c| match c {
                        HtmlNode::Element(t, _, inner) if t == "li" => Some(Item { task: None, line: 0, blocks: html_nodes_to_blocks(inner) }),
                        _ => None,
                    })
                    .collect();
                blocks.push(Block::List { start: (tag == "ol").then_some(1), items });
            }
            "table" => blocks.extend(html_table(children)),
            "pre" => {
                let mut inl = Vec::new();
                html_inlines(children, Style::default(), None, None, &mut inl);
                blocks.push(Block::Code(None, plain_text(&inl)));
            }
            "details" => {
                let summary = children.iter().find_map(|c| match c {
                    HtmlNode::Element(t, _, inner) if t == "summary" => Some(inner),
                    _ => None,
                });
                let mut title = Vec::new();
                if let Some(inner) = summary {
                    html_inlines(inner, Style::default(), None, None, &mut title);
                } else {
                    title.push(Inline::Text(tr("Details"), Style::default()));
                }
                let mut inner = Vec::new();
                for piece in children.iter().filter(|c| !matches!(c, HtmlNode::Element(t, ..) if t == "summary")) {
                    inner.extend(html_nodes_to_blocks(std::slice::from_ref(piece)));
                }
                blocks.push(Block::Details(title, inner, attributes.to_lowercase().contains("open")));
            }
            "blockquote" => blocks.push(Block::Quote(html_nodes_to_blocks(children))),
            "audio" | "video" => {
                if let Some(src) = html_attribute(attributes, "src") {
                    blocks.push(Block::Paragraph(vec![Inline::Image(src, String::new(), None)]));
                }
            }
            "iframe" | "embed" | "object" => {
                if let Some(src) = html_attribute(attributes, "src").or_else(|| html_attribute(attributes, "data")) {
                    let shown = src.split("://").nth(1).unwrap_or(&src).trim_end_matches('/').to_string();
                    blocks.push(Block::Paragraph(vec![Inline::Link(trf("Eingebettete Seite: %@", &[&shown]), Style { link: true, ..Style::default() }, src)]));
                }
            }
            _ => {
                let align = html_attribute(attributes, "align").unwrap_or_default().to_lowercase();
                let centered = tag == "center" || align == "center" || html_attribute(attributes, "style").is_some_and(|s| s.replace(' ', "").to_lowercase().contains("text-align:center"));
                let right = align == "right" || html_attribute(attributes, "style").is_some_and(|s| s.replace(' ', "").to_lowercase().contains("text-align:right"));
                let inner = html_nodes_to_blocks(children);
                if centered {
                    blocks.push(Block::Centered(inner));
                } else if right {
                    blocks.push(Block::Right(inner));
                } else {
                    blocks.extend(inner);
                }
            }
        }
    }
    flush(&mut run, &mut blocks);
    blocks
}

fn html_table(children: &[HtmlNode]) -> Vec<Block> {
    fn rows_of<'a>(nodes: &'a [HtmlNode], out: &mut Vec<&'a Vec<HtmlNode>>) {
        for node in nodes {
            if let HtmlNode::Element(tag, _, inner) = node {
                match tag.as_str() {
                    "tr" => out.push(inner),
                    "thead" | "tbody" | "tfoot" => rows_of(inner, out),
                    _ => {}
                }
            }
        }
    }
    let mut rows = Vec::new();
    rows_of(children, &mut rows);
    let cells = |row: &Vec<HtmlNode>| -> Vec<Vec<Inline>> {
        row.iter()
            .filter_map(|c| match c {
                HtmlNode::Element(t, _, inner) if t == "td" || t == "th" => {
                    let mut inl = Vec::new();
                    html_inlines(inner, Style::default(), None, None, &mut inl);
                    Some(inl)
                }
                _ => None,
            })
            .collect()
    };
    let Some((first, rest)) = rows.split_first() else { return Vec::new() };
    let head = cells(first);
    let aligns = vec![pulldown_cmark::Alignment::None; head.len()];
    vec![Block::Table { head, rows: rest.iter().map(|r| cells(r)).collect(), aligns }]
}

fn html_text(html: &str) -> String {
    static COMMENT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?s)<!--.*?-->").unwrap());
    static BREAK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)<br\s*/?>|</(p|div|li|tr|h[1-6])>").unwrap());
    static TAG: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>]+>").unwrap());
    static HIDDEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?is)<script\b.*?(?:</script\s*>|$)|<style\b.*?(?:</style\s*>|$)").unwrap());
    let text = HIDDEN.replace_all(html, "");
    let text = COMMENT.replace_all(&text, "");
    let text = BREAK.replace_all(&text, "\n");
    TAG.replace_all(&text, "").replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
}

/// Keyboard walk through a note's links.
#[derive(Clone)]
pub struct LinkFocus {
    pub index: Option<usize>,
    /// The note being shown, so it cannot embed itself.
    pub note: Option<PathBuf>,
    /// The list the note is drawn in, for jumps inside it.
    pub list: Option<ListState>,
    /// Plain clicks on text in a top-level block.
    pub text_click: Option<BlockClick>,
    /// Rewrites the source of a list item in the live view.
    pub edit_item: Option<EditItem>,
    /// What the reader has folded, kept across sessions.
    pub folds: Folds,
    /// The text the note's find bar looks for.
    pub find: Option<Rc<FindMarks>>,
}

/// What the reader marks while the find bar is open.
pub struct FindMarks {
    pub query: String,
    pub exact: bool,
    /// The top-level block holding the current match, and which match of that block it is.
    pub block: usize,
    pub nth: usize,
    /// The source line of the current match, and which match of that line it is, for the live view.
    pub line: usize,
    pub line_nth: usize,
}

struct FindDraw {
    marks: Rc<FindMarks>,
    current: Option<usize>,
    seen: Cell<usize>,
}

impl FindDraw {
    fn new(focus: Option<&LinkFocus>, current: impl Fn(&FindMarks) -> Option<usize>) -> Option<Self> {
        let marks = focus?.find.clone()?;
        let current = current(&marks);
        Some(Self { marks, current, seen: Cell::new(0) })
    }

    fn overlay(&self, text: &str, highlights: Vec<(std::ops::Range<usize>, HighlightStyle)>, t: &Theme) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
        let found = crate::core::find::matches(text, &self.marks.query, self.marks.exact);
        let first = self.seen.get();
        self.seen.set(first + found.len());
        let fills: Vec<(std::ops::Range<usize>, Hsla)> = found.into_iter().enumerate().map(|(i, r)| (r, find_fill(self.current == Some(first + i), t))).collect();
        with_fills(highlights, &fills)
    }
}

/// The background of a find match; the current one is stronger.
pub fn find_fill(current: bool, t: &Theme) -> Hsla {
    t.orange.opacity(if current { 0.7 } else { 0.28 })
}

/// `highlights` with a background laid under each fill range, split where they overlap, so no two ranges overlap.
fn with_fills(highlights: Vec<(std::ops::Range<usize>, HighlightStyle)>, fills: &[(std::ops::Range<usize>, Hsla)]) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    if fills.is_empty() {
        return highlights;
    }
    let mut cuts: Vec<usize> = highlights.iter().flat_map(|(r, _)| [r.start, r.end]).chain(fills.iter().flat_map(|(r, _)| [r.start, r.end])).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut out: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let base = highlights.iter().find(|(r, _)| r.start <= from && to <= r.end).map(|(_, h)| *h);
        let fill = fills.iter().find(|(r, _)| r.start <= from && to <= r.end).map(|(_, c)| *c);
        let style = match (base, fill) {
            (None, None) => continue,
            (Some(h), None) => h,
            (h, Some(c)) => HighlightStyle { background_color: Some(c), ..h.unwrap_or_default() },
        };
        match out.last_mut() {
            Some((r, h)) if r.end == from && *h == style => r.end = to,
            _ => out.push((from..to, style)),
        }
    }
    out
}

/// Called with a top-level block, the drawn text and the byte clicked.
pub type BlockClick = Rc<dyn Fn(usize, Option<(usize, usize)>, &str, usize, &mut Window, &mut App)>;

/// Called with a list item and how to change its source text.
pub type EditItem = Rc<dyn Fn(usize, Rc<dyn Fn(&str) -> String>, &mut App)>;

/// The foldable things whose state differs from how the note starts.
pub type Folds = Rc<RefCell<std::collections::HashSet<String>>>;

thread_local! {
    static CONTROL_PRESSED: Cell<bool> = const { Cell::new(false) };
}

/// Remembers that a press landed on a control inside a block, so the block does not open for it.
pub fn press_control() {
    CONTROL_PRESSED.with(|c| c.set(true));
}

/// Whether the press now ending landed on a control; asking ends it.
pub fn take_control_press() -> bool {
    CONTROL_PRESSED.with(Cell::take)
}

/// Flips one fold.
fn toggle_fold(folds: &Folds, key: &str) {
    let mut set = folds.borrow_mut();
    if !set.remove(key) {
        set.insert(key.to_string());
    }
}

struct Ctx<'a> {
    t: &'a Theme,
    base: Option<&'a Path>,
    on_toggle: Option<OnToggle>,
    footnote_order: &'a [String],
    order: &'a Cell<u64>,
    /// Notes being embedded right now, so a note embedding itself ends.
    chain: Vec<PathBuf>,
    focus: Option<&'a LinkFocus>,
    links_seen: &'a Cell<usize>,
    /// The top-level block being drawn.
    block: usize,
    /// Drawing the head of a foldable block, where a click folds instead of opening.
    head_text: Cell<bool>,
    /// The table cell being drawn, with row and column.
    cell: Cell<Option<(usize, usize)>>,
    /// The field for the cell being written.
    open_cell: Option<(usize, usize, RefCell<Option<AnyElement>>)>,
    /// The matches of the find bar to mark in this block.
    find: Option<FindDraw>,
}

/// Whether an image is small enough to stand in a line of text.
fn fits_in_line(alt: &str, size: f32) -> Option<(f32, f32)> {
    let (width, height) = image_size(alt);
    let limit = size * 2.;
    match (width, height) {
        (Some(w), Some(h)) if h <= limit => Some((w, h)),
        (None, Some(h)) if h <= limit => Some((h, h)),
        (Some(w), None) if w <= limit => Some((w, w)),
        _ => None,
    }
}

/// Text with its big images standing between the lines, where they were written.
fn styled_text(inlines: &[Inline], cx: &Ctx, size: f32) -> AnyElement {
    let is_big = |i: &Inline| matches!(i, Inline::Image(_, alt, _) if fits_in_line(alt, size).is_none());
    if !inlines.iter().any(is_big) {
        return styled_line(inlines, cx, size);
    }
    let mut parts: Vec<AnyElement> = Vec::new();
    let mut pictures: Vec<AnyElement> = Vec::new();
    let flush = |pictures: &mut Vec<AnyElement>, parts: &mut Vec<AnyElement>| {
        if !pictures.is_empty() {
            parts.push(div().flex().flex_wrap().items_end().gap(px(space::XS)).children(std::mem::take(pictures)).into_any_element());
        }
    };
    let mut run: &[Inline] = inlines;
    while let Some(at) = run.iter().position(is_big) {
        if !plain_text(&run[..at]).trim().is_empty() || run[..at].iter().any(|i| matches!(i, Inline::Image(..) | Inline::Math(_))) {
            flush(&mut pictures, &mut parts);
            parts.push(styled_line(&run[..at], cx, size));
        }
        if let Inline::Image(src, alt, target) = &run[at] {
            let picture = Path::new(src.trim_end_matches('\\')).extension().is_some_and(|e| IMAGE_TYPES.contains(&e.to_string_lossy().to_lowercase().as_str()));
            if picture {
                pictures.push(linked_image(src, alt, target.as_deref(), cx));
            } else {
                flush(&mut pictures, &mut parts);
                parts.push(linked_image(src, alt, target.as_deref(), cx));
            }
        }
        run = &run[at + 1..];
    }
    flush(&mut pictures, &mut parts);
    if !plain_text(run).trim().is_empty() || run.iter().any(|i| matches!(i, Inline::Image(..) | Inline::Math(_))) {
        parts.push(styled_line(run, cx, size));
    }
    div().flex().flex_col().gap(px(space::S)).children(parts).into_any_element()
}

/// An image, opening the link around it instead of itself if there is one.
fn linked_image(src: &str, alt: &str, target: Option<&str>, cx: &Ctx) -> AnyElement {
    let shown = image(src, alt, cx);
    let Some(target) = target else { return shown };
    let order = cx.order.get();
    cx.order.set(order + 1);
    let action = link_action(target, cx.base);
    div().id(("md-image-link", order)).when(!is_sample_base(cx.base), |d| d.cursor_pointer().soft(cx.t.ink.opacity(0.04), radius::CONTROL)).capture_any_mouse_down(|_, _, _| press_control()).on_click(move |_, _, cx| action(cx)).child(shown).into_any_element()
}

/// What a plain click on text in this block should do.
fn text_click_of(cx: &Ctx) -> Option<super::selectable::TextClick> {
    if cx.head_text.get() {
        return None;
    }
    let (click, block, cell) = (cx.focus.and_then(|f| f.text_click.clone())?, cx.block, cx.cell.get());
    Some(Rc::new(move |text, at, window, app| click(block, cell, text, at, window, app)))
}

fn styled_line(inlines: &[Inline], cx: &Ctx, size: f32) -> AnyElement {
    let t = cx.t;
    let mut text = String::new();
    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    let mut links: Vec<(std::ops::Range<usize>, LinkAction)> = Vec::new();
    let mut hovers: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut slots: Vec<InlineSlot> = Vec::new();
    let mut pills: Vec<std::ops::Range<usize>> = Vec::new();
    let sample = is_sample_base(cx.base);
    for i in inlines {
        match i {
            Inline::Tag(tag) => {
                let start = text.len();
                text.push('#');
                text.push_str(tag);
                let name = tag.clone();
                if !sample {
                    links.push((start..text.len(), Rc::new(move |cx| super::state::act(cx, |l| l.show_tag(&name)))));
                }
                pills.push(start..text.len());
                highlights.push((start..text.len(), HighlightStyle { color: Some(t.accent_text), ..Default::default() }));
            }
            Inline::Text(s, st) | Inline::Link(s, st, _) => {
                let start = text.len();
                text.push_str(s);
                let (mut unresolved, mut focused) = (false, false);
                if let Inline::Link(_, _, url) = i {
                    unresolved = !is_external(url) && !link_exists(url, cx.base);
                    if is_external(url) {
                        text.push_str(" ↗");
                    }
                    if !sample {
                        links.push((start..text.len(), link_action(url, cx.base)));
                        hovers.push((start..text.len(), url.clone()));
                    }
                    if let Some(focus) = cx.focus {
                        let index = cx.links_seen.get();
                        cx.links_seen.set(index + 1);
                        focused = focus.index == Some(index);
                    }
                }
                let mut h = HighlightStyle::default();
                if st.bold {
                    h.font_weight = Some(FontWeight::BOLD);
                }
                if st.italic {
                    h.font_style = Some(FontStyle::Italic);
                }
                if st.strike {
                    h.strikethrough = Some(StrikethroughStyle { thickness: px(1.), color: Some(t.secondary) });
                }
                if st.code {
                    h.background_color = Some(t.ink.opacity(0.08));
                    h.color = Some(t.primary);
                }
                if st.underline {
                    h.underline = Some(UnderlineStyle { thickness: px(1.), color: None, wavy: false });
                }
                if st.small {
                    h.fade_out = Some(0.25);
                }
                if let Some(color) = st.color {
                    h.color = Some(rgb(color).into());
                }
                if let Some(color) = st.mark {
                    h.background_color = Some(mark_tint(color, t).opacity(0.3));
                }
                if st.link && unresolved {
                    h.color = Some(t.tertiary);
                    h.underline = Some(UnderlineStyle { thickness: px(1.), color: Some(t.tertiary), wavy: true });
                } else if st.link {
                    h.color = Some(t.accent_text);
                    h.underline = Some(UnderlineStyle { thickness: px(1.), color: Some(t.accent_text.opacity(0.4)), wavy: false });
                }
                if focused {
                    h.background_color = Some(t.accent.opacity(0.28));
                }
                if h != HighlightStyle::default() {
                    highlights.push((start..text.len(), h));
                }
            }
            Inline::FootnoteRef(id) => {
                let n = cx.footnote_order.iter().position(|f| f == id).map_or(0, |p| p + 1);
                let start = text.len();
                text.push_str(&superscript(&n.to_string()));
                highlights.push((start..text.len(), HighlightStyle { color: Some(t.accent_text), ..Default::default() }));
                if let Some(list) = cx.focus.and_then(|f| f.list.clone()) {
                    links.push((start..text.len(), Rc::new(move |_| reveal_item(&list, list.item_count().saturating_sub(1)))));
                }
            }
            Inline::Image(src, alt, target) => {
                if let Some((w, h)) = fits_in_line(alt, size) {
                    slots.push(InlineSlot { at: text.len(), kind: InlineKind::Sized(linked_image(src, alt, target.as_deref(), cx), w, h) });
                }
            }
            Inline::Math(source) => slots.push(InlineSlot { at: text.len(), kind: InlineKind::Math(source.clone(), super::theme::note_scale()) }),
        }
    }
    let order = cx.order.get();
    cx.order.set(order + 1);
    let base = cx.base.map(Path::to_path_buf);
    let highlights = match &cx.find {
        Some(find) => find.overlay(&text, highlights, t),
        None => highlights,
    };
    let body = div().text_size(px(size)).line_height(relative(1.5)).child(selectable(("md-text", order), text, highlights, order, t.accent.opacity(0.3)).links(links).hovers(hovers, base).inline(slots).pills(pills, t.accent.opacity(0.14)).on_text_click(text_click_of(cx)));
    body.into_any_element()
}

/// What a table cell shows, for measuring it: text runs as bold and italic, and the pictures in between.
#[derive(Default, Hash)]
struct CellText {
    runs: Vec<(String, bool, bool)>,
    /// Widths of pictures standing in the text, which nothing wraps inside.
    pictures: Vec<u32>,
    /// Widths of pictures too big for a line, standing between the lines.
    figures: Vec<u32>,
    formulas: Vec<String>,
}

fn cell_text(inlines: &[Inline], cx: &Ctx, size: f32) -> CellText {
    let mut out = CellText::default();
    let mut push = |text: String, bold: bool, italic: bool| out.runs.push((text, bold, italic));
    let mut pictures = Vec::new();
    let mut figures = Vec::new();
    let mut formulas = Vec::new();
    for i in inlines {
        match i {
            Inline::Tag(tag) => push(format!("#{tag}"), false, false),
            Inline::Text(s, st) => push(s.clone(), st.bold, st.italic),
            Inline::Link(s, st, url) => push(if is_external(url) { format!("{s} ↗") } else { s.clone() }, st.bold, st.italic),
            Inline::FootnoteRef(id) => push(superscript(&cx.footnote_order.iter().position(|f| f == id).map_or(0, |p| p + 1).to_string()), false, false),
            Inline::Image(_, alt, _) => match fits_in_line(alt, size) {
                Some((w, _)) => pictures.push((w + space::XS).ceil() as u32),
                None => figures.push(image_size(alt).0.unwrap_or(FIGURE_WIDTH).ceil() as u32),
            },
            Inline::Math(source) => formulas.push(source.clone()),
        }
    }
    out.pictures = pictures;
    out.figures = figures;
    out.formulas = formulas;
    out
}

/// How wide a cell's text runs on one line, and its longest word, from the position after each byte.
fn cell_extent(text: &str, x_at: impl Fn(usize) -> f32, objects: &[f32], figures: &[f32]) -> (f32, f32) {
    let mut word = 0_f32;
    let mut at = 0;
    for piece in text.split_inclusive(char::is_whitespace) {
        word = word.max(x_at(at + piece.trim_end().len()) - x_at(at));
        at += piece.len();
    }
    let widest = x_at(text.len()) - x_at(0) + objects.iter().sum::<f32>();
    let figure = figures.iter().copied().fold(0., f32::max);
    (widest.max(figure), objects.iter().copied().fold(word, f32::max).max(figure))
}

/// The natural and the narrowest width of each column, padding included; long words may break past `longest_word`.
fn column_widths(cells: &[Vec<(f32, f32)>], columns: usize, longest_word: f32) -> (Vec<f32>, Vec<f32>) {
    let pad = space::S * 2. + TABLE_RULE;
    let mut natural = vec![pad; columns];
    let mut least = vec![pad; columns];
    for row in cells {
        for (c, (widest, word)) in row.iter().enumerate().take(columns) {
            natural[c] = natural[c].max((widest + pad).ceil());
            least[c] = least[c].max((word.min(longest_word) + pad).ceil());
        }
    }
    (natural, least)
}

/// The width of a table's rules.
const TABLE_RULE: f32 = 1.;
/// About how wide a letter of a formula is while it is not drawn yet.
const FORMULA_LETTER: f32 = 7.;
/// The longest word, in font sizes, that a table column keeps whole when the note is narrow.
const LONGEST_UNBROKEN: f32 = 14.;
/// The width a picture without a stated size is given in a table.
const FIGURE_WIDTH: f32 = 160.;

/// The natural and the narrowest width of each column.
type ColumnWidths = Rc<(Vec<f32>, Vec<f32>)>;

thread_local! {
    static COLUMN_WIDTHS: RefCell<HashMap<u64, ColumnWidths>> = RefCell::new(HashMap::new());
}

/// A table whose columns take the width their text needs and wrap only when the note is too narrow.
#[derive(IntoElement)]
struct Table {
    id: ElementId,
    columns: usize,
    /// The rows drawn, the head first when it is drawn.
    cells: Vec<Vec<Div>>,
    /// The text of every row of the whole table, the head first, for measuring.
    measure: Rc<Vec<Vec<CellText>>>,
    /// Names what `measure` holds, so the widths are found without reading it.
    key: Option<u64>,
    with_head: bool,
    /// Whether the top and the bottom edge of the table are drawn here.
    ends: (bool, bool),
    /// The sideways scroll shared with the table's other parts.
    scroll: Option<ScrollHandle>,
    size: f32,
    border: Hsla,
    head_bg: Hsla,
}

impl Table {
    /// The natural and the narrowest width of each column, measured in the note's font.
    fn widths(&self, window: &mut Window, cx: &mut App) -> ColumnWidths {
        use std::hash::{Hash, Hasher};
        let base = window.text_style().font();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.size.to_bits().hash(&mut hasher);
        base.family.hash(&mut hasher);
        match self.key {
            Some(key) => key.hash(&mut hasher),
            None => {
                for row in self.measure.iter() {
                    row.len().hash(&mut hasher);
                    for text in row {
                        text.hash(&mut hasher);
                    }
                }
            }
        }
        let key = hasher.finish();
        if let Some(found) = COLUMN_WIDTHS.with(|c| c.borrow().get(&key).cloned()) {
            return found;
        }
        let mut drawn_all = true;
        let mut measured: Vec<Vec<(f32, f32)>> = Vec::new();
        for (r, row) in self.measure.iter().enumerate() {
            let mut sizes = Vec::new();
            for cell in row.iter().take(self.columns) {
                let mut text = String::new();
                let mut runs = Vec::new();
                for (part, bold, italic) in &cell.runs {
                    let mut font = base.clone();
                    font.weight = if *bold { FontWeight::BOLD } else if r == 0 { FontWeight::SEMIBOLD } else { base.weight };
                    if *italic {
                        font.style = FontStyle::Italic;
                    }
                    let part = part.replace(['\n', '\r'], " ");
                    runs.push(TextRun { len: part.len(), font, color: black(), background_color: None, underline: None, strikethrough: None });
                    text.push_str(&part);
                }
                let line = window.text_system().shape_line(text.clone().into(), px(self.size), &runs, None);
                let glyphs: Vec<(usize, f32)> = line.runs.iter().flat_map(|run| run.glyphs.iter().map(|g| (g.index, f32::from(g.position.x)))).collect();
                let end = f32::from(line.width);
                let sorted = glyphs.windows(2).all(|w| w[0].0 <= w[1].0);
                let x_at = |i: usize| if sorted { glyphs.get(glyphs.partition_point(|(at, _)| *at < i)) } else { glyphs.iter().find(|(at, _)| *at >= i) }.map_or(end, |(_, x)| *x);
                let mut objects: Vec<f32> = cell.pictures.iter().map(|p| *p as f32).collect();
                for formula in &cell.formulas {
                    objects.push(match super::embeds::inline_math(formula, super::theme::note_scale(), cx) {
                        Some((_, w, _)) => w + space::XS,
                        None => {
                            drawn_all &= !super::embeds::inline_math_pending(formula, cx);
                            formula.len() as f32 * FORMULA_LETTER
                        }
                    });
                }
                let figures: Vec<f32> = cell.figures.iter().map(|f| *f as f32).collect();
                sizes.push(cell_extent(&text, x_at, &objects, &figures));
            }
            measured.push(sizes);
        }
        let (natural, least) = column_widths(&measured, self.columns, self.size * LONGEST_UNBROKEN);
        let widths = Rc::new((natural, least));
        if drawn_all {
            COLUMN_WIDTHS.with(|c| {
                let mut c = c.borrow_mut();
                if c.len() > 512 {
                    c.clear();
                }
                c.insert(key, widths.clone());
            });
        }
        widths
    }
}

impl RenderOnce for Table {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let widths = self.widths(window, cx);
        let (natural, least) = (&widths.0, &widths.1);
        let column = |c: usize, cell: Div| {
            let (basis, narrowest) = (natural.get(c).copied().unwrap_or(0.), least.get(c).copied().unwrap_or(0.));
            let mut d = cell.flex_basis(px(narrowest)).flex_shrink_0().min_w(px(narrowest)).max_w(px(basis.max(narrowest)));
            d.style().flex_grow = Some((basis - narrowest).max(0.));
            d
        };
        let border = self.border;
        let (with_head, (top, bottom)) = (self.with_head, self.ends);
        let mut rows = self.cells.into_iter().enumerate().map(|(r, row)| {
            div().flex().when(r > 0 || !top, |d| d.border_t_1().border_color(border)).children(row.into_iter().enumerate().map(|(c, element)| column(c, element)).collect::<Vec<_>>())
        });
        let head = if with_head { rows.next().map(|d| d.bg(self.head_bg)) } else { None };
        let outline = TABLE_RULE * 2.;
        let corner = px(radius::CONTROL);
        let frame = div()
            .w_full()
            .min_w(px(least.iter().sum::<f32>() + outline))
            .max_w(px(natural.iter().sum::<f32>() + outline))
            .flex()
            .flex_col()
            .when(top, |d| d.rounded_t(corner).border_t_1())
            .when(bottom, |d| d.rounded_b(corner).border_b_1())
            .border_l_1()
            .border_r_1()
            .border_color(border)
            .overflow_hidden()
            .children(head)
            .children(rows);
        div().id(self.id).w_full().overflow_x_scroll().when_some(self.scroll, |d, scroll| d.track_scroll(&scroll)).child(frame)
    }
}

fn mark_tint(color: MarkColor, t: &Theme) -> Hsla {
    match color {
        MarkColor::Default => t.orange,
        MarkColor::Red => t.red,
        MarkColor::Orange => t.palette("orange"),
        MarkColor::Yellow => t.palette("yellow"),
        MarkColor::Green => t.green,
        MarkColor::Blue => t.blue,
        MarkColor::Purple => t.palette("purple"),
    }
}

fn is_external(url: &str) -> bool {
    url.contains("://") || url.starts_with("mailto:")
}

/// The note a relative or vault link is written in. `base` is its folder; the note's own name is not needed to resolve.
fn from_note(base: Option<&Path>) -> Option<PathBuf> {
    base.map(|b| b.join("."))
}

fn link_exists(url: &str, base: Option<&Path>) -> bool {
    let decoded = markdown::percent_decode(url);
    let target = links::LinkTarget::parse(&decoded);
    target.path.is_empty() || base.is_some_and(|b| b.join(&target.path).is_file()) || index::current().resolver.resolve(&target, from_note(base).as_deref()).is_some() || Path::new(&target.path).is_absolute()
}

/// Whether notes drawn from `base` are guide examples, which must not act on the user's data.
pub(super) fn is_sample_base(base: Option<&Path>) -> bool {
    base.is_some_and(super::help::is_sample)
}

/// Web links open in the browser; everything else is a link inside the vault, created if it is missing.
pub fn link_action(url: &str, base: Option<&Path>) -> LinkAction {
    if is_sample_base(base) {
        return Rc::new(|_| {});
    }
    let url = url.to_string();
    if url.starts_with("obsidian://") {
        return Rc::new(move |cx| {
            use crate::core::vault::config::{UriAction, parse_uri};
            let root = crate::core::vault::index::current().root.clone();
            match parse_uri(&url, &root) {
                Some(UriAction::Open(file)) => {
                    let opened = super::state::act(cx, |l| l.open_link(&file, None));
                    if !opened {
                        super::state::act(cx, |l| l.create_link_target(&file, None));
                    }
                }
                Some(UriAction::New { name, content }) => {
                    let path = root.join(if name.ends_with(".md") { name } else { format!("{name}.md") });
                    if !path.exists() && std::fs::write(&path, content).is_ok() {
                        super::state::reindex(cx);
                    }
                    super::state::act(cx, |l| l.open_file(&path));
                }
                Some(UriAction::Daily) => {
                    super::state::act(cx, |l| l.open_daily_note());
                    super::state::reindex(cx);
                }
                Some(UriAction::Search(query)) => super::state::act(cx, |l| {
                    l.search_query = query;
                    l.set_route(crate::core::library::Route::Search);
                }),
                None => {
                    crate::core::library::open_path(&url);
                }
            }
        });
    }
    if is_external(&url) {
        return Rc::new(move |_| {
            crate::core::library::open_path(&url);
        });
    }
    let from = from_note(base);
    Rc::new(move |cx| {
        let destination = markdown::percent_decode(&url);
        let opened = super::state::act(cx, |l| l.open_link(&destination, from.as_deref()));
        if !opened {
            super::state::act(cx, |l| l.create_link_target(&destination, from.as_deref()));
        }
    })
}

/// The width and height of `![[x|100x145]]`, `![[x|100]]` or `![100](url)`.
fn image_size(alt: &str) -> (Option<f32>, Option<f32>) {
    let word = alt.rsplit('|').next().unwrap_or(alt).trim();
    match word.split_once('x') {
        Some((w, h)) => (w.parse().ok(), h.parse().ok()),
        None => (word.parse().ok(), None),
    }
}

const IMAGE_TYPES: [&str; 8] = ["avif", "bmp", "gif", "jpeg", "jpg", "png", "svg", "webp"];
const AUDIO_TYPES: [&str; 7] = ["flac", "m4a", "mp3", "ogg", "wav", "webm", "3gp"];
const VIDEO_TYPES: [&str; 5] = ["mkv", "mov", "mp4", "ogv", "webm"];

// Parsed notes that are embedded elsewhere, by file and part; checked against the file's time.
thread_local! {
    static EMBEDS: RefCell<HashMap<(PathBuf, String), (Option<std::time::SystemTime>, Rc<ParsedNote>)>> = RefCell::new(HashMap::new());
}

fn embedded_note(path: &Path, target: &links::LinkTarget) -> Rc<ParsedNote> {
    let key = (path.to_path_buf(), target.subpath());
    let time = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    EMBEDS.with(|cache| {
        if let Some((known, note)) = cache.borrow().get(&key)
            && *known == time
        {
            return note.clone();
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let part = links::section(&text, target);
        let note = Rc::new(parse(&part));
        cache.borrow_mut().insert(key, (time, note.clone()));
        note
    })
}

/// A small card for what is embedded but shown elsewhere.
fn file_card(path: PathBuf, detail: Option<String>, action: String, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let order = cx.order.get();
    cx.order.set(order + 1);
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let open = path.clone();
    let sample = super::help::is_sample(&path);
    div()
        .id(("md-file-card", order))
        .px(px(space::M))
        .py(px(space::S))
        .flex()
        .items_center()
        .gap(px(space::M))
        .rounded(px(radius::CONTROL))
        .border_1()
        .border_color(t.separator)
        .when(!sample, |d| d.cursor_pointer().soft(t.ink.opacity(0.04), radius::CONTROL))
        .on_click(move |_, _, cx| if !super::help::is_sample(&open) { super::state::act(cx, |l| l.open_file(&open)) })
        .child(super::widgets::file_icon(&path, iconsize::L))
        .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().child(name)).when_some(detail, |d, text| d.child(super::widgets::caption(text, t))))
        .child(super::widgets::caption(action, t).flex_none())
        .into_any_element()
}

fn missing_embed(name: &str, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    div().px(px(space::M)).py(px(space::S)).flex().items_center().gap(px(space::S)).rounded(px(radius::CONTROL)).border_1().border_color(t.separator).text_color(t.secondary).child(super::widgets::icon("file-question", iconsize::M, t.tertiary)).child(trf("%@ nicht gefunden", &[&name.to_string()])).into_any_element()
}

/// Shows what `![[…]]` embeds: an image, a whole note or part of one, or a card for other files.
fn embed(src: &str, alt: &str, cx: &Ctx) -> AnyElement {
    let decoded = markdown::percent_decode(src);
    let target = links::LinkTarget::parse(&decoded);
    let vault = index::current();
    let from = from_note(cx.base);
    let beside = cx.base.map(|b| b.join(&target.path)).filter(|p| !target.path.is_empty() && p.is_file());
    let Some(path) = beside.or_else(|| vault.resolver.resolve(&target, from.as_deref()).filter(|_| !is_sample_base(cx.base))) else { return missing_embed(&target.path, cx) };
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if IMAGE_TYPES.contains(&ext.as_str()) {
        return image_at(path, alt, &decoded, cx);
    }
    if markdown::is_markdown(&path) {
        return embed_note(path, &target, cx);
    }
    let query = target.headings.first().cloned().unwrap_or_default();
    if ext == "pdf" {
        return super::embeds::pdf(path, &query, cx.order.get(), cx.t);
    }
    if AUDIO_TYPES.contains(&ext.as_str()) && ext != "webm" {
        return super::embeds::audio(path, cx.order.get(), cx.t);
    }
    if ext == "base" {
        return super::embeds::base_file(path, cx.order.get(), cx.t);
    }
    if ext == "canvas" {
        return super::embeds::canvas_file(path, cx.order.get(), cx.t);
    }
    if VIDEO_TYPES.contains(&ext.as_str()) {
        return file_card(path, Some(tr("Video")), tr("Im Player öffnen"), cx);
    }
    file_card(path, None, tr("Öffnen"), cx)
}

fn embed_note(path: PathBuf, target: &links::LinkTarget, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    if cx.chain.contains(&path) {
        return div().text_color(t.secondary).child(tr("Bettet sich selbst ein")).into_any_element();
    }
    let order = cx.order.get();
    cx.order.set(order + 1);
    let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let open = path.clone();
    let sample = is_sample_base(cx.base);
    if cx.chain.len() >= 3 {
        return div().id(("md-embed-link", order)).text_color(t.accent_text).when(!sample, |d| d.cursor_pointer().soft(t.ink.opacity(0.04), radius::SMALL)).on_click(move |_, _, cx| if !super::help::is_sample(&open) { super::state::act(cx, |l| l.open_file(&open)) }).child(name).into_any_element();
    }
    let note = embedded_note(&path, target);
    let mut chain = cx.chain.clone();
    chain.push(path.clone());
    let inner = Ctx { t, base: path.parent(), on_toggle: None, footnote_order: &note.footnote_order, order: cx.order, chain, focus: None, links_seen: cx.links_seen, block: cx.block, head_text: Cell::new(false), cell: Cell::new(None), open_cell: None, find: None };
    let blocks = render_blocks(&note.blocks, &inner);
    div()
        .flex()
        .flex_col()
        .rounded(px(radius::CONTROL))
        .border_1()
        .border_color(t.separator)
        .overflow_hidden()
        .child(
            div()
                .id(("md-embed-head", order))
                .px(px(space::M))
                .py(px(space::XS))
                .flex()
                .items_center()
                .gap(px(space::XS))
                .bg(t.secondary_bg)
                .when(!sample, |d| d.cursor_pointer().soft(t.ink.opacity(0.04), 0.))
                .text_size(px(font::CAPTION))
                .text_color(t.secondary)
                .on_click(move |_, _, cx| if !super::help::is_sample(&open) { super::state::act(cx, |l| l.open_file(&open)) })
                .child(div().flex_1().min_w_0().truncate().child(name))
                .child(super::widgets::icon("external-link", iconsize::XS, t.tertiary)),
        )
        .child(div().p(px(space::M)).flex().flex_col().gap(px(space::M)).children(blocks))
        .into_any_element()
}

fn image(src: &str, alt: &str, cx: &Ctx) -> AnyElement {
    let src = src.trim_end_matches('\\');
    if src.contains("://") {
        let (width, height) = image_size(alt);
        return img(SharedString::from(src.to_string())).max_w_full().rounded(px(radius::CONTROL)).when_some(width, |d, w| d.w(px(w))).when_some(height, |d, h| d.h(px(h))).into_any_element();
    }
    let path = markdown::percent_decode(src);
    let p: PathBuf = if Path::new(&path).is_absolute() { PathBuf::from(&path) } else { cx.base.map(|b| b.join(&path)).unwrap_or_else(|| PathBuf::from(&path)) };
    if p.exists() && IMAGE_TYPES.contains(&p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default().as_str()) {
        return image_at(p, alt, src, cx);
    }
    embed(src, alt, cx)
}

fn image_at(p: PathBuf, alt: &str, source_written: &str, cx: &Ctx) -> AnyElement {
    let (width, height) = image_size(alt);
    let _ = &alt;
    {
        let order = cx.order.get();
        cx.order.set(order + 1);
        let open = p.clone();
        let sample = is_sample_base(cx.base);
        let drawn = div().id(("md-image", order)).when(!sample, |d| d.cursor_pointer().soft(cx.t.ink.opacity(0.04), radius::CONTROL)).capture_any_mouse_down(|_, _, _| press_control()).on_click(move |_, _, cx| if !super::help::is_sample(&open) { super::preview::show(&open, cx) }).child(img(p).when_some(width, |d, w| d.w(px(w))).when_some(height, |d, h| d.h(px(h))).max_w_full().max_h(px(560.)).object_fit(ObjectFit::Contain).rounded(px(radius::CONTROL)));
        let source_name = source_written.to_string();
        match (cx.focus.and_then(|f| f.edit_item.clone()), cx.block) {
            (Some(edit), item) => drawn
                .context_menu(move |mut menu, _, _| {
                    for (label, width) in [(tr("Klein"), Some(200)), (tr("Mittel"), Some(400)), (tr("Groß"), Some(640)), (tr("Originalgröße"), None)] {
                        let (edit, name) = (edit.clone(), source_name.clone());
                        menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                            let name = name.clone();
                            edit(item, Rc::new(move |text| crate::core::live_text::set_image_width(text, &name, width)), cx);
                        }));
                    }
                    menu
                })
                .into_any_element(),
            _ => drawn.into_any_element(),
        }
    }
}

thread_local! {
    static COLOURED: RefCell<HashMap<(Option<String>, String), Rc<Vec<(String, TokenKind)>>>> = RefCell::new(HashMap::new());
}

/// Colours code once; scrolling draws the same block again and again.
pub(super) fn coloured(lang: Option<&str>, code: &str) -> Rc<Vec<(String, TokenKind)>> {
    let key = (lang.map(str::to_string), code.to_string());
    COLOURED.with(|c| {
        let mut cache = c.borrow_mut();
        if let Some(found) = cache.get(&key) {
            return found.clone();
        }
        if cache.len() > 256 {
            cache.clear();
        }
        let tokens = Rc::new(markdown::highlight(code, lang));
        cache.insert(key, tokens.clone());
        tokens
    })
}

fn code_block(lang: Option<&str>, code: &str, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let colors = super::theme::syntax_palette(t.kind);
    let mut text = String::new();
    let mut highlights = Vec::new();
    for (s, kind) in coloured(lang, code).iter() {
        let start = text.len();
        text.push_str(s);
        if let Some(c) = colors.of(*kind) {
            highlights.push((start..text.len(), HighlightStyle { color: Some(c), ..Default::default() }));
        }
    }
    let highlights = match &cx.find {
        Some(find) => find.overlay(&text, highlights, t),
        None => highlights,
    };
    let order = cx.order.get();
    cx.order.set(order + 1);
    let copied = code.to_string();
    let group = SharedString::from(format!("md-code-{order}"));
    div()
        .relative()
        .group(group.clone())
        .child(
            div()
                .id(("md-code", order))
                .w_full()
                .overflow_x_scroll()
                .p(px(space::M))
                .pr(px(space::S + control::S + space::XS))
                .rounded(px(radius::CONTROL))
                .bg(t.secondary_bg)
                .border_1()
                .border_color(t.separator.opacity(0.6))
                .font_family(Theme::mono_family())
                .text_size(px(note_size(font::CALLOUT)))
                .child(selectable(("md-code-text", order), text, highlights, order, t.accent.opacity(0.3)).on_text_click(text_click_of(cx))),
        )
        .child(
            div()
                .id(("md-code-copy", order))
                .capture_any_mouse_down(|_, _, _| press_control())
                .absolute()
                .top(px(space::S))
                .right(px(space::S))
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(radius::SMALL))
                .bg(t.secondary_bg)
                .cursor_pointer()
                .soft(t.ink.opacity(0.06), radius::SMALL)
                .invisible()
                .group_hover(group, |s| s.visible())
                .tooltip(super::widgets::tip(tr("Code kopieren")))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    super::drive::copy(copied.to_string(), cx);
                })
                .child(super::widgets::icon("copy", iconsize::S, t.secondary)),
        )
        .into_any_element()
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => font::TITLE,
        2 => font::TITLE2,
        3 => font::TITLE3,
        4 | 5 => font::BODY,
        _ => font::CALLOUT,
    }
}

fn heading_top(level: u8) -> f32 {
    if level <= 2 { space::S } else { space::XS }
}

/// The items of a list from `first` on, numbered as in the whole list.
fn list_items(start: &Option<u64>, items: &[Item], first: usize, cx: &Ctx) -> Div {
    let t = cx.t;
    div()
        .flex()
        .flex_col()
        .gap(px(space::XS))
        .children(items.iter().enumerate().map(|(i, item)| {
            let marker: AnyElement = match (item.task, start) {
                (Some(mark), _) => {
                    let done = mark == 'x' || mark == 'X';
                    let special = !done && mark != ' ';
                    let line = item.line;
                    let toggle = cx.on_toggle.clone();
                    div()
                        .id(SharedString::from(format!("task-{line}")))
        .capture_any_mouse_down(|_, _, _| press_control())
                        .mt(px(space::XS))
                        .ml(px(space::XXS))
                        .size(px(iconsize::S))
                        .flex_none()
                        .rounded(px(radius::SMALL))
                        .soft(if done { t.on_accent.opacity(0.2) } else { t.ink.opacity(0.1) }, radius::SMALL)
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(done, |d| d.bg(t.accent))
                        .when(!done, |d| d.border_1().border_color(if special { t.accent } else { t.secondary }))
                        .when(special, |d| d.text_size(px(font::CAPTION2)).text_color(t.accent).child(mark.to_string()))
                        .when(toggle.is_some(), |d| d.cursor_pointer().tooltip(super::widgets::tip(if done { tr("Als offen markieren") } else { tr("Als erledigt markieren") })))
                        .when(done, |d| d.child(super::widgets::icon("check", iconsize::XS, t.on_accent)))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            if let Some(f) = &toggle {
                                f(line, cx);
                            }
                        })
                        .into_any_element()
                }
                (None, Some(s)) => div().flex_none().min_w(px(space::L)).flex().justify_end().text_size(px(note_size(font::BODY))).line_height(relative(1.5)).text_color(t.secondary).child(format!("{}.", s + (first + i) as u64)).into_any_element(),
                (None, None) => div().flex_none().min_w(px(space::L)).flex().justify_end().text_size(px(note_size(font::BODY))).line_height(relative(1.5)).text_color(t.secondary).child("•").into_any_element(),
            };
            let done = matches!(item.task, Some('x' | 'X'));
            let has_nested = item.blocks.iter().any(|b| matches!(b, Block::List { .. }));
            let fold_key = format!("l{}", item.line);
            let folds = cx.focus.map(|f| f.folds.clone());
            let folded = has_nested && folds.as_ref().is_some_and(|f| f.borrow().contains(&fold_key));
            let shown_blocks: Vec<AnyElement> = item
                .blocks
                .iter()
                .filter(|b| !(folded && matches!(b, Block::List { .. })))
                .map(|b| {
                    let inner = render_blocks(std::slice::from_ref(b), cx).into_iter().next().unwrap_or_else(|| div().into_any_element());
                    if matches!(b, Block::List { .. }) {
                        div().border_l_1().border_color(t.separator).pl(px(space::S)).child(inner).into_any_element()
                    } else if done {
                        div().text_color(t.secondary).line_through().child(inner).into_any_element()
                    } else {
                        inner
                    }
                })
                .collect();
            let group = SharedString::from(format!("md-item-{}", item.line));
            let fold_button = (has_nested && folds.is_some()).then(|| {
                let line = item.line;
                div()
                    .id(("md-list-fold", line))
                    .when(!folded, |d| d.invisible().group_hover(group.clone(), StyleRefinement::visible))
        .capture_any_mouse_down(|_, _, _| press_control())
                    .absolute()
                    .top_0()
                    .left_0()
                    .size(px(control::S))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .soft(t.ink.opacity(0.06), radius::SMALL)
                    .tooltip(super::widgets::tip(tr(if folded { "Ausklappen" } else { "Einklappen" })))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        if let Some(f) = &folds {
                            toggle_fold(f, &fold_key);
                            cx.refresh_windows();
                        }
                    })
                    .child(super::motion::chevron(("md-list-chevron", line), !folded, t.tertiary))
            });
            div()
                .relative()
                .ml(px(-(space::L + space::XS)))
                .pl(px(space::L + space::XS))
                .group(group)
                .flex()
                .items_start()
                .gap(px(space::S))
                .children(fold_button)
                .child(marker)
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XS)).children(shown_blocks))
        }))
}

/// A table, or the rows `from..to` of it with the head only when they start it, its columns measured from the whole table.
/// `part` also names the whole table by its source, so its parts measure it once.
fn table(head: &[Vec<Inline>], rows: &[Vec<Vec<Inline>>], aligns: &[pulldown_cmark::Alignment], part: Option<(usize, usize, (u64, usize))>, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let size = note_size(font::BODY);
    let cell = |inl: &Vec<Inline>, row: usize, i: usize, bold: bool| {
        cx.order.set(cx.order.get() + 1);
        cx.cell.set(Some((row, i)));
        let click = text_click_of(cx);
        let d = div().px(px(space::S)).py(px(space::XS)).when(bold, |d| d.font_weight(FontWeight::SEMIBOLD));
        let d = match aligns.get(i) {
            Some(pulldown_cmark::Alignment::Center) => d.flex().justify_center(),
            Some(pulldown_cmark::Alignment::Right) => d.flex().justify_end(),
            _ => d,
        };
        let open = cx.open_cell.as_ref().filter(|(r, c, _)| *r == row && *c == i).and_then(|(_, _, field)| field.borrow_mut().take());
        let drawn = match open {
            Some(field) => d.child(field),
            None => {
                let body = styled_text(inl, cx, size);
                d.when_some(click, |d, click| {
                    d.capture_any_mouse_down(|_, _, _| press_control()).on_mouse_up(MouseButton::Left, move |_, window, cx| click("", 0, window, cx))
                })
                .child(body)
            }
        };
        cx.cell.set(None);
        drawn
    };
    let order = cx.order.get();
    cx.order.set(order + 1);
    let columns = rows.iter().map(Vec::len).chain(std::iter::once(head.len())).max().unwrap_or(0);
    let filled = |mut row: Vec<Div>| {
        row.resize_with(columns, div);
        row
    };
    let (from, to) = part.map_or((0, rows.len()), |(from, to, _)| (from, to));
    let to = to.min(rows.len());
    let with_head = from == 0;
    let mut cells = Vec::new();
    if with_head {
        cells.push(filled(head.iter().enumerate().map(|(i, c)| cell(c, 0, i, true)).collect()));
    }
    cells.extend(rows[from.min(to)..to].iter().enumerate().map(|(r, row)| filled(row.iter().enumerate().map(|(i, c)| cell(c, from + r + 1, i, false)).collect())));
    let measure = || {
        let texts = std::iter::once(head).chain(rows.iter().map(Vec::as_slice)).map(|row| {
            let mut texts: Vec<CellText> = row.iter().map(|c| cell_text(c, cx, size)).collect();
            texts.resize_with(columns, CellText::default);
            texts
        });
        Rc::new(texts.collect::<Vec<_>>())
    };
    let key = part.map(|(.., source)| {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (source, rows.as_ptr() as usize, size.to_bits(), cx.footnote_order).hash(&mut hasher);
        hasher.finish()
    });
    let measure = match key {
        Some(key) => TABLE_TEXTS.with(|known| {
            let mut known = known.borrow_mut();
            if known.len() > 64 {
                known.clear();
            }
            known.entry(key).or_insert_with(measure).clone()
        }),
        None => measure(),
    };
    let scroll = part.map(|(.., source)| table_scroll(source));
    Table { id: ElementId::from(("md-table", order)), columns, cells, measure, key, with_head, ends: (with_head, to >= rows.len()), scroll, size, border: t.separator, head_bg: t.secondary_bg }.into_any_element()
}

thread_local! {
    static TABLE_SCROLLS: RefCell<HashMap<(u64, usize), ScrollHandle>> = RefCell::new(HashMap::new());
    static TABLE_TEXTS: RefCell<HashMap<u64, Rc<Vec<Vec<CellText>>>>> = RefCell::new(HashMap::new());
}

/// One sideways scroll shared by every part of a table, so its columns stay aligned.
fn table_scroll(key: (u64, usize)) -> ScrollHandle {
    TABLE_SCROLLS.with(|known| {
        let mut known = known.borrow_mut();
        if known.len() > 256 {
            known.clear();
        }
        known.entry(key).or_default().clone()
    })
}

fn render_blocks(blocks: &[Block], cx: &Ctx) -> Vec<AnyElement> {
    let t = cx.t;
    blocks
        .iter()
        .map(|b| match b {
            Block::Heading(level, inl) => {
                let size = note_size(heading_size(*level));
                div().pt(px(heading_top(*level))).font_weight(if *level == 1 { FontWeight::BOLD } else { FontWeight::SEMIBOLD }).when(*level >= 5, |d| d.text_color(t.secondary)).child(styled_text(inl, cx, size)).into_any_element()
            }
            Block::Paragraph(inl) => styled_text(inl, cx, note_size(font::BODY)),
            Block::Quote(inner) => div()
                .flex()
                .gap(px(space::S))
                .child(div().w(px(3.)).flex_none().rounded_full().bg(t.accent.opacity(0.6)))
                .child(div().flex_1().flex().flex_col().gap(px(space::S)).text_color(t.secondary).children(render_blocks(inner, cx)))
                .into_any_element(),
            Block::Code(lang, code) => match super::embeds::special_block(lang.as_deref(), code, cx.base, cx.order.get(), t) {
                Some(element) => element,
                None => code_block(lang.as_deref(), code, cx),
            },
            Block::Rule => div().h(px(1.)).my(px(space::XS)).bg(t.separator).into_any_element(),
            Block::Nothing => div().hidden().into_any_element(),
            Block::Math(source) => super::embeds::math(source, cx.order.get(), t),
            Block::Properties(props) => properties_row(props, cx),
            Block::Callout(head, title, inner) => {
                let order = cx.order.get();
                cx.order.set(order + 1);
                let shown = (!plain_text(title).trim().is_empty() || title.iter().any(|i| matches!(i, Inline::Link(..) | Inline::Math(_)))).then(|| {
                    cx.head_text.set(true);
                    let drawn = styled_text(title, cx, note_size(font::CALLOUT));
                    cx.head_text.set(false);
                    drawn
                });
                let key = format!("c{order}");
                let change_type = cx.focus.and_then(|f| f.edit_item.clone()).map(|edit| (edit, cx.block));
                Callout { change_type, head: head.clone(), title: shown, content: render_blocks(inner, cx), id: ElementId::from(("md-callout", order)), tint: callout_tint(head, t), secondary: t.secondary, folds: cx.focus.map(|f| (f.folds.clone(), key)) }.into_any_element()
            }
            Block::Right(inner) => div().w_full().flex().flex_col().items_end().gap(px(space::S)).children(render_blocks(inner, cx)).into_any_element(),
            Block::Centered(inner) => div().w_full().flex().flex_col().items_center().gap(px(space::S)).children(render_blocks(inner, cx)).into_any_element(),
            Block::Details(summary, inner, open_at_start) => {
                let order = cx.order.get();
                cx.order.set(order + 1);
                let key = format!("d{order}");
                let folds = cx.focus.map(|f| f.folds.clone());
                let toggled = folds.as_ref().is_some_and(|f| f.borrow().contains(&key));
                let is_open = *open_at_start != toggled;
                let toggle_key = key.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S))
                    .child(
                        div()
                            .id(("md-details", order))
                            .flex()
                            .items_center()
                            .gap(px(space::XS))
                            .cursor_pointer()
                            .soft(t.ink.opacity(0.04), radius::INNER)
                            .capture_any_mouse_down(|_, _, _| press_control())
                            .font_weight(FontWeight::MEDIUM)
                            .child(super::motion::chevron(("md-details-chevron", order), is_open, t.secondary))
                            .child(styled_text(summary, cx, note_size(font::BODY)))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                if let Some(f) = &folds {
                                    toggle_fold(f, &toggle_key);
                                    cx.refresh_windows();
                                }
                            }),
                    )
                    .when(is_open, |d| d.child(div().pl(px(space::L)).flex().flex_col().gap(px(space::S)).children(render_blocks(inner, cx))))
                    .into_any_element()
            }
            Block::List { start, items } => list_items(start, items, 0, cx).into_any_element(),
            Block::Table { head, rows, aligns } => table(head, rows, aligns, None, cx),
        })
        .collect()
}

impl ParsedNote {
    /// The note's top-level headings as level and text, for a table of contents.
    /// The number of top-level blocks.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// First line, line after the end and how the live view writes each top-level block.
    pub fn shapes(&self) -> Vec<(usize, usize, crate::core::live_text::Shape)> {
        use crate::core::live_text::Shape;
        fn plain_list(blocks: &[Block]) -> bool {
            blocks.iter().all(|b| match b {
                Block::Paragraph(_) | Block::Nothing => true,
                Block::List { items, .. } => items.iter().all(|i| plain_list(&i.blocks)),
                _ => false,
            })
        }
        self.blocks
            .iter()
            .zip(&self.sources)
            .map(|(block, source)| {
                let many = source.end_line > source.line + 1;
                let shape = match block {
                    Block::Table { .. } => Shape::Table,
                    Block::Paragraph(inl) if inl.iter().any(|i| matches!(i, Inline::Math(_))) && many => Shape::Whole,
                    Block::Paragraph(_) | Block::Rule | Block::Nothing => Shape::Lines,
                    Block::Heading(..) if !many => Shape::Lines,
                    Block::Quote(inner) if inner.iter().all(|b| matches!(b, Block::Paragraph(_) | Block::Nothing)) => Shape::Lines,
                    Block::List { items, .. } if items.iter().all(|i| plain_list(&i.blocks)) => Shape::Lines,
                    _ => Shape::Whole,
                };
                (source.line, source.end_line, shape)
            })
            .collect()
    }

    /// How many blocks the first source line makes, like text with display math in its middle.
    pub fn line_blocks(&self) -> usize {
        self.sources.iter().take(self.blocks.len()).take_while(|s| s.line == 0).count()
    }

    /// How many blocks from `i` on come from the same source lines, like text with display math in its middle.
    pub fn blocks_sharing(&self, i: usize) -> usize {
        let Some(first) = self.sources.get(i).filter(|_| i < self.blocks.len()) else { return 0 };
        self.sources.iter().take(self.blocks.len()).skip(i).take_while(|s| s.line == first.line && s.end_line == first.end_line).count()
    }

    /// How far below a block's top the middle of its first line of text is; `None` when it has no text line to meet, like a formula.
    pub fn first_line_middle(&self, i: usize) -> Option<f32> {
        let line = |size: f32| note_size(size) * 1.5 / 2.;
        let picture = |inl: &[Inline]| inl.iter().any(|x| matches!(x, Inline::Image(..))) && inl.iter().all(|x| matches!(x, Inline::Image(..)) || matches!(x, Inline::Text(s, _) if s.trim().is_empty()));
        Some(match self.blocks.get(i)? {
            Block::Heading(level, _) => heading_top(*level) + note_size(heading_size(*level)) * 1.5 / 2.,
            Block::Paragraph(inl) if picture(inl) => space::M,
            Block::Math(_) => return None,
            Block::Code(Some(lang), _) if lang == "mermaid" => return None,
            Block::Code(Some(lang), _) if lang == "query" || lang == "base" => space::M,
            Block::Code(..) => 1. + space::M + line(font::CALLOUT),
            Block::Rule => space::XS + 0.5,
            Block::Table { .. } => 1. + space::XS + line(font::BODY),
            Block::Callout(..) => 1. + space::S + line(font::CALLOUT),
            _ => line(font::BODY),
        })
    }

    /// Whether a block shows a fold control in the margin left of its first line.
    pub fn has_margin_control(&self, i: usize) -> bool {
        match self.blocks.get(i) {
            Some(Block::Heading(..)) => true,
            Some(Block::List { items, .. }) => items.first().is_some_and(|item| item.blocks.iter().any(|b| matches!(b, Block::List { .. }))),
            _ => false,
        }
    }

    /// The note's footnotes in the order they are numbered.
    pub fn footnote_order(&self) -> &[String] {
        &self.footnote_order
    }

    /// Where each top-level block came from.
    pub fn sources(&self) -> &[BlockSource] {
        &self.sources
    }

    /// The rows the reader draws, long lists and tables in parts.
    pub fn rows(&self) -> &[ReaderRow] {
        &self.rows
    }

    /// The room below a reader row: none or a list's gap where the next row goes on with its block, a block's gap otherwise.
    pub fn row_gap(&self, row: usize) -> f32 {
        let Some(this) = self.rows.get(row) else { return space::M };
        match (self.rows.get(row + 1), self.blocks.get(this.block)) {
            (Some(next), Some(Block::List { .. })) if next.block == this.block => space::XS,
            (Some(next), _) if next.block == this.block => 0.,
            _ => space::M,
        }
    }

    /// The reader row a source line is drawn in.
    pub fn row_of_line(&self, line: usize) -> usize {
        self.rows.iter().rposition(|r| r.line <= line).unwrap_or(0)
    }

    /// The reader row a link is drawn in, by its place in `links`.
    pub fn row_of_link(&self, index: usize) -> usize {
        let block = self.links.get(index).map_or(0, |(_, b)| *b);
        self.rows.iter().rposition(|r| r.block == block && r.links_before <= index).or_else(|| self.rows.iter().position(|r| r.block == block)).unwrap_or(0)
    }

    /// Every link in reading order, with its top-level block.
    pub fn links(&self) -> &[(String, usize)] {
        &self.links
    }

    /// The top-level block of each heading, in the order of `headings`.
    pub fn heading_blocks(&self) -> Vec<usize> {
        self.blocks.iter().enumerate().filter(|(_, b)| matches!(b, Block::Heading(..))).map(|(i, _)| i).collect()
    }

    pub fn headings(&self) -> Vec<(u8, String)> {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading(level, inl) => Some((*level, plain_text(inl))),
                _ => None,
            })
            .collect()
    }
}

fn plain_text(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Text(s, _) | Inline::Link(s, _, _) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

fn callout_tint(head: &CalloutHead, t: &Theme) -> Hsla {
    use syntax::CalloutKind as K;
    match head.kind {
        K::Note | K::Info | K::Todo => t.blue,
        K::Abstract | K::Tip => t.palette("teal"),
        K::Success => t.green,
        K::Question | K::Warning => t.orange,
        K::Failure | K::Danger | K::Bug => t.red,
        K::Example => t.palette("purple"),
        K::Quote => t.gray,
    }
}

/// A callout, on the same banner as the app's hints; folds like its other sections.

#[derive(IntoElement)]
struct Callout {
    change_type: Option<(EditItem, usize)>,
    head: CalloutHead,
    title: Option<AnyElement>,
    folds: Option<(Folds, String)>,
    content: Vec<AnyElement>,
    id: ElementId,
    tint: Hsla,
    secondary: Hsla,
}

impl RenderOnce for Callout {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let toggled = self.folds.as_ref().is_some_and(|(f, key)| f.borrow().contains(key));
        let is_open = !self.head.foldable || self.head.folded == toggled;
        let title: AnyElement = self.title.unwrap_or_else(|| tr(self.head.kind.label()).into_any_element());
        let folds = self.folds.clone();
        let tint = self.tint;
        let foldable = self.head.foldable;
        let has_body = !self.content.is_empty();
        let head_id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "head".into());
        let chevron_id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "chevron".into());
        let reveal_id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "body".into());
        let row = div()
            .id(head_id)
            .flex()
            .items_center()
            .gap(px(space::S))
            .text_size(px(note_size(font::CALLOUT)))
            .child(super::widgets::icon(self.head.kind.icon(), iconsize::S, tint))
            .child(div().flex_1().min_w_0().font_weight(FontWeight::SEMIBOLD).text_color(tint).child(title))
            .when(foldable, |d| {
                d.child(super::motion::chevron(chevron_id, is_open, self.secondary)).cursor_pointer().capture_any_mouse_down(|_, _, _| press_control()).on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    if let Some((f, key)) = &folds {
                        toggle_fold(f, key);
                        cx.refresh_windows();
                    }
                })
            });
        let change_type = self.change_type.clone();
        let row: AnyElement = match change_type {
            Some((edit, item)) => row
                .context_menu(move |mut menu, _, _| {
                    for kind in ["note", "abstract", "info", "todo", "tip", "success", "question", "warning", "failure", "danger", "bug", "example", "quote"] {
                        let edit = edit.clone();
                        menu = menu.item(PopupMenuItem::new(tr(syntax::CalloutKind::parse(kind).label())).icon(super::widgets::menu_icon(syntax::CalloutKind::parse(kind).icon())).on_click(move |_, _, cx| {
                            edit(item, Rc::new(move |text| crate::core::live_text::set_callout_type(text, kind)), cx);
                        }));
                    }
                    menu
                })
                .into_any_element(),
            None => row.into_any_element(),
        };
        let body = div().w_full().pt(px(space::S)).flex().flex_col().gap(px(space::S)).children(self.content);
        super::widgets::banner(tint).w_full().px(px(space::M)).py(px(space::S)).flex().flex_col().child(row).when(has_body, |d| d.child(div().w_full().child(super::motion::reveal(reveal_id, is_open, body))))
    }
}

/// One quiet line of a note's properties; opens to all of them.
fn properties_row(props: &[frontmatter::Property], cx: &Ctx) -> AnyElement {
    let visible: Vec<&frontmatter::Property> = props.iter().collect();
    if visible.is_empty() {
        return div().hidden().into_any_element();
    }
    let order = cx.order.get();
    cx.order.set(order + 1);
    Properties { id: ElementId::from(("md-properties", order)), items: visible.into_iter().map(|p| (p.name.clone(), p.values.clone())).collect(), base: cx.base.map(Path::to_path_buf), secondary: cx.t.secondary, separator: cx.t.separator, accent: cx.t.accent_text }.into_any_element()
}

#[derive(IntoElement)]
struct Properties {
    id: ElementId,
    items: Vec<(String, Vec<String>)>,
    base: Option<PathBuf>,
    secondary: Hsla,
    separator: Hsla,
    accent: Hsla,
}

impl RenderOnce for Properties {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (secondary, separator, accent) = (self.secondary, self.separator, self.accent);
        let base = self.base.clone();
        let sample = is_sample_base(base.as_deref());
        let id = self.id.clone();
        div().flex().flex_col().pb(px(space::S)).border_b_1().border_color(separator).text_size(px(note_size(font::CALLOUT))).children(self.items.into_iter().enumerate().map(move |(i, (name, values))| {
            let base = base.clone();
            let row = ElementId::NamedChild(std::sync::Arc::new(id.clone()), SharedString::from(format!("row-{i}")));
            div()
                .id(row)
                .flex()
                .items_start()
                .gap(px(space::M))
                .py(px(space::XXS))
                .mx(px(-space::S))
                .px(px(space::S))
                .when(!sample, |d| {
                    d.cursor_pointer()
                        .soft(secondary.opacity(0.1), radius::INNER)
                        .tooltip(super::widgets::tip(tr("In den Details bearbeiten")))
                        .on_click(|_, _, cx| super::state::act(cx, |l| {
                            l.show_properties = true;
                            l.peek_inspector();
                        }))
                })
                .child(div().flex_none().w(px(140.)).truncate().text_color(secondary).child(name))
                .child(div().flex_1().min_w_0().flex().flex_wrap().gap(px(space::S)).children(values.into_iter().enumerate().map(move |(n, value)| {
                    let target = value.strip_prefix("[[").and_then(|v| v.strip_suffix("]]")).map(str::to_string);
                    match target {
                        Some(target) => {
                            let shown = target.split('|').next_back().unwrap_or(&target).to_string();
                            let action = link_action(&target, base.as_deref());
                            div()
                                .id(SharedString::from(format!("md-prop-link-{i}-{n}")))
                                .text_color(accent)
                                .when(!sample, Styled::cursor_pointer)
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    action(cx);
                                })
                                .child(shown)
                                .into_any_element()
                        }
                        None => div().child(value).into_any_element(),
                    }
                })))
        }))
    }
}

/// Index of the top-level block containing a source offset, counted like `ParsedNote`'s blocks.
pub fn block_at(src: &str, offset: usize) -> usize {
    let (mut depth, mut next, mut at) = (0, 0, 0);
    for (event, range) in Parser::new_ext(src, markdown::options()).into_offset_iter() {
        let starts_block = depth == 0 && !matches!(event, Event::End(_) | Event::Start(Tag::FootnoteDefinition(_)));
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth -= 1,
            _ => {}
        }
        if starts_block {
            if range.start > offset {
                break;
            }
            at = next;
            next += 1;
        }
    }
    at
}

/// Byte offset where each top-level block starts, counted like `block_at`.
pub fn block_starts(src: &str) -> Vec<usize> {
    let mut depth = 0;
    let mut starts = Vec::new();
    for (event, range) in Parser::new_ext(src, markdown::options()).into_offset_iter() {
        if depth == 0 && !matches!(event, Event::End(_) | Event::Start(Tag::FootnoteDefinition(_))) {
            starts.push(range.start);
        }
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }
    starts
}

/// A block to scroll to and mark briefly, after arriving from a search match.
#[derive(Clone)]
pub struct Found {
    pub block: usize,
    /// The source line of the match.
    pub line: usize,
    pub token: usize,
    /// Drawn without fading, for reduced motion; the note clears it after `FOUND_HOLD`.
    pub still: bool,
}

/// How long a search match takes to fade.
const FOUND_FADE: std::time::Duration = std::time::Duration::from_millis(1800);
/// How long a search match stays marked when motion is reduced and nothing fades.
pub const FOUND_HOLD: std::time::Duration = std::time::Duration::from_millis(3000);

/// Renders a parsed note; task boxes call `on_toggle` with their source line.
pub fn render(note: &ParsedNote, base: Option<&Path>, on_toggle: Option<OnToggle>, t: &Theme) -> Div {
    let (order, links_seen) = (Cell::new(0), Cell::new(0));
    let cx = Ctx { t, base, on_toggle, footnote_order: &note.footnote_order, order: &order, chain: Vec::new(), focus: None, links_seen: &links_seen, block: 0, head_text: Cell::new(false), cell: Cell::new(None), open_cell: None, find: None };
    div().flex().flex_col().gap(px(space::M)).children(render_blocks(&note.blocks, &cx)).children(render_footnotes(note, None, t))
}

impl ParsedNote {
    /// Which top-level blocks a folded heading hides.
    pub fn hidden_blocks(&self, folds: &Folds) -> Vec<bool> {
        let set = folds.borrow();
        let mut hidden = vec![false; self.blocks.len()];
        if set.is_empty() {
            return hidden;
        }
        let mut folded_level: Option<u8> = None;
        for (i, block) in self.blocks.iter().enumerate() {
            if let Block::Heading(level, _) = block {
                if folded_level.is_some_and(|l| *level <= l) {
                    folded_level = None;
                }
                if folded_level.is_some() {
                    hidden[i] = true;
                } else if set.contains(&format!("h{}", self.sources.get(i).map_or(0, |s| s.line))) {
                    folded_level = Some(*level);
                }
            } else if folded_level.is_some() {
                hidden[i] = true;
            }
        }
        hidden
    }
}

/// What the live view adds when it draws a block.
#[derive(Default)]
pub struct LiveDraw<'a> {
    /// The list item being drawn, for reading order and clicks.
    pub item: Option<usize>,
    /// The source line of the first line of this text.
    pub line: usize,
    /// Footnote numbers of the whole note, when only a line of it is drawn.
    pub footnotes: Option<&'a [String]>,
    /// The field of the table cell being written, with its row and column.
    pub open_cell: Option<(usize, usize, std::cell::RefCell<Option<AnyElement>>)>,
    /// How many blocks from the first are drawn together, as one line can make several.
    pub count: usize,
}

/// Renders one top-level block on its own, so long notes only build what is on screen.
pub fn render_block(note: &ParsedNote, i: usize, base: Option<&Path>, on_toggle: Option<OnToggle>, found: Option<&Found>, focus: Option<&LinkFocus>, t: &Theme) -> AnyElement {
    render_live(note, i, base, on_toggle, found, focus, LiveDraw::default(), t)
}

/// Renders one top-level block for the reader or the live view.
#[allow(clippy::too_many_arguments)]
pub fn render_live(note: &ParsedNote, i: usize, base: Option<&Path>, on_toggle: Option<OnToggle>, found: Option<&Found>, focus: Option<&LinkFocus>, live: LiveDraw, t: &Theme) -> AnyElement {
    let Some(block) = note.blocks.get(i) else { return div().into_any_element() };
    let item = live.item.unwrap_or(i);
    let together = note.blocks.get(i..(i + live.count).min(note.blocks.len())).filter(|b| b.len() > 1);
    let order = Cell::new((item as u64 + 1) * 100_000);
    let links_seen = Cell::new(note.links.iter().take_while(|(_, top)| *top < i).count());
    let cx = Ctx { t, base, on_toggle, footnote_order: live.footnotes.unwrap_or(&note.footnote_order), order: &order, chain: focus.and_then(|f| f.note.clone()).into_iter().collect(), focus, links_seen: &links_seen, block: item, head_text: Cell::new(false), cell: Cell::new(None), open_cell: live.open_cell, find: FindDraw::new(focus, |m| match (live.item, live.footnotes) {
        (Some(_), Some(_)) => (live.line == m.line).then_some(m.line_nth),
        _ => (m.block == i).then_some(m.nth),
    }) };
    let element = match together {
        Some(blocks) => div().flex().flex_col().gap(px(space::S)).children(render_blocks(blocks, &cx)).into_any_element(),
        None => render_blocks(std::slice::from_ref(block), &cx).into_iter().next().unwrap_or_else(|| div().into_any_element()),
    };
    let element = match (block, focus) {
        (Block::Heading(..), Some(focus)) => {
            let key = format!("h{}", note.sources.get(i).map_or(0, |s| s.line) + live.line);
            let is_open = !focus.folds.borrow().contains(&key);
            let folds = focus.folds.clone();
            let group = SharedString::from(format!("md-fold-{i}"));
            div()
                .relative()
                .ml(px(-20.))
                .pl(px(20.))
                .group(group.clone())
                .child(element)
                .child(
                    div()
                        .id(("md-fold", i))
                .capture_any_mouse_down(|_, _, _| press_control())
                        .absolute()
                        .top(px(note.first_line_middle(i).map_or(space::S, |middle| middle - 9.)))
                        .left(px(0.))
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .soft(t.ink.opacity(0.06), radius::SMALL)
                        .when(is_open, |d| d.invisible().group_hover(group, |s| s.visible()))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            toggle_fold(&folds, &key);
                            cx.refresh_windows();
                        })
                        .child(super::motion::chevron(("md-fold-chevron", i), is_open, t.secondary)),
                )
                .into_any_element()
        }
        _ => element,
    };
    found_mark(element, found.filter(|f| f.block == i), t)
}

/// Marks a block briefly after arriving from a search match.
fn found_mark(element: AnyElement, found: Option<&Found>, t: &Theme) -> AnyElement {
    let accent = t.accent;
    match found {
        Some(f) if f.still => div().rounded(px(radius::INNER)).bg(accent.opacity(0.25)).child(element).into_any_element(),
        Some(f) => div()
            .id(("md-found", f.token))
            .rounded(px(radius::INNER))
            .child(element)
            .with_animation(("md-found-mark", f.token), Animation::new(FOUND_FADE).with_easing(ease_in_out), move |d, delta| d.bg(accent.opacity(0.25 * (1. - delta))))
            .into_any_element(),
        None => element,
    }
}

/// Renders one row of the reader: a whole top-level block, or a part of a long list or table.
pub fn render_row(note: &ParsedNote, row: &ReaderRow, base: Option<&Path>, on_toggle: Option<OnToggle>, found: Option<&Found>, focus: Option<&LinkFocus>, t: &Theme) -> AnyElement {
    let Some((from, to)) = row.part else { return render_live(note, row.block, base, on_toggle, found, focus, LiveDraw::default(), t) };
    let (order, links_seen) = (Cell::new(row.order), Cell::new(row.links_before));
    let cx = Ctx { t, base, on_toggle, footnote_order: &note.footnote_order, order: &order, chain: focus.and_then(|f| f.note.clone()).into_iter().collect(), focus, links_seen: &links_seen, block: row.block, head_text: Cell::new(false), cell: Cell::new(None), open_cell: None, find: FindDraw::new(focus, |_| None) };
    let element = match note.blocks.get(row.block) {
        Some(Block::List { start, items }) => list_items(start, items.get(from..to.min(items.len())).unwrap_or_default(), from, &cx).into_any_element(),
        Some(Block::Table { head, rows, aligns }) => table(head, rows, aligns, Some((from, to, note.sources.get(row.block).map_or((0, 0), |s| (s.hash, s.line)))), &cx),
        _ => div().into_any_element(),
    };
    found_mark(element, found.filter(|f| f.block == row.block && (row.line..row.end_line).contains(&f.line)), t)
}

/// Brings an item into view with the row before it for context, and leaves the list alone when the item already shows.
pub fn reveal_item_in_context(list: &ListState, ix: usize) {
    let view = list.viewport_bounds();
    if list.bounds_for_item(ix).is_some_and(|b| b.top() >= view.top() && b.bottom() <= view.bottom()) {
        return;
    }
    list.scroll_to(ListOffset { item_ix: ix.saturating_sub(1), offset_in_item: px(0.) });
}

/// Scrolls a list so an item shows: not at all when it already does, just enough when it is near, else to its top, since rows not yet drawn have no height to count.
pub fn reveal_item(list: &ListState, ix: usize) {
    let view = list.viewport_bounds();
    let top = list.logical_scroll_top().item_ix;
    let near = ix <= top || (top..=ix).all(|k| list.bounds_for_item(k).is_some_and(|b| k == ix || b.bottom() < view.bottom()));
    let shown = near && list.bounds_for_item(ix).is_some_and(|b| b.top() >= view.top() && b.bottom() <= view.bottom());
    if shown {
        return;
    }
    if near {
        list.scroll_to_reveal_item(ix);
    } else {
        list.scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) });
    }
}

/// The footnotes a note refers to, as they appear at its end, each with a way back.
/// `list` scrolls back to the row a footnote is referred from, found by its source line.
pub fn render_footnotes(note: &ParsedNote, list: Option<(ListState, Rc<dyn Fn(usize) -> usize>)>, t: &Theme) -> Option<AnyElement> {
    let used: Vec<_> = note.footnote_order.iter().filter_map(|id| note.footnotes.iter().find(|(f, _)| f == id)).collect();
    if used.is_empty() {
        return None;
    }
    let (order, links_seen) = (Cell::new(u64::MAX / 2), Cell::new(0));
    let cx = Ctx { t, base: None, on_toggle: None, footnote_order: &note.footnote_order, order: &order, chain: Vec::new(), focus: None, links_seen: &links_seen, block: 0, head_text: Cell::new(false), cell: Cell::new(None), open_cell: None, find: None };
    let rows = used.iter().enumerate().map(|(i, (id, blocks))| {
        let home = note.footnote_homes.get(id).copied().unwrap_or(0);
        let list = list.clone();
        div()
            .flex()
            .gap(px(space::S))
            .text_size(px(note_size(font::CALLOUT)))
            .line_height(relative(1.5))
            .child(div().flex_none().text_color(t.secondary).child(format!("{}.", i + 1)))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XS)).children(render_blocks(blocks, &cx)))
            .child(
                div()
                    .id(("md-footnote-back", i))
                    .flex_none()
                    .cursor_pointer()
                    .text_color(t.accent_text)
                    .tooltip(super::widgets::tip(tr("Zurück zum Text")))
                    .on_click(move |_, _, _| {
                        if let Some((l, item_of_line)) = &list {
                            reveal_item(l, item_of_line(home));
                        }
                    })
                    .child("↩"),
            )
    });
    Some(div().flex().flex_col().gap(px(space::XS)).child(div().h(px(1.)).mt(px(space::S)).mb(px(space::S)).bg(t.separator)).children(rows.collect::<Vec<_>>()).into_any_element())
}

#[cfg(test)]
mod tests {
    use super::{cell_extent, column_widths, space};

    fn advance(text: &str) -> impl Fn(usize) -> f32 + '_ {
        move |i| text.char_indices().take_while(|(at, _)| *at < i).map(|(_, c)| if c == 'z' { 12. } else { 7. }).sum()
    }

    #[test]
    fn a_column_is_as_wide_as_its_text_measures_not_as_its_letters_count() {
        let (widest, word) = cell_extent("Absatz", advance("Absatz"), &[], &[]);
        assert_eq!((widest, word), (47., 47.));
        let (natural, least) = column_widths(&[vec![(widest, word)]], 1, 200.);
        assert!(natural[0] >= 47. + space::S * 2. && least[0] == natural[0]);
    }

    #[test]
    fn a_column_wraps_only_between_words() {
        let text = "Gestalt und Nähe";
        let (widest, word) = cell_extent(text, advance(text), &[], &[]);
        assert_eq!(widest, 16. * 7.);
        assert_eq!(word, 7. * 7.);
        let (natural, least) = column_widths(&[vec![(widest, word)], vec![(30., 30.)]], 1, 200.);
        assert!(least[0] < natural[0]);
        assert_eq!(least[0], (49. + space::S * 2. + 1.).ceil());
    }

    #[test]
    fn very_long_words_and_pictures_set_the_narrowest_width() {
        let long = "Donaudampfschifffahrtsgesellschaftskapitän";
        let (widest, word) = cell_extent(long, advance(long), &[], &[]);
        let (_, least) = column_widths(&[vec![(widest, word)]], 1, 180.);
        assert_eq!(least[0], (180. + space::S * 2. + 1.).ceil());
        let (widest, word) = cell_extent("a", advance("a"), &[120.], &[]);
        assert_eq!((widest, word), (127., 120.));
        let (widest, word) = cell_extent("ein Foto", advance("ein Foto"), &[], &[300.]);
        assert_eq!((widest, word), (300., 300.));
    }

    #[test]
    fn block_at_counts_like_the_parser() {
        use super::{block_at, parse};
        let src = "# Titel\n\nErster Absatz\n\n- a\n- b\n\n---\n\n```\ncode\n```\n\n| x |\n|---|\n| y |\n\nLetzter Absatz[^1]\n\n[^1]: Fuß\n";
        let note = parse(src);
        assert_eq!(block_at(src, src.find("Letzter").unwrap()), note.blocks.len() - 1);
        assert_eq!(block_at(src, src.find("Erster").unwrap()), 1);
        assert_eq!(block_at(src, src.find("code").unwrap()), 4);
    }

    use super::{Block, parse};

    #[test]
    fn lists_top_level_headings() {
        let note = parse("# Eins\n\ntext\n\n## Zwei **fett**\n\n> # im Zitat\n");
        assert_eq!(note.headings(), vec![(1, "Eins".to_string()), (2, "Zwei fett".to_string())]);
    }

    #[test]
    fn records_task_lines() {
        let note = parse("# T\n\n- [ ] a\n- [x] b\n\ntext");
        let Some(Block::List { items, .. }) = note.blocks.iter().find(|b| matches!(b, Block::List { .. })) else { panic!("no list") };
        assert_eq!(items.iter().map(|i| (i.task, i.line)).collect::<Vec<_>>(), vec![(Some(' '), 2), (Some('x'), 3)]);
    }
}

#[cfg(test)]
mod reader_tests {
    use super::{Block, html_text, parse, plain_text};

    #[test]
    fn a_hidden_comment_leaves_one_space() {
        let note = parse("und <!-- verborgen --> sichtbar, <kbd>K</kbd> und `a  b`\n");
        let Some(Block::Paragraph(inlines)) = note.blocks.first() else { panic!("no paragraph") };
        assert_eq!(plain_text(inlines), "und sichtbar, K und a  b");
    }

    fn kinds(src: &str) -> Vec<&'static str> {
        parse(src)
            .blocks
            .iter()
            .map(|b| match b {
                Block::Heading(..) => "heading",
                Block::Paragraph(_) => "paragraph",
                Block::Quote(_) => "quote",
                Block::Code(..) => "code",
                Block::List { .. } => "list",
                Block::Rule => "rule",
                Block::Table { .. } => "table",
                Block::Callout(..) => "callout",
                Block::Properties(_) => "properties",
                Block::Centered(_) => "centered",
                Block::Right(_) => "right",
                Block::Details(..) => "details",
                Block::Math(_) => "math",
                Block::Nothing => "nothing",
            })
            .collect()
    }

    #[test]
    fn a_foldable_callout_title_loses_its_mark() {
        let parsed = parse("> [!tip]+ Titel mit **fett**\n> Inhalt\n");
        let Block::Callout(_, title, _) = &parsed.blocks[0] else { panic!() };
        assert_eq!(super::plain_text(title), "Titel mit fett");
    }

    #[test]
    fn display_math_reaches_the_reader_in_order() {
        assert_eq!(kinds("$$\na^2\n$$\n"), ["math"]);
        assert_eq!(kinds("Text $$a$$ mehr\n"), ["paragraph", "math", "paragraph"]);
        assert_eq!(kinds("$$a$$\n\nDanach\n"), ["math", "paragraph"]);
    }

    #[test]
    fn text_beside_display_math_has_no_stray_space_or_break() {
        let texts = |src: &str| parse(src).blocks.iter().filter_map(|b| if let Block::Paragraph(inl) = b { Some(plain_text(inl)) } else { None }).collect::<Vec<_>>();
        assert_eq!(texts("Text davor $$a$$ Text danach.\n"), ["Text davor", "Text danach."]);
        assert_eq!(texts("A\n$$x$$\nB\n"), ["A", "B"]);
        assert_eq!(kinds("$$x$$ danach\n"), ["math", "paragraph"]);
    }

    #[test]
    fn a_line_draws_every_block_it_holds() {
        assert_eq!(parse("Text davor $$a$$ Text danach.\n\n[x]: https://beispiel.de\n").line_blocks(), 3);
        assert_eq!(parse("Nur Text\n\n[a] b ]: c\n").line_blocks(), 1);
        assert_eq!(parse("").line_blocks(), 0);
    }

    #[test]
    fn a_paragraph_with_display_math_draws_every_block_of_its_lines() {
        let note = parse("Formel $x$ und\n$$y$$ danach\n\nNext\n");
        assert_eq!(kinds("Formel $x$ und\n$$y$$ danach\n"), ["paragraph", "math", "paragraph"]);
        assert_eq!(note.blocks_sharing(0), 3);
        assert_eq!(note.blocks_sharing(3), 1);
        assert_eq!(note.blocks_sharing(9), 0);
    }

    #[test]
    fn a_heading_keeps_its_text_before_display_math() {
        assert_eq!(kinds("# Titel $$x$$ mehr\n"), ["heading", "math", "paragraph"]);
        let note = parse("# Titel $$x$$ mehr\n");
        let Block::Heading(_, title) = &note.blocks[0] else { panic!() };
        assert_eq!(plain_text(title), "Titel");
        let Block::Paragraph(rest) = &note.blocks[2] else { panic!() };
        assert_eq!(plain_text(rest), "mehr");
        assert!(note.has_margin_control(0));
    }

    #[test]
    fn tight_items_and_quotes_have_no_stray_space_beside_display_math() {
        let item = parse("- a $$x$$ b\n");
        let Block::List { items, .. } = &item.blocks[0] else { panic!() };
        let texts: Vec<String> = items[0].blocks.iter().filter_map(|b| if let Block::Paragraph(inl) = b { Some(plain_text(inl)) } else { None }).collect();
        assert_eq!(texts, ["a", "b"]);
        let quote = parse("> a $$x$$ b\n");
        let Block::Quote(inner) = &quote.blocks[0] else { panic!() };
        let texts: Vec<String> = inner.iter().filter_map(|b| if let Block::Paragraph(inl) = b { Some(plain_text(inl)) } else { None }).collect();
        assert_eq!(texts, ["a", "b"]);
    }

    #[test]
    fn the_gutter_meets_the_first_line() {
        let middle = |src: &str| parse(src).first_line_middle(0);
        let body = middle("Text\n").unwrap();
        assert!(middle("# Titel\n").unwrap() > body);
        assert!(middle("> [!note] Titel\n> a\n").unwrap() > body);
        assert!(middle("```\ncode\n```\n").unwrap() > body);
        assert_eq!(middle("$$\nx\n$$\n"), None);
        assert!(parse("# Titel\n").has_margin_control(0));
        assert!(parse("- a\n  - b\n").has_margin_control(0));
        assert!(!parse("- a\n- b\n").has_margin_control(0));
        assert!(!parse("Text\n").has_margin_control(0));
    }

    #[test]
    fn scripts_styles_and_comments_stay_hidden() {
        assert_eq!(kinds("<script>\nvar a = 1;\n\nvar b = 2;\n</script>\n\nText\n"), ["paragraph"]);
        assert_eq!(kinds("<!--\nverborgen\nauch das\n-->\n\nText\n"), ["paragraph"]);
        assert_eq!(kinds("<style>\na { color: red }\n</style>\n"), [] as [&str; 0]);
        assert_eq!(html_text("<div>sichtbar</div><script>weg</script>").trim(), "sichtbar");
    }

    #[test]
    fn blocks_in_a_tight_list_keep_their_order() {
        let parsed = parse("- Liste\n  > Zitat\n");
        let Block::List { items, .. } = &parsed.blocks[0] else { panic!() };
        assert!(matches!(items[0].blocks[0], Block::Paragraph(_)), "text comes first");
        assert!(matches!(items[0].blocks[1], Block::Quote(_)));
    }

    #[test]
    fn links_spaces_and_lines_read_the_same_in_a_long_note() {
        use super::Inline;
        let src = format!("{}Text  mit\tLücken, www.beispiel.de, muster@beispiel.de und https://beispiel.de/a.\n\n- eins\n- zwei\n", "Absatz ohne Adresse.\n\n".repeat(300));
        let note = parse(&src);
        let Some(Block::Paragraph(inlines)) = note.blocks.get(300) else { panic!("no paragraph") };
        let links: Vec<(&str, &str)> = inlines.iter().filter_map(|i| if let Inline::Link(shown, _, to) = i { Some((shown.as_str(), to.as_str())) } else { None }).collect();
        assert_eq!(links, [("www.beispiel.de", "https://www.beispiel.de"), ("muster@beispiel.de", "mailto:muster@beispiel.de"), ("https://beispiel.de/a", "https://beispiel.de/a")]);
        assert!(plain_text(inlines).starts_with("Text mit Lücken, "));
        assert_eq!(note.sources().iter().map(|s| s.line).skip(299).collect::<Vec<_>>(), [598, 600, 602]);
        let Some(Block::List { items, .. }) = note.blocks.last() else { panic!("no list") };
        assert_eq!(items.iter().map(|i| i.line).collect::<Vec<_>>(), [602, 603]);
    }

    #[test]
    fn long_lists_and_tables_are_drawn_in_parts_that_keep_lines_links_and_order() {
        let items: String = (0..100).map(|i| if i == 70 { format!("{i}. Punkt [Ziel](https://beispiel.de)[^a]
") } else { format!("{i}. Punkt
") }).collect();
        let body: String = (0..90).map(|i| if i == 50 { format!("| {i} | [b](https://beispiel.de/b) |
") } else { format!("| {i} | x |
") }).collect();
        let src = format!("# Titel

{items}
| A | B |
|---|---|
{body}
Ende

[^a]: Fuß
");
        let note = parse(&src);
        let rows = note.rows();
        let chunks = |n: usize| (0..n).step_by(super::PART).map(move |from| (from, (from + super::PART).min(n)));
        let (lists, tables) = (chunks(100).count(), chunks(90).count());
        let mut expected = vec![(0, None, 0, 1)];
        expected.extend(chunks(100).map(|(from, to)| (1, Some((from, to)), 2 + from, if to == 100 { 102 } else { 2 + to })));
        expected.extend(chunks(90).map(|(from, to)| (2, Some((from, to)), if from == 0 { 103 } else { 105 + from }, if to == 90 { 195 } else { 105 + to })));
        expected.push((3, None, 196, 197));
        assert_eq!(rows.iter().map(|r| (r.block, r.part, r.line, r.end_line)).collect::<Vec<_>>(), expected);
        assert!(rows.windows(2).all(|w| w[0].order < w[1].order));
        assert_eq!(rows[1].order, 200_000);
        let (list_row, table_row) = (|item: usize| 1 + item / super::PART, |row: usize| 1 + lists + row / super::PART);
        assert_eq!(note.row_of_line(72), list_row(70));
        assert_eq!(note.row_of_line(104), table_row(0));
        assert_eq!(note.row_of_line(155), table_row(50));
        assert_eq!(note.row_of_line(500), rows.len() - 1);
        assert_eq!(note.links().len(), 2);
        assert_eq!((note.row_of_link(0), note.row_of_link(1)), (list_row(70), table_row(50)));
        assert!(rows.iter().enumerate().all(|(i, r)| r.links_before == usize::from(i > list_row(70)) + usize::from(i > table_row(50))));
        assert_eq!(note.footnote_homes.get("a"), Some(&72));
        assert_eq!((note.row_gap(1), note.row_gap(lists), note.row_gap(lists + 1), note.row_gap(lists + tables)), (super::space::XS, super::space::M, 0., super::space::M));
        let again = parse(&src.replacen("50 | [b]", "50 | [c]", 1));
        let changed: Vec<usize> = rows.iter().zip(again.rows()).enumerate().filter(|(_, (a, b))| a.hash != b.hash).map(|(i, _)| i).collect();
        assert_eq!(changed, (1 + lists..1 + lists + tables).collect::<Vec<_>>());
        let toggled = parse(&src.replacen("75. Punkt", "75. Punkt!", 1));
        let changed: Vec<usize> = rows.iter().zip(toggled.rows()).enumerate().filter(|(_, (a, b))| a.hash != b.hash).map(|(i, _)| i).collect();
        assert_eq!(changed, [list_row(75)]);
    }

    #[test]
    fn short_lists_and_tables_stay_one_row() {
        let note = parse("- a
- b

| A |
|---|
| 1 |
");
        assert_eq!(note.rows().iter().map(|r| r.part).collect::<Vec<_>>(), [None, None]);
        assert_eq!(note.rows().iter().map(|r| r.hash).collect::<Vec<_>>(), note.sources().iter().map(|s| s.hash).collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod find_tests {
    use super::{block_at, block_starts, with_fills};
    use super::{FontWeight, HighlightStyle, Hsla};

    fn bold() -> HighlightStyle {
        HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() }
    }

    #[test]
    fn fills_split_styles_where_they_overlap() {
        let fill = Hsla { h: 0.1, s: 1., l: 0.5, a: 0.3 };
        let out = with_fills(vec![(0..6, bold())], &[(4..9, fill)]);
        let ranges: Vec<_> = out.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, [0..4, 4..6, 6..9]);
        assert_eq!(out[0].1, bold());
        assert_eq!(out[1].1, HighlightStyle { background_color: Some(fill), ..bold() });
        assert_eq!(out[2].1, HighlightStyle { background_color: Some(fill), ..Default::default() });
    }

    #[test]
    fn fills_keep_the_other_styles_and_stay_ordered() {
        let fill = Hsla { h: 0.1, s: 1., l: 0.5, a: 0.3 };
        let styles = vec![(0..2, bold()), (5..8, bold())];
        let out = with_fills(styles.clone(), &[(3..4, fill), (9..10, fill)]);
        let ranges: Vec<_> = out.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, [0..2, 3..4, 5..8, 9..10]);
        assert!(out.windows(2).all(|w| w[0].0.end <= w[1].0.start));
        assert_eq!(with_fills(styles.clone(), &[]), styles);
    }

    #[test]
    fn block_starts_agree_with_block_at() {
        let src = "# Titel

Erster Absatz

- a
- b

Letzter
";
        let starts = block_starts(src);
        assert_eq!(starts.len(), 4);
        for offset in 0..src.len() {
            assert_eq!(starts.partition_point(|s| *s <= offset).saturating_sub(1), block_at(src, offset), "offset {offset}");
        }
    }

    #[test]
    fn neighbouring_fills_with_one_style_join() {
        let fill = Hsla { h: 0.1, s: 1., l: 0.5, a: 0.3 };
        let out = with_fills(Vec::new(), &[(0..2, fill), (2..4, fill)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, 0..4);
    }
}
