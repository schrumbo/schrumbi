use std::cell::Cell;
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
    /// `#` with the tag typed so far, and where the `#` starts in the line.
    Tag { typed: String, start: usize },
    /// `/` at the start of a line.
    Slash { typed: String, start: usize },
    /// `\` with the command typed so far inside a formula, and where the `\` starts in the line.
    Command { typed: String, start: usize },
}

fn context_at(line: &str, column: usize) -> Option<Context_> {
    let before = line.get(..column)?;
    if let Some(open) = before.rfind("[[")
        && !before[open..].contains("]]")
    {
        let start = before[..open].chars().count();
        return Some(Context_::Link { typed: before[open + 2..].to_string(), start });
    }
    if let Some(slash) = before.rfind('\\')
        && before[slash + 1..].chars().all(|c| c.is_ascii_alphabetic())
        && (slash == 0 || !before[..slash].ends_with('\\'))
    {
        return Some(Context_::Command { typed: before[slash + 1..].to_string(), start: before[..slash].chars().count() });
    }
    let word_start = before.rfind(char::is_whitespace).map_or(0, |i| i + before[i..].chars().next().map_or(1, char::len_utf8));
    if let Some(rest) = before[word_start..].strip_prefix('#')
        && (word_start > 0 || !rest.is_empty())
        && rest.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
    {
        return Some(Context_::Tag { typed: rest.to_string(), start: before[..word_start].chars().count() });
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

struct SlashCommand {
    name: &'static str,
    text: &'static str,
    mark: &'static str,
    icon: &'static str,
    action: Option<&'static str>,
}

const fn slash(name: &'static str, text: &'static str, mark: &'static str, icon: &'static str, action: Option<&'static str>) -> SlashCommand {
    SlashCommand { name, text, mark, icon, action }
}

impl SlashCommand {
    fn inline(&self) -> bool {
        matches!(self.action, Some("embedNote" | "insertFootnote" | "insertCard" | "insertCloze"))
    }
}

const SLASH: [SlashCommand; 17] = [
    slash("Überschrift 1", "# ", "#", "heading-1", Some("Heading1")),
    slash("Überschrift 2", "## ", "##", "heading-2", Some("Heading2")),
    slash("Überschrift 3", "### ", "###", "heading-3", Some("Heading3")),
    slash("Überschrift 4", "#### ", "####", "heading", Some("heading4")),
    slash("Aufgabe", "- [ ] ", "- [ ]", "square-check", Some("ToggleTask")),
    slash("Liste", "- ", "-", "list", Some("BulletList")),
    slash("Nummerierte Liste", "1. ", "1.", "list-ordered", Some("NumberedList")),
    slash("Tabelle", "| $0 |  |  |\n|--|--|--|\n|  |  |  |\n", "|", "table", Some("insertTable")),
    slash("Formelblock", "$$\n$0\n$$", "$$", "sigma", Some("MathBlock")),
    slash("Zitat", "> ", ">", "quote", Some("Quote")),
    slash("Hinweisbox", "> [!note] ", "> [!note]", "info", Some("insertCallout")),
    slash("Notiz einbetten", "![[", "![[", "file-text", Some("embedNote")),
    slash("Codeblock", "```\n$0\n```", "```", "code", Some("insertCodeBlock")),
    slash("Trennlinie", "---\n", "---", "minus", Some("insertDivider")),
    slash("Fußnote", "[^1]", "[^1]", "sticky-note", Some("insertFootnote")),
    slash("Karteikarte", " :: ", "::", "layers", Some("insertCard")),
    slash("Lückentext", "{{c1::$0}}", "{{c1::}}", "square-dashed", Some("insertCloze")),
];

/// What the field being written in holds, when the text alone cannot tell.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Field {
    #[default]
    Text,
    Formula,
    Cell,
}

/// Suggestions while writing a link or a slash command.
pub struct NoteCompletion {
    pub note: PathBuf,
    pub field: Rc<Cell<Field>>,
}

impl NoteCompletion {
    fn link_items(&self, typed: &str, range: Range, open: &[PathBuf]) -> Vec<CompletionItem> {
        let vault = index::current();
        let close = "]]";
        let from = Some(self.note.as_path());
        let across = typed.strip_prefix("##").map(|q| (q, true)).or_else(|| typed.strip_prefix("^^").map(|q| (q, false)));
        if let Some((query, headings)) = across {
            let mut found: Vec<(i64, String, String, String)> = Vec::new();
            for (path, facts) in &vault.notes {
                let link = vault.link_path(from, path);
                let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                if headings {
                    found.extend(facts.headings.iter().filter_map(|(_, h, _)| score(h, query).map(|s| (s, h.clone(), name.clone(), format!("[[{link}#{h}{close}")))));
                } else {
                    found.extend(facts.blocks.iter().filter_map(|(id, _)| score(id, query).map(|s| (s, format!("^{id}"), name.clone(), format!("[[{link}#^{id}{close}")))));
                }
            }
            found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            found.truncate(40);
            return found.into_iter().enumerate().map(|(i, (_, label, note, text))| item(label, Some(note), format!("{i:05}"), range, text)).collect();
        }
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
        let mut found: Vec<(i64, String, Option<String>, &PathBuf)> = Vec::new();
        for path in &vault.files {
            let is_note = crate::core::markdown::is_markdown(path);
            let name = if is_note { path.file_stem() } else { path.file_name() }.map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let facts = vault.notes.get(path);
            let by_alias = facts.and_then(|f| f.aliases.iter().filter_map(|a| score(a, typed).map(|s| (s, a.clone()))).min());
            let by_name = score(&name, typed);
            let Some(best) = by_name.into_iter().chain(by_alias.as_ref().map(|(s, _)| *s)).min() else { continue };
            let top = |p: &std::path::Path| p.strip_prefix(&vault.root).ok().and_then(|r| r.components().next()).map(|c| c.as_os_str().to_os_string());
            let near = if path.parent().map(PathBuf::from) == folder {
                -500
            } else if open.contains(path) {
                -400
            } else if top(path) == top(&self.note) {
                -200
            } else {
                0
            };
            let alias = by_alias.filter(|(s, _)| by_name.is_none_or(|n| *s < n)).map(|(_, a)| a);
            found.push((best + near + i64::from(!is_note) * 300, name, alias, path));
        }
        found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        found.truncate(40);
        let mut items: Vec<CompletionItem> = found
            .into_iter()
            .enumerate()
            .map(|(i, (_, name, alias, path))| {
                let link = vault.link_path(from, path);
                let detail = path.strip_prefix(&vault.root).ok().and_then(|p| p.parent()).map(|p| p.to_string_lossy().replace('\\', "/")).filter(|d| !d.is_empty());
                let (label, text) = match alias {
                    Some(alias) => (format!("{alias} → {name}"), format!("[[{link}|{alias}{close}")),
                    None => (name, format!("[[{link}{close}")),
                };
                item(label, detail, format!("{i:05}"), range, text)
            })
            .collect();
        let exact = typed.trim();
        if !exact.is_empty() && vault.resolve(exact, from).is_none() && !exact.contains('|') {
            items.push(item(format!("{} „{exact}“", tr("Neue Notiz")), None, "99999".into(), range, format!("[[{exact}{close}")));
        }
        items
    }

    fn tag_items(&self, typed: &str, range: Range) -> Vec<CompletionItem> {
        let mut found: Vec<(i64, String, usize)> = index::current().tag_tree().into_iter().filter_map(|(tag, count)| score(&tag, typed).map(|s| (s - count as i64, tag, count))).collect();
        found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        found.truncate(30);
        found.into_iter().enumerate().map(|(i, (_, tag, count))| item(format!("#{tag}"), Some(count.to_string()), format!("{i:05}"), range, format!("#{tag}"))).collect()
    }

    fn slash_items(&self, typed: &str, range: Range, keys: impl Fn(&str) -> Option<String>) -> Vec<CompletionItem> {
        let cell = self.field.get() == Field::Cell;
        let mut found: Vec<(i64, usize, String, &SlashCommand)> = SLASH.iter().enumerate().filter(|(_, c)| !cell || c.inline()).filter_map(|(i, c)| score(&tr(c.name), typed).map(|s| (s, i, tr(c.name), c))).collect();
        found.sort_by_key(|f| (f.0, f.1));
        found
            .into_iter()
            .enumerate()
            .map(|(i, (_, _, name, c))| CompletionItem { data: Some(serde_json::json!({ "icon": format!("icons/{}.svg", c.icon) })), insert_text_format: Some(lsp_types::InsertTextFormat::SNIPPET), ..item(name, c.action.and_then(&keys).or_else(|| Some(c.mark.to_string())), format!("{i:05}"), range, c.text.to_string()) })
            .collect()
    }

    fn command_items(&self, typed: &str, range: Range) -> Vec<CompletionItem> {
        let mut found: Vec<(i64, usize, &str, &str)> = LATEX.iter().enumerate().filter_map(|(i, (name, text))| score(name, typed).map(|s| (s, i, *name, *text))).collect();
        found.sort_by_key(|f| (f.0, f.1));
        found.truncate(30);
        found.into_iter().enumerate().map(|(i, (_, _, name, text))| item(format!("\\{name}"), None, format!("{i:05}"), range, text.to_string())).collect()
    }
}

const LATEX: [(&str, &str); 40] = [
    ("frac", "\\frac{}{}"),
    ("sqrt", "\\sqrt{}"),
    ("sum", "\\sum_{}^{}"),
    ("int", "\\int_{}^{}"),
    ("lim", "\\lim_{}"),
    ("prod", "\\prod_{}^{}"),
    ("cdot", "\\cdot"),
    ("times", "\\times"),
    ("infty", "\\infty"),
    ("partial", "\\partial"),
    ("nabla", "\\nabla"),
    ("leq", "\\leq"),
    ("geq", "\\geq"),
    ("neq", "\\neq"),
    ("approx", "\\approx"),
    ("in", "\\in"),
    ("subseteq", "\\subseteq"),
    ("to", "\\to"),
    ("Rightarrow", "\\Rightarrow"),
    ("Leftrightarrow", "\\Leftrightarrow"),
    ("forall", "\\forall"),
    ("exists", "\\exists"),
    ("mathbb", "\\mathbb{}"),
    ("mathcal", "\\mathcal{}"),
    ("mathrm", "\\mathrm{}"),
    ("text", "\\text{}"),
    ("vec", "\\vec{}"),
    ("overline", "\\overline{}"),
    ("binom", "\\binom{}{}"),
    ("left", "\\left( \\right)"),
    ("begin", "\\begin{pmatrix}\n\n\\end{pmatrix}"),
    ("alpha", "\\alpha"),
    ("beta", "\\beta"),
    ("gamma", "\\gamma"),
    ("delta", "\\delta"),
    ("epsilon", "\\varepsilon"),
    ("lambda", "\\lambda"),
    ("mu", "\\mu"),
    ("pi", "\\pi"),
    ("sigma", "\\sigma"),
];

/// Whether `offset` sits inside a formula: between `$` signs on its line or within a `$$` block.
fn in_formula(all: &str, line_start: usize, offset: usize) -> bool {
    let line = &all[line_start..offset];
    let blocks = all[..line_start].split('\n').filter(|l| l.trim() == "$$").count();
    blocks % 2 == 1 || line.matches('$').count() % 2 == 1
}

/// Colours the letters of a suggestion that what was typed matched: the run where it occurs, else each letter in order; nothing while nothing is typed.
fn highlight_typed(query: &str, item: &CompletionItem) -> Vec<std::ops::Range<usize>> {
    let typed = query.trim_start_matches(['[', '/', '#', '!', '\\']).to_lowercase();
    if typed.is_empty() {
        return Vec::new();
    }
    let label = item.label.to_lowercase();
    if let Some(at) = label.find(&typed).filter(|_| label.len() == item.label.len()) {
        return vec![at..at + typed.len()];
    }
    let mut wanted = typed.chars().peekable();
    let mut ranges = Vec::new();
    for (at, c) in item.label.char_indices() {
        let Some(next) = wanted.peek() else { break };
        if c.to_lowercase().eq(std::iter::once(*next)) {
            ranges.push(at..at + c.len_utf8());
            wanted.next();
        }
    }
    if wanted.peek().is_some() { Vec::new() } else { ranges }
}

/// Sets the suggestions and their highlighting on a note's editor.
pub fn attach(state: &mut gpui_kit::component::input::EditorState, note: PathBuf, field: Rc<Cell<Field>>) {
    let lsp = state.lsp_mut();
    lsp.completion_provider = Some(provider(note, field));
    lsp.completion_menu.max_width = px(super::theme::suggestions::WIDTH);
    lsp.set_completion_highlighter(Rc::new(highlight_typed));
}

impl CompletionProvider for NoteCompletion {
    fn completions(&self, text: &Rope, offset: usize, _: CompletionContext, _: &mut Window, cx: &mut App) -> Task<Result<CompletionResponse>> {
        let open = super::state::read(cx, |l| l.open_documents.clone());
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
                self.link_items(&typed, range, &open)
            }
            Some(Context_::Tag { typed, start }) => {
                let start_utf16: u32 = line.chars().take(start).map(|c| c.len_utf16() as u32).sum();
                let range = Range { start: Position { line: line_number, character: start_utf16 }, end: Position { line: line_number, character: caret } };
                self.tag_items(&typed, range)
            }
            Some(Context_::Command { typed, start }) if self.field.get() == Field::Formula || in_formula(&all, line_start, offset) => {
                let start_utf16: u32 = line.chars().take(start).map(|c| c.len_utf16() as u32).sum();
                let range = Range { start: Position { line: line_number, character: start_utf16 }, end: Position { line: line_number, character: caret } };
                self.command_items(&typed, range)
            }
            Some(Context_::Command { .. }) => Vec::new(),
            Some(Context_::Slash { typed, start }) => {
                let start_utf16: u32 = line.chars().take(start).map(|c| c.len_utf16() as u32).sum();
                let range = Range { start: Position { line: line_number, character: start_utf16 }, end: Position { line: line_number, character: caret } };
                self.slash_items(&typed, range, |action| super::help::key_text(action, cx))
            }
            None => Vec::new(),
        };
        let crlf = all.contains("\r\n") || super::state::read(cx, |l| l.buffers.get(&self.note).is_some_and(|b| b.text.contains("\r\n")));
        let items = if crlf {
            items
                .into_iter()
                .map(|mut item| {
                    if let Some(CompletionTextEdit::Edit(edit)) = &mut item.text_edit
                        && !edit.new_text.contains('\r')
                    {
                        edit.new_text = edit.new_text.replace('\n', "\r\n");
                    }
                    item
                })
                .collect()
        } else {
            items
        };
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        new_text.chars().count() == 1 && new_text.chars().all(|c| c.is_alphanumeric() || matches!(c, '[' | '/' | '#' | '^' | ' ' | '-' | '_' | '\\'))
    }
}

/// The provider a note's editor uses.
pub fn provider(note: PathBuf, field: Rc<Cell<Field>>) -> Rc<dyn CompletionProvider> {
    Rc::new(NoteCompletion { note, field })
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
        assert_eq!(context_at("$\\fr", 4), Some(Context_::Command { typed: "fr".into(), start: 1 }));
        assert_eq!(context_at("$\\\\", 3), None);
    }

    #[test]
    fn formulas_are_known_by_their_dollars() {
        assert!(super::in_formula("a $x", 0, 4));
        assert!(!super::in_formula("a $x$ b", 0, 7));
        assert!(super::in_formula("$$\nx", 3, 4));
        assert!(!super::in_formula("$$\nx\n$$\ny", 8, 9));
    }

    #[test]
    fn slash_commands_that_start_with_the_letters_come_first() {
        let menu = super::NoteCompletion { note: "a.md".into(), field: std::rc::Rc::default() };
        let labels: Vec<String> = menu.slash_items("t", lsp_types::Range::default(), |_| None).into_iter().map(|i| i.label).collect();
        assert_eq!(labels.first().map(String::as_str), Some("Tabelle"));
    }

    #[test]
    fn a_table_cell_offers_only_what_fits_in_a_cell() {
        let menu = super::NoteCompletion { note: "a.md".into(), field: std::rc::Rc::new(std::cell::Cell::new(super::Field::Cell)) };
        let labels: Vec<String> = menu.slash_items("", lsp_types::Range::default(), |_| None).into_iter().map(|i| i.label).collect();
        assert_eq!(labels, ["Notiz einbetten", "Fußnote", "Karteikarte", "Lückentext"]);
    }

    #[test]
    fn only_matched_letters_are_coloured() {
        let named = |label: &str| lsp_types::CompletionItem { label: label.into(), ..Default::default() };
        assert_eq!(super::highlight_typed("[[", &named("Radar")), Vec::<std::ops::Range<usize>>::new());
        assert_eq!(super::highlight_typed("/ab", &named("Tabelle")), vec![1..3]);
        assert_eq!(super::highlight_typed("/üs", &named("Überschrift 1")), vec![0..2, 5..6]);
        assert_eq!(super::highlight_typed("/xq", &named("Tabelle")), Vec::<std::ops::Range<usize>>::new());
    }

    #[test]
    fn better_matches_come_first() {
        assert!(score("Radar", "rad").unwrap() < score("Mein Radar", "rad").unwrap());
        assert!(score("Mein Radar", "rad").unwrap() < score("Rahmen dar", "rdr").unwrap());
        assert_eq!(score("Radar", "xyz"), None);
    }
}
