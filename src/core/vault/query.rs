use std::path::{Path, PathBuf};

use regex::Regex;

use super::frontmatter;
use super::index::VaultIndex;
use super::links::{self};

/// A parsed search in Obsidian's syntax.
#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    Term(Matcher),
    All(Vec<Query>),
    Any(Vec<Query>),
    Not(Box<Query>),
    File(Box<Query>),
    Path(Box<Query>),
    Content(Box<Query>),
    Tag(String),
    Line(Box<Query>),
    Block(Box<Query>),
    Section(Box<Query>),
    Task(Box<Query>, TaskState),
    Property { name: Box<Query>, value: Option<PropertyTest> },
    MatchCase(Box<Query>),
    IgnoreCase(Box<Query>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Any,
    Todo,
    Done,
}

/// A word, an exact phrase, or a regular expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Matcher {
    Word(String),
    Phrase(String),
    Regex(String),
}

/// A condition on a property's value.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyTest {
    Null,
    Less(f64),
    Greater(f64),
    Matches(Box<Query>),
}

impl Matcher {
    fn test(&self, text: &str, case_sensitive: bool) -> bool {
        match self {
            Matcher::Word(w) | Matcher::Phrase(w) => {
                if case_sensitive {
                    text.contains(w.as_str())
                } else {
                    text.to_lowercase().contains(&w.to_lowercase())
                }
            }
            Matcher::Regex(r) => Regex::new(&if case_sensitive { r.clone() } else { format!("(?i){r}") }).is_ok_and(|re| re.is_match(text)),
        }
    }
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn spaces(&mut self) {
        while self.chars.get(self.at).is_some_and(|c| c.is_whitespace()) {
            self.at += 1;
        }
    }

    fn peek_word(&self, word: &str) -> bool {
        let w: Vec<char> = word.chars().collect();
        self.chars[self.at..].starts_with(&w) && self.chars.get(self.at + w.len()).is_none_or(|c| c.is_whitespace() || *c == '(' || *c == ')')
    }

    fn any(&mut self) -> Result<Query, String> {
        let mut parts = vec![self.all()?];
        loop {
            self.spaces();
            if self.peek_word("OR") {
                self.at += 2;
                parts.push(self.all()?);
            } else {
                break;
            }
        }
        Ok(if parts.len() == 1 { parts.remove(0) } else { Query::Any(parts) })
    }

    fn all(&mut self) -> Result<Query, String> {
        let mut parts = Vec::new();
        loop {
            self.spaces();
            match self.chars.get(self.at) {
                None | Some(')') | Some(']') => break,
                _ if self.peek_word("OR") => break,
                _ => parts.push(self.term()?),
            }
        }
        if parts.is_empty() {
            return Err("Leere Suche".into());
        }
        Ok(if parts.len() == 1 { parts.remove(0) } else { Query::All(parts) })
    }

    fn group_or_term(&mut self) -> Result<Query, String> {
        self.spaces();
        if self.chars.get(self.at) == Some(&'(') {
            self.at += 1;
            let q = self.any()?;
            self.spaces();
            if self.chars.get(self.at) != Some(&')') {
                return Err(") fehlt".into());
            }
            self.at += 1;
            return Ok(q);
        }
        self.atom()
    }

    fn term(&mut self) -> Result<Query, String> {
        self.spaces();
        match self.chars.get(self.at) {
            Some('-') => {
                self.at += 1;
                Ok(Query::Not(self.term()?.into()))
            }
            Some('[') => self.property(),
            Some('(') => self.group_or_term(),
            _ => {
                for (name, make) in OPERATORS {
                    let w: Vec<char> = name.chars().collect();
                    if self.chars[self.at..].starts_with(&w) {
                        self.at += w.len();
                        if *name == "tag:" {
                            self.spaces();
                            let Query::Term(Matcher::Word(t) | Matcher::Phrase(t)) = self.atom()? else { return Err("tag: braucht einen Namen".into()) };
                            return Ok(Query::Tag(t.trim_start_matches('#').to_string()));
                        }
                        let inner = self.group_or_term()?;
                        return Ok(make(inner));
                    }
                }
                self.atom()
            }
        }
    }

    fn atom(&mut self) -> Result<Query, String> {
        self.spaces();
        match self.chars.get(self.at).copied() {
            Some('"') => {
                self.at += 1;
                let mut out = String::new();
                while let Some(c) = self.chars.get(self.at).copied() {
                    self.at += 1;
                    match c {
                        '"' => return Ok(Query::Term(Matcher::Phrase(out))),
                        '\\' => {
                            if let Some(n) = self.chars.get(self.at).copied() {
                                self.at += 1;
                                out.push(n);
                            }
                        }
                        c => out.push(c),
                    }
                }
                Err("Anführungszeichen fehlt".into())
            }
            Some('/') => {
                self.at += 1;
                let mut out = String::new();
                while let Some(c) = self.chars.get(self.at).copied() {
                    self.at += 1;
                    match c {
                        '/' => {
                            Regex::new(&out).map_err(|e| e.to_string())?;
                            return Ok(Query::Term(Matcher::Regex(out)));
                        }
                        '\\' => {
                            out.push('\\');
                            if let Some(n) = self.chars.get(self.at).copied() {
                                self.at += 1;
                                out.push(n);
                            }
                        }
                        c => out.push(c),
                    }
                }
                Err("/ fehlt".into())
            }
            _ => {
                let start = self.at;
                while self.chars.get(self.at).is_some_and(|c| !c.is_whitespace() && !matches!(c, '(' | ')' | '[' | ']')) {
                    self.at += 1;
                }
                if self.at == start {
                    return Err("Suchbegriff fehlt".into());
                }
                Ok(Query::Term(Matcher::Word(self.chars[start..self.at].iter().collect())))
            }
        }
    }

    fn property(&mut self) -> Result<Query, String> {
        self.at += 1;
        let start = self.at;
        let mut depth = 0;
        let mut colon = None;
        let mut quoted = false;
        while let Some(c) = self.chars.get(self.at).copied() {
            match c {
                '"' => quoted = !quoted,
                '(' if !quoted => depth += 1,
                ')' if !quoted => depth -= 1,
                ':' if !quoted && depth == 0 && colon.is_none() => colon = Some(self.at),
                ']' if !quoted && depth == 0 => break,
                _ => {}
            }
            self.at += 1;
        }
        if self.chars.get(self.at) != Some(&']') {
            return Err("] fehlt".into());
        }
        let end = self.at;
        self.at += 1;
        let parse_inner = |from: usize, to: usize| -> Result<Query, String> {
            let mut p = Parser { chars: self.chars[from..to].to_vec(), at: 0 };
            p.any()
        };
        let Some(colon) = colon else { return Ok(Query::Property { name: parse_inner(start, end)?.into(), value: None }) };
        let name = parse_inner(start, colon)?;
        let raw: String = self.chars[colon + 1..end].iter().collect();
        let raw = raw.trim();
        let value = if raw == "null" {
            PropertyTest::Null
        } else if let Some(n) = raw.strip_prefix('<').and_then(|n| n.trim().parse().ok()) {
            PropertyTest::Less(n)
        } else if let Some(n) = raw.strip_prefix('>').and_then(|n| n.trim().parse().ok()) {
            PropertyTest::Greater(n)
        } else {
            PropertyTest::Matches(parse_inner(colon + 1, end)?.into())
        };
        Ok(Query::Property { name: name.into(), value: Some(value) })
    }
}

type Make = fn(Query) -> Query;

const OPERATORS: &[(&str, Make)] = &[
    ("file:", |q| Query::File(q.into())),
    ("path:", |q| Query::Path(q.into())),
    ("content:", |q| Query::Content(q.into())),
    ("match-case:", |q| Query::MatchCase(q.into())),
    ("ignore-case:", |q| Query::IgnoreCase(q.into())),
    ("tag:", |q| q),
    ("line:", |q| Query::Line(q.into())),
    ("block:", |q| Query::Block(q.into())),
    ("section:", |q| Query::Section(q.into())),
    ("task-todo:", |q| Query::Task(q.into(), TaskState::Todo)),
    ("task-done:", |q| Query::Task(q.into(), TaskState::Done)),
    ("task:", |q| Query::Task(q.into(), TaskState::Any)),
];

/// Parses Obsidian's search language.
pub fn parse(source: &str) -> Result<Query, String> {
    let mut p = Parser { chars: source.chars().collect(), at: 0 };
    let q = p.any()?;
    p.spaces();
    if p.at < p.chars.len() { Err(format!("Unerwartet: {}", p.chars[p.at..].iter().collect::<String>())) } else { Ok(q) }
}

/// Whether the search uses anything beyond plain words, so the ordinary search stays for the rest.
pub fn is_structured(source: &str) -> bool {
    let s = source.trim();
    s.contains(" OR ") || s.starts_with('[') || s.starts_with('-') || s.contains(" -") || s.contains('"') || (s.starts_with('/') && s.len() > 2 && s.ends_with('/')) || OPERATORS.iter().any(|(name, _)| s.contains(name))
}

/// One file's text, cut as the operators need it.
struct Doc<'a> {
    path: &'a Path,
    root: &'a Path,
    text: &'a str,
    index: &'a VaultIndex,
    case_sensitive: bool,
}

fn blocks_of(text: &str) -> Vec<String> {
    text.split("\n\n").map(String::from).collect()
}

fn sections_of(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut cuts: Vec<usize> = links::headings(text).iter().map(|(_, _, l)| *l).collect();
    cuts.insert(0, 0);
    cuts.push(lines.len());
    cuts.windows(2).map(|w| lines[w[0]..w[1]].join("\n")).collect()
}

impl Doc<'_> {
    fn with_text(&self, text: &str, q: &Query) -> bool {
        Doc { path: self.path, root: self.root, text, index: self.index, case_sensitive: self.case_sensitive }.test_text(q)
    }

    /// Plain terms against just this text; used inside `line:` and friends.
    fn test_text(&self, q: &Query) -> bool {
        match q {
            Query::Term(m) => m.test(self.text, self.case_sensitive),
            Query::All(l) => l.iter().all(|x| self.test_text(x)),
            Query::Any(l) => l.iter().any(|x| self.test_text(x)),
            Query::Not(x) => !self.test_text(x),
            other => self.test(other),
        }
    }

    fn test(&self, q: &Query) -> bool {
        let rel = self.path.strip_prefix(self.root).unwrap_or(self.path).to_string_lossy().replace('\\', "/");
        let name = self.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        match q {
            Query::Term(m) => m.test(self.text, self.case_sensitive) || m.test(&name, self.case_sensitive),
            Query::All(l) => l.iter().all(|x| self.test(x)),
            Query::Any(l) => l.iter().any(|x| self.test(x)),
            Query::Not(x) => !self.test(x),
            Query::File(x) => self.with_text(&name, x),
            Query::Path(x) => self.with_text(&rel, x),
            Query::Content(x) => self.test_text(x),
            Query::Tag(t) => self.index.notes.get(self.path).is_some_and(|f| f.tags.iter().any(|x| x.eq_ignore_ascii_case(t) || x.to_lowercase().starts_with(&format!("{}/", t.to_lowercase())))),
            Query::Line(x) => self.text.split('\n').any(|l| self.with_text(l, x)),
            Query::Block(x) => blocks_of(self.text).iter().any(|b| self.with_text(b, x)),
            Query::Section(x) => sections_of(self.text).iter().any(|s| self.with_text(s, x)),
            Query::Task(x, state) => self.text.split('\n').any(|l| {
                let Some((_, mark)) = super::syntax::task_marker(l) else { return false };
                let wanted = match state {
                    TaskState::Any => true,
                    TaskState::Todo => mark == ' ',
                    TaskState::Done => mark != ' ',
                };
                wanted && self.with_text(l, x)
            }),
            Query::Property { name, value } => frontmatter::properties(self.text).is_some_and(|props| {
                props.iter().any(|p| {
                    if !self.with_text(&p.name, name) {
                        return false;
                    }
                    let joined = p.values.join(" ");
                    match value {
                        None => true,
                        Some(PropertyTest::Null) => p.values.iter().all(String::is_empty) && !p.values.is_empty(),
                        Some(PropertyTest::Less(n)) => joined.parse::<f64>().is_ok_and(|v| v < *n),
                        Some(PropertyTest::Greater(n)) => joined.parse::<f64>().is_ok_and(|v| v > *n),
                        Some(PropertyTest::Matches(m)) => p.values.iter().any(|v| self.with_text(v, m)),
                    }
                })
            }),
            Query::MatchCase(x) => Doc { path: self.path, root: self.root, text: self.text, index: self.index, case_sensitive: true }.test(x),
            Query::IgnoreCase(x) => Doc { path: self.path, root: self.root, text: self.text, index: self.index, case_sensitive: false }.test(x),
        }
    }
}

/// How results are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    NameAsc,
    NameDesc,
    ModifiedNew,
    ModifiedOld,
    CreatedNew,
    CreatedOld,
}

/// Files matching the search, in order. Reads files, so run it off the main thread.
pub fn search(query: &Query, index: &VaultIndex, sort: Sort) -> Vec<PathBuf> {
    use rayon::prelude::*;
    let mut found: Vec<PathBuf> = index
        .files
        .par_iter()
        .filter(|file| {
            let is_text = crate::core::markdown::is_markdown(file) || file.extension().is_some_and(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "txt" | "canvas" | "base"));
            let text = if is_text { std::fs::read_to_string(file).unwrap_or_default() } else { String::new() };
            Doc { path: file, root: &index.root, text: &text, index, case_sensitive: false }.test(query)
        })
        .cloned()
        .collect();
    let key = |p: &PathBuf, created: bool| std::fs::metadata(p).ok().and_then(|m| if created { m.created().ok() } else { m.modified().ok() });
    match sort {
        Sort::NameAsc => found.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase())),
        Sort::NameDesc => found.sort_by_key(|p| std::cmp::Reverse(p.file_name().map(|n| n.to_string_lossy().to_lowercase()))),
        Sort::ModifiedNew => found.sort_by_key(|p| std::cmp::Reverse(key(p, false))),
        Sort::ModifiedOld => found.sort_by_key(|p| key(p, false)),
        Sort::CreatedNew => found.sort_by_key(|p| std::cmp::Reverse(key(p, true))),
        Sort::CreatedOld => found.sort_by_key(|p| key(p, true)),
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::vault::index::{facts_of, rebuild_for_test};

    fn check(cases: &[(&str, &[&str])]) {
        let dir = std::env::temp_dir().join(format!("schrumbi-query-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Tage/2022-07")).unwrap();
        let notes = [
            ("Treffen.md", "---\nstatus: Draft\nduration: 3\naliases: [Name]\nleer:\n---\n# Arbeit\nmeeting work heute\n\n- [ ] call boss\n- [x] email done\n#work\n"),
            ("Tage/2022-07/Montag.md", "meeting personal\n\ndog\n\ncat\n# Neu\nHappyCat star wars\n#myjob/werk\n"),
            ("Notiz.md", "---\nstatus: Published\nduration: 9\n---\ndog cat\n\nsie sagte \"hallo\" zu mir 2026-10-31\n"),
        ];
        for (p, t) in notes {
            std::fs::write(dir.join(p), t).unwrap();
        }
        let dir = dunce::canonicalize(&dir).unwrap();
        let index = rebuild_for_test(&dir, |t| facts_of(t, 0.));
        for (source, wanted) in cases {
            let q = parse(source).unwrap_or_else(|e| panic!("{source}: {e}"));
            let mut got: Vec<String> = search(&q, &index, Sort::NameAsc).iter().map(|p| p.file_stem().unwrap().to_string_lossy().to_string()).collect();
            got.sort();
            let mut want: Vec<String> = wanted.iter().map(|s| s.to_string()).collect();
            want.sort();
            assert_eq!(got, want, "{source}");
        }
    }

    #[test]
    fn every_operator_from_the_help_works() {
        check(&[
            ("meeting work", &["Treffen"]),
            ("\"star wars\"", &["Montag"]),
            (r#""sagte \"hallo\" zu""#, &["Notiz"]),
            ("meeting OR dog", &["Treffen", "Montag", "Notiz"]),
            ("meeting work OR dog personal", &["Treffen", "Montag"]),
            ("meeting (work OR personal) heute", &["Treffen"]),
            ("meeting -work", &["Montag"]),
            ("meeting -work -personal", &[]),
            ("dog -(cat personal)", &["Notiz"]),
            ("file:Mon", &["Montag"]),
            ("file:.md meeting", &["Treffen", "Montag"]),
            ("path:\"Tage/2022-07\"", &["Montag"]),
            ("content:\"HappyCat\"", &["Montag"]),
            ("match-case:happycat", &[]),
            ("match-case:HappyCat", &["Montag"]),
            ("ignore-case:happycat", &["Montag"]),
            ("tag:#work", &["Treffen"]),
            ("tag:myjob", &["Montag"]),
            ("tag:#werk", &[]),
            ("line:(dog cat)", &["Notiz"]),
            ("block:(dog cat)", &["Notiz"]),
            ("section:(meeting work)", &["Treffen"]),
            ("section:(dog HappyCat)", &[]),
            ("task:call", &["Treffen"]),
            ("task-todo:call", &["Treffen"]),
            ("task-done:call", &[]),
            ("task-done:email", &["Treffen"]),
            ("task:(call OR email)", &["Treffen"]),
            ("[aliases]", &["Treffen"]),
            ("[aliases:Name]", &["Treffen"]),
            ("[leer:null]", &["Treffen"]),
            ("[status:Draft OR Published]", &["Treffen", "Notiz"]),
            ("[duration:<5]", &["Treffen"]),
            ("[duration:>5]", &["Notiz"]),
            (r"/\d{4}-\d{2}-\d{2}/", &["Notiz"]),
            (r"path:/\d{4}-\d{2}/", &["Montag"]),
        ]);
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(parse("").is_err());
        assert!(parse("(a").is_err());
        assert!(parse("\"a").is_err());
        assert!(parse("/(/").is_err());
        assert!(parse("[a").is_err());
    }

    #[test]
    fn only_structured_searches_are_routed_here() {
        assert!(!is_structured("radar w3"));
        for s in ["a OR b", "-a", "a -b", "\"a b\"", "tag:x", "[a]", "/a/", "file:x"] {
            assert!(is_structured(s), "{s}");
        }
    }
}
