use std::path::{Path, PathBuf};

use super::pagination::{SHIFTS, count, filler, page_of, paginate, scratch};

const TEXT_WIDTH: f64 = 595.276 - 112.0;
const TALLEST: f64 = 420.5;

fn picture(dir: &Path, name: &str, width: u32, height: u32) {
    image::RgbImage::from_pixel(width, height, image::Rgb([90, 140, 220])).save(dir.join(name)).unwrap();
}

fn folder(name: &str) -> PathBuf {
    let dir = scratch(name);
    picture(&dir, "quer.png", 1600, 900);
    picture(&dir, "hoch.png", 600, 1000);
    picture(&dir, "mittel.png", 300, 200);
    picture(&dir, "icon.png", 24, 24);
    dir
}

fn shown(markdown: &str, dir: &Path) -> Vec<(f64, f64, f64, f64)> {
    paginate(markdown, dir).iter().flat_map(|p| p.images.clone()).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.6
}

#[test]
fn an_explicit_width_is_honored_and_the_aspect_kept() {
    let dir = folder("width");
    let [(_, _, w, h)] = shown("![[mittel.png|200]]\n", &dir)[..] else { panic!("one picture") };
    assert!(close(w, 150.0) && close(h, 100.0), "{w} x {h}");
    let [(_, _, w, h)] = shown("![Beschreibung|240](mittel.png)\n", &dir)[..] else { panic!("one picture") };
    assert!(close(w, 180.0) && close(h, 120.0), "{w} x {h}");
    let [(_, _, w, h)] = shown("![[hoch.png|200x100]]\n", &dir)[..] else { panic!("one picture") };
    assert!(close(w, 45.0) && close(h, 75.0), "{w} x {h}");
    let [(_, _, w, _)] = shown("<img src=\"mittel.png\" width=\"120\">\n", &dir)[..] else { panic!("one picture") };
    assert!(close(w, 90.0), "{w}");
}

#[test]
fn big_pictures_fit_the_text_and_never_exceed_the_page() {
    let dir = folder("fit");
    let [(x, _, w, h)] = shown("![](quer.png)\n", &dir)[..] else { panic!("one picture") };
    assert!(close(x, 56.0) && close(w, TEXT_WIDTH) && close(h, TEXT_WIDTH * 9.0 / 16.0), "{x} {w} {h}");
    let [(_, _, w, h)] = shown("![](hoch.png)\n", &dir)[..] else { panic!("one picture") };
    assert!(h <= TALLEST && close(w / h, 0.6), "{w} x {h}");
    let [(_, _, w, h)] = shown("![[hoch.png|1000]]\n", &dir)[..] else { panic!("one picture") };
    assert!(h <= TALLEST && close(w / h, 0.6), "{w} x {h}");
}

#[test]
fn small_sized_pictures_stay_in_the_line_and_big_ones_stand_apart() {
    let dir = folder("inline");
    let pages = paginate("Vorher ![|20](icon.png) nachher und weiter Text.\n\nDavor ![[mittel.png|200]] Danach\n", &dir);
    let lines = &pages[0].lines;
    assert!(lines.iter().any(|(_, l)| l.contains("Vorher") && l.contains("nachher")), "{lines:?}");
    assert!(lines.iter().any(|(_, l)| l.contains("Davor")) && lines.iter().any(|(_, l)| l.contains("Danach")));
    assert!(!lines.iter().any(|(_, l)| l.contains("Davor") && l.contains("Danach")), "{lines:?}");
    assert_eq!(pages[0].images.len(), 2);
}

#[test]
fn pictures_in_a_row_flow_side_by_side_and_wrap() {
    let dir = folder("row");
    let row = shown("![[mittel.png|100]] ![[mittel.png|100]] ![[mittel.png|100]]\n", &dir);
    assert_eq!(row.len(), 3);
    assert!(row.iter().all(|p| close(p.1, row[0].1)), "{row:?}");
    assert!(row[0].0 < row[1].0 && row[1].0 < row[2].0);
    let lines = shown("![[mittel.png|200]]\n![[mittel.png|200]]\n![[mittel.png|200]]\n![[mittel.png|200]]\n", &dir);
    assert_eq!(lines.len(), 4);
    let mut rows: Vec<f64> = lines.iter().map(|p| p.1).collect();
    rows.dedup_by(|a, b| close(*a, *b));
    assert_eq!(rows.len(), 2, "{lines:?}");
}

#[test]
fn a_plain_caption_stays_with_its_picture() {
    let dir = folder("caption");
    for n in SHIFTS {
        let pages = paginate(&format!("{}![[hoch.png|300]]\n\nAbbildung 1: Hochformat Qq\n\nEnde\n", filler(n)), &dir);
        let picture = pages.iter().position(|p| !p.images.is_empty()).unwrap();
        assert_eq!(picture, page_of(&pages, "Qq"), "caption apart at shift {n}");
    }
}

#[test]
fn pictures_near_a_page_end_move_whole_and_leave_no_empty_page() {
    let dir = folder("end");
    for n in SHIFTS {
        let pages = paginate(&format!("{}![](hoch.png)\n\nEnde\n![](quer.png)\n\nSchluss\n", filler(n)), &dir);
        assert!(pages.len() <= 4, "{} pages at shift {n}", pages.len());
        assert_eq!(count(&pages, "Fuell"), n, "text lost at shift {n}");
        for (i, p) in pages.iter().enumerate() {
            assert!(!p.lines.is_empty() || !p.images.is_empty(), "page {i} empty at shift {n}");
            for (_, y, _, h) in &p.images {
                assert!(y + h <= p.height - 56.0 + 0.6, "picture over the margin at shift {n}: {y} + {h}");
            }
        }
    }
}

#[test]
fn pictures_inside_lists_and_callouts_stay_inside_the_text() {
    let dir = folder("nested");
    let pictures = shown("- Punkt\n- Mit Bild ![](quer.png)\n\n> [!tip] Tipp\n> ![](quer.png)\n", &dir);
    assert_eq!(pictures.len(), 2);
    for (x, _, w, _) in pictures {
        assert!(x >= 56.0 - 0.1 && x + w <= 56.0 + TEXT_WIDTH + 0.1, "{x} + {w}");
    }
}

#[test]
fn a_caption_pairs_only_with_the_picture_right_before_it() {
    let dir = folder("pairing");
    let items: String = (0..90).map(|i| format!("- Ip{i}\n")).collect();
    let pages = paginate(&format!("![](hoch.png)\n\n## Abschnitt\n\n{items}\n*kursiv*\n"), &dir);
    assert_eq!(count(&pages, "Ip"), 90);
    let long = format!("![](hoch.png)\n\n*Hinweis:* {}\n", "Wq ".repeat(400));
    let pages = paginate(&long, &dir);
    assert!(count(&pages, "Wq") >= 12);
    for text in ["![](mittel.png)\n\n> *erstes*\n>\n> zweites\n", "> [!tip] Tipp\n> ![](mittel.png)\n\n*cap*\n"] {
        let (source, _) = super::build(text, &dir, None, &super::Options { accent: "#0EA5E9".into(), header: false, page_numbers: false, lang: "de".into(), properties: false }, false);
        assert!(!source.contains("#keepfig["), "{text}");
    }
}

#[test]
fn odd_sizes_never_cost_the_other_pictures() {
    let dir = folder("odd");
    let pictures = shown("![[mittel.png|0x100]]\n\n![nan](mittel.png)\n\n![[mittel.png|100xinf]]\n\n![[mittel.png|-5]]\n", &dir);
    assert_eq!(pictures.len(), 4);
}

#[test]
fn a_wide_flat_picture_in_a_line_stays_inside_the_text() {
    let dir = folder("flat");
    let [(x, _, w, _)] = shown("Text ![[quer.png|800x10]] mehr\n", &dir)[..] else { panic!("one picture") };
    assert!(x + w <= 56.0 + TEXT_WIDTH + 0.1, "{x} + {w}");
}

#[test]
fn an_unquoted_html_source_with_a_folder_is_found() {
    let dir = folder("unquoted");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    picture(&dir.join("sub"), "mittel.png", 300, 200);
    let [(_, _, w, _)] = shown("<img src=sub/mittel.png width=200>\n", &dir)[..] else { panic!("one picture") };
    assert!(close(w, 150.0), "{w}");
}
