use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use super::theme::{Theme, font, radius};
use crate::core::markdown::{self, Run, TokenKind};

/// Called with the 0-based source line of a clicked task box.
pub type OnToggle = Rc<dyn Fn(usize, &mut App)>;

#[derive(Clone)]
enum Inline {
    Text(String, Style),
    Image(String, String),
    FootnoteRef(String),
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    mark: bool,
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
    let mut stack: Vec<(Option<Tag>, Vec<Block>, Vec<Inline>)> = vec![(None, Vec::new(), Vec::new())];
    let mut style = Style::default();
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
                Tag::Link { .. } => style.link = true,
                Tag::Image { dest_url, .. } => image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph => {
                    let (_, _, inl) = stack.pop().unwrap();
                    stack.last_mut().unwrap().1.push(Block::Paragraph(inl));
                }
                TagEnd::Heading(level) => {
                    let (_, _, inl) = stack.pop().unwrap();
                    stack.last_mut().unwrap().1.push(Block::Heading(markdown::heading_number(level), inl));
                }
                TagEnd::BlockQuote(_) => {
                    flush_inline(&mut stack);
                    let (_, blocks, _) = stack.pop().unwrap();
                    stack.last_mut().unwrap().1.push(Block::Quote(blocks));
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
                    let (_, blocks, _) = stack.pop().unwrap();
                    let (task, line) = item_meta.pop().unwrap_or((None, 0));
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
                TagEnd::Link => style.link = false,
                _ => {}
            },
            Event::Text(t) => {
                for r in markdown::runs(&t) {
                    let target = &mut stack.last_mut().unwrap().2;
                    match r {
                        Run::Text(s) => target.push(Inline::Text(s, style)),
                        Run::Mark(s) => target.push(Inline::Text(s, Style { mark: true, ..style })),
                    }
                }
            }
            Event::Code(c) => stack.last_mut().unwrap().2.push(Inline::Text(c.to_string(), Style { code: true, ..style })),
            Event::SoftBreak => stack.last_mut().unwrap().2.push(Inline::Text(" ".into(), style)),
            Event::HardBreak => stack.last_mut().unwrap().2.push(Inline::Text("\n".into(), style)),
            Event::Rule => stack.last_mut().unwrap().1.push(Block::Rule),
            Event::Html(h) => stack.last_mut().unwrap().1.push(Block::Html(h.trim_end().to_string())),
            Event::InlineHtml(h) => stack.last_mut().unwrap().2.push(Inline::Text(h.to_string(), Style { code: true, ..style })),
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

struct Ctx<'a> {
    t: &'a Theme,
    base: Option<&'a Path>,
    on_toggle: Option<OnToggle>,
    footnote_order: &'a [String],
}

fn styled_text(inlines: &[Inline], cx: &Ctx, size: f32) -> AnyElement {
    let t = cx.t;
    let mut text = String::new();
    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    let mut images: Vec<AnyElement> = Vec::new();
    for i in inlines {
        match i {
            Inline::Text(s, st) => {
                let start = text.len();
                text.push_str(s);
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
                if st.mark {
                    h.background_color = Some(t.orange.opacity(0.3));
                }
                if st.link {
                    h.color = Some(t.accent);
                    h.underline = Some(UnderlineStyle { thickness: px(1.), color: Some(t.accent.opacity(0.4)), wavy: false });
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
        return div().flex().flex_col().gap(px(8.)).children(images).into_any_element();
    }
    let body = div().text_size(px(size)).line_height(relative(1.5)).child(StyledText::new(text).with_highlights(highlights));
    if images.is_empty() { body.into_any_element() } else { div().flex().flex_col().gap(px(8.)).child(body).children(images).into_any_element() }
}

fn image(src: &str, alt: &str, cx: &Ctx) -> AnyElement {
    if src.contains("://") {
        return img(SharedString::from(src.to_string())).max_w_full().rounded(px(radius::CONTROL)).into_any_element();
    }
    let path = markdown::percent_decode(src);
    let p: PathBuf = if path.starts_with('/') { PathBuf::from(&path) } else { cx.base.map(|b| b.join(&path)).unwrap_or_else(|| PathBuf::from(&path)) };
    if p.exists() {
        img(p).max_w_full().max_h(px(560.)).object_fit(ObjectFit::Contain).rounded(px(radius::CONTROL)).into_any_element()
    } else {
        div().text_color(cx.t.secondary).child(format!("[{alt}]")).into_any_element()
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
        .p(px(12.))
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
                    _ => font::HEADLINE,
                };
                div().pt(px(if *level <= 2 { 8. } else { 4. })).font_weight(if *level == 1 { FontWeight::BOLD } else { FontWeight::SEMIBOLD }).child(styled_text(inl, cx, size)).into_any_element()
            }
            Block::Paragraph(inl) => styled_text(inl, cx, font::BODY),
            Block::Quote(inner) => div()
                .flex()
                .gap(px(10.))
                .child(div().w(px(3.)).flex_none().rounded(px(1.5)).bg(t.accent.opacity(0.6)))
                .child(div().flex_1().flex().flex_col().gap(px(8.)).text_color(t.secondary).children(render_blocks(inner, cx)))
                .into_any_element(),
            Block::Code(lang, code) => code_block(lang.as_deref(), code, t),
            Block::Rule => div().h(px(1.)).my(px(4.)).bg(t.separator).into_any_element(),
            Block::Html(h) => div().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).text_color(t.secondary).child(h.clone()).into_any_element(),
            Block::List { start, items } => div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .children(items.iter().enumerate().map(|(i, item)| {
                    let marker: AnyElement = match (item.task, start) {
                        (Some(done), _) => {
                            let line = item.line;
                            let toggle = cx.on_toggle.clone();
                            div()
                                .id(SharedString::from(format!("task-{line}")))
                                .mt(px(3.5))
                                .ml(px(2.))
                                .size(px(14.))
                                .flex_none()
                                .rounded(px(3.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(done, |d| d.bg(t.accent))
                                .when(!done, |d| d.border_1().border_color(t.secondary))
                                .when(toggle.is_some(), |d| d.cursor_pointer())
                                .when(done, |d| d.child(super::widgets::icon("check", 10., white())))
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
                        .gap(px(8.))
                        .child(marker)
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(5.)).when(done, |d| d.text_color(t.secondary).line_through()).children(render_blocks(&item.blocks, cx)))
                })
            )
            .into_any_element(),
            Block::Table { head, rows, aligns } => {
                let cell = |inl: &Vec<Inline>, i: usize, bold: bool| {
                    let d = div().flex_1().min_w(px(80.)).px(px(8.)).py(px(4.)).when(bold, |d| d.font_weight(FontWeight::SEMIBOLD));
                    let d = match aligns.get(i) {
                        Some(pulldown_cmark::Alignment::Center) => d.flex().justify_center(),
                        Some(pulldown_cmark::Alignment::Right) => d.flex().justify_end(),
                        _ => d,
                    };
                    d.child(styled_text(inl, cx, font::BODY))
                };
                div()
                    .flex()
                    .flex_col()
                    .rounded(px(radius::CONTROL))
                    .border_1()
                    .border_color(t.separator)
                    .overflow_hidden()
                    .child(div().flex().bg(t.secondary_bg).children(head.iter().enumerate().map(|(i, c)| cell(c, i, true))))
                    .children(rows.iter().map(|r| div().flex().border_t_1().border_color(t.separator).children(r.iter().enumerate().map(|(i, c)| cell(c, i, false)))))
                    .into_any_element()
            }
        })
        .collect()
}

/// Renders a parsed note; task boxes call `on_toggle` with their source line.
pub fn render(note: &ParsedNote, base: Option<&Path>, on_toggle: Option<OnToggle>, t: &Theme) -> Div {
    let cx = Ctx { t, base, on_toggle, footnote_order: &note.footnote_order };
    let mut col = div().flex().flex_col().gap(px(12.)).children(render_blocks(&note.blocks, &cx));
    let used: Vec<_> = note.footnote_order.iter().filter_map(|id| note.footnotes.iter().find(|(f, _)| f == id)).collect();
    if !used.is_empty() {
        col = col.child(div().h(px(1.)).mt(px(8.)).bg(t.separator)).child(div().flex().flex_col().gap(px(4.)).children(used.iter().enumerate().map(|(n, (_, inl))| {
            div().flex().gap(px(6.)).text_size(px(font::CALLOUT)).line_height(relative(1.5)).child(div().flex_none().text_color(t.secondary).child(format!("{}.", n + 1))).child(styled_text(inl, &cx, font::CALLOUT))
        })));
    }
    col
}

#[cfg(test)]
mod tests {
    use super::{Block, parse};

    #[test]
    fn records_task_lines() {
        let note = parse("# T\n\n- [ ] a\n- [x] b\n\ntext");
        let Some(Block::List { items, .. }) = note.blocks.iter().find(|b| matches!(b, Block::List { .. })) else { panic!("no list") };
        assert_eq!(items.iter().map(|i| (i.task, i.line)).collect::<Vec<_>>(), vec![(Some(false), 2), (Some(true), 3)]);
    }
}
