use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};

/// One card found in a note's text. Notes stay the only source of what a card says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCard {
    /// Follows the question, not the place: moving a note keeps the learning progress.
    pub id: String,
    pub kind: CardKind,
    pub front: String,
    pub back: String,
    /// The first and last line of the source, counted from 0.
    pub lines: (usize, usize),
    /// The headings above, outermost first.
    pub context: Vec<String>,
    /// Cards from one place in a note; they do not come up on the same day.
    pub group: String,
    /// Progress the Obsidian Spaced Repetition plugin wrote beside the card.
    pub sr: Option<SrProgress>,
    /// The id older versions gave the card while its scheduling comment still counted as text.
    pub legacy: Option<String>,
}

/// One schedule of an `<!--SR:!2026-11-20,12,250-->` comment: due day, interval in days, ease in percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SrProgress {
    pub due: chrono::NaiveDate,
    pub interval: u32,
    pub ease: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    Basic,
    /// The way back of a card learned in both directions.
    Reverse,
    Cloze,
}

fn hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn normal(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn card_id(kind: CardKind, question: &str) -> String {
    let tag = match kind {
        CardKind::Basic => "b",
        CardKind::Reverse => "r",
        CardKind::Cloze => "c",
    };
    hash(&format!("{tag}|{}", normal(question)))
}

/// Where inline code and formulas stand in a line; markers inside them mean nothing.
fn quoted_spans(line: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if matches!(bytes[i], b'`' | b'$') && (i == 0 || bytes[i - 1] != b'\\') {
            let mark = bytes[i];
            if let Some(end) = line[i + 1..].find(mark as char) {
                spans.push(i..i + 1 + end + 1);
                i += end + 2;
                continue;
            }
        }
        i += 1;
    }
    spans
}

/// The body of a line: list marks, tasks and quote marks are not part of the question.
fn strip_marks(line: &str) -> &str {
    static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(?:(?:[-*+]|\d+[.)])\s+)?(?:\[.\]\s+)?").unwrap());
    let at = MARK.find(line).map_or(0, |m| m.end());
    &line[at..]
}

/// Splits at a separator with spaces on both sides, so `std::vector` and Dataview's `key:: value` stay text.
fn split_at(line: &str, separator: &str) -> Option<(String, String)> {
    let needle = format!(" {separator} ");
    let spans = quoted_spans(line);
    let mut from = 0;
    while let Some(found) = line[from..].find(&needle) {
        let at = from + found;
        if !spans.iter().any(|s| s.contains(&at)) {
            let (front, back) = (line[..at].trim(), line[at + needle.len()..].trim());
            if !front.is_empty() && !back.is_empty() {
                return Some((front.to_string(), back.to_string()));
            }
            return None;
        }
        from = at + 1;
    }
    None
}

static MARKED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"==([^=\n]+?)==").unwrap());

struct Gap {
    range: std::ops::Range<usize>,
    number: Option<u32>,
    text: String,
    hint: Option<String>,
    in_formula: bool,
}

/// Finds `{{c1::text::hint}}` and `{{text}}`. Braces inside count, so `{{c1::\\frac{a}{b}}}` holds all of its formula.
fn scan_gaps(text: &str) -> Vec<(std::ops::Range<usize>, Option<u32>, String, Option<String>)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find("{{").map(|at| from + at) {
        let inner_start = open + 2;
        let mut depth = 0i32;
        let mut end = None;
        let bytes = text.as_bytes();
        let mut i = inner_start;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 1,
                b'{' => depth += 1,
                b'}' if depth > 0 => depth -= 1,
                b'}' if bytes.get(i + 1) == Some(&b'}') => {
                    end = Some(i);
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        let Some(end) = end else { break };
        let body = &text[inner_start..end];
        let (number, rest) = match body.strip_prefix('c').and_then(|r| r.split_once("::")) {
            Some((n, rest)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => (n.parse().ok(), rest),
            _ => (None, body),
        };
        let mut depth = 0i32;
        let mut split = None;
        let rb = rest.as_bytes();
        for at in 0..rb.len() {
            match rb[at] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                b':' if depth == 0 && rb.get(at + 1) == Some(&b':') => {
                    split = Some(at);
                    break;
                }
                _ => {}
            }
        }
        let (shown, hint) = match split {
            Some(at) => (&rest[..at], Some(rest[at + 2..].to_string())),
            None => (rest, None),
        };
        if !shown.trim().is_empty() {
            found.push((open..end + 2, number, shown.to_string(), hint));
        }
        from = end + 2;
    }
    found
}

fn code_spans(line: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(open) = line[from..].find('`').map(|at| from + at) {
        let Some(close) = line[open + 1..].find('`').map(|at| open + 1 + at) else { break };
        spans.push(open..close + 1);
        from = close + 1;
    }
    spans
}

fn formula_spans(line: &str) -> Vec<std::ops::Range<usize>> {
    quoted_spans(line).into_iter().filter(|s| line.as_bytes()[s.start] == b'$').collect()
}

fn gaps_in(text: &str, tagged: bool) -> Vec<Gap> {
    let code = code_spans(text);
    let formulas = formula_spans(text);
    let mut gaps: Vec<Gap> = scan_gaps(text)
        .into_iter()
        .filter(|(range, number, ..)| (number.is_some() || tagged) && !code.iter().any(|s| s.contains(&range.start)))
        .map(|(range, number, shown, hint)| {
            let in_formula = formulas.iter().any(|s| s.contains(&range.start));
            Gap { range, number, text: shown, hint, in_formula }
        })
        .collect();
    if tagged {
        let spans = quoted_spans(text);
        gaps.extend(MARKED.captures_iter(text).filter_map(|c| {
            let whole = c.get(0)?;
            (!spans.iter().any(|s| s.contains(&whole.start()))).then(|| Gap { range: whole.range(), number: None, text: c[1].to_string(), hint: None, in_formula: false })
        }));
        gaps.sort_by_key(|g| g.range.start);
    }
    gaps
}

fn cloze_cards(text: &str, lines: (usize, usize), context: &[String], tagged: bool, out: &mut Vec<RawCard>) {
    let gaps = gaps_in(text, tagged);
    if gaps.is_empty() {
        return;
    }
    let mut numbers: Vec<u32> = Vec::new();
    let mut next_free = gaps.iter().filter_map(|g| g.number).max().unwrap_or(0) + 1;
    let assigned: Vec<u32> = gaps
        .iter()
        .map(|g| {
            g.number.unwrap_or_else(|| {
                next_free += 1;
                next_free - 1
            })
        })
        .collect();
    for n in &assigned {
        if !numbers.contains(n) {
            numbers.push(*n);
        }
    }
    let group = hash(&normal(text));
    for number in numbers {
        let render = |hidden: bool| {
            let mut result = String::new();
            let mut at = 0;
            for (gap, n) in gaps.iter().zip(&assigned) {
                result.push_str(&text[at..gap.range.start]);
                if *n != number {
                    result.push_str(&gap.text);
                } else if gap.in_formula {
                    result.push_str(&if hidden { format!("\\boxed{{\\text{{{}}}}}", gap.hint.as_deref().unwrap_or("?")) } else { format!("\\boxed{{{}}}", gap.text) });
                } else if hidden {
                    result.push_str(&format!("**[{}]**", gap.hint.as_deref().unwrap_or("…")));
                } else {
                    result.push_str(&format!("=={}==", gap.text));
                }
                at = gap.range.end;
            }
            result.push_str(&text[at..]);
            result
        };
        let id = card_id(CardKind::Cloze, &format!("{number}|{text}"));
        out.push(RawCard { id, kind: CardKind::Cloze, front: render(true), back: render(false), lines, context: context.to_vec(), group: group.clone(), sr: None, legacy: None });
    }
}

fn add_basic(front: String, back: String, both: bool, lines: (usize, usize), context: &[String], out: &mut Vec<RawCard>) {
    let group = hash(&normal(&format!("{front}|{back}")));
    out.push(RawCard { id: card_id(CardKind::Basic, &front), kind: CardKind::Basic, front: front.clone(), back: back.clone(), lines, context: context.to_vec(), group: group.clone(), sr: None, legacy: None });
    if both {
        out.push(RawCard { id: card_id(CardKind::Reverse, &back), kind: CardKind::Reverse, front: back, back: front, lines, context: context.to_vec(), group, sr: None, legacy: None });
    }
}

fn frontmatter_end(lines: &[&str]) -> usize {
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return 0;
    }
    lines.iter().skip(1).position(|l| matches!(l.trim_end(), "---" | "...")).map_or(0, |at| at + 2)
}

fn is_plain_line(line: &str) -> bool {
    let body = strip_marks(line).trim();
    let table = body.starts_with('|') && body.ends_with('|') && !body.contains(" :: ");
    let table = table || (body.starts_with('|') && body.ends_with('|') && body.matches('|').count() > 2);
    !body.is_empty() && !table && !body.starts_with("```") && !body.starts_with("~~~") && body != "$$" && !body.starts_with("[!")
}

/// What stands before the text of a line that holds a card: list marks, quote marks, indent.
pub fn line_marks(line: &str) -> &str {
    &line[..line.len() - strip_marks(line).len()]
}

/// Finds every card of a note.
///
/// `Frage :: Antwort` is a card, `:::` learns both ways, a line with only `?` (or `??`) separates a longer question
/// from its answer, `{{c1::Lücke}}` hides a part. Notes tagged `#flashcards` or `#karten` also count `{{Lücke}}` and
/// `==Markiertes==`. Frontmatter `cards: false` leaves a note out. Code, formulas and comments never make a card.
/// Scheduling comments are progress, not text: a card's id and answer leave them out.
pub fn cards_in(text: &str) -> Vec<RawCard> {
    let mut cards = read_cards(text, true);
    if text.contains("<!--SR:") {
        let older = read_cards(text, false);
        if older.len() == cards.len() {
            for (card, old) in cards.iter_mut().zip(older) {
                if old.id != card.id && old.kind == card.kind && old.lines.0 == card.lines.0 {
                    card.legacy = Some(old.id);
                }
            }
        }
    }
    cards
}

static SR_COMMENT: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"[ \t]*<!--SR:((?:![0-9]{4}-[0-9]{2}-[0-9]{2},[0-9]+,[0-9]+)+)-->").ok());
static SR_ENTRY: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"!([0-9]{4}-[0-9]{2}-[0-9]{2}),([0-9]+),([0-9]+)").ok());

fn take_schedules(line: &str) -> (String, Vec<SrProgress>, bool) {
    let (Some(comments), Some(entries)) = (SR_COMMENT.as_ref(), SR_ENTRY.as_ref()) else { return (line.to_string(), Vec::new(), false) };
    let code = code_spans(line);
    let mut found = Vec::new();
    let mut kept = String::with_capacity(line.len());
    let mut at = 0;
    for comment in comments.captures_iter(line) {
        let Some(whole) = comment.get(0) else { continue };
        if code.iter().any(|s| s.contains(&whole.start()) || s.contains(&(whole.end() - 1))) {
            continue;
        }
        kept.push_str(line.get(at..whole.start()).unwrap_or_default());
        at = whole.end();
        for entry in entries.captures_iter(&comment[1]) {
            if let (Ok(due), Ok(interval), Ok(ease)) = (chrono::NaiveDate::parse_from_str(&entry[1], "%Y-%m-%d"), entry[2].parse(), entry[3].parse()) {
                found.push(SrProgress { due, interval, ease });
            }
        }
    }
    if at == 0 {
        return (line.to_string(), found, false);
    }
    kept.push_str(line.get(at..).unwrap_or_default());
    let alone = kept.trim().is_empty();
    (kept, found, alone)
}

/// The Obsidian Spaced Repetition plugin's comments standing in these lines, to keep them when a card is rewritten.
pub fn schedule_comments(lines: &[String]) -> String {
    let Some(comments) = SR_COMMENT.as_ref() else { return String::new() };
    lines.iter().flat_map(|line| comments.find_iter(line).map(|m| m.as_str().to_string())).collect()
}

/// The text as a person wrote it, without the plugin's comments, to edit it.
pub fn without_schedules(text: &str) -> String {
    text.split('\n').map(|line| if line.contains("<!--SR:") { take_schedules(line).0 } else { line.to_string() }).collect::<Vec<_>>().join("\n")
}

/// A line that holds nothing but the plugin's comments.
pub fn is_schedule_line(line: &str) -> bool {
    line.contains("<!--SR:") && take_schedules(line).2
}

fn attach_schedules(cards: &mut [RawCard], schedules: &[(Vec<SrProgress>, bool)], first: usize, last: usize) {
    let mut found: Vec<SrProgress> = schedules[first..=last].iter().flat_map(|(s, _)| s.iter().copied()).collect();
    if let Some((below, true)) = schedules.get(last + 1) {
        found.extend(below.iter().copied());
    }
    for (card, schedule) in cards.iter_mut().zip(found) {
        card.sr = Some(schedule);
    }
}

fn read_cards(text: &str, strip_schedules: bool) -> Vec<RawCard> {
    let prepared = crate::core::vault::syntax::prepare(text);
    let mut schedules: Vec<(Vec<SrProgress>, bool)> = Vec::new();
    let owned: Vec<std::borrow::Cow<str>> = prepared
        .split('\n')
        .map(|line| {
            if strip_schedules && line.contains("<!--SR:") {
                let (kept, found, alone) = take_schedules(line);
                schedules.push((found, alone));
                std::borrow::Cow::Owned(kept)
            } else {
                schedules.push((Vec::new(), false));
                std::borrow::Cow::Borrowed(line)
            }
        })
        .collect();
    let lines: Vec<&str> = owned.iter().map(|l| &**l).collect();
    let start = frontmatter_end(&lines);
    let head = lines.get(..start).unwrap_or_default().join("\n");
    if head.lines().any(|l| matches!(l.replace(' ', "").as_str(), "cards:false" | "karten:false")) {
        return Vec::new();
    }
    let tagged = crate::core::vault::syntax::tags_in(text).iter().chain(crate::core::vault::frontmatter::tags(text).iter()).any(|t| {
        let t = t.to_lowercase();
        t == "flashcards" || t == "karten" || t == "flashcard" || t == "karteikarten"
    });
    let mut out = Vec::new();
    let mut context: Vec<(usize, String)> = Vec::new();
    let mut fence: Option<&str> = None;
    let mut in_math = false;
    let mut in_comment = false;
    let mut in_list = false;
    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if in_comment || trimmed.starts_with("<!--") {
            in_comment = !trimmed.contains("-->");
            i += 1;
            continue;
        }
        if let Some(mark) = fence {
            if trimmed.starts_with(mark) {
                fence = None;
            }
            i += 1;
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(if trimmed.starts_with("```") { "```" } else { "~~~" });
            i += 1;
            continue;
        }
        if in_math {
            in_math = !trimmed.contains("$$");
            i += 1;
            continue;
        }
        if trimmed.starts_with("$$") && !trimmed[2..].contains("$$") {
            in_math = true;
            i += 1;
            continue;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            context.retain(|(level, _)| *level < hashes);
            context.push((hashes, trimmed[hashes..].trim().to_string()));
            i += 1;
            continue;
        }
        let indented_code = (line.starts_with("    ") || line.starts_with('\t')) && strip_marks(line).len() == trimmed.len() && lines.get(i.wrapping_sub(1)).is_none_or(|l| l.trim().is_empty()) && !in_list;
        in_list = !trimmed.is_empty() && (strip_marks(line).len() != trimmed.len() || (in_list && line.starts_with(' ')));
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        if !is_plain_line(line) || indented_code {
            i += 1;
            continue;
        }
        let crumbs: Vec<String> = context.iter().map(|(_, h)| h.clone()).collect();
        let next_mark = lines.get(i + 1).map(|l| strip_marks(l).trim());
        let from = out.len();
        if matches!(next_mark, Some("?") | Some("??")) {
            let both = next_mark == Some("??");
            let mut end = i + 2;
            while end < lines.len() && !lines[end].trim().is_empty() && !is_heading(lines[end]) {
                end += 1;
            }
            let back = lines[i + 2..end].iter().map(|l| strip_quote(l)).collect::<Vec<_>>().join("\n");
            let front = strip_marks(line).trim();
            if !gaps_in(front, tagged).is_empty() {
                cloze_cards(front, (i, end - 1), &crumbs, tagged, &mut out);
                for card in &mut out[from..] {
                    card.back = format!("{}\n\n{}", card.back, back.trim());
                }
            } else if !back.trim().is_empty() {
                add_basic(front.to_string(), back.trim().to_string(), both, (i, end - 1), &crumbs, &mut out);
            }
            attach_schedules(&mut out[from..], &schedules, i, end - 1);
            i = end;
            continue;
        }
        let body = strip_marks(line);
        if let Some((front, back)) = split_at(body, ":::") {
            add_basic(front, back, true, (i, i), &crumbs, &mut out);
        } else if let Some((front, back)) = split_at(body, "::") {
            add_basic(front, back, false, (i, i), &crumbs, &mut out);
        } else {
            cloze_cards(body, (i, i), &crumbs, tagged, &mut out);
        }
        attach_schedules(&mut out[from..], &schedules, i, i);
        i += 1;
    }
    out
}

fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && (trimmed[hashes..].starts_with(' ') || trimmed.len() == hashes)
}

fn strip_quote(line: &str) -> &str {
    let mut rest = line;
    while let Some(r) = rest.trim_start().strip_prefix('>') {
        rest = r.strip_prefix(' ').unwrap_or(r);
    }
    rest
}

/// The text a card is written as, for appending to a note. Long questions and answers take the form with `?`.
pub fn write_card(front: &str, back: &str, both: bool) -> String {
    let (front, back) = (front.trim(), back.trim());
    if back.is_empty() {
        return front.to_string();
    }
    let escaped;
    let front = if is_heading(front) {
        escaped = format!("\\{front}");
        escaped.as_str()
    } else {
        front
    };
    let needs_lines = |t: &str| t.contains(" :: ") || t.contains(" ::: ") || t.contains('\n');
    if needs_lines(front) || needs_lines(back) {
        let front = front.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
        let back = back.split('\n').filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n");
        format!("{front}\n{}\n{back}", if both { "??" } else { "?" })
    } else {
        format!("{front} {} {back}", if both { ":::" } else { "::" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_one_line_forms() {
        let cards = cards_in("# T\n## U\nWas ist x? :: Eine Zahl\n- Term ::: Erklärung\n");
        assert_eq!(cards.len(), 3);
        assert_eq!((cards[0].front.as_str(), cards[0].back.as_str()), ("Was ist x?", "Eine Zahl"));
        assert_eq!(cards[0].context, vec!["T", "U"]);
        assert_eq!(cards[0].kind, CardKind::Basic);
        assert_eq!(cards[1].front, "Term");
        assert_eq!((cards[2].kind, cards[2].front.as_str(), cards[2].back.as_str()), (CardKind::Reverse, "Erklärung", "Term"));
        assert_eq!(cards[1].group, cards[2].group);
    }

    #[test]
    fn what_only_looks_like_a_card_stays_text() {
        for text in [
            "std::vector ist ein Typ",
            "Autor:: Jemand",
            "`a :: b` im Code",
            "Formel $x :: y$ hier",
            "```\nq :: a\n```",
            "$$\nq :: a\n$$",
            "---\ntitle :: x\n---\n",
            "| a :: b | c |",
            "# Titel :: Untertitel",
            "%% q :: a %%",
            "<!-- q :: a -->",
            "{{date}} und ==wichtig==",
            " :: nur Antwort",
        ] {
            assert!(cards_in(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn reads_the_long_form() {
        let cards = cards_in("Wie groß?\n?\nSehr\n$$x$$\n\nDanach Text");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].back, "Sehr\n$$x$$");
        assert_eq!(cards[0].lines, (0, 3));
        assert_eq!(cards_in("Hin\n??\nHer").len(), 2);
    }

    #[test]
    fn hides_one_gap_per_card() {
        let cards = cards_in("Der {{c1::Satz}} folgt aus {{c2::Cauchy::Name}}.");
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].front, "Der **[…]** folgt aus Cauchy.");
        assert_eq!(cards[0].back, "Der ==Satz== folgt aus Cauchy.");
        assert_eq!(cards[1].front, "Der Satz folgt aus **[Name]**.");
        assert_eq!(cards[0].group, cards[1].group);
        assert_eq!(cards_in("{{c1::a}} und {{c1::b}}").len(), 1);
    }

    #[test]
    fn tagged_notes_count_marks_as_gaps() {
        assert_eq!(cards_in("#karten\nDas ist ==wichtig== und {{auch}}.").len(), 2);
        assert_eq!(cards_in("---\ntags: [flashcards]\n---\nDas ist ==wichtig==.").len(), 1);
    }

    #[test]
    fn a_gap_can_have_more_behind_it() {
        let cards = cards_in("Der {{c1::Satz}}\n?\nMehr dazu");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].back, "Der ==Satz==\n\nMehr dazu");
    }

    #[test]
    fn answers_with_separators_stay_whole() {
        let written = write_card("F", "a :: b", false);
        assert_eq!(cards_in(&written)[0].back, "a :: b");
    }

    #[test]
    fn formulas_can_stand_in_cards() {
        assert_eq!(cards_in("$$x^2$$ :: Quadrat").len(), 1);
        assert_eq!(cards_in("$$x^2$$\n?\nQuadrat").len(), 1);
        assert_eq!(cards_in("|x| :: Betrag").len(), 1);
        let gap = cards_in("Der Bruch {{c1::\\frac{a}{b}}} gilt");
        assert_eq!(gap[0].back, "Der Bruch ==\\frac{a}{b}== gilt");
        let inner = cards_in("Es gilt $x = {{c1::2}}$ hier");
        assert_eq!(inner[0].front, "Es gilt $x = \\boxed{\\text{?}}$ hier");
        assert_eq!(inner[0].back, "Es gilt $x = \\boxed{2}$ hier");
        assert_eq!(cards_in("`{{c1::code}}`").len(), 0);
    }

    #[test]
    fn long_answers_keep_tags_and_stop_at_headings() {
        assert_eq!(cards_in("F\n?\nA\n#tag dahinter\n# Neu")[0].back, "A\n#tag dahinter");
    }

    #[test]
    fn indented_code_is_no_card_but_nested_lists_are() {
        assert!(cards_in("Text\n\n    q :: a").is_empty());
        assert_eq!(cards_in("- oben\n    - q :: a").len(), 1);
        assert!(cards_in("> [!note] a :: b").is_empty());
    }

    #[test]
    fn a_question_like_a_heading_survives_writing() {
        assert_eq!(cards_in(&write_card("# Titel?", "A", false)).len(), 1);
    }

    #[test]
    fn a_note_can_opt_out() {
        assert!(cards_in("---\ncards: false\n---\nq :: a").is_empty());
    }

    #[test]
    fn ids_survive_small_changes_and_moves() {
        let a = &cards_in("Frage? :: Antwort")[0];
        let b = &cards_in("# Neu\n- frage?   :: andere Antwort")[0];
        assert_eq!(a.id, b.id);
        assert_ne!(a.id, cards_in("Andere Frage? :: Antwort")[0].id);
    }

    fn sr(due: &str, interval: u32, ease: u32) -> Option<SrProgress> {
        Some(SrProgress { due: chrono::NaiveDate::parse_from_str(due, "%Y-%m-%d").unwrap(), interval, ease })
    }

    #[test]
    fn spaced_repetition_comments_are_progress_not_text() {
        let inline = cards_in("Frage? :: Antwort <!--SR:!2026-11-20,12,250-->");
        assert_eq!(inline.len(), 1);
        assert_eq!(inline[0].back, "Antwort");
        assert_eq!(inline[0].sr, sr("2026-11-20", 12, 250));
        assert_eq!(inline[0].id, cards_in("Frage? :: Antwort")[0].id);
        let below = cards_in("Frage? :: Antwort\n<!--SR:!2026-11-20,12,250-->\n\nAndere? :: Text");
        assert_eq!(below.len(), 2);
        assert_eq!(below[0].sr, sr("2026-11-20", 12, 250));
        assert_eq!(below[1].sr, None);
        let long = cards_in("Lange Frage\n?\nAntwort\nzweite Zeile\n<!--SR:!2026-12-01,30,270-->");
        assert_eq!(long[0].back, "Antwort\nzweite Zeile");
        assert_eq!(long[0].sr, sr("2026-12-01", 30, 270));
        assert_eq!(long[0].lines, (0, 3));
    }

    #[test]
    fn both_directions_and_gaps_take_one_schedule_each() {
        let both = cards_in("Term ::: Erklärung <!--SR:!2026-11-10,3,230!2026-11-12,5,250-->");
        assert_eq!(both.len(), 2);
        assert_eq!((both[0].sr, both[1].sr), (sr("2026-11-10", 3, 230), sr("2026-11-12", 5, 250)));
        assert_eq!(both[1].front, "Erklärung");
        assert_eq!(both[1].id, cards_in("Term ::: Erklärung")[1].id);
        let gaps = cards_in("Der {{c1::Satz}} von {{c2::Euler}}\n<!--SR:!2026-11-10,3,230!2026-11-12,5,250-->");
        assert_eq!(gaps.len(), 2);
        assert_eq!((gaps[0].sr, gaps[1].sr), (sr("2026-11-10", 3, 230), sr("2026-11-12", 5, 250)));
        let inline = cards_in("Der {{c1::Satz}} von {{c2::Euler}} <!--SR:!2026-11-10,3,230-->");
        assert_eq!(inline[0].sr, sr("2026-11-10", 3, 230));
        assert_eq!(inline[1].sr, None);
        assert!(!inline[0].back.contains("SR"));
        assert_eq!(inline[0].id, cards_in("Der {{c1::Satz}} von {{c2::Euler}}")[0].id);
    }

    #[test]
    fn ids_stay_put_when_the_plugin_reschedules() {
        let a = cards_in("Term ::: Erklärung <!--SR:!2026-11-10,3,230!2026-11-12,5,250-->");
        let b = cards_in("Term ::: Erklärung <!--SR:!2027-01-10,60,250!2027-02-12,90,270-->");
        assert_eq!(a.iter().map(|c| &c.id).collect::<Vec<_>>(), b.iter().map(|c| &c.id).collect::<Vec<_>>());
        let long_a = cards_in("Hin\n??\nHer\n<!--SR:!2026-11-10,3,230!2026-11-12,5,250-->");
        let long_b = cards_in("Hin\n??\nHer\n<!--SR:!2026-12-10,9,230!2026-12-12,9,250-->");
        assert_eq!(long_a[1].id, long_b[1].id);
        assert_eq!(long_a[1].id, cards_in("Hin\n??\nHer")[1].id);
    }

    #[test]
    fn the_id_an_older_version_gave_is_kept_for_moving_progress() {
        let line = "Term ::: Erklärung <!--SR:!2026-11-10,3,230!2026-11-12,5,250-->";
        let cards = cards_in(line);
        assert_eq!(cards[0].legacy, None);
        assert_eq!(cards[1].legacy, Some(card_id(CardKind::Reverse, "Erklärung <!--SR:!2026-11-10,3,230!2026-11-12,5,250-->")));
        let gap = "Der {{c1::Satz}} gilt <!--SR:!2026-11-10,3,230-->";
        assert_eq!(cards_in(gap)[0].legacy, Some(card_id(CardKind::Cloze, &format!("1|{gap}"))));
        let long = cards_in("Hin\n??\nHer\n<!--SR:!2026-11-10,3,230-->");
        assert_eq!(long[1].legacy, Some(card_id(CardKind::Reverse, "Her\n<!--SR:!2026-11-10,3,230-->")));
        assert!(cards_in("Frage :: Antwort").iter().all(|c| c.legacy.is_none()));
    }

    #[test]
    fn broken_or_foreign_comments_stay_comments() {
        assert!(cards_in("<!--SR:!2026-11-10,3,230-->").is_empty());
        let card = &cards_in("F :: A <!-- Notiz -->")[0];
        assert_eq!(card.sr, None);
        assert_eq!(cards_in("F :: A <!--SR:!kaputt-->")[0].sr, None);
        assert_eq!(cards_in("`F :: A <!--SR:!2026-11-10,3,230-->`").len(), 0);
    }

    #[test]
    fn rewriting_a_card_can_keep_its_comments() {
        let lines = vec!["F :: A <!--SR:!2026-11-10,3,230-->".to_string(), "mehr".to_string()];
        assert_eq!(schedule_comments(&lines), " <!--SR:!2026-11-10,3,230-->");
        assert_eq!(schedule_comments(&["F :: A".to_string()]), "");
        assert!(is_schedule_line("  <!--SR:!2026-11-10,3,230!2026-11-11,4,250-->"));
        assert!(!is_schedule_line("F :: A <!--SR:!2026-11-10,3,230-->"));
        assert!(!is_schedule_line("<!-- Notiz -->"));
        assert_eq!(without_schedules("Der {{c1::Satz}} gilt <!--SR:!2026-11-10,3,230-->\nund `<!--SR:!2026-11-10,3,230-->`"), "Der {{c1::Satz}} gilt\nund `<!--SR:!2026-11-10,3,230-->`");
    }

    #[test]
    fn written_cards_read_back_the_same() {
        for (front, back, both, count) in [("F", "A", false, 1), ("F", "A", true, 2), ("F\nmehr", "A\nB", false, 1)] {
            let cards = cards_in(&write_card(front, back, both));
            assert_eq!(cards.len(), count);
            assert_eq!(cards[0].back, back);
        }
    }
}
