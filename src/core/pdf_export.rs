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

use super::markdown::{self, Run};

/// Default note accent, matching the Swift app.
pub const DEFAULT_ACCENT: &str = "#0EA5E9";

/// The small line above the note: module and week on the left, author and date on the right.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub left: String,
    pub right: String,
}

/// PDF settings from Settings → Notizen.
#[derive(Debug, Clone)]
pub struct Options {
    pub accent: String,
    pub header: bool,
    pub page_numbers: bool,
    pub extra_css: String,
    pub lang: String,
}

impl Options {
    pub fn current() -> Self {
        let p = super::prefs::get();
        Self { accent: p.note_accent.clone(), header: p.pdf_header, page_numbers: p.pdf_page_numbers, extra_css: p.pdf_extra_css.clone(), lang: if super::i18n::lang() == super::i18n::Lang::En { "en".into() } else { "de".into() } }
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
        if id == self.main.id() { Ok(self.main.clone()) } else { Err(FileError::NotFound(PathBuf::from(id.vpath().get_with_slash()))) }
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
}

impl<'a> Converter<'a> {
    pub fn new(base: &'a Path) -> Self {
        Self { base, images: Vec::new(), stack: vec![String::new()], items: Vec::new(), lists: Vec::new(), table: None, footnotes: Vec::new(), footnote_order: Vec::new(), code: None, image: None }
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

    fn image_markup(&mut self, src: &str, alt: &str) -> String {
        let local = if src.contains("://") {
            None
        } else {
            let decoded = markdown::percent_decode(src);
            let p = if Path::new(&decoded).is_absolute() { PathBuf::from(&decoded) } else { self.base.join(&decoded) };
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            (p.is_file() && matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp")).then_some((p, ext))
        };
        match local {
            Some((p, ext)) => {
                let name = format!("/img/{}.{ext}", self.images.len());
                self.images.push(p);
                format!("#fitimg({})", str_arg(&name))
            }
            None if alt.is_empty() => String::new(),
            None => format!("#text(fill: muted)[{}]", lit(alt)),
        }
    }

    fn html(&mut self, h: &str) {
        static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->|<[^>]+>").unwrap());
        static BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^<br\s*/?>$").unwrap());
        let mut last = 0;
        let mut out = String::new();
        for m in TAG.find_iter(h) {
            let text = &h[last..m.start()];
            if !text.is_empty() {
                out.push_str(&lit(text));
            }
            if BR.is_match(m.as_str()) {
                out.push_str("#linebreak()");
            }
            last = m.end();
        }
        if last < h.len() {
            out.push_str(&lit(&h[last..]));
        }
        self.out().push_str(&out);
    }

    fn end_list(&mut self) {
        let (start, items) = self.lists.pop().unwrap_or_default();
        let depth = self.lists.len();
        let bullet = ["•", "◦", "▪"][depth.min(2)];
        let mut cells = Vec::new();
        for (i, (task, body)) in items.into_iter().enumerate() {
            let marker = match (task, start) {
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
        let events: Vec<Event> = Parser::new_ext(src, markdown::options()).collect();
        for (i, event) in events.iter().enumerate() {
            if let Some((_, buf)) = self.code.as_mut() {
                match event {
                    Event::Text(t) => buf.push_str(t),
                    Event::End(TagEnd::CodeBlock) => {
                        let (lang, buf) = self.code.take().unwrap();
                        let code = buf.strip_suffix('\n').unwrap_or(&buf).to_string();
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
            match event {
                Event::Start(tag) => match tag {
                    Tag::Heading { level, .. } => {
                        let n = markdown::heading_number(*level);
                        self.out().push_str(&format!("#heading(level: {n})["));
                    }
                    Tag::BlockQuote(_) => self.out().push_str("#quote(block: true)["),
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
                        let s = format!("#link({})[", str_arg(dest_url));
                        self.out().push_str(&s);
                    }
                    Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
                    _ => {}
                },
                Event::End(end) => match end {
                    TagEnd::Paragraph => {
                        let inside = !self.items.is_empty() || self.stack.len() > 1;
                        self.out().push_str(if inside { "#parbreak()" } else { "\n\n" });
                    }
                    TagEnd::Heading(_) => self.out().push_str("]\n\n"),
                    TagEnd::BlockQuote(_) => self.out().push_str("]\n\n"),
                    TagEnd::Item => {
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
                    let mut s = String::new();
                    for r in markdown::runs(t) {
                        match r {
                            Run::Text(x) => s.push_str(&lit(&x)),
                            Run::Mark(x) => s.push_str(&format!("#highlight[{}]", lit(&x))),
                        }
                    }
                    self.out().push_str(&s);
                }
                Event::Code(c) => {
                    let s = format!("#raw({})", str_arg(c));
                    self.out().push_str(&s);
                }
                Event::SoftBreak => self.out().push(' '),
                Event::HardBreak => self.out().push_str("#linebreak()"),
                Event::Rule => self.out().push_str("#noterule\n\n"),
                Event::Html(h) | Event::InlineHtml(h) => self.html(h),
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
                _ => {}
            }
        }
        let mut out = self.pop();
        let notes: Vec<String> = self.footnote_order.iter().filter_map(|id| self.footnotes.iter().find(|(f, _)| f == id)).enumerate().map(|(n, (_, body))| format!("[{}.], [{body}]", n + 1)).collect();
        if !notes.is_empty() {
            out.push_str(&format!("#footnotes({})\n", notes.join(", ")));
        }
        (out, self.images)
    }
}

fn preamble(o: &Options, meta: Option<&Meta>) -> String {
    let accent = if valid_hex(&o.accent) { o.accent.trim().to_string() } else { DEFAULT_ACCENT.to_string() };
    let css = css::Sheet::parse(&o.extra_css);
    let var = |name: &str, default: &str| css.vars.get(name).cloned().unwrap_or_else(|| format!("rgb(\"{default}\")"));
    let accent = css.vars.get("accent").cloned().unwrap_or_else(|| format!("rgb(\"{accent}\")"));
    let lang = if o.lang.starts_with("en") { "en" } else { "de" };
    let footer = if o.page_numbers {
        r##"context { let total = counter(page).final().first(); if total > 1 { align(right, text(size: 8pt, weight: 500, fill: rgb("#646d76"), counter(page).display("1 / 1", both: true))) } }"##
    } else {
        "none"
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
        r##"#let accent = {accent}
#let ink = {text}
#let muted = {muted}
#let rule = {rule}
#let soft = {soft}
#let headc = {heading}
#let marker = {marker}
#let code-fill = {code_fill}
#let pre-fill = {pre_fill}
#set document(title: none)
#set page(paper: "a4", margin: 56pt, footer: {footer}, footer-descent: 18pt)
#set text(font: {SANS}, size: 10.5pt, fill: ink, lang: "{lang}", hyphenate: true)
#set par(leading: 0.9em, spacing: 3mm)
#set strong(delta: 250)
#set highlight(fill: accent.transparentize(78%), extent: 1pt, radius: 2pt)
#set raw(theme: "/theme.tmTheme")
#show link: set text(fill: accent)
#show raw: set text(font: {MONO}, size: 9pt)
#show raw.where(block: false): it => box(fill: code-fill, inset: (x: 3pt), outset: (y: 2pt), radius: 3pt, it)
#show raw.where(block: true): it => block(width: 100%, fill: pre-fill, stroke: 0.75pt + rule, radius: 5pt, inset: (x: 4mm, y: 3mm), above: 3mm, below: 4mm, breakable: false, it)
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
#show quote.where(block: true): it => block(above: 4mm, below: 4mm, stroke: (left: 1.5pt + accent), inset: (left: 4mm, y: 1mm), {{ set text(fill: muted); set par(spacing: 1.5mm); it.body }})
#let noterule = block(above: 7mm, below: 7mm, line(length: 100%, stroke: 0.75pt + rule))
#let taskbox(done) = box(width: 9pt, height: 9pt, baseline: 1pt, radius: 2.5pt, stroke: if done {{ 0.9pt + accent }} else {{ 0.9pt + rgb("#a3abb4") }}, fill: if done {{ accent }} else {{ none }}, if done {{ place(dx: 1.6pt, dy: 1.4pt, curve(stroke: (paint: white, thickness: 1.3pt, cap: "round", join: "round"), curve.move((0.2pt, 3.2pt)), curve.line((2.1pt, 5.1pt)), curve.line((5.8pt, 1.2pt)))) }})
#let notelist(..cells) = block(above: 3mm, below: 3mm, grid(columns: (5.5mm, 1fr), column-gutter: 0pt, row-gutter: 0.9em + 1.2mm, align: (right + top, left + top), ..cells.pos().enumerate().map(((i, c)) => if calc.even(i) {{ box(inset: (right: 2.2mm), c) }} else {{ c }})))
#let notetable(columns: 1, rows: 1, align: (left,), ..cells) = block(width: 100%, above: 2mm, below: 5mm, breakable: false, {{ set text(size: 9.5pt); table(columns: (1fr,) * columns, align: align, inset: (x: 2.5mm, y: 2mm), stroke: (x, y) => if y == 0 {{ (bottom: 1.1pt + headc) }} else if y < rows - 1 {{ (bottom: 0.75pt + rule) }} else {{ none }}, table.header(..cells.pos().slice(0, columns).map(c => strong(c))), ..cells.pos().slice(columns)) }})
#let fnref(n) = super(size: 7pt, text(fill: accent, str(n)))
#let footnotes(..cells) = block(width: 100%, above: 8mm, stroke: (top: 0.75pt + rule), inset: (top: 3mm), {{ set text(size: 8.5pt, fill: muted); grid(columns: (5mm, 1fr), row-gutter: 1.2em, ..cells) }})
#let fitimg(path) = layout(size => {{ let img = image(path); let w = measure(img).width; if w > size.width {{ image(path, width: 100%) }} else {{ img }} }})
{rules}
{header}"##,
        text = var("text", "#1d2125"),
        muted = var("muted", "#646d76"),
        rule = var("rule", "#e2e5e9"),
        soft = var("soft", "#f5f7f9"),
        heading = var("heading", "#111417"),
        marker = var("marker", "#8a939c"),
        code_fill = css.code_fill.clone().unwrap_or_else(|| "soft".into()),
        pre_fill = css.pre_fill.clone().unwrap_or_else(|| "soft".into()),
        rules = css.rules.join("\n"),
    )
}

/// Builds the Typst source for a note, for tests and diagnostics.
pub fn source(markdown_text: &str, base: &Path, meta: Option<&Meta>, o: &Options) -> (String, Vec<PathBuf>) {
    let (body, images) = Converter::new(base).convert(markdown_text);
    (format!("{}{body}", preamble(o, meta)), images)
}

/// Renders a note to PDF bytes.
pub fn render(markdown_text: &str, base: &Path, meta: Option<&Meta>, o: &Options) -> Result<Vec<u8>, String> {
    let (text, images) = source(markdown_text, base, meta, o);
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
    let doc: PagedDocument = typst::compile(&world).output.map_err(|errs| errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; "))?;
    typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default()).map_err(|errs| errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; "))
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

/// The supported subset of the "Eigenes CSS" setting, mapped onto Typst rules.
mod css {
    use std::collections::HashMap;
    use std::sync::LazyLock;

    use regex::Regex;

    #[derive(Default)]
    pub struct Sheet {
        pub vars: HashMap<String, String>,
        pub rules: Vec<String>,
        pub code_fill: Option<String>,
        pub pre_fill: Option<String>,
    }

    fn color(v: &str, vars: &HashMap<String, String>) -> Option<String> {
        static VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^var\(--([\w-]+)\)$").unwrap());
        static RGB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)(?:[\s,/]+([\d.]+%?))?\s*\)$").unwrap());
        let v = v.trim().trim_end_matches("!important").trim();
        if let Some(c) = VAR.captures(v) {
            let name = &c[1];
            return vars.get(name).cloned().or_else(|| matches!(name, "accent" | "text" | "muted" | "rule" | "soft" | "heading" | "marker").then(|| if name == "text" { "ink".into() } else if name == "heading" { "headc".into() } else { name.into() }));
        }
        if super::valid_hex(v) {
            return Some(format!("rgb(\"{v}\")"));
        }
        if let Some(c) = RGB.captures(v) {
            let a = c.get(4).map(|a| {
                let s = a.as_str();
                if let Some(p) = s.strip_suffix('%') { p.parse::<f32>().unwrap_or(100.) / 100. } else { s.parse::<f32>().unwrap_or(1.) }
            });
            let (r, g, b) = (c[1].parse::<u8>().ok()?, c[2].parse::<u8>().ok()?, c[3].parse::<u8>().ok()?);
            return Some(match a {
                Some(a) => format!("rgb({r}, {g}, {b}, {}%)", (a.clamp(0., 1.) * 100.).round()),
                None => format!("rgb({r}, {g}, {b})"),
            });
        }
        let hex = match v.to_ascii_lowercase().as_str() {
            "black" => "#000000",
            "white" => "#ffffff",
            "red" => "#ff0000",
            "green" => "#008000",
            "blue" => "#0000ff",
            "gray" | "grey" => "#808080",
            "orange" => "#ffa500",
            "purple" => "#800080",
            "navy" => "#000080",
            "teal" => "#008080",
            "maroon" => "#800000",
            "transparent" => return Some("none".into()),
            _ => return None,
        };
        Some(format!("rgb(\"{hex}\")"))
    }

    fn length(v: &str) -> Option<String> {
        static LEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(-?[\d.]+)(pt|px|em|rem|mm|cm|%)$").unwrap());
        let c = LEN.captures(v.trim())?;
        let n: f32 = c[1].parse().ok()?;
        Some(match &c[2] {
            "px" => format!("{}pt", n * 0.75),
            "rem" => format!("{}pt", n * 10.5),
            "%" => format!("{}em", n / 100.),
            u => format!("{n}{u}"),
        })
    }

    fn fonts(v: &str) -> Option<String> {
        let names: Vec<String> = v
            .split(',')
            .map(|f| f.trim().trim_matches(['"', '\'']).to_string())
            .filter(|f| !f.is_empty() && !matches!(f.as_str(), "sans-serif" | "serif" | "monospace" | "system-ui" | "-apple-system" | "BlinkMacSystemFont" | "ui-sans-serif" | "ui-monospace" | "inherit"))
            .collect();
        (!names.is_empty()).then(|| format!("({},)", names.iter().map(|n| format!("{n:?}")).collect::<Vec<_>>().join(", ")))
    }

    fn targets(selector: &str) -> Vec<&'static str> {
        selector
            .split(',')
            .filter_map(|s| match s.trim() {
                "html" | "body" | "*" => Some("body"),
                "h1" => Some("heading.where(level: 1)"),
                "h2" => Some("heading.where(level: 2)"),
                "h3" => Some("heading.where(level: 3)"),
                "h4" => Some("heading.where(level: 4)"),
                "h5" => Some("heading.where(level: 5)"),
                "h6" => Some("heading.where(level: 6)"),
                "h1, h2, h3, h4, h5, h6" => Some("heading"),
                "p" => Some("par"),
                "a" => Some("link"),
                "strong" | "b" => Some("strong"),
                "em" | "i" => Some("emph"),
                "code" => Some("raw"),
                "pre" | "pre code" => Some("raw.where(block: true)"),
                "blockquote" => Some("quote"),
                "mark" => Some("highlight"),
                "table" | "td" => Some("table"),
                "th" => Some("table.header"),
                _ => None,
            })
            .collect()
    }

    impl Sheet {
        pub fn parse(src: &str) -> Self {
            static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/").unwrap());
            static BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([^{}]+)\{([^{}]*)\}").unwrap());
            let src = COMMENT.replace_all(src, "");
            let mut sheet = Sheet::default();
            for b in BLOCK.captures_iter(&src) {
                let selector = b[1].trim();
                let decls: Vec<(String, String)> = b[2].split(';').filter_map(|d| d.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().trim_end_matches("!important").trim().to_string())).collect();
                if selector == ":root" {
                    for (k, v) in &decls {
                        if let Some(name) = k.strip_prefix("--")
                            && let Some(c) = color(v, &sheet.vars)
                        {
                            sheet.vars.insert(name.to_string(), c);
                        }
                    }
                }
                for target in targets(selector) {
                    let mut text = Vec::new();
                    for (k, v) in &decls {
                        match k.as_str() {
                            "color" => {
                                if let Some(c) = color(v, &sheet.vars) {
                                    text.push(format!("fill: {c}"));
                                }
                            }
                            "font-size" => {
                                if let Some(l) = length(v) {
                                    text.push(format!("size: {l}"));
                                }
                            }
                            "font-family" | "font" if k == "font-family" => {
                                if let Some(f) = fonts(v) {
                                    text.push(format!("font: {f}"));
                                }
                            }
                            "font-weight" => match v.as_str() {
                                "bold" | "bolder" => text.push("weight: 700".into()),
                                "normal" | "lighter" => text.push("weight: 400".into()),
                                n => {
                                    if let Ok(n) = n.parse::<u16>() {
                                        text.push(format!("weight: {}", n.clamp(100, 900)));
                                    }
                                }
                            },
                            "font-style" if v == "italic" || v == "oblique" => text.push("style: \"italic\"".into()),
                            "font-style" if v == "normal" => text.push("style: \"normal\"".into()),
                            "letter-spacing" => {
                                if let Some(l) = length(v) {
                                    text.push(format!("tracking: {l}"));
                                }
                            }
                            "text-transform" if v == "uppercase" && target != "body" => sheet.rules.push(format!("#show {target}: upper")),
                            "text-align" => {
                                let rule = match v.as_str() {
                                    "justify" => Some("set par(justify: true)".to_string()),
                                    "center" | "right" | "left" => Some(format!("set align({v})")),
                                    _ => None,
                                };
                                if let Some(r) = rule {
                                    sheet.rules.push(if target == "body" { format!("#{r}") } else { format!("#show {target}: {r}") });
                                }
                            }
                            "background" | "background-color" => {
                                if let Some(c) = color(v, &sheet.vars) {
                                    match target {
                                        "raw" => sheet.code_fill = Some(c),
                                        "raw.where(block: true)" => sheet.pre_fill = Some(c),
                                        "highlight" => sheet.rules.push(format!("#set highlight(fill: {c})")),
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    if !text.is_empty() {
                        let args = text.join(", ");
                        sheet.rules.push(if target == "body" { format!("#set text({args})") } else { format!("#show {target}: set text({args})") });
                    }
                }
            }
            sheet
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Meta, Options, render, source};

    fn opts(css: &str) -> Options {
        Options { accent: "#0EA5E9".into(), header: true, page_numbers: true, extra_css: css.into(), lang: "de".into() }
    }

    const SAMPLE: &str = "# Titel \"mit\" #Zeichen\n\nText mit **fett**, *kursiv*, `code`, ==mark==, [Link](https://a.b) und $5 \\ Backslash.[^1]\n\n## Aufgaben\n\n- [x] fertig\n- [ ] offen\n  - verschachtelt\n\n1. eins\n2. zwei\n\n| A | B |\n|:-:|--:|\n| 1 | 2 |\n| 3 | 4 |\n\n> Zitat\n\n```rust\nfn main() {}\n```\n\n---\n\n<!-- Kommentar --><br>\n\n![fehlt](nicht-da.png)\n\n[^1]: Fußnote.\n";

    #[test]
    fn renders_every_block_type() {
        let pdf = render(SAMPLE, Path::new("/"), Some(&Meta { left: "P1 · Projekt".into(), right: "Name · 30. Oktober 2026".into() }), &opts("")).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn applies_css_subset_and_survives_garbage() {
        let css = ":root { --accent: #ff0000; } h2 { color: var(--accent); font-size: 18px; text-transform: uppercase } body { font-family: Georgia, serif; text-align: justify } code { background: rgba(0,0,0,0.1) } }}} nonsense {";
        let (src, _) = source(SAMPLE, Path::new("/"), None, &opts(css));
        assert!(src.contains("#let accent = rgb(\"#ff0000\")"));
        assert!(src.contains("#show heading.where(level: 2): set text(fill: rgb(\"#ff0000\"), size: 13.5pt)"));
        assert!(src.contains("#set par(justify: true)"));
        assert!(render(SAMPLE, Path::new("/"), None, &opts(css)).unwrap().starts_with(b"%PDF"));
    }

    #[test]
    fn renders_empty_and_huge_notes() {
        assert!(render("", Path::new("/"), None, &opts("")).unwrap().starts_with(b"%PDF"));
        let long = (0..400).map(|i| format!("## Abschnitt {i}\n\n- [ ] Aufgabe {i}\n\nText {i} ")).collect::<String>();
        assert!(render(&long, Path::new("/"), None, &opts("")).unwrap().starts_with(b"%PDF"));
    }
}
