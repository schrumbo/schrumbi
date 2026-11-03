use std::sync::LazyLock;

use regex::Regex;

use super::i18n::{Lang, tr};
use super::text::strip_diacritics;

/// Where a page sits in the guide's sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Start,
    Study,
    Notes,
    More,
}

impl Group {
    pub fn label(self) -> String {
        tr(match self {
            Group::Start => "Los geht’s",
            Group::Study => "Semester & Abgaben",
            Group::Notes => "Notizen & Lernen",
            Group::More => "Mehr",
        })
    }
}

/// One page of the guide: its file name in `assets/docs/<lang>/`, sidebar icon and group.
pub struct Page {
    pub slug: &'static str,
    pub icon: &'static str,
    pub group: Group,
    de: &'static str,
    en: &'static str,
}

macro_rules! pages {
    ($($slug:literal, $icon:literal, $group:ident;)*) => {
        /// Every page in reading order.
        pub const PAGES: &[Page] = &[$(Page { slug: $slug, icon: $icon, group: Group::$group, de: include_str!(concat!("../../assets/docs/de/", $slug, ".md")), en: include_str!(concat!("../../assets/docs/en/", $slug, ".md")) }),*];
    };
}

pages! {
    "erste-schritte", "rocket", Start;
    "uebersicht", "layout-grid", Start;
    "suchen", "search", Start;
    "semester", "table", Study;
    "abgaben", "calendar-check", Study;
    "ablegen", "inbox", Study;
    "notizen", "book-open", Notes;
    "markdown", "file-text", Notes;
    "karten", "layers", Notes;
    "sync", "cloud", More;
    "einstellungen", "settings", More;
    "tastenkuerzel", "keyboard", More;
}

impl Page {
    /// The page's Markdown in a language.
    pub fn source(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::De => self.de,
            Lang::En => self.en,
        }
    }
}

/// Part of a section: Markdown to read, or an example shown rendered with its source one click away.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Text(String),
    Example { title: String, source: String },
}

/// A page part that starts at a `##` heading, or the lead before the first one.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub heading: Option<String>,
    pub pieces: Vec<Piece>,
}

/// A page split for drawing and searching.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub title: String,
    pub sections: Vec<Section>,
    title_forms: String,
    index: Vec<Indexed>,
}

#[derive(Debug, Clone, PartialEq)]
struct Indexed {
    heading: Option<String>,
    body: String,
    forms: String,
}

fn indexed(section: &Section) -> Indexed {
    let prose = |piece: &Piece| match piece {
        Piece::Text(t) => plain(&t.lines().filter(|l| !l.starts_with("## ")).collect::<Vec<_>>().join("\n")),
        Piece::Example { title, .. } => title.clone(),
    };
    let mut body = String::new();
    let mut after_example = false;
    for piece in &section.pieces {
        let text = prose(piece);
        if text.is_empty() {
            continue;
        }
        let example = matches!(piece, Piece::Example { .. });
        if !body.is_empty() {
            body.push_str(if example && after_example { " · " } else { " " });
        }
        body.push_str(&text);
        after_example = example;
    }
    let sources = section.pieces.iter().filter_map(|p| if let Piece::Example { source, .. } = p { Some(plain(source)) } else { None }).collect::<Vec<_>>().join(" ");
    let forms = simple(&format!("{body} {sources}"));
    Indexed { heading: section.heading.as_deref().map(simple), body, forms }
}

const FENCE: &str = "````example";

/// Splits a page into its `# title`, sections at `##`, and ````` ````example ````` blocks.
pub fn split(source: &str) -> Doc {
    let mut title = String::new();
    let mut sections = vec![Section { heading: None, pieces: Vec::new() }];
    let mut text = String::new();
    let mut example: Option<(String, String)> = None;
    let mut in_code = false;
    let flush = |text: &mut String, sections: &mut Vec<Section>| {
        if !text.trim().is_empty() && let Some(section) = sections.last_mut() {
            section.pieces.push(Piece::Text(text.trim_end().to_string()));
        }
        text.clear();
    };
    for line in source.lines() {
        if let Some((name, body)) = example.as_mut() {
            if line.trim_end() == "````" {
                let (title, source) = (std::mem::take(name), std::mem::take(body));
                if let Some(section) = sections.last_mut() {
                    section.pieces.push(Piece::Example { title, source: source.trim_end_matches('\n').to_string() });
                }
                example = None;
            } else {
                body.push_str(line);
                body.push('\n');
            }
            continue;
        }
        if !in_code && let Some(rest) = line.strip_prefix(FENCE) {
            flush(&mut text, &mut sections);
            example = Some((rest.trim().to_string(), String::new()));
            continue;
        }
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
        }
        if !in_code && title.is_empty() && let Some(t) = line.strip_prefix("# ") {
            title = t.trim().to_string();
            continue;
        }
        if !in_code && let Some(h) = line.strip_prefix("## ") {
            flush(&mut text, &mut sections);
            sections.push(Section { heading: Some(plain(h.trim())), pieces: Vec::new() });
        }
        text.push_str(line);
        text.push('\n');
    }
    flush(&mut text, &mut sections);
    if sections[0].pieces.is_empty() && sections.len() > 1 {
        sections.remove(0);
    }
    let index = sections.iter().map(indexed).collect();
    Doc { title_forms: simple(&title), title, sections, index }
}

static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{(key|ui):([^{}\n]+)\}").expect("placeholder pattern"));

/// Every `{key:…}` and `{ui:…}` in a text, in order.
#[cfg(test)]
pub fn placeholders(source: &str) -> Vec<(String, String)> {
    PLACEHOLDER.captures_iter(source).map(|c| (c[1].to_string(), c[2].to_string())).collect()
}

/// Fills `{key:name}` with the keys `keys` gives (as `<kbd>`), and `{ui:Label}` with the translated label in bold.
pub fn expand(source: &str, keys: &dyn Fn(&str) -> Option<String>) -> String {
    PLACEHOLDER
        .replace_all(source, |c: &regex::Captures| match &c[1] {
            "key" => format!("<kbd>{}</kbd>", keys(&c[2]).unwrap_or_else(|| tr("ohne Kürzel"))),
            _ => format!("**{}**", tr(&c[2])),
        })
        .into_owned()
}

const KEYS: [&str; 9] = ["return", "escape", "tab", "space", "up", "down", "left", "right", "delete"];

/// A key written like `return`, `h` or `cmd-shift-a`, as this platform writes it; `None` for anything else.
pub fn plain_key(name: &str) -> Option<String> {
    let (mods, key) = match name.strip_suffix("--") {
        Some(rest) => (rest, "-"),
        None => name.rsplit_once('-').unwrap_or(("", name)),
    };
    let known = KEYS.contains(&key) || key.chars().count() == 1 && !key.chars().next().is_some_and(char::is_uppercase) || key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=12).contains(&n));
    if !known {
        return None;
    }
    let mut s = super::prefs::Shortcut { key: key.to_string(), command: false, shift: false, option: false, control: false };
    for m in mods.split('-').filter(|m| !m.is_empty()) {
        match m {
            "cmd" => s.command = true,
            "shift" => s.shift = true,
            "alt" => s.option = true,
            "ctrl" => s.control = true,
            _ => return None,
        }
    }
    Some(s.display())
}

static MARKUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</?kbd>|\*\*|__|`|\[\[|\]\]|^#+ |^> ?|^\s*[-*] (\[.\] )?|^\s*\d+\. |\|").expect("markup pattern"));

/// Readable text of a Markdown snippet, line by line, for search and snippets.
pub fn plain(markdown: &str) -> String {
    let rule = |l: &str| l.trim().starts_with('|') && l.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
    let lines: Vec<&str> = markdown.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut in_table = false;
    for (i, l) in lines.iter().enumerate() {
        if rule(l) || lines.get(i + 1).is_some_and(|next| rule(next)) {
            continue;
        }
        let row = l.trim().starts_with('|');
        let text = if row {
            l.split('|').map(|c| MARKUP.replace_all(c, "").trim().to_string()).filter(|c| !c.is_empty()).collect::<Vec<_>>().join(" · ")
        } else {
            MARKUP.replace_all(l, "").trim().to_string()
        };
        if text.is_empty() || text.chars().all(|c| c == '-' || c == ':' || c == ' ') {
            continue;
        }
        match out.last_mut() {
            Some(last) if row && in_table => *last = format!("{last} · {text}"),
            _ => out.push(text),
        }
        in_table = row;
    }
    out.join(" ")
}

/// A page section that matches a search.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub page: usize,
    pub section: usize,
    /// The section's heading, or `None` for the page itself.
    pub heading: Option<String>,
    /// Text around the first match.
    pub snippet: String,
    /// Byte ranges of the query words in `snippet`.
    pub marks: Vec<std::ops::Range<usize>>,
}

fn simple(s: &str) -> String {
    strip_diacritics(&s.to_lowercase()).replace('ß', "ss")
}

/// A query word as typed, and with `ae`, `oe`, `ue` read as umlauts.
fn variants(word: &str) -> Vec<String> {
    let plain = simple(word);
    let umlauts = plain.replace("ae", "a").replace("oe", "o").replace("ue", "u");
    if umlauts == plain { vec![plain] } else { vec![plain, umlauts] }
}

/// The text folded for matching, with the byte in `text` each folded byte came from.
fn folded(text: &str) -> (String, Vec<(usize, usize)>) {
    let (mut out, mut from) = (String::new(), Vec::new());
    for (i, ch) in text.char_indices() {
        let f = simple(&ch.to_string());
        from.extend(std::iter::repeat_n((i, i + ch.len_utf8()), f.len()));
        out.push_str(&f);
    }
    (out, from)
}

fn contains(hay: &str, word: &[String]) -> bool {
    word.iter().any(|w| hay.contains(w.as_str()))
}

/// Where a query word first occurs in `text`, ignoring case and umlauts.
fn find_word(text: &str, word: &str) -> Option<std::ops::Range<usize>> {
    let (hay, from) = folded(text);
    variants(word).iter().filter(|w| !w.is_empty()).filter_map(|w| hay.find(w.as_str()).map(|at| from[at].0..from[at + w.len() - 1].1)).min_by_key(|r| r.start)
}

fn snippet(text: &str, words: &[&str]) -> (String, Vec<std::ops::Range<usize>>) {
    const BEFORE: usize = 40;
    const LENGTH: usize = 140;
    let first = words.iter().filter_map(|w| find_word(text, w)).map(|r| r.start).min().unwrap_or(0);
    let part = |r: std::ops::Range<usize>| text.get(r).unwrap_or_default();
    let mut start = part(0..first).char_indices().rev().nth(BEFORE).map_or(0, |(i, _)| i);
    if start > 0 {
        start = part(start..first).find(' ').map_or(start, |s| start + s + 1);
    }
    let end = part(start..text.len()).char_indices().nth(LENGTH).map_or(text.len(), |(i, _)| start + i);
    let end = if end < text.len() { part(start..end).rfind(' ').map_or(end, |s| start + s) } else { end };
    let lead = if start > 0 { "…" } else { "" };
    let tail = if end < text.len() { "…" } else { "" };
    let shown = format!("{lead}{}{tail}", part(start..end));
    let mut marks = Vec::new();
    let longest = words.iter().map(|w| w.chars().count()).max().unwrap_or(0);
    for w in words.iter().filter(|w| longest < 2 || w.chars().count() > 1) {
        let mut from = 0;
        while let Some(r) = shown.get(from..).and_then(|rest| find_word(rest, w)) {
            marks.push(from + r.start..from + r.end);
            from += r.end;
        }
    }
    marks.sort_by_key(|r| r.start);
    let mut merged: Vec<std::ops::Range<usize>> = Vec::with_capacity(marks.len());
    for r in marks {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    (shown, merged)
}

/// Sections where every query word occurs, best first: page title, then heading, then text.
pub fn search(docs: &[Doc], query: &str) -> Vec<Hit> {
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let wanted: Vec<Vec<String>> = words.iter().map(|w| variants(w)).collect();
    let mut hits: Vec<(u32, Hit)> = Vec::new();
    for (p, doc) in docs.iter().enumerate() {
        let title = &doc.title_forms;
        for (s, (section, found)) in doc.sections.iter().zip(&doc.index).enumerate() {
            let heading = found.heading.as_ref();
            let body_forms = &found.forms;
            let mut score = 0;
            let mut all = true;
            for w in &wanted {
                let in_title = contains(title, w);
                let in_heading = heading.is_some_and(|h| contains(h, w));
                let in_body = contains(body_forms, w);
                if !(in_title || in_heading || in_body) {
                    all = false;
                    break;
                }
                score += if in_heading { 4 } else if in_title { 2 } else { 1 };
            }
            if !all {
                continue;
            }
            if s == 0 && wanted.iter().all(|w| contains(title, w)) {
                score += 8;
            }
            let text = if found.body.is_empty() { section.heading.as_deref().unwrap_or_default() } else { &found.body };
            let (snippet, marks) = snippet(text, &words);
            hits.push((score, Hit { page: p, section: s, heading: section.heading.clone(), snippet, marks }));
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.page.cmp(&b.1.page)).then(a.1.section.cmp(&b.1.section)));
    hits.into_iter().map(|(_, h)| h).take(60).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(lang: Lang, slug: &str) -> Doc {
        split(PAGES.iter().find(|p| p.slug == slug).unwrap().source(lang))
    }

    #[test]
    fn splits_title_sections_and_examples() {
        let d = split("# Titel\n\nVorweg.\n\n## Eins\n\nText\n\n````example Fett\n**fett**\n```js\nx\n```\n````\n\n## Zwei\n\n```md\n## kein Abschnitt\n````example\n```\n");
        assert_eq!(d.title, "Titel");
        assert_eq!(d.sections.len(), 3);
        assert_eq!(d.sections[0].heading, None);
        assert_eq!(d.sections[1].heading.as_deref(), Some("Eins"));
        assert_eq!(d.sections[1].pieces[1], Piece::Example { title: "Fett".into(), source: "**fett**\n```js\nx\n```".into() });
        assert_eq!(d.sections[2].pieces.len(), 1);
    }

    #[test]
    fn fills_placeholders() {
        let keys = |name: &str| (name == "quickOpen").then(|| "⌘O".to_string());
        assert!(plain_key("return").is_some() && plain_key("cmd-shift-a").is_some() && plain_key("cmd--").is_some() && plain_key("h").is_some());
        assert_eq!(plain_key("quickOpen"), None);
        assert_eq!(plain_key("GoToday"), None);
        assert_eq!(expand("Mit {key:quickOpen} öffnen, {ui:Abbrechen}.", &keys), "Mit <kbd>⌘O</kbd> öffnen, **Abbrechen**.");
        assert_eq!(placeholders("{ui:A} {key:b} {nichts} $\\frac{1}{2}$"), vec![("ui".to_string(), "A".to_string()), ("key".to_string(), "b".to_string())]);
    }

    #[test]
    fn table_rows_read_as_cells_without_header() {
        assert_eq!(plain("| Taste | Wirkung |\n|---|---|\n| `e` | Alle **Felder** |\n| x | y |"), "e · Alle Felder · x · y");
    }

    #[test]
    fn snippet_marks_never_overlap() {
        let (shown, marks) = snippet("Hat eine andere App die Datei geändert, bis du wählst", &["datei", "d", "du"]);
        assert!(marks.windows(2).all(|w| w[0].end <= w[1].start), "{marks:?}");
        assert!(marks.iter().all(|r| shown.is_char_boundary(r.start) && shown.is_char_boundary(r.end)));
        let (_, marks) = snippet("Datei da", &["datei", "d"]);
        assert_eq!(marks, vec![0..5]);
    }

    #[test]
    fn finds_sections_ignoring_case_and_umlauts() {
        let docs = vec![split("# Übersicht\n\nWas heute ansteht.\n\n## Fällig\n\nAbgaben der Woche.\n"), split("# Karten\n\n## Lernen\n\nKarten wiederholen, fällig oder nicht.\n")];
        let hits = search(&docs, "fallig");
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].page, hits[0].heading.as_deref()), (0, Some("Fällig")));
        assert_eq!(search(&docs, "uebersicht")[0].page, 0);
        assert!(search(&docs, "karten nichtda").is_empty());
        let hit = &search(&docs, "wiederholen")[0];
        assert_eq!(hit.snippet.get(hit.marks[0].clone()), Some("wiederholen"));
    }

    #[test]
    fn languages_have_the_same_pages_and_structure() {
        for page in PAGES {
            let (de, en) = (doc(Lang::De, page.slug), doc(Lang::En, page.slug));
            assert!(!de.title.is_empty() && !en.title.is_empty(), "{}: missing # title", page.slug);
            assert_eq!(de.sections.len(), en.sections.len(), "{}: sections differ", page.slug);
            for (a, b) in de.sections.iter().zip(&en.sections) {
                let shape = |s: &Section| s.pieces.iter().map(|p| matches!(p, Piece::Example { .. })).collect::<Vec<_>>();
                assert_eq!(shape(a), shape(b), "{}: examples differ under {:?} / {:?}", page.slug, a.heading, b.heading);
                assert_eq!(a.heading.is_some(), b.heading.is_some(), "{}: headings differ", page.slug);
                let heads = |s: &Section| s.pieces.iter().filter_map(|p| if let Piece::Text(t) = p { Some(t.lines().filter(|l| l.starts_with('#')).map(|l| l.split(' ').next().unwrap().len()).collect::<Vec<_>>()) } else { None }).flatten().collect::<Vec<_>>();
                assert_eq!(heads(a), heads(b), "{}: heading levels differ under {:?}", page.slug, a.heading);
            }
            let mut a = placeholders(page.source(Lang::De));
            let mut b = placeholders(page.source(Lang::En));
            a.sort();
            b.sort();
            assert_eq!(a, b, "{}: placeholders differ", page.slug);
        }
        let files = |lang: &str| {
            let mut names: Vec<String> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/docs/").to_string() + lang).unwrap().flatten().map(|e| e.file_name().to_string_lossy().trim_end_matches(".md").to_string()).collect();
            names.sort();
            names
        };
        let mut listed: Vec<String> = PAGES.iter().map(|p| p.slug.to_string()).collect();
        listed.sort();
        assert_eq!(files("de"), listed, "assets/docs/de and PAGES differ");
        assert_eq!(files("en"), listed, "assets/docs/en and PAGES differ");
    }

    #[test]
    fn quoted_labels_are_live_translation_keys() {
        let catalog: std::collections::HashMap<String, serde_json::Value> = serde_json::from_str(include_str!("../../assets/i18n/en.json")).unwrap();
        let mut sources = String::new();
        for file in walkdir::WalkDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).into_iter().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "rs") && !e.path().ends_with("core/docs.rs")) {
            sources.push_str(&std::fs::read_to_string(file.path()).unwrap());
        }
        let mut stale = Vec::new();
        for page in PAGES {
            for (kind, name) in placeholders(page.source(Lang::De)) {
                if kind == "ui" && !(catalog.contains_key(&name) && sources.contains(&format!("\"{name}\""))) {
                    stale.push(format!("{}: {name}", page.slug));
                }
            }
        }
        assert!(stale.is_empty(), "labels no longer in the app: {stale:?}");
    }

    #[test]
    fn every_shortcut_is_on_the_shortcut_page() {
        let page = PAGES.iter().find(|p| p.slug == "tastenkuerzel").unwrap().source(Lang::De);
        let missing: Vec<&str> = super::super::shortcuts::ShortcutAction::ALL.iter().map(|a| a.id()).filter(|id| !page.contains(&format!("{{key:{id}}}"))).collect();
        assert!(missing.is_empty(), "shortcuts missing from tastenkuerzel.md: {missing:?}");
    }
}
