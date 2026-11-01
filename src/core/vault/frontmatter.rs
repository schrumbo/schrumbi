use std::ops::Range;

use serde_yaml_ng::Value;

/// The kinds of property Obsidian offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyKind {
    Text,
    List,
    Number,
    Checkbox,
    Date,
    DateTime,
    Tags,
}

/// One property as read from the note.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    pub kind: PropertyKind,
    pub values: Vec<String>,
    /// Holds nested data that fields cannot show. It stays readable and is never written back.
    pub nested: bool,
}

/// The byte range of the front matter's content, between the `---` lines, if the note has any.
pub fn range(text: &str) -> Option<Range<usize>> {
    let start = text.strip_prefix('\u{feff}').map_or(0, |_| '\u{feff}'.len_utf8());
    let rest = &text[start..];
    let first = rest.split_inclusive('\n').next()?;
    if first.trim_end() != "---" {
        return None;
    }
    let body = start + first.len();
    let mut offset = body;
    for line in text[body..].split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some(body..offset);
        }
        offset += line.len();
    }
    None
}

/// The text after the front matter.
pub fn body_offset(text: &str) -> usize {
    let Some(r) = range(text) else { return 0 };
    let after = &text[r.end..];
    r.end + after.split_inclusive('\n').next().map_or(0, str::len)
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null => Some(String::new()),
        _ => None,
    }
}

fn looks_like_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn looks_like_datetime(s: &str) -> bool {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").is_ok() || chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").is_ok()
}

/// What a value looks like, when `types.json` does not say.
pub fn infer_kind(name: &str, v: &Value) -> PropertyKind {
    if matches!(name, "tags" | "tag") {
        return PropertyKind::Tags;
    }
    match v {
        Value::Sequence(_) => PropertyKind::List,
        Value::Number(_) => PropertyKind::Number,
        Value::Bool(_) => PropertyKind::Checkbox,
        Value::String(s) if looks_like_date(s) => PropertyKind::Date,
        Value::String(s) if looks_like_datetime(s) => PropertyKind::DateTime,
        _ if matches!(name, "aliases" | "alias" | "cssclasses" | "cssclass") => PropertyKind::List,
        _ => PropertyKind::Text,
    }
}

/// Reads the properties; `None` when the block is not valid YAML. One nested value never hides the others.
pub fn properties(text: &str) -> Option<Vec<Property>> {
    let r = range(text)?;
    let value: Value = serde_yaml_ng::from_str(&text[r]).ok()?;
    let Value::Mapping(map) = value else { return Some(Vec::new()) };
    let mut out = Vec::new();
    for (k, v) in &map {
        let Some(name) = scalar(k) else { continue };
        let simple = match v {
            Value::Sequence(items) => items.iter().map(scalar).collect::<Option<Vec<_>>>(),
            other => scalar(other).map(|s| vec![s]),
        };
        let nested = simple.is_none();
        let kind = if nested { PropertyKind::Text } else { infer_kind(&name, v) };
        let values = simple.unwrap_or_else(|| vec![serde_yaml_ng::to_string(v).unwrap_or_default().trim().to_string()]);
        out.push(Property { name, kind, values, nested });
    }
    Some(out)
}

/// The string values of one property, comma lists and single values included.
pub fn list_property(text: &str, names: &[&str]) -> Vec<String> {
    let Some(props) = properties(text) else { return Vec::new() };
    props.into_iter().filter(|p| names.contains(&p.name.as_str())).flat_map(|p| p.values).flat_map(|v| if v.contains(',') { v.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>() } else { vec![v] }).filter(|v| !v.is_empty()).collect()
}

pub fn aliases(text: &str) -> Vec<String> {
    list_property(text, &["aliases", "alias"])
}

pub fn tags(text: &str) -> Vec<String> {
    list_property(text, &["tags", "tag"]).into_iter().map(|t| t.trim_start_matches('#').to_string()).collect()
}

fn line_ending(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

fn quote(value: &str) -> String {
    let needs = value.is_empty() || value.starts_with(['[', '{', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', '-', ' ']) || value.ends_with(' ') || value.contains(": ") || value.contains(" #") || matches!(value, "true" | "false" | "null" | "~") || value.parse::<f64>().is_ok();
    if needs { format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")) } else { value.to_string() }
}

/// How a property is written for its kind.
fn render(name: &str, kind: PropertyKind, values: &[String], nl: &str) -> String {
    match kind {
        PropertyKind::List | PropertyKind::Tags => {
            if values.is_empty() {
                return format!("{name}: []{nl}");
            }
            let mut out = format!("{name}:{nl}");
            for v in values {
                out.push_str(&format!("  - {}{nl}", quote(v)));
            }
            out
        }
        PropertyKind::Number | PropertyKind::Checkbox | PropertyKind::Date | PropertyKind::DateTime => format!("{name}: {}{nl}", values.first().map_or("", String::as_str)),
        PropertyKind::Text => format!("{name}: {}{nl}", values.first().map_or(String::new(), |v| quote(v))),
    }
}

/// Lines of the block belonging to one property, as a byte range of the note.
fn property_lines(text: &str, block: &Range<usize>, name: &str) -> Option<Range<usize>> {
    let prefix = format!("{name}:");
    let mut offset = block.start;
    let mut found: Option<Range<usize>> = None;
    for line in text[block.clone()].split_inclusive('\n') {
        let top_level = !line.starts_with([' ', '\t', '-']) && !line.trim().is_empty();
        match &mut found {
            Some(r) if !top_level => r.end = offset + line.len(),
            Some(_) => break,
            None if line.starts_with(&prefix) => found = Some(offset..offset + line.len()),
            None => {}
        }
        offset += line.len();
    }
    found
}

/// Sets a property and touches only its lines. Creates the block if the note has none.
pub fn set(text: &str, name: &str, kind: PropertyKind, values: &[String]) -> String {
    let nl = line_ending(text);
    let rendered = render(name, kind, values, nl);
    match range(text) {
        Some(block) => match property_lines(text, &block, name) {
            Some(lines) => format!("{}{rendered}{}", &text[..lines.start], &text[lines.end..]),
            None => format!("{}{rendered}{}", &text[..block.end], &text[block.end..]),
        },
        None => {
            let start = text.strip_prefix('\u{feff}').map_or(0, |_| '\u{feff}'.len_utf8());
            format!("{}---{nl}{rendered}---{nl}{}", &text[..start], &text[start..])
        }
    }
}

/// Removes a property; an empty block goes with it.
pub fn remove(text: &str, name: &str) -> String {
    let Some(block) = range(text) else { return text.to_string() };
    let Some(lines) = property_lines(text, &block, name) else { return text.to_string() };
    let out = format!("{}{}", &text[..lines.start], &text[lines.end..]);
    if let Some(r) = range(&out)
        && out[r.clone()].trim().is_empty()
    {
        let start = out.strip_prefix('\u{feff}').map_or(0, |_| '\u{feff}'.len_utf8());
        return format!("{}{}", &out[..start], &out[body_offset(&out)..]);
    }
    out
}

/// Renames a property, keeping its value and place.
pub fn rename(text: &str, old: &str, new: &str) -> String {
    let Some(block) = range(text) else { return text.to_string() };
    let Some(lines) = property_lines(text, &block, old) else { return text.to_string() };
    format!("{}{new}:{}", &text[..lines.start], &text[lines.start + old.len() + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nested_value_hides_nothing_else() {
        let text = "---\ntags: [a]\naliases: [B]\nmeta:\n  x: 1\nstatus: offen\n---\nText";
        let props = properties(text).unwrap();
        assert_eq!(props.len(), 4);
        assert!(props.iter().find(|p| p.name == "meta").unwrap().nested);
        assert_eq!(aliases(text), ["B"]);
        assert_eq!(tags(text), ["a"]);
    }

    const NOTE: &str = "---\ntitle: Hallo\nanzahl: 3\nfertig: true\ndatum: 2026-10-31\nzeit: 2026-10-31T12:00:00\ntags:\n  - a\n  - b/c\naliases: [Eins, Zwei]\nlink: \"[[Ziel]]\"\n---\nText\n";

    #[test]
    fn reads_all_kinds() {
        let props = properties(NOTE).unwrap();
        let kind = |n: &str| props.iter().find(|p| p.name == n).unwrap().kind;
        assert_eq!(kind("title"), PropertyKind::Text);
        assert_eq!(kind("anzahl"), PropertyKind::Number);
        assert_eq!(kind("fertig"), PropertyKind::Checkbox);
        assert_eq!(kind("datum"), PropertyKind::Date);
        assert_eq!(kind("zeit"), PropertyKind::DateTime);
        assert_eq!(kind("tags"), PropertyKind::Tags);
        assert_eq!(kind("aliases"), PropertyKind::List);
        assert_eq!(aliases(NOTE), ["Eins", "Zwei"]);
        assert_eq!(tags(NOTE), ["a", "b/c"]);
        assert_eq!(&NOTE[body_offset(NOTE)..], "Text\n");
    }

    #[test]
    fn notes_without_or_with_broken_blocks_are_left_alone() {
        assert_eq!(range("Text\n---\n"), None);
        assert_eq!(properties("---\nnicht: geschlossen\n"), None);
        assert_eq!(properties("---\n: : kaputt\n  x\n---\n"), None);
        assert!(properties("---\na:\n  b: verschachtelt\n---\n").unwrap()[0].nested);
    }

    #[test]
    fn setting_touches_only_that_property() {
        let changed = set(NOTE, "anzahl", PropertyKind::Number, &["4".to_string()]);
        assert_eq!(changed, NOTE.replace("anzahl: 3", "anzahl: 4"));
        let tags = set(NOTE, "tags", PropertyKind::Tags, &["x".to_string()]);
        assert_eq!(tags, NOTE.replace("  - a\n  - b/c\n", "  - x\n"));
        let added = set(NOTE, "neu", PropertyKind::Text, &["Wert: mit Doppelpunkt".to_string()]);
        assert!(added.contains("link: \"[[Ziel]]\"\nneu: \"Wert: mit Doppelpunkt\"\n---\n"));
        assert_eq!(properties(&added).unwrap().iter().find(|p| p.name == "neu").unwrap().values, ["Wert: mit Doppelpunkt"]);
    }

    #[test]
    fn a_new_block_goes_first_and_keeps_bom_and_line_endings() {
        assert_eq!(set("Text", "a", PropertyKind::Text, &["b".to_string()]), "---\na: b\n---\nText");
        let crlf = set("\u{feff}x\r\ny", "a", PropertyKind::List, &["b".to_string()]);
        assert_eq!(crlf, "\u{feff}---\r\na:\r\n  - b\r\n---\r\nx\r\ny");
    }

    #[test]
    fn removing_and_renaming() {
        let without = remove(NOTE, "tags");
        assert!(!without.contains("tags") && !without.contains("  - a"));
        assert!(without.contains("aliases:"));
        assert_eq!(remove("---\na: b\n---\nText", "a"), "Text");
        assert_eq!(remove("Text", "a"), "Text");
        assert!(rename(NOTE, "anzahl", "menge").contains("menge: 3\n"));
    }

    #[test]
    fn values_that_would_change_meaning_get_quotes() {
        for v in ["[[Link]]", "true", "12", "#tag", "a: b", ""] {
            let note = set("Text", "k", PropertyKind::Text, &[v.to_string()]);
            let read = properties(&note).unwrap();
            assert_eq!(read[0].values, [v], "{note}");
        }
    }
}
