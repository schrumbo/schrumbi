use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use super::selectable::{InlineKind, InlineSlot, LinkAction, selectable};
use super::theme::{Theme, font, radius, space, iconsize};
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

/// A parsed note, kept between frames so long notes don't re-parse while scrolling.
pub struct ParsedNote {
    /// Source line and content hash of each top-level block, in order.
    sources: Vec<(usize, u64)>,
    /// Every link in reading order, with the top-level block it sits in.
    links: Vec<(String, usize)>,
    blocks: Vec<Block>,
    footnotes: Vec<(String, Vec<Block>)>,
    /// The top-level block that first refers to a footnote.
    footnote_homes: HashMap<String, usize>,
    footnote_order: Vec<String>,
}

fn line_of(src: &str, offset: usize) -> usize {
    src[..offset.min(src.len())].bytes().filter(|b| *b == b'\n').count()
}

/// Parses Markdown into blocks with source lines for task items.
pub fn parse(src: &str) -> ParsedNote {
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
    let mut sources: Vec<(usize, u64)> = Vec::new();
    let mut footnote_homes: HashMap<String, usize> = HashMap::new();
    let mut last_range = 0..0;
    let mut bare_wikilink = false;
    let mut raised: Option<fn(&str) -> String> = None;
    for (event, range) in Parser::new_ext(src, markdown::options()).into_offset_iter() {
        note_sources(src, &stack, &last_range, &mut sources);
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
                    item_meta.push((None, line_of(src, range.start)));
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
                    let (_, inner, mut inl) = stack.pop().unwrap();
                    join_marks(&mut inl);
                    let had_blocks = !inner.is_empty();
                    stack.last_mut().unwrap().1.extend(inner);
                    if !(had_blocks && inl.is_empty()) {
                        let block = if strip_block_id(&mut inl) && plain_text(&inl).trim().is_empty() { Block::Nothing } else { Block::Paragraph(inl) };
                        stack.last_mut().unwrap().1.push(block);
                    }
                }
                TagEnd::Heading(level) => {
                    let (_, inner, mut inl) = stack.pop().unwrap();
                    join_marks(&mut inl);
                    stack.last_mut().unwrap().1.push(Block::Heading(markdown::heading_number(level), inl));
                    stack.last_mut().unwrap().1.extend(inner);
                }
                TagEnd::BlockQuote(_) => {
                    flush_inline(&mut stack);
                    let (_, blocks, _) = stack.pop().unwrap();
                    stack.last_mut().unwrap().1.push(into_callout(blocks));
                }
                TagEnd::FootnoteDefinition => {
                    flush_inline(&mut stack);
                    if let (Some(Tag::FootnoteDefinition(id)), blocks, _) = stack.pop().unwrap() {
                        footnotes.push((id.to_string(), blocks));
                    }
                }
                TagEnd::Item => {
                    flush_inline(&mut stack);
                    let (_, mut blocks, _) = stack.pop().unwrap();
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
                            None => push_with_autolinks(target, &collapse_spaces(&s), style),
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
                    footnote_homes.insert(id.to_string(), stack.first().map_or(0, |s| s.1.len()));
                }
                stack.last_mut().unwrap().2.push(Inline::FootnoteRef(id.to_string()));
            }
        }
    }
    flush_inline(&mut stack);
    note_sources(src, &stack, &last_range, &mut sources);
    let blocks = stack.pop().map(|s| s.1).unwrap_or_default();
    let mut links = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        collect_links(std::slice::from_ref(block), i, &mut links);
    }
    ParsedNote { sources, links, blocks, footnotes, footnote_homes, footnote_order }
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
    static URL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"https?://[^\s<>\]\)]+").unwrap());
    let mut copied = 0;
    for hit in URL.find_iter(text) {
        let shown = hit.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"']);
        if hit.start() > copied {
            target.push(Inline::Text(text[copied..hit.start()].to_string(), style));
        }
        target.push(Inline::Link(shown.to_string(), Style { link: true, ..style }, shown.to_string()));
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
fn note_sources<T>(src: &str, stack: &[(T, Vec<Block>, Vec<Inline>)], range: &std::ops::Range<usize>, sources: &mut Vec<(usize, u64)>) {
    use std::hash::{Hash, Hasher};
    let [(_, blocks, _)] = stack else { return };
    while sources.len() < blocks.len() {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        src.get(range.clone()).unwrap_or_default().hash(&mut hash);
        sources.push((line_of(src, range.start), hash.finish()));
    }
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
            Block::Quote(inner) | Block::Callout(_, _, inner) | Block::Centered(inner) | Block::Details(_, inner, _) => collect_links(inner, top, out),
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
        title.retain_mut(|piece| {
            if skipped {
                return true;
            }
            let Inline::Text(text, _) = piece else { return true };
            match text.find(']') {
                Some(close) => {
                    skipped = true;
                    *text = text[close + 1..].trim_start_matches(['+', '-']).trim_start().to_string();
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
    matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "section" | "article" | "aside" | "header" | "footer" | "main" | "nav" | "figure" | "figcaption" | "center" | "blockquote" | "ul" | "ol" | "table" | "hr" | "details" | "pre" | "dl" | "dt" | "dd")
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
            _ => {
                let align = html_attribute(attributes, "align").unwrap_or_default().to_lowercase();
                let centered = tag == "center" || align == "center" || html_attribute(attributes, "style").is_some_and(|s| s.replace(' ', "").to_lowercase().contains("text-align:center"));
                let inner = html_nodes_to_blocks(children);
                if centered {
                    blocks.push(Block::Centered(inner));
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
    /// What the reader has folded, kept across sessions.
    pub folds: Folds,
}

/// The foldable things whose state differs from how the note starts.
pub type Folds = Rc<RefCell<std::collections::HashSet<String>>>;

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
    let mut run: &[Inline] = inlines;
    while let Some(at) = run.iter().position(is_big) {
        if !plain_text(&run[..at]).trim().is_empty() || run[..at].iter().any(|i| matches!(i, Inline::Image(..) | Inline::Math(_))) {
            parts.push(styled_line(&run[..at], cx, size));
        }
        if let Inline::Image(src, alt, target) = &run[at] {
            parts.push(linked_image(src, alt, target.as_deref(), cx));
        }
        run = &run[at + 1..];
    }
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
    div().id(("md-image-link", order)).cursor_pointer().on_click(move |_, _, cx| action(cx)).child(shown).into_any_element()
}

fn styled_line(inlines: &[Inline], cx: &Ctx, size: f32) -> AnyElement {
    let t = cx.t;
    let mut text = String::new();
    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    let mut links: Vec<(std::ops::Range<usize>, LinkAction)> = Vec::new();
    let mut hovers: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut slots: Vec<InlineSlot> = Vec::new();
    let mut pills: Vec<std::ops::Range<usize>> = Vec::new();
    for i in inlines {
        match i {
            Inline::Tag(tag) => {
                let start = text.len();
                text.push('#');
                text.push_str(tag);
                let name = tag.clone();
                links.push((start..text.len(), Rc::new(move |cx| super::state::act(cx, |l| l.show_tag(&name)))));
                pills.push(start..text.len());
                highlights.push((start..text.len(), HighlightStyle { color: Some(t.accent_text), ..Default::default() }));
            }
            Inline::Text(s, st) | Inline::Link(s, st, _) => {
                let start = text.len();
                text.push_str(s);
                let mut unresolved = false;
                if let Inline::Link(_, _, url) = i {
                    unresolved = !is_external(url) && !link_exists(url, cx.base);
                    if is_external(url) {
                        text.push_str(" ↗");
                    }
                    links.push((start..text.len(), link_action(url, cx.base)));
                    hovers.push((start..text.len(), url.clone()));
                    if let Some(focus) = cx.focus {
                        let index = cx.links_seen.get();
                        cx.links_seen.set(index + 1);
                        if focus.index == Some(index) {
                            highlights.push((start..text.len(), HighlightStyle { background_color: Some(t.accent.opacity(0.28)), ..Default::default() }));
                        }
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
                    links.push((start..text.len(), Rc::new(move |_| list.scroll_to_reveal_item(list.item_count().saturating_sub(1)))));
                }
            }
            Inline::Image(src, alt, target) => {
                if let Some((w, h)) = fits_in_line(alt, size) {
                    slots.push(InlineSlot { at: text.len(), kind: InlineKind::Sized(linked_image(src, alt, target.as_deref(), cx), w, h) });
                }
            }
            Inline::Math(source) => slots.push(InlineSlot { at: text.len(), kind: InlineKind::Math(source.clone()) }),
        }
    }
    let order = cx.order.get();
    cx.order.set(order + 1);
    let base = cx.base.map(Path::to_path_buf);
    let body = div().text_size(px(size)).line_height(relative(1.5)).child(selectable(("md-text", order), text, highlights, order, t.accent.opacity(0.3)).links(links).hovers(hovers, base).inline(slots).pills(pills, t.accent.opacity(0.14)));
    body.into_any_element()
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

/// Web links open in the browser; everything else is a link inside the vault, created if it is missing.
pub fn link_action(url: &str, base: Option<&Path>) -> LinkAction {
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
                    let _ = opener::open(&url);
                }
            }
        });
    }
    if is_external(&url) {
        return Rc::new(move |_| {
            let _ = opener::open(&url);
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
        .cursor_pointer()
        .hover(|s| s.bg(t.ink.opacity(0.04)))
        .on_click(move |_, _, cx| super::state::act(cx, |l| l.open_file(&open)))
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
    let Some(path) = beside.or_else(|| vault.resolver.resolve(&target, from.as_deref())) else { return missing_embed(&target.path, cx) };
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if IMAGE_TYPES.contains(&ext.as_str()) {
        return image_at(path, alt, cx);
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
    if cx.chain.len() >= 3 {
        return div().id(("md-embed-link", order)).text_color(t.accent_text).cursor_pointer().on_click(move |_, _, cx| super::state::act(cx, |l| l.open_file(&open))).child(name).into_any_element();
    }
    let note = embedded_note(&path, target);
    let mut chain = cx.chain.clone();
    chain.push(path.clone());
    let inner = Ctx { t, base: path.parent(), on_toggle: None, footnote_order: &note.footnote_order, order: cx.order, chain, focus: None, links_seen: cx.links_seen };
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
                .cursor_pointer()
                .text_size(px(font::CAPTION))
                .text_color(t.secondary)
                .on_click(move |_, _, cx| super::state::act(cx, |l| l.open_file(&open)))
                .child(div().flex_1().min_w_0().truncate().child(name))
                .child(super::widgets::icon("external-link", iconsize::XS, t.tertiary)),
        )
        .child(div().p(px(space::M)).flex().flex_col().gap(px(space::M)).children(blocks))
        .into_any_element()
}

fn image(src: &str, alt: &str, cx: &Ctx) -> AnyElement {
    if src.contains("://") {
        let (width, height) = image_size(alt);
        return img(SharedString::from(src.to_string())).max_w_full().rounded(px(radius::CONTROL)).when_some(width, |d, w| d.w(px(w))).when_some(height, |d, h| d.h(px(h))).into_any_element();
    }
    let path = markdown::percent_decode(src);
    let p: PathBuf = if Path::new(&path).is_absolute() { PathBuf::from(&path) } else { cx.base.map(|b| b.join(&path)).unwrap_or_else(|| PathBuf::from(&path)) };
    if p.exists() && IMAGE_TYPES.contains(&p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default().as_str()) {
        return image_at(p, alt, cx);
    }
    embed(src, alt, cx)
}

fn image_at(p: PathBuf, alt: &str, cx: &Ctx) -> AnyElement {
    let (width, height) = image_size(alt);
    let _ = &alt;
    {
        let order = cx.order.get();
        cx.order.set(order + 1);
        let open = p.clone();
        div().id(("md-image", order)).cursor_pointer().on_click(move |_, _, cx| super::preview::show(&open, cx)).child(img(p).when_some(width, |d, w| d.w(px(w))).when_some(height, |d, h| d.h(px(h))).max_w_full().max_h(px(560.)).object_fit(ObjectFit::Contain).rounded(px(radius::CONTROL))).into_any_element()
    }
}

thread_local! {
    static COLOURED: RefCell<HashMap<(Option<String>, String), Rc<Vec<(String, TokenKind)>>>> = RefCell::new(HashMap::new());
}

/// Colours code once; scrolling draws the same block again and again.
fn coloured(lang: Option<&str>, code: &str) -> Rc<Vec<(String, TokenKind)>> {
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
    let mut text = String::new();
    let mut highlights = Vec::new();
    for (s, kind) in coloured(lang, code).iter() {
        let start = text.len();
        text.push_str(s);
        let color = match kind {
            TokenKind::Plain => None,
            TokenKind::Keyword => Some(t.palette("pink")),
            TokenKind::String => Some(t.palette("orange")),
            TokenKind::Comment => Some(t.secondary),
            TokenKind::Number => Some(t.palette("purple")),
            TokenKind::Type => Some(t.palette("teal")),
            TokenKind::Attribute => Some(t.palette("brown")),
        };
        if let Some(c) = color {
            highlights.push((start..text.len(), HighlightStyle { color: Some(c), ..Default::default() }));
        }
    }
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
                .rounded(px(radius::CONTROL))
                .bg(t.secondary_bg)
                .border_1()
                .border_color(t.separator.opacity(0.6))
                .font_family(Theme::mono_family())
                .text_size(px(font::CALLOUT))
                .child(selectable(("md-code-text", order), text, highlights, order, t.accent.opacity(0.3))),
        )
        .child(
            div()
                .id(("md-code-copy", order))
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
                .invisible()
                .group_hover(group, |s| s.visible())
                .tooltip(super::widgets::tip(tr("Code kopieren")))
                .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone())))
                .child(super::widgets::icon("copy", iconsize::S, t.secondary)),
        )
        .into_any_element()
}

fn render_blocks(blocks: &[Block], cx: &Ctx) -> Vec<AnyElement> {
    let t = cx.t;
    blocks
        .iter()
        .map(|b| match b {
            Block::Heading(level, inl) => {
                let size = match level {
                    1 => font::TITLE,
                    2 => font::TITLE2,
                    3 => font::TITLE3,
                    4 => font::BODY,
                    _ => font::CALLOUT,
                };
                div().pt(px(if *level <= 2 { space::S } else { space::XS })).font_weight(if *level == 1 { FontWeight::BOLD } else { FontWeight::SEMIBOLD }).child(styled_text(inl, cx, size)).into_any_element()
            }
            Block::Paragraph(inl) => styled_text(inl, cx, font::BODY),
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
                let shown = (!plain_text(title).trim().is_empty() || title.iter().any(|i| matches!(i, Inline::Link(..) | Inline::Math(_)))).then(|| styled_text(title, cx, font::CALLOUT));
                let key = format!("c{order}");
                Callout { head: head.clone(), title: shown, content: render_blocks(inner, cx), id: ElementId::from(("md-callout", order)), tint: callout_tint(head, t), secondary: t.secondary, folds: cx.focus.map(|f| (f.folds.clone(), key)) }.into_any_element()
            }
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
                            .font_weight(FontWeight::MEDIUM)
                            .child(super::motion::chevron(("md-details-chevron", order), is_open, t.secondary))
                            .child(styled_text(summary, cx, font::BODY))
                            .on_click(move |_, window, _| {
                                if let Some(f) = &folds {
                                    toggle_fold(f, &toggle_key);
                                    window.refresh();
                                }
                            }),
                    )
                    .when(is_open, |d| d.child(div().pl(px(space::L)).flex().flex_col().gap(px(space::S)).children(render_blocks(inner, cx))))
                    .into_any_element()
            }
            Block::List { start, items } => div()
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
                                .mt(px(space::XS))
                                .ml(px(space::XXS))
                                .size(px(14.))
                                .flex_none()
                                .rounded(px(radius::SMALL))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(done, |d| d.bg(t.accent))
                                .when(!done, |d| d.border_1().border_color(if special { t.accent } else { t.secondary }))
                                .when(special, |d| d.text_size(px(font::CAPTION2)).text_color(t.accent).child(mark.to_string()))
                                .when(toggle.is_some(), |d| d.cursor_pointer())
                                .when(done, |d| d.child(super::widgets::icon("check", iconsize::XS, white())))
                                .on_click(move |_, _, cx| {
                                    if let Some(f) = &toggle {
                                        f(line, cx);
                                    }
                                })
                                .into_any_element()
                        }
                        (None, Some(s)) => div().flex_none().min_w(px(16.)).flex().justify_end().text_size(px(font::BODY)).line_height(relative(1.5)).text_color(t.secondary).child(format!("{}.", s + i as u64)).into_any_element(),
                        (None, None) => div().flex_none().min_w(px(16.)).flex().justify_end().text_size(px(font::BODY)).line_height(relative(1.5)).text_color(t.secondary).child("•").into_any_element(),
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
                            if matches!(b, Block::List { .. }) { div().border_l_1().border_color(t.separator).pl(px(space::S)).child(inner).into_any_element() } else { inner }
                        })
                        .collect();
                    let fold_button = (has_nested && folds.is_some()).then(|| {
                        let line = item.line;
                        div()
                            .id(("md-list-fold", line))
                            .absolute()
                            .top(px(space::XS))
                            .left(px(-16.))
                            .size(px(14.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .on_click(move |_, window, _| {
                                if let Some(f) = &folds {
                                    toggle_fold(f, &fold_key);
                                    window.refresh();
                                }
                            })
                            .child(super::motion::chevron(("md-list-chevron", line), !folded, t.tertiary))
                    });
                    div()
                        .relative()
                        .flex()
                        .items_start()
                        .gap(px(space::S))
                        .children(fold_button)
                        .child(marker)
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XS)).when(done, |d| d.text_color(t.secondary).line_through()).children(shown_blocks))
                })
            )
            .into_any_element(),
            Block::Table { head, rows, aligns } => {
                let width_of = |inl: &Vec<Inline>| plain_text(inl).chars().count() as f32;
                let widths: Vec<f32> = (0..head.len())
                    .map(|c| rows.iter().filter_map(|r| r.get(c)).chain(head.get(c)).map(width_of).fold(4_f32, f32::max).min(40.))
                    .collect();
                let total: f32 = widths.iter().sum::<f32>().max(1.);
                let cell = |inl: &Vec<Inline>, i: usize, bold: bool| {
                    let share = widths.get(i).copied().unwrap_or(4.) / total;
                    let d = div().flex_none().w(relative(share)).px(px(space::S)).py(px(space::XS)).when(bold, |d| d.font_weight(FontWeight::SEMIBOLD));
                    let d = match aligns.get(i) {
                        Some(pulldown_cmark::Alignment::Center) => d.flex().justify_center(),
                        Some(pulldown_cmark::Alignment::Right) => d.flex().justify_end(),
                        _ => d,
                    };
                    d.child(styled_text(inl, cx, font::BODY))
                };
                let order = cx.order.get();
                cx.order.set(order + 1);
                let min_width = px(widths.iter().map(|w| w.min(14.) * 9. + space::M).sum::<f32>());
                div().id(("md-table", order)).w_full().overflow_x_scroll().child(div()
                    .min_w(min_width)
                    .flex()
                    .flex_col()
                    .rounded(px(radius::CONTROL))
                    .border_1()
                    .border_color(t.separator)
                    .overflow_hidden()
                    .child(div().flex().bg(t.secondary_bg).children(head.iter().enumerate().map(|(i, c)| cell(c, i, true))))
                    .children(rows.iter().map(|r| div().flex().border_t_1().border_color(t.separator).children(r.iter().enumerate().map(|(i, c)| cell(c, i, false))))))
                    .into_any_element()
            }
        })
        .collect()
}

impl ParsedNote {
    /// The note's top-level headings as level and text, for a table of contents.
    /// The number of top-level blocks.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Source line and content hash of each top-level block.
    pub fn sources(&self) -> &[(usize, u64)] {
        &self.sources
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
            .text_size(px(font::CALLOUT))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(tint)
            .child(super::widgets::icon(self.head.kind.icon(), iconsize::S, tint))
            .child(div().flex_1().min_w_0().child(title))
            .when(foldable, |d| {
                d.child(super::motion::chevron(chevron_id, is_open, self.secondary)).cursor_pointer().on_click(move |_, window, _| {
                    if let Some((f, key)) = &folds {
                        toggle_fold(f, key);
                        window.refresh();
                    }
                })
            });
        let body = div().w_full().pt(px(space::S)).flex().flex_col().gap(px(space::S)).children(self.content);
        super::widgets::banner(tint).w_full().px(px(space::M)).py(px(space::S)).flex().flex_col().child(row).when(has_body, |d| d.child(div().w_full().child(super::motion::reveal(reveal_id, is_open, body))))
    }
}

/// One quiet line of a note's properties; opens to all of them.
fn properties_row(props: &[frontmatter::Property], cx: &Ctx) -> AnyElement {
    let visible: Vec<&frontmatter::Property> = props.iter().filter(|p| p.name != "title").collect();
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
        let id = self.id.clone();
        div().flex().flex_col().pb(px(space::S)).border_b_1().border_color(separator).text_size(px(font::CALLOUT)).children(self.items.into_iter().enumerate().map(move |(i, (name, values))| {
            let base = base.clone();
            let row = ElementId::NamedChild(std::sync::Arc::new(id.clone()), SharedString::from(format!("row-{i}")));
            div()
                .id(row)
                .flex()
                .items_start()
                .gap(px(space::M))
                .py(px(space::XXS))
                .cursor_pointer()
                .tooltip(super::widgets::tip(tr("In den Details bearbeiten")))
                .on_click(|_, _, cx| super::state::act(cx, |l| {
                    l.show_properties = true;
                    l.show_inspector = true;
                }))
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
                                .cursor_pointer()
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

/// A block to scroll to and mark briefly, after arriving from a search match.
#[derive(Clone)]
pub struct Found {
    pub block: usize,
    pub token: usize,
}

/// Renders a parsed note; task boxes call `on_toggle` with their source line.
pub fn render(note: &ParsedNote, base: Option<&Path>, on_toggle: Option<OnToggle>, t: &Theme) -> Div {
    let (order, links_seen) = (Cell::new(0), Cell::new(0));
    let cx = Ctx { t, base, on_toggle, footnote_order: &note.footnote_order, order: &order, chain: Vec::new(), focus: None, links_seen: &links_seen };
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
                } else if set.contains(&format!("h{}", self.sources.get(i).map_or(0, |s| s.0))) {
                    folded_level = Some(*level);
                }
            } else if folded_level.is_some() {
                hidden[i] = true;
            }
        }
        hidden
    }
}

/// Renders one top-level block on its own, so long notes only build what is on screen.
pub fn render_block(note: &ParsedNote, i: usize, base: Option<&Path>, on_toggle: Option<OnToggle>, found: Option<&Found>, focus: Option<&LinkFocus>, t: &Theme) -> AnyElement {
    let Some(block) = note.blocks.get(i) else { return div().into_any_element() };
    let order = Cell::new((i as u64 + 1) * 100_000);
    let links_seen = Cell::new(note.links.iter().take_while(|(_, top)| *top < i).count());
    let cx = Ctx { t, base, on_toggle, footnote_order: &note.footnote_order, order: &order, chain: focus.and_then(|f| f.note.clone()).into_iter().collect(), focus, links_seen: &links_seen };
    let element = render_blocks(std::slice::from_ref(block), &cx).into_iter().next().unwrap_or_else(|| div().into_any_element());
    let element = match (block, focus) {
        (Block::Heading(..), Some(focus)) => {
            let key = format!("h{}", note.sources.get(i).map_or(0, |s| s.0));
            let is_open = !focus.folds.borrow().contains(&key);
            let folds = focus.folds.clone();
            let group = SharedString::from(format!("md-fold-{i}"));
            div()
                .relative()
                .group(group.clone())
                .child(element)
                .child(
                    div()
                        .id(("md-fold", i))
                        .absolute()
                        .top(px(space::S))
                        .left(px(-20.))
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .when(is_open, |d| d.invisible().group_hover(group, |s| s.visible()))
                        .on_click(move |_, window, _| {
                            toggle_fold(&folds, &key);
                            window.refresh();
                        })
                        .child(super::motion::chevron(("md-fold-chevron", i), is_open, t.secondary)),
                )
                .into_any_element()
        }
        _ => element,
    };
    let accent = t.accent;
    match found.filter(|f| f.block == i) {
        Some(f) => div()
            .id(("md-found", f.token))
            .rounded(px(radius::INNER))
            .child(element)
            .with_animation(("md-found-mark", f.token), Animation::new(std::time::Duration::from_millis(1800)).with_easing(ease_in_out), move |d, delta| d.bg(accent.opacity(0.25 * (1. - delta))))
            .into_any_element(),
        None => element,
    }
}

/// The footnotes a note refers to, as they appear at its end, each with a way back.
pub fn render_footnotes(note: &ParsedNote, list: Option<ListState>, t: &Theme) -> Option<AnyElement> {
    let used: Vec<_> = note.footnote_order.iter().filter_map(|id| note.footnotes.iter().find(|(f, _)| f == id)).collect();
    if used.is_empty() {
        return None;
    }
    let (order, links_seen) = (Cell::new(u64::MAX / 2), Cell::new(0));
    let cx = Ctx { t, base: None, on_toggle: None, footnote_order: &note.footnote_order, order: &order, chain: Vec::new(), focus: None, links_seen: &links_seen };
    let rows = used.iter().enumerate().map(|(i, (id, blocks))| {
        let home = note.footnote_homes.get(id).copied().unwrap_or(0);
        let list = list.clone();
        div()
            .flex()
            .gap(px(space::S))
            .text_size(px(font::CALLOUT))
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
                        if let Some(l) = &list {
                            l.scroll_to_reveal_item(home + 1);
                        }
                    })
                    .child("↩"),
            )
    });
    Some(div().flex().flex_col().gap(px(space::XS)).child(div().h(px(1.)).mt(px(space::S)).mb(px(space::S)).bg(t.separator)).children(rows.collect::<Vec<_>>()).into_any_element())
}

#[cfg(test)]
mod tests {
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
    use super::{Block, html_text, parse};

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
                Block::Details(..) => "details",
                Block::Math(_) => "math",
                Block::Nothing => "nothing",
            })
            .collect()
    }

    #[test]
    fn display_math_reaches_the_reader_in_order() {
        assert_eq!(kinds("$$\na^2\n$$\n"), ["math"]);
        assert_eq!(kinds("Text $$a$$ mehr\n"), ["paragraph", "math", "paragraph"]);
        assert_eq!(kinds("$$a$$\n\nDanach\n"), ["math", "paragraph"]);
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
}
