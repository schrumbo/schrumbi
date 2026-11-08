use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The file in the workspace that holds all highlights. PDFs stay untouched, they may be submissions.
pub const FILE_NAME: &str = ".schrumbi-highlights.json";

/// The part of a highlight on one page; rectangles are x, y, width and height in page points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HighlightPart {
    pub page: usize,
    pub rects: Vec<[f32; 4]>,
}

/// A standalone note at a point of a page, in page points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    pub page: usize,
    pub x: f32,
    pub y: f32,
}

/// A freehand stroke on one page: its width and its points, in page points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InkLine {
    pub page: usize,
    pub width: f32,
    pub points: Vec<[f32; 2]>,
}

/// A mark in a PDF: a marked passage, a note at a point, or a freehand drawing, each with an optional comment. Coordinates are page points as the page is shown, origin top left.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub id: u64,
    pub color: String,
    pub text: String,
    pub parts: Vec<HighlightPart>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<Pin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ink: Vec<InkLine>,
}

const PIN_REACH: f32 = 10.;
const INK_REACH: f32 = 4.;

fn distance_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    let t = if length > 0. { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length).clamp(0., 1.) } else { 0. };
    ((p[0] - (a[0] + t * dx)).powi(2) + (p[1] - (a[1] + t * dy)).powi(2)).sqrt()
}

impl Highlight {
    /// Whether a point on a page hits the marked text, the pin or the drawing.
    pub fn covers(&self, page: usize, x: f32, y: f32) -> bool {
        let text = self.parts.iter().filter(|p| p.page == page).any(|p| p.rects.iter().any(|r| x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3]));
        let pin = self.pin.is_some_and(|p| p.page == page && ((p.x - x).powi(2) + (p.y - y).powi(2)).sqrt() <= PIN_REACH);
        text || pin || self.ink.iter().filter(|l| l.page == page).any(|l| {
            let reach = l.width.max(INK_REACH);
            match l.points.as_slice() {
                [] => false,
                [only] => distance_to_segment([x, y], *only, *only) <= reach,
                points => points.windows(2).any(|w| distance_to_segment([x, y], w[0], w[1]) <= reach),
            }
        })
    }

    /// Where the highlight starts: page, then position on it. A pin or a drawing starts at its point, as a rectangle without size.
    pub fn start(&self) -> Option<(usize, [f32; 4])> {
        self.parts
            .iter()
            .find_map(|p| Some((p.page, *p.rects.first()?)))
            .or_else(|| self.pin.map(|p| (p.page, [p.x, p.y, 0., 0.])))
            .or_else(|| self.ink.iter().find_map(|l| Some((l.page, [l.points.first()?[0], l.points.first()?[1], 0., 0.]))))
    }
}

/// Highlights per document.
pub type Highlights = HashMap<String, Vec<Highlight>>;

/// Identifies a document by its content, so highlights survive renaming and moving it, also outside the app.
pub fn document_key(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Highlight {
        Highlight { id: 1, color: "yellow".into(), text: "Wort".into(), parts: vec![HighlightPart { page: 2, rects: vec![[10., 20., 30., 5.]] }], ..Default::default() }
    }

    fn pin() -> Highlight {
        Highlight { id: 2, color: "blue".into(), note: "Frage".into(), pin: Some(Pin { page: 1, x: 100., y: 200. }), ..Default::default() }
    }

    fn ink() -> Highlight {
        Highlight { id: 3, color: "red".into(), ink: vec![InkLine { page: 0, width: 2., points: vec![[10., 10.], [50., 10.], [50., 50.]] }], ..Default::default() }
    }

    #[test]
    fn covers_only_its_rectangles_on_its_page() {
        let h = sample();
        assert!(h.covers(2, 15., 22.));
        assert!(!h.covers(1, 15., 22.));
        assert!(!h.covers(2, 41., 22.));
        assert_eq!(h.start(), Some((2, [10., 20., 30., 5.])));
    }

    #[test]
    fn a_pin_is_hit_within_ten_points_on_its_page() {
        let h = pin();
        assert!(h.covers(1, 100., 200.));
        assert!(h.covers(1, 106., 208.));
        assert!(!h.covers(1, 108., 208.));
        assert!(!h.covers(0, 100., 200.));
        assert_eq!(h.start(), Some((1, [100., 200., 0., 0.])));
    }

    #[test]
    fn ink_is_hit_near_a_segment_and_never_beyond_its_ends() {
        let h = ink();
        assert!(h.covers(0, 30., 10.));
        assert!(h.covers(0, 30., 13.9));
        assert!(!h.covers(0, 30., 14.5));
        assert!(h.covers(0, 50., 30.));
        assert!(h.covers(0, 53.9, 50.));
        assert!(!h.covers(0, 56., 50.));
        assert!(!h.covers(0, 7., 7.), "the corner past the start is out of reach");
        assert!(!h.covers(1, 30., 10.));
        assert_eq!(h.start(), Some((0, [10., 10., 0., 0.])));
    }

    #[test]
    fn thick_ink_reaches_as_far_as_it_is_wide() {
        let mut h = ink();
        h.ink[0].width = 8.;
        assert!(h.covers(0, 30., 17.9));
        assert!(!h.covers(0, 30., 18.5));
    }

    #[test]
    fn a_single_point_and_an_empty_line_do_not_panic() {
        let mut h = ink();
        h.ink[0].points = vec![[5., 5.]];
        assert!(h.covers(0, 7., 5.));
        assert!(!h.covers(0, 20., 5.));
        h.ink[0].points.clear();
        assert!(!h.covers(0, 5., 5.));
        assert_eq!(h.start(), None);
    }

    #[test]
    fn text_comes_before_pin_and_ink_when_a_mark_starts() {
        let mut h = sample();
        h.pin = pin().pin;
        h.ink = ink().ink;
        assert_eq!(h.start(), Some((2, [10., 20., 30., 5.])));
    }

    #[test]
    fn old_files_load_and_new_fields_stay_out_of_unchanged_marks() {
        let old = r#"{"k":[{"id":1,"color":"yellow","text":"Wort","parts":[{"page":2,"rects":[[10.0,20.0,30.0,5.0]]}]}]}"#;
        let store: Highlights = serde_json::from_str(old).unwrap();
        assert_eq!(store["k"], vec![sample()]);
        let written = serde_json::to_string(&store).unwrap();
        assert!(!written.contains("note") && !written.contains("pin") && !written.contains("ink"), "{written}");
        let full: Highlights = HashMap::from([("k".to_string(), vec![pin(), ink()])]);
        let back: Highlights = serde_json::from_str(&serde_json::to_string(&full).unwrap()).unwrap();
        assert_eq!(back, full);
    }

    #[test]
    fn older_versions_ignore_the_new_fields() {
        #[derive(Deserialize)]
        struct Old {
            id: u64,
            parts: Vec<HighlightPart>,
        }
        let json = serde_json::to_string(&pin()).unwrap();
        let old: Old = serde_json::from_str(&json).unwrap();
        assert_eq!((old.id, old.parts.len()), (2, 0));
    }

    #[test]
    fn key_follows_the_content() {
        assert_eq!(document_key(b"a"), document_key(b"a"));
        assert_ne!(document_key(b"a"), document_key(b"b"));
    }

    #[test]
    fn survives_saving_and_loading() {
        let dir = std::env::temp_dir().join(format!("schrumbi-highlights-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        let store: Highlights = HashMap::from([("k".to_string(), vec![sample()])]);
        crate::core::workspace::save(&store, &path).unwrap();
        assert_eq!(crate::core::workspace::load::<Highlights>(&path).unwrap(), Some(store));
    }
}
