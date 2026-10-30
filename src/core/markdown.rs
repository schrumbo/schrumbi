use std::path::Path;
use std::sync::LazyLock;

use base64::Engine;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use regex::Regex;

use super::naming::extension;

pub const EXTENSIONS: [&str; 3] = ["md", "markdown", "mdown"];

pub fn is_markdown(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| EXTENSIONS.contains(&extension(n).to_lowercase().as_str()))
}

pub fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_FOOTNOTES | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
}

/// Flips the task box on `line` (0-based), `- [ ]` ↔ `- [x]`.
pub fn toggle_task(text: &str, line: usize) -> String {
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    let Some(l) = lines.get_mut(line) else { return text.to_string() };
    if let Some(i) = l.find("[ ]") {
        l.replace_range(i..i + 3, "[x]");
    } else if let Some(i) = l.find("[x]").or_else(|| l.find("[X]")) {
        l.replace_range(i..i + 3, "[ ]");
    }
    lines.join("\n")
}

/// Front matter `title:` or the first heading.
pub fn title(text: &str) -> Option<String> {
    static FRONT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^title:\s*(.+)$").unwrap());
    let head: String = text.chars().take(4000).collect();
    if head.starts_with("---")
        && let Some(c) = FRONT.captures(&head)
    {
        let t = c[1].trim_matches(|c: char| c == ' ' || c == '"' || c == '\'').to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    let mut in_heading = false;
    let mut out = String::new();
    for e in Parser::new_ext(&head, options()) {
        match e {
            Event::Start(Tag::Heading { .. }) => in_heading = true,
            Event::End(TagEnd::Heading(_)) => return Some(out.trim().to_string()),
            Event::Text(t) | Event::Code(t) if in_heading => out.push_str(&t),
            _ => {}
        }
    }
    None
}

/// Open and done task boxes.
pub fn task_counts(text: &str) -> (usize, usize) {
    static OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*([-*+]|\d+[.)])\s+\[ \]").unwrap());
    static DONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*([-*+]|\d+[.)])\s+\[(x|X)\]").unwrap());
    text.split('\n').fold((0, 0), |(o, d), l| (o + OPEN.is_match(l) as usize, d + DONE.is_match(l) as usize))
}

/// Text split into plain and highlighted (`==x==`) runs.
#[derive(Debug, Clone, PartialEq)]
pub enum Run {
    Text(String),
    Mark(String),
}

pub fn runs(s: &str) -> Vec<Run> {
    static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"==[^=\n]+==").unwrap());
    let mut out = Vec::new();
    let mut last = 0;
    for m in MARK.find_iter(s) {
        if m.start() > last {
            out.push(Run::Text(s[last..m.start()].to_string()));
        }
        out.push(Run::Mark(s[m.start() + 2..m.end() - 2].to_string()));
        last = m.end();
    }
    if last < s.len() {
        out.push(Run::Text(s[last..].to_string()));
    }
    out
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Local images become data URIs so the renderer needs no file access.
pub fn image_source(source: &str, base: Option<&Path>) -> String {
    if source.contains("://") || source.starts_with("data:") {
        return source.to_string();
    }
    let path = percent_decode(source);
    let p = if Path::new(&path).is_absolute() { Some(std::path::PathBuf::from(&path)) } else { base.map(|b| b.join(&path)) };
    let Some(data) = p.as_ref().and_then(|p| std::fs::read(p).ok()) else { return source.to_string() };
    let ext = extension(&path).to_lowercase();
    let mime = match ext.as_str() {
        "svg" => "image/svg+xml".to_string(),
        "jpg" => "image/jpeg".to_string(),
        e => format!("image/{e}"),
    };
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(data))
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Header line shown above an exported note.
pub struct Meta {
    pub left: String,
    pub right: String,
}

/// Standalone HTML document for PDF export.
pub fn html_document(text: &str, base: Option<&Path>, css: &str, meta: Option<&Meta>) -> String {
    let mut body = String::new();
    if let Some(m) = meta {
        body += &format!("<div class=\"meta\"><span>{}</span><span>{}</span></div>\n", escape(&m.left), escape(&m.right));
    }
    body += &to_html(text, base);
    format!("<!doctype html>\n<html lang=\"de\"><head><meta charset=\"utf-8\"><style>{css}</style></head>\n<body>{body}</body></html>")
}

/// Note body as HTML: CommonMark + GFM tables, tasks, strikethrough, footnotes and `==highlight==`.
pub fn to_html(text: &str, base: Option<&Path>) -> String {
    let mut out = String::new();
    let mut aligns: Vec<Alignment> = Vec::new();
    let mut cell = 0;
    let mut in_head = false;
    let mut code: Option<(Option<String>, String)> = None;
    let mut image: Option<(String, String)> = None;
    let mut footnotes: Vec<String> = Vec::new();
    let mut footnote_body: Option<(String, String)> = None;
    let mut ordered_footnotes: Vec<(String, String)> = Vec::new();
    for event in Parser::new_ext(text, options()) {
        match &event {
            Event::Start(Tag::FootnoteDefinition(id)) => {
                footnote_body = Some((id.to_string(), String::new()));
                continue;
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                ordered_footnotes.extend(footnote_body.take());
                continue;
            }
            _ => {}
        }
        let o = match footnote_body.as_mut() {
            Some((_, b)) => b,
            None => &mut out,
        };
        if let Some((_, buf)) = code.as_mut() {
            match event {
                Event::Text(t) => buf.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, buf) = code.take().unwrap();
                    let src = buf.strip_suffix('\n').unwrap_or(&buf);
                    let class = lang.as_ref().map(|l| format!(" class=\"language-{}\"", escape(l))).unwrap_or_default();
                    o.push_str(&format!("<pre><code{class}>{}</code></pre>\n", highlighted_html(src, lang.as_deref())));
                }
                _ => {}
            }
            continue;
        }
        if let Some((_, alt)) = image.as_mut() {
            match event {
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    let (src, alt) = image.take().unwrap();
                    o.push_str(&format!("<img src=\"{}\" alt=\"{}\">", escape(&image_source(&src, base)), escape(&alt)));
                }
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => o.push_str("<p>"),
                Tag::Heading { level, .. } => o.push_str(&format!("<h{}>", heading_number(level))),
                Tag::BlockQuote(_) => o.push_str("<blockquote>"),
                Tag::CodeBlock(kind) => {
                    let lang = match kind {
                        CodeBlockKind::Fenced(l) if !l.is_empty() => Some(l.split_whitespace().next().unwrap_or("").to_string()),
                        _ => None,
                    };
                    code = Some((lang, String::new()));
                }
                Tag::List(Some(1)) => o.push_str("<ol>"),
                Tag::List(Some(n)) => o.push_str(&format!("<ol start=\"{n}\">")),
                Tag::List(None) => o.push_str("<ul>"),
                Tag::Item => o.push_str("<li>"),
                Tag::Table(a) => {
                    aligns = a;
                    o.push_str("<table>");
                }
                Tag::TableHead => {
                    in_head = true;
                    cell = 0;
                    o.push_str("<thead><tr>");
                }
                Tag::TableRow => {
                    cell = 0;
                    o.push_str("<tr>");
                }
                Tag::TableCell => {
                    let style = match aligns.get(cell) {
                        Some(Alignment::Left) => " style=\"text-align:left\"",
                        Some(Alignment::Center) => " style=\"text-align:center\"",
                        Some(Alignment::Right) => " style=\"text-align:right\"",
                        _ => "",
                    };
                    o.push_str(&format!("<{}{style}>", if in_head { "th" } else { "td" }));
                }
                Tag::Emphasis => o.push_str("<em>"),
                Tag::Strong => o.push_str("<strong>"),
                Tag::Strikethrough => o.push_str("<del>"),
                Tag::Link { dest_url, .. } => o.push_str(&format!("<a href=\"{}\">", escape(&dest_url))),
                Tag::Image { dest_url, .. } => image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => o.push_str("</p>\n"),
                TagEnd::Heading(level) => o.push_str(&format!("</h{}>\n", heading_number(level))),
                TagEnd::BlockQuote(_) => o.push_str("</blockquote>\n"),
                TagEnd::List(true) => o.push_str("</ol>\n"),
                TagEnd::List(false) => o.push_str("</ul>\n"),
                TagEnd::Item => o.push_str("</li>\n"),
                TagEnd::Table => o.push_str("</tbody></table>\n"),
                TagEnd::TableHead => {
                    in_head = false;
                    o.push_str("</tr></thead><tbody>");
                }
                TagEnd::TableRow => o.push_str("</tr>"),
                TagEnd::TableCell => {
                    o.push_str(if in_head { "</th>" } else { "</td>" });
                    cell += 1;
                }
                TagEnd::Emphasis => o.push_str("</em>"),
                TagEnd::Strong => o.push_str("</strong>"),
                TagEnd::Strikethrough => o.push_str("</del>"),
                TagEnd::Link => o.push_str("</a>"),
                _ => {}
            },
            Event::Text(t) => {
                for r in runs(&t) {
                    match r {
                        Run::Text(s) => o.push_str(&escape(&s)),
                        Run::Mark(s) => o.push_str(&format!("<mark>{}</mark>", escape(&s))),
                    }
                }
            }
            Event::Code(c) => o.push_str(&format!("<code>{}</code>", escape(&c))),
            Event::Html(h) | Event::InlineHtml(h) => o.push_str(&h),
            Event::SoftBreak | Event::HardBreak => o.push_str("<br>"),
            Event::Rule => o.push_str("<hr>\n"),
            Event::TaskListMarker(done) => {
                strip_last(o, "<li>");
                o.push_str(&format!("<li class=\"task\"><span class=\"box{}\"></span>", if done { " done" } else { "" }));
            }
            Event::FootnoteReference(id) => {
                let n = footnotes.iter().position(|f| f == id.as_ref()).unwrap_or_else(|| {
                    footnotes.push(id.to_string());
                    footnotes.len() - 1
                }) + 1;
                o.push_str(&format!("<sup class=\"fnref\"><a href=\"#fn-{n}\">{n}</a></sup>"));
            }
            _ => {}
        }
    }
    let used: Vec<_> = footnotes.iter().filter_map(|id| ordered_footnotes.iter().find(|(i, _)| i == id)).collect();
    if !used.is_empty() {
        out += "\n<section class=\"footnotes\"><ol>";
        for (n, (_, body)) in used.iter().enumerate() {
            let inner = body.trim().strip_prefix("<p>").and_then(|b| b.strip_suffix("</p>")).unwrap_or(body.trim());
            out += &format!("<li id=\"fn-{}\">{inner}</li>", n + 1);
        }
        out += "</ol></section>";
    }
    out.replace("<li><p>", "<li>").replace("</p>\n</li>", "</li>")
}

fn strip_last(s: &mut String, suffix: &str) {
    if s.ends_with(suffix) {
        s.truncate(s.len() - suffix.len());
    }
}

pub fn heading_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Syntax token kinds for fenced code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Attribute,
}

impl TokenKind {
    pub fn css(self) -> &'static str {
        match self {
            TokenKind::Plain => "plain",
            TokenKind::Keyword => "keyword",
            TokenKind::String => "string",
            TokenKind::Comment => "comment",
            TokenKind::Number => "number",
            TokenKind::Type => "type",
            TokenKind::Attribute => "attribute",
        }
    }
}

struct Language {
    keywords: &'static [&'static str],
    line_comment: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    types_capitalized: bool,
}

const C_LIKE: &[&str] = &[
    "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "return", "true", "false", "null", "nil", "new", "class", "struct", "enum", "interface", "extends", "implements",
    "import", "package", "public", "private", "protected", "static", "final", "const", "let", "var", "func", "function", "void", "int", "double", "float", "bool", "boolean", "char", "string", "try", "catch",
    "throw", "throws", "async", "await", "in", "of", "this", "self", "super", "guard", "where", "protocol", "extension", "init", "override", "typeof", "instanceof", "export", "from", "as", "is", "fn", "pub",
    "mut", "impl", "use", "match", "long", "short", "byte", "undefined",
];
const PYTHON: &[&str] = &[
    "def", "class", "if", "elif", "else", "for", "while", "return", "import", "from", "as", "in", "not", "and", "or", "is", "None", "True", "False", "with", "try", "except", "finally", "raise", "lambda",
    "yield", "pass", "break", "continue", "global", "self", "async", "await",
];
const SHELL: &[&str] = &["if", "then", "else", "elif", "fi", "for", "do", "done", "while", "case", "esac", "function", "in", "echo", "export", "local", "return", "cd", "sudo"];
const SQL: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create", "table", "join", "left", "right", "inner", "on", "group", "by", "order", "having", "and", "or", "not", "null",
    "as", "limit", "primary", "key",
];

fn language(name: &str) -> Option<Language> {
    Some(match name.to_lowercase().as_str() {
        "swift" | "java" | "kotlin" | "js" | "javascript" | "ts" | "typescript" | "jsx" | "tsx" | "c" | "cpp" | "c++" | "h" | "cs" | "csharp" | "go" | "rust" | "rs" | "dart" | "php" | "scala" => {
            Language { keywords: C_LIKE, line_comment: &["//"], block_comment: Some(("/*", "*/")), types_capitalized: true }
        }
        "python" | "py" => Language { keywords: PYTHON, line_comment: &["#"], block_comment: None, types_capitalized: true },
        "sh" | "bash" | "zsh" | "shell" | "console" => Language { keywords: SHELL, line_comment: &["#"], block_comment: None, types_capitalized: false },
        "sql" => Language { keywords: SQL, line_comment: &["--"], block_comment: Some(("/*", "*/")), types_capitalized: false },
        "json" | "yaml" | "yml" | "toml" => Language { keywords: &["true", "false", "null"], line_comment: &["#"], block_comment: None, types_capitalized: false },
        "css" | "scss" => Language { keywords: &["important"], line_comment: &[], block_comment: Some(("/*", "*/")), types_capitalized: false },
        "html" | "xml" | "svg" => Language { keywords: &[], line_comment: &[], block_comment: Some(("<!--", "-->")), types_capitalized: false },
        _ => return None,
    })
}

/// Splits code into coloured runs; unknown languages come back as one plain run.
pub fn highlight(code: &str, lang_name: Option<&str>) -> Vec<(String, TokenKind)> {
    let Some(name) = lang_name else { return vec![(code.to_string(), TokenKind::Plain)] };
    let Some(lang) = language(name) else { return vec![(code.to_string(), TokenKind::Plain)] };
    let markup = ["html", "xml", "svg"].contains(&name.to_lowercase().as_str());
    let case_insensitive = name.eq_ignore_ascii_case("sql");
    let chars: Vec<char> = code.chars().collect();
    let mut out: Vec<(String, TokenKind)> = Vec::new();
    let mut plain = String::new();
    let starts = |s: &str, k: usize| -> bool {
        let a: Vec<char> = s.chars().collect();
        k + a.len() <= chars.len() && chars[k..k + a.len()] == a[..]
    };
    let flush = |plain: &mut String, out: &mut Vec<(String, TokenKind)>| {
        if !plain.is_empty() {
            out.push((std::mem::take(plain), TokenKind::Plain));
        }
    };
    let collect = |i: usize, j: usize| chars[i..j].iter().collect::<String>();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some((open, close)) = lang.block_comment
            && starts(open, i)
        {
            flush(&mut plain, &mut out);
            let mut j = i + open.chars().count();
            while j < chars.len() && !starts(close, j) {
                j += 1;
            }
            j = (j + close.chars().count()).min(chars.len());
            out.push((collect(i, j), TokenKind::Comment));
            i = j;
            continue;
        }
        if let Some(lc) = lang.line_comment.iter().find(|lc| starts(lc, i))
            && !(*lc == "#" && i > 0 && !chars[i - 1].is_whitespace())
        {
            flush(&mut plain, &mut out);
            let mut j = i;
            while j < chars.len() && chars[j] != '\n' {
                j += 1;
            }
            out.push((collect(i, j), TokenKind::Comment));
            i = j;
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            flush(&mut plain, &mut out);
            let mut j = i + 1;
            while j < chars.len() && chars[j] != c && !(chars[j] == '\n' && c != '`') {
                if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            j = (j + 1).min(chars.len());
            out.push((collect(i, j), TokenKind::String));
            i = j;
            continue;
        }
        if markup && c == '<' {
            flush(&mut plain, &mut out);
            let mut j = i + 1;
            if j < chars.len() && chars[j] == '/' {
                j += 1;
            }
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '-') {
                j += 1;
            }
            out.push((collect(i, j), TokenKind::Keyword));
            i = j;
            continue;
        }
        if c == '@' && !markup {
            flush(&mut plain, &mut out);
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            out.push((collect(i, j), TokenKind::Attribute));
            i = j;
            continue;
        }
        if c.is_ascii_digit() && (i == 0 || !(chars[i - 1].is_alphabetic() || chars[i - 1] == '_')) {
            flush(&mut plain, &mut out);
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_hexdigit() || chars[j] == '.' || chars[j] == 'x' || chars[j] == '_') {
                j += 1;
            }
            out.push((collect(i, j), TokenKind::Number));
            i = j;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word = collect(i, j);
            let key = if case_insensitive { word.to_lowercase() } else { word.clone() };
            if lang.keywords.contains(&key.as_str()) {
                flush(&mut plain, &mut out);
                out.push((word, TokenKind::Keyword));
            } else if lang.types_capitalized && word.chars().next().is_some_and(char::is_uppercase) {
                flush(&mut plain, &mut out);
                out.push((word, TokenKind::Type));
            } else {
                plain.push_str(&word);
            }
            i = j;
            continue;
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut plain, &mut out);
    out
}

fn highlighted_html(code: &str, lang: Option<&str>) -> String {
    highlight(code, lang)
        .into_iter()
        .map(|(t, k)| if k == TokenKind::Plain { escape(&t) } else { format!("<span class=\"tok-{}\">{}</span>", k.css(), escape(&t)) })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_tasks() {
        assert_eq!(toggle_task("a\n- [ ] b", 1), "a\n- [x] b");
        assert_eq!(toggle_task("- [X] b", 0), "- [ ] b");
    }

    #[test]
    fn finds_titles() {
        assert_eq!(title("---\ntitle: \"Hi\"\n---\n# X").as_deref(), Some("Hi"));
        assert_eq!(title("text\n\n## Second *one*").as_deref(), Some("Second one"));
    }

    #[test]
    fn renders_tasks_marks_and_footnotes() {
        let html = to_html("- [x] done ==hot==\n\nSee[^a].\n\n[^a]: Note.", None);
        assert!(html.contains("<li class=\"task\"><span class=\"box done\"></span>done <mark>hot</mark></li>"), "{html}");
        assert!(html.contains("<li id=\"fn-1\">Note.</li>"), "{html}");
    }

    #[test]
    fn highlights_code() {
        let t = highlight("let x = \"a\" // c", Some("swift"));
        assert_eq!(t[0], ("let".into(), TokenKind::Keyword));
        assert!(t.iter().any(|(s, k)| s == "\"a\"" && *k == TokenKind::String));
        assert!(t.iter().any(|(s, k)| s == "// c" && *k == TokenKind::Comment));
    }
}
