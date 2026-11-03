use std::collections::HashMap;
use std::fmt::Write;

use super::package::{self, Package, Rel};
use super::typ::{self, Align};
use super::xml::El;
use super::{OpenError, Source};

const EMU: f32 = 12700.;

/// A colour as six hex digits with an opacity.
#[derive(Debug, Clone, PartialEq)]
pub struct Color {
    pub hex: String,
    pub alpha: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunStyle {
    pub size: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<Color>,
    pub font: Option<String>,
    pub baseline: Option<i8>,
    pub caps: Option<bool>,
}

impl RunStyle {
    fn over(&mut self, o: &RunStyle) {
        macro_rules! take {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        take!(size, bold, italic, underline, strike, color, font, baseline, caps);
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Bullet {
    None,
    Char(String),
    Auto(String, i64),
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Level {
    align: Option<Align>,
    mar_l: Option<f32>,
    indent: Option<f32>,
    line: Option<Spacing>,
    before: Option<Spacing>,
    after: Option<Spacing>,
    bullet: Option<Bullet>,
    bullet_color: Option<Color>,
    bullet_font: Option<String>,
    bullet_size: Option<f32>,
    run: RunStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Spacing {
    Percent(f32),
    Points(f32),
}

impl Level {
    fn over(&mut self, o: &Level) {
        macro_rules! take {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        take!(align, mar_l, indent, line, before, after, bullet, bullet_color, bullet_font, bullet_size);
        self.run.over(&o.run);
    }
}

type Levels = Vec<Level>;

fn merge_levels(into: &mut Levels, o: &Levels) {
    for (i, l) in o.iter().enumerate() {
        if let Some(t) = into.get_mut(i) {
            t.over(l);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct BodyProps {
    inset: [Option<f32>; 4],
    anchor: Option<&'static str>,
    wrap: Option<bool>,
    font_scale: Option<f32>,
    line_reduction: Option<f32>,
    grow: Option<bool>,
}

impl BodyProps {
    fn over(&mut self, o: &BodyProps) {
        for i in 0..4 {
            if o.inset[i].is_some() {
                self.inset[i] = o.inset[i];
            }
        }
        macro_rules! take {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f; } )* };
        }
        take!(anchor, wrap, font_scale, line_reduction, grow);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: RunStyle,
    pub link: Option<String>,
    pub line_break: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Para {
    pub align: Align,
    pub mar_l: f32,
    pub indent: f32,
    pub bullet: Option<(String, RunStyle)>,
    line: Option<Spacing>,
    before: Option<Spacing>,
    after: Option<Spacing>,
    pub runs: Vec<Run>,
    pub end: RunStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub inset: [f32; 4],
    pub anchor: &'static str,
    pub paras: Vec<Para>,
    pub scale: f32,
    pub line_reduction: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Geom {
    Rect,
    Round(f32),
    Ellipse,
    Line,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub col: usize,
    pub row: usize,
    pub span: usize,
    pub rowspan: usize,
    pub fill: Option<Color>,
    pub text: Text,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlideTable {
    pub cols: Vec<f32>,
    pub rows: Vec<f32>,
    pub cells: Vec<TableCell>,
    pub border: Option<Color>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub rot: f32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub geom: Geom,
    pub fill: Option<Color>,
    pub line: Option<(f32, Color)>,
    pub text: Option<Text>,
    pub image: Option<(usize, [f32; 4])>,
    pub table: Option<SlideTable>,
    pub placeholder: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Background {
    Color(Color),
    Image(usize),
}

pub struct Slide {
    pub background: Background,
    pub shapes: Vec<Shape>,
}

/// A presentation reduced to what the viewer draws.
pub struct Deck {
    pub width: f32,
    pub height: f32,
    pub slides: Vec<Slide>,
    pub images: Vec<super::docx::Image>,
}

struct Theme {
    colors: HashMap<String, String>,
    major: Option<String>,
    minor: Option<String>,
    fills: Vec<El>,
    bg_fills: Vec<El>,
}

struct Master {
    rels: HashMap<String, Rel>,
    root: El,
    map: HashMap<String, String>,
    title: Levels,
    body: Levels,
    other: Levels,
}

struct Parser {
    pkg: Package,
    theme: Theme,
    default_text: Levels,
    images: Vec<super::docx::Image>,
    image_index: HashMap<String, usize>,
}

fn emu(v: Option<&str>) -> Option<f32> {
    v?.parse::<f64>().ok().map(|e| (e / EMU as f64) as f32)
}

fn pct(v: Option<&str>) -> Option<f32> {
    v?.parse::<f32>().ok().map(|p| p / 100_000.)
}

fn empty_levels() -> Levels {
    vec![Level::default(); 9]
}

#[derive(Clone, Copy, PartialEq)]
struct Transform {
    sx: f32,
    sy: f32,
    dx: f32,
    dy: f32,
}

impl Transform {
    const ID: Transform = Transform { sx: 1., sy: 1., dx: 0., dy: 0. };

    fn apply(&self, x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
        (x * self.sx + self.dx, y * self.sy + self.dy, w * self.sx, h * self.sy)
    }
}

/// What a placeholder is called, for finding its template on the layout and master.
fn ph_of(sp: &El) -> Option<(String, Option<String>)> {
    let ph = sp.els().find(|e| e.name.starts_with("nv"))?.child("nvPr")?.child("ph")?;
    Some((ph.attr("type").unwrap_or("body").to_string(), ph.attr("idx").map(str::to_string)))
}

fn master_type(t: &str) -> &str {
    match t {
        "ctrTitle" | "title" => "title",
        "dt" | "ftr" | "sldNum" => t,
        _ => "body",
    }
}

fn find_ph<'a>(tree: Option<&'a El>, kind: &str, idx: Option<&str>, by_type_only: bool) -> Option<&'a El> {
    let shapes: Vec<&El> = tree?.els().filter(|e| e.name == "sp").collect();
    if !by_type_only
        && let Some(i) = idx
        && let Some(s) = shapes.iter().find(|s| ph_of(s).is_some_and(|(_, j)| j.as_deref() == Some(i)))
    {
        return Some(s);
    }
    let want = if by_type_only { master_type(kind) } else { kind };
    shapes.into_iter().find(|s| ph_of(s).is_some_and(|(t, _)| if by_type_only { master_type(&t) == want } else { t == want }))
}

impl Parser {
    fn color(&self, parent: Option<&El>, map: &HashMap<String, String>, placeholder: Option<&Color>) -> Option<Color> {
        let parent = parent?;
        let c = parent.els().find(|e| matches!(e.name.as_str(), "srgbClr" | "schemeClr" | "sysClr" | "prstClr" | "scrgbClr" | "hslClr"))?;
        let (mut hex, mut alpha) = match c.name.as_str() {
            "srgbClr" => (typ::hex(c.attr("val")?)?, 1.),
            "sysClr" => (typ::hex(c.attr("lastClr").unwrap_or(if c.attr("val") == Some("window") { "FFFFFF" } else { "000000" }))?, 1.),
            "schemeClr" => {
                let name = c.attr("val")?;
                if name == "phClr" {
                    let p = placeholder?;
                    (p.hex.clone(), p.alpha)
                } else {
                    let mapped = map.get(name).map(String::as_str).unwrap_or(match name {
                        "bg1" => "lt1",
                        "tx1" => "dk1",
                        "bg2" => "lt2",
                        "tx2" => "dk2",
                        n => n,
                    });
                    (self.theme.colors.get(mapped).cloned().unwrap_or_else(|| "000000".into()), 1.)
                }
            }
            "prstClr" => (
                match c.attr("val")? {
                    "white" => "ffffff",
                    "red" => "ff0000",
                    "green" => "008000",
                    "blue" => "0000ff",
                    "yellow" => "ffff00",
                    "gray" | "grey" => "808080",
                    _ => "000000",
                }
                .to_string(),
                1.,
            ),
            _ => ("000000".into(), 1.),
        };
        let (mut lum_mod, mut lum_off, mut tint, mut shade) = (1., 0., None, None);
        for m in c.els() {
            let v = pct(m.attr("val"));
            match m.name.as_str() {
                "lumMod" => lum_mod = v.unwrap_or(1.),
                "lumOff" => lum_off = v.unwrap_or(0.),
                "tint" => tint = v,
                "shade" => shade = v,
                "alpha" => alpha = v.unwrap_or(1.),
                _ => {}
            }
        }
        if lum_mod != 1. || lum_off != 0. || tint.is_some() || shade.is_some() {
            hex = typ::adjust(&hex, lum_mod, lum_off, tint, shade);
        }
        Some(Color { hex, alpha })
    }

    /// The fill of a shape's properties: `Some(None)` is an explicit "no fill".
    fn fill(&self, sppr: Option<&El>, map: &HashMap<String, String>) -> Option<Option<Color>> {
        let sppr = sppr?;
        for e in sppr.els() {
            match e.name.as_str() {
                "noFill" => return Some(None),
                "solidFill" => return Some(self.color(Some(e), map, None)),
                "gradFill" => return Some(e.child("gsLst").and_then(|l| l.els().next()).and_then(|gs| self.color(Some(gs), map, None))),
                "pattFill" => return Some(self.color(e.child("fgClr"), map, None)),
                "grpFill" => return None,
                _ => {}
            }
        }
        None
    }

    fn style_fill(&self, list: &[El], idx: usize, with: Option<Color>, map: &HashMap<String, String>) -> Option<Color> {
        let entry = list.get(idx.checked_sub(1)?)?;
        let e = entry.els().next().unwrap_or(entry);
        let wrap = El { name: "x".into(), attrs: vec![], children: vec![super::xml::Node::El(e.clone())] };
        match e.name.as_str() {
            "noFill" => None,
            "solidFill" => self.color(Some(e), map, with.as_ref()),
            "gradFill" => e.child("gsLst").and_then(|l| l.els().next()).and_then(|gs| self.color(Some(gs), map, with.as_ref())).or(with),
            _ => self.fill(Some(&wrap), map).flatten().or(with),
        }
    }

    fn levels(&self, list: Option<&El>, map: &HashMap<String, String>) -> Levels {
        let mut out = empty_levels();
        let Some(list) = list else { return out };
        for (i, slot) in out.iter_mut().enumerate() {
            let name = format!("lvl{}pPr", i + 1);
            if let Some(p) = list.child(&name) {
                *slot = self.level(p, map);
            }
        }
        if let Some(p) = list.child("defPPr") {
            let d = self.level(p, map);
            for l in &mut out {
                let mut base = d.clone();
                base.over(l);
                *l = base;
            }
        }
        out
    }

    fn spacing(e: Option<&El>) -> Option<Spacing> {
        let e = e?;
        if let Some(p) = e.child("spcPct") {
            return pct(p.attr("val")).map(Spacing::Percent);
        }
        e.child("spcPts").and_then(|p| p.attr("val")).and_then(|v| v.parse::<f32>().ok()).map(|v| Spacing::Points(v / 100.))
    }

    fn level(&self, p: &El, map: &HashMap<String, String>) -> Level {
        let bullet = if p.child("buNone").is_some() {
            Some(Bullet::None)
        } else if let Some(c) = p.child("buChar") {
            Some(Bullet::Char(c.attr("char").unwrap_or("•").to_string()))
        } else {
            p.child("buAutoNum").map(|a| Bullet::Auto(a.attr("type").unwrap_or("arabicPeriod").to_string(), a.attr("startAt").and_then(|v| v.parse().ok()).unwrap_or(1)))
        };
        Level {
            align: p.attr("algn").map(|a| match a {
                "ctr" => Align::Center,
                "r" => Align::Right,
                "just" | "dist" => Align::Justify,
                _ => Align::Left,
            }),
            mar_l: emu(p.attr("marL")),
            indent: emu(p.attr("indent")),
            line: Self::spacing(p.child("lnSpc")),
            before: Self::spacing(p.child("spcBef")),
            after: Self::spacing(p.child("spcAft")),
            bullet,
            bullet_color: p.child("buClr").and_then(|c| self.color(Some(c), map, None)),
            bullet_font: p.child("buFont").and_then(|f| f.attr("typeface")).map(str::to_string),
            bullet_size: p.child("buSzPct").and_then(|s| pct(s.attr("val"))),
            run: self.run_style(p.child("defRPr"), map),
        }
    }

    fn font(&self, face: &str) -> Option<String> {
        match face {
            "+mj-lt" | "+mj-ea" | "+mj-cs" => self.theme.major.clone(),
            "+mn-lt" | "+mn-ea" | "+mn-cs" => self.theme.minor.clone(),
            "" => None,
            f => Some(f.to_string()),
        }
    }

    fn run_style(&self, r: Option<&El>, map: &HashMap<String, String>) -> RunStyle {
        let Some(r) = r else { return RunStyle::default() };
        let flag = |name: &str| r.attr(name).map(|v| v == "1" || v == "true");
        RunStyle {
            size: r.attr("sz").and_then(|v| v.parse::<f32>().ok()).map(|v| v / 100.),
            bold: flag("b"),
            italic: flag("i"),
            underline: r.attr("u").map(|u| u != "none"),
            strike: r.attr("strike").map(|s| s != "noStrike"),
            color: r.child("solidFill").and_then(|f| self.color(Some(f), map, None)),
            font: r.child("latin").and_then(|l| l.attr("typeface")).and_then(|f| self.font(f)),
            baseline: r.attr("baseline").and_then(|b| b.parse::<i32>().ok()).map(|b| b.signum() as i8),
            caps: r.attr("cap").map(|c| c == "all"),
        }
    }

    fn body_props(e: Option<&El>) -> BodyProps {
        let Some(b) = e else { return BodyProps::default() };
        let anchor = b.attr("anchor").map(|a| match a {
            "ctr" => "horizon",
            "b" => "bottom",
            _ => "top",
        });
        let norm = b.child("normAutofit");
        BodyProps {
            inset: [emu(b.attr("lIns")), emu(b.attr("tIns")), emu(b.attr("rIns")), emu(b.attr("bIns"))],
            anchor,
            wrap: b.attr("wrap").map(|w| w != "none"),
            font_scale: norm.map(|n| pct(n.attr("fontScale")).unwrap_or(1.)),
            line_reduction: norm.map(|n| pct(n.attr("lnSpcReduction")).unwrap_or(0.)),
            grow: b.child("spAutoFit").map(|_| true),
        }
    }

    fn image(&mut self, rels: &HashMap<String, Rel>, rid: &str) -> Option<usize> {
        let rel = rels.get(rid)?;
        if rel.external {
            return None;
        }
        if let Some(i) = self.image_index.get(&rel.target) {
            return Some(*i);
        }
        let bytes = self.pkg.read(&rel.target)?;
        let kind = package::image_kind(&bytes);
        self.images.push(super::docx::Image { bytes, kind });
        let i = self.images.len() - 1;
        self.image_index.insert(rel.target.clone(), i);
        Some(i)
    }

    fn load_theme(pkg: &mut Package, master_part: &str) -> Theme {
        let rels = pkg.rels(master_part);
        let theme = rels.values().find(|r| r.kind == "theme").and_then(|r| pkg.xml(&r.target));
        let mut colors = HashMap::new();
        let (mut major, mut minor, mut fills, mut bg_fills) = (None, None, Vec::new(), Vec::new());
        if let Some(t) = theme {
            if let Some(scheme) = t.find("clrScheme") {
                for c in scheme.els() {
                    let hex = c.child("srgbClr").and_then(|s| s.attr("val")).or_else(|| c.child("sysClr").and_then(|s| s.attr("lastClr"))).and_then(typ::hex);
                    if let Some(h) = hex {
                        colors.insert(c.name.clone(), h);
                    }
                }
            }
            let face = |name: &str| t.find(name).and_then(|f| f.child("latin")).and_then(|l| l.attr("typeface")).filter(|f| !f.is_empty()).map(str::to_string);
            major = face("majorFont");
            minor = face("minorFont");
            fills = t.find("fillStyleLst").map(|l| l.els().cloned().collect()).unwrap_or_default();
            bg_fills = t.find("bgFillStyleLst").map(|l| l.els().cloned().collect()).unwrap_or_default();
        }
        Theme { colors, major, minor, fills, bg_fills }
    }

    fn master(&self, rels: HashMap<String, Rel>, root: El) -> Master {
        let map: HashMap<String, String> = root.child("clrMap").map(|m| m.attrs.iter().map(|(k, v)| (k.rsplit(':').next().unwrap_or(k).to_string(), v.clone())).collect()).unwrap_or_default();
        let styles = root.child("txStyles");
        let title = self.levels(styles.and_then(|s| s.child("titleStyle")), &map);
        let body = self.levels(styles.and_then(|s| s.child("bodyStyle")), &map);
        let other = self.levels(styles.and_then(|s| s.child("otherStyle")), &map);
        Master { rels, root, map, title, body, other }
    }

    fn background(&mut self, parts: &[(&El, &HashMap<String, Rel>)], map: &HashMap<String, String>) -> Background {
        for (root, rels) in parts {
            let Some(bg) = root.path(&["cSld", "bg"]) else { continue };
            if let Some(pr) = bg.child("bgPr") {
                if let Some(blip) = pr.child("blipFill").and_then(|b| b.child("blip")).and_then(|b| b.attr("r:embed"))
                    && let Some(i) = self.image(rels, blip)
                {
                    return Background::Image(i);
                }
                if let Some(Some(c)) = self.fill(Some(pr), map) {
                    return Background::Color(c);
                }
            }
            if let Some(r) = bg.child("bgRef") {
                let with = self.color(Some(r), map, None);
                let idx: usize = r.attr("idx").and_then(|i| i.parse().ok()).unwrap_or(0);
                let c = if idx >= 1001 { self.style_fill(&self.theme.bg_fills.clone(), idx - 1000, with.clone(), map) } else { self.style_fill(&self.theme.fills.clone(), idx, with.clone(), map) };
                if let Some(c) = c.or(with) {
                    return Background::Color(c);
                }
            }
        }
        Background::Color(Color { hex: "ffffff".into(), alpha: 1. })
    }

    fn text(&self, body: &El, levels: &Levels, props: &BodyProps, font_ref: Option<&Color>, map: &HashMap<String, String>, rels: &HashMap<String, Rel>, slide: usize) -> Text {
        let mut levels = levels.clone();
        if let Some(c) = font_ref {
            for l in &mut levels {
                l.run.color = Some(c.clone());
            }
        }
        merge_levels(&mut levels, &self.levels(body.child("lstStyle"), map));
        let mut paras = Vec::new();
        let mut counter: Option<(usize, String, i64)> = None;
        for p in body.all("p") {
            let ppr = p.child("pPr");
            let lvl = ppr.and_then(|e| e.attr("lvl")).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0).min(8);
            let mut level = levels[lvl].clone();
            if let Some(ppr) = ppr {
                level.over(&self.level(ppr, map));
            }
            let base = level.run.clone();
            let mut runs = Vec::new();
            for r in p.els() {
                match r.name.as_str() {
                    "r" | "fld" => {
                        let rpr = r.child("rPr");
                        let mut style = base.clone();
                        style.over(&self.run_style(rpr, map));
                        let link = rpr.and_then(|p| p.child("hlinkClick")).and_then(|h| h.attr("r:id")).and_then(|id| rels.get(id)).filter(|r| r.external).map(|r| r.target.clone());
                        if link.is_some() && rpr.and_then(|p| p.child("solidFill")).is_none() {
                            style.color = self.theme.colors.get("hlink").map(|h| Color { hex: h.clone(), alpha: 1. }).or(style.color);
                            style.underline = Some(true);
                        }
                        let text = if r.name == "fld" && r.attr("type") == Some("slidenum") { (slide + 1).to_string() } else { r.child("t").map(El::text).unwrap_or_default() };
                        runs.push(Run { text, style, link, line_break: false });
                    }
                    "br" => {
                        let mut style = base.clone();
                        style.over(&self.run_style(r.child("rPr"), map));
                        runs.push(Run { text: String::new(), style, link: None, line_break: true });
                    }
                    _ => {}
                }
            }
            let mut end = base.clone();
            end.over(&self.run_style(p.child("endParaRPr"), map));
            let has_text = runs.iter().any(|r| !r.text.trim().is_empty());
            let bullet = match (&level.bullet, has_text) {
                (Some(Bullet::Char(c)), true) => {
                    counter = None;
                    let ch: String = c.chars().map(|ch| typ::symbol_char(ch, level.bullet_font.as_deref())).collect();
                    let mut s = runs.first().map(|r| r.style.clone()).unwrap_or_else(|| base.clone());
                    s.font = None;
                    s.underline = None;
                    if let Some(c) = &level.bullet_color {
                        s.color = Some(c.clone());
                    }
                    s.size = s.size.map(|v| v * level.bullet_size.unwrap_or(1.));
                    Some((ch, s))
                }
                (Some(Bullet::Auto(kind, start)), true) => {
                    let n = match &counter {
                        Some((l, k, n)) if *l == lvl && k == kind => n + 1,
                        _ => *start,
                    };
                    counter = Some((lvl, kind.clone(), n));
                    let mut s = runs.first().map(|r| r.style.clone()).unwrap_or_else(|| base.clone());
                    s.underline = None;
                    Some((auto_number(kind, n), s))
                }
                _ => None,
            };
            paras.push(Para {
                align: level.align.unwrap_or_default(),
                mar_l: level.mar_l.unwrap_or(0.),
                indent: level.indent.unwrap_or(0.),
                bullet,
                line: level.line,
                before: level.before,
                after: level.after,
                runs,
                end,
            });
        }
        let inset = [props.inset[0].unwrap_or(7.2), props.inset[1].unwrap_or(3.6), props.inset[2].unwrap_or(7.2), props.inset[3].unwrap_or(3.6)];
        Text { inset, anchor: props.anchor.unwrap_or("top"), paras, scale: props.font_scale.unwrap_or(1.), line_reduction: props.line_reduction.unwrap_or(0.) }
    }

    #[allow(clippy::too_many_arguments)]
    fn shapes(&mut self, tree: &El, t: Transform, ctx: &Ctx, out: &mut Vec<Shape>, slide: usize, skip_placeholders: bool) {
        for e in tree.els() {
            match e.name.as_str() {
                "sp" | "cxnSp" => {
                    if let Some(s) = self.shape(e, t, ctx, slide, skip_placeholders) {
                        out.push(s);
                    }
                }
                "pic" => {
                    if skip_placeholders && ph_of(e).is_some() {
                        continue;
                    }
                    let Some((x, y, w, h, rot, fh, fv)) = xfrm(e.child("spPr").and_then(|s| s.child("xfrm"))) else { continue };
                    let (x, y, w, h) = t.apply(x, y, w, h);
                    let fill = e.child("blipFill");
                    let Some(rid) = fill.and_then(|b| b.child("blip")).and_then(|b| b.attr("r:embed")) else { continue };
                    let Some(index) = self.image(&ctx.rels, rid) else { continue };
                    let crop = fill.and_then(|f| f.child("srcRect")).map(|r| [pct(r.attr("l")).unwrap_or(0.), pct(r.attr("t")).unwrap_or(0.), pct(r.attr("r")).unwrap_or(0.), pct(r.attr("b")).unwrap_or(0.)]).unwrap_or_default();
                    let line = self.line(e.child("spPr"), None, ctx);
                    out.push(Shape { x, y, w, h, rot, flip_h: fh, flip_v: fv, geom: Geom::Rect, fill: None, line, text: None, image: Some((index, crop)), table: None, placeholder: false });
                }
                "grpSp" => {
                    let x = e.path(&["grpSpPr", "xfrm"]);
                    let inner = match x {
                        Some(x) => {
                            let g = |n: &str, a: &str| x.child(n).and_then(|c| emu(c.attr(a)));
                            let (ox, oy, cw, ch) = (g("off", "x").unwrap_or(0.), g("off", "y").unwrap_or(0.), g("ext", "cx").unwrap_or(0.), g("ext", "cy").unwrap_or(0.));
                            let (cox, coy, ccw, cch) = (g("chOff", "x").unwrap_or(ox), g("chOff", "y").unwrap_or(oy), g("chExt", "cx").unwrap_or(cw), g("chExt", "cy").unwrap_or(ch));
                            let sx = if ccw > 0. { cw / ccw } else { 1. };
                            let sy = if cch > 0. { ch / cch } else { 1. };
                            let local = Transform { sx, sy, dx: ox - cox * sx, dy: oy - coy * sy };
                            Transform { sx: t.sx * local.sx, sy: t.sy * local.sy, dx: t.sx * local.dx + t.dx, dy: t.sy * local.dy + t.dy }
                        }
                        None => t,
                    };
                    self.shapes(e, inner, ctx, out, slide, skip_placeholders);
                }
                "graphicFrame" => {
                    let Some((x, y, w, h, ..)) = xfrm(e.child("xfrm")) else { continue };
                    let (x, y, w, h) = t.apply(x, y, w, h);
                    if let Some(tbl) = e.find("tbl") {
                        let table = self.table(tbl, ctx, slide);
                        out.push(Shape { x, y, w, h, rot: 0., flip_h: false, flip_v: false, geom: Geom::Rect, fill: None, line: None, text: None, image: None, table: Some(table), placeholder: false });
                    } else {
                        let fill = Some(Color { hex: "eef0f3".into(), alpha: 1. });
                        out.push(Shape { x, y, w, h, rot: 0., flip_h: false, flip_v: false, geom: Geom::Rect, fill, line: Some((0.75, Color { hex: "c8ccd2".into(), alpha: 1. })), text: None, image: None, table: None, placeholder: false });
                    }
                }
                "AlternateContent" => self.shapes(e, t, ctx, out, slide, skip_placeholders),
                _ => {}
            }
        }
    }

    fn line(&self, sppr: Option<&El>, style: Option<&El>, ctx: &Ctx) -> Option<(f32, Color)> {
        let ln = sppr.and_then(|s| s.child("ln"));
        let width = ln.and_then(|l| emu(l.attr("w"))).unwrap_or(0.75).max(0.25);
        if let Some(ln) = ln {
            if ln.child("noFill").is_some() {
                return None;
            }
            if let Some(f) = ln.child("solidFill") {
                return self.color(Some(f), &ctx.map, None).map(|c| (width, c));
            }
            if let Some(g) = ln.child("gradFill") {
                return g.child("gsLst").and_then(|l| l.els().next()).and_then(|gs| self.color(Some(gs), &ctx.map, None)).map(|c| (width, c));
            }
        }
        let r = style?.child("lnRef")?;
        if r.attr("idx").is_none_or(|i| i == "0") {
            return None;
        }
        self.color(Some(r), &ctx.map, None).map(|c| (width, c))
    }

    fn shape(&mut self, sp: &El, t: Transform, ctx: &Ctx, slide: usize, skip_placeholders: bool) -> Option<Shape> {
        let ph = ph_of(sp);
        if skip_placeholders && ph.is_some() {
            return None;
        }
        let (layout_ph, master_ph) = match &ph {
            Some((kind, idx)) => (find_ph(ctx.layout_tree.as_ref(), kind, idx.as_deref(), false), find_ph(ctx.master_tree.as_ref(), kind, idx.as_deref(), true)),
            None => (None, None),
        };
        let own = sp.child("spPr");
        let frame = xfrm(own.and_then(|s| s.child("xfrm"))).or_else(|| xfrm(layout_ph.and_then(|p| p.path(&["spPr", "xfrm"])))).or_else(|| xfrm(master_ph.and_then(|p| p.path(&["spPr", "xfrm"]))));
        let (x, y, w, h, rot, flip_h, flip_v) = frame?;
        let (x, y, w, h) = t.apply(x, y, w, h);
        let style = sp.child("style");
        let fill = match self.fill(own, &ctx.map) {
            Some(f) => f,
            None => {
                let inherited = [layout_ph, master_ph].into_iter().flatten().find_map(|p| self.fill(p.child("spPr"), &ctx.map));
                match inherited {
                    Some(f) => f,
                    None => style.and_then(|s| s.child("fillRef")).and_then(|r| {
                        let idx: usize = r.attr("idx")?.parse().ok()?;
                        let with = self.color(Some(r), &ctx.map, None);
                        self.style_fill(&self.theme.fills.clone(), idx, with, &ctx.map)
                    }),
                }
            }
        };
        let geom = match own.and_then(|s| s.child("prstGeom")).and_then(|g| g.attr("prst")) {
            Some("ellipse" | "circle") => Geom::Ellipse,
            Some("roundRect") => Geom::Round(own.and_then(|s| s.find("gd")).and_then(|g| g.attr("fmla")).and_then(|f| f.strip_prefix("val ")).and_then(|v| v.parse::<f32>().ok()).map_or(0.1667, |v| v / 100_000.)),
            Some("line" | "straightConnector1" | "bentConnector3" | "curvedConnector3") => Geom::Line,
            _ if sp.name == "cxnSp" => Geom::Line,
            _ => Geom::Rect,
        };
        let line = self.line(own, style, ctx);
        let text = sp.child("txBody").filter(|b| b.all("p").any(|p| p.find("t").is_some_and(|t| !t.text().is_empty()))).map(|body| {
            let base = match ph.as_ref().map(|(k, _)| master_type(k)) {
                Some("title") => ctx.title.clone(),
                Some("body") => ctx.body.clone(),
                Some(_) => ctx.other.clone(),
                None => ctx.default_text.clone(),
            };
            let mut levels = base;
            let mut props = BodyProps::default();
            for p in [master_ph, layout_ph].into_iter().flatten() {
                if let Some(b) = p.child("txBody") {
                    merge_levels(&mut levels, &self.levels(b.child("lstStyle"), &ctx.map));
                    props.over(&Self::body_props(b.child("bodyPr")));
                }
            }
            props.over(&Self::body_props(body.child("bodyPr")));
            let font_ref = style.and_then(|s| s.child("fontRef")).and_then(|r| self.color(Some(r), &ctx.map, None));
            self.text(body, &levels, &props, font_ref.as_ref(), &ctx.map, &ctx.rels, slide)
        });
        Some(Shape { x, y, w, h, rot, flip_h, flip_v, geom, fill, line, text, image: None, table: None, placeholder: ph.is_some() })
    }

    fn table(&self, tbl: &El, ctx: &Ctx, slide: usize) -> SlideTable {
        let cols: Vec<f32> = tbl.child("tblGrid").map(|g| g.all("gridCol").map(|c| emu(c.attr("w")).unwrap_or(72.)).collect()).unwrap_or_default();
        let pr = tbl.child("tblPr");
        let flag = |n: &str| pr.and_then(|p| p.attr(n)).is_some_and(|v| v == "1" || v == "true");
        let (first, band) = (flag("firstRow"), flag("bandRow"));
        let accent = self.theme.colors.get("accent1").cloned().unwrap_or_else(|| "4472c4".into());
        let mut rows = Vec::new();
        let mut cells = Vec::new();
        for (ri, tr) in tbl.all("tr").enumerate() {
            rows.push(emu(tr.attr("h")).unwrap_or(20.));
            for (ci, tc) in tr.all("tc").enumerate() {
                if tc.attr("hMerge").is_some() || tc.attr("vMerge").is_some() {
                    continue;
                }
                let tcpr = tc.child("tcPr");
                let header = first && ri == 0;
                let fill = self.fill(tcpr, &ctx.map).unwrap_or_else(|| {
                    if header {
                        Some(Color { hex: accent.clone(), alpha: 1. })
                    } else if band || first {
                        Some(Color { hex: typ::adjust(&accent, 1., 0., Some(if (ri + usize::from(!first)) % 2 == 1 { 0.2 } else { 0.4 }), None), alpha: 1. })
                    } else {
                        None
                    }
                });
                let mut levels = ctx.default_text.clone();
                if header {
                    for l in &mut levels {
                        l.run.bold = Some(true);
                        l.run.color = Some(Color { hex: "ffffff".into(), alpha: 1. });
                    }
                }
                let ins = |n: &str, d: f32| tcpr.and_then(|p| emu(p.attr(n))).unwrap_or(d);
                let props = BodyProps { inset: [Some(ins("marL", 7.2)), Some(ins("marT", 3.6)), Some(ins("marR", 7.2)), Some(ins("marB", 3.6))], anchor: tcpr.and_then(|p| p.attr("anchor")).map(|a| if a == "ctr" { "horizon" } else if a == "b" { "bottom" } else { "top" }), ..Default::default() };
                let text = tc.child("txBody").map(|b| self.text(b, &levels, &props, None, &ctx.map, &ctx.rels, slide)).unwrap_or(Text { inset: [7.2, 3.6, 7.2, 3.6], anchor: "top", paras: Vec::new(), scale: 1., line_reduction: 0. });
                let span = tc.attr("gridSpan").and_then(|v| v.parse().ok()).unwrap_or(1usize).max(1);
                let rowspan = tc.attr("rowSpan").and_then(|v| v.parse().ok()).unwrap_or(1usize).max(1);
                cells.push(TableCell { col: ci, row: ri, span, rowspan, fill, text });
            }
        }
        SlideTable { cols, rows, cells, border: Some(Color { hex: "ffffff".into(), alpha: 1. }) }
    }
}

struct Ctx {
    rels: HashMap<String, Rel>,
    map: HashMap<String, String>,
    layout_tree: Option<El>,
    master_tree: Option<El>,
    title: Levels,
    body: Levels,
    other: Levels,
    default_text: Levels,
}

fn xfrm(x: Option<&El>) -> Option<(f32, f32, f32, f32, f32, bool, bool)> {
    let x = x?;
    let off = x.child("off")?;
    let ext = x.child("ext")?;
    let rot = x.attr("rot").and_then(|r| r.parse::<f32>().ok()).unwrap_or(0.) / 60000.;
    let flag = |n: &str| x.attr(n).is_some_and(|v| v == "1" || v == "true");
    Some((emu(off.attr("x"))?, emu(off.attr("y"))?, emu(ext.attr("cx"))?, emu(ext.attr("cy"))?, rot, flag("flipH"), flag("flipV")))
}

fn auto_number(kind: &str, n: i64) -> String {
    let roman = |n: i64| {
        let table = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
        let (mut n, mut out) = (n, String::new());
        for (v, s) in table {
            while n >= v {
                out.push_str(s);
                n -= v;
            }
        }
        out
    };
    let letter = |n: i64| ((b'a' + ((n - 1).rem_euclid(26)) as u8) as char).to_string();
    let core = if kind.starts_with("alphaLc") {
        letter(n)
    } else if kind.starts_with("alphaUc") {
        letter(n).to_uppercase()
    } else if kind.starts_with("romanLc") {
        roman(n)
    } else if kind.starts_with("romanUc") {
        roman(n).to_uppercase()
    } else {
        n.to_string()
    };
    if kind.ends_with("ParenBoth") {
        format!("({core})")
    } else if kind.ends_with("ParenR") {
        format!("{core})")
    } else if kind.ends_with("Plain") {
        core
    } else {
        format!("{core}.")
    }
}

/// Reads a PowerPoint file. Fails only when the package or its slide list cannot be read.
pub fn parse(bytes: Vec<u8>) -> Result<Deck, OpenError> {
    let mut pkg = Package::from_bytes(bytes)?;
    let main = pkg.rels("").values().find(|r| r.kind == "officeDocument").map(|r| r.target.clone()).unwrap_or_else(|| "ppt/presentation.xml".into());
    let pres = pkg.xml(&main).ok_or(OpenError::Broken)?;
    let pres_rels = pkg.rels(&main);
    let size = pres.child("sldSz");
    let width = emu(size.and_then(|s| s.attr("cx"))).unwrap_or(720.);
    let height = emu(size.and_then(|s| s.attr("cy"))).unwrap_or(405.);
    let slide_parts: Vec<String> = pres.child("sldIdLst").map(|l| l.all("sldId").filter_map(|s| s.attr("r:id")).filter_map(|id| pres_rels.get(id)).map(|r| r.target.clone()).collect()).unwrap_or_default();
    let first_master = pres_rels.values().find(|r| r.kind == "slideMaster").map(|r| r.target.clone()).unwrap_or_default();
    let theme = Parser::load_theme(&mut pkg, &first_master);
    let mut parser = Parser { pkg, theme, default_text: empty_levels(), images: Vec::new(), image_index: HashMap::new() };
    parser.default_text = parser.levels(pres.child("defaultTextStyle"), &HashMap::new());
    let mut masters: HashMap<String, Master> = HashMap::new();
    let mut slides = Vec::new();
    for (n, part) in slide_parts.iter().enumerate() {
        let Some(root) = parser.pkg.xml(part) else { continue };
        let rels = parser.pkg.rels(part);
        let layout_part = rels.values().find(|r| r.kind == "slideLayout").map(|r| r.target.clone());
        let layout = layout_part.as_ref().and_then(|p| parser.pkg.xml(p));
        let layout_rels = layout_part.as_ref().map(|p| parser.pkg.rels(p)).unwrap_or_default();
        let master_part = layout_rels.values().find(|r| r.kind == "slideMaster").map(|r| r.target.clone()).unwrap_or_else(|| first_master.clone());
        if !masters.contains_key(&master_part) {
            let mroot = parser.pkg.xml(&master_part).unwrap_or_default();
            let mrels = parser.pkg.rels(&master_part);
            let m = parser.master(mrels, mroot);
            masters.insert(master_part.clone(), m);
        }
        let master = &masters[&master_part];
        let map = root.child("clrMapOvr").and_then(|o| o.child("overrideClrMapping")).map(|m| m.attrs.iter().map(|(k, v)| (k.rsplit(':').next().unwrap_or(k).to_string(), v.clone())).collect()).unwrap_or_else(|| master.map.clone());
        let mut parts: Vec<(&El, &HashMap<String, Rel>)> = vec![(&root, &rels)];
        if let Some(l) = &layout {
            parts.push((l, &layout_rels));
        }
        parts.push((&master.root, &master.rels));
        let background = parser.background(&parts, &map);
        let default_text = {
            let mut d = parser.default_text.clone();
            merge_levels(&mut d, &master.other.iter().map(|l| Level { run: RunStyle { size: None, ..l.run.clone() }, ..Default::default() }).collect());
            d
        };
        let tree = |e: &El| e.path(&["cSld", "spTree"]).cloned();
        let ctx_for = |rels: HashMap<String, Rel>| Ctx {
            rels,
            map: map.clone(),
            layout_tree: layout.as_ref().and_then(tree),
            master_tree: tree(&master.root),
            title: master.title.clone(),
            body: master.body.clone(),
            other: master.other.clone(),
            default_text: default_text.clone(),
        };
        let show_master = |e: &El| e.attr("showMasterSp").is_none_or(|v| v != "0");
        let mut shapes = Vec::new();
        let (master_rels, master_root) = (master.rels.clone(), master.root.clone());
        if show_master(&root) && layout.as_ref().is_none_or(show_master)
            && let Some(t) = tree(&master_root)
        {
            let ctx = ctx_for(master_rels);
            parser.shapes(&t, Transform::ID, &ctx, &mut shapes, n, true);
        }
        if show_master(&root)
            && let Some(t) = layout.as_ref().and_then(tree)
        {
            let ctx = ctx_for(layout_rels.clone());
            parser.shapes(&t, Transform::ID, &ctx, &mut shapes, n, true);
        }
        if let Some(t) = tree(&root) {
            let ctx = ctx_for(rels.clone());
            parser.shapes(&t, Transform::ID, &ctx, &mut shapes, n, false);
        }
        slides.push(Slide { background, shapes });
    }
    Ok(Deck { width, height, slides, images: parser.images })
}

fn color_expr(c: &Color) -> String {
    typ::rgb(&c.hex, c.alpha)
}

fn run_expr(r: &Run, scale: f32) -> String {
    let s = &r.style;
    let mut a = String::new();
    if let Some(f) = &s.font {
        let _ = write!(a, "font: ({}, {}), ", typ::s(f), typ::FALLBACK);
    }
    let _ = write!(a, "size: {}, ", typ::pt(s.size.unwrap_or(18.) * scale));
    if let Some(c) = &s.color {
        let _ = write!(a, "fill: {}, ", color_expr(c));
    }
    if s.bold == Some(true) {
        a.push_str("weight: \"bold\", ");
    }
    if s.italic == Some(true) {
        a.push_str("style: \"italic\", ");
    }
    let text = if s.caps == Some(true) { r.text.to_uppercase() } else { r.text.clone() };
    let mut e = format!("text({a}{})", typ::s(&text));
    if s.underline == Some(true) {
        e = format!("underline({e})");
    }
    if s.strike == Some(true) {
        e = format!("strike({e})");
    }
    match s.baseline {
        Some(1) => e = format!("super({e})"),
        Some(-1) => e = format!("sub({e})"),
        _ => {}
    }
    if let Some(l) = &r.link {
        e = format!("link({}, {e})", typ::s(l));
    }
    e
}

fn spacing_pt(s: Option<Spacing>, size: f32) -> f32 {
    match s {
        Some(Spacing::Points(p)) => p,
        Some(Spacing::Percent(p)) => p * size * 1.2,
        None => 0.,
    }
}

fn text_markup(t: &Text) -> String {
    let mut out = String::new();
    for p in &t.paras {
        let size = p.runs.first().map(|r| &r.style).unwrap_or(&p.end).size.unwrap_or(18.) * t.scale;
        let leading = match p.line {
            Some(Spacing::Percent(v)) => format!("{:.3}em", ((v - t.line_reduction) * 1.2 - 1.).max(0.)),
            Some(Spacing::Points(v)) => typ::pt((v - size).max(0.)),
            None => "0.2em".into(),
        };
        let _ = write!(out, "#v({}, weak: true)", typ::pt(spacing_pt(p.before, size)));
        let hanging = (-p.indent).max(0.);
        let pad = (p.mar_l + p.indent.min(0.)).max(0.);
        let mut body = String::new();
        if let Some((label, style)) = &p.bullet {
            let e = run_expr(&Run { text: label.clone(), style: style.clone(), link: None, line_break: false }, t.scale);
            if hanging > 0. {
                let _ = write!(body, "#box(width: {})[#{e}]", typ::pt(hanging));
            } else {
                let _ = write!(body, "#{e}#h(0.4em)");
            }
        }
        for r in &p.runs {
            if r.line_break {
                body.push_str("#linebreak()");
            } else if !r.text.is_empty() {
                let _ = write!(body, "#{}", run_expr(r, t.scale));
            }
        }
        if p.runs.iter().all(|r| r.text.is_empty()) && p.bullet.is_none() {
            let _ = write!(body, "#{}", run_expr(&Run { text: "\u{200B}".into(), style: p.end.clone(), link: None, line_break: false }, t.scale));
        }
        let first = if p.indent > 0. { p.indent } else { 0. };
        let _ = write!(
            out,
            "#[#set par(leading: {leading}, justify: {}, hanging-indent: {}, first-line-indent: (amount: {}, all: true))\n#set align({})\n#pad(left: {})[{body}]]",
            p.align == Align::Justify,
            typ::pt(hanging),
            typ::pt(first),
            p.align.typst(),
            typ::pt(pad)
        );
        let _ = writeln!(out, "#v({}, weak: true)", typ::pt(spacing_pt(p.after, size)));
    }
    out
}

fn shape_markup(s: &Shape, images: &[super::docx::Image]) -> String {
    let fill = s.fill.as_ref().map(color_expr).unwrap_or_else(|| "none".into());
    let stroke = s.line.as_ref().map(|(w, c)| format!("{} + {}", typ::pt(*w), color_expr(c))).unwrap_or_else(|| "none".into());
    let (w, h) = (s.w.max(0.), s.h.max(0.));
    if s.geom == Geom::Line {
        let (mut x1, mut y1, mut x2, mut y2) = (0., 0., w, h);
        if s.flip_h {
            std::mem::swap(&mut x1, &mut x2);
        }
        if s.flip_v {
            std::mem::swap(&mut y1, &mut y2);
        }
        let stroke = if s.line.is_none() { "0.75pt + black".to_string() } else { stroke };
        return format!("line(start: ({}, {}), end: ({}, {}), stroke: {stroke})", typ::pt(x1), typ::pt(y1), typ::pt(x2), typ::pt(y2));
    }
    if let Some((index, crop)) = s.image {
        let img = &images[index];
        let Some(kind) = img.kind else { return format!("office-missing(\"\", width: {}, height: {})", typ::pt(w), typ::pt(h)) };
        let (sx, sy) = ((1. - crop[0] - crop[2]).max(0.01), (1. - crop[1] - crop[3]).max(0.01));
        let (iw, ih) = (w / sx, h / sy);
        let mut e = format!("image({}, width: {}, height: {}, fit: \"stretch\")", typ::s(&format!("/img/{index}.{kind}")), typ::pt(iw), typ::pt(ih));
        if s.flip_h {
            e = format!("scale(x: -100%, {e})");
        }
        return format!("box(width: {}, height: {}, clip: true, stroke: {stroke})[#place(top + left, dx: {}, dy: {}, {e})]", typ::pt(w), typ::pt(h), typ::pt(-crop[0] * iw), typ::pt(-crop[1] * ih));
    }
    if let Some(t) = &s.table {
        return table_markup(t, w);
    }
    let body = s.text.as_ref().map(|t| format!("#align({})[{}]", t.anchor, text_markup(t))).unwrap_or_default();
    let inset = s.text.as_ref().map(|t| t.inset).unwrap_or([0.; 4]);
    let inset = format!("(left: {}, top: {}, right: {}, bottom: {})", typ::pt(inset[0]), typ::pt(inset[1]), typ::pt(inset[2]), typ::pt(inset[3]));
    match s.geom {
        Geom::Ellipse => format!("ellipse(width: {}, height: {}, fill: {fill}, stroke: {stroke}, inset: {inset})[{body}]", typ::pt(w), typ::pt(h)),
        Geom::Round(r) => format!("block(width: {}, height: {}, fill: {fill}, stroke: {stroke}, radius: {}, inset: {inset}, breakable: false)[{body}]", typ::pt(w), typ::pt(h), typ::pt(w.min(h) * r.clamp(0., 0.5))),
        _ => format!("block(width: {}, height: {}, fill: {fill}, stroke: {stroke}, inset: {inset}, breakable: false)[{body}]", typ::pt(w), typ::pt(h)),
    }
}

fn table_markup(t: &SlideTable, width: f32) -> String {
    let total: f32 = t.cols.iter().sum();
    let scale = if total > 0. && total > width * 1.05 { width / total } else { 1. };
    let cols: Vec<String> = t.cols.iter().map(|c| typ::pt(c * scale)).collect();
    let rows: Vec<String> = t.rows.iter().map(|r| typ::pt(*r)).collect();
    let stroke = t.border.as_ref().map(|c| format!("1pt + {}", color_expr(c))).unwrap_or_else(|| "none".into());
    let mut out = format!("table(columns: ({},), rows: ({},), stroke: {stroke}, inset: 0pt,\n", cols.join(", "), rows.join(", "));
    let n = t.cols.len();
    for c in &t.cells {
        if c.col >= n || c.row >= t.rows.len() {
            continue;
        }
        let fill = c.fill.as_ref().map(|f| format!("fill: {}, ", color_expr(f))).unwrap_or_default();
        let inset = c.text.inset;
        let _ = writeln!(
            out,
            "table.cell(x: {}, y: {}, colspan: {}, rowspan: {}, {fill}inset: (left: {}, top: {}, right: {}, bottom: {}), align: {})[{}],",
            c.col,
            c.row,
            c.span.min(n - c.col),
            c.rowspan.min(t.rows.len() - c.row),
            typ::pt(inset[0]),
            typ::pt(inset[1]),
            typ::pt(inset[2]),
            typ::pt(inset[3]),
            c.text.anchor,
            text_markup(&c.text)
        );
    }
    out.push(')');
    out
}

/// Typst source with one page per slide at the deck's size.
pub fn to_typst(deck: &Deck) -> Source {
    let mut out = String::from(typ::PRELUDE);
    let _ = writeln!(out, "#set page(width: {}, height: {}, margin: 0pt)\n#set text(font: ({}), size: 18pt, top-edge: \"ascender\", bottom-edge: \"descender\", hyphenate: false)\n#set par(spacing: 0pt)\n#set block(spacing: 0pt)", typ::pt(deck.width), typ::pt(deck.height), typ::FALLBACK);
    for slide in &deck.slides {
        let (fill, picture) = match &slide.background {
            Background::Color(c) => (color_expr(c), String::new()),
            Background::Image(i) => match deck.images[*i].kind {
                Some(k) => ("white".into(), format!("#place(top + left, image({}, width: 100%, height: 100%, fit: \"cover\"))\n", typ::s(&format!("/img/{i}.{k}")))),
                None => ("white".into(), String::new()),
            },
        };
        let _ = writeln!(out, "#page(fill: {fill})[{picture}");
        for s in &slide.shapes {
            let mut e = shape_markup(s, &deck.images);
            if s.rot.abs() > 0.01 {
                e = format!("rotate({:.2}deg, reflow: false, {e})", s.rot);
            }
            let _ = writeln!(out, "#place(top + left, dx: {}, dy: {}, {e})", typ::pt(s.x), typ::pt(s.y));
        }
        out.push_str("]\n");
    }
    if deck.slides.is_empty() {
        out.push_str("#page[]\n");
    }
    let files = deck.images.iter().enumerate().filter_map(|(i, img)| img.kind.map(|k| (format!("/img/{i}.{k}"), img.bytes.clone()))).collect();
    Source { text: out, files }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::office::docx::tests::{PNG, zip};

    const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

    fn rels(items: &[(&str, &str, &str)]) -> String {
        let mut s = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
        for (id, kind, target) in items {
            let _ = write!(s, r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#);
        }
        s.push_str("</Relationships>");
        s
    }

    pub fn sample_pptx() -> Vec<u8> {
        let pres = format!(r#"<p:presentation {NS}><p:sldMasterIdLst><p:sldMasterId r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#);
        let master = format!(
            r#"<p:sldMaster {NS}><p:cSld><p:bg><p:bgPr><a:solidFill><a:schemeClr val="bg1"/></a:solidFill></p:bgPr></p:bg><p:spTree>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="10515600" cy="1325563"/></a:xfrm></p:spPr></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="3" name="Body"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="838200" y="1825625"/><a:ext cx="10515600" cy="4351338"/></a:xfrm></p:spPr></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="9" name="Band"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="6600000"/><a:ext cx="12192000" cy="258000"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></p:spPr></p:sp>
</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>
<p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz="4400"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mj-lt"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr marL="228600" indent="-228600"><a:buChar char="•"/><a:defRPr sz="2800"/></a:lvl1pPr><a:lvl2pPr marL="685800" indent="-228600"><a:buChar char="–"/><a:defRPr sz="2400"/></a:lvl2pPr></p:bodyStyle><p:otherStyle/></p:txStyles></p:sldMaster>"#
        );
        let layout = format!(r#"<p:sldLayout {NS}><p:cSld><p:spTree/></p:cSld></p:sldLayout>"#);
        let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="x"><a:dk1><a:srgbClr val="111111"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:hlink><a:srgbClr val="0563C1"/></a:hlink></a:clrScheme><a:fontScheme name="x"><a:majorFont><a:latin typeface="Calibri Light"/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/></a:minorFont></a:fontScheme><a:fmtScheme><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#;
        let slide1 = format!(
            r#"<p:sld {NS}><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:rPr lang="de-DE"/><a:t>Willkommen</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="3" name="B"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>Erster Punkt</a:t></a:r></a:p><a:p><a:pPr lvl="1"/><a:r><a:rPr b="1"/><a:t>Unterpunkt</a:t></a:r></a:p></p:txBody></p:sp>
<p:pic><p:nvPicPr><p:cNvPr id="4" name="P"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId5"/></p:blipFill><p:spPr><a:xfrm><a:off x="9144000" y="4572000"/><a:ext cx="1270000" cy="1270000"/></a:xfrm></p:spPr></p:pic>
<p:sp><p:nvSpPr><p:cNvPr id="5" name="O"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="127000" y="127000"/><a:ext cx="635000" cy="635000"/></a:xfrm><a:prstGeom prst="ellipse"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></p:spPr></p:sp>
</p:spTree></p:cSld></p:sld>"#
        );
        let slide2 = format!(r#"<p:sld {NS}><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="1F2937"/></a:solidFill></p:bgPr></p:bg><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:rPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></a:rPr><a:t>Zweite Folie</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#);
        let root = rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]);
        let pres_rels = rels(&[("rId1", "slideMaster", "slideMasters/slideMaster1.xml"), ("rId2", "slide", "slides/slide1.xml"), ("rId3", "slide", "slides/slide2.xml")]);
        let slide_rels = rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"), ("rId5", "image", "../media/image1.png")]);
        let slide2_rels = rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")]);
        let layout_rels = rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]);
        let master_rels = rels(&[("rId1", "theme", "../theme/theme1.xml")]);
        zip(&[
            ("_rels/.rels", root.as_bytes()),
            ("ppt/presentation.xml", pres.as_bytes()),
            ("ppt/_rels/presentation.xml.rels", pres_rels.as_bytes()),
            ("ppt/slideMasters/slideMaster1.xml", master.as_bytes()),
            ("ppt/slideMasters/_rels/slideMaster1.xml.rels", master_rels.as_bytes()),
            ("ppt/slideLayouts/slideLayout1.xml", layout.as_bytes()),
            ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", layout_rels.as_bytes()),
            ("ppt/theme/theme1.xml", theme.as_bytes()),
            ("ppt/slides/slide1.xml", slide1.as_bytes()),
            ("ppt/slides/_rels/slide1.xml.rels", slide_rels.as_bytes()),
            ("ppt/slides/slide2.xml", slide2.as_bytes()),
            ("ppt/slides/_rels/slide2.xml.rels", slide2_rels.as_bytes()),
            ("ppt/media/image1.png", PNG),
        ])
    }

    #[test]
    fn reads_slides_with_inherited_positions_styles_pictures_and_fills() {
        let deck = parse(sample_pptx()).unwrap();
        assert_eq!((deck.width, deck.height, deck.slides.len()), (960., 540., 2));
        let s = &deck.slides[0];
        let band = s.shapes.iter().find(|x| !x.placeholder && x.fill.as_ref().is_some_and(|f| f.hex == "4472c4")).unwrap();
        assert!((band.y - 519.69).abs() < 0.1);
        let title = s.shapes.iter().find(|x| x.text.as_ref().is_some_and(|t| t.paras[0].runs[0].text == "Willkommen")).unwrap();
        assert!((title.x - 66.).abs() < 0.1 && (title.w - 828.).abs() < 0.1);
        let run = &title.text.as_ref().unwrap().paras[0].runs[0].style;
        assert_eq!((run.size, run.font.as_deref(), run.color.as_ref().map(|c| c.hex.as_str())), (Some(44.), Some("Calibri Light"), Some("111111")));
        let body = s.shapes.iter().find(|x| x.text.as_ref().is_some_and(|t| t.paras.len() == 2)).unwrap().text.as_ref().unwrap();
        assert_eq!(body.paras[0].bullet.as_ref().map(|b| b.0.as_str()), Some("•"));
        assert_eq!(body.paras[1].bullet.as_ref().map(|b| b.0.as_str()), Some("–"));
        assert_eq!((body.paras[1].runs[0].style.size, body.paras[1].runs[0].style.bold), (Some(24.), Some(true)));
        assert!((body.paras[1].mar_l - 54.).abs() < 0.01);
        let pic = s.shapes.iter().find(|x| x.image.is_some()).unwrap();
        assert!((pic.x - 720.).abs() < 0.1 && (pic.w - 100.).abs() < 0.1);
        assert!(s.shapes.iter().any(|x| x.geom == Geom::Ellipse && x.fill.as_ref().is_some_and(|f| f.hex == "ff0000")));
        assert_eq!(deck.slides[1].background, Background::Color(Color { hex: "1f2937".into(), alpha: 1. }));
        assert_eq!(s.background, Background::Color(Color { hex: "ffffff".into(), alpha: 1. }));
    }

    #[test]
    fn renders_one_page_per_slide_at_its_size() {
        let source = to_typst(&parse(sample_pptx()).unwrap());
        let pdf = crate::core::pdf_export::compile(&source.text, source.files).unwrap();
        let d = crate::core::pdf_render::PdfDoc::from_data(std::sync::Arc::new(pdf)).unwrap();
        assert_eq!(d.page_count(), 2);
        let (w, h) = d.page_size(1).unwrap();
        assert!((w - 960.).abs() < 1. && (h - 540.).abs() < 1.);
        assert!(d.page_text(0).slice(0, d.page_text(0).glyph_count()).contains("Willkommen"));
    }

    #[test]
    fn broken_decks_fail_cleanly() {
        assert_eq!(parse(b"PK\x03\x04garbage".to_vec()).err(), Some(OpenError::Broken));
        assert_eq!(parse(zip(&[("x.txt", b"x")])).err(), Some(OpenError::Broken));
    }
}
