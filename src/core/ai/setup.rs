use std::path::Path;

use serde_json::{Value, json};

use super::tools::Outcome;
use crate::core::day;
use crate::core::i18n::{tr, trf};
use crate::core::library::Library;
use crate::core::model::{Break, Module, ModuleLink, PALETTE, Rule, Semester, StudyConfig, new_id};
use crate::core::text::fold;

const NO_CONFIG: &str = "No study folder is open in Schrumbi.";
const MAX_MODULES: usize = 30;
const MAX_RULES: usize = 10;
const MAX_RENAMES: usize = 200;

/// One weekly deadline of a module as proposed.
#[derive(Debug, Clone, PartialEq)]
pub struct Weekly {
    pub title: String,
    pub weekday: u32,
    pub time: String,
    pub week_offset: i64,
    pub optional: bool,
    pub upload: bool,
}

/// A module as proposed: new, or a change to one that exists.
#[derive(Debug, Clone, PartialEq)]
pub struct ModulePlan {
    pub existing: Option<String>,
    pub code: String,
    pub name: String,
    pub folder: String,
    pub naming: Option<String>,
    pub moodle: Option<String>,
    pub weekly: Vec<Weekly>,
}

/// What one part of a proposal changes.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Semester(Semester),
    Module(ModulePlan),
    Naming { on: bool, author: Option<String> },
    Calendar(String),
}

/// One part of a proposal the user can take or leave.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub key: String,
    pub title: String,
    pub lines: Vec<String>,
    pub change: Change,
}

/// A file the agent would rename or move; Schrumbi only shows it, nothing is executed.
#[derive(Debug, Clone, PartialEq)]
pub struct Rename {
    pub from: String,
    pub to: String,
    pub why: String,
}

/// A whole setup proposal, checked against the study folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub items: Vec<Item>,
    pub renames: Vec<Rename>,
    pub workflow: String,
    pub source: Value,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn date(v: &Value, key: &str, what: &str) -> Result<String, String> {
    let raw = text(v, key).ok_or_else(|| format!("{what}: `{key}` is required (YYYY-MM-DD)"))?;
    day::parse(raw).map(day::format).ok_or_else(|| format!("{what}: `{key}` must be YYYY-MM-DD, got `{raw}`"))
}

fn shown(d: &str) -> String {
    day::parse(d).map_or_else(|| d.to_string(), |d| d.format("%d.%m.%Y").to_string())
}

fn clock(raw: &str) -> Option<String> {
    let (h, m) = raw.trim().split_once(':')?;
    let (h, m) = (h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?);
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

fn weekday_name(n: u32) -> String {
    let monday = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap_or_default();
    day::weekday_short(monday + chrono::Days::new(u64::from(n.clamp(1, 7) - 1))).to_string()
}

fn folder_of(lib: &Library, raw: &str) -> Result<String, String> {
    let rel = raw.trim().trim_matches(['/', '\\']).replace('\\', "/");
    if rel.is_empty() {
        return Err("A module needs a `folder`, relative to the study folder.".into());
    }
    super::tools::inside_root(lib, Path::new(&rel))?;
    Ok(rel)
}

fn url_of(raw: &str) -> Result<String, String> {
    let link = ModuleLink { address: raw.trim().to_string(), ..ModuleLink::default() };
    link.url().map(|u| u.to_string()).ok_or_else(|| format!("`{raw}` is not a web address"))
}

fn semester_plan(v: &Value, current: Option<&Semester>) -> Result<Semester, String> {
    let start = date(v, "start", "semester")?;
    let end = date(v, "end", "semester")?;
    if end < start {
        return Err("semester: `end` lies before `start`".into());
    }
    let mut breaks = Vec::new();
    for b in v.get("breaks").and_then(Value::as_array).into_iter().flatten() {
        let (s, e) = (date(b, "start", "break")?, date(b, "end", "break")?);
        if e < s {
            return Err("break: `end` lies before `start`".into());
        }
        breaks.push(Break { name: text(b, "name").unwrap_or_default().to_string(), start: s, end: e });
    }
    let base = current.cloned().unwrap_or_default();
    let name = text(v, "name").map(String::from).unwrap_or_else(|| if base.name.is_empty() { crate::core::workspace::semester_name(day::parse(&start).unwrap_or_default()) } else { base.name.clone() });
    Ok(Semester { name, start, end, breaks, ..base })
}

fn weekly_of(v: &Value) -> Result<Weekly, String> {
    let title = text(v, "title").ok_or("every weekly deadline needs a `title`")?.to_string();
    let weekday = v.get("weekday").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).filter(|n| (1..=7).contains(n)).ok_or_else(|| format!("`{title}`: `weekday` is 1 (Monday) … 7 (Sunday)"))?;
    let time = match text(v, "time") {
        Some(t) => clock(t).ok_or_else(|| format!("`{title}`: `time` must be HH:MM"))?,
        None => "23:59".to_string(),
    };
    let week_offset = v.get("week_offset").and_then(Value::as_i64).unwrap_or(0).clamp(0, 4);
    Ok(Weekly { title, weekday, time, week_offset, optional: v.get("optional").and_then(Value::as_bool).unwrap_or(false), upload: v.get("upload").and_then(Value::as_bool).unwrap_or(true) })
}

fn matching<'a>(config: &'a StudyConfig, semester: &str, folder: &str, code: &str) -> Option<&'a Module> {
    let mine = |m: &&Module| semester.is_empty() || m.semester == semester;
    config.modules.iter().filter(mine).find(|m| m.folder.eq_ignore_ascii_case(folder)).or_else(|| config.modules.iter().filter(mine).find(|m| !code.is_empty() && fold(&m.code) == fold(code)))
}

fn module_plan(lib: &Library, v: &Value, config: &StudyConfig, semester: &str) -> Result<ModulePlan, String> {
    let folder = folder_of(lib, text(v, "folder").unwrap_or_default())?;
    let name = text(v, "name").map(String::from).unwrap_or_else(|| folder.rsplit('/').next().unwrap_or(&folder).to_string());
    let code = text(v, "code").map(String::from).unwrap_or_default();
    let existing = matching(config, semester, &folder, &code);
    let code = if code.is_empty() { existing.map_or_else(|| crate::core::model::abbreviation(&name), |m| m.code.clone()) } else { code };
    let naming = text(v, "naming").map(String::from);
    if let Some(n) = &naming
        && !n.contains("{title}")
    {
        return Err(format!("module {code}: a `naming` pattern needs {{title}}, e.g. {{code}}_{{yy}}_{{title}}_{{author}}_{{date}}"));
    }
    let moodle = text(v, "moodle").map(url_of).transpose()?;
    let weekly = v.get("weekly").and_then(Value::as_array).into_iter().flatten().take(MAX_RULES).map(weekly_of).collect::<Result<Vec<_>, _>>()?;
    Ok(ModulePlan { existing: existing.map(|m| m.id.clone()), code, name, folder, naming, moodle, weekly })
}

fn weekly_line(w: &Weekly) -> String {
    let offset = if w.week_offset > 0 { trf(" (%lld Wochen später)", &[&w.week_offset]) } else { String::new() };
    trf("Jede Woche: %@, %@ %@%@", &[&w.title, &weekday_name(w.weekday), &w.time, &offset])
}

fn module_item(index: usize, m: &ModulePlan, config: &StudyConfig) -> Item {
    let label = if m.name.is_empty() || m.name == m.code { m.code.clone() } else { format!("{} · {}", m.code, m.name) };
    let before = m.existing.as_deref().and_then(|id| config.module(id));
    let title = if before.is_some() { trf("Modul %@ anpassen", &[&label]) } else { trf("Modul %@ anlegen", &[&label]) };
    let mut lines = Vec::new();
    if before.is_none_or(|b| b.folder != m.folder) {
        lines.push(trf("Ordner: %@", &[&m.folder]));
    }
    if let Some(n) = &m.naming
        && before.is_none_or(|b| &b.naming != n)
    {
        lines.push(trf("Dateinamen: %@", &[n]));
    }
    lines.extend(m.weekly.iter().filter(|w| before.is_none_or(|b| !b.rules.iter().any(|r| fold(&r.title) == fold(&w.title) && r.weekday == w.weekday && r.time == w.time))).map(weekly_line));
    if let Some(u) = &m.moodle
        && before.is_none_or(|b| !b.links.iter().any(|l| l.url().is_some_and(|x| x.as_str() == u)))
    {
        lines.push(trf("Moodle: %@", &[u]));
    }
    Item { key: format!("module:{index}"), title, lines, change: Change::Module(m.clone()) }
}

/// Checks a proposal of the agent against the study folder and splits it into parts the user takes one by one.
pub fn plan(lib: &Library, proposal: &Value) -> Result<Plan, String> {
    let config = lib.config.as_ref().ok_or(NO_CONFIG)?;
    if !proposal.is_object() {
        return Err("The proposal must be an object.".into());
    }
    let mut items = Vec::new();
    let current = lib.semester();
    let semester_id = current.map(|s| s.id.clone()).unwrap_or_default();
    if let Some(s) = proposal.get("semester").filter(|v| v.is_object()) {
        let s = semester_plan(s, current)?;
        let mut lines: Vec<String> = s.breaks.iter().map(|b| trf("Pause %@: %@ – %@", &[&b.name, &shown(&b.start), &shown(&b.end)])).collect();
        if current.is_some_and(|c| c.start == s.start && c.end == s.end && c.breaks == s.breaks && c.name == s.name) {
            lines.insert(0, tr("Wie bisher"));
        }
        items.push(Item { key: "semester".into(), title: trf("Semester %@: %@ – %@", &[&s.name, &shown(&s.start), &shown(&s.end)]), lines, change: Change::Semester(s) });
    }
    let modules = proposal.get("modules").and_then(Value::as_array).cloned().unwrap_or_default();
    if modules.len() > MAX_MODULES {
        return Err(format!("At most {MAX_MODULES} modules per proposal."));
    }
    let mut seen = Vec::new();
    for (i, m) in modules.iter().enumerate() {
        let plan = module_plan(lib, m, config, &semester_id)?;
        if seen.contains(&plan.folder) {
            return Err(format!("The folder `{}` is named for two modules.", plan.folder));
        }
        seen.push(plan.folder.clone());
        items.push(module_item(i, &plan, config));
    }
    if let Some(n) = proposal.get("naming").filter(|v| v.is_object()) {
        let on = n.get("on").and_then(Value::as_bool).unwrap_or(true);
        let author = text(n, "author").map(String::from);
        let title = match (&author, on) {
            (Some(a), true) => trf("Dateinamen nach Muster, Kürzel %@", &[a]),
            (None, true) => tr("Dateinamen nach Muster"),
            (_, false) => tr("Dateinamen nicht umbenennen"),
        };
        items.push(Item { key: "naming".into(), title, lines: Vec::new(), change: Change::Naming { on, author } });
    }
    if let Some(raw) = text(proposal, "moodle_calendar") {
        let url = url_of(raw)?;
        let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
        items.push(Item { key: "calendar".into(), title: tr("Moodle-Kalender verbinden"), lines: vec![host], change: Change::Calendar(url) });
    }
    let mut renames = Vec::new();
    for r in proposal.get("renames").and_then(Value::as_array).into_iter().flatten().take(MAX_RENAMES) {
        let (Some(from), Some(to)) = (text(r, "from"), text(r, "to")) else { return Err("every rename needs `from` and `to`".into()) };
        super::tools::inside_root(lib, Path::new(from))?;
        super::tools::inside_root(lib, Path::new(to))?;
        renames.push(Rename { from: from.to_string(), to: to.to_string(), why: text(r, "why").unwrap_or_default().to_string() });
    }
    if items.is_empty() && renames.is_empty() {
        return Err("The proposal is empty: give semester, modules, naming, moodle_calendar or renames.".into());
    }
    Ok(Plan { items, renames, workflow: text(proposal, "workflow").unwrap_or_default().chars().take(600).collect(), source: proposal.clone() })
}

/// The plan as the agent reads it back.
pub fn plan_json(plan: &Plan) -> Value {
    json!({
        "items": plan.items.iter().map(|i| json!({ "key": i.key, "title": i.title, "details": i.lines })).collect::<Vec<_>>(),
        "renames": plan.renames.iter().map(|r| json!({ "from": r.from, "to": r.to })).collect::<Vec<_>>(),
        "note": "Renames are only shown to the user as a list; Schrumbi never renames or moves files for a setup.",
    })
}

fn apply_module(config: &mut StudyConfig, plan: &ModulePlan, semester: &str) {
    let index = plan.existing.as_deref().and_then(|id| config.modules.iter().position(|m| m.id == id));
    let position = config.modules.len();
    let module = match index {
        Some(i) => &mut config.modules[i],
        None => {
            config.modules.push(Module { id: new_id(), semester: semester.to_string(), color: PALETTE[position % PALETTE.len()].into(), ..Module::default() });
            let last = config.modules.len() - 1;
            &mut config.modules[last]
        }
    };
    module.code.clone_from(&plan.code);
    module.name.clone_from(&plan.name);
    module.folder.clone_from(&plan.folder);
    if let Some(n) = &plan.naming {
        module.naming.clone_from(n);
    }
    for w in &plan.weekly {
        let rule = Rule { title: w.title.clone(), weekday: w.weekday, time: w.time.clone(), week_offset: w.week_offset, optional: w.optional, upload: w.upload, ..Rule::default() };
        match module.rules.iter_mut().find(|r| fold(&r.title) == fold(&w.title)) {
            Some(r) => {
                r.weekday = rule.weekday;
                r.time = rule.time;
                r.week_offset = rule.week_offset;
                r.optional = rule.optional;
                r.upload = rule.upload;
            }
            None => module.rules.push(rule),
        }
    }
    if let Some(u) = &plan.moodle
        && !module.links.iter().any(|l| l.url().is_some_and(|x| x.as_str() == u))
    {
        let upload = module.upload_link().is_none();
        module.links.push(ModuleLink { title: "Moodle".into(), address: u.clone(), upload, ..ModuleLink::default() });
    }
}

/// Applies the chosen parts of a plan as one undoable step; only settings change, never a file.
pub fn apply(lib: &mut Library, plan: &Plan, keys: &[String]) -> Result<Outcome, String> {
    if lib.busy() {
        return Err("Schrumbi is syncing or moving files right now. Try again in a moment.".into());
    }
    let mut config = lib.config.clone().ok_or(NO_CONFIG)?;
    let chosen: Vec<&Item> = plan.items.iter().filter(|i| keys.is_empty() || keys.contains(&i.key)).collect();
    if chosen.is_empty() {
        return Err("None of the given items is in the proposal.".into());
    }
    let mut semester = lib.semester().map(|s| s.id.clone()).unwrap_or_default();
    let mut calendar = None;
    for item in &chosen {
        match &item.change {
            Change::Semester(s) => match config.semesters.iter_mut().find(|x| x.id == s.id) {
                Some(x) => *x = s.clone(),
                None => {
                    let fresh = Semester { id: if s.id.is_empty() { new_id() } else { s.id.clone() }, ..s.clone() };
                    let orphan: Vec<String> = config.modules.iter().filter(|m| m.semester.is_empty() || config.semester(&m.semester).is_none()).map(|m| m.id.clone()).collect();
                    semester.clone_from(&fresh.id);
                    config.semesters.push(fresh);
                    for m in config.modules.iter_mut().filter(|m| orphan.contains(&m.id)) {
                        m.semester.clone_from(&semester);
                    }
                }
            },
            Change::Naming { on, author } => {
                config.set_naming_convention(*on);
                if let Some(a) = author {
                    config.author.clone_from(a);
                }
            }
            Change::Calendar(url) => calendar = Some(url.clone()),
            Change::Module(_) => {}
        }
    }
    for item in &chosen {
        if let Change::Module(m) = &item.change {
            apply_module(&mut config, m, &semester);
        }
    }
    let label = if chosen.len() == 1 { trf("Einrichtung: %@", &[&chosen[0].title]) } else { trf("Einrichtung: %lld Punkte übernommen", &[&chosen.len()]) };
    if !lib.apply_settings(label.clone(), config, calendar) {
        return match lib.last_error.take() {
            Some(e) => Err(e),
            None => Ok(Outcome { value: json!({ "applied": [], "unchanged": true }), changed: None }),
        };
    }
    Ok(Outcome { value: json!({ "applied": chosen.iter().map(|i| i.key.clone()).collect::<Vec<_>>(), "files_changed": 0 }), changed: Some(label) })
}

/// What Schrumbi is set up with now, for the agent to build a proposal on.
pub fn read(lib: &Library, calendar: bool) -> Result<Value, String> {
    let config = lib.config.as_ref().ok_or(NO_CONFIG)?;
    let semester = lib.semester();
    let root = lib.root.clone().ok_or(NO_CONFIG)?;
    let mut folders = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        let mut names: Vec<(String, usize)> = entries
            .flatten()
            .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| (e.file_name().to_string_lossy().to_string(), std::fs::read_dir(e.path()).map(|d| d.count()).unwrap_or(0)))
            .collect();
        names.sort();
        folders = names.into_iter().take(200).map(|(name, entries)| json!({ "folder": name, "entries": entries })).collect();
    }
    Ok(json!({
        "today": lib.today().format("%Y-%m-%d").to_string(),
        "semester": semester.map(|s| json!({ "name": s.name, "start": s.start, "end": s.end, "breaks": s.breaks.iter().map(|b| json!({ "name": b.name, "start": b.start, "end": b.end })).collect::<Vec<_>>(), "guessed_by_schrumbi": crate::core::workspace::is_guessed(s) })),
        "modules": lib.semester_modules().iter().map(|m| json!({
            "id": m.id, "code": m.code, "name": m.name, "folder": m.folder, "naming": m.naming,
            "weekly": m.rules.iter().map(|r| json!({ "title": r.title, "weekday": r.weekday, "time": r.time, "week_offset": r.week_offset, "optional": r.optional })).collect::<Vec<_>>(),
            "links": m.links.iter().map(|l| l.address.clone()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "naming": { "on": config.naming_convention(), "author": config.author, "placeholders": "{code} {yy} {yyyy} {title} {author} {matrikel} {date} {date:FORMAT} {w} {kw}" },
        "moodle_calendar_connected": calendar,
        "top_folders": folders,
    }))
}
