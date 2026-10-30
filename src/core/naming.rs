use chrono::{Datelike, Local, NaiveDate};
use regex::Regex;

use super::model::{CoauthorStyle, Module, Semester, StudyConfig};
use super::{day, filing, text};

/// Title, co-authors and date read back from a convention name.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub title: String,
    pub coauthors: Vec<String>,
    pub date: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
enum Token {
    Literal(String),
    Code,
    Yy,
    Yyyy,
    Title,
    Author,
    Matrikel,
    Week,
    IsoWeek,
    Date(String),
}

/// Renders, parses and validates file names against a module's naming pattern.
///
/// Placeholders: `{code}` `{yy}` `{yyyy}` `{title}` `{author}` `{matrikel}` `{date}` (yyMMdd),
/// `{date:FORMAT}` (e.g. `{date:yyyy-MM-dd}`), `{w}` lecture week, `{kw}` calendar week.
#[derive(Debug, Clone)]
pub struct Naming {
    code: String,
    year: i32,
    author: String,
    matrikel: String,
    coauthor_style: CoauthorStyle,
    tokens: Vec<Token>,
    valid: Option<Regex>,
    capture: Option<Regex>,
}

impl Naming {
    pub fn new(module: &Module, semester: Option<&Semester>, config: &StudyConfig) -> Self {
        Self::with_author(module, semester, &config.author, &config.matrikel, config.coauthor_style)
    }

    pub fn with_author(module: &Module, semester: Option<&Semester>, author: &str, matrikel: &str, style: CoauthorStyle) -> Self {
        let year = semester.and_then(|s| s.start_date()).map_or_else(|| Local::now().year(), |d| d.year());
        let mut n = Self {
            code: module.code.clone(),
            year,
            author: author.to_string(),
            matrikel: matrikel.to_string(),
            coauthor_style: style,
            tokens: tokenize(&module.naming),
            valid: None,
            capture: None,
        };
        n.valid = Regex::new(&n.regex(false)).ok();
        n.capture = Regex::new(&n.regex(true)).ok();
        n
    }

    /// Normalizes free text into a file-name-safe title: words joined with `-`, umlauts transliterated.
    pub fn clean(raw: &str) -> String {
        let s = raw.replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('Ä', "Ae").replace('Ö', "Oe").replace('Ü', "Ue").replace('ß', "ss");
        let joined = text::words(&text::strip_diacritics(&s)).collect::<Vec<_>>().join("-");
        let mut chars = joined.chars();
        let capitalized: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
        capitalized.chars().take(60).collect()
    }

    /// Normalizes a co-author entry like "Müller C" to `Mueller-C` (initial style) or a clean token (free style).
    pub fn clean_author(raw: &str, style: CoauthorStyle) -> String {
        let cleaned = Self::clean(raw);
        if style != CoauthorStyle::Initial {
            return cleaned;
        }
        let parts: Vec<&str> = cleaned.split('-').collect();
        match parts.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{}-{}", rest.join("-"), last.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
            _ => cleaned,
        }
    }

    /// `byline` replaces the author and co-authors, e.g. `Aufgabe` for a task sheet; empty drops the author part.
    pub fn render(&self, title: &str, date: NaiveDate, ext: &str, coauthors: &[String], week: Option<u32>, iso_week: Option<u32>, byline: Option<&str>) -> String {
        let clean = Self::clean(title);
        let authors = match byline {
            Some(b) => Self::clean_author(b, self.coauthor_style),
            None => std::iter::once(self.author.clone())
                .chain(coauthors.iter().map(|c| Self::clean_author(c, self.coauthor_style)).filter(|c| !c.is_empty() && *c != self.author))
                .collect::<Vec<_>>()
                .join("_"),
        };
        let mut base = String::new();
        for t in &self.tokens {
            match t {
                Token::Literal(s) => base.push_str(s),
                Token::Code => base.push_str(&self.code),
                Token::Yy => base.push_str(&format!("{:02}", self.year.rem_euclid(100))),
                Token::Yyyy => base.push_str(&self.year.to_string()),
                Token::Title => base.push_str(if clean.is_empty() { "Titel" } else { &clean }),
                Token::Author => base.push_str(&authors),
                Token::Matrikel => base.push_str(&self.matrikel),
                Token::Week => base.push_str(&format!("W{:02}", week.unwrap_or(0))),
                Token::IsoWeek => base.push_str(&format!("KW{:02}", iso_week.unwrap_or_else(|| day::iso_week(date)))),
                Token::Date(f) => base.push_str(&date.format(&strftime(f)).to_string()),
            }
        }
        if authors.is_empty() {
            while base.contains("__") {
                base = base.replace("__", "_");
            }
            base = base.trim_matches('_').to_string();
        }
        if ext.is_empty() { base } else { format!("{base}.{}", ext.to_lowercase()) }
    }

    fn regex(&self, capturing: bool) -> String {
        let co = if self.coauthor_style == CoauthorStyle::Initial { "(?:_[^_]+-[A-Z])*" } else { "(?:_[^_.]+)*?" };
        let mut out = String::from("^");
        for t in &self.tokens {
            match t {
                Token::Literal(s) => out.push_str(&regex::escape(s)),
                Token::Code => out.push_str(&regex::escape(&self.code)),
                Token::Yy => out.push_str(&if capturing { r"\d{2}".to_string() } else { format!("{:02}", self.year.rem_euclid(100)) }),
                Token::Yyyy => out.push_str(&if capturing { r"\d{4}".to_string() } else { self.year.to_string() }),
                Token::Title => out.push_str(if capturing { "(?P<title>.+?)" } else { ".+" }),
                Token::Author => {
                    out.push_str(&regex::escape(&self.author));
                    out.push_str(&if capturing { format!("(?P<co>{co})") } else { co.to_string() });
                }
                Token::Matrikel => out.push_str(&regex::escape(&self.matrikel)),
                Token::Week => out.push_str(r"W\d{2}"),
                Token::IsoWeek => out.push_str(r"KW\d{2}"),
                Token::Date(f) => {
                    let r = date_regex(f);
                    out.push_str(&if capturing { format!("(?P<date>{r})") } else { r });
                }
            }
        }
        out + r"\.[A-Za-z0-9]+$"
    }

    pub fn is_valid(&self, file_name: &str) -> bool {
        self.valid.as_ref().is_some_and(|r| r.is_match(file_name))
    }

    /// Extracts title, co-authors and date from a name that follows this pattern (any year).
    pub fn parse(&self, file_name: &str) -> Option<Parsed> {
        let c = self.capture.as_ref()?.captures(file_name)?;
        let coauthors = c.name("co").map_or("", |m| m.as_str()).split('_').filter(|s| !s.is_empty()).map(String::from).collect();
        let date = c.name("date").and_then(|raw| {
            self.tokens.iter().find_map(|t| match t {
                Token::Date(f) => NaiveDate::parse_from_str(raw.as_str(), &strftime(f)).ok(),
                _ => None,
            })
        });
        Some(Parsed { title: c.name("title").map_or(String::new(), |m| m.as_str().to_string()), coauthors, date })
    }

    /// Best-effort title extracted from an existing file name.
    pub fn guess_title(file_name: &str) -> String {
        Self::clean(&filing::clean_file_name(file_name).unwrap_or_else(|| stem(file_name).to_string()))
    }
}

pub fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

pub fn extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[i + 1..],
        _ => "",
    }
}

fn tokenize(pattern: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut rest = pattern;
    while let Some(c) = rest.chars().next() {
        if c == '{'
            && let Some(close) = rest.find('}')
        {
            let name = &rest[1..close];
            out.push(match name {
                "code" => Token::Code,
                "yy" => Token::Yy,
                "yyyy" => Token::Yyyy,
                "title" => Token::Title,
                "author" => Token::Author,
                "matrikel" => Token::Matrikel,
                "w" => Token::Week,
                "kw" => Token::IsoWeek,
                "date" => Token::Date("yyMMdd".into()),
                n if n.starts_with("date:") => Token::Date(n[5..].to_string()),
                n => Token::Literal(format!("{{{n}}}")),
            });
            rest = &rest[close + 1..];
        } else {
            out.push(Token::Literal(c.to_string()));
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

fn runs(f: &str) -> Vec<(char, usize)> {
    let mut out: Vec<(char, usize)> = Vec::new();
    for c in f.chars() {
        match out.last_mut() {
            Some((p, n)) if *p == c => *n += 1,
            _ => out.push((c, 1)),
        }
    }
    out
}

/// Unicode date pattern (`yyMMdd`) to a chrono format string.
fn strftime(f: &str) -> String {
    runs(f)
        .into_iter()
        .map(|(c, n)| match (c, n) {
            ('y', 4) => "%Y".into(),
            ('y', _) => "%y".into(),
            ('M', 1) => "%-m".into(),
            ('M', _) => "%m".into(),
            ('d', 1) => "%-d".into(),
            ('d', _) => "%d".into(),
            ('H', 1) => "%-H".into(),
            ('H', _) => "%H".into(),
            ('m', _) => "%M".into(),
            ('s', _) => "%S".into(),
            ('%', n) => "%%".repeat(n),
            (c, n) => c.to_string().repeat(n),
        })
        .collect()
}

fn date_regex(f: &str) -> String {
    runs(f)
        .into_iter()
        .map(|(c, n)| {
            if c.is_alphabetic() {
                let width = if c == 'y' && n == 4 { 4 } else if n == 1 { 1 } else { 2 };
                format!(r"\d{{{width}}}{}", if n == 1 { r"\d?" } else { "" })
            } else {
                regex::escape(&c.to_string().repeat(n))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naming(pattern: &str) -> Naming {
        let m = Module { code: "P1".into(), naming: pattern.into(), ..Default::default() };
        let s = Semester { start: "2026-10-12".into(), end: "2027-02-01".into(), ..Default::default() };
        Naming::with_author(&m, Some(&s), "Muster-M", "", CoauthorStyle::Initial)
    }

    #[test]
    fn renders_and_parses_round_trip() {
        let n = naming(crate::core::model::DEFAULT_NAMING);
        let d = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let name = n.render("Stakeholder Radar", d, "PDF", &["Müller C".into()], None, None, None);
        assert_eq!(name, "P1_26_Stakeholder-Radar_Muster-M_Mueller-C_261028.pdf");
        assert!(n.is_valid(&name));
        let p = n.parse(&name).unwrap();
        assert_eq!(p.title, "Stakeholder-Radar");
        assert_eq!(p.coauthors, ["Mueller-C"]);
        assert_eq!(p.date, Some(d));
        assert!(!n.is_valid("P1_26_x_Other-A_261028.pdf"));
    }

    #[test]
    fn supports_custom_date_formats() {
        let n = naming("{code}-{date:yyyy-MM-dd}-{title}");
        let d = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let name = n.render("Notiz", d, "md", &[], None, None, None);
        assert_eq!(name, "P1-2026-01-05-Notiz.md");
        assert_eq!(n.parse(&name).unwrap().date, Some(d));
    }

    #[test]
    fn cleans_titles() {
        assert_eq!(Naming::clean("  über die Café-Brücke! "), "Ueber-die-Cafe-Bruecke");
        assert_eq!(Naming::clean_author("Müller C", CoauthorStyle::Initial), "Mueller-C");
    }
}
