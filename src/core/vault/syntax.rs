use std::sync::LazyLock;

use regex::Regex;

use super::links::code_ranges;

/// Removes `%%comments%%` and rewrites `^[inline footnotes]` so a Markdown parser understands the note.
/// Line numbers stay as they were, ticking a task relies on them. An unclosed comment hides the rest of the note.
pub fn prepare(text: &str) -> String {
    let code = code_ranges(text);
    let is_code = |at: usize| code.iter().any(|r| r.contains(&at));
    let mut out = String::with_capacity(text.len());
    let mut footnotes: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut copied = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'%' && bytes[i + 1] == b'%' && !is_code(i) {
            out.push_str(&text[copied..i]);
            let end = text[i + 2..].find("%%").map(|e| i + 2 + e + 2);
            let hidden = &text[i..end.unwrap_or(text.len())];
            out.extend(std::iter::repeat_n('\n', hidden.matches('\n').count()));
            i = end.unwrap_or(text.len());
            copied = i;
            continue;
        }
        if bytes[i] == b'^' && bytes[i + 1] == b'[' && !is_code(i) && (i == 0 || bytes[i - 1] != b'[') {
            let mut depth = 0;
            let mut close = None;
            for (off, c) in text[i + 1..].char_indices() {
                match c {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(i + 1 + off);
                            break;
                        }
                    }
                    '\n' => break,
                    _ => {}
                }
            }
            if let Some(close) = close {
                out.push_str(&text[copied..i]);
                footnotes.push(text[i + 2..close].to_string());
                out.push_str(&format!("[^inline-{}]", footnotes.len()));
                i = close + 1;
                copied = i;
                continue;
            }
        }
        i += 1;
    }
    if copied < text.len() {
        out.push_str(&text[copied..]);
    }
    if !footnotes.is_empty() {
        out.push_str("\n\n");
        for (n, note) in footnotes.iter().enumerate() {
            out.push_str(&format!("[^inline-{}]: {note}\n", n + 1));
        }
    }
    out
}

/// A parsed callout head: `[!type]+ Title`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalloutHead {
    pub kind: CalloutKind,
    /// The type as written, for an unknown one.
    pub raw: String,
    pub title: Option<String>,
    pub foldable: bool,
    pub folded: bool,
}

/// The thirteen callout types of Obsidian; an unknown one counts as `Note`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutKind {
    Note,
    Abstract,
    Info,
    Todo,
    Tip,
    Success,
    Question,
    Warning,
    Failure,
    Danger,
    Bug,
    Example,
    Quote,
}

impl CalloutKind {
    pub fn parse(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "abstract" | "summary" | "tldr" => Self::Abstract,
            "info" => Self::Info,
            "todo" => Self::Todo,
            "tip" | "hint" | "important" => Self::Tip,
            "success" | "check" | "done" => Self::Success,
            "question" | "help" | "faq" => Self::Question,
            "warning" | "caution" | "attention" => Self::Warning,
            "failure" | "fail" | "missing" => Self::Failure,
            "danger" | "error" => Self::Danger,
            "bug" => Self::Bug,
            "example" => Self::Example,
            "quote" | "cite" => Self::Quote,
            _ => Self::Note,
        }
    }

    /// The default title, which the app translates.
    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "Notiz",
            Self::Abstract => "Zusammenfassung",
            Self::Info => "Info",
            Self::Todo => "Aufgabe",
            Self::Tip => "Tipp",
            Self::Success => "Erfolg",
            Self::Question => "Frage",
            Self::Warning => "Warnung",
            Self::Failure => "Fehlschlag",
            Self::Danger => "Gefahr",
            Self::Bug => "Fehler",
            Self::Example => "Beispiel",
            Self::Quote => "Zitat",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Note | Self::Info => "info",
            Self::Abstract => "clipboard-list",
            Self::Todo => "circle-check",
            Self::Tip => "flame",
            Self::Success => "check",
            Self::Question => "circle-help",
            Self::Warning => "triangle-alert",
            Self::Failure => "x",
            Self::Danger => "zap",
            Self::Bug => "bug",
            Self::Example => "list",
            Self::Quote => "quote",
        }
    }
}

/// Reads the first line of a quote; `None` when it is an ordinary quote.
pub fn callout_head(first_line: &str) -> Option<CalloutHead> {
    static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\[!([^\]\s]+)\]([+-]?)\s*(.*)$").unwrap());
    let c = HEAD.captures(first_line)?;
    let title = c[3].trim();
    Some(CalloutHead { kind: CalloutKind::parse(&c[1]), raw: c[1].to_string(), title: (!title.is_empty()).then(|| title.to_string()), foldable: !c[2].is_empty(), folded: &c[2] == "-" })
}

/// Colors of `==🔴text==`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkColor {
    Default,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
}

/// The color and text of a highlight's content.
pub fn mark_color(content: &str) -> (MarkColor, &str) {
    for (emoji, color) in [("🔴", MarkColor::Red), ("🟠", MarkColor::Orange), ("🟡", MarkColor::Yellow), ("🟢", MarkColor::Green), ("🔵", MarkColor::Blue), ("🟣", MarkColor::Purple)] {
        if let Some(rest) = content.strip_prefix(emoji) {
            return (color, rest);
        }
    }
    (MarkColor::Default, content)
}

/// Whether a word is a valid tag: letters, digits, `_`, `-`, `/`, emoji, and at least one thing that is not a digit.
pub fn is_tag(word: &str) -> bool {
    !word.is_empty() && !word.starts_with('/') && word.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/') || (!c.is_ascii() && !c.is_whitespace() && !c.is_ascii_punctuation() && !".,;:!?()[]{}\"'„“”‚‘’".contains(c))) && word.chars().any(|c| !c.is_ascii_digit() && c != '/')
}

/// Splits text into plain parts and `#tags`. A tag starts the text or follows white space.
pub fn split_tags(text: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let mut from = 0;
    let mut search = 0;
    while let Some(off) = text[search..].find('#') {
        let at = search + off;
        let before_ok = at == 0 || text[..at].chars().next_back().is_some_and(char::is_whitespace);
        let end = text[at + 1..].char_indices().find(|(_, c)| c.is_whitespace() || (c.is_ascii_punctuation() && !matches!(c, '_' | '-' | '/')) || ",;:!?()[]{}\"'„“”‚‘’".contains(*c)).map_or(text.len(), |(i, _)| at + 1 + i);
        if before_ok && is_tag(&text[at + 1..end]) {
            if at > from {
                out.push((false, &text[from..at]));
            }
            out.push((true, &text[at + 1..end]));
            from = end;
        }
        search = end.max(at + 1);
    }
    if from < text.len() {
        out.push((false, &text[from..]));
    }
    out
}

/// All tags in a note's text, without code.
pub fn tags_in(text: &str) -> Vec<String> {
    let code = code_ranges(text);
    let mut out = Vec::new();
    let body = super::frontmatter::body_offset(text);
    let mut offset = body;
    for line in text[body..].split_inclusive('\n') {
        if !code.iter().any(|r| r.contains(&offset)) {
            let is_heading = line.trim_start().starts_with("# ") || line.trim_start().starts_with("## ");
            for (tag, part) in split_tags(line) {
                if tag && !(is_heading && part.is_empty()) {
                    out.push(part.to_string());
                }
            }
        }
        offset += line.len();
    }
    out
}

/// Whether a task marker counts as done: any character but a space does.
pub fn task_marker(line: &str) -> Option<(usize, char)> {
    static TASK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(?:[-*+]|\d+[.)])\s+\[(.)\]").unwrap());
    let c = TASK.captures(line)?;
    let m = c.get(1)?;
    Some((m.start(), m.as_str().chars().next()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_vanish_and_lines_stay() {
        assert_eq!(prepare("a %%weg%% b"), "a  b");
        assert_eq!(prepare("a\n%%\nblock\n%%\nb"), "a\n\n\n\nb");
        assert_eq!(prepare("a\n%%offen\nb\nc"), "a\n\n\n");
        assert_eq!(prepare("`%%code%%` und\n```\n%%x%%\n```\n"), "`%%code%%` und\n```\n%%x%%\n```\n");
    }

    #[test]
    fn inline_footnotes_become_ordinary_ones() {
        let out = prepare("Text^[Eine [Notiz] hier] weiter\n- [ ] Aufgabe");
        assert!(out.starts_with("Text[^inline-1] weiter\n- [ ] Aufgabe"), "{out}");
        assert!(out.ends_with("[^inline-1]: Eine [Notiz] hier\n"));
        assert_eq!(prepare("x^2 und ![[b]] und [^id]"), "x^2 und ![[b]] und [^id]");
    }

    #[test]
    fn callouts_cover_types_aliases_folding_and_titles() {
        let h = callout_head("[!TIP]- Mein Titel").unwrap();
        assert_eq!(h.kind, CalloutKind::Tip);
        assert_eq!(h.title.as_deref(), Some("Mein Titel"));
        assert!(h.foldable && h.folded);
        let open = callout_head("[!faq]+").unwrap();
        assert_eq!(open.kind, CalloutKind::Question);
        assert!(open.foldable && !open.folded);
        assert_eq!(callout_head("[!eigenes] x").unwrap().kind, CalloutKind::Note);
        assert_eq!(callout_head("kein callout"), None);
        for (a, b) in [("summary", CalloutKind::Abstract), ("tldr", CalloutKind::Abstract), ("hint", CalloutKind::Tip), ("important", CalloutKind::Tip), ("check", CalloutKind::Success), ("done", CalloutKind::Success), ("help", CalloutKind::Question), ("caution", CalloutKind::Warning), ("attention", CalloutKind::Warning), ("fail", CalloutKind::Failure), ("missing", CalloutKind::Failure), ("error", CalloutKind::Danger), ("cite", CalloutKind::Quote)] {
            assert_eq!(CalloutKind::parse(a), b, "{a}");
        }
    }

    #[test]
    fn marks_can_have_colors() {
        assert_eq!(mark_color("🔴wichtig"), (MarkColor::Red, "wichtig"));
        assert_eq!(mark_color("normal"), (MarkColor::Default, "normal"));
    }

    #[test]
    fn tags_follow_obsidians_rules() {
        assert_eq!(split_tags("a #gut b#nein #1984 #y1984 #in/nen, #ümläut"), [(false, "a "), (true, "gut"), (false, " b#nein #1984 "), (true, "y1984"), (false, " "), (true, "in/nen"), (false, ", "), (true, "ümläut")]);
        assert_eq!(tags_in("---\ntags: [nein]\n---\n# Titel\n#ja `#code`\n```\n#fence\n```\n"), ["ja"]);
    }

    #[test]
    fn any_character_marks_a_task_done() {
        assert_eq!(task_marker("- [?] x"), Some((3, '?')));
        assert_eq!(task_marker("  1. [ ] x"), Some((6, ' ')));
        assert_eq!(task_marker("> - [x] x"), Some((5, 'x')));
        assert_eq!(task_marker("[ ] kein"), None);
    }
}
