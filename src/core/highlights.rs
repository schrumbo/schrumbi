use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::workspace;

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

/// The sidecar with everything the first release does not know: notes, pins and drawings. That release rewrites `FILE_NAME` from what it knows and never reads this file.
pub const ANNOTATIONS_NAME: &str = ".schrumbi-annotations.json";

/// The version of the sidecar layout; a newer file is never written.
pub const ANNOTATIONS_VERSION: u32 = 1;

/// Ids of pins and drawings start here, far above any id the first release hands out as its highest plus one.
pub const SIDECAR_ID_BASE: u64 = 1 << 52;

/// Key of a version marker that an earlier build wrote into `FILE_NAME`; dropped on load.
pub const VERSION_KEY: &str = "~version:";

/// What the sidecar adds to one marked passage of `FILE_NAME`. It counts only while the passage still has the same text and parts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Attached {
    #[serde(default)]
    pub note: String,
    pub text_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<Pin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ink: Vec<InkLine>,
}

/// The sidecar part of one document: additions to marked passages by their id, and the marks that have no passage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentAnnotations {
    #[serde(default)]
    pub notes: BTreeMap<u64, Attached>,
    #[serde(default)]
    pub items: Vec<Highlight>,
}

/// The content of the sidecar file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotations {
    pub version: u32,
    #[serde(default)]
    pub documents: BTreeMap<String, DocumentAnnotations>,
}

impl Default for Annotations {
    fn default() -> Self {
        Self { version: ANNOTATIONS_VERSION, documents: BTreeMap::new() }
    }
}

/// Notes that found no passage, per document and id. They stay in the file and are not shown.
pub type Orphans = BTreeMap<String, BTreeMap<u64, Attached>>;

/// Why the marks files are not written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lock {
    Newer,
    Unreadable,
}

/// What was read from the two files, and what is believed to be on disk now.
#[derive(Default)]
pub struct Loaded {
    pub marks: Highlights,
    pub orphans: Orphans,
    pub disk: (Highlights, Annotations),
    pub lock: Option<Lock>,
}

/// Identifies a passage by its text and parts, so a note only follows the passage it was written on.
pub fn fingerprint(h: &Highlight) -> String {
    let unsigned = |v: f32| if v == 0. { 0. } else { v };
    let parts: Vec<HighlightPart> = h.parts.iter().map(|p| HighlightPart { page: p.page, rects: p.rects.iter().map(|r| r.map(unsigned)).collect() }).collect();
    let json = serde_json::to_string(&(&h.text, &parts)).unwrap_or_default();
    Sha256::digest(json.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

const ID_BITS: u32 = 40;

/// A sidecar id derived from `seed`, the first free one from there on. The same seed and the same taken ids always give the same id.
pub fn derived_id(seed: &[u8], taken: impl Fn(u64) -> bool) -> u64 {
    let digest = Sha256::digest(seed);
    let mut id = SIDECAR_ID_BASE + (u64::from_be_bytes([0, 0, 0, digest[0], digest[1], digest[2], digest[3], digest[4]]) & ((1 << ID_BITS) - 1));
    while taken(id) {
        id += 1;
    }
    id
}

/// A random sidecar id that is not taken, so two devices hand out different ones.
pub fn random_id(taken: impl Fn(u64) -> bool) -> u64 {
    derived_id(uuid::Uuid::new_v4().as_bytes(), taken)
}

/// Joins the main file and the sidecar into the marks as the app shows them, and the notes that fit no passage.
pub fn merge(main: &Highlights, annotations: &Annotations) -> (Highlights, Orphans) {
    let mut marks = Highlights::new();
    let mut orphans = Orphans::new();
    let keys: BTreeSet<&String> = main.keys().chain(annotations.documents.keys()).collect();
    for key in keys {
        let extra = annotations.documents.get(key);
        let mut used = BTreeSet::new();
        let mut list: Vec<Highlight> = main.get(key).cloned().unwrap_or_default();
        for h in &mut list {
            if let Some(a) = extra.and_then(|e| e.notes.get(&h.id)).filter(|a| a.text_fingerprint == fingerprint(h)) {
                used.insert(h.id);
                h.note = a.note.clone();
                h.pin = a.pin;
                h.ink = a.ink.clone();
            }
        }
        if let Some(extra) = extra {
            let lost: BTreeMap<u64, Attached> = extra.notes.iter().filter(|(id, _)| !used.contains(id)).map(|(id, a)| (*id, a.clone())).collect();
            if !lost.is_empty() {
                orphans.insert(key.clone(), lost);
            }
            let mut taken: BTreeSet<u64> = list.iter().chain(&extra.items).map(|h| h.id).collect();
            let passages: BTreeSet<u64> = list.iter().map(|h| h.id).collect();
            let mut seen = BTreeSet::new();
            for item in &extra.items {
                let mut item = item.clone();
                if item.id < SIDECAR_ID_BASE && (passages.contains(&item.id) || !seen.insert(item.id)) {
                    let seed = serde_json::to_vec(&item).unwrap_or_default();
                    item.id = derived_id(&seed, |id| taken.contains(&id));
                    taken.insert(item.id);
                }
                list.push(item);
            }
        }
        if !list.is_empty() {
            marks.insert(key.clone(), list);
        }
    }
    (marks, orphans)
}

/// Splits the marks for saving: the main file gets the passages as the first release writes them, the sidecar everything else plus the orphaned notes.
pub fn split(marks: &Highlights, orphans: &Orphans) -> (Highlights, Annotations) {
    let mut main = Highlights::new();
    let mut annotations = Annotations::default();
    for (key, list) in marks {
        let mut extra = DocumentAnnotations::default();
        let mut plain = Vec::new();
        for h in list {
            if h.parts.is_empty() {
                extra.items.push(h.clone());
                continue;
            }
            if !h.note.is_empty() || h.pin.is_some() || !h.ink.is_empty() {
                extra.notes.insert(h.id, Attached { note: h.note.clone(), text_fingerprint: fingerprint(h), pin: h.pin, ink: h.ink.clone() });
            }
            plain.push(Highlight { id: h.id, color: h.color.clone(), text: h.text.clone(), parts: h.parts.clone(), ..Default::default() });
        }
        if !plain.is_empty() {
            main.insert(key.clone(), plain);
        }
        if !extra.notes.is_empty() || !extra.items.is_empty() {
            annotations.documents.insert(key.clone(), extra);
        }
    }
    for (key, lost) in orphans {
        let extra = annotations.documents.entry(key.clone()).or_default();
        let mut taken: BTreeSet<u64> = extra.notes.keys().copied().chain(extra.items.iter().map(|h| h.id)).chain(lost.keys().copied()).collect();
        for (id, a) in lost {
            if extra.notes.contains_key(id) {
                let seed = format!("{key}
{id}
{}", a.text_fingerprint);
                let moved = derived_id(seed.as_bytes(), |candidate| taken.contains(&candidate));
                taken.insert(moved);
                extra.notes.insert(moved, a.clone());
            } else {
                extra.notes.insert(*id, a.clone());
            }
        }
    }
    (main, annotations)
}

/// A free id for a new mark: passages count up from the highest id below the reserved range, pins and drawings get a random one inside it. Ids of orphaned notes count as taken.
pub fn next_id(list: &[Highlight], orphans: Option<&BTreeMap<u64, Attached>>, passage: bool) -> u64 {
    let ids = list.iter().map(|h| h.id).chain(orphans.into_iter().flat_map(|o| o.keys().copied()));
    if passage {
        ids.filter(|id| *id < SIDECAR_ID_BASE).max().map_or(1, |m| m + 1)
    } else {
        let taken: BTreeSet<u64> = ids.collect();
        random_id(|id| taken.contains(&id))
    }
}

/// Reads both files of a workspace. A file that cannot be read or comes from a newer version locks the marks, so nothing is written over it. A sidecar that cannot be used leaves the plain highlights visible.
pub fn load(root: &Path) -> Loaded {
    let mut raw = match workspace::load::<Highlights>(&root.join(FILE_NAME)) {
        Ok(found) => found.unwrap_or_default(),
        Err(_) => return Loaded { lock: Some(Lock::Unreadable), ..Default::default() },
    };
    let disk_main = raw.clone();
    raw.retain(|key, _| !key.starts_with(VERSION_KEY));
    let locked = |raw: &Highlights, lock: Lock| {
        let (marks, _) = merge(raw, &Annotations::default());
        Loaded { marks, disk: (disk_main.clone(), Annotations::default()), lock: Some(lock), ..Default::default() }
    };
    let annotations = match workspace::load::<serde_json::Value>(&root.join(ANNOTATIONS_NAME)) {
        Ok(None) => Annotations::default(),
        Ok(Some(value)) => {
            if value.get("version").and_then(|v| v.as_u64()).is_some_and(|v| v > u64::from(ANNOTATIONS_VERSION)) {
                return locked(&raw, Lock::Newer);
            }
            match serde_json::from_value::<Annotations>(value) {
                Ok(annotations) => annotations,
                Err(_) => return locked(&raw, Lock::Unreadable),
            }
        }
        Err(_) => return locked(&raw, Lock::Unreadable),
    };
    let (marks, orphans) = merge(&raw, &annotations);
    Loaded { marks, orphans, disk: (disk_main, annotations), lock: None }
}

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

    fn rich() -> Vec<Highlight> {
        let mut noted = sample();
        noted.note = "Merken".into();
        let mut pinned = pin();
        pinned.id = SIDECAR_ID_BASE;
        let mut drawn = ink();
        drawn.id = SIDECAR_ID_BASE + 1;
        vec![noted, pinned, drawn]
    }

    #[test]
    fn the_main_file_keeps_the_shape_of_the_first_release_and_the_marks_come_back_whole() {
        let marks: Highlights = HashMap::from([("k".to_string(), rich())]);
        let (main, annotations) = split(&marks, &Orphans::new());
        assert_eq!(main["k"], vec![sample()]);
        let json = serde_json::to_string(&main).unwrap();
        assert!(!json.contains("note") && !json.contains("pin") && !json.contains("ink"), "{json}");
        assert_eq!(annotations.documents["k"].items.len(), 2);
        assert_eq!(annotations.documents["k"].notes[&1].note, "Merken");
        let (back, orphans) = merge(&main, &annotations);
        assert_eq!((back, orphans), (marks, Orphans::new()));
    }

    #[test]
    fn a_note_belongs_to_its_passage_only_and_is_kept_when_it_finds_none() {
        let marks: Highlights = HashMap::from([("k".to_string(), rich())]);
        let (mut main, annotations) = split(&marks, &Orphans::new());
        main.get_mut("k").unwrap()[0].text = "anderes Wort".into();
        let (shown, orphans) = merge(&main, &annotations);
        assert!(shown["k"][0].note.is_empty(), "a passage that reuses the id gets no foreign note");
        assert_eq!(orphans["k"][&1].note, "Merken");
        let (_, saved) = split(&shown, &orphans);
        assert_eq!(saved.documents["k"].notes[&1].note, "Merken", "the orphan stays in the file");
        main.remove("k");
        let (_, orphans) = merge(&main, &annotations);
        assert_eq!(orphans["k"].len(), 1);
    }

    #[test]
    fn ids_of_new_marks_stay_apart_from_what_the_first_release_hands_out() {
        let list = rich();
        let lost = BTreeMap::from([(7, Attached::default())]);
        assert_eq!(next_id(&list, None, true), 2);
        assert_eq!(next_id(&list, Some(&lost), true), 8);
        assert_eq!(next_id(&[], None, true), 1);
        let free = |id: u64| (SIDECAR_ID_BASE..SIDECAR_ID_BASE + (1 << 40)).contains(&id);
        let (a, b) = (next_id(&list, None, false), next_id(&list, None, false));
        assert!(free(a) && free(b) && a != SIDECAR_ID_BASE && a != SIDECAR_ID_BASE + 1);
        assert_ne!(a, b, "two devices must not hand out the same id");
    }

    #[test]
    fn an_orphaned_note_stays_when_a_live_passage_with_its_id_gets_a_note() {
        let old = Attached { note: "alt".into(), text_fingerprint: "weg".into(), ..Default::default() };
        let orphans = Orphans::from([("k".to_string(), BTreeMap::from([(1, old)]))]);
        let mut noted = sample();
        noted.note = "neu".into();
        let marks: Highlights = HashMap::from([("k".to_string(), vec![noted])]);
        let (main, annotations) = split(&marks, &orphans);
        let notes: Vec<&str> = annotations.documents["k"].notes.values().map(|a| a.note.as_str()).collect();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes.contains(&"alt") && notes.contains(&"neu"));
        assert_eq!(split(&marks, &orphans).1, annotations, "saving twice writes the same file");
        let (shown, lost) = merge(&main, &annotations);
        assert_eq!(shown["k"][0].note, "neu");
        assert_eq!(lost["k"].values().map(|a| a.note.as_str()).collect::<Vec<_>>(), ["alt"]);
        assert_eq!(split(&shown, &lost).1, annotations, "and again after loading");
    }

    #[test]
    fn negative_zero_does_not_change_a_fingerprint_across_saving_and_loading() {
        let mut h = sample();
        h.parts[0].rects[0] = [-0., 20., -0., 5.];
        let mut zero = h.clone();
        zero.parts[0].rects[0] = [0., 20., 0., 5.];
        assert_eq!(fingerprint(&h), fingerprint(&zero));
        let dir = std::env::temp_dir().join(format!("schrumbi-fingerprint-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        workspace::save(&HashMap::from([("k".to_string(), vec![h.clone()])]), &path).unwrap();
        let back: Highlights = workspace::load(&path).unwrap().unwrap();
        assert_eq!(fingerprint(&back["k"][0]), fingerprint(&h));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_mark_without_parts_that_shares_a_main_file_id_gets_a_sidecar_id() {
        let main: Highlights = HashMap::from([("k".to_string(), vec![sample()])]);
        let mut shared = pin();
        shared.id = 1;
        let annotations = Annotations { documents: BTreeMap::from([("k".to_string(), DocumentAnnotations { notes: BTreeMap::new(), items: vec![shared.clone()] })]), ..Default::default() };
        let (shown, _) = merge(&main, &annotations);
        let ids: Vec<u64> = shown["k"].iter().map(|h| h.id).collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], 1);
        assert!(ids[1] >= SIDECAR_ID_BASE);
        assert_eq!(shown["k"][1].pin, shared.pin);
        let (_, again) = merge(&main, &split(&shown, &Orphans::new()).1);
        assert_eq!(again, Orphans::new());
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("schrumbi-highlights-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_broken_sidecar_locks_the_marks_but_keeps_the_plain_highlights_visible() {
        let dir = scratch("broken-sidecar");
        workspace::save(&HashMap::from([("k".to_string(), vec![sample()])]), &dir.join(FILE_NAME)).unwrap();
        std::fs::write(dir.join(ANNOTATIONS_NAME), "[1,2").unwrap();
        let loaded = load(&dir);
        assert_eq!(loaded.lock, Some(Lock::Unreadable));
        assert_eq!(loaded.marks["k"], vec![sample()]);
        assert!(loaded.disk.1 == Annotations::default());
        std::fs::write(dir.join(ANNOTATIONS_NAME), r#"{"version":1,"documents":{"k":{"notes":"kaputt"}}}"#).unwrap();
        let loaded = load(&dir);
        assert_eq!((loaded.lock, loaded.marks["k"].len()), (Some(Lock::Unreadable), 1));
        let _ = std::fs::remove_dir_all(dir);
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
