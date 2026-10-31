use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use pulldown_cmark::{CodeBlockKind, Event, LinkType, Parser, Tag, TagEnd};

use super::selectable::{LinkAction, selectable};
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
    Image(String, String),
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
}

enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    Quote(Vec<Block>),
    Code(Option<String>, String),
    List { start: Option<u64>, items: Vec<Item> },
    Rule,
    Table { head: Vec<Vec<Inline>>, rows: Vec<Vec<Vec<Inline>>>, aligns: Vec<pulldown_cmark::Alignment> },
    Html(String),
    Callout(CalloutHead, Vec<Block>),
    Properties(Vec<frontmatter::Property>),
    Math(String),
    /// Takes its place in the count of blocks and shows nothing, like a `^id` on its own line.
    Nothing,
}

struct Item {
    task: Option<bool>,
    line: usize,
    blocks: Vec<Block>,
}

/// A parsed note, kept between frames so long notes don't re-parse while scrolling.
pub struct ParsedNote {
    blocks: Vec<Block>,
    footnotes: Vec<(String, Vec<Inline>)>,
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
    let mut item_meta: Vec<(Option<bool>, usize)> = Vec::new();
    let mut table: Option<(Vec<Vec<Inline>>, Vec<Vec<Vec<Inline>>>, Vec<pulldown_cmark::Alignment>, bool)> = None;
    let mut row: Vec<Vec<Inline>> = Vec::new();
    let mut code: Option<(Option<String>, String)> = None;
    let mut image: Option<(String, String)> = None;
    let mut footnotes: Vec<(String, Vec<Inline>)> = Vec::new();
    let mut footnote_order: Vec<String> = Vec::new();
    let flush_inline = |stack: &mut Vec<(Option<Tag>, Vec<Block>, Vec<Inline>)>| {
        let top = stack.last_mut().unwrap();
        if !top.2.is_empty() {
            let inl = std::mem::take(&mut top.2);
            top.1.push(Block::Paragraph(inl));
        }
    };
    for (event, range) in Parser::new_ext(src, markdown::options()).into_offset_iter() {
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
                    stack.last_mut().unwrap().2.push(Inline::Image(src, alt));
                }
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::MetadataBlock(_) => metadata = Some(String::new()),
                Tag::Paragraph | Tag::Heading { .. } | Tag::BlockQuote(_) | Tag::FootnoteDefinition(_) => stack.push((Some(tag), Vec::new(), Vec::new())),
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
                Tag::Table(a) => table = Some((Vec::new(), Vec::new(), a, false)),
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
                Tag::Link { dest_url, .. } => {
                    style.link = true;
                    link = Some(dest_url.to_string());
                }
                Tag::Image { dest_url, .. } => image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph => {
                    let (_, _, mut inl) = stack.pop().unwrap();
                    let block = if strip_block_id(&mut inl) && plain_text(&inl).trim().is_empty() { Block::Nothing } else { Block::Paragraph(inl) };
                    stack.last_mut().unwrap().1.push(block);
                }
                TagEnd::Heading(level) => {
                    let (_, _, inl) = stack.pop().unwrap();
                    stack.last_mut().unwrap().1.push(Block::Heading(markdown::heading_number(level), inl));
                }
                TagEnd::BlockQuote(_) => {
                    flush_inline(&mut stack);
                    let (_, blocks, _) = stack.pop().unwrap();
                    stack.last_mut().unwrap().1.push(into_callout(blocks));
                }
                TagEnd::FootnoteDefinition => {
                    flush_inline(&mut stack);
                    if let (Some(Tag::FootnoteDefinition(id)), blocks, _) = stack.pop().unwrap() {
                        let inl = blocks.into_iter().flat_map(|b| if let Block::Paragraph(i) = b { i } else { Vec::new() }).collect();
                        footnotes.push((id.to_string(), inl));
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
                for r in markdown::runs(&t) {
                    let target = &mut stack.last_mut().unwrap().2;
                    match r {
                        Run::Text(s) => target.push(match &link {
                            Some(url) => Inline::Link(s, style, url.clone()),
                            None => Inline::Text(s, style),
                        }),
                        Run::Mark(s, color) => target.push(Inline::Text(s, Style { mark: Some(color), ..style })),
                        Run::Tag(s) => target.push(Inline::Tag(s)),
                    }
                }
            }
            Event::Code(c) => stack.last_mut().unwrap().2.push(Inline::Text(c.to_string(), Style { code: true, ..style })),
            Event::SoftBreak => stack.last_mut().unwrap().2.push(Inline::Text(if strict { " " } else { "\n" }.into(), style)),
            Event::InlineMath(m) => {
                let shown = crate::core::vault::render::math_inline(m.as_ref());
                let code = shown.is_none();
                stack.last_mut().unwrap().2.push(Inline::Text(shown.unwrap_or_else(|| m.to_string()), Style { code, ..style }));
            }
            Event::DisplayMath(m) => {
                flush_inline(&mut stack);
                stack.last_mut().unwrap().1.push(Block::Math(m.to_string()));
            }
            Event::HardBreak => stack.last_mut().unwrap().2.push(Inline::Text("\n".into(), style)),
            Event::Rule => stack.last_mut().unwrap().1.push(Block::Rule),
            Event::Html(h) => {
                let text = html_text(&h);
                if !text.trim().is_empty() {
                    stack.last_mut().unwrap().1.push(Block::Html(text.trim_end().to_string()));
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
                "<mark>" => style.mark = Some(MarkColor::Default),
                "</mark>" => style.mark = None,
                "<kbd>" | "<code>" => style.code = true,
                "</kbd>" | "</code>" => style.code = false,
                _ => {}
            },
            Event::TaskListMarker(done) => {
                if let Some(m) = item_meta.last_mut() {
                    m.0 = Some(done);
                }
            }
            Event::FootnoteReference(id) => {
                if !footnote_order.iter().any(|f| f == id.as_ref()) {
                    footnote_order.push(id.to_string());
                }
                stack.last_mut().unwrap().2.push(Inline::FootnoteRef(id.to_string()));
            }
            _ => {}
        }
    }
    flush_inline(&mut stack);
    let blocks = stack.pop().map(|s| s.1).unwrap_or_default();
    ParsedNote { blocks, footnotes, footnote_order }
}

/// A note's source of `^id` at a paragraph's end disappears from what is shown.
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
    if let Block::Paragraph(inl) = &mut blocks[0] {
        let mut rest = Vec::new();
        let mut found = false;
        for item in inl.drain(..) {
            if found {
                rest.push(item);
            } else if let Inline::Text(t, st) | Inline::Link(t, st, _) = &item
                && let Some((_, after)) = t.split_once('\n')
            {
                found = true;
                if !after.is_empty() {
                    rest.push(Inline::Text(after.to_string(), *st));
                }
            }
        }
        *inl = rest;
    }
    if matches!(&blocks[0], Block::Paragraph(inl) if inl.is_empty()) {
        blocks.remove(0);
    }
    Block::Callout(head, blocks)
}

/// A list item starting with `[c] `: any character but a space means done, as in Obsidian.
fn take_task_marker(blocks: &mut [Block]) -> Option<bool> {
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
    Some(mark != ' ')
}

/// Text of an HTML block: comments and tags go, Markdown inside stays as it is.
fn html_text(html: &str) -> String {
    static COMMENT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?s)<!--.*?-->").unwrap());
    static BREAK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)<br\s*/?>|</(p|div|li|tr|h[1-6])>").unwrap());
    static TAG: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>]+>").unwrap());
    let text = COMMENT.replace_all(html, "");
    let text = BREAK.replace_all(&text, "\n");
    TAG.replace_all(&text, "").replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
}

/// Keyboard walk through a note's links.
#[derive(Clone)]
pub struct LinkFocus {
    pub index: Option<usize>,
    pub anchor: ScrollAnchor,
    /// Filled while rendering, in reading order.
    pub found: Rc<RefCell<Vec<String>>>,
    /// The note being shown, so it cannot embed itself.
    pub note: Option<PathBuf>,
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

fn styled_text(inlines: &[Inline], cx: &Ctx, size: f32) -> AnyElement {
    let t = cx.t;
    let mut text = String::new();
    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    let mut images: Vec<AnyElement> = Vec::new();
    let mut links: Vec<(std::ops::Range<usize>, LinkAction)> = Vec::new();
    let mut hovers: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut focused_here = false;
    for i in inlines {
        match i {
            Inline::Tag(tag) => {
                let start = text.len();
                text.push('#');
                text.push_str(tag);
                let name = tag.clone();
                links.push((start..text.len(), Rc::new(move |cx| super::state::act(cx, |l| l.show_tag(&name)))));
                highlights.push((start..text.len(), HighlightStyle { background_color: Some(t.ink.opacity(0.08)), color: Some(t.secondary), ..Default::default() }));
            }
            Inline::Text(s, st) | Inline::Link(s, st, _) => {
                let start = text.len();
                text.push_str(s);
                let mut unresolved = false;
                if let Inline::Link(_, _, url) = i {
                    unresolved = !is_external(url) && !link_exists(url, cx.base);
                    links.push((start..text.len(), link_action(url, cx.base)));
                    hovers.push((start..text.len(), url.clone()));
                    if let Some(focus) = cx.focus {
                        let index = cx.links_seen.get();
                        cx.links_seen.set(index + 1);
                        focus.found.borrow_mut().push(url.clone());
                        if focus.index == Some(index) {
                            focused_here = true;
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
                text.push_str(&format!("[{n}]"));
                highlights.push((start..text.len(), HighlightStyle { color: Some(t.accent), ..Default::default() }));
            }
            Inline::Image(src, alt) => images.push(image(src, alt, cx)),
        }
    }
    if text.trim().is_empty() && !images.is_empty() {
        return div().flex().flex_col().gap(px(space::S)).children(images).into_any_element();
    }
    let order = cx.order.get();
    cx.order.set(order + 1);
    let base = cx.base.map(Path::to_path_buf);
    let mut body = div().text_size(px(size)).line_height(relative(1.5)).child(selectable(("md-text", order), text, highlights, order, t.accent.opacity(0.3)).links(links).hovers(hovers, base));
    if focused_here && let Some(focus) = cx.focus {
        body = div().child(div().id(("md-link-focus", order)).anchor_scroll(Some(focus.anchor.clone())).child(body));
    }
    if images.is_empty() { body.into_any_element() } else { div().flex().flex_col().gap(px(space::S)).child(body).children(images).into_any_element() }
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

/// Parsed notes that are embedded elsewhere, by file and part; checked against the file's time.
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

fn code_block(lang: Option<&str>, code: &str, t: &Theme) -> AnyElement {
    let mut text = String::new();
    let mut highlights = Vec::new();
    for (s, kind) in markdown::highlight(code, lang) {
        let start = text.len();
        text.push_str(&s);
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
    div()
        .id(SharedString::from(format!("code-{}", text.len())))
        .w_full()
        .overflow_x_scroll()
        .p(px(space::M))
        .rounded(px(radius::CONTROL))
        .bg(t.secondary_bg)
        .border_1()
        .border_color(t.separator.opacity(0.6))
        .font_family(Theme::mono_family())
        .text_size(px(font::CALLOUT))
        .child(StyledText::new(text).with_highlights(highlights))
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
                    _ => font::BODY,
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
                None => code_block(lang.as_deref(), code, t),
            },
            Block::Rule => div().h(px(1.)).my(px(space::XS)).bg(t.separator).into_any_element(),
            Block::Html(h) => div().text_size(px(font::BODY)).line_height(relative(1.5)).child(h.clone()).into_any_element(),
            Block::Nothing => div().hidden().into_any_element(),
            Block::Math(source) => super::embeds::math(source, cx.order.get(), t),
            Block::Properties(props) => properties_row(props, cx),
            Block::Callout(head, inner) => {
                let order = cx.order.get();
                cx.order.set(order + 1);
                callout(head, render_blocks(inner, cx), order, t)
            }
            Block::List { start, items } => div()
                .flex()
                .flex_col()
                .gap(px(space::XS))
                .children(items.iter().enumerate().map(|(i, item)| {
                    let marker: AnyElement = match (item.task, start) {
                        (Some(done), _) => {
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
                                .when(!done, |d| d.border_1().border_color(t.secondary))
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
                    let done = item.task == Some(true);
                    div()
                        .flex()
                        .items_start()
                        .gap(px(space::S))
                        .child(marker)
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XS)).when(done, |d| d.text_color(t.secondary).line_through()).children(render_blocks(&item.blocks, cx)))
                })
            )
            .into_any_element(),
            Block::Table { head, rows, aligns } => {
                let cell = |inl: &Vec<Inline>, i: usize, bold: bool| {
                    let d = div().flex_1().min_w(px(80.)).px(px(space::S)).py(px(space::XS)).when(bold, |d| d.font_weight(FontWeight::SEMIBOLD));
                    let d = match aligns.get(i) {
                        Some(pulldown_cmark::Alignment::Center) => d.flex().justify_center(),
                        Some(pulldown_cmark::Alignment::Right) => d.flex().justify_end(),
                        _ => d,
                    };
                    d.child(styled_text(inl, cx, font::BODY))
                };
                let order = cx.order.get();
                cx.order.set(order + 1);
                let min_width = px(head.len() as f32 * 140.);
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
fn callout(head: &CalloutHead, content: Vec<AnyElement>, order: u64, t: &Theme) -> AnyElement {
    Callout { head: head.clone(), content, id: ElementId::from(("md-callout", order)), tint: callout_tint(head, t), secondary: t.secondary }.into_any_element()
}

#[derive(IntoElement)]
struct Callout {
    head: CalloutHead,
    content: Vec<AnyElement>,
    id: ElementId,
    tint: Hsla,
    secondary: Hsla,
}

impl RenderOnce for Callout {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let folded = self.head.folded;
        let open = window.use_keyed_state(self.id.clone(), cx, move |_, _| !folded);
        let is_open = !self.head.foldable || *open.read(cx);
        let title = self.head.title.clone().unwrap_or_else(|| tr(self.head.kind.label()));
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
                d.child(super::motion::chevron(chevron_id, is_open, self.secondary)).cursor_pointer().on_click(move |_, _, cx| {
                    open.update(cx, |o, cx| {
                        *o = !*o;
                        cx.notify();
                    })
                })
            });
        let body = div().pt(px(space::S)).flex().flex_col().gap(px(space::S)).children(self.content);
        super::widgets::banner(tint).w_full().px(px(space::M)).py(px(space::S)).flex().flex_col().child(row).when(has_body, |d| d.child(super::motion::reveal(reveal_id, is_open, body)))
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
    Properties { id: ElementId::from(("md-properties", order)), items: visible.into_iter().map(|p| (p.name.clone(), p.values.join(", "))).collect(), secondary: cx.t.secondary, tertiary: cx.t.tertiary }.into_any_element()
}

#[derive(IntoElement)]
struct Properties {
    id: ElementId,
    items: Vec<(String, String)>,
    secondary: Hsla,
    tertiary: Hsla,
}

impl RenderOnce for Properties {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let open = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let is_open = *open.read(cx);
        let summary = self.items.iter().take(4).map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(" · ");
        let chevron_id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "chevron".into());
        let reveal_id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "body".into());
        let head_id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "head".into());
        let (secondary, tertiary) = (self.secondary, self.tertiary);
        let grid = div().pt(px(space::S)).flex().flex_col().gap(px(space::XS)).children(self.items.iter().map(|(k, v)| div().flex().gap(px(space::M)).child(div().w(px(120.)).flex_none().truncate().text_color(tertiary).child(k.clone())).child(div().flex_1().min_w_0().child(v.clone()))));
        div()
            .flex()
            .flex_col()
            .text_size(px(font::CALLOUT))
            .text_color(secondary)
            .child(div().id(head_id).flex().items_center().gap(px(space::XS)).cursor_pointer().child(super::motion::chevron(chevron_id, is_open, secondary)).child(div().truncate().child(summary)).on_click(move |_, _, cx| {
                open.update(cx, |o, cx| {
                    *o = !*o;
                    cx.notify();
                })
            }))
            .child(super::motion::reveal(reveal_id, is_open, grid))
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
    pub anchor: ScrollAnchor,
    pub token: usize,
}

/// Renders a note with `anchors` on its top-level headings, one per entry of `ParsedNote::headings`.
pub fn render_with_anchors(note: &ParsedNote, base: Option<&Path>, on_toggle: Option<OnToggle>, anchors: &[ScrollAnchor], found: Option<&Found>, focus: Option<&LinkFocus>, t: &Theme) -> Div {
    build(note, base, on_toggle, anchors, found, focus, t)
}

/// Renders a parsed note; task boxes call `on_toggle` with their source line.
pub fn render(note: &ParsedNote, base: Option<&Path>, on_toggle: Option<OnToggle>, t: &Theme) -> Div {
    build(note, base, on_toggle, &[], None, None, t)
}

fn build(note: &ParsedNote, base: Option<&Path>, on_toggle: Option<OnToggle>, anchors: &[ScrollAnchor], found: Option<&Found>, focus: Option<&LinkFocus>, t: &Theme) -> Div {
    let (order, links_seen) = (Cell::new(0), Cell::new(0));
    if let Some(f) = focus {
        f.found.borrow_mut().clear();
    }
    let cx = Ctx { t, base, on_toggle, footnote_order: &note.footnote_order, order: &order, chain: focus.and_then(|f| f.note.clone()).into_iter().collect(), focus, links_seen: &links_seen };
    let mut heading = 0;
    let accent = t.accent;
    let blocks = note.blocks.iter().zip(render_blocks(&note.blocks, &cx)).enumerate().map(|(i, (block, element))| {
        let element = if matches!(block, Block::Heading(..)) {
            heading += 1;
            match anchors.get(heading - 1) {
                Some(anchor) => div().id(("md-heading", heading)).anchor_scroll(Some(anchor.clone())).child(element).into_any_element(),
                None => element,
            }
        } else {
            element
        };
        match found.filter(|f| f.block == i) {
            Some(f) => div()
                .id(("md-found", f.token))
                .anchor_scroll(Some(f.anchor.clone()))
                .rounded(px(radius::INNER))
                .child(element)
                .with_animation(("md-found-mark", f.token), Animation::new(std::time::Duration::from_millis(1800)).with_easing(ease_in_out), move |d, p| d.bg(accent.opacity(0.28 * (1. - p))))
                .into_any_element(),
            None => element,
        }
    });
    let mut col = div().flex().flex_col().gap(px(space::M)).children(blocks.collect::<Vec<_>>());
    let used: Vec<_> = note.footnote_order.iter().filter_map(|id| note.footnotes.iter().find(|(f, _)| f == id)).collect();
    if !used.is_empty() {
        col = col.child(div().h(px(1.)).mt(px(space::S)).bg(t.separator)).child(div().flex().flex_col().gap(px(space::XS)).children(used.iter().enumerate().map(|(n, (_, inl))| {
            div().flex().gap(px(space::S)).text_size(px(font::CALLOUT)).line_height(relative(1.5)).child(div().flex_none().text_color(t.secondary).child(format!("{}.", n + 1))).child(styled_text(inl, &cx, font::CALLOUT))
        })));
    }
    col
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
        assert_eq!(items.iter().map(|i| (i.task, i.line)).collect::<Vec<_>>(), vec![(Some(false), 2), (Some(true), 3)]);
    }
}
