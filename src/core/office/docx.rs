use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use super::package::{self, Package, Rel};
use super::typ::{self, Align};
use super::xml::El;
use super::{OpenError, Source};

/// Character formatting; unset fields inherit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunStyle {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub size: Option<f32>,
    pub color: Option<String>,
    pub font: Option<String>,
    pub highlight: Option<String>,
    pub script: Option<i8>,
    pub caps: Option<bool>,
    pub small_caps: Option<bool>,
    pub hidden: Option<bool>,
}

impl RunStyle {
    fn over(&mut self, o: &RunStyle) {
        macro_rules! take {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        take!(bold, italic, underline, strike, size, color, font, highlight, script, caps, small_caps, hidden);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Line {
    Auto(f32),
    Exact(f32),
    AtLeast(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TabKind {
    Left,
    Right,
    Center,
    Clear,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ParaStyle {
    align: Option<Align>,
    before: Option<f32>,
    after: Option<f32>,
    line: Option<Line>,
    left: Option<f32>,
    right: Option<f32>,
    first: Option<f32>,
    num: Option<(String, u8)>,
    outline: Option<u8>,
    page_break_before: Option<bool>,
    contextual: Option<bool>,
    shading: Option<String>,
    tabs: Vec<(f32, TabKind, bool)>,
}

impl ParaStyle {
    fn over(&mut self, o: &ParaStyle) {
        macro_rules! take {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        take!(align, before, after, line, left, right, first, num, outline, page_break_before, contextual, shading);
        for t in &o.tabs {
            self.tabs.retain(|x| (x.0 - t.0).abs() > 0.5);
            if t.1 != TabKind::Clear {
                self.tabs.push(*t);
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Style {
    based: Option<String>,
    para: ParaStyle,
    run: RunStyle,
    borders: TableBorders,
    first_row: Option<(RunStyle, Option<String>)>,
    band: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct TableBorders {
    top: Option<Option<Border>>,
    bottom: Option<Option<Border>>,
    left: Option<Option<Border>>,
    right: Option<Option<Border>>,
    inside_h: Option<Option<Border>>,
    inside_v: Option<Option<Border>>,
}

impl TableBorders {
    fn over(&mut self, o: &TableBorders) {
        macro_rules! take {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        take!(top, bottom, left, right, inside_h, inside_v);
    }
}

/// One side of a cell's frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Border {
    pub width: f32,
    pub color: String,
}

#[derive(Debug, Clone)]
struct Level {
    fmt: String,
    text: String,
    start: i64,
    para: ParaStyle,
    run: RunStyle,
    tab_after: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
    pub header: f32,
    pub footer: f32,
}

/// Where a link goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    Url(String),
    Anchor(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text { text: String, style: RunStyle, link: Option<Link> },
    Image { index: usize, width: f32, height: f32, link: Option<Link> },
    Break,
    Tab,
    PageNumber(RunStyle),
    PageCount(RunStyle),
    Anchor(String),
    Footnote(Vec<Block>),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Para {
    pub heading: Option<u8>,
    pub align: Align,
    pub before: f32,
    pub after: f32,
    pub line: Option<Line>,
    pub left: f32,
    pub right: f32,
    pub first: f32,
    pub label: Option<(String, RunStyle)>,
    pub inlines: Vec<Inline>,
    pub mark: RunStyle,
    pub shading: Option<String>,
    style: String,
    contextual: bool,
    right_tab: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub col: usize,
    pub span: usize,
    pub rowspan: usize,
    pub blocks: Vec<Block>,
    pub fill: Option<String>,
    /// Top, right, bottom, left.
    pub borders: [Option<Border>; 4],
    pub valign: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub height: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub cols: Vec<f32>,
    pub rows: Vec<Row>,
    pub align: Align,
    pub indent: f32,
    pub inset: (f32, f32, f32, f32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Para(Para),
    Table(Table),
    PageBreak,
}

pub struct Image {
    pub bytes: Vec<u8>,
    pub kind: Option<&'static str>,
}

/// A Word document reduced to what the viewer draws.
pub struct Document {
    pub page: Page,
    pub blocks: Vec<Block>,
    pub header: Vec<Block>,
    pub footer: Vec<Block>,
    pub images: Vec<Image>,
    pub base: RunStyle,
}

struct Field {
    instr: String,
    result: bool,
    kind: FieldKind,
}

#[derive(Clone, PartialEq)]
enum FieldKind {
    Unknown,
    Page,
    Pages,
    Link(Link),
}

struct Parser {
    pkg: Package,
    styles: HashMap<String, Style>,
    default_para: Option<String>,
    doc_para: ParaStyle,
    doc_run: RunStyle,
    theme_major: Option<String>,
    theme_minor: Option<String>,
    nums: HashMap<String, (String, HashMap<u8, i64>)>,
    abstracts: HashMap<String, Vec<Level>>,
    counters: HashMap<String, Vec<i64>>,
    rels: HashMap<String, Rel>,
    part: String,
    images: Vec<Image>,
    image_index: HashMap<String, usize>,
    fields: Vec<Field>,
    footnotes: HashMap<String, El>,
    anchors: HashSet<String>,
    placed: HashSet<String>,
    table_style: Vec<String>,
}

fn twips(v: &str) -> Option<f32> {
    v.parse::<f32>().ok().map(|t| t / 20.)
}

fn emu(v: &str) -> Option<f32> {
    v.parse::<f64>().ok().map(|e| (e / 12700.) as f32)
}

fn highlight_hex(name: &str) -> Option<&'static str> {
    Some(match name {
        "yellow" => "ffff00",
        "green" => "00ff00",
        "cyan" => "00ffff",
        "magenta" => "ff00ff",
        "blue" => "0000ff",
        "red" => "ff0000",
        "darkBlue" => "000080",
        "darkCyan" => "008080",
        "darkGreen" => "008000",
        "darkMagenta" => "800080",
        "darkRed" => "800000",
        "darkYellow" => "808000",
        "darkGray" => "808080",
        "lightGray" => "c0c0c0",
        "black" => "000000",
        "white" => "ffffff",
        _ => return None,
    })
}

fn border(e: Option<&El>) -> Option<Option<Border>> {
    let e = e?;
    let val = e.attr("val").unwrap_or("single");
    if matches!(val, "nil" | "none") {
        return Some(None);
    }
    let width = e.attr("sz").and_then(|s| s.parse::<f32>().ok()).map_or(0.5, |s| (s / 8.).max(0.25));
    let color = e.attr("color").and_then(typ::hex).unwrap_or_else(|| "000000".into());
    Some(Some(Border { width: if val == "double" { width * 2.5 } else { width }, color }))
}

fn table_borders(e: Option<&El>) -> TableBorders {
    let Some(e) = e else { return TableBorders::default() };
    TableBorders {
        top: border(e.child("top")),
        bottom: border(e.child("bottom")),
        left: border(e.child("left").or_else(|| e.child("start"))),
        right: border(e.child("right").or_else(|| e.child("end"))),
        inside_h: border(e.child("insideH")),
        inside_v: border(e.child("insideV")),
    }
}

fn shading(e: Option<&El>) -> Option<String> {
    let e = e?;
    if e.attr("val") == Some("solid") {
        return e.attr("color").and_then(typ::hex);
    }
    e.attr("fill").and_then(typ::hex)
}

impl Parser {
    fn run_style(&self, rpr: Option<&El>) -> RunStyle {
        let Some(r) = rpr else { return RunStyle::default() };
        let underline = r.child("u").map(|u| !matches!(u.attr("val"), Some("none" | "0" | "false")));
        let fonts = r.child("rFonts");
        let font = fonts.and_then(|f| f.attr("ascii").or_else(|| f.attr("hAnsi")).map(str::to_string)).or_else(|| {
            let theme = fonts?.attr("asciiTheme").or_else(|| fonts?.attr("hAnsiTheme"))?;
            if theme.starts_with("major") { self.theme_major.clone() } else { self.theme_minor.clone() }
        });
        RunStyle {
            bold: r.flag("b"),
            italic: r.flag("i"),
            underline,
            strike: r.flag("strike").or_else(|| r.flag("dstrike")),
            size: r.val("sz").and_then(|v| v.parse::<f32>().ok()).map(|v| v / 2.),
            color: r.child("color").and_then(|c| c.attr("val")).map(|v| typ::hex(v).unwrap_or_default()),
            font,
            highlight: r.val("highlight").and_then(highlight_hex).map(str::to_string).or_else(|| shading(r.child("shd"))),
            script: r.val("vertAlign").map(|v| match v {
                "superscript" => 1,
                "subscript" => -1,
                _ => 0,
            }),
            caps: r.flag("caps"),
            small_caps: r.flag("smallCaps"),
            hidden: r.flag("vanish"),
        }
    }

    fn para_style(&self, ppr: Option<&El>) -> ParaStyle {
        let Some(p) = ppr else { return ParaStyle::default() };
        let spacing = p.child("spacing");
        let ind = p.child("ind");
        let line = spacing.and_then(|s| {
            let v = s.attr("line")?.parse::<f32>().ok()?;
            Some(match s.attr("lineRule").unwrap_or("auto") {
                "exact" => Line::Exact(v / 20.),
                "atLeast" => Line::AtLeast(v / 20.),
                _ => Line::Auto(v / 240.),
            })
        });
        let before = spacing.and_then(|s| if s.attr("beforeAutospacing").is_some_and(|v| v != "0") { Some(14.) } else { s.attr("before").and_then(twips) });
        let after = spacing.and_then(|s| if s.attr("afterAutospacing").is_some_and(|v| v != "0") { Some(14.) } else { s.attr("after").and_then(twips) });
        let first = ind.and_then(|i| i.attr("hanging").and_then(twips).map(|h| -h).or_else(|| i.attr("firstLine").and_then(twips)));
        let num = p.child("numPr").and_then(|n| {
            let id = n.val("numId")?.to_string();
            Some((id, n.val("ilvl").and_then(|v| v.parse().ok()).unwrap_or(0)))
        });
        let tabs = p
            .child("tabs")
            .map(|t| {
                t.all("tab")
                    .filter_map(|tab| {
                        let pos = tab.attr("pos").and_then(twips)?;
                        let kind = match tab.attr("val").unwrap_or("left") {
                            "right" | "end" => TabKind::Right,
                            "center" => TabKind::Center,
                            "clear" => TabKind::Clear,
                            _ => TabKind::Left,
                        };
                        Some((pos, kind, matches!(tab.attr("leader"), Some("dot" | "middleDot" | "hyphen" | "underscore"))))
                    })
                    .collect()
            })
            .unwrap_or_default();
        ParaStyle {
            align: p.val("jc").map(|v| match v {
                "center" => Align::Center,
                "right" | "end" => Align::Right,
                "both" | "distribute" | "lowKashida" | "mediumKashida" | "highKashida" => Align::Justify,
                _ => Align::Left,
            }),
            before,
            after,
            line,
            left: ind.and_then(|i| i.attr("left").or_else(|| i.attr("start")).and_then(twips)),
            right: ind.and_then(|i| i.attr("right").or_else(|| i.attr("end")).and_then(twips)),
            first,
            num,
            outline: p.val("outlineLvl").and_then(|v| v.parse().ok()),
            page_break_before: p.flag("pageBreakBefore"),
            contextual: p.flag("contextualSpacing"),
            shading: shading(p.child("shd")),
            tabs,
        }
    }

    fn load_theme(&mut self) {
        let Some(theme) = self.pkg.xml("word/theme/theme1.xml") else { return };
        let face = |name: &str| theme.find(name).and_then(|f| f.child("latin")).and_then(|l| l.attr("typeface")).filter(|f| !f.is_empty()).map(str::to_string);
        self.theme_major = face("majorFont");
        self.theme_minor = face("minorFont");
    }

    fn load_styles(&mut self) {
        let Some(root) = self.pkg.xml("word/styles.xml") else { return };
        if let Some(d) = root.child("docDefaults") {
            self.doc_run = self.run_style(d.path(&["rPrDefault", "rPr"]));
            self.doc_para = self.para_style(d.path(&["pPrDefault", "pPr"]));
        }
        for s in root.all("style") {
            let Some(id) = s.attr("styleId") else { continue };
            let kind = s.attr("type").unwrap_or("paragraph");
            if kind == "paragraph" && s.attr("default").is_some_and(|v| v == "1" || v == "true") {
                self.default_para = Some(id.to_string());
            }
            let first_row = s.all("tblStylePr").find(|t| t.attr("type") == Some("firstRow")).map(|t| (self.run_style(t.child("rPr")), shading(t.path(&["tcPr", "shd"]))));
            let band = s.all("tblStylePr").find(|t| t.attr("type") == Some("band1Horz")).and_then(|t| shading(t.path(&["tcPr", "shd"])));
            let style = Style {
                based: s.val("basedOn").map(str::to_string),
                para: self.para_style(s.child("pPr")),
                run: self.run_style(s.child("rPr")),
                borders: table_borders(s.path(&["tblPr", "tblBorders"])),
                first_row,
                band,
            };
            self.styles.insert(id.to_string(), style);
        }
    }

    fn chain(&self, id: &str) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut at = Some(id.to_string());
        let mut seen = HashSet::new();
        while let Some(i) = at {
            if !seen.insert(i.clone()) || out.len() > 24 {
                break;
            }
            let Some(s) = self.styles.get(&i) else { break };
            out.push(s);
            at = s.based.clone();
        }
        out.reverse();
        out
    }

    fn load_numbering(&mut self) {
        let Some(root) = self.pkg.xml("word/numbering.xml") else { return };
        for a in root.all("abstractNum") {
            let Some(id) = a.attr("abstractNumId") else { continue };
            let mut levels = Vec::new();
            for l in a.all("lvl") {
                let level = Level {
                    fmt: l.val("numFmt").unwrap_or("decimal").to_string(),
                    text: l.val("lvlText").unwrap_or("").to_string(),
                    start: l.val("start").and_then(|v| v.parse().ok()).unwrap_or(1),
                    para: self.para_style(l.child("pPr")),
                    run: self.run_style(l.child("rPr")),
                    tab_after: l.val("suff").is_none_or(|v| v == "tab"),
                };
                levels.push(level);
            }
            self.abstracts.insert(id.to_string(), levels);
        }
        for n in root.all("num") {
            let (Some(id), Some(abs)) = (n.attr("numId"), n.val("abstractNumId")) else { continue };
            let starts = n.all("lvlOverride").filter_map(|o| Some((o.attr("ilvl")?.parse().ok()?, o.val("startOverride")?.parse().ok()?))).collect();
            self.nums.insert(id.to_string(), (abs.to_string(), starts));
        }
    }

    fn level(&self, num: &str, ilvl: u8) -> Option<&Level> {
        let (abs, _) = self.nums.get(num)?;
        self.abstracts.get(abs)?.get(ilvl as usize)
    }

    fn label(&mut self, num: &str, ilvl: u8) -> Option<(String, RunStyle, bool)> {
        let (abs, starts) = self.nums.get(num)?.clone();
        let levels = self.abstracts.get(&abs)?.clone();
        let lvl = levels.get(ilvl as usize)?;
        if lvl.fmt == "none" {
            return None;
        }
        let counters = self.counters.entry(num.to_string()).or_insert_with(|| levels.iter().enumerate().map(|(i, l)| starts.get(&(i as u8)).copied().unwrap_or(l.start) - 1).collect());
        let i = ilvl as usize;
        if i >= counters.len() {
            return None;
        }
        counters[i] += 1;
        for (deeper, c) in counters.iter_mut().enumerate().skip(i + 1) {
            *c = starts.get(&(deeper as u8)).copied().unwrap_or(levels.get(deeper).map_or(1, |l| l.start)) - 1;
        }
        if lvl.fmt == "bullet" {
            let font = lvl.run.font.clone();
            let text: String = lvl.text.chars().map(|c| typ::symbol_char(c, font.as_deref())).collect();
            let mut run = lvl.run.clone();
            run.font = None;
            return Some((if text.is_empty() { "•".into() } else { text }, run, lvl.tab_after));
        }
        let mut text = lvl.text.clone();
        for (n, value) in counters.iter().enumerate().take(i + 1).rev() {
            let fmt = levels.get(n).map_or("decimal", |l| l.fmt.as_str());
            text = text.replace(&format!("%{}", n + 1), &number(*value.max(&1), fmt));
        }
        Some((text, lvl.run.clone(), lvl.tab_after))
    }

    fn image(&mut self, rid: &str) -> Option<usize> {
        let rel = self.rels.get(rid)?.clone();
        if rel.external {
            return None;
        }
        let key = format!("{}#{}", self.part, rel.target);
        if let Some(i) = self.image_index.get(&key) {
            return Some(*i);
        }
        let bytes = self.pkg.read(&rel.target)?;
        let kind = package::image_kind(&bytes);
        self.images.push(Image { bytes, kind });
        let i = self.images.len() - 1;
        self.image_index.insert(key, i);
        Some(i)
    }

    fn link_of(&self, e: &El) -> Option<Link> {
        if let Some(rid) = e.attr("r:id").or_else(|| e.attr("id")) {
            return self.rels.get(rid).map(|r| Link::Url(r.target.clone()));
        }
        e.attr("anchor").filter(|a| self.anchors.contains(*a)).map(|a| Link::Anchor(a.to_string()))
    }

    fn resolved_para(&self, ppr: Option<&El>) -> (ParaStyle, RunStyle, String) {
        let id = ppr.and_then(|p| p.val("pStyle")).map(str::to_string).or_else(|| self.default_para.clone()).unwrap_or_default();
        let mut para = self.doc_para.clone();
        let mut run = self.doc_run.clone();
        let is_default = self.default_para.as_deref() == Some(id.as_str());
        let table = self.table_style.last().map(|t| self.chain(t)).unwrap_or_default();
        if !is_default {
            for s in &table {
                para.over(&s.para);
                run.over(&s.run);
            }
        }
        for s in self.chain(&id) {
            para.over(&s.para);
            run.over(&s.run);
        }
        if is_default {
            for s in &table {
                para.over(&s.para);
                run.over(&s.run);
            }
        }
        let direct = self.para_style(ppr);
        if let Some((num, ilvl)) = direct.num.clone().or_else(|| para.num.clone())
            && let Some(level) = self.level(&num, ilvl)
        {
            para.over(&ParaStyle { left: level.para.left, first: level.para.first, tabs: level.para.tabs.clone(), ..Default::default() });
        }
        para.over(&direct);
        (para, run, id)
    }

    fn paragraph(&mut self, p: &El, out: &mut Vec<Block>) {
        let ppr = p.child("pPr");
        let (style, base_run, id) = self.resolved_para(ppr);
        let mark = {
            let mut m = base_run.clone();
            m.over(&self.run_style(ppr.and_then(|p| p.child("rPr"))));
            m
        };
        let mut para = Para {
            heading: style.outline.filter(|l| *l < 9).map(|l| l + 1),
            align: style.align.unwrap_or_default(),
            before: style.before.unwrap_or(0.),
            after: style.after.unwrap_or(0.),
            line: style.line,
            left: style.left.unwrap_or(0.),
            right: style.right.unwrap_or(0.),
            first: style.first.unwrap_or(0.),
            mark,
            shading: style.shading.clone(),
            style: id,
            contextual: style.contextual.unwrap_or(false),
            right_tab: style.tabs.iter().filter(|t| t.1 == TabKind::Right).max_by(|a, b| a.0.total_cmp(&b.0)).map(|t| t.2),
            ..Default::default()
        };
        if style.page_break_before == Some(true) && !out.is_empty() {
            out.push(Block::PageBreak);
        }
        if let Some((num, ilvl)) = style.num.clone()
            && num != "0"
            && let Some((text, run, tab)) = self.label(&num, ilvl)
        {
            let mut r = para.mark.clone();
            r.underline = None;
            r.over(&run);
            let text = if tab || para.first < 0. { text } else { format!("{text} ") };
            para.label = Some((text, r));
        }
        let mut extra = Vec::new();
        self.inlines(p, &base_run, None, &mut para, out, &mut extra);
        out.push(Block::Para(para));
        out.extend(extra);
    }

    fn split(&self, para: &mut Para, out: &mut Vec<Block>) {
        let mut done = std::mem::take(para);
        let rest = Para { label: None, before: 0., inlines: Vec::new(), ..done.clone() };
        done.after = 0.;
        out.push(Block::Para(done));
        out.push(Block::PageBreak);
        *para = rest;
    }

    fn hidden_by_field(&self) -> bool {
        self.fields.iter().any(|f| !f.result || matches!(f.kind, FieldKind::Page | FieldKind::Pages))
    }

    fn field_link(&self) -> Option<Link> {
        self.fields.iter().rev().find_map(|f| match &f.kind {
            FieldKind::Link(Link::Anchor(a)) if !self.anchors.contains(a) => None,
            FieldKind::Link(l) => Some(l.clone()),
            _ => None,
        })
    }

    fn inlines(&mut self, parent: &El, base: &RunStyle, link: Option<&Link>, para: &mut Para, out: &mut Vec<Block>, extra: &mut Vec<Block>) {
        for e in parent.els() {
            match e.name.as_str() {
                "r" => self.run(e, base, link, para, out, extra),
                "hyperlink" => {
                    let l = self.link_of(e);
                    self.inlines(e, base, l.as_ref().or(link), para, out, extra);
                }
                "fldSimple" => {
                    let instr = e.attr("instr").unwrap_or("").trim().to_uppercase();
                    let mut style = base.clone();
                    if let Some(r) = e.child("r") {
                        style.over(&self.run_char_style(r));
                    }
                    if instr.starts_with("PAGE") && !instr.starts_with("PAGEREF") {
                        para.inlines.push(Inline::PageNumber(style));
                    } else if instr.starts_with("NUMPAGES") || instr.starts_with("SECTIONPAGES") {
                        para.inlines.push(Inline::PageCount(style));
                    } else {
                        self.inlines(e, base, link, para, out, extra);
                    }
                }
                "ins" | "smartTag" | "customXml" | "dir" | "bdo" => self.inlines(e, base, link, para, out, extra),
                "sdt" => {
                    if let Some(c) = e.child("sdtContent") {
                        self.inlines(c, base, link, para, out, extra);
                    }
                }
                "bookmarkStart" => {
                    if let Some(name) = e.attr("name").filter(|n| self.anchors.contains(*n) && self.placed.insert(n.to_string())) {
                        para.inlines.push(Inline::Anchor(name.to_string()));
                    }
                }
                "oMath" | "oMathPara" => {
                    let text: String = e.descendants().into_iter().filter(|d| d.name == "t").map(El::text).collect();
                    para.inlines.push(Inline::Text { text, style: RunStyle { italic: Some(true), ..base.clone() }, link: None });
                }
                _ => {}
            }
        }
    }

    fn run_char_style(&self, r: &El) -> RunStyle {
        let rpr = r.child("rPr");
        let mut style = RunStyle::default();
        if let Some(id) = rpr.and_then(|p| p.val("rStyle")) {
            for s in self.chain(id) {
                style.over(&s.run);
            }
        }
        style.over(&self.run_style(rpr));
        style
    }

    fn run(&mut self, r: &El, base: &RunStyle, link: Option<&Link>, para: &mut Para, out: &mut Vec<Block>, extra: &mut Vec<Block>) {
        let mut style = base.clone();
        style.over(&self.run_char_style(r));
        if style.hidden == Some(true) {
            return;
        }
        for c in r.els() {
            match c.name.as_str() {
                "fldChar" => match c.attr("fldCharType") {
                    Some("begin") => self.fields.push(Field { instr: String::new(), result: false, kind: FieldKind::Unknown }),
                    Some("separate") => {
                        if let Some(f) = self.fields.last_mut() {
                            f.result = true;
                            f.kind = field_kind(&f.instr);
                            match f.kind {
                                FieldKind::Page => para.inlines.push(Inline::PageNumber(style.clone())),
                                FieldKind::Pages => para.inlines.push(Inline::PageCount(style.clone())),
                                _ => {}
                            }
                        }
                    }
                    Some("end") => {
                        if let Some(f) = self.fields.pop()
                            && !f.result
                        {
                            match field_kind(&f.instr) {
                                FieldKind::Page => para.inlines.push(Inline::PageNumber(style.clone())),
                                FieldKind::Pages => para.inlines.push(Inline::PageCount(style.clone())),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                },
                "instrText" => {
                    if let Some(f) = self.fields.last_mut()
                        && !f.result
                    {
                        f.instr.push_str(&c.text());
                    }
                }
                _ if self.hidden_by_field() => {}
                "t" => {
                    let text = c.text();
                    let link = link.cloned().or_else(|| self.field_link());
                    if let Some(Inline::Text { text: last, style: s, link: l }) = para.inlines.last_mut()
                        && *s == style
                        && *l == link
                    {
                        last.push_str(&text);
                    } else {
                        para.inlines.push(Inline::Text { text, style: style.clone(), link });
                    }
                }
                "tab" | "ptab" => para.inlines.push(Inline::Tab),
                "br" => match c.attr("type") {
                    Some("page") => self.split(para, out),
                    _ => para.inlines.push(Inline::Break),
                },
                "cr" => para.inlines.push(Inline::Break),
                "noBreakHyphen" => para.inlines.push(Inline::Text { text: "\u{2011}".into(), style: style.clone(), link: link.cloned() }),
                "sym" => {
                    let ch = c.attr("char").and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32).map(|ch| typ::symbol_char(ch, c.attr("font")));
                    if let Some(ch) = ch {
                        para.inlines.push(Inline::Text { text: ch.to_string(), style: style.clone(), link: link.cloned() });
                    }
                }
                "drawing" => self.drawing(c, link, para, extra),
                "pict" | "object" => {
                    if let Some(img) = c.find("imagedata")
                        && let Some(rid) = img.attr("r:id")
                        && let Some(index) = self.image(rid)
                    {
                        let shape = c.find("shape");
                        let css = shape.and_then(|s| s.attr("style")).unwrap_or("");
                        let (w, h) = (css_length(css, "width").unwrap_or(120.), css_length(css, "height").unwrap_or(90.));
                        para.inlines.push(Inline::Image { index, width: w, height: h, link: link.cloned() });
                    }
                }
                "footnoteReference" => {
                    if let Some(note) = c.attr("id").and_then(|id| self.footnotes.get(id).cloned()) {
                        let mut blocks = Vec::new();
                        for p in note.all("p") {
                            self.paragraph(p, &mut blocks);
                        }
                        para.inlines.push(Inline::Footnote(blocks));
                    }
                }
                _ => {}
            }
        }
    }

    fn drawing(&mut self, d: &El, link: Option<&Link>, para: &mut Para, extra: &mut Vec<Block>) {
        let Some(frame) = d.child("inline").or_else(|| d.child("anchor")) else { return };
        let extent = frame.child("extent");
        let w = extent.and_then(|e| e.attr("cx")).and_then(emu).unwrap_or(100.);
        let h = extent.and_then(|e| e.attr("cy")).and_then(emu).unwrap_or(100.);
        if let Some(blip) = frame.find("blip")
            && let Some(rid) = blip.attr("r:embed")
            && let Some(index) = self.image(rid)
        {
            para.inlines.push(Inline::Image { index, width: w, height: h, link: link.cloned() });
            return;
        }
        if let Some(text) = frame.find("txbxContent") {
            for p in text.all("p") {
                self.paragraph(p, extra);
            }
        }
    }

    fn table(&mut self, t: &El, out: &mut Vec<Block>) {
        let tpr = t.child("tblPr");
        let style_id = tpr.and_then(|p| p.val("tblStyle")).map(str::to_string);
        let mut borders = TableBorders::default();
        let mut first_row = None;
        let mut band = None;
        if let Some(id) = &style_id {
            for s in self.chain(id) {
                borders.over(&s.borders);
                first_row = s.first_row.clone().or(first_row);
                band = s.band.clone().or(band);
            }
        }
        borders.over(&table_borders(tpr.and_then(|p| p.child("tblBorders"))));
        let look = tpr.and_then(|p| p.child("tblLook"));
        let look_bit = |name: &str, bit: u32| look.is_some_and(|l| l.attr(name).map(|v| v == "1" || v == "true").unwrap_or_else(|| l.attr("val").and_then(|v| u32::from_str_radix(v, 16).ok()).is_some_and(|v| v & bit != 0)));
        let (use_first, no_band) = (look.is_none() || look_bit("firstRow", 0x20), look_bit("noHBand", 0x200));
        let margins = tpr.and_then(|p| p.child("tblCellMar"));
        let margin = |side: &str, alt: &str, default: f32| margins.and_then(|m| m.child(side).or_else(|| m.child(alt))).and_then(|e| e.attr("w")).and_then(twips).unwrap_or(default);
        let inset = (margin("left", "start", 5.4), margin("right", "end", 5.4), margin("top", "top", 0.), margin("bottom", "bottom", 0.));
        let mut cols: Vec<f32> = t.child("tblGrid").map(|g| g.all("gridCol").map(|c| c.attr("w").and_then(twips).unwrap_or(0.)).collect()).unwrap_or_default();
        let align = match tpr.and_then(|p| p.val("jc")) {
            Some("center") => Align::Center,
            Some("right" | "end") => Align::Right,
            _ => Align::Left,
        };
        let indent = tpr.and_then(|p| p.child("tblInd")).and_then(|i| i.attr("w")).and_then(twips).unwrap_or(0.);
        if let Some(id) = &style_id {
            self.table_style.push(id.clone());
        }
        let mut rows = Vec::new();
        let mut open: HashMap<usize, (usize, usize)> = HashMap::new();
        let trs: Vec<&El> = t.els().flat_map(|e| if e.name == "sdt" { e.child("sdtContent").map(|c| c.all("tr").collect::<Vec<_>>()).unwrap_or_default() } else if e.name == "tr" { vec![e] } else { vec![] }).collect();
        let row_count = trs.len();
        for (ri, tr) in trs.into_iter().enumerate() {
            let trpr = tr.child("trPr");
            let mut col = trpr.and_then(|p| p.val("gridBefore")).and_then(|v| v.parse().ok()).unwrap_or(0usize);
            let height = trpr.and_then(|p| p.child("trHeight")).and_then(|h| h.attr("val")).and_then(twips).filter(|h| *h > 0.);
            let mut cells = Vec::new();
            for tc in tr.els().flat_map(|e| if e.name == "sdt" { e.child("sdtContent").map(|c| c.all("tc").cloned().collect::<Vec<_>>()).unwrap_or_default() } else if e.name == "tc" { vec![e.clone()] } else { vec![] }) {
                let tcpr = tc.child("tcPr");
                let span = tcpr.and_then(|p| p.val("gridSpan")).and_then(|v| v.parse().ok()).unwrap_or(1usize).max(1);
                let vmerge = tcpr.and_then(|p| p.child("vMerge")).map(|m| m.attr("val").unwrap_or("continue").to_string());
                if vmerge.as_deref() == Some("continue") {
                    if let Some((r, c)) = open.get(&col).copied()
                        && let Some(Row { cells, .. }) = rows.get_mut(r)
                        && let Some(cell) = cells.get_mut(c)
                    {
                        let cell: &mut Cell = cell;
                        cell.rowspan += 1;
                    }
                    col += span;
                    continue;
                }
                let mut blocks = Vec::new();
                for e in tc.els() {
                    match e.name.as_str() {
                        "p" => self.paragraph(e, &mut blocks),
                        "tbl" => self.table(e, &mut blocks),
                        "sdt" => {
                            if let Some(c) = e.child("sdtContent") {
                                for p in c.all("p") {
                                    self.paragraph(p, &mut blocks);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                blocks.retain(|b| *b != Block::PageBreak);
                let own = table_borders(tcpr.and_then(|p| p.child("tcBorders")));
                let last_col = col + span >= cols.len().max(1);
                let pick = |own: &Option<Option<Border>>, outer: &Option<Option<Border>>, inner: &Option<Option<Border>>, edge: bool| own.clone().unwrap_or_else(|| if edge { outer.clone().flatten() } else { inner.clone().flatten() });
                let cell_borders = [
                    pick(&own.top, &borders.top, &borders.inside_h, ri == 0),
                    pick(&own.right, &borders.right, &borders.inside_v, last_col),
                    pick(&own.bottom, &borders.bottom, &borders.inside_h, ri + 1 == row_count),
                    pick(&own.left, &borders.left, &borders.inside_v, col == 0),
                ];
                let mut fill = shading(tcpr.and_then(|p| p.child("shd")));
                if ri == 0 && use_first
                    && let Some((run, shd)) = &first_row
                {
                    fill = fill.or_else(|| shd.clone());
                    for b in &mut blocks {
                        restyle(b, run);
                    }
                } else if !no_band && (ri % 2 == if use_first && first_row.is_some() { 1 } else { 0 }) {
                    fill = fill.or_else(|| band.clone());
                }
                let valign = match tcpr.and_then(|p| p.val("vAlign")) {
                    Some("center") => "horizon",
                    Some("bottom") => "bottom",
                    _ => "top",
                };
                if vmerge.is_some() {
                    open.insert(col, (rows.len(), cells.len()));
                } else {
                    open.remove(&col);
                }
                cells.push(Cell { col, span, rowspan: 1, blocks, fill, borders: cell_borders, valign });
                col += span;
            }
            if col > cols.len() {
                let fill = cols.iter().copied().sum::<f32>() / cols.len().max(1) as f32;
                cols.resize(col, if fill > 0. { fill } else { 72. });
            }
            rows.push(Row { cells, height });
        }
        if style_id.is_some() {
            self.table_style.pop();
        }
        if cols.is_empty() {
            return;
        }
        out.push(Block::Table(Table { cols, rows, align, indent, inset }));
    }

    fn blocks(&mut self, parent: &El, out: &mut Vec<Block>) {
        for e in parent.els() {
            match e.name.as_str() {
                "p" => self.paragraph(e, out),
                "tbl" => self.table(e, out),
                "sdt" => {
                    if let Some(c) = e.child("sdtContent") {
                        self.blocks(c, out);
                    }
                }
                "customXml" | "ins" => self.blocks(e, out),
                _ => {}
            }
        }
    }

    fn part_blocks(&mut self, part: &str) -> Vec<Block> {
        let Some(root) = self.pkg.xml(part) else { return Vec::new() };
        let (saved_rels, saved_part) = (std::mem::replace(&mut self.rels, self.pkg.rels(part)), std::mem::replace(&mut self.part, part.to_string()));
        let mut out = Vec::new();
        self.blocks(&root, &mut out);
        self.rels = saved_rels;
        self.part = saved_part;
        out.retain(|b| *b != Block::PageBreak);
        out
    }
}

fn restyle(b: &mut Block, run: &RunStyle) {
    match b {
        Block::Para(p) => {
            for i in &mut p.inlines {
                if let Inline::Text { style, .. } = i {
                    let mut merged = run.clone();
                    merged.over(style);
                    *style = merged;
                }
            }
        }
        Block::Table(_) | Block::PageBreak => {}
    }
}

fn css_length(css: &str, key: &str) -> Option<f32> {
    let part = css.split(';').find_map(|p| p.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(key)).map(|(_, v)| v.trim().to_string()))?;
    let num: String = part.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
    let v: f32 = num.parse().ok()?;
    let unit = part[num.len()..].trim();
    Some(match unit {
        "in" => v * 72.,
        "cm" => v * 72. / 2.54,
        "mm" => v * 72. / 25.4,
        "px" => v * 0.75,
        _ => v,
    })
}

fn field_kind(instr: &str) -> FieldKind {
    let i = instr.trim();
    let head = i.split_whitespace().next().unwrap_or("").to_uppercase();
    match head.as_str() {
        "PAGE" => FieldKind::Page,
        "NUMPAGES" | "SECTIONPAGES" => FieldKind::Pages,
        "HYPERLINK" => {
            let quoted = i.split('"').nth(1).unwrap_or("").to_string();
            if i.contains("\\l") { FieldKind::Link(Link::Anchor(quoted)) } else if quoted.is_empty() { FieldKind::Unknown } else { FieldKind::Link(Link::Url(quoted)) }
        }
        _ => FieldKind::Unknown,
    }
}

fn roman(mut n: i64) -> String {
    let table = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut out = String::new();
    for (v, s) in table {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

fn letters(n: i64) -> String {
    let n = (n - 1).max(0);
    let c = (b'a' + (n % 26) as u8) as char;
    c.to_string().repeat((n / 26 + 1) as usize)
}

/// A list counter in a Word number format.
fn number(n: i64, fmt: &str) -> String {
    match fmt {
        "lowerLetter" => letters(n),
        "upperLetter" => letters(n).to_uppercase(),
        "lowerRoman" => roman(n),
        "upperRoman" => roman(n).to_uppercase(),
        "decimalZero" => format!("{n:02}"),
        _ => n.to_string(),
    }
}

/// Reads a Word file. Fails only when the package or its main part cannot be read.
pub fn parse(bytes: Vec<u8>) -> Result<Document, OpenError> {
    let mut pkg = Package::from_bytes(bytes)?;
    let main = pkg.rels("").values().find(|r| r.kind == "officeDocument").map(|r| r.target.clone()).unwrap_or_else(|| "word/document.xml".into());
    let root = pkg.xml(&main).ok_or(OpenError::Broken)?;
    let body = root.child("body").ok_or(OpenError::Broken)?;
    let rels = pkg.rels(&main);
    let footnotes = pkg.xml("word/footnotes.xml").map(|f| f.all("footnote").filter(|n| n.attr("type").is_none_or(|t| t == "normal")).filter_map(|n| Some((n.attr("id")?.to_string(), n.clone()))).collect()).unwrap_or_default();
    let mut parser = Parser {
        pkg,
        styles: HashMap::new(),
        default_para: None,
        doc_para: ParaStyle::default(),
        doc_run: RunStyle::default(),
        theme_major: None,
        theme_minor: None,
        nums: HashMap::new(),
        abstracts: HashMap::new(),
        counters: HashMap::new(),
        rels,
        part: main.clone(),
        images: Vec::new(),
        image_index: HashMap::new(),
        fields: Vec::new(),
        footnotes,
        anchors: HashSet::new(),
        placed: HashSet::new(),
        table_style: Vec::new(),
    };
    parser.load_theme();
    parser.load_styles();
    parser.load_numbering();
    let links: HashSet<String> = body.descendants().into_iter().filter(|e| e.name == "hyperlink").filter_map(|e| e.attr("anchor").map(str::to_string)).collect();
    parser.anchors = body.descendants().into_iter().filter(|e| e.name == "bookmarkStart").filter_map(|e| e.attr("name").map(str::to_string)).filter(|n| links.contains(n)).collect();
    let sect = body.child("sectPr").or_else(|| body.descendants().into_iter().filter(|e| e.name == "sectPr").last());
    let size = sect.and_then(|s| s.child("pgSz"));
    let mar = sect.and_then(|s| s.child("pgMar"));
    let m = |name: &str, default: f32| mar.and_then(|m| m.attr(name)).and_then(twips).unwrap_or(default).abs();
    let mut page = Page {
        width: size.and_then(|s| s.attr("w")).and_then(twips).unwrap_or(595.3),
        height: size.and_then(|s| s.attr("h")).and_then(twips).unwrap_or(841.9),
        top: m("top", 72.),
        bottom: m("bottom", 72.),
        left: m("left", 72.),
        right: m("right", 72.),
        header: m("header", 36.),
        footer: m("footer", 36.),
    };
    if page.width < 72. || page.height < 72. {
        page.width = 595.3;
        page.height = 841.9;
    }
    let reference = |kind: &str| sect.and_then(|s| s.all(kind).find(|h| h.attr("type").is_none_or(|t| t == "default")).and_then(|h| h.attr("r:id")).map(str::to_string));
    let (header_id, footer_id) = (reference("headerReference"), reference("footerReference"));
    let mut blocks = Vec::new();
    parser.blocks(body, &mut blocks);
    let target = |id: Option<String>, parser: &Parser| id.and_then(|i| parser.rels.get(&i).map(|r| r.target.clone()));
    let (header_part, footer_part) = (target(header_id, &parser), target(footer_id, &parser));
    parser.anchors.clear();
    let header = header_part.map(|p| parser.part_blocks(&p)).unwrap_or_default();
    let footer = footer_part.map(|p| parser.part_blocks(&p)).unwrap_or_default();
    while matches!(blocks.last(), Some(Block::PageBreak)) {
        blocks.pop();
    }
    collapse_contextual(&mut blocks);
    Ok(Document { page, blocks, header, footer, images: parser.images, base: parser.doc_run })
}

fn collapse_contextual(blocks: &mut [Block]) {
    for i in 1..blocks.len() {
        let (a, b) = blocks.split_at_mut(i);
        if let (Some(Block::Para(prev)), Some(Block::Para(next))) = (a.last_mut(), b.first_mut())
            && prev.style == next.style
        {
            if prev.contextual {
                prev.after = 0.;
            }
            if next.contextual {
                next.before = 0.;
            }
        }
    }
}

struct Writer<'a> {
    doc: &'a Document,
    out: String,
    base_size: f32,
    base_font: Option<String>,
}

fn label_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' }).collect();
    format!("bm-{clean}")
}

impl Writer<'_> {
    fn text_args(&self, style: &RunStyle, size_scale: f32) -> String {
        let mut a = String::new();
        if let Some(f) = style.font.as_ref().filter(|f| Some(*f) != self.base_font.as_ref()) {
            let _ = write!(a, "font: ({}, {}), ", typ::s(f), typ::FALLBACK);
        }
        let size = style.size.unwrap_or(self.base_size) * size_scale;
        if (size - self.base_size).abs() > 0.01 {
            let _ = write!(a, "size: {}, ", typ::pt(size));
        }
        if let Some(c) = style.color.as_ref().filter(|c| !c.is_empty() && *c != "000000") {
            let _ = write!(a, "fill: {}, ", typ::rgb(c, 1.));
        }
        if style.bold == Some(true) {
            a.push_str("weight: \"bold\", ");
        }
        if style.italic == Some(true) {
            a.push_str("style: \"italic\", ");
        }
        a
    }

    fn wrap(&self, mut expr: String, style: &RunStyle, link: Option<&Link>) -> String {
        if style.small_caps == Some(true) {
            expr = format!("smallcaps({expr})");
        }
        if style.underline == Some(true) {
            expr = format!("underline({expr})");
        }
        if style.strike == Some(true) {
            expr = format!("strike({expr})");
        }
        match style.script {
            Some(1) => expr = format!("super({expr})"),
            Some(-1) => expr = format!("sub({expr})"),
            _ => {}
        }
        if let Some(h) = &style.highlight {
            expr = format!("highlight(fill: {}, {expr})", typ::rgb(h, 1.));
        }
        match link {
            Some(Link::Url(u)) if !u.is_empty() => expr = format!("link({}, {expr})", typ::s(u)),
            Some(Link::Anchor(a)) => expr = format!("link(label({}), {expr})", typ::s(&label_name(a))),
            _ => {}
        }
        expr
    }

    fn inline(&mut self, i: &Inline, para: &Para, out: &mut String) {
        match i {
            Inline::Text { text, style, link } => {
                if text.is_empty() {
                    return;
                }
                let shown = if style.caps == Some(true) { text.to_uppercase() } else { text.clone() };
                let expr = format!("text({}{})", self.text_args(style, 1.), typ::s(&shown));
                let _ = write!(out, "#{}", self.wrap(expr, style, link.as_ref()));
            }
            Inline::Image { index, width, height, link } => {
                let img = &self.doc.images[*index];
                let expr = match img.kind {
                    Some(kind) => format!("box(image({}, width: {}, height: {}, fit: \"stretch\"))", typ::s(&format!("/img/{index}.{kind}")), typ::pt(*width), typ::pt(*height)),
                    None => format!("box(office-missing(\"\", width: {}, height: {}))", typ::pt(*width), typ::pt(*height)),
                };
                let expr = match link {
                    Some(Link::Url(u)) => format!("link({}, {expr})", typ::s(u)),
                    _ => expr,
                };
                let _ = write!(out, "#{expr}");
            }
            Inline::Break => out.push_str("#linebreak()"),
            Inline::Tab => match para.right_tab {
                Some(true) => out.push_str("#box(width: 1fr, repeat(\" . \"))"),
                Some(false) => out.push_str("#h(1fr)"),
                None => out.push_str("#h(2.2em)"),
            },
            Inline::PageNumber(style) => {
                let _ = write!(out, "#context text({}counter(page).display())", self.text_args(style, 1.));
            }
            Inline::PageCount(style) => {
                let _ = write!(out, "#context text({}str(counter(page).final().first()))", self.text_args(style, 1.));
            }
            Inline::Anchor(name) => {
                let _ = write!(out, "#[#metadata(none)#label({})]", typ::s(&label_name(name)));
            }
            Inline::Footnote(blocks) => {
                let mut body = String::new();
                for b in blocks {
                    if let Block::Para(p) = b {
                        for i in &p.inlines {
                            self.inline(i, p, &mut body);
                        }
                    }
                }
                let _ = write!(out, "#footnote[{body}]");
            }
        }
    }

    fn para(&mut self, p: &Para, in_cell: bool) {
        let size = p.mark.size.unwrap_or(self.base_size);
        let leading = match p.line {
            Some(Line::Auto(m)) => format!("{:.3}em", (m * 1.17 - 1.).max(0.05)),
            Some(Line::Exact(v) | Line::AtLeast(v)) => format!("{}", typ::pt((v - size).max(1.))),
            None => "0.17em".into(),
        };
        let mut content = String::new();
        if let Some((label, style)) = &p.label {
            let expr = format!("text({}{})", self.text_args(style, 1.), typ::s(label));
            let expr = self.wrap(expr, &RunStyle { bold: None, ..style.clone() }, None);
            if p.first < 0. {
                let _ = write!(content, "#box(width: {})[#{expr}]", typ::pt(-p.first));
            } else {
                let _ = write!(content, "#{expr}");
            }
        }
        let visible = p.inlines.iter().any(|i| !matches!(i, Inline::Anchor(_)));
        for i in &p.inlines {
            self.inline(i, p, &mut content);
        }
        if !visible && p.label.is_none() {
            let _ = write!(content, "#text({}\"\u{200B}\")", self.text_args(&p.mark, 1.));
        }
        let body = match p.heading {
            Some(level) => format!("#heading(level: {level}, outlined: true, bookmarked: true)[{content}]"),
            None => content,
        };
        let (pad_left, hanging, first) = if p.first < 0. { (p.left + p.first, -p.first, 0.) } else { (p.left, 0., p.first) };
        let _ = writeln!(self.out, "#v({}, weak: true)", typ::pt(p.before));
        let _ = write!(self.out, "#[#set par(leading: {leading}, justify: {}, hanging-indent: {}, first-line-indent: (amount: {}, all: true))\n#set align({})\n", p.align == Align::Justify, typ::pt(hanging), typ::pt(first), p.align.typst());
        let padded = if pad_left.abs() > 0.01 || p.right.abs() > 0.01 { format!("#pad(left: {}, right: {})[{body}]", typ::pt(pad_left.max(-36.)), typ::pt(p.right.max(0.))) } else { body };
        match &p.shading {
            Some(fill) if !in_cell => {
                let _ = write!(self.out, "#block(width: 100%, fill: {}, outset: (x: 2pt), spacing: 0pt)[{padded}]", typ::rgb(fill, 1.));
            }
            _ => self.out.push_str(&padded),
        }
        let _ = writeln!(self.out, "]\n#v({}, weak: true)", typ::pt(p.after));
    }

    fn stroke(b: &Option<Border>) -> String {
        match b {
            Some(b) => format!("{} + {}", typ::pt(b.width), typ::rgb(&b.color, 1.)),
            None => "none".into(),
        }
    }

    fn table(&mut self, t: &Table, width: f32) {
        let total: f32 = t.cols.iter().sum();
        let scale = if total > width * 1.02 && total > 0. { width / total } else { 1. };
        let cols: Vec<String> = t.cols.iter().map(|c| if *c > 0. { typ::pt(c * scale) } else { "auto".into() }).collect();
        let ncols = t.cols.len();
        let align = match t.align {
            Align::Center => "center",
            Align::Right => "right",
            _ => "left",
        };
        let _ = writeln!(self.out, "#v(0pt, weak: true)\n#pad(left: {})[#align({align})[#table(columns: ({},), inset: (left: {}, right: {}, top: {}, bottom: {}), stroke: none, align: left,", typ::pt(t.indent.max(0.)), cols.join(", "), typ::pt(t.inset.0), typ::pt(t.inset.1), typ::pt(t.inset.2.max(1.)), typ::pt(t.inset.3.max(1.)));
        let mut taken: HashSet<(usize, usize)> = HashSet::new();
        for (y, row) in t.rows.iter().enumerate() {
            for cell in &row.cells {
                if cell.col >= ncols {
                    continue;
                }
                let span = cell.span.min(ncols - cell.col);
                let rowspan = cell.rowspan.min(t.rows.len() - y).max(1);
                let slots: Vec<(usize, usize)> = (y..y + rowspan).flat_map(|r| (cell.col..cell.col + span).map(move |c| (r, c))).collect();
                if slots.iter().any(|s| taken.contains(s)) {
                    continue;
                }
                taken.extend(slots);
                let saved = std::mem::take(&mut self.out);
                for b in &cell.blocks {
                    match b {
                        Block::Para(p) => self.para(p, true),
                        Block::Table(inner) => {
                            let w: f32 = t.cols[cell.col..cell.col + span].iter().sum::<f32>() * scale - t.inset.0 - t.inset.1;
                            self.table(inner, w.max(20.));
                        }
                        Block::PageBreak => {}
                    }
                }
                let body = std::mem::replace(&mut self.out, saved);
                let [top, right, bottom, left] = &cell.borders;
                let fill = cell.fill.as_ref().map(|f| format!("fill: {}, ", typ::rgb(f, 1.))).unwrap_or_default();
                let height = row.height.map(|h| format!("#block(height: {}, width: 0pt)", typ::pt((h - t.inset.2 - t.inset.3 - 2.).max(0.)))).unwrap_or_default();
                let _ = writeln!(
                    self.out,
                    "table.cell(x: {}, y: {y}, colspan: {span}, rowspan: {rowspan}, {fill}align: {}, stroke: (top: {}, right: {}, bottom: {}, left: {}))[{height}{body}],",
                    cell.col,
                    cell.valign,
                    Self::stroke(top),
                    Self::stroke(right),
                    Self::stroke(bottom),
                    Self::stroke(left)
                );
            }
        }
        self.out.push_str(")]]\n#v(0pt, weak: true)\n");
    }

    fn blocks(&mut self, blocks: &[Block], width: f32) {
        for b in blocks {
            match b {
                Block::Para(p) => self.para(p, false),
                Block::Table(t) => self.table(t, width),
                Block::PageBreak => self.out.push_str("#pagebreak(weak: true)\n"),
            }
        }
    }

    fn part(&mut self, blocks: &[Block], width: f32) -> String {
        let saved = std::mem::take(&mut self.out);
        self.blocks(blocks, width);
        std::mem::replace(&mut self.out, saved)
    }
}

impl Document {
    pub fn text(&self) -> String {
        fn walk(blocks: &[Block], out: &mut String) {
            for b in blocks {
                match b {
                    Block::Para(p) => {
                        if let Some((l, _)) = &p.label {
                            out.push_str(l);
                        }
                        for i in &p.inlines {
                            if let Inline::Text { text, .. } = i {
                                out.push_str(text);
                            }
                        }
                        out.push('\n');
                    }
                    Block::Table(t) => {
                        for r in &t.rows {
                            for c in &r.cells {
                                walk(&c.blocks, out);
                            }
                        }
                    }
                    Block::PageBreak => {}
                }
            }
        }
        let mut out = String::new();
        walk(&self.blocks, &mut out);
        out
    }
}

/// Typst source that lays the document out on its own pages.
pub fn to_typst(doc: &Document) -> Source {
    let base_size = doc.base.size.unwrap_or(10.);
    let base_font = doc.base.font.clone();
    let mut w = Writer { doc, out: String::new(), base_size, base_font: base_font.clone() };
    let p = doc.page;
    let width = (p.width - p.left - p.right).max(36.);
    let header = w.part(&doc.header, width);
    let footer = w.part(&doc.footer, width);
    let mut out = String::from(typ::PRELUDE);
    let family = base_font.map(|f| format!("{}, ", typ::s(&f))).unwrap_or_default();
    let color = doc.base.color.as_ref().filter(|c| !c.is_empty()).map(|c| typ::rgb(c, 1.)).unwrap_or_else(|| "black".into());
    let _ = writeln!(
        out,
        "#set page(width: {}, height: {}, margin: (top: {}, bottom: {}, left: {}, right: {}), fill: white, header-ascent: 0pt, footer-descent: 0pt, header: block(width: 100%, height: 100%)[#place(top + left, dy: {})[{header}]], footer: block(width: 100%, height: 100%)[#place(bottom + left, dy: {})[{footer}]])",
        typ::pt(p.width),
        typ::pt(p.height),
        typ::pt(p.top),
        typ::pt(p.bottom),
        typ::pt(p.left),
        typ::pt(p.right),
        typ::pt(p.header),
        typ::pt(-p.footer)
    );
    let _ = writeln!(out, "#set text(font: ({family}{}), size: {}, fill: {color}, top-edge: \"ascender\", bottom-edge: \"descender\", hyphenate: false)", typ::FALLBACK, typ::pt(base_size));
    out.push_str("#set par(spacing: 0pt)\n#set block(spacing: 0pt)\n#set heading(numbering: none)\n#show heading: it => block(width: 100%, it.body)\n#set footnote.entry(separator: line(length: 30%, stroke: 0.5pt))\n");
    w.blocks(&doc.blocks, width);
    out.push_str(&w.out);
    let files = doc.images.iter().enumerate().filter_map(|(i, img)| img.kind.map(|k| (format!("/img/{i}.{k}"), img.bytes.clone()))).collect();
    Source { text: out, files }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    pub fn zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, data) in parts {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
        buf.into_inner()
    }

    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

    pub const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    pub fn sample_docx() -> Vec<u8> {
        let body = format!(
            r#"<w:document {W}><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Projektbericht</w:t></w:r></w:p>
<w:p><w:pPr><w:jc w:val="center"/><w:spacing w:before="120" w:after="240"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Fett </w:t></w:r><w:r><w:rPr><w:i/><w:u w:val="single"/><w:strike/><w:sz w:val="28"/><w:color w:val="FF0000"/></w:rPr><w:t>bunt</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>Erstens</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>Unterpunkt</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>Zweitens</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>Punkt</w:t></w:r></w:p>
<w:tbl><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4"/><w:left w:val="single" w:sz="4"/><w:bottom w:val="single" w:sz="4"/><w:right w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="4"/><w:insideV w:val="single" w:sz="4"/></w:tblBorders></w:tblPr>
<w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>
<w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:shd w:fill="DDEEFF"/></w:tcPr><w:p><w:r><w:t>Breit</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>Hoch</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr>
</w:tbl>
<w:p><w:r><w:drawing><wp:inline xmlns:wp="wp"><wp:extent cx="1270000" cy="635000"/><a:graphic xmlns:a="a"><a:graphicData><pic:pic xmlns:pic="pic"><pic:blipFill><a:blip r:embed="rId9"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r><w:hyperlink r:id="rId5"><w:r><w:t>Webseite</w:t></w:r></w:hyperlink></w:p>
<w:p><w:r><w:t>vorher</w:t></w:r><w:r><w:br w:type="page"/></w:r><w:r><w:t>nachher</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Seite </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1417" w:right="1417" w:bottom="1134" w:left="1417" w:header="708" w:footer="708"/></w:sectPr>
</w:body></w:document>"#
        );
        let styles = format!(
            r#"<w:styles {W}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="240"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:color w:val="2F5496"/><w:sz w:val="32"/></w:rPr></w:style></w:styles>"#
        );
        let numbering = format!(
            r#"<w:numbering {W}><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%1.%2)"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>
<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="&#xF0B7;"/><w:rPr><w:rFonts w:ascii="Symbol"/></w:rPr><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>
<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
        );
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com" TargetMode="External"/><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/></Relationships>"#;
        let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
        zip(&[("_rels/.rels", root.as_bytes()), ("word/document.xml", body.as_bytes()), ("word/styles.xml", styles.as_bytes()), ("word/numbering.xml", numbering.as_bytes()), ("word/_rels/document.xml.rels", rels.as_bytes()), ("word/media/image1.png", PNG)])
    }

    fn paras(doc: &Document) -> Vec<&Para> {
        doc.blocks.iter().filter_map(|b| if let Block::Para(p) = b { Some(p) } else { None }).collect()
    }

    fn text_of(p: &Para) -> String {
        p.inlines.iter().filter_map(|i| if let Inline::Text { text, .. } = i { Some(text.as_str()) } else { None }).collect()
    }

    #[test]
    fn reads_headings_runs_lists_tables_images_links_and_breaks() {
        let doc = parse(sample_docx()).unwrap();
        assert!((doc.page.width - 595.3).abs() < 0.1 && (doc.page.left - 70.85).abs() < 0.1);
        let ps = paras(&doc);
        assert_eq!(ps[0].heading, Some(1));
        assert!((ps[0].before - 12.).abs() < 0.01);
        let Inline::Text { style, .. } = &ps[0].inlines[0] else { panic!() };
        assert_eq!((style.size, style.color.as_deref()), (Some(16.), Some("2f5496")));
        assert_eq!(ps[1].align, Align::Center);
        assert!((ps[1].after - 12.).abs() < 0.01);
        let Inline::Text { style: bold, .. } = &ps[1].inlines[0] else { panic!() };
        let Inline::Text { style: fancy, .. } = &ps[1].inlines[1] else { panic!() };
        assert_eq!(bold.bold, Some(true));
        assert_eq!((fancy.italic, fancy.underline, fancy.strike, fancy.size, fancy.color.as_deref()), (Some(true), Some(true), Some(true), Some(14.), Some("ff0000")));
        let labels: Vec<String> = ps[2..6].iter().map(|p| p.label.clone().unwrap().0).collect();
        assert_eq!(labels, ["1.", "1.a)", "2.", "•"]);
        assert!((ps[3].left - 72.).abs() < 0.01 && (ps[3].first + 18.).abs() < 0.01);
        let table = doc.blocks.iter().find_map(|b| if let Block::Table(t) = b { Some(t) } else { None }).unwrap();
        assert_eq!(table.cols.len(), 3);
        assert_eq!((table.rows[0].cells[0].span, table.rows[0].cells[0].fill.as_deref()), (2, Some("ddeeff")));
        assert_eq!(table.rows[0].cells[1].rowspan, 2);
        assert_eq!(table.rows[1].cells.len(), 2);
        assert!(table.rows[0].cells[0].borders.iter().all(Option::is_some));
        let pic = ps.iter().find(|p| p.inlines.iter().any(|i| matches!(i, Inline::Image { .. }))).unwrap();
        assert!(matches!(pic.inlines[0], Inline::Image { width, height, .. } if (width - 100.).abs() < 0.01 && (height - 50.).abs() < 0.01));
        assert!(matches!(&pic.inlines[1], Inline::Text { link: Some(Link::Url(u)), .. } if u == "https://example.com"));
        assert_eq!(doc.images[0].kind, Some("png"));
        let at = doc.blocks.iter().position(|b| *b == Block::PageBreak).unwrap();
        assert!(matches!(&doc.blocks[at - 1], Block::Para(p) if text_of(p) == "vorher"));
        assert!(matches!(&doc.blocks[at + 1], Block::Para(p) if text_of(p) == "nachher"));
        let last = ps.last().unwrap();
        assert!(matches!(last.inlines[1], Inline::PageNumber(_)));
        assert_eq!(text_of(last), "Seite ");
    }

    #[test]
    fn renders_to_pdf_pages() {
        let doc = parse(sample_docx()).unwrap();
        let source = to_typst(&doc);
        let pdf = crate::core::pdf_export::compile(&source.text, source.files).unwrap();
        let d = crate::core::pdf_render::PdfDoc::from_data(std::sync::Arc::new(pdf)).unwrap();
        assert_eq!(d.page_count(), 2);
        let (w, h) = d.page_size(0).unwrap();
        assert!((w - 595.3).abs() < 1. && (h - 841.9).abs() < 1.);
        let text = d.page_text(0).slice(0, d.page_text(0).glyph_count());
        assert!(text.contains("Projektbericht") && text.contains("Unterpunkt") && text.contains("Breit"), "{text}");
        assert!(d.page_text(1).slice(0, d.page_text(1).glyph_count()).contains("nachher"));
    }

    #[test]
    fn broken_and_locked_files_say_why() {
        assert_eq!(parse(b"not a zip".to_vec()).err(), Some(OpenError::Broken));
        assert_eq!(parse(vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0]).err(), Some(OpenError::Locked));
        assert_eq!(parse(zip(&[("word/document.xml", b"<w:document><broken")])).err(), Some(OpenError::Broken));
    }
}
