use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde_json::{Value, json};

use super::tools::{self, Outcome};
use crate::core::filing::BRIEF_TAG;
use crate::core::i18n::{tr, trf};
use crate::core::library::Library;
use crate::core::model::{Kind, ManualAssignment};

const MAX_ROWS: usize = 50;

/// What a file in the inbox is to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Lecture,
    Exercise,
    Submission,
    Notes,
    Other,
}

impl FileKind {
    fn parse(raw: &str) -> Result<Self, String> {
        Ok(match raw {
            "lecture" => FileKind::Lecture,
            "exercise" => FileKind::Exercise,
            "submission" => FileKind::Submission,
            "notes" => FileKind::Notes,
            "other" => FileKind::Other,
            other => return Err(format!("Unknown kind `{other}`; use lecture, exercise, submission, notes or other.")),
        })
    }

    /// The kind in the user's words.
    pub fn label(self) -> String {
        match self {
            FileKind::Lecture => tr("Vorlesung"),
            FileKind::Exercise => tr("Aufgabenstellung"),
            FileKind::Submission => tr("Meine Abgabe"),
            FileKind::Notes => tr("Mitschrift"),
            FileKind::Other => tr("Sonstiges"),
        }
    }

    /// Whether the file belongs to a deadline.
    pub fn for_deadline(self) -> bool {
        matches!(self, FileKind::Exercise | FileKind::Submission)
    }
}

/// A deadline the agent read from a sheet that Schrumbi does not know yet.
#[derive(Debug, Clone, PartialEq)]
pub struct NewDeadline {
    pub title: String,
    pub due: chrono::DateTime<chrono::Local>,
}

/// Where one inbox file goes, as the agent read it, checked against the folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub path: PathBuf,
    pub module: Option<String>,
    pub kind: FileKind,
    pub date: Option<NaiveDate>,
    pub week: Option<u32>,
    pub title: String,
    pub deadline: Option<String>,
    pub new_deadline: Option<NewDeadline>,
    pub folder: PathBuf,
    pub name: String,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn name_for(lib: &Library, row: &Row, module: Option<&str>, week: Option<u32>, iso_week: Option<u32>) -> String {
    let original = file_name(&row.path);
    let renames = lib.config.as_ref().is_some_and(crate::core::model::StudyConfig::naming_convention);
    let (Some(module), true, true) = (module, renames, row.kind.for_deadline()) else { return original };
    let Some(naming) = lib.naming(module) else { return original };
    let date = row.date.unwrap_or_else(|| lib.today());
    let ext = crate::core::naming::extension(&original).to_string();
    let byline = (row.kind == FileKind::Exercise).then_some(BRIEF_TAG);
    naming.render(&row.title, date, &ext, &[], week.or(row.week), iso_week, byline)
}

fn row_of(lib: &Library, v: &Value) -> Result<Row, String> {
    let rel = text(v, "path").ok_or("every file needs its `path`")?;
    let path = tools::resolve(lib, rel)?;
    if !path.is_file() {
        return Err(format!("No such file: {rel}"));
    }
    let kind = FileKind::parse(text(v, "kind").unwrap_or("other"))?;
    let module = text(v, "module").map(|m| tools::find_module(lib, m).map(|m| m.id.clone())).transpose()?;
    let date = text(v, "date").map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| format!("{rel}: `date` must be YYYY-MM-DD"))).transpose()?;
    let week = v.get("week").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let title = text(v, "title").map_or_else(|| crate::core::naming::stem(&file_name(&path)).to_string(), String::from);
    let deadline = text(v, "deadline").map(String::from);
    if let Some(id) = &deadline
        && lib.assignment(id).is_none()
    {
        return Err(format!("{rel}: no deadline with id `{id}`; list_deadlines shows them, or give `new_deadline`."));
    }
    let new_deadline = match v.get("new_deadline").filter(|n| n.is_object()) {
        Some(n) if deadline.is_none() => {
            if module.is_none() {
                return Err(format!("{rel}: a new deadline needs the `module`."));
            }
            Some(NewDeadline { title: text(n, "title").unwrap_or(&title).to_string(), due: tools::parse_due(text(n, "due").ok_or_else(|| format!("{rel}: `new_deadline.due` is required"))?)? })
        }
        _ => None,
    };
    let mut row = Row { path, module: module.clone(), kind, date, week, title, deadline: deadline.clone(), new_deadline, folder: PathBuf::new(), name: String::new() };
    let assignment = deadline.as_deref().and_then(|id| lib.assignment(id)).cloned();
    let module = module.or_else(|| assignment.as_ref().map(|a| a.module_id.clone()));
    row.module.clone_from(&module);
    row.folder = match (text(v, "folder"), &assignment) {
        (Some(f), _) => tools::resolve(lib, f)?,
        (None, Some(a)) => lib.filing_folder(a).ok_or_else(|| format!("{rel}: the deadline's folder does not exist and Schrumbi may not create folders."))?,
        (None, None) => {
            let root = lib.root.clone().unwrap_or_default();
            match (&module, row.new_deadline.is_some()) {
                (Some(m), true) => lib.config.as_ref().and_then(|c| c.module(m).map(|x| root.join(c.submissions_of(x)))).unwrap_or(root),
                (Some(m), false) => lib.material_targets().into_iter().find(|t| &t.module.id == m).map(|t| t.path).or_else(|| lib.module_by_id(m).map(|x| root.join(&x.folder))).unwrap_or(root),
                (None, _) => return Err(format!("{rel}: give the `module` or a `folder`.")),
            }
        }
    };
    row.name = match text(v, "name") {
        Some(n) if n.contains(['/', '\\']) => return Err(format!("{rel}: `name` is a file name, not a path.")),
        Some(n) => n.to_string(),
        None => name_for(lib, &row, module.as_deref(), assignment.as_ref().and_then(|a| a.week), assignment.as_ref().and_then(|a| a.iso_week)),
    };
    Ok(row)
}

/// One field of an inbox row the user changes in the table.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Module(String),
    Kind(FileKind),
    Title(String),
    Date(Option<NaiveDate>),
    /// An existing deadline by id, or none.
    Deadline(Option<String>),
}

impl FileKind {
    /// Every kind, in the order the table offers them.
    pub const ALL: [FileKind; 5] = [FileKind::Lecture, FileKind::Exercise, FileKind::Submission, FileKind::Notes, FileKind::Other];

    fn key(self) -> &'static str {
        match self {
            FileKind::Lecture => "lecture",
            FileKind::Exercise => "exercise",
            FileKind::Submission => "submission",
            FileKind::Notes => "notes",
            FileKind::Other => "other",
        }
    }
}

/// The row with one field changed, its target folder and name worked out again the way the agent's proposal was.
pub fn revise(lib: &Library, row: &Row, change: Change) -> Result<Row, String> {
    let mut next = row.clone();
    let mut refile = false;
    let mut rename = false;
    match change {
        Change::Module(m) => {
            if next.module.as_deref() != Some(m.as_str()) {
                if next.deadline.as_deref().and_then(|id| lib.assignment(id)).is_some_and(|a| a.module_id != m) {
                    next.deadline = None;
                }
                next.module = Some(m);
                refile = true;
                rename = true;
            }
        }
        Change::Kind(k) => {
            if !k.for_deadline() {
                next.deadline = None;
                next.new_deadline = None;
            }
            next.kind = k;
            refile = true;
            rename = true;
        }
        Change::Title(t) => {
            let t = t.trim().to_string();
            if t.is_empty() {
                return Err(tr("Der Titel darf nicht leer sein."));
            }
            next.title = t;
            rename = true;
        }
        Change::Date(d) => {
            next.date = d;
            rename = true;
        }
        Change::Deadline(id) => {
            if let Some(a) = id.as_deref().and_then(|id| lib.assignment(id)) {
                next.module = Some(a.module_id.clone());
            }
            next.new_deadline = None;
            next.deadline = id;
            refile = true;
            rename = true;
        }
    }
    let mut v = json!({ "path": lib.relative_path(&next.path), "kind": next.kind.key(), "title": next.title });
    if let Some(m) = &next.module {
        v["module"] = json!(m);
    }
    if let Some(d) = next.date {
        v["date"] = json!(d.format("%Y-%m-%d").to_string());
    }
    if let Some(w) = next.week {
        v["week"] = json!(w);
    }
    if let Some(id) = &next.deadline {
        v["deadline"] = json!(id);
    } else if let Some(n) = &next.new_deadline {
        v["new_deadline"] = json!({ "title": n.title, "due": n.due.format("%Y-%m-%dT%H:%M").to_string() });
    }
    if !refile {
        v["folder"] = json!(lib.relative_path(&next.folder));
    }
    let custom = !next.kind.for_deadline() && next.name != file_name(&next.path);
    if !rename || custom {
        let name = match crate::core::naming::extension(&next.name) {
            _ if !custom || next.title == row.title => next.name.clone(),
            "" => crate::core::text::safe_file_stem(&next.title),
            ext => format!("{}.{ext}", crate::core::text::safe_file_stem(&next.title)),
        };
        v["name"] = json!(name);
    }
    row_of(lib, &v)
}

/// Checks the agent's reading of inbox files; nothing is moved or created.
pub fn plan(lib: &Library, proposal: &Value) -> Result<Vec<Row>, String> {
    let files = proposal.get("files").and_then(Value::as_array).ok_or("`files` is required")?;
    if files.is_empty() {
        return Err("`files` is empty".into());
    }
    files.iter().take(MAX_ROWS).map(|f| row_of(lib, f)).collect()
}

/// One line per row for the agent and the permission card.
pub fn summary(lib: &Library, row: &Row) -> String {
    let deadline = row.deadline.as_deref().and_then(|id| lib.assignment(id)).map(|a| a.title.clone()).or_else(|| row.new_deadline.as_ref().map(|n| trf("neu: %@", &[&n.title])));
    let target = format!("{}/{}", lib.relative_path(&row.folder), row.name);
    match deadline {
        Some(d) => format!("{} → {target} ({d})", file_name(&row.path)),
        None => format!("{} → {target}", file_name(&row.path)),
    }
}

/// The plan as the agent reads it back.
pub fn plan_json(lib: &Library, rows: &[Row]) -> Value {
    json!({ "files": rows.iter().map(|r| json!({
        "path": lib.relative_path(&r.path),
        "kind": format!("{:?}", r.kind).to_lowercase(),
        "module": r.module.as_deref().and_then(|m| lib.module_by_id(m)).map(|m| m.code.clone()),
        "to": format!("{}/{}", lib.relative_path(&r.folder), r.name),
        "deadline": r.deadline,
        "new_deadline": r.new_deadline.as_ref().map(|n| json!({ "title": n.title, "due": n.due.format("%Y-%m-%dT%H:%M").to_string() })),
    })).collect::<Vec<_>>() })
}

/// Files the chosen rows through the app's own filing, new deadlines first; each move is undoable and one turn undoes as one.
pub fn apply(lib: &mut Library, rows: &[Row]) -> Result<Outcome, String> {
    if lib.busy() {
        return Err("Schrumbi is syncing or moving files right now. Try again in a moment.".into());
    }
    let mut filed = Vec::new();
    let mut made: Vec<(String, String, i64, String)> = Vec::new();
    for row in rows {
        let mut deadline = row.deadline.clone();
        let mut folder = row.folder.clone();
        let same = row.new_deadline.as_ref().zip(row.module.as_ref()).and_then(|(n, m)| made.iter().find(|(mm, t, due, _)| mm == m && crate::core::text::fold(t) == crate::core::text::fold(&n.title) && *due == n.due.timestamp()).map(|x| x.3.clone()));
        if let Some(id) = same {
            if let Some(f) = lib.assignment(&id).cloned().and_then(|a| lib.filing_folder(&a)) {
                folder = f;
            }
            deadline = Some(id);
        } else if let (Some(n), Some(module)) = (&row.new_deadline, &row.module) {
            let kind = if row.kind == FileKind::Submission || row.kind == FileKind::Exercise { Kind::Submission } else { Kind::Task };
            let m = ManualAssignment { module: module.clone(), title: n.title.clone(), due: n.due.with_timezone(&chrono::Utc), kind, upload: true, ..ManualAssignment::default() };
            let id = format!("manual.{}", m.id);
            made.push((module.clone(), n.title.clone(), n.due.timestamp(), id.clone()));
            lib.add_manual(m);
            if let Some(f) = lib.assignment(&id).cloned().and_then(|a| lib.filing_folder(&a)) {
                folder = f;
            }
            deadline = Some(id);
        }
        let target = lib.file_for(&row.path, &folder, &row.name, deadline.as_deref(), true).ok_or_else(|| lib.last_error.take().unwrap_or_else(|| "The file could not be filed.".into()))?;
        filed.push(lib.relative_path(&target));
    }
    let label = if filed.len() == 1 { trf("%@ abgelegt", &[&filed[0].rsplit('/').next().unwrap_or_default()]) } else { trf("%lld Dateien abgelegt", &[&filed.len()]) };
    Ok(Outcome { value: json!({ "filed": filed }), changed: Some(label) })
}
