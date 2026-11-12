use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Parser, Tag, TagEnd};
use regex::Regex;
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime};
use typst_layout::PagedDocument;
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_kit::fonts::FontStore;

use super::vault;
use super::markdown::{self, Run};

/// Default note accent, matching the Swift app.
pub const DEFAULT_ACCENT: &str = "#0EA5E9";

/// The small line above the note: module and week on the left, author and date on the right.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub left: String,
    pub right: String,
    /// The document title in the file's properties.
    pub title: String,
}

/// PDF settings from Settings → Notizen.
#[derive(Debug, Clone)]
pub struct Options {
    pub accent: String,
    pub header: bool,
    pub page_numbers: bool,
    pub lang: String,
    /// The note's properties on top of the first page.
    pub properties: bool,
}

impl Options {
    pub fn current() -> Self {
        let p = super::prefs::get();
        Self { accent: p.note_accent.clone(), header: p.pdf_header, page_numbers: p.pdf_page_numbers, properties: p.pdf_properties, lang: if super::i18n::lang() == super::i18n::Lang::En { "en".into() } else { "de".into() } }
    }
}

static FONTS: LazyLock<FontStore> = LazyLock::new(|| {
    let mut store = FontStore::new();
    store.extend(typst_kit::fonts::system());
    store.extend(typst_kit::fonts::embedded());
    store
});

static LIBRARY: LazyLock<LazyHash<Library>> = LazyLock::new(|| LazyHash::new(Library::default()));

/// Loads system fonts ahead of the first export, like the Swift app keeps WebKit warm.
pub fn prewarm() {
    LazyLock::force(&FONTS);
    LazyLock::force(&LIBRARY);
}

const THEME: &str = include_str!("pdf_theme.tmTheme");
const SANS: &str = r#"("Helvetica Neue", "SF Pro Text", "Segoe UI", "Helvetica", "Arial", "Noto Sans", "Cantarell", "DejaVu Sans", "Libertinus Serif")"#;
const MONO: &str = r#"("SF Mono", "Menlo", "Consolas", "Cascadia Mono", "Noto Sans Mono", "DejaVu Sans Mono")"#;

fn file_id(path: &str) -> FileId {
    RootedPath::new(VirtualRoot::Project, VirtualPath::new(path).expect("valid path")).intern()
}

/// What LaTeX commands mean in Typst, as a file formulas can import; kept parsed, so a formula takes no longer than it must.
static MITEX: LazyLock<Source> = LazyLock::new(|| Source::new(file_id("/mitex.typ"), include_str!("../../assets/typst/mitex.typ").to_string()));

struct NoteWorld {
    main: Source,
    files: HashMap<FileId, Bytes>,
}

impl World for NoteWorld {
    fn library(&self) -> &LazyHash<Library> {
        &LIBRARY
    }

    fn book(&self) -> &LazyHash<FontBook> {
        FONTS.book()
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else if id == MITEX.id() {
            Ok(MITEX.clone())
        } else {
            Err(FileError::NotFound(PathBuf::from(id.vpath().get_with_slash())))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.get(&id).cloned().ok_or_else(|| FileError::NotFound(PathBuf::from(id.vpath().get_with_slash())))
    }

    fn font(&self, index: usize) -> Option<Font> {
        FONTS.font(index)
    }

    fn today(&self, _: Option<typst::foundations::Duration>) -> Option<Datetime> {
        let d = chrono::Local::now().date_naive();
        use chrono::Datelike;
        Datetime::from_ymd(d.year(), d.month() as u8, d.day() as u8)
    }
}

/// Escapes text as a Typst string literal usable in markup (`#"…"`).
fn lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    out.push_str("#\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn str_arg(s: &str) -> String {
    lit(s)[1..].to_string()
}

fn valid_hex(s: &str) -> bool {
    static HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$").unwrap());
    HEX.is_match(s.trim())
}

/// Builds the Typst document for a note.
pub struct Converter<'a> {
    base: &'a Path,
    images: Vec<PathBuf>,
    stack: Vec<String>,
    items: Vec<(Option<bool>, String)>,
    lists: Vec<(Option<u64>, Vec<(Option<bool>, String)>)>,
    table: Option<(Vec<Alignment>, Vec<String>, usize)>,
    footnotes: Vec<(String, String)>,
    footnote_order: Vec<String>,
    code: Option<(Option<String>, String)>,
    image: Option<(String, String)>,
    /// What stands in the note as a placeholder until it is drawn.
    embeds: Vec<Embed>,
    /// Inline HTML tags still open, closed at the latest where their block ends.
    open_html: Vec<String>,
    /// The head of a callout was just closed; the line break after it does not count.
    after_head: bool,
    /// Pictures and PDFs are named instead of drawn, because one of them could not be read.
    no_media: bool,
    /// Where the last paragraph that is only a picture starts, so an italic caption right after it can be kept with it.
    figure: Option<usize>,
    para_start: usize,
}

/// Something shown in a note that is not text.
enum Embed {
    Pdf { path: PathBuf, page: usize },
    Canvas(PathBuf),
    Base { source: String, this: Option<PathBuf> },
    File { name: String, icon: &'static str },
}

const CALLOUT_OPEN: char = '\u{E000}';
const CALLOUT_KIND_END: char = '\u{E001}';
const CALLOUT_CLOSE: char = '\u{E002}';
const EMBED_OPEN: char = '\u{E003}';
const EMBED_CLOSE: char = '\u{E004}';
const TASK_MARK: char = '\u{E005}';
const PICTURE_OPEN: char = '\u{E006}';
const PICTURE_CLOSE: char = '\u{E007}';
const RASTER: [&str; 6] = ["png", "jpg", "jpeg", "gif", "svg", "webp"];
const AUDIO: [&str; 7] = ["mp3", "wav", "m4a", "ogg", "flac", "3gp", "webm"];
const VIDEO: [&str; 4] = ["mp4", "mov", "mkv", "ogv"];

impl<'a> Converter<'a> {
    pub fn new(base: &'a Path) -> Self {
        Self { base, images: Vec::new(), stack: vec![String::new()], items: Vec::new(), lists: Vec::new(), table: None, footnotes: Vec::new(), footnote_order: Vec::new(), code: None, image: None, embeds: Vec::new(), open_html: Vec::new(), after_head: false, no_media: false, figure: None, para_start: 0 }
    }

    fn without_media(mut self) -> Self {
        self.no_media = true;
        self
    }

    fn out(&mut self) -> &mut String {
        self.stack.last_mut().unwrap()
    }

    fn push(&mut self) {
        self.stack.push(String::new());
    }

    fn pop(&mut self) -> String {
        self.stack.pop().unwrap_or_default()
    }

    fn file_named(&mut self, p: &Path) -> String {
        let name = format!("/img/{}.{}", self.images.len(), p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default());
        self.images.push(p.to_path_buf());
        name
    }

    fn image_markup(&mut self, src: &str, alt: &str) -> String {
        let local = if src.contains("://") {
            None
        } else {
            let decoded = markdown::percent_decode(src);
            let direct = if Path::new(&decoded).is_absolute() { PathBuf::from(&decoded) } else { self.base.join(&decoded) };
            let name = decoded.split(['|', '#']).next().unwrap_or(&decoded);
            let p = if direct.is_file() { direct } else { vault::index::current().resolve(name, Some(&self.base.join("x.md"))).unwrap_or(direct) };
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            (p.is_file() && RASTER.contains(&ext.as_str())).then_some(p)
        };
        let (width, height) = image_size(alt);
        let sized = width.is_some() || height.is_some();
        let caption = if sized { alt.rsplit_once('|').map_or("", |(text, _)| text) } else { alt };
        match local {
            Some(p) if !self.no_media => {
                let name = self.file_named(&p);
                let pt = |v: f64| v * 0.75;
                match picture_in_line(width, height) {
                    Some((w, h)) => format!("#inlineimg({}, {:.1}pt, {:.1}pt)", str_arg(&name), pt(w), pt(h)),
                    None => {
                        let mut args = str_arg(&name);
                        if let Some(w) = width {
                            args.push_str(&format!(", width: {:.1}pt", pt(w)));
                        }
                        if let Some(h) = height {
                            args.push_str(&format!(", height: {:.1}pt", pt(h)));
                        }
                        format!("{PICTURE_OPEN}#bigimg({args}){PICTURE_CLOSE}")
                    }
                }
            }
            Some(p) => format!("#filechip(\"image\")[{}]", lit(&p.file_name().unwrap_or_default().to_string_lossy())),
            None if caption.is_empty() => String::new(),
            None => format!("#text(fill: muted)[{}]", lit(caption)),
        }
    }

    /// Writes an HTML fragment: tags that mean something as markup, the rest only as its text.
    fn html(&mut self, h: &str) {
        static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->|<[^>]+>").unwrap());
        let mut last = 0;
        for m in TAG.find_iter(h) {
            let text = &h[last..m.start()];
            if !text.is_empty() {
                let t = lit(text);
                self.out().push_str(&t);
            }
            last = m.end();
            let tag = m.as_str().trim_matches(['<', '>', ' ', '/']).split_whitespace().next().unwrap_or_default().to_lowercase();
            if tag == "br" {
                self.out().push_str("#linebreak()");
            } else if tag == "img" {
                let attribute = |name: &str| html_attribute(m.as_str(), name);
                let size = match (attribute("width"), attribute("height")) {
                    (Some(w), Some(h)) => format!("{w}x{h}"),
                    (Some(w), None) => w,
                    _ => String::new(),
                };
                let markup = self.image_markup(&attribute("src").unwrap_or_default(), &size);
                self.out().push_str(&markup);
            } else if m.as_str().starts_with("</") {
                if let Some(at) = self.open_html.iter().rposition(|t| *t == tag) {
                    let closes = self.open_html.len() - at;
                    self.open_html.truncate(at);
                    self.out().push_str(&"]".repeat(closes));
                }
            } else if let Some(open) = html_markup(&tag).filter(|_| !m.as_str().ends_with("/>")) {
                self.open_html.push(tag);
                self.out().push_str(open);
            }
        }
        if last < h.len() {
            let t = lit(&h[last..]);
            self.out().push_str(&t);
        }
    }

    fn keep_caption(&mut self) {
        let start = self.para_start;
        let pending = self.figure.take();
        let para = self.out().get(start..).unwrap_or_default().trim().to_string();
        if let Some(figure) = pending.filter(|_| is_caption(&para)) {
            let tail = self.out().split_off(figure);
            let wrapped = format!("#keepfig[{}]\n\n", tail.trim_end());
            self.out().push_str(&wrapped);
        } else if is_picture_row(&para) {
            self.figure = Some(start);
        }
    }

    fn close_html(&mut self) {
        let closes = std::mem::take(&mut self.open_html).len();
        self.out().push_str(&"]".repeat(closes));
    }

    fn embed_markup(&mut self, at: usize) -> String {
        let Some(embed) = self.embeds.get(at) else { return String::new() };
        match embed {
            Embed::File { name, icon } => format!("#filechip({})[{}]", str_arg(icon), lit(name)),
            Embed::Pdf { path, page } => {
                let (path, page) = (path.clone(), *page);
                let title = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if self.no_media {
                    return format!("#filechip(\"file-text\")[{}]", lit(&title));
                }
                let pages = lopdf::Document::load(&path).ok().map(|d| d.get_pages().len());
                let Some(pages) = pages.filter(|n| *n > 0) else { return format!("#filechip(\"file-text\")[{}]", lit(&title)) };
                let page = page.clamp(1, pages);
                let name = self.file_named(&path);
                format!("#pdfpage({}, {page}, {})", str_arg(&name), str_arg(&format!("{title} · {page} / {pages}")))
            }
            Embed::Canvas(path) => {
                let path = path.clone();
                canvas_markup(&path)
            }
            Embed::Base { source, this } => base_markup(source, this.as_deref()),
        }
    }

    fn end_list(&mut self) {
        let (start, items) = self.lists.pop().unwrap_or_default();
        let depth = self.lists.len();
        let bullet = ["•", "◦", "▪"][depth.min(2)];
        let mut cells = Vec::new();
        for (i, (task, body)) in items.into_iter().enumerate() {
            let custom = body.strip_prefix(TASK_MARK).and_then(|rest| rest.split_once(TASK_MARK)).map(|(mark, rest)| (mark.to_string(), rest.to_string()));
            let (custom_mark, body) = match custom {
                Some((mark, rest)) => (Some(mark), rest),
                None => (None, body),
            };
            let marker = match (task, start) {
                _ if custom_mark.is_some() => format!("customtask({})", str_arg(custom_mark.as_deref().unwrap_or_default())),
                (Some(done), _) => format!("taskbox({done})"),
                (None, Some(s)) => format!("align(right, text(fill: marker, number-type: \"lining\", number-width: \"tabular\")[{}])", lit(&format!("{}.", s + i as u64))),
                (None, None) => format!("align(right, text(fill: marker)[{}])", lit(bullet)),
            };
            let body = if task == Some(true) { format!("text(fill: muted)[{body}]") } else { format!("[{body}]") };
            cells.push(format!("{marker}, {body}"));
        }
        let list = format!("#notelist({})\n", cells.join(", "));
        self.out().push_str(&list);
    }

    fn end_table(&mut self) {
        let Some((aligns, cells, columns)) = self.table.take() else { return };
        let columns = columns.max(1);
        let align: Vec<&str> = (0..columns)
            .map(|i| match aligns.get(i) {
                Some(Alignment::Center) => "center",
                Some(Alignment::Right) => "right",
                _ => "left",
            })
            .collect();
        let rows = cells.len().div_ceil(columns);
        let body = cells.into_iter().map(|c| format!("[{c}]")).collect::<Vec<_>>().join(", ");
        let t = format!("#notetable(columns: {columns}, rows: {rows}, align: ({},), {body})\n", align.join(", "));
        self.out().push_str(&t);
    }

    /// Converts Markdown to Typst markup and returns the images referenced as `/img/N.ext`.
    pub fn convert(mut self, src: &str) -> (String, Vec<PathBuf>) {
        let body = &src[vault::frontmatter::body_offset(src)..];
        let expanded = expand_embeds(&vault::syntax::prepare(body), self.base, 0, &mut self.embeds);
        let src = callouts_as_quotes(&gaps_shown(&without_block_ids(&expanded)));
        let strict = vault::index::current().config.strict_line_breaks;
        let events: Vec<Event> = Parser::new_ext(&src, markdown::options()).collect();
        for (i, event) in events.iter().enumerate() {
            if let Some((_, buf)) = self.code.as_mut() {
                match event {
                    Event::Text(t) => buf.push_str(t),
                    Event::End(TagEnd::CodeBlock) => {
                        let (lang, buf) = self.code.take().unwrap();
                        let code = buf.strip_suffix('\n').unwrap_or(&buf).to_string();
                        if lang.as_deref() == Some("query") {
                            let m = query_markup(&code);
                            self.out().push_str(&m);
                            continue;
                        }
                        if lang.as_deref() == Some("base") {
                            let m = base_markup(&code, Some(&self.base.join("x.md")));
                            self.out().push_str(&m);
                            continue;
                        }
                        if lang.as_deref() == Some("mermaid")
                            && let Ok(svg) = vault::render::mermaid_svg(&code, false)
                        {
                            let file = std::env::temp_dir().join(format!("schrumbi-mermaid-{}-{}.svg", std::process::id(), self.images.len()));
                            if std::fs::write(&file, svg).is_ok() {
                                let name = format!("/img/{}.svg", self.images.len());
                                self.images.push(file);
                                self.out().push_str(&format!("#fitimg({})\n\n", str_arg(&name)));
                                continue;
                            }
                        }
                        let lang = lang.map(|l| format!("lang: {}, ", str_arg(&l))).unwrap_or_default();
                        let block = format!("#raw(block: true, {lang}{})\n\n", str_arg(&code));
                        self.out().push_str(&block);
                    }
                    _ => {}
                }
                continue;
            }
            if let Some((_, alt)) = self.image.as_mut() {
                match event {
                    Event::Text(t) | Event::Code(t) => alt.push_str(t),
                    Event::End(TagEnd::Image) => {
                        let (src, alt) = self.image.take().unwrap();
                        let m = self.image_markup(&src, &alt);
                        self.out().push_str(&m);
                    }
                    _ => {}
                }
                continue;
            }
            if self.figure.is_some() && starts_or_ends_a_block(event) {
                self.figure = None;
            }
            match event {
                Event::Start(tag) => match tag {
                    Tag::Paragraph => self.para_start = self.out().len(),
                    Tag::Heading { level, .. } => {
                        let n = markdown::heading_number(*level);
                        self.out().push_str(&format!("#heading(level: {n})["));
                    }
                    Tag::BlockQuote(_) => {
                        let head = events.get(i + 2).and_then(|e| if let Event::Text(t) = e { t.strip_prefix(CALLOUT_OPEN) } else { None }).and_then(|t| t.split(CALLOUT_KIND_END).next().map(str::to_string));
                        match head {
                            Some(kind) => self.out().push_str(&format!("#callout({})[", str_arg(&kind))),
                            None => self.out().push_str("#quote(block: true)["),
                        }
                    }
                    Tag::CodeBlock(kind) => {
                        let lang = match kind {
                            CodeBlockKind::Fenced(l) if !l.is_empty() => l.split_whitespace().next().map(String::from),
                            _ => None,
                        };
                        self.code = Some((lang, String::new()));
                    }
                    Tag::List(start) => self.lists.push((*start, Vec::new())),
                    Tag::Item => {
                        self.items.push((None, String::new()));
                        self.push();
                    }
                    Tag::FootnoteDefinition(_) => self.push(),
                    Tag::Table(aligns) => self.table = Some((aligns.clone(), Vec::new(), aligns.len())),
                    Tag::TableCell => self.push(),
                    Tag::Emphasis => self.out().push_str("#emph["),
                    Tag::Strong => self.out().push_str("#strong["),
                    Tag::Strikethrough => self.out().push_str("#strike["),
                    Tag::Link { dest_url, .. } => {
                        let s = if dest_url.contains("://") { format!("#link({})[", str_arg(dest_url)) } else { "#text(fill: accent)[".to_string() };
                        self.out().push_str(&s);
                    }
                    Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
                    _ => {}
                },
                Event::End(end) => match end {
                    TagEnd::Paragraph if std::mem::take(&mut self.after_head) => self.close_html(),
                    TagEnd::Paragraph => {
                        self.close_html();
                        let inside = !self.items.is_empty() || self.stack.len() > 1;
                        self.out().push_str(if inside { "#parbreak()" } else { "\n\n" });
                        if !inside {
                            self.keep_caption();
                        }
                    }
                    TagEnd::Heading(_) => {
                        self.close_html();
                        self.out().push_str("]\n\n");
                    }
                    TagEnd::BlockQuote(_) => self.out().push_str("]\n\n"),
                    TagEnd::Item => {
                        self.close_html();
                        let body = self.pop();
                        let (task, _) = self.items.pop().unwrap_or_default();
                        let body = body.trim_end_matches("#parbreak()").to_string();
                        if let Some(l) = self.lists.last_mut() {
                            l.1.push((task, body));
                        }
                    }
                    TagEnd::List(_) => self.end_list(),
                    TagEnd::FootnoteDefinition => {
                        let body = self.pop();
                        let id = events[..i].iter().rev().find_map(|e| if let Event::Start(Tag::FootnoteDefinition(id)) = e { Some(id.to_string()) } else { None }).unwrap_or_default();
                        self.footnotes.push((id, body.trim_end_matches("#parbreak()").to_string()));
                    }
                    TagEnd::TableCell => {
                        self.close_html();
                        let c = self.pop();
                        if let Some(t) = self.table.as_mut() {
                            t.1.push(c);
                        }
                    }
                    TagEnd::Table => self.end_table(),
                    TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => self.out().push(']'),
                    _ => {}
                },
                Event::Text(t) => {
                    let mut rest: &str = t;
                    if let Some(mark) = rest.strip_prefix(TASK_MARK).and_then(|r| r.split_once(TASK_MARK)).filter(|_| self.stack.last().is_some_and(String::is_empty)) {
                        let opened = format!("{TASK_MARK}{}{TASK_MARK}", mark.0);
                        self.out().push_str(&opened);
                        rest = mark.1;
                    }
                    if let Some(after) = rest.strip_prefix(CALLOUT_OPEN) {
                        let (kind, title) = after.split_once(CALLOUT_KIND_END).unwrap_or(("note", after));
                        self.out().push_str(&format!("#calloutitle({})[", str_arg(kind)));
                        rest = title;
                    }
                    if let Some(number) = rest.strip_prefix(EMBED_OPEN).and_then(|r| r.strip_suffix(EMBED_CLOSE)) {
                        let m = self.embed_markup(number.parse().unwrap_or(usize::MAX));
                        self.out().push_str(&m);
                        continue;
                    }
                    let (body, closed) = match rest.strip_suffix(CALLOUT_CLOSE) {
                        Some(body) => (body, true),
                        None => (rest, false),
                    };
                    let mut s = String::new();
                    for r in markdown::runs(body) {
                        match r {
                            Run::Text(x) => s.push_str(&lit(&x)),
                            Run::Mark(x, color) => s.push_str(&format!("#highlight(fill: {})[{}]", mark_fill(color), lit(&x))),
                            Run::Tag(x) => s.push_str(&format!("#notetag[{}]", lit(&format!("#{x}")))),
                        }
                    }
                    if closed {
                        s.push(']');
                        self.after_head = true;
                    }
                    self.out().push_str(&s);
                }
                Event::Code(c) => {
                    let s = format!("#raw({})", str_arg(c));
                    self.out().push_str(&s);
                }
                Event::InlineMath(m) => {
                    let s = vault::render::math_markup(m, false).unwrap_or_else(|_| format!("#raw({})", str_arg(m)));
                    self.out().push_str(&s);
                }
                Event::DisplayMath(m) => {
                    let s = vault::render::math_markup(m, true).unwrap_or_else(|_| format!("#raw({})", str_arg(m)));
                    self.out().push_str(&s);
                }
                Event::SoftBreak if std::mem::take(&mut self.after_head) => {}
                Event::SoftBreak if !strict => self.out().push_str("#linebreak()"),
                Event::SoftBreak => self.out().push(' '),
                Event::HardBreak => self.out().push_str("#linebreak()"),
                Event::Rule => self.out().push_str("#noterule\n\n"),
                Event::Html(h) => {
                    self.html(h);
                    self.close_html();
                }
                Event::InlineHtml(h) => self.html(h),
                Event::TaskListMarker(done) => {
                    if let Some(it) = self.items.last_mut() {
                        it.0 = Some(*done);
                    }
                }
                Event::FootnoteReference(id) => {
                    let n = match self.footnote_order.iter().position(|f| f == id.as_ref()) {
                        Some(p) => p + 1,
                        None => {
                            self.footnote_order.push(id.to_string());
                            self.footnote_order.len()
                        }
                    };
                    self.out().push_str(&format!("#fnref({n})"));
                }
            }
        }
        let mut out = self.pop();
        let notes: Vec<String> = self.footnote_order.iter().filter_map(|id| self.footnotes.iter().find(|(f, _)| f == id)).enumerate().map(|(n, (_, body))| format!("[{}.], [{body}]", n + 1)).collect();
        if !notes.is_empty() {
            out.push_str(&format!("#footnotes({})\n", notes.join(", ")));
        }
        (picture_rows(&out), self.images)
    }
}

fn starts_or_ends_a_block(event: &Event) -> bool {
    match event {
        Event::Start(tag) => !matches!(tag, Tag::Paragraph | Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }),
        Event::End(tag) => !matches!(tag, TagEnd::Paragraph | TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image),
        Event::Rule | Event::Html(_) => true,
        _ => false,
    }
}

/// The width and height written after the last `|` of an image, in pixels as the reading view takes them.
fn image_size(alt: &str) -> (Option<f64>, Option<f64>) {
    let word = alt.rsplit('|').next().unwrap_or(alt).trim();
    let size = |v: &str| v.parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.);
    match word.split_once('x') {
        Some((w, h)) => (size(w), size(h)),
        None => (size(word), None),
    }
}

/// A picture with a size small enough to stand in a line of text, as the reading view decides it.
fn picture_in_line(width: Option<f64>, height: Option<f64>) -> Option<(f64, f64)> {
    const LIMIT: f64 = 30.;
    match (width, height) {
        (Some(w), Some(h)) if h <= LIMIT => Some((w, h)),
        (None, Some(h)) if h <= LIMIT => Some((h, h)),
        (Some(w), None) if w <= LIMIT => Some((w, w)),
        _ => None,
    }
}

fn html_attribute(tag: &str, name: &str) -> Option<String> {
    static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\b([a-z-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap());
    ATTRIBUTE.captures_iter(tag).find(|c| c[1].eq_ignore_ascii_case(name)).and_then(|c| c.get(2).or_else(|| c.get(3)).or_else(|| c.get(4)).map(|m| m.as_str().trim().trim_end_matches('/').to_string()))
}

fn is_picture_row(paragraph: &str) -> bool {
    static ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"^(?:{PICTURE_OPEN}[^{PICTURE_CLOSE}]*{PICTURE_CLOSE}(?:[ \t]|#linebreak\(\)|#"[ \t]*")*)+$"#)).unwrap());
    ROW.is_match(paragraph)
}

/// A short line right after a picture that reads as its caption: in italics, or starting like one.
fn is_caption(paragraph: &str) -> bool {
    static NAMED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^#"(?:Abbildung|Abb\.|Bild|Grafik|Diagramm|Figure|Fig\.|Quelle|Source)\b"#).unwrap());
    paragraph.len() <= 300 && !paragraph.contains('\n') && (paragraph.starts_with("#emph[") || NAMED.is_match(paragraph))
}

/// Pictures that follow each other with nothing but space between them flow side by side and wrap, as in the reading view.
fn picture_rows(text: &str) -> String {
    static RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"(?s){PICTURE_OPEN}.*?{PICTURE_CLOSE}(?:(?:[ \t]|#linebreak\(\)|#"[ \t]*")*{PICTURE_OPEN}.*?{PICTURE_CLOSE})*"#)).unwrap());
    static ONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?s){PICTURE_OPEN}(.*?){PICTURE_CLOSE}")).unwrap());
    if !text.contains(PICTURE_OPEN) {
        return text.to_string();
    }
    RUN.replace_all(text, |run: &regex::Captures| format!("#bigrow[{}]", ONE.captures_iter(&run[0]).map(|c| c[1].to_string()).collect::<Vec<_>>().join("#h(4pt)"))).into_owned()
}

fn preamble(o: &Options, meta: Option<&Meta>) -> String {
    let accent = if valid_hex(&o.accent) { o.accent.trim().to_string() } else { DEFAULT_ACCENT.to_string() };
    let lang = if o.lang.starts_with("en") { "en" } else { "de" };
    let footer = if o.page_numbers {
        r##"context { let total = counter(page).final().first(); if total > 1 { align(right, text(size: 8pt, weight: 500, fill: rgb("#646d76"), counter(page).display("1 / 1", both: true))) } }"##
    } else {
        "none"
    };
    let title = match meta.map(|m| m.title.trim()).filter(|t| !t.is_empty()) {
        Some(t) => str_arg(t),
        None => "none".to_string(),
    };
    let header = match meta.filter(|_| o.header) {
        Some(m) if !(m.left.is_empty() && m.right.is_empty()) => format!(
            "#block(width: 100%, below: 9mm, stroke: (bottom: 0.75pt + rule), inset: (bottom: 2.5mm), text(size: 8.5pt, fill: muted, tracking: 0.01em, grid(columns: (1fr, auto), column-gutter: 12pt, {}, {})))\n\n",
            str_arg(&m.left),
            str_arg(&m.right)
        ),
        _ => String::new(),
    };
    format!(
        r##"#let accent = rgb("{accent}")
#let ink = rgb("#1d2125")
#let muted = rgb("#646d76")
#let rule = rgb("#e2e5e9")
#let soft = rgb("#f5f7f9")
#let headc = rgb("#111417")
#let marker = rgb("#8a939c")
#let code-fill = soft
#let pre-fill = soft
#let texth = 730pt
#let fitblock(props, body, above: auto, below: auto, limit: 0.4) = block(above: above, below: below, layout(size => {{
  let probe = block(width: 100%, ..props, body)
  block(breakable: measure(width: size.width, probe).height > limit * texth, width: 100%, ..props, body)
}}))
#set document(title: {title})
#set page(paper: "a4", margin: 56pt, footer: {footer}, footer-descent: 18pt)
#set text(font: {SANS}, size: 10.5pt, fill: ink, lang: "{lang}", hyphenate: true)
#set par(leading: 0.9em, spacing: 3mm)
#set strong(delta: 250)
#set highlight(fill: accent.transparentize(78%), extent: 1pt, radius: 2pt)
#set raw(theme: "/theme.tmTheme")
#show link: set text(fill: accent)
#show raw: set text(font: {MONO}, size: 9pt)
#show raw.where(block: false): it => box(fill: code-fill, inset: (x: 3pt), outset: (y: 2pt), radius: 3pt, it)
#show raw.where(block: true): it => fitblock((fill: pre-fill, stroke: 0.75pt + rule, radius: 5pt, inset: (x: 4mm, y: 3mm)), it, above: 3mm, below: 4mm, limit: 0.5)
#show heading: set text(fill: headc, weight: 650)
#show heading: set par(leading: 0.5em)
#show heading.where(level: 1): set text(size: 21pt, tracking: -0.015em)
#show heading.where(level: 2): set text(size: 14pt)
#show heading.where(level: 3): set text(size: 11.5pt, weight: 600)
#show heading.where(level: 4): set text(size: 8.5pt, weight: 600, tracking: 0.06em, fill: muted)
#show heading.where(level: 5): set text(size: 8.5pt, weight: 600, tracking: 0.06em, fill: muted)
#show heading.where(level: 6): set text(size: 8.5pt, weight: 600, tracking: 0.06em, fill: muted)
#show heading: it => {{
  let l = it.level
  if l == 1 {{ block(above: 0mm, below: 7mm, sticky: true, {{ block(below: 3.5mm, it.body); box(width: 34pt, height: 3pt, radius: 2pt, fill: accent) }}) }}
  else if l == 2 {{ block(width: 100%, above: 8mm, below: 3mm, sticky: true, stroke: (bottom: 0.75pt + rule), inset: (bottom: 1.5mm), it.body) }}
  else if l == 3 {{ block(above: 6mm, below: 2mm, sticky: true, it.body) }}
  else {{ block(above: 5mm, below: 1.5mm, sticky: true, upper(it.body)) }}
}}
#show quote.where(block: true): it => fitblock((stroke: (left: 1.5pt + accent), inset: (left: 4mm, y: 1mm)), {{ set text(fill: muted); set par(spacing: 1.5mm); it.body }}, above: 4mm, below: 4mm)
#let noterule = block(above: 7mm, below: 7mm, line(length: 100%, stroke: 0.75pt + rule))
#let taskbox(done) = box(width: 9pt, height: 9pt, baseline: 1pt, radius: 2.5pt, stroke: if done {{ 0.9pt + accent }} else {{ 0.9pt + rgb("#a3abb4") }}, fill: if done {{ accent }} else {{ none }}, if done {{ place(dx: 1.6pt, dy: 1.4pt, curve(stroke: (paint: white, thickness: 1.3pt, cap: "round", join: "round"), curve.move((0.2pt, 3.2pt)), curve.line((2.1pt, 5.1pt)), curve.line((5.8pt, 1.2pt)))) }})
#let notelist(..cells) = block(above: 3mm, below: 3mm, {{
  set block(spacing: 0.9em + 1.2mm)
  let c = cells.pos()
  for i in range(0, c.len(), step: 2) {{
    fitblock((:), grid(columns: (5.5mm, 1fr), column-gutter: 0pt, align: (right + top, left + top), box(inset: (right: 2.2mm), c.at(i)), c.at(i + 1)))
  }}
}})
#let notetable(columns: 1, rows: 1, align: (left,), ..cells) = fitblock((:), {{ set text(size: 9.5pt); table(columns: (auto,) * columns, align: align, inset: (x: 2.5mm, y: 2mm), stroke: (x, y) => if y == 0 {{ (bottom: 1.1pt + headc) }} else if y < rows - 1 {{ (bottom: 0.75pt + rule) }} else {{ none }}, table.header(..cells.pos().slice(0, columns).map(c => strong(c))), ..cells.pos().slice(columns)) }}, above: 2mm, below: 5mm, limit: 0.5)
#let keepfig(body) = block(breakable: false, body)
#let fnref(n) = super(size: 7pt, text(fill: accent, str(n)))
#let footnotes(..cells) = block(width: 100%, above: 8mm, stroke: (top: 0.75pt + rule), inset: (top: 3mm), {{ set text(size: 8.5pt, fill: muted); grid(columns: (5mm, 1fr), row-gutter: 1.2em, ..cells) }})
#let picmax = 420pt
#let picsize(path, width, height, room) = {{
  let m = measure(image(path))
  let (w, h) = if width != none and height != none {{ (width, height) }} else if width != none {{ (width, m.height * (width / m.width)) }} else if height != none {{ (m.width * (height / m.height), height) }} else {{ (m.width, m.height) }}
  let s = calc.min(1, room / w, picmax / h)
  (w * s, h * s)
}}
#let bigimg(path, width: none, height: none) = box(layout(size => {{ let (w, h) = picsize(path, width, height, size.width); image(path, width: w, height: h, fit: "contain") }}))
#let bigrow(body) = block(above: 3mm, below: 3mm, {{ set par(leading: 4pt, spacing: 4pt); body }})
#let inlineimg(path, width, height) = box(baseline: 0.2em, layout(size => {{ let w = calc.min(width, size.width); image(path, width: w, height: height * (w / width), fit: "contain") }}))
#let fitimg(path) = layout(size => {{ let img = image(path); let m = measure(img); let s = calc.min(1, size.width / m.width, 0.95 * texth / m.height); if s < 1 {{ image(path, width: m.width * s) }} else {{ img }} }})
#let calloutcolors = (note: rgb("#3b7dd8"), info: rgb("#3b7dd8"), todo: rgb("#3b7dd8"), abstract: rgb("#1e9bb0"), tip: rgb("#1e9bb0"), success: rgb("#2f9e5b"), question: rgb("#d9822b"), warning: rgb("#d9822b"), failure: rgb("#d64545"), danger: rgb("#d64545"), bug: rgb("#d64545"), example: rgb("#7c5cd6"), quote: rgb("#8a939c"), embed: rule)
#let callout(kind, body) = {{
  let c = calloutcolors.at(kind, default: accent)
  let frame = if kind == "embed" {{ (stroke: 0.75pt + rule, fill: none) }} else {{ (stroke: (left: 2.5pt + c), fill: c.transparentize(91%)) }}
  fitblock((inset: (x: 3.5mm, y: 3mm), radius: 3pt, ..frame), {{ set par(spacing: 2mm); body }}, above: 4mm, below: 4mm)
}}
#let calloutitle(kind, title) = {{
  if kind == "embed" {{ block(below: 2.5mm, sticky: true, text(size: 8.5pt, fill: muted, title)) }}
  else {{ let c = calloutcolors.at(kind, default: accent); block(below: 2mm, sticky: true, text(weight: 650, fill: c.darken(12%), title)) }}
}}
#let notetag(body) = box(fill: accent.transparentize(88%), inset: (x: 3pt), outset: (y: 1.5pt), radius: 4pt, text(fill: accent.darken(15%), size: 0.92em, body))
#let kbd(body) = box(fill: code-fill, stroke: 0.6pt + rule, inset: (x: 3pt), outset: (y: 2pt), radius: 3pt, text(font: {MONO}, size: 9pt, body))
#let filechip(kind, name) = block(above: 2mm, below: 2mm, inset: (x: 3mm, y: 2mm), radius: 3pt, stroke: 0.75pt + rule, text(size: 9.5pt, fill: muted, name))
#let pdfpage(path, page, title) = block(width: 72%, above: 3mm, below: 4mm, breakable: false, {{ block(width: 100%, stroke: 0.75pt + rule, image(path, page: page, width: 100%)); v(1.5mm, weak: true); text(size: 8.5pt, fill: muted, title) }})
#let customtask(mark) = box(width: 9pt, height: 9pt, baseline: 1pt, radius: 2.5pt, stroke: 0.9pt + accent, align(center + horizon, text(size: 6.5pt, weight: 700, fill: accent, mark)))
{header}"##,
    )
}

/// Builds the Typst source for a note, for tests and diagnostics.
pub fn source(markdown_text: &str, base: &Path, meta: Option<&Meta>, o: &Options) -> (String, Vec<PathBuf>) {
    build(markdown_text, base, meta, o, false)
}

fn build(markdown_text: &str, base: &Path, meta: Option<&Meta>, o: &Options, no_media: bool) -> (String, Vec<PathBuf>) {
    let converter = Converter::new(base);
    let converter = if no_media { converter.without_media() } else { converter };
    let (body, images) = converter.convert(markdown_text);
    let properties = if o.properties { properties_markup(markdown_text) } else { String::new() };
    (format!("{}{properties}{body}", preamble(o, meta)), images)
}

/// The note's properties as a small table on top.
fn properties_markup(text: &str) -> String {
    let Some(properties) = vault::frontmatter::properties(text).filter(|p| !p.is_empty()) else { return String::new() };
    let cells: Vec<String> = properties.iter().map(|p| format!("text(fill: muted)[{}], [{}]", lit(&p.name), lit(&p.values.join(", ")))).collect();
    format!("#block(width: 100%, below: 7mm, inset: (bottom: 3mm), stroke: (bottom: 0.75pt + rule), {{ set text(size: 9pt); grid(columns: (auto, 1fr), column-gutter: 6mm, row-gutter: 1.1em, {}) }})\n\n", cells.join(", "))
}

/// The markup that opens an inline HTML tag, for the tags the reader knows.
fn html_markup(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "b" | "strong" => "#strong[",
        "i" | "em" | "cite" => "#emph[",
        "u" | "ins" | "abbr" => "#underline[",
        "s" | "del" | "strike" => "#strike[",
        "mark" => "#highlight[",
        "sup" => "#super[",
        "sub" => "#sub[",
        "small" => "#text(size: 0.85em)[",
        "kbd" | "code" | "samp" | "tt" => "#kbd[",
        _ => return None,
    })
}

fn mark_fill(color: vault::syntax::MarkColor) -> &'static str {
    use vault::syntax::MarkColor as M;
    match color {
        M::Default => "accent.transparentize(78%)",
        M::Red => "rgb(\"#f8c9c9\")",
        M::Orange => "rgb(\"#fbd9b5\")",
        M::Yellow => "rgb(\"#fbeeb0\")",
        M::Green => "rgb(\"#c8ecd2\")",
        M::Blue => "rgb(\"#c8def8\")",
        M::Purple => "rgb(\"#e0d3f8\")",
    }
}

/// A card's gaps read as marked answers; inside a formula the formula's own preparation takes them.
fn gaps_shown(text: &str) -> String {
    static TASK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(\s*(?:>\s*)*(?:[-*+]|\d+[.)])\s+)\[([^\s\]xX])\]\s").unwrap());
    let marked = TASK.replace_all(text, |c: &regex::Captures| format!("{}{TASK_MARK}{}{TASK_MARK}", &c[1], &c[2]));
    let text: &str = &marked;
    static GAP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{c\d+::((?:[^{}]|\{[^{}]*\})*?)(?:::[^{}]*)?\}\}").unwrap());
    if !text.contains("{{c") {
        return text.to_string();
    }
    let code = vault::links::code_ranges(text);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for c in GAP.captures_iter(text) {
        let whole = c.get(0).unwrap();
        let line = text[..whole.start()].rsplit('\n').next().unwrap_or_default();
        if code.iter().any(|r| r.contains(&whole.start())) || line.matches('$').count() % 2 == 1 {
            continue;
        }
        out.push_str(&text[last..whole.start()]);
        out.push_str(&format!("=={}==", &c[1]));
        last = whole.end();
    }
    out.push_str(&text[last..]);
    out
}

/// A `^id` belongs to links, not to the page.
fn without_block_ids(text: &str) -> String {
    static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)(^|[ \t])\^[A-Za-z0-9-]+[ \t]*$").unwrap());
    let code = vault::links::code_ranges(text);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in ID.find_iter(text) {
        if code.iter().any(|r| r.contains(&m.start())) || text[..m.start()].rsplit('\n').next().is_some_and(|line| line.starts_with("[^")) {
            continue;
        }
        out.push_str(&text[last..m.start()]);
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

/// A search block with what it finds, as far as a page holds it.
fn query_markup(source: &str) -> String {
    let vault = vault::index::current();
    let Ok(parsed) = vault::query::parse(source) else { return format!("#raw(block: true, {})\n\n", str_arg(source)) };
    let found = vault::query::search(&parsed, &vault, vault::query::Sort::NameAsc);
    let shown: Vec<String> = found.iter().take(100).map(|p| format!("[{}]", lit(&p.file_stem().unwrap_or_default().to_string_lossy()))).collect();
    let more = found.len().saturating_sub(100);
    format!(
        "#callout(\"embed\")[#calloutitle(\"embed\")[{}]#grid(row-gutter: 0.8em{}){}]\n\n",
        lit(source.trim()),
        shown.iter().map(|s| format!(", {s}")).collect::<String>(),
        if more > 0 { format!("#linebreak()#text(fill: muted)[+{more}]") } else { String::new() }
    )
}

/// A base's first view as a table.
fn base_markup(source: &str, this: Option<&Path>) -> String {
    let vault = vault::index::current();
    let Ok(base) = vault::bases::Base::parse(source) else { return format!("#raw(block: true, {})\n\n", str_arg(source)) };
    let view = base.views.first().cloned().unwrap_or_default();
    let Ok(rows) = base.rows(&view, &vault, this, chrono::Local::now()) else { return format!("#raw(block: true, {})\n\n", str_arg(source)) };
    let columns = base.columns(&view);
    if columns.is_empty() || rows.is_empty() {
        return "#text(fill: muted)[–]\n\n".to_string();
    }
    let mut cells: Vec<String> = columns.iter().map(|c| format!("[{}]", lit(&base.column_label(c)))).collect();
    for row in rows.iter().take(200) {
        for column in &columns {
            let value = row.cells.iter().find(|(name, _)| name == column).map_or(String::new(), |(_, v)| v.display());
            cells.push(format!("[{}]", lit(&value)));
        }
    }
    let align = vec!["left"; columns.len()].join(", ");
    let total = rows.len().min(200) + 1;
    format!("#notetable(columns: {}, rows: {total}, align: ({align},), {})\n\n", columns.len(), cells.join(", "))
}

/// A canvas drawn to scale: its cards with their text and the lines between them.
fn canvas_markup(path: &Path) -> String {
    let Some(doc) = std::fs::read_to_string(path).ok().and_then(|t| vault::canvas::Canvas::parse(&t)) else { return String::new() };
    let Some((x0, y0, x1, y1)) = doc.bounds() else { return String::new() };
    let (width, height) = ((x1 - x0).max(1.), (y1 - y0).max(1.));
    let scale = (480. / width).min(560. / height).min(1.);
    let mut items = String::new();
    let center = |n: &vault::canvas::Node| ((n.x + n.width / 2. - x0) * scale, (n.y + n.height / 2. - y0) * scale);
    for edge in &doc.edges {
        if let (Some(a), Some(b)) = (doc.nodes.iter().find(|n| n.id == edge.from), doc.nodes.iter().find(|n| n.id == edge.to)) {
            let ((ax, ay), (bx, by)) = (center(a), center(b));
            items.push_str(&format!("place(line(start: ({ax:.1}pt, {ay:.1}pt), end: ({bx:.1}pt, {by:.1}pt), stroke: 0.9pt + marker))\n"));
        }
    }
    for node in &doc.nodes {
        let (x, y, w, h) = ((node.x - x0) * scale, (node.y - y0) * scale, node.width * scale, node.height * scale);
        let (text, group) = match &node.kind {
            vault::canvas::NodeKind::Text(t) => (crate::core::cards::plain_text(t.trim_start_matches(['#', ' '])), false),
            vault::canvas::NodeKind::File { path, .. } => (path.rsplit('/').next().unwrap_or(path).to_string(), false),
            vault::canvas::NodeKind::Link(url) => (url.clone(), false),
            vault::canvas::NodeKind::Group { label, .. } => (label.clone().unwrap_or_default(), true),
        };
        let fill = if group { "none" } else { "white" };
        items.push_str(&format!("place(dx: {x:.1}pt, dy: {y:.1}pt, block(width: {w:.1}pt, height: {h:.1}pt, clip: true, radius: 4pt, fill: {fill}, stroke: 0.75pt + marker, inset: 5pt, text(size: 8pt)[{}]))\n", lit(&text)));
    }
    format!("#block(above: 3mm, below: 4mm, breakable: false, width: {:.1}pt, height: {:.1}pt, {{\n{items}}})\n\n", width * scale, height * scale)
}

/// Compiles a small Typst document to SVG, with its size in points.
pub fn typst_svg(source: &str) -> Result<(String, f32, f32), String> {
    let world = NoteWorld { main: Source::new(file_id("/main.typ"), source.to_string()), files: HashMap::new() };
    let doc: PagedDocument = typst::compile(&world).output.map_err(|errs| errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; "))?;
    let page = doc.pages().first().ok_or_else(|| "leer".to_string())?;
    let size = page.frame.size();
    Ok((typst_svg::svg(page, &typst_svg::SvgOptions::default()), size.x.to_pt() as f32, size.y.to_pt() as f32))
}

/// Marks the head of each callout for the conversion, which has no folding and shows all of them open.
fn callouts_as_quotes(text: &str) -> String {
    static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^((?:>\s*)+)\[!([^\]\s]+)\][+-]?[ \t]*(.*)$").unwrap());
    HEAD.replace_all(text, |c: &regex::Captures| {
        let parsed = vault::syntax::CalloutKind::parse(&c[2]);
        let kind = if &c[2] == "embed" { "embed".to_string() } else { format!("{parsed:?}").to_lowercase() };
        let title = if c[3].trim().is_empty() { super::i18n::tr(parsed.label()) } else { c[3].trim().to_string() };
        format!("{}{CALLOUT_OPEN}{kind}{CALLOUT_KIND_END}{title}{CALLOUT_CLOSE}\n{}", &c[1], &c[1])
    })
    .into_owned()
}

/// Replaces `![[…]]` lines by what they show: notes up to three levels deep, framed with where they come from, other files by placeholders.
fn expand_embeds(text: &str, base: &Path, depth: usize, embeds: &mut Vec<Embed>) -> String {
    static EMBED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^[ \t]*!\[\[([^\]|]+)(?:\|[^\]]*)?\]\][ \t]*$").unwrap());
    if depth >= 3 {
        return text.to_string();
    }
    let vault = vault::index::current();
    let from = base.join("x.md");
    let code = vault::links::code_ranges(text);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for c in EMBED.captures_iter(text) {
        let whole = c.get(0).unwrap();
        if code.iter().any(|r| r.contains(&whole.start())) {
            continue;
        }
        out.push_str(&text[last..whole.start()]);
        last = whole.end();
        let target = vault::links::LinkTarget::parse(&c[1]);
        let name = c[1].to_string();
        let lower = target.path.to_lowercase();
        let extension = lower.rsplit('.').next().unwrap_or_default().to_string();
        if RASTER.contains(&extension.as_str()) {
            out.push_str(whole.as_str());
            continue;
        }
        let Some(path) = vault.resolver.resolve(&target, Some(&from)) else {
            out.push_str(&format!("*{name}*"));
            continue;
        };
        let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let mut placed = |embed: Embed| {
            embeds.push(embed);
            format!("\n{EMBED_OPEN}{}{EMBED_CLOSE}\n", embeds.len() - 1)
        };
        let kind = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if markdown::is_markdown(&path) {
            let Ok(inner) = std::fs::read_to_string(&path) else { continue };
            let body = &inner[vault::frontmatter::body_offset(&inner)..];
            let whole_note = target.headings.is_empty() && target.block.is_none();
            let shown = if whole_note { body.to_string() } else { vault::links::section(body, &target) };
            let folder = path.parent().unwrap_or(base).to_path_buf();
            let inner_text = expand_embeds(&vault::syntax::prepare(&shown), &folder, depth + 1, embeds);
            let title = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            out.push_str(&format!("> [!embed] {title}\n"));
            for line in inner_text.lines() {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            continue;
        }
        let placeholder = match kind.as_str() {
            "pdf" => {
                let page = c[1].split("page=").nth(1).and_then(|p| p.split(['&', '#']).next()).and_then(|p| p.parse().ok()).unwrap_or(1);
                placed(Embed::Pdf { path, page })
            }
            "canvas" => placed(Embed::Canvas(path)),
            "base" => {
                let source = std::fs::read_to_string(&path).unwrap_or_default();
                placed(Embed::Base { source, this: Some(from.clone()) })
            }
            k if AUDIO.contains(&k) => placed(Embed::File { name: file_name, icon: "audio" }),
            k if VIDEO.contains(&k) => placed(Embed::File { name: file_name, icon: "video" }),
            _ => placed(Embed::File { name: file_name, icon: "file" }),
        };
        out.push_str(&placeholder);
    }
    out.push_str(&text[last..]);
    out
}

/// Renders a note to PDF bytes.
pub fn render(markdown_text: &str, base: &Path, meta: Option<&Meta>, o: &Options) -> Result<Vec<u8>, String> {
    render_with(markdown_text, base, meta, o, false).or_else(|first| if first.2 { render_with(markdown_text, base, meta, o, true).map_err(|e| e.0) } else { Err(first.0) })
}

/// A broken picture is no reason to lose the note: the error says whether trying without pictures can help.
fn render_with(markdown_text: &str, base: &Path, meta: Option<&Meta>, o: &Options, no_media: bool) -> Result<Vec<u8>, (String, (), bool)> {
    let (text, images) = build(markdown_text, base, meta, o, no_media);
    let has_media = !images.is_empty();
    let main = Source::new(file_id("/main.typ"), text);
    let mut files = HashMap::new();
    files.insert(file_id("/theme.tmTheme"), Bytes::new(THEME.as_bytes().to_vec()));
    for (i, p) in images.iter().enumerate() {
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if let Ok(b) = std::fs::read(p) {
            files.insert(file_id(&format!("/img/{i}.{ext}")), Bytes::new(b));
        }
    }
    let world = NoteWorld { main, files };
    let joined = |message: String| (message, (), has_media && !no_media);
    let doc: PagedDocument = typst::compile(&world).output.map_err(|errs| joined(errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; ")))?;
    typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default()).map_err(|errs| joined(errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; ")))
}

/// Compiles Typst source with the files it reads, by absolute name, to PDF bytes.
pub fn compile(source: &str, files: Vec<(String, Vec<u8>)>) -> Result<Vec<u8>, String> {
    let files = files.into_iter().map(|(name, bytes)| (file_id(&name), Bytes::new(bytes))).collect();
    let world = NoteWorld { main: Source::new(file_id("/main.typ"), source.to_string()), files };
    let joined = |errs: &[typst::diag::SourceDiagnostic]| errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; ");
    let doc: PagedDocument = typst::compile(&world).output.map_err(|e| joined(&e))?;
    typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default()).map_err(|e| joined(&e))
}

/// Whether a PDF came from this export. Anything else may be a submission and must not be replaced.
pub fn is_export(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| bytes.windows(5).any(|w| w == b"Typst"))
}

/// Renders `markdown_text` and writes it atomically to `target`.
pub fn export(markdown_text: &str, base: &Path, target: &Path, meta: Option<&Meta>, o: &Options) -> Result<(), String> {
    let bytes = render(markdown_text, base, meta, o)?;
    let dir = target.parent().ok_or("no folder")?;
    let tmp = dir.join(format!(".{}.pdf", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

#[cfg(test)]
mod pagination;

#[cfg(test)]
mod pictures;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Meta, Options, render};

    fn opts() -> Options {
        Options { accent: "#0EA5E9".into(), header: true, page_numbers: true, lang: "de".into(), properties: false }
    }

    #[test]
    fn obsidian_syntax_reaches_the_pdf() {
        let (markup, _) = super::source("Sicht %%geheim%%bar\n> [!warning]- Achtung\n> Inhalt\n\n[[Ziel|Anzeige]] und $x^2$ und [web](https://a.b)\n\n```mermaid\ngraph TD; A-->B\n```\n", Path::new("/tmp"), None, &opts());
        assert!(!markup.contains("geheim"));
        assert!(markup.contains("#callout(\"warning\")"), "{}", markup.lines().rev().take(12).collect::<Vec<_>>().join("\n"));
        assert!(markup.contains("Anzeige"));
        assert!(!markup.contains("#link(\"Ziel"), "{markup}");
        assert!(markup.contains("#link(\"https://a.b\")"));
        assert!(markup.contains('$'));
        assert!(markup.contains("#fitimg"), "{markup}");
        assert!(render("Sicht %%geheim%%bar\n\n$\\frac{a}{b}$\n\n> [!tip] Tipp\n> x\n\n```mermaid\ngraph TD; A-->B\n```\n", Path::new("/tmp"), None, &opts()).is_ok());
    }

    const SAMPLE: &str = "# Titel \"mit\" #Zeichen\n\nText mit **fett**, *kursiv*, `code`, ==mark==, [Link](https://a.b) und $5 \\ Backslash.[^1]\n\n## Aufgaben\n\n- [x] fertig\n- [ ] offen\n  - verschachtelt\n\n1. eins\n2. zwei\n\n| A | B |\n|:-:|--:|\n| 1 | 2 |\n| 3 | 4 |\n\n> Zitat\n\n```rust\nfn main() {}\n```\n\n---\n\n<!-- Kommentar --><br>\n\n![fehlt](nicht-da.png)\n\n[^1]: Fußnote.\n";

    #[test]
    fn broken_pictures_do_not_cost_the_note() {
        let dir = std::env::temp_dir().join(format!("schrumbi-export-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("kaputt.png"), b"keine bilddaten").unwrap();
        std::fs::write(dir.join("kaputt.pdf"), b"keine pdfdaten").unwrap();
        let pdf = render("Text\n\n![a](kaputt.png)\n\n<b>offen\n\n<kbd>K</kbd> </u> ==🔴rot== #tag ^block\n\n> [!tip]\n\n> [!bug] **Fett** $x$\n> > [!note]\n> > tief", &dir, None, &opts()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn renders_every_block_type() {
        let pdf = render(SAMPLE, Path::new("/"), Some(&Meta { left: "P1 · Projekt".into(), right: "Name · 30. Oktober 2026".into(), title: String::new() }), &opts()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn recognizes_its_own_exports() {
        let dir = std::env::temp_dir().join(format!("schrumbi-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let own = dir.join("a.pdf");
        super::export("# Titel", Path::new("/"), &own, None, &opts()).unwrap();
        assert!(super::is_export(&own));
        let other = dir.join("b.pdf");
        std::fs::write(&other, b"%PDF-1.4 Abgabe").unwrap();
        assert!(!super::is_export(&other));
    }

    #[test]
    fn renders_empty_and_huge_notes() {
        assert!(render("", Path::new("/"), None, &opts()).unwrap().starts_with(b"%PDF"));
        let long = (0..400).map(|i| format!("## Abschnitt {i}\n\n- [ ] Aufgabe {i}\n\nText {i} ")).collect::<String>();
        assert!(render(&long, Path::new("/"), None, &opts()).unwrap().starts_with(b"%PDF"));
    }
}

#[cfg(test)]
mod demo {
    /// Exports every note of the folder named by `SCHRUMBI_PDF_DEMO`, to look at the result.
    #[test]
    #[ignore]
    fn export_folder() {
        let dir = std::path::PathBuf::from(std::env::var("SCHRUMBI_PDF_DEMO").unwrap());
        crate::core::vault::index::rebuild(&dir, &[]);
        let o = super::Options { accent: "#0EA5E9".into(), header: true, page_numbers: true, lang: "de".into(), properties: true };
        let out = std::path::PathBuf::from(std::env::var("SCHRUMBI_PDF_OUT").unwrap());
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                let text = std::fs::read_to_string(&path).unwrap();
                match super::render(&text, &dir, None, &o) {
                    Ok(pdf) => std::fs::write(out.join(format!("{}.pdf", path.file_stem().unwrap().to_string_lossy())), pdf).unwrap(),
                    Err(e) => eprintln!("FAILED {}: {e}", path.display()),
                }
            }
        }
    }
}
