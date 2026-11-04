use std::collections::HashMap;
use std::path::Path;

use typst::foundations::Bytes;
use typst::layout::{Frame, FrameItem, Point};
use typst::syntax::Source;
use typst_layout::PagedDocument;

use super::{NoteWorld, Options, THEME, build, file_id};

pub(super) struct Page {
    pub(super) lines: Vec<(f64, String)>,
    pub(super) images: Vec<(f64, f64, f64, f64)>,
    pub(super) height: f64,
}

fn options() -> Options {
    Options { accent: "#0EA5E9".into(), header: false, page_numbers: false, lang: "de".into(), properties: false }
}

fn walk(frame: &Frame, origin: Point, texts: &mut Vec<(f64, f64, String)>, images: &mut Vec<(f64, f64, f64, f64)>) {
    for (pos, item) in frame.items() {
        let at = origin + *pos;
        match item {
            FrameItem::Group(g) => walk(&g.frame, at, texts, images),
            FrameItem::Text(t) => texts.push((at.y.to_pt(), at.x.to_pt(), t.text.to_string())),
            FrameItem::Image(_, size, _) => images.push((at.x.to_pt(), at.y.to_pt(), size.x.to_pt(), size.y.to_pt())),
            _ => {}
        }
    }
}

pub(super) fn paginate(markdown: &str, base: &Path) -> Vec<Page> {
    let (text, images) = build(markdown, base, None, &options(), false);
    let mut files = HashMap::new();
    files.insert(file_id("/theme.tmTheme"), Bytes::new(THEME.as_bytes().to_vec()));
    for (i, p) in images.iter().enumerate() {
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        files.insert(file_id(&format!("/img/{i}.{ext}")), Bytes::new(std::fs::read(p).unwrap()));
    }
    let world = NoteWorld { main: Source::new(file_id("/main.typ"), text), files };
    let doc: PagedDocument = typst::compile(&world).output.unwrap_or_else(|e| panic!("{}", e.iter().map(|d| d.message.to_string()).collect::<Vec<_>>().join("; ")));
    let pages: Vec<Page> = doc
        .pages()
        .iter()
        .map(|page| {
            let mut texts = Vec::new();
            let mut pictures = Vec::new();
            walk(&page.frame, Point::zero(), &mut texts, &mut pictures);
            texts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            let mut lines: Vec<(f64, String)> = Vec::new();
            for (y, _, t) in texts {
                match lines.last_mut() {
                    Some((ly, line)) if (y - *ly).abs() < 2.0 => line.push_str(&t),
                    _ => lines.push((y, t)),
                }
            }
            Page { lines, images: pictures, height: page.frame.height().to_pt() }
        })
        .collect();
    for (i, page) in pages.iter().enumerate() {
        if let Some((y, line)) = page.lines.iter().find(|(y, _)| *y > page.height - 56.0 + 1.0) {
            panic!("page {i}: text past the bottom margin at {y}: {line}");
        }
    }
    pages
}

pub(super) fn page_of(pages: &[Page], marker: &str) -> usize {
    pages.iter().position(|p| p.lines.iter().any(|(_, l)| l.contains(marker))).unwrap_or_else(|| panic!("{marker} missing"))
}

pub(super) fn count(pages: &[Page], marker: &str) -> usize {
    pages.iter().map(|p| p.lines.iter().filter(|(y, l)| *y <= p.height - 56.0 && l.contains(marker)).count()).sum()
}

pub(super) fn filler(n: usize) -> String {
    (0..n).map(|i| format!("Fuell{i}\n\n")).collect()
}

pub(super) const SHIFTS: std::ops::RangeInclusive<usize> = 0..=38;

fn words(marker: &str, n: usize) -> String {
    (0..n).map(|i| format!("{marker}{i}")).collect::<Vec<_>>().join(" ")
}

fn code(marker: &str, lines: usize) -> String {
    format!("```\n{}\n```\n", (0..lines).map(|i| format!("{marker}{i} = {i};")).collect::<Vec<_>>().join("\n"))
}

fn table(marker: &str, rows: usize) -> String {
    format!("| Kopfq | Wert |\n|---|---|\n{}\n", (0..rows).map(|i| format!("| {marker}{i} | {i} |")).collect::<Vec<_>>().join("\n"))
}

fn paragraphs(marker: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{marker}{i} Zeile")).collect()
}

fn callout(marker: &str, n: usize) -> String {
    format!("> [!tip] Kopfq\n{}\n", paragraphs(marker, n).iter().map(|p| format!("> {p}")).collect::<Vec<_>>().join("\n>\n"))
}

fn quote(marker: &str, n: usize) -> String {
    paragraphs(marker, n).iter().map(|p| format!("> {p}")).collect::<Vec<_>>().join("\n>\n") + "\n"
}

fn list(marker: &str, items: usize) -> String {
    (0..items).map(|i| format!("- {marker}{i}a<br>{marker}{i}b<br>{marker}{i}c\n")).collect()
}

fn tall_svg(dir: &Path, name: &str, height: u32) {
    std::fs::write(dir.join(name), format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"{height}\"><rect width=\"100\" height=\"{height}\" fill=\"#888\"/></svg>")).unwrap();
}

pub(super) fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("schrumbi-pagination-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn assert_whole(block: &str, first: &str, last: &str, tail: &str) {
    for n in SHIFTS {
        let pages = paginate(&format!("{}{block}\n{tail}", filler(n)), Path::new("/"));
        assert_eq!(page_of(&pages, first), page_of(&pages, last), "{first}..{last} split at shift {n}");
    }
}

#[test]
fn headings_stay_with_what_follows() {
    let base = scratch("headings");
    tall_svg(&base, "bild.svg", 120);
    let blocks: Vec<(&str, String)> = vec![
        ("Pq0", format!("{}\n", words("Pq", 400))),
        ("Lq0a", list("Lq", 6)),
        ("Lq0a", list("Lq", 60)),
        ("Tq0", table("Tq", 6)),
        ("Tq0", table("Tq", 70)),
        ("Cq0", code("Cq", 12)),
        ("Cq0", code("Cq", 120)),
        ("Aq0", callout("Aq", 5)),
        ("Aq0", callout("Aq", 60)),
        ("Qq0", quote("Qq", 5)),
        ("Qq0", quote("Qq", 60)),
        ("Zq", "![x](bild.svg)\n\nZq Abbildung\n".to_string()),
    ];
    for (first, block) in &blocks {
        for n in SHIFTS {
            let pages = paginate(&format!("{}# Hq1\n\n## Hq2\n\n### Hq3\n\n{block}\nEnde\n", filler(n)), &base);
            let head = page_of(&pages, "Hq1");
            assert_eq!(head, page_of(&pages, "Hq2"), "H1 and H2 apart at shift {n}\n{block}");
            assert_eq!(head, page_of(&pages, "Hq3"), "H2 and H3 apart at shift {n}\n{block}");
            let body = if first.starts_with("Zq") { pages.iter().position(|p| !p.images.is_empty()).unwrap() } else { page_of(&pages, first) };
            assert_eq!(head, body, "heading left behind at shift {n} before {first}\n{block}");
        }
    }
}

#[test]
fn short_blocks_are_not_split() {
    assert_whole(&code("Cq", 14), "Cq0", "Cq13", "Ende\n");
    assert_whole(&table("Tq", 7), "Kopfq", "Tq6", "Ende\n");
    assert_whole(&callout("Aq", 6), "Kopfq", "Aq5", "Ende\n");
    assert_whole(&quote("Qq", 6), "Qq0", "Qq5", "Ende\n");
    for item in 0..4 {
        for n in SHIFTS {
            let pages = paginate(&format!("{}{}\nEnde\n", filler(n), list("Lq", 4)), Path::new("/"));
            assert_eq!(page_of(&pages, &format!("Lq{item}a")), page_of(&pages, &format!("Lq{item}c")), "item {item} split at shift {n}");
        }
    }
}

#[test]
fn a_picture_stays_with_its_caption() {
    let base = scratch("caption");
    tall_svg(&base, "bild.svg", 200);
    for n in SHIFTS {
        let pages = paginate(&format!("{}![x](bild.svg)\n\n*Zq Abbildung*\n\nEnde\n", filler(n)), &base);
        let picture = pages.iter().position(|p| !p.images.is_empty()).unwrap();
        assert_eq!(picture, page_of(&pages, "Zq"), "caption apart at shift {n}");
    }
}

#[test]
fn paragraphs_leave_no_single_line_behind() {
    for n in SHIFTS {
        let pages = paginate(&format!("{}{}\n\nEnde\n", filler(n), "Wq ".repeat(900)), Path::new("/"));
        let per_page: Vec<usize> = pages.iter().map(|p| p.lines.iter().filter(|(_, l)| l.contains("Wq")).count()).filter(|c| *c > 0).collect();
        assert!(per_page.iter().all(|c| *c >= 2), "{per_page:?} at shift {n}");
    }
}

#[test]
fn a_long_table_repeats_its_header() {
    for n in SHIFTS {
        let pages = paginate(&format!("{}{}\nEnde\n", filler(n), table("Rq", 90)), Path::new("/"));
        assert_eq!(count(&pages, "Rq"), 90);
        for (i, p) in pages.iter().enumerate() {
            let rows = p.lines.iter().filter(|(_, l)| l.contains("Rq")).count();
            if rows > 0 {
                assert!(p.lines.iter().any(|(_, l)| l.contains("Kopfq")), "page {i} has no header at shift {n}");
                assert!(rows >= 2 || i + 1 == pages.len(), "page {i} has {rows} row at shift {n}");
            }
        }
    }
}

#[test]
fn long_blocks_split_at_lines_and_every_line_is_kept() {
    let base = scratch("long");
    tall_svg(&base, "hoch.svg", 3000);
    let text = format!(
        "# Titel\n\n{}\n{}\n{}\n{}\n{}\n- {}\n\n![x](hoch.svg)\n\nEnde\n",
        code("Cq", 220),
        callout("Aq", 120),
        quote("Qq", 120),
        table("Tq", 150),
        format!("| a |\n|---|\n| {} |\n", (0..120).map(|i| format!("Zq{i}")).collect::<Vec<_>>().join("<br>")),
        (0..120).map(|i| format!("Iq{i}")).collect::<Vec<_>>().join("<br>"),
    );
    let pages = paginate(&text, &base);
    assert!(pages.len() < 40, "{} pages", pages.len());
    for (marker, total) in [("Cq", 220), ("Aq", 120), ("Qq", 120), ("Tq", 150), ("Zq", 120), ("Iq", 120)] {
        assert_eq!(count(&pages, marker), total, "{marker}");
    }
    assert_eq!(pages.iter().map(|p| p.images.len()).sum::<usize>(), 1, "{:?}", pages.iter().map(|p| p.images.clone()).collect::<Vec<_>>());
    assert!(pages.iter().all(|p| !p.lines.is_empty() || !p.images.is_empty()), "empty page");
    for p in &pages {
        for (_, y, _, h) in &p.images {
            assert!(y + h <= p.height + 0.5, "picture runs off the page: {y} + {h}");
        }
    }
}
