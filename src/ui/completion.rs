use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Result;
use gpui_kit::component::input::CompletionProvider;
use gpui_kit::*;
use lsp_types::{CompletionContext, CompletionItem, CompletionResponse, CompletionTextEdit, Position, Range, TextEdit};
use ropey::Rope;

use crate::core::i18n::tr;
use crate::core::vault::index;

/// What the caret is inside of.
#[derive(Debug, PartialEq, Eq)]
enum Context_ {
    /// `[[` with the text typed so far, and where the `[[` starts in the line.
    Link { typed: String, start: usize },
    /// `/` at the start of a line.
    Slash { typed: String, start: usize },
}

fn context_at(line: &str, column: usize) -> Option<Context_> {
    let before = line.get(..column)?;
    if let Some(open) = before.rfind("[[")
        && !before[open..].contains("]]")
    {
        let start = before[..open].chars().count();
        return Some(Context_::Link { typed: before[open + 2..].to_string(), start });
    }
    let trimmed = before.trim_start();
    if let Some(rest) = trimmed.strip_prefix('/')
        && !rest.contains(' ')
    {
        return Some(Context_::Slash { typed: rest.to_string(), start: before.chars().count() - trimmed.chars().count() });
    }
    None
}

/// Lower is better; `None` when the letters do not occur in order.
fn score(name: &str, query: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let (name, query) = (name.to_lowercase(), query.to_lowercase());
    if name.starts_with(&query) {
        return Some(-2000 + name.len() as i64);
    }
    if let Some(at) = name.find(&query) {
        return Some(-1000 + at as i64 + name.len() as i64);
    }
    crate::core::catalog::fuzzy(&query, &name).map(|s| (1000. - s * 100.) as i64)
}

fn item(label: String, detail: Option<String>, sort: String, range: Range, new_text: String) -> CompletionItem {
    CompletionItem { label, detail, sort_text: Some(sort), text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text })), ..Default::default() }
}

/// Suggestions while writing a link or a slash command.
pub struct NoteCompletion {
    pub note: PathBuf,
}

impl NoteCompletion {
    fn link_items(&self, typed: &str, range: Range) -> Vec<CompletionItem> {
        let vault = index::current();
        let close = "]]";
        let from = Some(self.note.as_path());
        if let Some((file, sub)) = typed.split_once('#') {
            let target = if file.is_empty() { Some(self.note.clone()) } else { vault.resolve(file, from) };
            let Some(target) = target else { return Vec::new() };
            let Some(facts) = vault.notes.get(&target) else { return Vec::new() };
            let prefix = if file.is_empty() { String::new() } else { file.to_string() };
            if let Some(block) = sub.strip_prefix('^') {
                return facts.blocks.iter().enumerate().filter(|(_, (id, _))| score(id, block).is_some()).map(|(i, (id, _))| item(format!("^{id}"), None, format!("{i:05}"), range, format!("[[{prefix}#^{id}{close}"))).collect();
            }
            return facts.headings.iter().enumerate().filter(|(_, (_, h, _))| score(h, sub).is_some()).map(|(i, (level, h, _))| item(h.clone(), Some("#".repeat(*level as usize)), format!("{i:05}"), range, format!("[[{prefix}#{h}{close}"))).collect();
        }
        let folder = self.note.parent().map(PathBuf::from);
        let mut found: Vec<(i64, String, String, String)> = Vec::new();
        for path in &vault.files {
            let is_note = crate::core::markdown::is_markdown(path);
            let name = if is_note { path.file_stem() } else { path.file_name() }.map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let facts = vault.notes.get(path);
            let by_alias = facts.and_then(|f| f.aliases.iter().filter_map(|a| score(a, typed).map(|s| (s, a.clone()))).min());
            let by_name = score(&name, typed);
            let Some(best) = by_name.into_iter().chain(by_alias.as_ref().map(|(s, _)| *s)).min() else { continue };
            let near = if path.parent().map(PathBuf::from) == folder { -500 } else { 0 };
            let link = vault.link_path(from, path);
            let relative = path.strip_prefix(&vault.root).ok().and_then(|p| p.parent()).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let (label, new_text) = match by_alias.filter(|(s, _)| by_name.is_none_or(|n| *s < n)) {
                Some((_, alias)) => (format!("{alias} → {name}"), format!("[[{link}|{alias}{close}")),
                None => (name.clone(), format!("[[{link}{close}")),
            };
            found.push((best + near + i64::from(!is_note) * 300, label, relative, new_text));
        }
        found.sort();
        found.truncate(40);
        let mut items: Vec<CompletionItem> = found.into_iter().enumerate().map(|(i, (_, label, detail, text))| item(label, (!detail.is_empty()).then_some(detail), format!("{i:05}"), range, text)).collect();
        let exact = typed.trim();
        if !exact.is_empty() && vault.resolve(exact, from).is_none() && !exact.contains('|') {
            items.push(item(format!("{} „{exact}“", tr("Neue Notiz")), None, "99999".into(), range, format!("[[{exact}{close}")));
        }
        items
    }

    fn slash_items(&self, typed: &str, range: Range) -> Vec<CompletionItem> {
        let commands: [(&str, &str); 12] = [
            ("Überschrift 1", "# "),
            ("Überschrift 2", "## "),
            ("Überschrift 3", "### "),
            ("Aufgabe", "- [ ] "),
            ("Liste", "- "),
            ("Nummerierte Liste", "1. "),
            ("Zitat", "> "),
            ("Callout", "> [!note] "),
            ("Code", "```\n\n```"),
            ("Tabelle", "|  |  |\n|--|--|\n|  |  |\n"),
            ("Trennlinie", "---\n"),
            ("Formel", "$$\n\n$$"),
        ];
        commands.iter().enumerate().filter(|(_, (name, _))| score(&tr(name), typed).is_some()).map(|(i, (name, text))| item(tr(name), None, format!("{i:05}"), range, text.to_string())).collect()
    }
}

impl CompletionProvider for NoteCompletion {
    fn completions(&self, text: &Rope, offset: usize, _: CompletionContext, _: &mut Window, _: &mut App) -> Task<Result<CompletionResponse>> {
        let all = text.to_string();
        let offset = offset.min(all.len());
        let line_start = all[..offset].rfind('\n').map_or(0, |i| i + 1);
        let line_end = all[offset..].find('\n').map_or(all.len(), |i| offset + i);
        let line = &all[line_start..line_end];
        let line_number = all[..line_start].bytes().filter(|b| *b == b'\n').count() as u32;
        let column = offset - line_start;
        let caret = line[..column].encode_utf16().count() as u32;
        let items = match context_at(line, column) {
            Some(Context_::Link { typed, start }) => {
                let start_utf16: u32 = line.chars().take(start).map(|c| c.len_utf16() as u32).sum();
                let end = if line[column..].starts_with("]]") { caret + 2 } else { caret };
                let range = Range { start: Position { line: line_number, character: start_utf16 }, end: Position { line: line_number, character: end } };
                self.link_items(&typed, range)
            }
            Some(Context_::Slash { typed, start }) => {
                let start_utf16: u32 = line.chars().take(start).map(|c| c.len_utf16() as u32).sum();
                let range = Range { start: Position { line: line_number, character: start_utf16 }, end: Position { line: line_number, character: caret } };
                self.slash_items(&typed, range)
            }
            None => Vec::new(),
        };
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        new_text.chars().count() == 1 && new_text.chars().all(|c| c.is_alphanumeric() || matches!(c, '[' | '/' | '#' | '^' | ' ' | '-' | '_'))
    }
}

/// The provider a note's editor uses.
pub fn provider(note: PathBuf) -> Rc<dyn CompletionProvider> {
    Rc::new(NoteCompletion { note })
}

#[cfg(test)]
mod tests {
    use super::{Context_, context_at, score};

    #[test]
    fn knows_where_the_caret_is() {
        assert_eq!(context_at("text [[No", 9), Some(Context_::Link { typed: "No".into(), start: 5 }));
        assert_eq!(context_at("text [[No]] x", 13), None);
        assert_eq!(context_at("[[ä#H", 6), Some(Context_::Link { typed: "ä#H".into(), start: 0 }));
        assert_eq!(context_at("  /auf", 6), Some(Context_::Slash { typed: "auf".into(), start: 2 }));
        assert_eq!(context_at("a/b", 3), None);
        assert_eq!(context_at("/a b", 4), None);
        assert_eq!(context_at("normal", 3), None);
    }

    #[test]
    fn better_matches_come_first() {
        assert!(score("Radar", "rad").unwrap() < score("Mein Radar", "rad").unwrap());
        assert!(score("Mein Radar", "rad").unwrap() < score("Rahmen dar", "rdr").unwrap());
        assert_eq!(score("Radar", "xyz"), None);
    }
}
