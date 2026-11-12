use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{DateTime, Duration, Local, NaiveDate};
use regex::Regex;

use super::model::{Assignment, Module};
use super::naming::{Naming, extension, stem};
use super::text::fold;
use super::{day, markdown};

/// What the app knows about a file before filing it.
#[derive(Debug, Clone)]
pub struct FileFacts {
    pub path: PathBuf,
    pub created: DateTime<Local>,
    pub name_date: Option<NaiveDate>,
    pub pdf_title: Option<String>,
    pub pdf_author: Option<String>,
    pub page_count: usize,
    pub text: String,
    /// First heading or front-matter title of a Markdown note.
    pub note_title: Option<String>,
}

fn name_of(p: &Path) -> &str {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

fn prefix(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

impl FileFacts {
    pub fn read(path: &Path) -> Self {
        let meta = std::fs::metadata(path).ok();
        let created = meta.as_ref().and_then(|m| m.created().or_else(|_| m.modified()).ok()).map(DateTime::<Local>::from).unwrap_or_else(Local::now);
        let ext = extension(name_of(path)).to_lowercase();
        let (mut text, mut note_title, mut pdf_title, mut pdf_author, mut pages) = (String::new(), None, None, None, 0);
        if markdown::is_markdown(path) || ext == "txt" {
            if let Ok(s) = std::fs::read_to_string(path) {
                text = prefix(&s, 4000);
                if markdown::is_markdown(path) {
                    note_title = markdown::title(&s);
                }
            }
        }
        if ext == "pdf"
            && let Ok(doc) = lopdf::Document::load(path)
        {
            pages = doc.get_pages().len();
            let info = doc.trailer.get(b"Info").ok().and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_dict().ok().cloned());
            let field = |key: &[u8]| info.as_ref().and_then(|d| d.get(key).ok()).and_then(|o| lopdf::decode_text_string(o).ok()).map(|s| s.trim().to_string());
            pdf_title = field(b"Title").and_then(|t| clean_metadata_title(&t));
            pdf_author = field(b"Author").filter(|a| !a.is_empty());
            text = prefix(&doc.extract_text(&[1]).unwrap_or_default(), 4000);
        }
        Self { path: path.to_path_buf(), created, name_date: day::date_in_name(name_of(path)), pdf_title, pdf_author, page_count: pages, text, note_title }
    }

    pub fn name(&self) -> &str {
        name_of(&self.path)
    }
}

/// What a file is to the user: their own work, the task they were given, or course material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileRole {
    Work,
    Brief,
    Material,
}

/// Author slot used when a task description is filed under the naming convention, e.g. `P1_26_Infografik_Aufgabe_261028.pdf`.
pub const BRIEF_TAG: &str = "Aufgabe";

const BRIEF_WORDS: [&str; 20] = [
    "aufgabe", "aufgaben", "aufgabenstellung", "aufgabenblatt", "aufgabenzettel", "arbeitsauftrag", "arbeitszettel", "arbeitsblatt", "uebungsblatt", "uebungszettel",
    "hausuebung", "tagesuebung", "angabe", "brief", "briefing", "task", "assignment", "exercise", "worksheet", "handout",
];

/// Working files (notes, plain text) that show progress but are not what gets handed in.
pub const DRAFT_EXTENSIONS: [&str; 4] = ["md", "markdown", "txt", "rtf"];

impl FileRole {
    pub const ALL: [FileRole; 3] = [FileRole::Work, FileRole::Brief, FileRole::Material];

    /// Whether a file is the task description rather than a submission:
    /// it does not carry the author's name but is named like a task sheet.
    pub fn is_brief(file_name: &str, author: &str) -> bool {
        let folded = fold(file_name);
        let me = fold(author);
        if !me.is_empty() && folded.contains(&me) {
            return false;
        }
        stem(&folded).split(|c: char| !c.is_alphanumeric()).any(|token| {
            let word: String = token.chars().skip_while(char::is_ascii_digit).take_while(|c| !c.is_ascii_digit()).collect();
            BRIEF_WORDS.contains(&word.as_str())
        })
    }

    pub fn is_draft(path: &Path) -> bool {
        DRAFT_EXTENSIONS.contains(&extension(name_of(path)).to_lowercase().as_str())
    }

    /// Best guess before filing.
    pub fn guess(facts: &FileFacts, author: &str) -> FileRole {
        static HINTS: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"aufgabenstellung|arbeitsauftrag|arbeitszettel|uebungsblatt|hausuebung|tagesuebung|abgabe(termin)?\b|bis zum|aufgabe \d|deadline|submission|due date").unwrap()
        });
        let name = facts.name();
        let me = fold(author);
        if !me.is_empty() && fold(name).contains(&me) {
            return FileRole::Work;
        }
        if Self::is_brief(name, author) {
            return FileRole::Brief;
        }
        let ext = extension(name).to_lowercase();
        if ["ppt", "pptx", "key"].contains(&ext.as_str()) {
            return FileRole::Material;
        }
        if ext != "pdf" {
            return FileRole::Work;
        }
        let text = fold(&facts.text);
        let surname = me.split('-').next().unwrap_or("").to_string();
        if surname.chars().count() >= 3 && text.contains(&surname) {
            return FileRole::Work;
        }
        if facts.page_count <= 4 && HINTS.is_match(&text) {
            return FileRole::Brief;
        }
        if facts.page_count > 4 {
            return FileRole::Material;
        }
        if let Some(a) = &facts.pdf_author
            && (surname.is_empty() || !fold(a).contains(&surname))
        {
            return FileRole::Material;
        }
        FileRole::Work
    }
}

/// A proposed human title and where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Suggestion {
    pub title: String,
    pub source: &'static str,
}

/// Removes what Office adds to document titles, e.g. "Folien - Compatibility Mode".
fn strip_office_suffix(title: &str) -> String {
    static SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[\s\-_–\[(]*(compatibility[\s_-]*mode|kompatibilit(ä|ae)tsmodus)[\])]*\s*$").unwrap());
    SUFFIX.replace(title, "").into_owned()
}

/// The title a file keeps unless the user picks another: the title of a convention name, else the file's own name with every word kept.
pub fn default_title(file_name: &str, namings: &[Naming]) -> Option<Suggestion> {
    if let Some(p) = namings.iter().find_map(|n| n.parse(file_name)).filter(|p| !Naming::clean(&p.title).is_empty()) {
        return Some(Suggestion { title: Naming::clean(&p.title), source: "Bisheriger Titel" });
    }
    let own = Naming::clean(stem(file_name));
    (!own.is_empty()).then_some(Suggestion { title: own, source: "Dateiname" })
}

/// Ordered, de-duplicated candidates: the default title first, then worksheet title, PDF metadata and headline.
/// `ignore` holds words that never make a detected title on their own (course name, module codes, the author's name).
pub fn title_suggestions(facts: &FileFacts, namings: &[Naming], ignore: &[String]) -> Vec<Suggestion> {
    let name = facts.name();
    let mut out: Vec<Suggestion> = default_title(name, namings).into_iter().collect();
    let mut add = |raw: Option<String>, source: &'static str| {
        let Some(raw) = raw else { return };
        let t = Naming::clean(&strip_office_suffix(&raw));
        if t.chars().count() < 3 || is_ignored(&t, ignore) || out.iter().any(|s| s.title.to_lowercase() == t.to_lowercase()) {
            return;
        }
        out.push(Suggestion { title: t, source });
    };
    add(facts.note_title.clone(), "Überschrift in Notiz");
    for t in task_titles(&facts.text) {
        add(Some(t), "Aufgabentitel im PDF");
    }
    add(facts.pdf_title.clone(), "PDF-Titel");
    add(headline(&facts.text, ignore), "Überschrift im PDF");
    out.truncate(5);
    out
}

fn is_ignored(title: &str, ignore: &[String]) -> bool {
    let lower = title.to_lowercase();
    let words: Vec<&str> = lower.split('-').collect();
    let ig: Vec<String> = ignore.iter().map(|s| s.to_lowercase()).collect();
    !words.is_empty() && words.iter().all(|w| ig.iter().any(|i| i == w) || w.chars().all(|c| c.is_ascii_digit()))
}

/// Titles after labels like "Arbeitszettel 2: Beobachtungsprobe" or "Hausübung 1 : Infografiken vergleichen".
pub fn task_titles(text: &str) -> Vec<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(arbeitszettel|arbeitsblatt|übungsblatt|aufgabe|aufgabenblatt|übung|hausübung|tagesübung|hausarbeit|blatt|worksheet|exercise)\s*\d*\s*:\s*([^\n|·]{3,60})").unwrap()
    });
    RE.captures_iter(text)
        .take(3)
        .filter_map(|c| {
            let t = c[2].trim_matches(|ch: char| ch.is_whitespace() || ch.is_ascii_punctuation()).to_string();
            let words = t.split(' ').filter(|w| !w.is_empty()).count();
            ((1..=6).contains(&words) && !t.to_lowercase().starts_with("montag") && (!t.chars().any(|c| c.is_ascii_digit()) || words > 1)).then_some(t)
        })
        .collect()
}

pub fn clean_metadata_title(raw: &str) -> Option<String> {
    static OFFICE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^microsoft (word|powerpoint) - ").unwrap());
    static EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.(docx?|pptx?|pages|key)$").unwrap());
    let t = EXT.replace(&OFFICE.replace(raw.trim(), ""), "").into_owned();
    let n = t.chars().count();
    ((3..=70).contains(&n) && !t.to_lowercase().starts_with("untitled") && t.chars().any(char::is_alphabetic)).then_some(t)
}

/// First short line that looks like a heading and is not a header, name or date line.
pub fn headline(text: &str, ignore: &[String]) -> Option<String> {
    static SKIP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"seite \d|name:|abgabe|datum|\d{1,2}\.\d{1,2}\.").unwrap());
    let ig: Vec<String> = ignore.iter().map(|s| s.to_lowercase()).collect();
    for line in text.lines().filter(|l| !l.is_empty()).take(12) {
        let l = line.trim();
        let lower = l.to_lowercase();
        let words: Vec<&str> = l.split(' ').filter(|w| !w.is_empty()).collect();
        let len = l.chars().count();
        if !(2..=7).contains(&words.len()) || !(5..=60).contains(&len) {
            continue;
        }
        if (l.chars().filter(|c| c.is_alphabetic()).count() as f64) / (len as f64) <= 0.7 {
            continue;
        }
        if ig.iter().any(|i| !i.is_empty() && lower.contains(i.as_str())) {
            continue;
        }
        let mut unique: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        unique.sort();
        unique.dedup();
        if unique.len() < words.len() || SKIP.is_match(&lower) {
            continue;
        }
        return Some(l.to_string());
    }
    None
}

/// Ranks assignments as filing targets for a file.
/// `bases` and `active_modules` boost targets of recent classes (see `Library::class_boosts`).
pub fn rank_targets<'a>(
    facts: &FileFacts,
    assignments: &'a [Assignment],
    modules: &[Module],
    is_done: impl Fn(&Assignment) -> bool,
    now: DateTime<Local>,
    bases: &HashMap<PathBuf, f64>,
    active_modules: &HashMap<String, f64>,
) -> Vec<&'a Assignment> {
    let name = fold(facts.name());
    let name_tokens: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).collect();
    let text = fold(&facts.text);
    let file_day = facts.name_date.unwrap_or_else(|| day::local_date(facts.created));
    let text_dates = dates_in_text(&facts.text);
    let lecture_week = lecture_week_in_text(&facts.text);
    let mut scored: Vec<(&Assignment, f64)> = assignments
        .iter()
        .filter(|a| a.tracks_files() && a.due > now - Duration::days(21) && a.due < now + Duration::days(28))
        .map(|a| {
            let mut score = 0.;
            let due_day = day::local_date(a.due);
            if let Some(code) = modules.iter().find(|m| m.id == a.module_id).map(|m| fold(&m.code))
                && (name_tokens.contains(&code.as_str()) || name.contains(&format!("{code}_")))
            {
                score += 40.;
            }
            for k in a.keywords.iter().map(|k| fold(k)).filter(|k| k.chars().count() >= 2) {
                if name.contains(&k) {
                    score += 35.;
                } else if text.contains(&k) {
                    score += 12.;
                }
            }
            for token in fold(&a.short).split(' ').filter(|t| t.chars().count() >= 2) {
                if name_tokens.contains(&token) {
                    score += 30.;
                }
            }
            for word in fold(&a.title).split(' ').filter(|w| w.chars().count() >= 5) {
                if name.contains(word) {
                    score += 25.;
                } else if text.contains(word) {
                    score += 10.;
                }
            }
            if file_day == due_day {
                score += 25.;
            }
            if text_dates.contains(&due_day) {
                score += 30.;
            }
            if lecture_week.is_some() && a.week == lecture_week {
                score += 20.;
            }
            if file_day <= due_day && file_day > due_day - chrono::Days::new(7) {
                score += 15.;
            }
            if is_done(a) {
                score -= 30.;
            }
            if a.due >= now {
                let days = (a.due - now).num_seconds() as f64 / 86400.;
                score += (20. - days * 3.).max(0.);
            } else {
                score -= 10.;
            }
            if a.optional {
                score -= 5.;
            }
            score += a.folder.parent().and_then(|p| bases.get(p)).copied().unwrap_or(0.);
            score += active_modules.get(&a.module_id).copied().unwrap_or(0.);
            (a, score)
        })
        .collect();
    scored.sort_by(|x, y| if x.1 == y.1 { x.0.due.cmp(&y.0.due) } else { y.1.total_cmp(&x.1) });
    scored.into_iter().map(|(a, _)| a).collect()
}

/// The word of a file name that points at `assignment`: its module code, a keyword, its short name or a long word of its title.
pub fn name_match(file_name: &str, assignment: &Assignment, module: Option<&Module>) -> Option<String> {
    let name = fold(file_name);
    let tokens: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).collect();
    if let Some(code) = module.map(|m| m.code.clone()).filter(|c| tokens.contains(&fold(c).as_str()) || name.contains(&format!("{}_", fold(c)))) {
        return Some(code);
    }
    let keyword = assignment.keywords.iter().find(|k| k.chars().count() >= 2 && name.contains(&fold(k)));
    let short = assignment.short.split(' ').find(|t| t.chars().count() >= 2 && tokens.contains(&fold(t).as_str())).map(str::to_string);
    let title = assignment.title.split(' ').find(|w| w.chars().count() >= 5 && name.contains(&fold(w)));
    keyword.cloned().or(short).or_else(|| title.map(str::to_string))
}

/// Dates like "14.10.2026" in a document.
pub fn dates_in_text(text: &str) -> Vec<NaiveDate> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\D)(\d{1,2})\.(\d{1,2})\.(20\d{2})(?:\D|$)").unwrap());
    RE.captures_iter(text).take(6).filter_map(|c| NaiveDate::from_ymd_opt(c[3].parse().ok()?, c[2].parse().ok()?, c[1].parse().ok()?)).collect()
}

pub fn lecture_week_in_text(text: &str) -> Option<u32> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(lehrwoche|vorlesungswoche|woche)\s*(\d{1,2})").unwrap());
    RE.captures(text).and_then(|c| c[2].parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_office_suffixes_from_titles() {
        assert_eq!(strip_office_suffix("Ikigai und Team - Compatibility Mode"), "Ikigai und Team");
        assert_eq!(strip_office_suffix("Ikigai [Kompatibilitätsmodus]"), "Ikigai");
        assert_eq!(strip_office_suffix("Folien (compatibility-mode)"), "Folien");
        assert_eq!(strip_office_suffix("Mode und Kompatibilität"), "Mode und Kompatibilität");
    }

    #[test]
    fn names_the_word_that_points_at_a_deadline() {
        let a = Assignment {
            id: "a".into(),
            module_id: "m".into(),
            semester_id: "s".into(),
            rule_id: None,
            title: "Übungsblatt Recht".into(),
            short: "UE".into(),
            kind: super::super::model::Kind::Submission,
            due: Local::now(),
            week: None,
            iso_week: None,
            optional: false,
            upload: false,
            folder: PathBuf::new(),
            per_week_folder: false,
            file_pattern: String::new(),
            window_start: None,
            recursive: false,
            keywords: vec!["Skript".into()],
            is_moved: false,
            legacy: None,
        };
        assert_eq!(name_match("Skript_Recht_Kap1.pdf", &a, None).as_deref(), Some("Skript"));
        assert_eq!(name_match("Blatt05.pdf", &a, None), None);
    }

    #[test]
    fn recognizes_briefs() {
        assert!(FileRole::is_brief("Hausübung_3.pdf", "Muster-M"));
        assert!(FileRole::is_brief("Aufgabenstellung Woche2.pdf", "Muster-M"));
        assert!(!FileRole::is_brief("P1_26_Radar_Muster-M_261028.pdf", "Muster-M"));
        assert!(!FileRole::is_brief("Poster.pdf", "Muster-M"));
    }

    #[test]
    fn extracts_task_titles() {
        assert_eq!(task_titles("Arbeitszettel 2: Beobachtungsprobe\nfoo"), ["Beobachtungsprobe"]);
    }

    fn facts(name: &str) -> FileFacts {
        FileFacts { path: PathBuf::from(name), created: Local::now(), name_date: None, pdf_title: None, pdf_author: None, page_count: 1, text: String::new(), note_title: None }
    }

    fn naming() -> Naming {
        let module = Module { code: "P1".into(), naming: crate::core::model::DEFAULT_NAMING.into(), ..Default::default() };
        Naming::with_author(&module, None, "Muster-M", "", crate::core::model::CoauthorStyle::Initial)
    }

    #[test]
    fn default_title_keeps_every_word_of_the_file_name() {
        assert_eq!(default_title("Scan 2026-10-12 um 14.33.pdf", &[]).unwrap().title, "Scan-2026-10-12-um-14-33");
        assert_eq!(default_title("IMG_Poster_Entwurf final.png", &[]).unwrap().title, "IMG-Poster-Entwurf-final");
        assert_eq!(default_title("Übung (2) für Müller.pdf", &[]).unwrap().title, "Uebung-2-fuer-Mueller");
        assert_eq!(default_title("A1.pdf", &[]).unwrap().title, "A1");
        assert_eq!(default_title("Kopie.pdf", &[]).unwrap().source, "Dateiname");
        assert!(default_title("___.pdf", &[]).is_none());
    }

    #[test]
    fn default_title_reads_back_a_convention_name() {
        let d = default_title("P1_26_Radar-Chart_Muster-M_261028.pdf", &[naming()]).unwrap();
        assert_eq!((d.title.as_str(), d.source), ("Radar-Chart", "Bisheriger Titel"));
    }

    #[test]
    fn detected_titles_follow_the_file_name() {
        let mut f = facts("Scan_0042.pdf");
        f.pdf_title = Some("Infografik Analyse".into());
        f.text = "Arbeitszettel 2: Beobachtungsprobe\n".into();
        let s = title_suggestions(&f, &[], &[]);
        assert_eq!(s.iter().map(|s| (s.title.as_str(), s.source)).collect::<Vec<_>>(), [("Scan-0042", "Dateiname"), ("Beobachtungsprobe", "Aufgabentitel im PDF"), ("Infografik-Analyse", "PDF-Titel"), ("Arbeitszettel-2-Beobachtungsprobe", "Überschrift im PDF")]);
    }

    #[test]
    fn file_name_is_not_dropped_for_ignored_words() {
        let s = title_suggestions(&facts("P1.pdf"), &[], &["P1".into()]);
        assert_eq!(s.first().map(|s| s.title.as_str()), Some("P1"));
    }

    #[test]
    fn note_heading_is_an_alternative() {
        let mut f = facts("notiz.md");
        f.note_title = Some("Radar Chart".into());
        let s = title_suggestions(&f, &[], &[]);
        assert_eq!(s.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Notiz", "Radar-Chart"]);
    }
}
