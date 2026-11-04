use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::numfmt::{self, Format, Locale};
use super::package::Package;
use super::xml::{self, El};
use super::OpenError;

const MAX_ROWS: u32 = 1_048_576;
const MAX_COLS: u32 = 16_384;
const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const LEGACY_FLAG: &str = "--read-legacy-workbook";
const LEGACY_CELLS: usize = 4_000_000;
const LEGACY_TEXT: usize = 256 << 20;
const LEGACY_OUTPUT: u64 = 512 << 20;
const LEGACY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HAlign {
    #[default]
    General,
    Left,
    Center,
    Right,
    Fill,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    Top,
    Center,
    #[default]
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub width: f32,
    pub color: u32,
}

/// How a cell is drawn: font, fill, alignment and borders.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub size: f32,
    pub font: Option<String>,
    pub color: Option<u32>,
    pub fill: Option<u32>,
    pub h: HAlign,
    pub v: VAlign,
    pub wrap: bool,
    pub indent: u8,
    pub borders: [Option<Border>; 4],
}

impl Default for Style {
    fn default() -> Self {
        Self { bold: false, italic: false, underline: false, strike: false, size: 11., font: None, color: None, fill: None, h: HAlign::General, v: VAlign::Bottom, wrap: false, indent: 0, borders: [None; 4] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Text,
    Number,
    Bool,
    Error,
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub col: u32,
    pub text: String,
    pub kind: CellKind,
    pub style: u32,
    pub color: Option<u32>,
    pub formula: Option<Box<str>>,
    /// A number in General format, which narrow columns may round.
    pub general: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub index: u32,
    pub cells: Vec<Cell>,
}

/// A merged range, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Merge {
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

/// Positions along rows or columns: sizes from the file, then the default size.
#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    starts: Vec<f32>,
    pub default: f32,
}

impl Axis {
    fn new(sizes: Vec<f32>, default: f32) -> Self {
        let mut starts = Vec::with_capacity(sizes.len() + 1);
        let mut at = 0.;
        starts.push(0.);
        for s in sizes {
            at += s;
            starts.push(at);
        }
        Self { starts, default }
    }

    /// Where an index begins.
    pub fn start(&self, i: u32) -> f32 {
        let n = self.starts.len() - 1;
        match self.starts.get(i as usize) {
            Some(s) => *s,
            None => self.starts[n] + (i as usize - n) as f32 * self.default,
        }
    }

    pub fn size(&self, i: u32) -> f32 {
        self.start(i + 1) - self.start(i)
    }

    /// The index under a position.
    pub fn at(&self, pos: f32) -> u32 {
        let n = self.starts.len() - 1;
        let end = self.starts[n];
        if pos >= end {
            if self.default <= 0. {
                return (0..n as u32).rev().find(|&i| self.size(i) > 0.).unwrap_or(0);
            }
            return (n as f32 + ((pos - end) / self.default.max(1.)).floor()) as u32;
        }
        let i = self.starts.partition_point(|s| *s <= pos);
        let mut i = i.saturating_sub(1);
        while i + 1 < n && self.starts[i + 1] <= pos {
            i += 1;
        }
        i as u32
    }
}

/// One worksheet, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub rows: Vec<Row>,
    pub styles: Vec<Style>,
    pub cols: Axis,
    pub heights: Axis,
    pub merges: Vec<Merge>,
    pub frozen_rows: u32,
    pub frozen_cols: u32,
    pub grid_lines: bool,
    pub used_rows: u32,
    pub used_cols: u32,
}

impl Sheet {
    pub fn row(&self, index: u32) -> Option<&Row> {
        self.rows.binary_search_by_key(&index, |r| r.index).ok().map(|i| &self.rows[i])
    }

    pub fn cell(&self, row: u32, col: u32) -> Option<&Cell> {
        let r = self.row(row)?;
        r.cells.binary_search_by_key(&col, |c| c.col).ok().map(|i| &r.cells[i])
    }

    /// The rows from `first` on, in order.
    pub fn rows_from(&self, first: u32) -> &[Row] {
        &self.rows[self.rows.partition_point(|r| r.index < first)..]
    }

    pub fn style(&self, cell: &Cell) -> &Style {
        self.styles.get(cell.style as usize).unwrap_or(&self.styles[0])
    }

    /// The merge a cell belongs to.
    pub fn merge_at(&self, row: u32, col: u32) -> Option<Merge> {
        self.merges.iter().copied().find(|m| (m.r0..=m.r1).contains(&row) && (m.c0..=m.c1).contains(&col))
    }

    /// Tab-separated text of a range, as spreadsheets paste it.
    pub fn tsv(&self, r0: u32, c0: u32, r1: u32, c1: u32) -> String {
        let mut out = String::new();
        let rows = self.rows_from(r0);
        let mut next = rows.iter().peekable();
        for r in r0..=r1 {
            let row = match next.peek() {
                Some(row) if row.index == r => next.next(),
                _ => None,
            };
            let mut line = String::new();
            let last = row.and_then(|row| row.cells.last()).map_or(c0, |cell| cell.col.clamp(c0, c1));
            for c in c0..=last {
                if c > c0 {
                    line.push('\t');
                }
                if let Some(cell) = row.and_then(|row| row.cells.binary_search_by_key(&c, |x| x.col).ok().map(|i| &row.cells[i])) {
                    let t = &cell.text;
                    if t.contains(['\t', '\n', '"']) {
                        line.push('"');
                        line.push_str(&t.replace('"', "\"\""));
                        line.push('"');
                    } else {
                        line.push_str(t);
                    }
                }
            }
            out.push_str(&line);
            if r < r1 {
                out.push_str("\r\n");
            }
        }
        out
    }
}

/// Column letters for an index: 0 is A, 26 is AA.
pub fn column_name(mut c: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Row and column of a reference like `B12`, zero based.
pub fn parse_ref(r: &str) -> Option<(u32, u32)> {
    let r = r.trim_start_matches('$');
    let split = r.find(|c: char| c.is_ascii_digit() || c == '$')?;
    let (letters, digits) = r.split_at(split);
    if letters.is_empty() {
        return None;
    }
    let mut col = 0u32;
    for b in letters.bytes() {
        if !b.is_ascii_alphabetic() {
            return None;
        }
        col = col.checked_mul(26)?.checked_add((b.to_ascii_uppercase() - b'A') as u32 + 1)?;
        if col > MAX_COLS {
            return None;
        }
    }
    let row: u32 = digits.trim_start_matches('$').parse().ok()?;
    (row >= 1 && row <= MAX_ROWS && col >= 1).then(|| (row - 1, col - 1))
}

fn parse_range(r: &str) -> Option<Merge> {
    let (a, b) = r.split_once(':').unwrap_or((r, r));
    let (r0, c0) = parse_ref(a)?;
    let (r1, c1) = parse_ref(b)?;
    Some(Merge { r0: r0.min(r1), c0: c0.min(c1), r1: r0.max(r1), c1: c0.max(c1) })
}

/// Pixel width of an Excel column width, for the default 7 px digit.
fn width_px(w: f32) -> f32 {
    ((256. * w + (128. / 7f32).trunc()) / 256. * 7.).trunc()
}

const INDEXED: [u32; 64] = [
    0x000000, 0xffffff, 0xff0000, 0x00ff00, 0x0000ff, 0xffff00, 0xff00ff, 0x00ffff, 0x000000, 0xffffff, 0xff0000, 0x00ff00, 0x0000ff, 0xffff00, 0xff00ff, 0x00ffff, 0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080, 0xc0c0c0, 0x808080, 0x9999ff, 0x993366, 0xffffcc, 0xccffff, 0x660066, 0xff8080, 0x0066cc, 0xccccff, 0x000080, 0xff00ff, 0xffff00, 0x00ffff, 0x800080, 0x800000, 0x008080,
    0x0000ff, 0x00ccff, 0xccffff, 0xccffcc, 0xffff99, 0x99ccff, 0xff99cc, 0xcc99ff, 0xffcc99, 0x3366ff, 0x33cccc, 0x99cc00, 0xffcc00, 0xff9900, 0xff6600, 0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399, 0x333333,
];

struct Styles {
    styles: Vec<Style>,
    formats: Vec<Format>,
}

fn theme_colors(pkg: &mut Package, book_part: &str) -> Vec<String> {
    let rels = pkg.rels(book_part);
    let Some(theme) = rels.values().find(|r| r.kind == "theme").and_then(|r| pkg.xml(&r.target)) else { return Vec::new() };
    let Some(scheme) = theme.find("clrScheme") else { return Vec::new() };
    let get = |n: &str| scheme.child(n).and_then(|c| c.child("srgbClr").and_then(|s| s.attr("val")).or_else(|| c.child("sysClr").and_then(|s| s.attr("lastClr")))).unwrap_or("000000").to_string();
    ["lt1", "dk1", "lt2", "dk2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"].iter().map(|n| get(n)).collect()
}

fn color(e: Option<&El>, theme: &[String]) -> Option<u32> {
    let e = e?;
    if e.attr("auto").is_some_and(|v| v == "1" || v == "true") {
        return None;
    }
    let hex = if let Some(rgb) = e.attr("rgb") {
        let rgb = if rgb.len() == 8 { rgb.get(2..).unwrap_or(rgb) } else { rgb };
        rgb.to_ascii_lowercase()
    } else if let Some(t) = e.attr("theme") {
        theme.get(t.parse::<usize>().ok()?)?.to_ascii_lowercase()
    } else if let Some(i) = e.attr("indexed") {
        let i: usize = i.parse().ok()?;
        if i >= 64 {
            return None;
        }
        format!("{:06x}", INDEXED[i])
    } else {
        return None;
    };
    let hex = match e.attr("tint").and_then(|t| t.parse::<f32>().ok()) {
        Some(t) if t < 0. => super::typ::adjust(&hex, 1. + t, 0., None, None),
        Some(t) if t > 0. => super::typ::adjust(&hex, 1. - t, t, None, None),
        _ => hex,
    };
    u32::from_str_radix(&hex, 16).ok()
}

fn border_width(style: &str) -> Option<f32> {
    Some(match style {
        "thin" | "hair" | "dotted" | "dashed" | "dashDot" | "dashDotDot" => 1.,
        "medium" | "mediumDashed" | "mediumDashDot" | "mediumDashDotDot" | "slantDashDot" => 2.,
        "thick" | "double" => 3.,
        _ => return None,
    })
}

fn load_styles(pkg: &mut Package, book_part: &str, locale: Locale) -> Styles {
    let theme = theme_colors(pkg, book_part);
    let rels = pkg.rels(book_part);
    let root = rels.values().find(|r| r.kind == "styles").and_then(|r| pkg.xml(&r.target));
    let Some(root) = root else { return Styles { styles: vec![Style::default()], formats: vec![Format::parse("General")] } };
    let custom: HashMap<u32, String> = root.child("numFmts").map(|n| n.all("numFmt").filter_map(|f| Some((f.attr("numFmtId")?.parse().ok()?, f.attr("formatCode")?.to_string()))).collect()).unwrap_or_default();
    let fonts: Vec<&El> = root.child("fonts").map(|f| f.all("font").collect()).unwrap_or_default();
    let fills: Vec<&El> = root.child("fills").map(|f| f.all("fill").collect()).unwrap_or_default();
    let borders: Vec<&El> = root.child("borders").map(|f| f.all("border").collect()).unwrap_or_default();
    let mut styles = Vec::new();
    let mut formats = Vec::new();
    let mut parsed: HashMap<u32, Format> = HashMap::new();
    for xf in root.child("cellXfs").map(|x| x.all("xf").collect::<Vec<_>>()).unwrap_or_default() {
        let id = |n: &str| xf.attr(n).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        let mut s = Style::default();
        if let Some(f) = fonts.get(id("fontId")) {
            s.bold = f.flag("b").unwrap_or(false);
            s.italic = f.flag("i").unwrap_or(false);
            s.strike = f.flag("strike").unwrap_or(false);
            s.underline = f.child("u").is_some_and(|u| u.attr("val") != Some("none"));
            s.size = f.val("sz").and_then(|v| v.parse().ok()).unwrap_or(11.);
            s.font = f.val("name").map(str::to_string);
            s.color = color(f.child("color"), &theme);
        }
        if let Some(p) = fills.get(id("fillId")).and_then(|f| f.child("patternFill"))
            && p.attr("patternType").is_some_and(|t| t != "none" && t != "gray125")
        {
            s.fill = color(p.child("fgColor"), &theme).or_else(|| color(p.child("bgColor"), &theme)).or(Some(0x000000).filter(|_| p.attr("patternType") == Some("solid") && p.child("fgColor").is_none()));
        }
        if let Some(b) = borders.get(id("borderId")) {
            for (i, side) in [("left", "start"), ("top", "top"), ("right", "end"), ("bottom", "bottom")].iter().enumerate() {
                let e = b.child(side.0).or_else(|| b.child(side.1));
                s.borders[i] = e.and_then(|e| Some(Border { width: border_width(e.attr("style")?)?, color: color(e.child("color"), &theme).unwrap_or(0) }));
            }
        }
        if let Some(a) = xf.child("alignment") {
            s.h = match a.attr("horizontal") {
                Some("left") => HAlign::Left,
                Some("center" | "centerContinuous" | "distributed") => HAlign::Center,
                Some("right") => HAlign::Right,
                Some("fill") => HAlign::Fill,
                Some("justify") => HAlign::Justify,
                _ => HAlign::General,
            };
            s.v = match a.attr("vertical") {
                Some("top") => VAlign::Top,
                Some("center" | "justify" | "distributed") => VAlign::Center,
                _ => VAlign::Bottom,
            };
            s.wrap = a.attr("wrapText").is_some_and(|v| v == "1" || v == "true");
            s.indent = a.attr("indent").and_then(|v| v.parse().ok()).unwrap_or(0);
        }
        let fmt_id = id("numFmtId") as u32;
        let format = parsed.entry(fmt_id).or_insert_with(|| Format::parse(custom.get(&fmt_id).map(String::as_str).or(numfmt::builtin(fmt_id, locale)).unwrap_or("General"))).clone();
        styles.push(s);
        formats.push(format);
    }
    if styles.is_empty() {
        styles.push(Style::default());
        formats.push(Format::parse("General"));
    }
    Styles { styles, formats }
}

fn shared_strings(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(false);
    let mut out = Vec::new();
    let (mut cur, mut in_t, mut skip) = (String::new(), false, 0usize);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.local_name().into_inner() {
                "si" => cur.clear(),
                "t" if skip == 0 => in_t = true,
                "rPh" | "phoneticPr" => skip += 1,
                _ => {}
            },
            Ok(Event::Empty(e)) if e.local_name().into_inner() == "si" => out.push(String::new()),
            Ok(Event::End(e)) => match e.local_name().into_inner() {
                "si" => out.push(std::mem::take(&mut cur)),
                "t" => in_t = false,
                "rPh" | "phoneticPr" => skip = skip.saturating_sub(1),
                _ => {}
            },
            Ok(Event::Text(t)) if in_t => cur.push_str(&t.xml10_content()),
            Ok(Event::CData(t)) if in_t => cur.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(r)) if in_t => {
                if let Some(c) = xml::reference(&r) {
                    cur.push(c);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn attr<'a>(e: &'a BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.local_name().into_inner() == name).and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
}

fn flag(e: &BytesStart, name: &str) -> bool {
    attr(e, name).is_some_and(|v| v == "1" || v == "true")
}

fn error_text(code: &str, german: bool) -> String {
    if !german {
        return code.to_string();
    }
    match code {
        "#DIV/0!" => "#DIV/0!",
        "#N/A" => "#NV",
        "#NAME?" => "#NAME?",
        "#NULL!" => "#NULL!",
        "#NUM!" => "#ZAHL!",
        "#REF!" => "#BEZUG!",
        "#VALUE!" => "#WERT!",
        c => c,
    }
    .to_string()
}

fn bool_text(v: bool, german: bool) -> String {
    match (v, german) {
        (true, true) => "WAHR",
        (false, true) => "FALSCH",
        (true, false) => "TRUE",
        (false, false) => "FALSE",
    }
    .to_string()
}

struct Builder {
    rows: Vec<Row>,
    widths: Vec<f32>,
    heights: Vec<(u32, f32)>,
    default_width: f32,
    default_height: f32,
    merges: Vec<Merge>,
    frozen: (u32, u32),
    grid_lines: bool,
    used: (u32, u32),
    tail: Option<(u32, f32)>,
}

impl Builder {
    fn new() -> Self {
        Self { rows: Vec::new(), widths: Vec::new(), heights: Vec::new(), default_width: 64., default_height: 20., merges: Vec::new(), frozen: (0, 0), grid_lines: true, used: (0, 0), tail: None }
    }

    fn finish(mut self, styles: Vec<Style>) -> Sheet {
        self.rows.sort_by_key(|r| r.index);
        self.rows.dedup_by_key(|r| r.index);
        for r in &mut self.rows {
            r.cells.sort_by_key(|c| c.col);
            r.cells.dedup_by_key(|c| c.col);
        }
        for m in &self.merges {
            self.used.0 = self.used.0.max(m.r1 + 1);
            self.used.1 = self.used.1.max(m.c1 + 1);
        }
        let tail_from = self.tail.map_or(u32::MAX, |t| t.0);
        let tail_width = self.tail.map(|t| t.1).filter(|w| !w.is_nan()).unwrap_or(self.default_width);
        let ncols = (self.widths.len() as u32).max(self.used.1).max(tail_from.min(MAX_COLS));
        let mut widths = self.widths;
        widths.resize(ncols as usize, f32::NAN);
        let widths: Vec<f32> = widths.into_iter().enumerate().map(|(i, w)| if !w.is_nan() { w } else if i as u32 >= tail_from { tail_width } else { self.default_width }).collect();
        self.heights.sort_by_key(|h| h.0);
        let nrows = self.heights.last().map_or(0, |h| h.0 + 1).max(self.used.0);
        let mut heights = vec![self.default_height; if self.heights.is_empty() { 0 } else { nrows as usize }];
        for (r, h) in &self.heights {
            if let Some(slot) = heights.get_mut(*r as usize) {
                *slot = *h;
            }
        }
        Sheet {
            rows: self.rows,
            styles,
            cols: Axis::new(widths, tail_width),
            heights: Axis::new(heights, self.default_height),
            merges: self.merges,
            frozen_rows: self.frozen.0,
            frozen_cols: self.frozen.1,
            grid_lines: self.grid_lines,
            used_rows: self.used.0,
            used_cols: self.used.1,
        }
    }
}

fn worksheet(bytes: &[u8], shared: &[String], styles: &Styles, date1904: bool, locale: Locale) -> Sheet {
    let text = String::from_utf8_lossy(bytes);
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(false);
    let mut b = Builder::new();
    let mut row: Option<Row> = None;
    let mut next_row = 0u32;
    let mut next_col = 0u32;
    let mut base_width: Option<f32> = None;
    let mut default_width: Option<f32> = None;
    let mut cell: Option<(u32, String, u32, Option<String>)> = None;
    let mut value = String::new();
    let mut formula = String::new();
    let mut inline = String::new();
    let mut target = 0u8;
    let mut shared_formulas: HashMap<String, String> = HashMap::new();
    let mut formula_si: Option<String> = None;
    let mut row_style: Option<u32> = None;
    let has_visible = |s: &Style| s.fill.is_some() || s.borders.iter().any(Option::is_some);
    loop {
        let ev = match reader.read_event() {
            Ok(e) => e,
            Err(_) => break,
        };
        let (e, empty) = match &ev {
            Event::Start(e) => (Some(e), false),
            Event::Empty(e) => (Some(e), true),
            _ => (None, false),
        };
        if let Some(e) = e {
            match e.local_name().into_inner() {
                "sheetFormatPr" => {
                    if let Some(h) = attr(e, "defaultRowHeight").and_then(|v| v.parse::<f32>().ok()) {
                        b.default_height = (h * 4. / 3.).round();
                    }
                    default_width = attr(e, "defaultColWidth").and_then(|v| v.parse().ok());
                    base_width = attr(e, "baseColWidth").and_then(|v| v.parse().ok());
                }
                "sheetView" => {
                    b.grid_lines = attr(e, "showGridLines").is_none_or(|v| v != "0" && v != "false");
                }
                "pane" => {
                    let state = attr(e, "state");
                    if state.as_deref().is_some_and(|s| s == "frozen" || s == "frozenSplit") {
                        let cols = attr(e, "xSplit").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.) as u32;
                        let rows = attr(e, "ySplit").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.) as u32;
                        b.frozen = (rows.min(1000), cols.min(200));
                    }
                }
                "col" => {
                    let min = attr(e, "min").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1).max(1);
                    let min = min.min(MAX_COLS);
                    let max = attr(e, "max").and_then(|v| v.parse::<u32>().ok()).unwrap_or(min).clamp(min, MAX_COLS);
                    let hidden = flag(e, "hidden");
                    let w = attr(e, "width").and_then(|v| v.parse::<f32>().ok());
                    if max - min > 2000 {
                        b.tail = Some((min - 1, if hidden { 0. } else { w.map_or(f32::NAN, width_px) }));
                        continue;
                    }
                    if b.widths.len() < max as usize {
                        b.widths.resize(max as usize, f32::NAN);
                    }
                    for c in min..=max {
                        b.widths[c as usize - 1] = if hidden { 0. } else { w.map_or(f32::NAN, width_px) };
                    }
                }
                "row" => {
                    let index = attr(e, "r").and_then(|v| v.parse::<u32>().ok()).map_or(next_row, |r| r.saturating_sub(1)).min(MAX_ROWS - 1);
                    next_row = index + 1;
                    next_col = 0;
                    if flag(e, "hidden") {
                        b.heights.push((index, 0.));
                    } else if let Some(h) = attr(e, "ht").and_then(|v| v.parse::<f32>().ok())
                        && (flag(e, "customHeight") || (h * 4. / 3. - b.default_height).abs() > 0.5)
                    {
                        b.heights.push((index, (h * 4. / 3.).round()));
                    }
                    row_style = if flag(e, "customFormat") { attr(e, "s").and_then(|v| v.parse().ok()) } else { None };
                    if let Some(r) = row.take().filter(|r| !r.cells.is_empty()) {
                        b.rows.push(r);
                    }
                    row = Some(Row { index, cells: Vec::new() });
                    if empty {
                        row = None;
                    }
                }
                "c" => {
                    let (r, c) = attr(e, "r").and_then(|v| parse_ref(&v)).unwrap_or((next_row.saturating_sub(1), next_col));
                    next_col = c + 1;
                    let style = attr(e, "s").and_then(|v| v.parse().ok()).or(row_style).unwrap_or(0);
                    let kind = attr(e, "t").unwrap_or_default();
                    if row.as_ref().is_none_or(|x| x.index != r) {
                        if let Some(old) = row.take().filter(|x| !x.cells.is_empty()) {
                            b.rows.push(old);
                        }
                        row = Some(Row { index: r, cells: Vec::new() });
                    }
                    value.clear();
                    formula.clear();
                    inline.clear();
                    formula_si = None;
                    cell = Some((c, kind, style, None));
                    if empty {
                        finish_cell(&mut b, &mut row, &mut cell, "", "", "", None, &mut shared_formulas, shared, styles, date1904, locale, has_visible);
                    }
                }
                "v" if cell.is_some() && !empty => target = 1,
                "f" if cell.is_some() => {
                    let t = attr(e, "t");
                    if t.as_deref() == Some("shared") {
                        formula_si = attr(e, "si");
                    }
                    if !empty {
                        target = 2;
                    }
                }
                "t" if cell.is_some() && !empty => target = 3,
                "rPh" => target = 9,
                "mergeCell" => {
                    if let Some(m) = attr(e, "ref").and_then(|r| parse_range(&r))
                        && (m.r0 != m.r1 || m.c0 != m.c1)
                    {
                        b.merges.push(m);
                    }
                }
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Text(t) => match target {
                1 => value.push_str(&t.xml10_content()),
                2 => formula.push_str(&t.xml10_content()),
                3 => inline.push_str(&t.xml10_content()),
                _ => {}
            },
            Event::GeneralRef(r) => {
                if let Some(ch) = xml::reference(&r) {
                    match target {
                        1 => value.push(ch),
                        2 => formula.push(ch),
                        3 => inline.push(ch),
                        _ => {}
                    }
                }
            }
            Event::End(e) => match e.local_name().into_inner() {
                "v" | "f" | "t" | "rPh" => target = 0,
                "c" => {
                    let (v, f, i) = (std::mem::take(&mut value), std::mem::take(&mut formula), std::mem::take(&mut inline));
                    let si = formula_si.take();
                    finish_cell(&mut b, &mut row, &mut cell, &v, &f, &i, si, &mut shared_formulas, shared, styles, date1904, locale, has_visible);
                }
                "row" => {
                    if let Some(r) = row.take().filter(|r| !r.cells.is_empty()) {
                        b.rows.push(r);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if let Some(r) = row.take().filter(|r| !r.cells.is_empty()) {
        b.rows.push(r);
    }
    b.default_width = match (default_width, base_width) {
        (Some(w), _) => width_px(w),
        (None, base) => ((base.unwrap_or(8.) * 7. + 5.) / 8.).ceil() * 8.,
    };
    b.finish(styles.styles.clone())
}

#[allow(clippy::too_many_arguments)]
fn finish_cell(
    b: &mut Builder,
    row: &mut Option<Row>,
    cell: &mut Option<(u32, String, u32, Option<String>)>,
    v: &str,
    f: &str,
    inline: &str,
    si: Option<String>,
    shared_formulas: &mut HashMap<String, String>,
    shared: &[String],
    styles: &Styles,
    date1904: bool,
    locale: Locale,
    has_visible: impl Fn(&Style) -> bool,
) {
    let Some((col, kind, style, _)) = cell.take() else { return };
    let Some(row) = row.as_mut() else { return };
    let format = styles.formats.get(style as usize);
    let formula = if !f.is_empty() {
        if let Some(si) = &si {
            shared_formulas.insert(si.clone(), f.to_string());
        }
        Some(format!("={f}").into_boxed_str())
    } else {
        si.as_ref().and_then(|si| shared_formulas.get(si)).map(|f| format!("={f}").into_boxed_str())
    };
    let mut general = false;
    let (text, kind, color) = match kind.as_str() {
        "s" => match v.trim().parse::<usize>().ok().and_then(|i| shared.get(i)) {
            Some(s) => {
                let shown = format.map(|f| f.text(s));
                (shown.as_ref().map_or_else(|| s.clone(), |x| x.text.clone()), CellKind::Text, shown.and_then(|x| x.color))
            }
            None => (String::new(), CellKind::Empty, None),
        },
        "inlineStr" => (inline.to_string(), CellKind::Text, None),
        "str" => (v.to_string(), CellKind::Text, None),
        "b" => (bool_text(v.trim() == "1", locale.german), CellKind::Bool, None),
        "e" => (error_text(v.trim(), locale.german), CellKind::Error, None),
        "d" => (v.to_string(), CellKind::Number, None),
        _ => match v.trim().parse::<f64>() {
            Ok(n) => {
                let shown = format.map_or_else(|| numfmt::Shown { text: numfmt::general(n, locale), color: None }, |f| f.number(n, date1904, locale));
                general = format.is_none_or(Format::is_general);
                (shown.text, CellKind::Number, shown.color)
            }
            Err(_) if v.is_empty() => (String::new(), CellKind::Empty, None),
            Err(_) => (v.to_string(), CellKind::Text, None),
        },
    };
    let visible = styles.styles.get(style as usize).is_some_and(&has_visible);
    if kind == CellKind::Empty && !visible && formula.is_none() {
        return;
    }
    b.used.0 = b.used.0.max(row.index + 1);
    b.used.1 = b.used.1.max(col + 1);
    row.cells.push(Cell { col, text, kind, style, color, formula, general });
}

/// A workbook that hands out its sheets one at a time.
pub struct Book {
    pub names: Vec<String>,
    pub active: usize,
    source: BookSource,
}

enum BookSource {
    Xlsx { pkg: Package, parts: Vec<String>, shared: Vec<String>, styles: Styles, date1904: bool, locale: Locale },
    Other { sheets: Box<calamine::Sheets<Cursor<Vec<u8>>>>, locale: Locale },
    Legacy { sheets: Vec<Vec<(u32, u32, Value)>>, locale: Locale },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum Value {
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
    Date { value: f64, duration: bool },
    Iso(String),
    Error(String),
}

impl Value {
    fn of(v: &calamine::Data) -> Option<Self> {
        use calamine::Data;
        Some(match v {
            Data::Int(i) => Self::Int(*i),
            Data::Float(f) => Self::Float(*f),
            Data::String(s) => Self::Text(s.clone()),
            Data::Bool(b) => Self::Bool(*b),
            Data::DateTime(d) => Self::Date { value: d.as_f64(), duration: d.is_duration() },
            Data::DateTimeIso(s) | Data::DurationIso(s) => Self::Iso(s.clone()),
            Data::Error(e) => Self::Error(e.to_string()),
            Data::Empty => return None,
        })
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LegacyBook {
    names: Vec<String>,
    sheets: Vec<Vec<(u32, u32, Value)>>,
}

/// When started as the legacy workbook reader, parses the workbook on stdin, writes its cells to stdout and returns the exit code; otherwise None.
pub fn legacy_reader() -> Option<i32> {
    use std::io::{Read as _, Write as _};
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(LEGACY_FLAG)) {
        return None;
    }
    let mut bytes = Vec::new();
    if std::io::stdin().read_to_end(&mut bytes).is_err() {
        return Some(2);
    }
    let Ok(out) = serde_json::to_vec(&legacy_read(bytes)) else { return Some(2) };
    Some(if std::io::stdout().write_all(&out).is_ok() { 0 } else { 2 })
}

fn legacy_read(bytes: Vec<u8>) -> Result<LegacyBook, OpenError> {
    use calamine::Reader as _;
    let mut book = calamine::open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(|e| calamine_error(&e))?;
    let names: Vec<String> = book.sheets_metadata().iter().filter(|s| s.visible == calamine::SheetVisible::Visible).map(|s| s.name.clone()).collect();
    let (mut cells, mut text) = (0, 0);
    let mut sheets = Vec::new();
    for name in &names {
        let mut sheet = Vec::new();
        if let Ok(range) = book.worksheet_range(name) {
            let (r0, c0) = range.start().unwrap_or((0, 0));
            for (r, c, v) in range.used_cells() {
                let (r, c) = (r as u32 + r0, c as u32 + c0);
                let Some(v) = Value::of(v).filter(|_| r < MAX_ROWS && c < MAX_COLS) else { continue };
                cells += 1;
                text += match &v {
                    Value::Text(s) | Value::Iso(s) | Value::Error(s) => s.len(),
                    _ => 0,
                };
                if cells > LEGACY_CELLS || text > LEGACY_TEXT {
                    return Err(OpenError::Broken);
                }
                sheet.push((r, c, v));
            }
        }
        sheets.push(sheet);
    }
    Ok(LegacyBook { names, sheets })
}

/// Parses a legacy workbook in a child process, so a file that makes calamine allocate without bound only takes the child down.
fn legacy_in_child(bytes: Vec<u8>) -> Result<LegacyBook, OpenError> {
    use std::io::{Read as _, Write as _};
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().map_err(|_| OpenError::Broken)?;
    let mut command = Command::new(exe);
    command.arg(LEGACY_FLAG).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|_| OpenError::Broken)?;
    let stdin = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(&bytes);
        }
    });
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(stdout) = stdout {
            let _ = stdout.take(LEGACY_OUTPUT).read_to_end(&mut out);
        }
        out
    });
    let deadline = std::time::Instant::now() + LEGACY_TIMEOUT;
    let finished = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let _ = writer.join();
    let out = reader.join().unwrap_or_default();
    if !finished {
        return Err(OpenError::Broken);
    }
    serde_json::from_slice::<Result<LegacyBook, OpenError>>(&out).unwrap_or(Err(OpenError::Broken))
}

fn calamine_error(e: &calamine::Error) -> OpenError {
    use calamine::Error as E;
    match e {
        E::Xls(calamine::XlsError::Password) | E::Xlsb(calamine::XlsbError::Password) | E::Ods(calamine::OdsError::Password) | E::Xlsx(calamine::XlsxError::Password) => OpenError::Locked,
        _ => OpenError::Broken,
    }
}

impl Book {
    /// Opens a workbook from its bytes; `ext` picks the reader.
    pub fn open(bytes: Vec<u8>, ext: &str) -> Result<Self, OpenError> {
        let locale = Locale::current();
        Self::open_with(bytes, ext, locale)
    }

    pub fn open_with(bytes: Vec<u8>, ext: &str, locale: Locale) -> Result<Self, OpenError> {
        match ext {
            "xlsx" | "xlsm" | "xltx" | "xltm" => Self::xlsx(bytes, locale),
            _ => {
                if bytes.starts_with(&CFB_MAGIC) {
                    let book = legacy_in_child(bytes)?;
                    if book.names.is_empty() {
                        return Err(OpenError::Broken);
                    }
                    return Ok(Self { names: book.names, active: 0, source: BookSource::Legacy { sheets: book.sheets, locale } });
                }
                let sheets = calamine::open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(|e| calamine_error(&e))?;
                use calamine::Reader as _;
                let names: Vec<String> = sheets.sheets_metadata().iter().filter(|s| s.visible == calamine::SheetVisible::Visible).map(|s| s.name.clone()).collect();
                if names.is_empty() {
                    return Err(OpenError::Broken);
                }
                Ok(Self { names, active: 0, source: BookSource::Other { sheets: Box::new(sheets), locale } })
            }
        }
    }

    fn xlsx(bytes: Vec<u8>, locale: Locale) -> Result<Self, OpenError> {
        let mut pkg = Package::from_bytes(bytes)?;
        let book_part = pkg.rels("").values().find(|r| r.kind == "officeDocument").map(|r| r.target.clone()).unwrap_or_else(|| "xl/workbook.xml".into());
        let book = pkg.xml(&book_part).ok_or(OpenError::Broken)?;
        let rels = pkg.rels(&book_part);
        let date1904 = book.child("workbookPr").is_some_and(|p| p.attr("date1904").is_some_and(|v| v == "1" || v == "true"));
        let mut names = Vec::new();
        let mut parts = Vec::new();
        let first_tab: usize = book.path(&["bookViews", "workbookView"]).and_then(|v| v.attr("activeTab")).and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut active = 0;
        for (i, s) in book.child("sheets").map(|s| s.all("sheet").collect::<Vec<_>>()).unwrap_or_default().into_iter().enumerate() {
            if s.attr("state").is_some_and(|v| v != "visible") {
                continue;
            }
            let Some(rel) = s.attr("r:id").and_then(|id| rels.get(id)) else { continue };
            if rel.kind != "worksheet" {
                continue;
            }
            if i == first_tab {
                active = names.len();
            }
            names.push(s.attr("name").unwrap_or("").to_string());
            parts.push(rel.target.clone());
        }
        if names.is_empty() {
            return Err(OpenError::Broken);
        }
        let shared = rels.values().find(|r| r.kind == "sharedStrings").and_then(|r| pkg.read(&r.target)).map(|b| shared_strings(&b)).unwrap_or_default();
        let styles = load_styles(&mut pkg, &book_part, locale);
        Ok(Self { names, active, source: BookSource::Xlsx { pkg, parts, shared, styles, date1904, locale } })
    }

    /// Reads one sheet; a sheet that cannot be read comes back empty.
    pub fn sheet(&mut self, index: usize) -> Sheet {
        match &mut self.source {
            BookSource::Xlsx { pkg, parts, shared, styles, date1904, locale } => match parts.get(index).and_then(|p| pkg.read(p)) {
                Some(bytes) => worksheet(&bytes, shared, styles, *date1904, *locale),
                None => Builder::new().finish(styles.styles.clone()),
            },
            BookSource::Legacy { sheets, locale } => from_cells(sheets.get(index).into_iter().flatten().map(|(r, c, v)| (*r, *c, v)), *locale),
            BookSource::Other { sheets, locale } => {
                use calamine::Reader as _;
                let name = self.names.get(index).cloned().unwrap_or_default();
                if let calamine::Sheets::Xlsb(x) = sheets.as_mut() {
                    let Ok(mut reader) = x.worksheet_cells_reader(&name) else { return Builder::new().finish(vec![Style::default()]) };
                    let mut cells = Vec::new();
                    while let Ok(Some(cell)) = reader.next_cell() {
                        let (r, c) = cell.get_position();
                        if let Some(v) = Value::of(&calamine::Data::from(cell.get_value().clone())).filter(|_| r < MAX_ROWS && c < MAX_COLS) {
                            cells.push((r, c, v));
                        }
                    }
                    return from_cells(cells.iter().map(|(r, c, v)| (*r, *c, v)), *locale);
                }
                match sheets.worksheet_range(&name) {
                    Ok(range) => {
                        let (r0, c0) = range.start().unwrap_or((0, 0));
                        let cells: Vec<_> = range.used_cells().map(|(r, c, v)| (r as u32 + r0, c as u32 + c0, v)).filter(|(r, c, _)| *r < MAX_ROWS && *c < MAX_COLS).filter_map(|(r, c, v)| Some((r, c, Value::of(v)?))).collect();
                        from_cells(cells.iter().map(|(r, c, v)| (*r, *c, v)), *locale)
                    }
                    Err(_) => Builder::new().finish(vec![Style::default()]),
                }
            }
        }
    }
}

fn from_cells<'a>(cells: impl Iterator<Item = (u32, u32, &'a Value)>, locale: Locale) -> Sheet {
    let mut b = Builder::new();
    let mut widths: Vec<usize> = Vec::new();
    let mut rows: HashMap<u32, Vec<Cell>> = HashMap::new();
    let general = Format::parse("General");
    let date = Format::parse(if locale.german { "dd.mm.yyyy" } else { "m/d/yyyy" });
    let date_time = Format::parse(if locale.german { "dd.mm.yyyy hh:mm" } else { "m/d/yyyy h:mm" });
    let duration = Format::parse("[h]:mm:ss");
    for (r, c, v) in cells {
        let plain = matches!(v, Value::Int(_) | Value::Float(_));
        let (text, kind) = match v {
            Value::Int(i) => (general.number(*i as f64, false, locale).text, CellKind::Number),
            Value::Float(f) => (general.number(*f, false, locale).text, CellKind::Number),
            Value::Text(s) => (s.clone(), CellKind::Text),
            Value::Bool(v) => (bool_text(*v, locale.german), CellKind::Bool),
            Value::Date { value, duration: is_duration } => {
                let f = if *is_duration {
                    &duration
                } else if value.fract() == 0. {
                    &date
                } else {
                    &date_time
                };
                (f.number(*value, false, locale).text, CellKind::Number)
            }
            Value::Iso(s) => (s.clone(), CellKind::Number),
            Value::Error(e) => (error_text(e, locale.german), CellKind::Error),
        };
        if widths.len() <= c as usize {
            widths.resize(c as usize + 1, 0);
        }
        widths[c as usize] = widths[c as usize].max(text.lines().map(|l| l.chars().count()).max().unwrap_or(0));
        b.used.0 = b.used.0.max(r + 1);
        b.used.1 = b.used.1.max(c + 1);
        rows.entry(r).or_default().push(Cell { col: c, text, kind, style: 0, color: None, formula: None, general: plain });
    }
    b.rows = rows.into_iter().map(|(index, cells)| Row { index, cells }).collect();
    b.widths = widths.into_iter().map(|n| if n == 0 { f32::NAN } else { (n as f32 * 7. + 12.).clamp(64., 320.) }).collect();
    b.finish(vec![Style::default()])
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_legacy_book_survives_the_trip_from_the_reader_process() {
        use super::{LegacyBook, Value};
        let cells = vec![(0, 0, Value::Text("Note".into())), (1, 0, Value::Float(1.5)), (1, 1, Value::Date { value: 45000.25, duration: false }), (2, 3, Value::Error("#DIV/0!".into()))];
        let sent: Result<LegacyBook, crate::core::office::OpenError> = Ok(LegacyBook { names: vec!["Noten".into()], sheets: vec![cells.clone()] });
        let back: Result<LegacyBook, crate::core::office::OpenError> = serde_json::from_slice(&serde_json::to_vec(&sent).unwrap()).unwrap();
        let back = back.ok().unwrap();
        assert_eq!((back.names, back.sheets), (vec!["Noten".to_string()], vec![cells]));
        let locked: Result<LegacyBook, _> = serde_json::from_slice(&serde_json::to_vec(&Err::<LegacyBook, _>(crate::core::office::OpenError::Locked)).unwrap()).unwrap();
        assert_eq!(locked.err(), Some(crate::core::office::OpenError::Locked), "a password stays a password, not a broken file");
    }


    use super::*;
    use crate::core::office::docx::tests::zip;

    const DE: Locale = Locale { german: true };

    fn rels(items: &[(&str, &str, &str)]) -> String {
        let mut s = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
        for (id, kind, target) in items {
            s.push_str(&format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#));
        }
        s.push_str("</Relationships>");
        s
    }

    pub fn sample_xlsx(rows: usize) -> Vec<u8> {
        let book = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><bookViews><workbookView activeTab="1"/></bookViews><sheets><sheet name="Noten" sheetId="1" r:id="rId1"/><sheet name="Daten" sheetId="2" r:id="rId2"/><sheet name="Versteckt" sheetId="3" state="hidden" r:id="rId3"/></sheets></workbook>"#;
        let styles = r##"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="#,##0.00\ &quot;€&quot;"/></numFmts><fonts><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="14"/><color rgb="FFC00000"/><name val="Calibri"/></font></fonts><fills><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor theme="4" tint="0.79998"/></patternFill></fill></fills><borders><border/><border><left style="thin"/><right style="thin"/><top style="thin"/><bottom style="medium"><color rgb="FF0000FF"/></bottom></border></borders><cellXfs><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/><xf numFmtId="14" fontId="0" fillId="0" borderId="0" applyNumberFormat="1"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0"/><xf numFmtId="9" fontId="1" fillId="2" borderId="1"><alignment horizontal="center" vertical="center" wrapText="1"/></xf></cellXfs></styleSheet>"##;
        let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="x"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#;
        let shared = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>Modul</t></si><si><r><t>Fett </t></r><r><t xml:space="preserve">&amp; normal</t></r><rPh><t>x</t></rPh></si></sst>"#;
        let sheet1 = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:D3"/><sheetViews><sheetView workbookViewId="0"><pane xSplit="1" ySplit="1" topLeftCell="B2" state="frozen"/></sheetView></sheetViews><sheetFormatPr defaultRowHeight="15"/><cols><col min="1" max="1" width="20.7109375" customWidth="1"/><col min="3" max="3" width="5" hidden="1"/></cols><sheetData><row r="1" ht="30" customHeight="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="D1" s="3"><v>0.256</v></c></row><row r="2"><c r="A2" s="1"><v>45943</v></c><c r="B2" s="2"><v>-1234.5</v></c><c r="C2" t="b"><v>1</v></c><c r="D2" t="e"><v>#DIV/0!</v></c></row><row r="3"><c r="A3"><f>SUM(B2:B2)</f><v>-1234.5</v></c><c r="B3" t="inlineStr"><is><t>inline</t></is></c><c r="E3" s="3"/></row></sheetData><mergeCells count="1"><mergeCell ref="B3:C4"/></mergeCells></worksheet>"#;
        let mut sheet2 = String::from(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#);
        for r in 1..=rows {
            sheet2.push_str(&format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}" t="s"><v>0</v></c></row>"#));
        }
        sheet2.push_str("</sheetData></worksheet>");
        let root = rels(&[("rId1", "officeDocument", "xl/workbook.xml")]);
        let book_rels = rels(&[("rId1", "worksheet", "worksheets/sheet1.xml"), ("rId2", "worksheet", "worksheets/sheet2.xml"), ("rId3", "worksheet", "worksheets/sheet2.xml"), ("rId4", "styles", "styles.xml"), ("rId5", "sharedStrings", "sharedStrings.xml"), ("rId6", "theme", "theme/theme1.xml")]);
        zip(&[
            ("_rels/.rels", root.as_bytes()),
            ("xl/workbook.xml", book.as_bytes()),
            ("xl/_rels/workbook.xml.rels", book_rels.as_bytes()),
            ("xl/styles.xml", styles.as_bytes()),
            ("xl/theme/theme1.xml", theme.as_bytes()),
            ("xl/sharedStrings.xml", shared.as_bytes()),
            ("xl/worksheets/sheet1.xml", sheet1.as_bytes()),
            ("xl/worksheets/sheet2.xml", sheet2.as_bytes()),
        ])
    }

    #[test]
    fn reads_values_formats_styles_sizes_merges_and_panes() {
        let mut book = Book::open_with(sample_xlsx(3), "xlsx", DE).unwrap();
        assert_eq!((book.names.clone(), book.active), (vec!["Noten".to_string(), "Daten".to_string()], 1));
        let s = book.sheet(0);
        let text = |r, c| s.cell(r, c).map(|c| c.text.as_str());
        assert_eq!(text(0, 0), Some("Modul"));
        assert_eq!(text(0, 1), Some("Fett & normal"));
        assert_eq!(text(0, 3), Some("26%"));
        assert_eq!(text(1, 0), Some("13.10.2025"));
        assert_eq!(text(1, 1), Some("-1.234,50 €"));
        assert_eq!(text(1, 2), Some("WAHR"));
        assert_eq!(text(1, 3), Some("#DIV/0!"));
        assert_eq!(text(2, 1), Some("inline"));
        assert_eq!(s.cell(2, 0).and_then(|c| c.formula.as_deref()), Some("=SUM(B2:B2)"));
        assert_eq!(s.cell(1, 1).map(|c| c.kind), Some(CellKind::Number));
        let st = s.style(s.cell(0, 3).unwrap());
        assert!(st.bold && st.wrap && st.size == 14. && st.color == Some(0xc00000) && st.h == HAlign::Center && st.v == VAlign::Center);
        assert_eq!(st.fill, Some(0xdae3f3));
        assert_eq!(st.borders[3], Some(Border { width: 2., color: 0x0000ff }));
        assert!(s.cell(2, 4).is_some());
        assert_eq!((s.cols.size(0), s.cols.size(1), s.cols.size(2)), (145., 64., 0.));
        assert_eq!((s.heights.size(0), s.heights.size(1), s.heights.size(5000)), (40., 20., 20.));
        assert_eq!(s.merges, vec![Merge { r0: 2, c0: 1, r1: 3, c1: 2 }]);
        assert_eq!((s.frozen_rows, s.frozen_cols), (1, 1));
        assert_eq!((s.used_rows, s.used_cols), (4, 5));
        assert_eq!(s.tsv(0, 0, 1, 1), "Modul\tFett & normal\r\n13.10.2025\t-1.234,50 €");
    }

    #[test]
    fn positions_and_references() {
        let a = Axis::new(vec![10., 0., 30.], 20.);
        assert_eq!((a.start(0), a.start(3), a.start(5)), (0., 40., 80.));
        assert_eq!((a.at(5.), a.at(10.), a.at(39.), a.at(41.), a.at(79.)), (0, 2, 2, 3, 4));
        assert_eq!((column_name(0), column_name(25), column_name(26), column_name(701), column_name(702)), ("A".into(), "Z".into(), "AA".into(), "ZZ".into(), "AAA".into()));
        assert_eq!(parse_ref("$AB$12"), Some((11, 27)));
        assert_eq!(parse_ref("12"), None);
    }

    #[test]
    fn reads_a_long_sheet() {
        let mut book = Book::open_with(sample_xlsx(20_000), "xlsx", DE).unwrap();
        let s = book.sheet(1);
        assert_eq!((s.rows.len(), s.used_rows), (20_000, 20_000));
        assert_eq!(s.cell(19_999, 0).map(|c| c.text.as_str()), Some("20000"));
        assert_eq!(s.rows_from(19_998).len(), 2);
    }

    fn only_sheet(data: &str) -> Sheet {
        let book = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="A" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
        let sheet = format!(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{data}</worksheet>"#);
        let root = rels(&[("rId1", "officeDocument", "xl/workbook.xml")]);
        let book_rels = rels(&[("rId1", "worksheet", "worksheets/sheet1.xml")]);
        let bytes = zip(&[("_rels/.rels", root.as_bytes()), ("xl/workbook.xml", book.as_bytes()), ("xl/_rels/workbook.xml.rels", book_rels.as_bytes()), ("xl/worksheets/sheet1.xml", sheet.as_bytes())]);
        Book::open_with(bytes, "xlsx", DE).unwrap().sheet(0)
    }

    #[test]
    fn hostile_sizes_stay_bounded() {
        let s = only_sheet(r#"<cols><col min="16385" max="16385" width="9"/></cols><sheetData><row r="1"><c r="ZZZZZZ1"><v>1</v></c><c r="B1"><v>2</v></c></row></sheetData><mergeCells><mergeCell ref="A1:ZZZZZZ9"/></mergeCells>"#);
        assert!(s.used_cols <= MAX_COLS && s.merges.is_empty());
        let s = only_sheet(r#"<cols><col min="30" max="16384" hidden="1"/></cols><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>"#);
        assert_eq!((s.cols.size(28), s.cols.size(29), s.cols.size(40)), (64., 0., 0.));
        assert_eq!(s.cols.at(s.cols.start(29) + 500.), 28);
        let s = only_sheet(r#"<sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="1000000"><c r="C1000000"><v>2</v></c></row></sheetData>"#);
        assert_eq!(s.tsv(0, 0, 999_999, 2).len(), 2 + 999_998 * 2 + 4);
    }

    #[test]
    fn broken_and_locked_workbooks_say_why() {
        assert_eq!(Book::open_with(b"nope".to_vec(), "xlsx", DE).err(), Some(OpenError::Broken));
        assert_eq!(Book::open_with(vec![0xD0, 0xCF, 0x11, 0xE0, 0, 0], "xlsx", DE).err(), Some(OpenError::Locked));
        assert_eq!(Book::open_with(zip(&[("x", b"x")]), "xlsx", DE).err(), Some(OpenError::Broken));
        assert_eq!(Book::open_with(b"nope".to_vec(), "ods", DE).err(), Some(OpenError::Broken));
    }
}
