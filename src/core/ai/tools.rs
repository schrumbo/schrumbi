use std::cell::Cell;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

use chrono::{Datelike, Local, NaiveDate, NaiveTime, TimeZone};
use serde_json::{Value, json};

use crate::core::cards::{parse, store};
use crate::core::i18n::{Lang, lang, trf};
use crate::core::library::{Library, Route};
use crate::core::model::{Kind, ManualAssignment, Status};
use crate::core::text::fold;

/// What the agent is told about this server; Claude Code's tool search reads exactly this field.
pub const INSTRUCTIONS: &str = "Schrumbi is the user's study planner working on their study folder (notes, PDFs, deadlines, flashcards). \
Use these tools for everything in that folder and never edit its files directly: edits through the tools are undoable in the app and conflict-checked, direct edits are not. \
Call get_context first. Paths are relative to the study folder, with forward slashes. \
Writes ask the user for permission in the app; a denied write is not an error of yours, tell the user what you wanted to do. \
Answer in the language get_context reports.";

const SEARCH_LIMIT: usize = 50;
const NOTE_CHARS: usize = 50_000;
const PDF_CHARS: usize = 8_000;
const PDF_PAGES: usize = 10;
const FOLDER_ENTRIES: usize = 500;
const CARD_LIMIT: usize = 200;

const STALE: &str = "The note changed since you read it (version mismatch). Call read_note again and retry with the new version.";
const NO_ROOT: &str = "No study folder is open in Schrumbi.";
const STALE_FILE: &str = "The file changed since you last read it, or you have not read it yet. Read it again and retry.";
const OUTSIDE: &str = "Paths are relative to the study folder and may not leave it.";

/// One tool as the agent sees it.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    /// Changes the folder or the deadlines; asks the user first.
    pub write: bool,
}

fn obj(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

/// Every tool with its JSON schema.
pub fn definitions() -> Vec<ToolDef> {
    let path = |what: &str| json!({ "type": "string", "description": format!("{what}, relative to the study folder") });
    vec![
        ToolDef { name: "get_context", description: "What the user looks at right now: open note or PDF, module, selected deadline, week, language. Call this first.", schema: obj(json!({}), &[]), write: false },
        ToolDef {
            name: "search",
            description: "Full-text search over notes and PDFs in the study folder; every word is a prefix, all must match. Also finds files by name.",
            schema: obj(json!({ "query": { "type": "string" }, "folder": path("Only below this folder"), "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 } }), &["query"]),
            write: false,
        },
        ToolDef {
            name: "read_note",
            description: "The text of a note or text file as the user sees it, including unsaved edits, with a version for write_note. Lines count from 1.",
            schema: obj(json!({ "path": path("The file"), "from_line": { "type": "integer", "minimum": 1 }, "to_line": { "type": "integer", "minimum": 1 } }), &["path"]),
            write: false,
        },
        ToolDef {
            name: "read_pdf_text",
            description: "The text of some pages of a PDF, at most 10 pages and 8000 characters per call. Scanned PDFs without a text layer yield nothing.",
            schema: obj(json!({ "path": path("The PDF"), "pages": { "type": "string", "description": "A page or range, counted from 1, e.g. \"3\" or \"3-5\"" } }), &["path", "pages"]),
            write: false,
        },
        ToolDef { name: "list_folder", description: "Files and folders directly inside a folder of the study folder.", schema: obj(json!({ "path": path("The folder; empty for the study folder itself") }), &[]), write: false },
        ToolDef {
            name: "list_deadlines",
            description: "Deadlines, tasks and exams of the current semester with their status.",
            schema: obj(
                json!({ "days": { "type": "integer", "description": "Only those due within this many days from now; omit for the whole semester" }, "module": { "type": "string", "description": "Module code, name or id" }, "include_done": { "type": "boolean", "default": false } }),
                &[],
            ),
            write: false,
        },
        ToolDef { name: "list_modules", description: "The modules of the current semester with folders and weekly sessions.", schema: obj(json!({}), &[]), write: false },
        ToolDef { name: "get_week", description: "The lecture week around a date: sessions and deadlines per day.", schema: obj(json!({ "date": { "type": "string", "description": "YYYY-MM-DD; today when omitted" } }), &[]), write: false },
        ToolDef {
            name: "list_cards",
            description: "Flashcards, all or those of one note, with how many are due. Cards are never rated by tools; the user reviews them.",
            schema: obj(json!({ "note": path("Only cards of this note"), "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 100 } }), &[]),
            write: false,
        },
        ToolDef {
            name: "write_note",
            description: "Changes a note. Needs the version from read_note; a stale version is refused and nothing is written. Modes: replace (whole text), append (at the end), insert_after_heading (at the end of that heading's section), replace_lines (from_line..to_line, counted from 1).",
            schema: obj(
                json!({
                    "path": path("The note"),
                    "version": { "type": "string" },
                    "mode": { "type": "string", "enum": ["replace", "append", "insert_after_heading", "replace_lines"] },
                    "text": { "type": "string" },
                    "heading": { "type": "string", "description": "For insert_after_heading: the heading text without #" },
                    "from_line": { "type": "integer", "minimum": 1 },
                    "to_line": { "type": "integer", "minimum": 1 }
                }),
                &["path", "version", "mode", "text"],
            ),
            write: true,
        },
        ToolDef {
            name: "create_note",
            description: "Creates a Markdown note. The file name follows the title; the folder defaults to the module's notes folder or the study folder.",
            schema: obj(json!({ "title": { "type": "string" }, "folder": path("Where to put it"), "text": { "type": "string", "description": "Body below the title heading" } }), &["title"]),
            write: true,
        },
        ToolDef {
            name: "add_cards",
            description: "Appends flashcards to a note as `Question :: Answer` lines. Cards whose question already exists in the note are skipped.",
            schema: obj(
                json!({ "note": path("The note the cards go to"), "cards": { "type": "array", "items": obj(json!({ "front": { "type": "string" }, "back": { "type": "string" }, "both": { "type": "boolean", "description": "Ask in both directions", "default": false } }), &["front", "back"]) } }),
                &["note", "cards"],
            ),
            write: true,
        },
        ToolDef {
            name: "add_deadline",
            description: "Adds a deadline, task or exam to a module.",
            schema: obj(
                json!({ "title": { "type": "string" }, "module": { "type": "string", "description": "Module code, name or id" }, "due": { "type": "string", "description": "YYYY-MM-DD or YYYY-MM-DDTHH:MM, local time; a date alone means 23:59" }, "kind": { "type": "string", "enum": ["submission", "task", "exam"], "default": "task" }, "optional": { "type": "boolean", "default": false } }),
                &["title", "module", "due"],
            ),
            write: true,
        },
        ToolDef {
            name: "set_status",
            description: "Marks a deadline as done, uploaded, skipped or open again; auto returns to what the folder says.",
            schema: obj(json!({ "id": { "type": "string", "description": "From list_deadlines" }, "status": { "type": "string", "enum": ["done", "uploaded", "skipped", "open", "in_progress", "auto"] } }), &["id", "status"]),
            write: true,
        },
        ToolDef {
            name: "file_document",
            description: "Moves a file inside the study folder, e.g. from the inbox into a module folder, optionally linking it to a deadline. Never overwrites.",
            schema: obj(json!({ "path": path("The file"), "folder": path("The target folder"), "name": { "type": "string", "description": "New file name with extension; keeps the name when omitted" }, "deadline": { "type": "string", "description": "Id from list_deadlines to link the file to" } }), &["path", "folder"]),
            write: true,
        },
        ToolDef { name: "open", description: "Shows a file in Schrumbi, optionally jumping to a text in a PDF. Changes nothing.", schema: obj(json!({ "path": path("The file"), "find": { "type": "string" } }), &["path"]), write: false },
    ]
}

/// Whether the tool changes something and so asks the user first.
pub fn is_write(name: &str) -> bool {
    definitions().iter().any(|t| t.name == name && t.write)
}

/// A tool's result and, for writes, what changed in the user's words.
pub struct Outcome {
    pub value: Value,
    pub changed: Option<String>,
}

/// What a write would do, shown before the user allows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub title: String,
    pub lines: Vec<String>,
}

/// A stable fingerprint of a note's text, handed out by `read_note` and checked by `write_note`.
pub fn version_of(text: &str) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn need_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    arg_str(args, key).ok_or_else(|| format!("`{key}` is required"))
}

fn arg_usize(args: &Value, key: &str) -> Option<usize> {
    args.get(key).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn settled(p: &Path) -> PathBuf {
    dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The nearest existing ancestor, resolved, so a symlink cannot lead out of the folder.
fn anchored(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = p.to_path_buf();
    while !cur.exists() {
        let Some(name) = cur.file_name().map(std::ffi::OsStr::to_os_string) else { return p.to_path_buf() };
        rest.push(name);
        if !cur.pop() {
            return p.to_path_buf();
        }
    }
    let mut out = settled(&cur);
    for name in rest.into_iter().rev() {
        out.push(name);
    }
    out
}

/// A path the agent named, inside the study folder; `rel` may be empty for the folder itself.
fn resolve(lib: &Library, rel: &str) -> Result<PathBuf, String> {
    let root = lib.root.as_ref().ok_or(NO_ROOT)?;
    let rel = rel.trim();
    if rel.starts_with("//") || rel.starts_with("\\\\") {
        return Err(OUTSIDE.to_string());
    }
    let rel = rel.trim_start_matches(['/', '\\']);
    let candidate = Path::new(rel);
    if candidate.is_absolute() || candidate.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(OUTSIDE.to_string());
    }
    let full = root.join(candidate);
    let settled_root = settled(root);
    if !anchored(&full).starts_with(&settled_root) {
        return Err(OUTSIDE.to_string());
    }
    Ok(full)
}

fn rel(lib: &Library, path: &Path) -> String {
    lib.relative_path(path)
}

fn is_text(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["md", "txt", "markdown", "csv", "json", "yaml", "yml", "tex", "bib", "html"].contains(&e.to_lowercase().as_str()))
}

/// The text as the user sees it: the open buffer when there is one, else the file.
fn current_text(lib: &Library, path: &Path) -> Result<String, String> {
    if let Some(b) = lib.buffers.get(&lib.buffer_key(path)) {
        if b.undecodable {
            return Err("The file is not UTF-8 text.".into());
        }
        return Ok(b.text.clone());
    }
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|_| "The file is not UTF-8 text.".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!("No such file: {}", rel(lib, path))),
        Err(e) => Err(e.to_string()),
    }
}

fn cut(text: &str, chars: usize) -> (String, bool) {
    let mut out: String = text.chars().take(chars).collect();
    let truncated = out.len() < text.len();
    if truncated {
        out.push('…');
    }
    (out, truncated)
}

fn module_json(m: &crate::core::model::Module) -> Value {
    json!({ "id": m.id, "code": m.code, "name": m.name, "folder": m.folder })
}

fn deadline_json(lib: &Library, a: &crate::core::model::Assignment) -> Value {
    let module = lib.module(a);
    json!({
        "id": a.id,
        "title": a.title,
        "module": module.map(|m| m.code.clone()),
        "kind": kind_name(a.kind),
        "due": a.due.format("%Y-%m-%dT%H:%M").to_string(),
        "status": status_name(lib.status(a)),
        "optional": a.optional,
        "week": a.week,
        "folder": rel(lib, &a.folder),
    })
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Submission => "submission",
        Kind::Task => "task",
        Kind::Exam => "exam",
    }
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Open => "open",
        Status::InProgress => "in_progress",
        Status::Ready => "ready",
        Status::Uploaded => "uploaded",
        Status::Done => "done",
        Status::Skipped => "skipped",
        Status::Overdue => "overdue",
    }
}

fn find_module<'a>(lib: &'a Library, query: &str) -> Result<&'a crate::core::model::Module, String> {
    let q = fold(query);
    let modules = lib.semester_modules();
    modules
        .iter()
        .find(|m| m.id == query || fold(&m.code) == q)
        .or_else(|| modules.iter().find(|m| fold(&m.name) == q))
        .or_else(|| modules.iter().find(|m| fold(&m.name).contains(&q) || fold(&m.code).contains(&q)))
        .copied()
        .ok_or_else(|| format!("No module matches `{query}`; list_modules shows them."))
}

fn parse_due(text: &str) -> Result<chrono::DateTime<Local>, String> {
    let text = text.trim();
    let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M"))
        .or_else(|_| NaiveDate::parse_from_str(text, "%Y-%m-%d").map(|d| d.and_time(NaiveTime::from_hms_opt(23, 59, 0).unwrap_or_default())))
        .map_err(|_| "`due` must be YYYY-MM-DD or YYYY-MM-DDTHH:MM".to_string())?;
    Local.from_local_datetime(&naive).single().ok_or_else(|| "`due` is not a valid local time".to_string())
}

fn section_end(lines: &[&str], heading: &str) -> Option<usize> {
    let wanted = fold(heading.trim().trim_start_matches('#').trim());
    let level_of = |line: &str| line.chars().take_while(|c| *c == '#').count();
    let start = lines.iter().position(|l| {
        let level = level_of(l);
        level > 0 && fold(l.trim_start_matches('#').trim()) == wanted
    })?;
    let level = level_of(lines[start]);
    let mut end = lines.len();
    for (i, l) in lines.iter().enumerate().skip(start + 1) {
        let lv = level_of(l);
        if lv > 0 && lv <= level && l.chars().nth(lv) == Some(' ') {
            end = i;
            break;
        }
    }
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    Some(end)
}

fn with_newline(mut s: String) -> String {
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// The text `write_note` would produce.
fn planned_text(before: &str, args: &Value) -> Result<String, String> {
    let text = args.get("text").and_then(Value::as_str).ok_or("`text` is required")?;
    match need_str(args, "mode")? {
        "replace" => Ok(text.to_string()),
        "append" => {
            let mut out = with_newline(before.to_string());
            if !out.is_empty() && !out.ends_with("\n\n") && !text.starts_with('\n') {
                out.push('\n');
            }
            out.push_str(text);
            Ok(with_newline(out))
        }
        "insert_after_heading" => {
            let heading = need_str(args, "heading")?;
            let lines: Vec<&str> = before.split('\n').collect();
            let end = section_end(&lines, heading).ok_or_else(|| format!("No heading `{heading}` in the note."))?;
            let mut out: Vec<String> = lines[..end].iter().map(|l| (*l).to_string()).collect();
            out.push(String::new());
            out.extend(text.trim_end_matches('\n').split('\n').map(String::from));
            if lines.get(end).is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.extend(lines[end..].iter().map(|l| (*l).to_string()));
            Ok(out.join("\n"))
        }
        "replace_lines" => {
            let from = arg_usize(args, "from_line").ok_or("`from_line` is required for replace_lines")?;
            let to = arg_usize(args, "to_line").unwrap_or(from);
            let lines: Vec<&str> = before.split('\n').collect();
            if from == 0 || from > to || to > lines.len() {
                return Err(format!("Lines {from}-{to} are outside the note, which has {} lines.", lines.len()));
            }
            let mut out: Vec<String> = lines[..from - 1].iter().map(|l| (*l).to_string()).collect();
            out.extend(text.trim_end_matches('\n').split('\n').map(String::from));
            out.extend(lines[to..].iter().map(|l| (*l).to_string()));
            Ok(out.join("\n"))
        }
        other => Err(format!("Unknown mode `{other}`")),
    }
}

fn diff_lines(old: &str, new: &str) -> (usize, usize, Vec<String>) {
    let diff = similar::TextDiff::from_lines(old, new);
    let (mut added, mut removed, mut shown) = (0, 0, Vec::new());
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            similar::ChangeTag::Insert => {
                added += 1;
                "+ "
            }
            similar::ChangeTag::Delete => {
                removed += 1;
                "− "
            }
            similar::ChangeTag::Equal => continue,
        };
        if shown.len() < 8 {
            shown.push(format!("{sign}{}", change.value().trim_end_matches('\n')));
        }
    }
    (added, removed, shown)
}

fn cards_from(args: &Value) -> Result<Vec<(String, String, bool)>, String> {
    let items = args.get("cards").and_then(Value::as_array).ok_or("`cards` is required")?;
    let mut out = Vec::new();
    for item in items {
        let front = arg_str(item, "front").ok_or("every card needs a `front`")?;
        let back = arg_str(item, "back").ok_or("every card needs a `back`")?;
        out.push((front.to_string(), back.to_string(), item.get("both").and_then(Value::as_bool).unwrap_or(false)));
    }
    if out.is_empty() {
        return Err("`cards` is empty".into());
    }
    Ok(out)
}

/// The cards that are new to the note, as lines to append.
fn new_card_lines(before: &str, cards: &[(String, String, bool)]) -> (Vec<String>, usize) {
    let known: HashSet<String> = parse::cards_in(before).into_iter().map(|c| fold(&c.front)).collect();
    let mut seen = HashSet::new();
    let mut lines = Vec::new();
    let mut duplicates = 0;
    for (front, back, both) in cards {
        let key = fold(front);
        if known.contains(&key) || !seen.insert(key) {
            duplicates += 1;
            continue;
        }
        lines.push(parse::write_card(front, back, *both));
    }
    (lines, duplicates)
}

fn appended(before: &str, lines: &[String]) -> String {
    let mut out = with_newline(before.to_string());
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(&lines.join("\n\n"));
    out.push('\n');
    out
}

/// A path the agent names in full, checked to lie inside the study folder.
pub fn inside_root(lib: &Library, path: &Path) -> Result<PathBuf, String> {
    let root = lib.root.as_ref().ok_or(NO_ROOT)?;
    if !path.is_absolute() {
        return resolve(lib, &path.to_string_lossy());
    }
    if !super::acp::inside(root, path) {
        return Err(OUTSIDE.to_string());
    }
    Ok(path.to_path_buf())
}

/// A text file as the user sees it, unsaved edits included; `line` counts from 1.
pub fn read_file(lib: &Library, path: &Path, line: Option<u32>, limit: Option<u32>) -> Result<String, String> {
    let path = inside_root(lib, path)?;
    if !is_text(&path) {
        return Err("Only text files can be read this way; use the schrumbi tools (read_pdf_text for PDFs).".into());
    }
    let text = current_text(lib, &path)?;
    if line.is_none() && limit.is_none() {
        return Ok(text);
    }
    let from = line.map_or(0, |l| usize::try_from(l).unwrap_or(usize::MAX).saturating_sub(1));
    let count = limit.map_or(usize::MAX, |l| usize::try_from(l).unwrap_or(usize::MAX));
    Ok(text.split_inclusive('\n').skip(from).take(count).collect())
}

/// The text of a file now, for showing a change; `None` when it is no text file or unreadable.
pub fn text_now(lib: &Library, path: &Path) -> Option<String> {
    is_text(path).then(|| current_text(lib, path).ok()).flatten()
}

/// The file a write tool changes, before it runs.
pub fn target(lib: &Library, name: &str, args: &Value) -> Option<PathBuf> {
    match name {
        "write_note" => resolve(lib, arg_str(args, "path")?).ok(),
        "add_cards" => resolve(lib, arg_str(args, "note")?).ok(),
        _ => None,
    }
}

/// What writing `content` over a whole file would change, for the permission card.
pub fn write_preview(lib: &Library, path: &Path, content: &str) -> Result<Preview, String> {
    let path = inside_root(lib, path)?;
    if !is_text(&path) {
        return Err("Only text files can be written this way; use the schrumbi tools.".into());
    }
    let before = if path.exists() { current_text(lib, &path)? } else { String::new() };
    let (added, removed, lines) = diff_lines(&before, content);
    Ok(Preview { title: trf("%@ ändern (+%lld −%lld Zeilen)", &[&name_of(&path), &added, &removed]), lines })
}

/// Writes a whole text file through the library: one undoable step, conflict-checked, never outside the folder.
/// An existing file is only replaced when `seen` is the version of the text the agent read, so typing since then is never lost.
pub fn write_file(lib: &mut Library, path: &Path, content: &str, seen: Option<&str>) -> Result<Outcome, String> {
    if lib.busy() {
        return Err("Schrumbi is syncing or moving files right now. Try again in a moment.".into());
    }
    let path = inside_root(lib, path)?;
    if !is_text(&path) {
        return Err("Only text files can be written this way; use the schrumbi tools.".into());
    }
    let created = (!path.exists()).then_some(());
    let target = if created.is_none() {
        path
    } else {
        let folder = path.parent().filter(|p| p.is_dir()).ok_or("The folder does not exist.")?.to_path_buf();
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "md".into());
        lib.create_note(&stem, &folder, &ext).ok_or_else(|| lib.last_error.take().unwrap_or_else(|| "The file could not be created.".into()))?
    };
    let key = lib.buffer_key(&target);
    if created.is_none() && current_text(lib, &key).map(|t| version_of(&t)).ok().as_deref() != seen {
        return Err(STALE_FILE.into());
    }
    let label = trf("%@ geändert", &[&name_of(&target)]);
    let body = content.to_string();
    if current_text(lib, &key).is_ok_and(|t| t == body) {
        return Ok(Outcome { value: json!({ "path": rel(lib, &target) }), changed: None });
    }
    if !lib.edit_note(&key, "ai", label.clone(), |_| body) {
        return Err(lib.last_error.take().unwrap_or_else(|| "The file could not be written.".into()));
    }
    Ok(Outcome { value: json!({ "path": rel(lib, &target) }), changed: Some(label) })
}

/// What a write tool would do, for the permission card. Reads are never previewed.
pub fn preview(lib: &Library, name: &str, args: &Value) -> Result<Preview, String> {
    match name {
        "write_note" => {
            let path = resolve(lib, need_str(args, "path")?)?;
            let before = current_text(lib, &path)?;
            if need_str(args, "version")? != version_of(&before) {
                return Err(STALE.into());
            }
            let after = planned_text(&before, args)?;
            let (added, removed, lines) = diff_lines(&before, &after);
            Ok(Preview { title: trf("%@ ändern (+%lld −%lld Zeilen)", &[&name_of(&path), &added, &removed]), lines })
        }
        "create_note" => {
            let title = need_str(args, "title")?;
            let folder = create_folder(lib, args)?;
            let lines = args.get("text").and_then(Value::as_str).map(|t| t.lines().take(6).map(String::from).collect()).unwrap_or_default();
            Ok(Preview { title: trf("Notiz „%@“ in %@ anlegen", &[&title, &folder_label(lib, &folder)]), lines })
        }
        "add_cards" => {
            let path = resolve(lib, need_str(args, "note")?)?;
            let before = current_text(lib, &path)?;
            let (lines, _) = new_card_lines(&before, &cards_from(args)?);
            if lines.is_empty() {
                return Err("Every card is already in the note.".into());
            }
            let shown: Vec<String> = lines.iter().take(8).map(|l| l.lines().next().unwrap_or_default().to_string()).collect();
            let title = if lines.len() == 1 { trf("1 Karte in %@ ergänzen", &[&name_of(&path)]) } else { trf("%lld Karten in %@ ergänzen", &[&lines.len(), &name_of(&path)]) };
            Ok(Preview { title, lines: shown })
        }
        "add_deadline" => {
            let title = need_str(args, "title")?;
            let module = find_module(lib, need_str(args, "module")?)?;
            let due = parse_due(need_str(args, "due")?)?;
            Ok(Preview { title: trf("%@ anlegen", &[&kind_from(args).label()]), lines: vec![title.to_string(), format!("{} · {}", module.code, due.format("%d.%m.%Y %H:%M"))] })
        }
        "set_status" => {
            let id = need_str(args, "id")?;
            let a = lib.assignment(id).ok_or_else(|| format!("No deadline with id `{id}`"))?;
            let status = status_from(args)?;
            let label = status.map_or_else(|| crate::core::i18n::tr("Automatisch"), Status::label);
            Ok(Preview { title: trf("%@: %@", &[&a.title, &label]), lines: Vec::new() })
        }
        "file_document" => {
            let path = resolve(lib, need_str(args, "path")?)?;
            let folder = resolve(lib, need_str(args, "folder")?)?;
            let name = arg_str(args, "name").map_or_else(|| name_of(&path), String::from);
            Ok(Preview { title: trf("%@ nach %@ ablegen", &[&name_of(&path), &folder_label(lib, &folder)]), lines: if name == name_of(&path) { Vec::new() } else { vec![trf("Neuer Name: %@", &[&name])] } })
        }
        other => Err(format!("`{other}` is not a write tool")),
    }
}

fn folder_label(lib: &Library, folder: &Path) -> String {
    let r = rel(lib, folder);
    if r.is_empty() { lib.root.as_deref().map(name_of).unwrap_or_default() } else { r }
}

fn kind_from(args: &Value) -> Kind {
    match arg_str(args, "kind") {
        Some("submission") => Kind::Submission,
        Some("exam") => Kind::Exam,
        _ => Kind::Task,
    }
}

fn status_from(args: &Value) -> Result<Option<Status>, String> {
    Ok(match need_str(args, "status")? {
        "done" => Some(Status::Done),
        "uploaded" => Some(Status::Uploaded),
        "skipped" => Some(Status::Skipped),
        "open" => Some(Status::Open),
        "in_progress" => Some(Status::InProgress),
        "auto" => None,
        other => return Err(format!("Unknown status `{other}`")),
    })
}

fn create_folder(lib: &Library, args: &Value) -> Result<PathBuf, String> {
    if let Some(folder) = arg_str(args, "folder") {
        return resolve(lib, folder);
    }
    let module = match lib.route() {
        Route::Module(id) => lib.module_by_id(id),
        Route::Doc(path) => lib.module_containing(path),
        _ => lib.active_document.as_ref().and_then(|p| lib.module_containing(p)),
    };
    let suggested = lib.suggested_folders(module.map(|m| m.id.as_str()));
    suggested.into_iter().next().or_else(|| lib.root.clone()).ok_or_else(|| NO_ROOT.to_string())
}

/// Runs a tool against the library. Writes expect that the user allowed them already.
pub fn call(lib: &mut Library, name: &str, args: &Value) -> Result<Outcome, String> {
    let plain = |value: Value| Ok(Outcome { value, changed: None });
    match name {
        "get_context" => plain(context(lib)),
        "search" => {
            let query = need_str(args, "query")?;
            let folder = arg_str(args, "folder").map(|f| resolve(lib, f)).transpose()?;
            let limit = arg_usize(args, "limit").unwrap_or(20).clamp(1, SEARCH_LIMIT);
            plain(search(lib, query, folder.as_deref(), limit))
        }
        "read_note" => {
            let path = resolve(lib, need_str(args, "path")?)?;
            if !is_text(&path) {
                return Err(if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) { "That is a PDF; use read_pdf_text.".to_string() } else { "Not a text file.".to_string() });
            }
            let text = current_text(lib, &path)?;
            let version = version_of(&text);
            let total = text.split('\n').count();
            let from = arg_usize(args, "from_line").unwrap_or(1).max(1);
            let to = arg_usize(args, "to_line").unwrap_or(total).min(total);
            if from > to {
                return Err(format!("Lines {from}-{to} are outside the note, which has {total} lines."));
            }
            let part: Vec<&str> = text.split('\n').skip(from - 1).take(to + 1 - from).collect();
            let (shown, truncated) = cut(&part.join("\n"), NOTE_CHARS);
            plain(json!({ "path": rel(lib, &path), "version": version, "lines": total, "from_line": from, "to_line": to, "truncated": truncated, "text": shown }))
        }
        "read_pdf_text" => {
            let path = resolve(lib, need_str(args, "path")?)?;
            plain(pdf_text(lib, &path, need_str(args, "pages")?)?)
        }
        "list_folder" => {
            let folder = resolve(lib, arg_str(args, "path").unwrap_or_default())?;
            plain(list_folder(lib, &folder)?)
        }
        "list_deadlines" => plain(list_deadlines(lib, args)?),
        "list_modules" => plain(json!({ "modules": lib.semester_modules().iter().map(|m| {
            let mut v = module_json(m);
            v["sessions"] = json!(m.sessions.iter().map(|s| json!({ "title": s.title, "weekday": s.weekday, "start": s.start, "end": s.end, "location": s.location })).collect::<Vec<_>>());
            v
        }).collect::<Vec<_>>() })),
        "get_week" => plain(week(lib, args)?),
        "list_cards" => plain(list_cards(lib, args)?),
        "write_note" => write_note(lib, args),
        "create_note" => {
            let title = need_str(args, "title")?;
            let folder = create_folder(lib, args)?;
            let path = lib.create_note(title, &folder, "md").ok_or_else(|| lib.last_error.take().unwrap_or_else(|| "The note could not be created.".into()))?;
            if let Some(text) = args.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                let body = text.to_string();
                lib.edit_note(&path, "ai", trf("%@ geändert", &[&name_of(&path)]), |before| with_newline(format!("{}\n{body}", with_newline(before.to_string()))));
            }
            let version = current_text(lib, &path).map(|t| version_of(&t)).unwrap_or_default();
            Ok(Outcome { value: json!({ "path": rel(lib, &path), "version": version }), changed: Some(trf("%@ angelegt", &[&name_of(&path)])) })
        }
        "add_cards" => {
            let path = resolve(lib, need_str(args, "note")?)?;
            if !is_text(&path) {
                return Err("Cards go into a Markdown note.".into());
            }
            let key = lib.buffer_key(&path);
            let cards = cards_from(args)?;
            let before = current_text(lib, &key)?;
            let (lines, duplicates) = new_card_lines(&before, &cards);
            if lines.is_empty() {
                return plain(json!({ "added": 0, "duplicates": duplicates, "note": rel(lib, &path) }));
            }
            let count = lines.len();
            let label = if count == 1 { trf("1 Karte in %@ ergänzt", &[&name_of(&path)]) } else { trf("%lld Karten in %@ ergänzt", &[&count, &name_of(&path)]) };
            if !lib.edit_note(&key, "ai", label.clone(), |b| appended(b, &lines)) {
                return Err(lib.last_error.take().unwrap_or_else(|| "The note could not be written.".into()));
            }
            Ok(Outcome { value: json!({ "added": count, "duplicates": duplicates, "note": rel(lib, &path) }), changed: Some(label) })
        }
        "add_deadline" => {
            let title = need_str(args, "title")?;
            let module = find_module(lib, need_str(args, "module")?)?.id.clone();
            let due = parse_due(need_str(args, "due")?)?;
            let kind = kind_from(args);
            let m = ManualAssignment { module, title: title.to_string(), due: due.with_timezone(&chrono::Utc), kind, optional: args.get("optional").and_then(Value::as_bool).unwrap_or(false), upload: kind == Kind::Submission, ..ManualAssignment::default() };
            let id = format!("manual.{}", m.id);
            lib.add_manual(m);
            Ok(Outcome { value: json!({ "id": id, "title": title, "due": due.format("%Y-%m-%dT%H:%M").to_string(), "kind": kind_name(kind) }), changed: Some(trf("%@ hinzugefügt", &[&title])) })
        }
        "set_status" => {
            let id = need_str(args, "id")?;
            let a = lib.assignment(id).cloned().ok_or_else(|| format!("No deadline with id `{id}`"))?;
            let status = status_from(args)?;
            lib.set_status(status, id, true);
            let now = lib.assignment(id).map_or("open", |a| status_name(lib.status(a)));
            let label = status.map_or_else(|| crate::core::i18n::tr("Automatisch"), Status::label);
            Ok(Outcome { value: json!({ "id": id, "status": now }), changed: Some(trf("%@: %@", &[&a.title, &label])) })
        }
        "file_document" => {
            let path = resolve(lib, need_str(args, "path")?)?;
            if !path.is_file() {
                return Err(format!("No such file: {}", rel(lib, &path)));
            }
            let folder = resolve(lib, need_str(args, "folder")?)?;
            let name = arg_str(args, "name").map_or_else(|| name_of(&path), String::from);
            if name.contains(['/', '\\']) {
                return Err("`name` is a file name, not a path.".into());
            }
            let deadline = arg_str(args, "deadline");
            if let Some(id) = deadline
                && lib.assignment(id).is_none()
            {
                return Err(format!("No deadline with id `{id}`"));
            }
            let target = lib.file_for(&path, &folder, &name, deadline, true).ok_or_else(|| lib.last_error.take().unwrap_or_else(|| "The file could not be moved.".into()))?;
            Ok(Outcome { value: json!({ "path": rel(lib, &target) }), changed: Some(trf("%@ abgelegt", &[&name_of(&target)])) })
        }
        "open" => {
            let path = resolve(lib, need_str(args, "path")?)?;
            if !path.exists() {
                return Err(format!("No such file: {}", rel(lib, &path)));
            }
            if let Some(find) = arg_str(args, "find") {
                lib.find_in_file = Some((path.clone(), find.to_string(), 0));
            }
            lib.quiet_open = true;
            lib.open_file(&path);
            plain(json!({ "opened": rel(lib, &path) }))
        }
        other => Err(format!("Unknown tool `{other}`")),
    }
}

fn context(lib: &Library) -> Value {
    let route = match lib.route() {
        Route::Today => "overview".to_string(),
        Route::Grid => "semester".to_string(),
        Route::Inbox => "inbox".to_string(),
        Route::Search => "search".to_string(),
        Route::Record => "record".to_string(),
        Route::Module(_) => "module".to_string(),
        Route::Doc(_) => "document".to_string(),
        Route::Notes => "notes".to_string(),
        Route::Graph => "graph".to_string(),
        Route::Cards => "cards".to_string(),
    };
    let module = match lib.route() {
        Route::Module(id) => lib.module_by_id(id),
        Route::Doc(path) => lib.module_containing(path),
        _ => None,
    };
    json!({
        "language": if lang() == Lang::En { "en" } else { "de" },
        "folder": lib.root.as_deref().map(name_of),
        "route": route,
        "module": module.map(module_json),
        "active_document": lib.active_document.as_ref().map(|p| rel(lib, p)),
        "split_document": lib.split_document.as_ref().map(|p| rel(lib, p)),
        "selected_deadline": lib.selection().and_then(|id| lib.assignment(id)).map(|a| deadline_json(lib, a)),
        "today": lib.today().format("%Y-%m-%d").to_string(),
        "week": lib.current_week().map(|w| json!({ "index": w.index, "monday": w.monday.format("%Y-%m-%d").to_string() })),
        "indexed_files": lib.index_count,
        "inbox_files": lib.inbox.len(),
        "due_next_7_days": lib.upcoming_days(7).len(),
    })
}

fn search(lib: &Library, query: &str, folder: Option<&Path>, limit: usize) -> Value {
    let terms: Vec<String> = query.split_whitespace().map(String::from).collect();
    let mut seen = HashSet::new();
    let mut hits = Vec::new();
    for hit in lib.search_index.content_search(&terms, folder, limit) {
        if seen.insert(hit.file.path.clone()) {
            hits.push(json!({ "path": rel(lib, &hit.file.path), "name": hit.file.name, "excerpts": hit.spots.iter().map(|s| s.excerpt.clone()).collect::<Vec<_>>() }));
        }
    }
    let folded = fold(query);
    for f in lib.catalog.iter().filter(|f| f.folded_name.contains(&folded) && folder.is_none_or(|d| f.path().starts_with(d))) {
        if hits.len() >= limit {
            break;
        }
        if seen.insert(f.path().to_path_buf()) {
            hits.push(json!({ "path": rel(lib, f.path()), "name": f.name(), "excerpts": [] }));
        }
    }
    json!({ "query": query, "hits": hits })
}

fn list_folder(lib: &Library, folder: &Path) -> Result<Value, String> {
    let entries = std::fs::read_dir(folder).map_err(|_| format!("No such folder: {}", rel(lib, folder)))?;
    let mut items: Vec<(bool, String, PathBuf)> = entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| {
            let path = e.path();
            (path.is_dir(), e.file_name().to_string_lossy().to_string(), path)
        })
        .collect();
    items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| crate::core::text::natural_cmp(&a.1, &b.1)));
    let total = items.len();
    let shown: Vec<Value> = items.into_iter().take(FOLDER_ENTRIES).map(|(dir, name, path)| json!({ "name": name, "path": rel(lib, &path), "kind": if dir { "folder" } else { "file" } })).collect();
    Ok(json!({ "folder": rel(lib, folder), "entries": shown, "total": total }))
}

fn list_deadlines(lib: &Library, args: &Value) -> Result<Value, String> {
    let module = arg_str(args, "module").map(|m| find_module(lib, m)).transpose()?.map(|m| m.id.clone());
    let days = args.get("days").and_then(Value::as_i64);
    let include_done = args.get("include_done").and_then(Value::as_bool).unwrap_or(false);
    let limit = lib.now + chrono::Duration::days(days.unwrap_or(0));
    let list: Vec<Value> = lib
        .semester_assignments()
        .into_iter()
        .filter(|a| module.as_ref().is_none_or(|m| &a.module_id == m))
        .filter(|a| days.is_none() || (a.due <= limit && (a.due >= lib.now || !lib.status(a).is_closed())))
        .filter(|a| include_done || !lib.status(a).is_closed())
        .map(|a| deadline_json(lib, a))
        .collect();
    Ok(json!({ "now": lib.now.format("%Y-%m-%dT%H:%M").to_string(), "deadlines": list }))
}

fn week(lib: &Library, args: &Value) -> Result<Value, String> {
    let date = match arg_str(args, "date") {
        Some(d) => NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| "`date` must be YYYY-MM-DD".to_string())?,
        None => lib.today(),
    };
    let week = lib.lecture_week_containing(date);
    let monday = week.as_ref().map_or_else(|| date - chrono::Duration::days(i64::from(date.weekday().num_days_from_monday())), |w| w.monday);
    let days: Vec<Value> = (0..7)
        .map(|i| {
            let day = monday + chrono::Duration::days(i);
            json!({
                "date": day.format("%Y-%m-%d").to_string(),
                "weekday": day.weekday().to_string(),
                "sessions": lib.sessions_on(day).into_iter().map(|(m, s)| json!({ "module": m.code, "title": s.title, "start": s.start, "end": s.end, "location": s.location })).collect::<Vec<_>>(),
                "deadlines": lib.assignments_on(day).into_iter().map(|a| deadline_json(lib, a)).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({ "week": week.map(|w| w.index), "monday": monday.format("%Y-%m-%d").to_string(), "days": days }))
}

fn list_cards(lib: &Library, args: &Value) -> Result<Value, String> {
    let note = arg_str(args, "note").map(|n| resolve(lib, n)).transpose()?.map(|p| rel(lib, &p));
    let limit = arg_usize(args, "limit").unwrap_or(100).clamp(1, CARD_LIMIT);
    let now = store::now();
    let all = store::all_cards();
    let counts = store::counts("", now);
    let matching: Vec<&store::Card> = all.iter().filter(|c| note.as_ref().is_none_or(|n| c.note_rel.as_str() == n)).collect();
    let total = matching.len();
    let cards: Vec<Value> = matching
        .into_iter()
        .take(limit)
        .map(|c| json!({ "id": c.id(), "front": c.raw.front, "back": c.raw.back, "note": c.note_rel.as_str(), "phase": format!("{:?}", c.phase()).to_lowercase(), "due": c.state.as_ref().map(|s| s.due) }))
        .collect();
    Ok(json!({ "total": total, "due": { "new": counts.new, "learning": counts.learning, "review": counts.review }, "cards": cards }))
}

fn pdf_text(lib: &Library, path: &Path, pages: &str) -> Result<Value, String> {
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        return Err("Not a PDF.".into());
    }
    let (from, to) = match pages.split_once('-') {
        Some((a, b)) => (a.trim().parse::<usize>().ok(), b.trim().parse::<usize>().ok()),
        None => {
            let n = pages.trim().parse::<usize>().ok();
            (n, n)
        }
    };
    let (Some(from), Some(to)) = (from, to) else { return Err("`pages` must be like \"3\" or \"3-5\"".into()) };
    if from == 0 || from > to {
        return Err("`pages` counts from 1 and the range must not be empty.".into());
    }
    let doc = crate::core::pdf_render::PdfDoc::open(path).ok_or_else(|| format!("The PDF could not be opened: {}", rel(lib, path)))?;
    let count = doc.page_count();
    if from > count {
        return Err(format!("The PDF has {count} pages."));
    }
    let to = to.min(count).min(from + PDF_PAGES - 1);
    let mut out = Vec::new();
    let mut used = 0;
    let mut truncated = false;
    for index in from..=to {
        let text = doc.page_text(index - 1);
        let full = text.slice(0, text.glyph_count());
        let room = PDF_CHARS.saturating_sub(used);
        let (shown, cut_here) = cut(&full, room);
        used += shown.chars().count();
        out.push(json!({ "page": index, "text": shown }));
        if cut_here {
            truncated = true;
            break;
        }
    }
    Ok(json!({ "path": rel(lib, path), "pages": count, "from": from, "to": to, "truncated": truncated, "content": out }))
}

fn write_note(lib: &mut Library, args: &Value) -> Result<Outcome, String> {
    let path = resolve(lib, need_str(args, "path")?)?;
    if !is_text(&path) {
        return Err("write_note only writes text files.".into());
    }
    let key = lib.buffer_key(&path);
    let version = need_str(args, "version")?.to_string();
    let before = current_text(lib, &key)?;
    if version_of(&before) != version {
        return Err(STALE.into());
    }
    let planned = planned_text(&before, args)?;
    if planned == before {
        return Ok(Outcome { value: json!({ "path": rel(lib, &path), "version": version, "unchanged": true }), changed: None });
    }
    let stale = Cell::new(false);
    let label = trf("%@ geändert", &[&name_of(&path)]);
    let written = lib.edit_note(&key, "ai", label.clone(), |current| {
        if version_of(current) != version {
            stale.set(true);
            return current.to_string();
        }
        planned.clone()
    });
    if stale.get() {
        return Err(STALE.into());
    }
    if !written {
        return Err(lib.last_error.take().unwrap_or_else(|| "The note could not be written.".into()));
    }
    let after = current_text(lib, &key)?;
    Ok(Outcome { value: json!({ "path": rel(lib, &path), "version": version_of(&after), "lines": after.split('\n').count() }), changed: Some(label) })
}
