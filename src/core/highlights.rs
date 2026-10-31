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

/// A marked passage of a PDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub id: u64,
    pub color: String,
    pub text: String,
    pub parts: Vec<HighlightPart>,
}

impl Highlight {
    pub fn covers(&self, page: usize, x: f32, y: f32) -> bool {
        self.parts.iter().filter(|p| p.page == page).any(|p| p.rects.iter().any(|r| x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3]))
    }

    /// Where the highlight starts: page, then position on it.
    pub fn start(&self) -> Option<(usize, [f32; 4])> {
        let part = self.parts.first()?;
        Some((part.page, *part.rects.first()?))
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
        Highlight { id: 1, color: "yellow".into(), text: "Wort".into(), parts: vec![HighlightPart { page: 2, rects: vec![[10., 20., 30., 5.]] }] }
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
