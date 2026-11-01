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

static CLOZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{(?:c(\d+)::)?(.+?)(?:::(.+?))?\}\}").unwrap());
static MARKED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"==([^=\n]+?)==").unwrap());

struct Gap {
    range: std::ops::Range<usize>,
    number: Option<u32>,
    text: String,
    hint: Option<String>,
}

fn gaps_in(text: &str, tagged: bool) -> Vec<Gap> {
    let spans = quoted_spans(text);
    let mut gaps: Vec<Gap> = CLOZE
        .captures_iter(text)
        .filter_map(|c| {
            let whole = c.get(0)?;
            let number = c.get(1).and_then(|n| n.as_str().parse().ok());
            (number.is_some() || tagged).then(|| Gap { range: whole.range(), number, text: c[2].to_string(), hint: c.get(3).map(|h| h.as_str().to_string()) })
        })
        .filter(|g| !spans.iter().any(|s| s.contains(&g.range.start)))
        .collect();
    if tagged {
        gaps.extend(MARKED.captures_iter(text).filter_map(|c| {
            let whole = c.get(0)?;
            (!spans.iter().any(|s| s.contains(&whole.start()))).then(|| Gap { range: whole.range(), number: None, text: c[1].to_string(), hint: None })
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
        out.push(RawCard { id, kind: CardKind::Cloze, front: render(true), back: render(false), lines, context: context.to_vec(), group: group.clone() });
    }
}

fn add_basic(front: String, back: String, both: bool, lines: (usize, usize), context: &[String], out: &mut Vec<RawCard>) {
    let group = hash(&normal(&format!("{front}|{back}")));
    out.push(RawCard { id: card_id(CardKind::Basic, &front), kind: CardKind::Basic, front: front.clone(), back: back.clone(), lines, context: context.to_vec(), group: group.clone() });
    if both {
        out.push(RawCard { id: card_id(CardKind::Reverse, &back), kind: CardKind::Reverse, front: back, back: front, lines, context: context.to_vec(), group });
    }
}

fn frontmatter_end(lines: &[&str]) -> usize {
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return 0;
    }
    lines.iter().skip(1).position(|l| matches!(l.trim_end(), "---" | "...")).map_or(0, |at| at + 2)
}

fn is_plain_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with('|') && !trimmed.starts_with("```") && !trimmed.starts_with("~~~") && !trimmed.starts_with("$$") && !trimmed.starts_with("[!")
}

/// Finds every card of a note.
///
/// `Frage :: Antwort` is a card, `:::` learns both ways, a line with only `?` (or `??`) separates a longer question
/// from its answer, `{{c1::Lücke}}` hides a part. Notes tagged `#flashcards` or `#karten` also count `{{Lücke}}` and
/// `==Markiertes==`. Frontmatter `cards: false` leaves a note out. Code, formulas and comments never make a card.
pub fn cards_in(text: &str) -> Vec<RawCard> {
    let prepared = crate::core::vault::syntax::prepare(text);
    let lines: Vec<&str> = prepared.split('\n').collect();
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
        if !is_plain_line(line) {
            i += 1;
            continue;
        }
        let crumbs: Vec<String> = context.iter().map(|(_, h)| h.clone()).collect();
        let next_mark = lines.get(i + 1).map(|l| strip_marks(l).trim());
        if matches!(next_mark, Some("?") | Some("??")) {
            let both = next_mark == Some("??");
            let mut end = i + 2;
            while end < lines.len() && !lines[end].trim().is_empty() && !lines[end].trim_start().starts_with('#') {
                end += 1;
            }
            let back = lines[i + 2..end].iter().map(|l| strip_quote(l)).collect::<Vec<_>>().join("\n");
            let front = strip_marks(line).trim();
            if !gaps_in(front, tagged).is_empty() {
                let from = out.len();
                cloze_cards(front, (i, end - 1), &crumbs, tagged, &mut out);
                for card in &mut out[from..] {
                    card.back = format!("{}\n\n{}", card.back, back.trim());
                }
            } else if !back.trim().is_empty() {
                add_basic(front.to_string(), back.trim().to_string(), both, (i, end - 1), &crumbs, &mut out);
            }
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
        i += 1;
    }
    out
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

    #[test]
    fn written_cards_read_back_the_same() {
        for (front, back, both, count) in [("F", "A", false, 1), ("F", "A", true, 2), ("F\nmehr", "A\nB", false, 1)] {
            let cards = cards_in(&write_card(front, back, both));
            assert_eq!(cards.len(), count);
            assert_eq!(cards[0].back, back);
        }
    }
}
