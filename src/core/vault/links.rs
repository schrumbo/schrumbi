use std::sync::LazyLock;

use regex::Regex;

/// Where a link points inside its target.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkTarget {
    /// The file part; empty for a link inside the same note.
    pub path: String,
    /// Heading path, `[[Note#A#B]]` gives `A`, `B`.
    pub headings: Vec<String>,
    /// Block identifier of `[[Note#^id]]`.
    pub block: Option<String>,
}

impl LinkTarget {
    /// Parses the destination of a link, without its display text: `Note#Heading`, `Note#^block`, `#Heading`.
    pub fn parse(destination: &str) -> Self {
        let mut parts = destination.split('#');
        let path = parts.next().unwrap_or_default().trim().to_string();
        let mut target = Self { path, ..Default::default() };
        for part in parts {
            let part = part.trim();
            if let Some(id) = part.strip_prefix('^') {
                target.block = Some(id.to_string());
            } else if !part.is_empty() {
                target.headings.push(part.to_string());
            }
        }
        target
    }

    /// The `#…` part as written after the file.
    pub fn subpath(&self) -> String {
        let mut out = String::new();
        for h in &self.headings {
            out.push('#');
            out.push_str(h);
        }
        if let Some(b) = &self.block {
            out.push_str("#^");
            out.push_str(b);
        }
        out
    }
}

/// A link found in text, with its byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundLink {
    pub range: std::ops::Range<usize>,
    pub target: LinkTarget,
    pub alias: Option<String>,
    pub embed: bool,
    /// Written as a Markdown link rather than a wikilink.
    pub markdown: bool,
}

static WIKI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[\[([^\[\]\n]+?)\]\]").unwrap());
static MARKDOWN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[([^\[\]\n]*)\]\(([^()\s]+|<[^<>\n]+>)\)").unwrap());

/// Byte ranges of code fences and inline code, where nothing is a link.
pub fn code_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut fence: Option<(char, usize, usize)> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        let run = marker.map_or(0, |m| trimmed.chars().take_while(|c| *c == m).count());
        match (&fence, marker) {
            (Some((m, n, start)), Some(c)) if c == *m && run >= *n && trimmed.trim_end().chars().all(|x| x == c) => {
                out.push(*start..offset + line.len());
                fence = None;
            }
            (None, Some(c)) if run >= 3 => fence = Some((c, run, offset)),
            (None, _) => {
                let mut rest = line;
                let mut at = offset;
                while let Some(i) = rest.find('`') {
                    let ticks = rest[i..].chars().take_while(|c| *c == '`').count();
                    let after = &rest[i + ticks..];
                    let close = "`".repeat(ticks);
                    let Some(j) = after.find(&close) else { break };
                    let end = i + ticks + j + ticks;
                    out.push(at + i..at + end);
                    at += end;
                    rest = &rest[end..];
                }
            }
            _ => {}
        }
        offset += line.len();
    }
    if let Some((_, _, start)) = fence {
        out.push(start..text.len());
    }
    out
}

fn inside(ranges: &[std::ops::Range<usize>], at: usize) -> bool {
    ranges.iter().any(|r| r.contains(&at))
}

fn percent_decode(s: &str) -> String {
    crate::core::markdown::percent_decode(s)
}

fn is_external(destination: &str) -> bool {
    destination.contains("://") || destination.starts_with("mailto:") || destination.starts_with("data:")
}

/// All internal links in a text: wikilinks, Markdown links and embeds of both. Code is skipped.
pub fn find_links(text: &str) -> Vec<FoundLink> {
    let code = code_ranges(text);
    let mut out = Vec::new();
    for c in WIKI.captures_iter(text) {
        let whole = c.get(0).unwrap();
        if inside(&code, whole.start()) {
            continue;
        }
        let inner = c[2].replace("\\|", "|");
        let (destination, alias) = match inner.split_once('|') {
            Some((d, a)) => (d.to_string(), Some(a.to_string())),
            None => (inner, None),
        };
        out.push(FoundLink { range: whole.range(), target: LinkTarget::parse(&destination), alias, embed: !c[1].is_empty(), markdown: false });
    }
    for c in MARKDOWN.captures_iter(text) {
        let whole = c.get(0).unwrap();
        let raw = c[3].trim_matches(|x| x == '<' || x == '>');
        if inside(&code, whole.start()) || is_external(raw) {
            continue;
        }
        out.push(FoundLink { range: whole.range(), target: LinkTarget::parse(&percent_decode(raw)), alias: Some(c[2].to_string()), embed: !c[1].is_empty(), markdown: true });
    }
    out.sort_by_key(|l| l.range.start);
    out
}

fn encode_markdown_path(path: &str) -> String {
    path.replace('%', "%25").replace(' ', "%20").replace('(', "%28").replace(')', "%29")
}

/// The text of a link pointing at `new_path`, keeping its kind, subpath and display text.
pub fn render_link(link: &FoundLink, new_path: &str) -> String {
    let bang = if link.embed { "!" } else { "" };
    let sub = link.target.subpath();
    if link.markdown {
        let alias = link.alias.as_deref().unwrap_or_default();
        return format!("{bang}[{alias}]({}{})", encode_markdown_path(new_path), encode_markdown_path(&sub));
    }
    match &link.alias {
        Some(alias) => format!("{bang}[[{new_path}{sub}|{alias}]]"),
        None => format!("{bang}[[{new_path}{sub}]]"),
    }
}

/// Applies replacements of link ranges, from the back so offsets stay valid.
pub fn apply_replacements(text: &str, mut changes: Vec<(std::ops::Range<usize>, String)>) -> String {
    changes.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = text.to_string();
    for (range, new) in changes {
        out.replace_range(range, &new);
    }
    out
}

/// Block identifiers defined in a note: `^id` at the end of a line, or alone on one.
pub fn block_ids(text: &str) -> Vec<(String, usize)> {
    static BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)\^([A-Za-z0-9-]+)\s*$").unwrap());
    let code = code_ranges(text);
    let mut offset = 0;
    let mut out = Vec::new();
    for (line_no, line) in text.split('\n').enumerate() {
        if !inside(&code, offset)
            && let Some(c) = BLOCK.captures(line)
        {
            out.push((c[1].to_string(), line_no));
        }
        offset += line.len() + 1;
    }
    out
}

/// Headings with their level and line.
pub fn headings(text: &str) -> Vec<(u8, String, usize)> {
    static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#{1,6})[ \t]+(.+?)[ \t#]*$").unwrap());
    let code = code_ranges(text);
    let mut offset = 0;
    let mut out = Vec::new();
    for (line_no, line) in text.split('\n').enumerate() {
        if !inside(&code, offset)
            && let Some(c) = HEADING.captures(line)
        {
            out.push((c[1].len() as u8, c[2].to_string(), line_no));
        }
        offset += line.len() + 1;
    }
    out
}

/// The part of a note a link points at: a heading with what follows it, or a block.
pub fn section(text: &str, target: &LinkTarget) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    if let Some(id) = &target.block {
        let Some((_, line)) = block_ids(text).into_iter().find(|(b, _)| b == id) else { return String::new() };
        let own_line = lines.get(line).is_some_and(|l| l.trim_start().starts_with('^'));
        let mut end = if own_line { line } else { line + 1 };
        while own_line && end > 0 && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
        let mut start = end.saturating_sub(1);
        while start > 0 && !lines[start - 1].trim().is_empty() {
            start -= 1;
        }
        let body = lines[start..end].join("\n");
        let pattern = format!(" ^{id}");
        return body.strip_suffix(&pattern).map_or(body.clone(), String::from);
    }
    let Some(wanted) = target.headings.last() else { return text.to_string() };
    let normalize = |s: &str| s.trim().to_lowercase();
    let all = headings(text);
    let Some(i) = all.iter().position(|(_, h, _)| normalize(h) == normalize(wanted)) else { return String::new() };
    let (level, _, start) = all[i];
    let end = all[i + 1..].iter().find(|(l, _, _)| *l <= level).map_or(lines.len(), |(_, _, line)| *line);
    lines[start..end].join("\n").trim_end().to_string()
}

/// How many times `term` occurs before `line`, so a search can land on that line.
pub fn occurrences_before(text: &str, term: &str, line: usize) -> usize {
    let offset: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    crate::core::search_index::occurrences(text, term).iter().filter(|r| r.start < offset).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_parse_like_obsidian() {
        assert_eq!(LinkTarget::parse("Note"), LinkTarget { path: "Note".into(), ..Default::default() });
        assert_eq!(LinkTarget::parse("A/Note.md#H1#H2").headings, ["H1", "H2"]);
        assert_eq!(LinkTarget::parse("Note#^abc-1").block.as_deref(), Some("abc-1"));
        let own = LinkTarget::parse("#Abschnitt");
        assert_eq!(own.path, "");
        assert_eq!(own.headings, ["Abschnitt"]);
        assert_eq!(LinkTarget::parse("Note#H#^x").subpath(), "#H#^x");
    }

    #[test]
    fn finds_every_kind_of_link_and_skips_code() {
        let text = "[[A]] ![[b.png|100]] [[C#H|Text]] [t](D%20E.md#x) ![i](f.png) [web](https://x.y)\n`[[nein]]`\n```\n[[auch nicht]]\n```\n[[Tabelle\\|Alias]]";
        let links = find_links(text);
        let paths: Vec<&str> = links.iter().map(|l| l.target.path.as_str()).collect();
        assert_eq!(paths, ["A", "b.png", "C", "D E.md", "f.png", "Tabelle"]);
        assert!(links[1].embed);
        assert_eq!(links[1].alias.as_deref(), Some("100"));
        assert_eq!(links[2].target.headings, ["H"]);
        assert!(links[3].markdown);
        assert_eq!(links[5].alias.as_deref(), Some("Alias"));
    }

    #[test]
    fn links_render_back_with_a_new_path() {
        let links = find_links("![[alt#H|B]] [T](alt.md#x) [[alt]]");
        assert_eq!(render_link(&links[0], "neu"), "![[neu#H|B]]");
        assert_eq!(render_link(&links[1], "Neu Name.md"), "[T](Neu%20Name.md#x)");
        assert_eq!(render_link(&links[2], "neu"), "[[neu]]");
    }

    #[test]
    fn replacements_keep_the_rest_byte_exact() {
        let text = "ä [[a]]\r\nü [[b]]";
        let links = find_links(text);
        let changed = apply_replacements(text, links.iter().map(|l| (l.range.clone(), render_link(l, "x"))).collect());
        assert_eq!(changed, "ä [[x]]\r\nü [[x]]");
    }

    #[test]
    fn sections_are_cut_out() {
        let text = "# A\neins\n## B\nzwei\n### C\ndrei\n## D\nvier\n\nabsatz ^x\n\nliste\nzwei\n\n^y\n\nende";
        let get = |d: &str| section(text, &LinkTarget::parse(d));
        assert_eq!(get("n#B"), "## B\nzwei\n### C\ndrei");
        assert_eq!(get("n#A#C"), "### C\ndrei");
        assert_eq!(get("n#^x"), "absatz");
        assert_eq!(get("n#^y"), "liste\nzwei");
        assert_eq!(get("n#fehlt"), "");
        assert_eq!(get("n"), text);
    }

    #[test]
    fn blocks_and_headings_are_found() {
        let text = "# Eins\nText ^abc\n\n^def\n\n```\n# kein\n^nein\n```\n## Zwei ##\nmit^keine";
        assert_eq!(block_ids(text), [("abc".to_string(), 1), ("def".to_string(), 3)]);
        assert_eq!(headings(text), [(1, "Eins".to_string(), 0), (2, "Zwei".to_string(), 9)]);
    }
}
