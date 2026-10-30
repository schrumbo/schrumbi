use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use hayro::hayro_interpret::font::{GlyphRun, Glyph};
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache, InterpreterSettings, SoftMask, interpret_page};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{Affine, BezPath, Point, Rect, Shape};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

/// A rectangle on a page in points, with the origin at the page's top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PageRect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

/// One rendered page as premultiplied BGRA pixels.
pub struct PagePixels {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// One entry of the document's table of contents.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineEntry {
    pub level: usize,
    pub title: String,
    pub page: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkTarget {
    Page(usize),
    Uri(String),
}

/// A clickable area on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageLink {
    pub rect: PageRect,
    pub target: LinkTarget,
}

/// Table of contents and links, which need a slower full parse of the file.
#[derive(Debug, Default)]
pub struct PdfMeta {
    pub outline: Vec<OutlineEntry>,
    pub links: HashMap<usize, Vec<PageLink>>,
}

struct TextGlyph {
    text: String,
    rect: PageRect,
    baseline: f32,
}

/// The text of one page with the position of every glyph.
#[derive(Default)]
pub struct PageText {
    glyphs: Vec<TextGlyph>,
    text: String,
    /// Byte offset in `text` where each glyph starts.
    starts: Vec<usize>,
    folded: String,
    /// For each byte of `folded`, the glyph it came from.
    folded_glyph: Vec<u32>,
}

fn fold(c: char, out: &mut String) {
    match c {
        '\n' | '\u{a0}' | '\t' => out.push(' '),
        'ß' => out.push_str("ss"),
        c => out.extend(c.to_lowercase()),
    }
}

/// Lowercases `query` and normalizes its whitespace the way page text is searched.
pub fn fold_query(query: &str) -> String {
    let mut out = String::new();
    for c in query.trim().chars() {
        fold(c, &mut out);
    }
    out
}

impl PageText {
    fn new(glyphs: Vec<TextGlyph>) -> Self {
        let (mut text, mut starts) = (String::new(), Vec::with_capacity(glyphs.len()));
        let mut previous: Option<&TextGlyph> = None;
        for g in &glyphs {
            if let Some(p) = previous {
                let em = p.rect.h.max(g.rect.h).max(1.);
                let same_line = (p.baseline - g.baseline).abs() < em * 0.5;
                if !same_line {
                    if !text.ends_with(' ') && !text.ends_with('\n') {
                        text.push('\n');
                    }
                } else if g.rect.x - (p.rect.x + p.rect.w) > em * 0.15 && !text.ends_with(' ') && !g.text.starts_with(' ') {
                    text.push(' ');
                }
            }
            starts.push(text.len());
            text.push_str(&g.text);
            previous = Some(g);
        }
        let (mut folded, mut folded_glyph) = (String::with_capacity(text.len()), Vec::with_capacity(text.len()));
        let mut glyph = 0usize;
        for (offset, c) in text.char_indices() {
            while glyph + 1 < starts.len() && starts[glyph + 1] <= offset {
                glyph += 1;
            }
            let before = folded.len();
            fold(c, &mut folded);
            folded_glyph.resize(folded.len(), glyph as u32);
            debug_assert!(folded.len() >= before);
        }
        Self { glyphs, text, starts, folded, folded_glyph }
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// Glyph ranges (start inclusive, end exclusive) of every match of an already folded query.
    pub fn find(&self, folded_query: &str) -> Vec<(usize, usize)> {
        if folded_query.is_empty() {
            return Vec::new();
        }
        self.folded
            .match_indices(folded_query)
            .filter_map(|(at, m)| {
                let (first, last) = (*self.folded_glyph.get(at)?, *self.folded_glyph.get(at + m.len() - 1)?);
                Some((first as usize, last as usize + 1))
            })
            .collect()
    }

    /// The glyph under or closest to a point: the nearest line first, then the nearest position on it.
    pub fn glyph_at(&self, x: f32, y: f32) -> Option<usize> {
        let line = |g: &TextGlyph| if y < g.rect.y { g.rect.y - y } else { (y - (g.rect.y + g.rect.h)).max(0.) };
        let across = |g: &TextGlyph| if x < g.rect.x { g.rect.x - x } else { (x - (g.rect.x + g.rect.w)).max(0.) };
        self.glyphs.iter().enumerate().min_by(|(_, a), (_, b)| line(a).total_cmp(&line(b)).then(across(a).total_cmp(&across(b)))).map(|(i, _)| i)
    }

    /// Whether a point lies on text, which decides the mouse cursor and where a selection starts.
    pub fn on_text(&self, x: f32, y: f32) -> bool {
        self.glyphs.iter().any(|g| g.rect.contains(x, y))
    }

    /// The glyph range of the word around a glyph.
    pub fn word_at(&self, glyph: usize) -> (usize, usize) {
        let is_word = |i: usize| self.glyphs.get(i).is_some_and(|g| g.text.chars().all(char::is_alphanumeric));
        if !is_word(glyph) {
            return (glyph, (glyph + 1).min(self.glyphs.len()));
        }
        let joined = |i: usize| self.starts[i] + self.glyphs[i].text.len() == self.starts[i + 1];
        let (mut start, mut end) = (glyph, glyph + 1);
        while start > 0 && is_word(start - 1) && joined(start - 1) {
            start -= 1;
        }
        while is_word(end) && joined(end - 1) {
            end += 1;
        }
        (start, end)
    }

    /// The left half of a glyph selects before it, the right half after it.
    pub fn caret_at(&self, x: f32, y: f32) -> usize {
        let Some(i) = self.glyph_at(x, y) else { return 0 };
        let r = self.glyphs[i].rect;
        if x > r.x + r.w / 2. { i + 1 } else { i }
    }

    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// One rectangle per line for a glyph range.
    pub fn rects(&self, start: usize, end: usize) -> Vec<PageRect> {
        let mut out: Vec<PageRect> = Vec::new();
        let mut baseline = f32::NAN;
        for g in self.glyphs.get(start..end.min(self.glyphs.len())).unwrap_or(&[]) {
            let r = g.rect;
            if let Some(last) = out.last_mut().filter(|l| (baseline - g.baseline).abs() < l.h.max(r.h) * 0.5) {
                let right = (last.x + last.w).max(r.x + r.w);
                let bottom = (last.y + last.h).max(r.y + r.h);
                last.x = last.x.min(r.x);
                last.y = last.y.min(r.y);
                last.w = right - last.x;
                last.h = bottom - last.y;
            } else {
                out.push(r);
                baseline = g.baseline;
            }
        }
        out
    }

    /// The text of a glyph range.
    pub fn slice(&self, start: usize, end: usize) -> String {
        if start >= end || start >= self.starts.len() {
            return String::new();
        }
        let from = self.starts[start];
        let to = self.starts.get(end).copied().unwrap_or(self.text.len());
        self.text[from..to].trim_end().to_string()
    }

    /// A short excerpt around a glyph range, for result lists.
    pub fn excerpt(&self, start: usize, end: usize) -> String {
        let from = self.starts.get(start).copied().unwrap_or(0);
        let to = self.starts.get(end).copied().unwrap_or(self.text.len());
        let mut a = from;
        for _ in 0..30 {
            match self.text[..a].char_indices().next_back() {
                Some((i, _)) => a = i,
                None => break,
            }
        }
        let mut b = to;
        for _ in 0..50 {
            match self.text[b..].chars().next() {
                Some(c) => b += c.len_utf8(),
                None => break,
            }
        }
        let body = self.text[a..b].replace('\n', " ");
        format!("{}{}{}", if a > 0 { "…" } else { "" }, body.trim(), if b < self.text.len() { "…" } else { "" })
    }
}

/// Collects positioned glyphs and ignores everything else on the page.
#[derive(Default)]
struct TextCollector {
    glyphs: Vec<TextGlyph>,
}

impl<'a> Device<'a> for TextCollector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        let glyphs = run.glyphs();
        for (i, g) in glyphs.iter().enumerate() {
            let Some(text) = g.as_unicode().map(|u| match u {
                hayro::hayro_interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
                hayro::hayro_interpret::hayro_cmap::BfString::String(s) => s,
            }) else {
                continue;
            };
            let t: Affine = props.transform * g.transform();
            let origin = t * Point::new(0., 0.);
            let top = t * Point::new(0., 800.);
            let bottom = t * Point::new(0., -200.);
            let height = (top.y - bottom.y).abs() as f32;
            let right = match &**g {
                Glyph::Outline(o) => {
                    let bounds = o.outline().bounding_box();
                    (t * Point::new(bounds.x1.max(0.), 0.)).x
                }
                Glyph::Type3(_) => origin.x + height as f64 * 0.5,
            };
            let advance = glyphs.get(i + 1).map(|next| (props.transform * next.transform() * Point::new(0., 0.)).x).filter(|x| *x > origin.x && *x - origin.x < height as f64 * 2.).unwrap_or(right);
            self.glyphs.push(TextGlyph {
                text,
                rect: PageRect { x: origin.x as f32, y: top.y.min(bottom.y) as f32, w: (advance - origin.x).max(0.) as f32, h: height },
                baseline: origin.y as f32,
            });
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// A loaded PDF whose pages render on demand.
pub struct PdfDoc {
    pdf: Pdf,
}

impl PdfDoc {
    pub fn open(path: &Path) -> Option<Self> {
        Self::from_data(Arc::new(std::fs::read(path).ok()?))
    }

    /// Opens shared bytes, so several worker threads can read one file without copying it.
    pub fn from_data(data: Arc<Vec<u8>>) -> Option<Self> {
        Some(Self { pdf: Pdf::new(data).ok()? })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Page size in points.
    pub fn page_size(&self, index: usize) -> Option<(f32, f32)> {
        self.pdf.pages().get(index).map(|p| p.render_dimensions())
    }

    /// PNG of one page at `scale` pixels per point, on white.
    pub fn render_png(&self, index: usize, scale: f32) -> Option<Vec<u8>> {
        let page = self.pdf.pages().get(index)?;
        let cache = RenderCache::new();
        let pixmap = hayro::render(page, &cache, &InterpreterSettings::default(), &RenderSettings::default(), &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE });
        pixmap.into_png().ok()
    }

    /// One page at `scale` pixels per point, on white.
    pub fn render_pixels(&self, index: usize, scale: f32) -> Option<PagePixels> {
        let page = self.pdf.pages().get(index)?;
        let cache = RenderCache::new();
        let pixmap = hayro::render(page, &cache, &InterpreterSettings::default(), &RenderSettings::default(), &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE });
        let (width, height) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
        let mut bgra = pixmap.data_as_u8_slice().to_vec();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Some(PagePixels { width, height, bgra })
    }

    /// The text of one page with glyph positions.
    pub fn page_text(&self, index: usize) -> PageText {
        let Some(page) = self.pdf.pages().get(index) else { return PageText::default() };
        let cache = InterpreterCache::new();
        let (w, h) = page.render_dimensions();
        let mut context = Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0., 0., f64::from(w), f64::from(h)), &cache, page.xref(), InterpreterSettings::default());
        let mut collector = TextCollector::default();
        interpret_page(page, &mut context, &mut collector);
        PageText::new(collector.glyphs)
    }

    /// Reads the table of contents and links. Slow on big files, so call it off the main thread.
    pub fn meta(&self, data: &[u8]) -> PdfMeta {
        let Ok(doc) = lopdf::Document::load_mem(data) else { return PdfMeta::default() };
        let mut meta = PdfMeta::default();
        if let Ok(toc) = doc.get_toc() {
            meta.outline = toc.toc.into_iter().map(|e| OutlineEntry { level: e.level, title: e.title, page: e.page.saturating_sub(1) }).collect();
        }
        let page_ids: HashMap<lopdf::ObjectId, usize> = doc.page_iter().enumerate().map(|(i, id)| (id, i)).collect();
        let named = named_destinations(&doc);
        for (&id, &index) in &page_ids {
            let Ok(annots) = doc.get_page_annotations(id) else { continue };
            let Some(page) = self.pdf.pages().get(index) else { continue };
            let transform = page.initial_transform(true).to_kurbo();
            let links: Vec<PageLink> = annots.iter().filter_map(|a| link_of(&doc, a, &page_ids, &named, transform)).collect();
            if !links.is_empty() {
                meta.links.insert(index, links);
            }
        }
        meta
    }
}

fn named_destinations(doc: &lopdf::Document) -> HashMap<Vec<u8>, lopdf::Object> {
    let mut out = HashMap::new();
    let Ok(catalog) = doc.catalog() else { return out };
    if let Ok(dests) = catalog.get(b"Dests").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_dict()) {
        for (name, value) in dests.iter() {
            out.insert(name.clone(), value.clone());
        }
    }
    if let Ok(root) = catalog.get(b"Names").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_dict()).and_then(|n| n.get(b"Dests")).and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_dict()) {
        collect_name_tree(doc, root, &mut out, 0);
    }
    out
}

fn collect_name_tree(doc: &lopdf::Document, node: &lopdf::Dictionary, out: &mut HashMap<Vec<u8>, lopdf::Object>, depth: usize) {
    if depth > 32 {
        return;
    }
    if let Ok(names) = node.get(b"Names").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_array()) {
        for pair in names.chunks_exact(2) {
            if let Ok(name) = pair[0].as_str() {
                out.insert(name.to_vec(), pair[1].clone());
            }
        }
    }
    if let Ok(kids) = node.get(b"Kids").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_array()) {
        for kid in kids {
            if let Ok(dict) = doc.dereference(kid).and_then(|(_, o)| o.as_dict()) {
                collect_name_tree(doc, dict, out, depth + 1);
            }
        }
    }
}

/// Follows a destination, which may be an array, a name or a dictionary holding either, to its page.
fn destination_page(doc: &lopdf::Document, dest: &lopdf::Object, pages: &HashMap<lopdf::ObjectId, usize>, named: &HashMap<Vec<u8>, lopdf::Object>, depth: usize) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    let (_, dest) = doc.dereference(dest).ok()?;
    match dest {
        lopdf::Object::Array(a) => match a.first()? {
            lopdf::Object::Reference(id) => pages.get(id).copied(),
            lopdf::Object::Integer(i) => usize::try_from(*i).ok(),
            _ => None,
        },
        lopdf::Object::Name(n) | lopdf::Object::String(n, _) => destination_page(doc, named.get(n)?, pages, named, depth + 1),
        lopdf::Object::Dictionary(d) => destination_page(doc, d.get(b"D").ok()?, pages, named, depth + 1),
        _ => None,
    }
}

fn link_of(doc: &lopdf::Document, annot: &lopdf::Dictionary, pages: &HashMap<lopdf::ObjectId, usize>, named: &HashMap<Vec<u8>, lopdf::Object>, transform: Affine) -> Option<PageLink> {
    if annot.get(b"Subtype").ok()?.as_name().ok()? != b"Link" {
        return None;
    }
    let number = |o: &lopdf::Object| o.as_float().ok().or_else(|| o.as_i64().ok().map(|i| i as f32));
    let (_, rect) = doc.dereference(annot.get(b"Rect").ok()?).ok()?;
    let rect = rect.as_array().ok()?;
    let (x1, y1, x2, y2) = (number(rect.first()?)?, number(rect.get(1)?)?, number(rect.get(2)?)?, number(rect.get(3)?)?);
    let (a, b) = (transform * Point::new(f64::from(x1), f64::from(y1)), transform * Point::new(f64::from(x2), f64::from(y2)));
    let rect = PageRect { x: a.x.min(b.x) as f32, y: a.y.min(b.y) as f32, w: (a.x - b.x).abs() as f32, h: (a.y - b.y).abs() as f32 };
    let target = if let Ok(dest) = annot.get(b"Dest") {
        LinkTarget::Page(destination_page(doc, dest, pages, named, 0)?)
    } else {
        let (_, action) = doc.dereference(annot.get(b"A").ok()?).ok()?;
        let action = action.as_dict().ok()?;
        match action.get(b"S").ok()?.as_name().ok()? {
            b"URI" => {
                let (_, uri) = doc.dereference(action.get(b"URI").ok()?).ok()?;
                LinkTarget::Uri(String::from_utf8_lossy(uri.as_str().ok()?).to_string())
            }
            b"GoTo" => LinkTarget::Page(destination_page(doc, action.get(b"D").ok()?, pages, named, 0)?),
            _ => return None,
        }
    };
    Some(PageLink { rect, target })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        use lopdf::content::{Content, Operation};
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
        let mut kids: Vec<Object> = Vec::new();
        let mut ids = Vec::new();
        for (i, line) in ["Erste Seite mit Regenbogen", "Zweite Seite ohne", "Dritte REGENBOGEN Seite"].iter().enumerate() {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 12.into()]),
                    Operation::new("Td", vec![72.into(), 700.into()]),
                    Operation::new("Tj", vec![Object::string_literal(*line)]),
                    Operation::new("ET", vec![]),
                ],
            };
            let contents = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => contents, "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } });
            ids.push(page);
            kids.push(page.into());
        }
        let annot = dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![72.into(), 690.into(), 200.into(), 712.into()], "Dest" => vec![ids[2].into(), "Fit".into()] };
        let uri = dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![72.into(), 600.into(), 200.into(), 612.into()], "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.org") } };
        doc.get_object_mut(ids[0]).unwrap().as_dict_mut().unwrap().set("Annots", vec![Object::Dictionary(annot), Object::Dictionary(uri)]);
        let count = kids.len() as i64;
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
        let outlines = doc.new_object_id();
        let item = doc.add_object(dictionary! { "Title" => Object::string_literal("Kapitel Drei"), "Parent" => outlines, "Dest" => vec![ids[2].into(), "Fit".into()] });
        doc.objects.insert(outlines, Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => item, "Last" => item, "Count" => 1 }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id, "Outlines" => outlines });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn open() -> (PdfDoc, Vec<u8>) {
        let data = fixture();
        (PdfDoc::from_data(Arc::new(data.clone())).unwrap(), data)
    }

    #[test]
    fn extracts_text_with_positions_on_the_page() {
        let (doc, _) = open();
        let text = doc.page_text(0);
        assert_eq!(text.slice(0, text.glyph_count()), "Erste Seite mit Regenbogen");
        let hit = text.find("regenbogen");
        assert_eq!(hit.len(), 1);
        let rects = text.rects(hit[0].0, hit[0].1);
        assert_eq!(rects.len(), 1);
        let r = rects[0];
        assert!(r.x > 72. && r.x < 300. && r.w > 40. && r.w < 120., "{r:?}");
        assert!(r.y > 70. && r.y < 100. && r.h > 8. && r.h < 16., "{r:?}");
    }

    #[test]
    fn search_ignores_case_and_handles_empty_input() {
        let (doc, _) = open();
        assert_eq!(doc.page_text(2).find(&fold_query("Regenbogen")).len(), 1);
        assert!(doc.page_text(1).find(&fold_query("Regenbogen")).is_empty());
        assert!(doc.page_text(0).find("").is_empty());
        assert_eq!(fold_query("  Straße "), "strasse");
    }

    #[test]
    fn caret_follows_the_nearest_glyph() {
        let (doc, _) = open();
        let text = doc.page_text(0);
        assert_eq!(text.caret_at(0., 85.), 0);
        assert_eq!(text.caret_at(600., 85.), text.glyph_count());
        assert_eq!(text.slice(text.caret_at(0., 85.), 5), "Erste");
    }

    #[test]
    fn word_selection_stops_at_gaps() {
        let (doc, _) = open();
        let text = doc.page_text(0);
        let (start, end) = text.word_at(1);
        assert_eq!(text.slice(start, end), "Erste");
    }

    #[test]
    fn reads_outline_and_links() {
        let (doc, data) = open();
        let meta = doc.meta(&data);
        assert_eq!(meta.outline, vec![OutlineEntry { level: 1, title: "Kapitel Drei".into(), page: 2 }]);
        let links = &meta.links[&0];
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target, LinkTarget::Page(2));
        assert_eq!(links[1].target, LinkTarget::Uri("https://example.org".into()));
        assert!(links[0].rect.contains(100., 90.), "{:?}", links[0].rect);
    }

    #[test]
    fn broken_files_do_not_panic() {
        assert!(PdfDoc::from_data(Arc::new(b"kein pdf".to_vec())).is_none());
        let (doc, _) = open();
        assert!(doc.render_pixels(99, 1.).is_none());
        assert!(doc.page_text(99).is_empty());
        assert!(doc.meta(b"kein pdf").outline.is_empty());
    }

    #[test]
    fn renders_opaque_bgra_at_the_requested_scale() {
        let (doc, _) = open();
        let p = doc.render_pixels(0, 0.5).unwrap();
        assert_eq!((p.width, p.height), (306, 396));
        assert_eq!(p.bgra.len(), 306 * 396 * 4);
        assert_eq!(&p.bgra[..4], &[255, 255, 255, 255]);
    }

    #[test]
    #[ignore = "measures a real file; set SCHRUMBI_BENCH_PDF"]
    fn bench_big_file() {
        let path = std::env::var("SCHRUMBI_BENCH_PDF").unwrap();
        let t = std::time::Instant::now();
        let data = Arc::new(std::fs::read(path).unwrap());
        let doc = PdfDoc::from_data(data.clone()).unwrap();
        println!("open {:?}, {} pages", t.elapsed(), doc.page_count());
        let t = std::time::Instant::now();
        let sizes: Vec<_> = (0..doc.page_count()).filter_map(|i| doc.page_size(i)).collect();
        println!("sizes {:?} ({})", t.elapsed(), sizes.len());
        for scale in [0.25, 1., 2., 4.] {
            let t = std::time::Instant::now();
            let p = doc.render_pixels(doc.page_count() / 2, scale).unwrap();
            println!("render x{scale}: {:?} ({}x{})", t.elapsed(), p.width, p.height);
        }
        let t = std::time::Instant::now();
        let text = doc.page_text(doc.page_count() / 2);
        println!("text one page {:?} ({} glyphs)", t.elapsed(), text.glyph_count());
        let t = std::time::Instant::now();
        let hits: usize = (0..doc.page_count()).map(|i| doc.page_text(i).find("regenbogen").len()).sum();
        println!("search all pages {:?} ({hits} hits)", t.elapsed());
        let t = std::time::Instant::now();
        let meta = doc.meta(&data);
        println!("meta {:?} ({} outline, {} pages with links)", t.elapsed(), meta.outline.len(), meta.links.len());
    }
}
