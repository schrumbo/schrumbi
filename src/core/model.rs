use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::day;
use super::json::iso8601;

pub const CURRENT_VERSION: u32 = 2;
pub const DEFAULT_NAMING: &str = "{code}_{yy}_{title}_{author}_{date}";
pub const PALETTE: [&str; 12] = ["blue", "green", "orange", "purple", "pink", "teal", "red", "yellow", "indigo", "mint", "brown", "gray"];

fn on() -> bool {
    true
}

fn submissions() -> String {
    "Abgaben".into()
}

fn default_exclude() -> Vec<String> {
    [".git", "node_modules", "build", "DerivedData", ".build", "Pods"].map(String::from).to_vec()
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

/// Workspace configuration stored as `schrumbi.json` in the workspace root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StudyConfig {
    pub version: u32,
    pub name: String,
    pub author: String,
    pub inbox: String,
    pub semesters: Vec<Semester>,
    pub modules: Vec<Module>,
    pub matrikel: String,
    pub coauthor_style: CoauthorStyle,
    /// IANA identifier; deadlines are interpreted in this zone (empty = system).
    pub time_zone: String,
    pub grading: GradingSystem,
    /// Name of credits, e.g. "ECTS", "CP", "Credits".
    pub credit_label: String,
    /// Folder names skipped by search and file browsing.
    pub search_exclude: Vec<String>,
    /// Filing renames to the module naming pattern unless switched off for the file. Configs written before this switch existed keep it on.
    #[serde(default = "on")]
    pub rename_on_filing: bool,
    /// Flags handed-in files that do not follow the naming pattern, and names notes in checked folders by it. Configs written before this switch existed keep it on.
    #[serde(default = "on")]
    pub check_names: bool,
    /// Reports folders that rules expect but that do not exist.
    pub check_folders: bool,
    /// Setup and filing may create folders; off, files only go into folders that exist.
    pub create_folders: bool,
    /// Folder inside each module where files for its deadlines go; empty = the module folder itself.
    #[serde(default = "submissions")]
    pub submission_folder: String,
}

impl Default for StudyConfig {
    fn default() -> Self {
        Self {
            version: 1,
            name: "Studium".into(),
            author: String::new(),
            inbox: "_Inbox".into(),
            semesters: Vec::new(),
            modules: Vec::new(),
            matrikel: String::new(),
            coauthor_style: CoauthorStyle::Initial,
            time_zone: String::new(),
            grading: GradingSystem::German,
            credit_label: "ECTS".into(),
            search_exclude: default_exclude(),
            rename_on_filing: false,
            check_names: false,
            check_folders: true,
            create_folders: true,
            submission_folder: submissions(),
        }
    }
}

impl StudyConfig {
    pub fn new(name: String, author: String, semesters: Vec<Semester>, modules: Vec<Module>) -> Self {
        Self { version: CURRENT_VERSION, name, author, semesters, modules, ..Default::default() }
    }

    /// Whether file names follow the module patterns at all; off, nothing is renamed or checked and new files keep their plain names.
    pub fn naming_convention(&self) -> bool {
        self.rename_on_filing || self.check_names
    }

    /// Switches renaming and name checks together.
    pub fn set_naming_convention(&mut self, on: bool) {
        (self.rename_on_filing, self.check_names) = (on, on);
    }

    pub fn module(&self, id: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.id == id)
    }

    pub fn module_mut(&mut self, id: &str) -> Option<&mut Module> {
        self.modules.iter_mut().find(|m| m.id == id)
    }

    /// Where files for a module's deadlines go, relative to the workspace root.
    pub fn submissions_of(&self, module: &Module) -> PathBuf {
        let folder = PathBuf::from(&module.folder);
        let name = self.submission_folder.trim().trim_matches(['/', '\\']);
        if name.is_empty() { folder } else { folder.join(name) }
    }

    pub fn semester(&self, id: &str) -> Option<&Semester> {
        self.semesters.iter().find(|s| s.id == id)
    }

    /// The semester containing `now`, else the next upcoming one, else the latest.
    pub fn current_semester(&self, now: DateTime<Local>) -> Option<&Semester> {
        let mut sorted: Vec<&Semester> = self.semesters.iter().collect();
        sorted.sort_by(|a, b| a.start.cmp(&b.start));
        let today = day::local_date(now);
        sorted
            .iter()
            .find(|s| s.contains(today))
            .or_else(|| sorted.iter().find(|s| s.start_date().is_some_and(|d| d > today)))
            .or(sorted.last())
            .copied()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Semester {
    pub id: String,
    pub name: String,
    /// First lecture day, `yyyy-MM-dd`.
    pub start: String,
    /// Last lecture day, `yyyy-MM-dd`.
    pub end: String,
    pub breaks: Vec<Break>,
    /// Single days on which deadlines move to the next day.
    pub holidays: Vec<Holiday>,
}

impl Default for Semester {
    fn default() -> Self {
        Self { id: new_id(), name: String::new(), start: String::new(), end: String::new(), breaks: Vec::new(), holidays: Vec::new() }
    }
}

impl Semester {
    pub fn start_date(&self) -> Option<NaiveDate> {
        day::parse(&self.start)
    }

    pub fn end_date(&self) -> Option<NaiveDate> {
        day::parse(&self.end)
    }

    pub fn contains(&self, d: NaiveDate) -> bool {
        matches!((self.start_date(), self.end_date()), (Some(s), Some(e)) if d >= s && d <= e)
    }

    pub fn break_at(&self, d: NaiveDate) -> Option<&Break> {
        self.breaks.iter().find(|b| b.contains(d))
    }

    pub fn is_holiday(&self, d: NaiveDate) -> bool {
        let s = day::format(d);
        self.holidays.iter().any(|h| h.date == s)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Break {
    pub name: String,
    pub start: String,
    pub end: String,
}

impl Break {
    pub fn contains(&self, d: NaiveDate) -> bool {
        matches!((day::parse(&self.start), day::parse(&self.end)), (Some(s), Some(e)) if d >= s && d <= e)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Holiday {
    pub name: String,
    /// `yyyy-MM-dd`.
    pub date: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Module {
    /// Folder of this module's notes inside an external vault, relative to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_folder: Option<String>,
    pub id: String,
    pub code: String,
    pub name: String,
    pub semester: String,
    /// Path relative to the workspace root.
    pub folder: String,
    pub color: String,
    /// File name pattern with `{code}`, `{yy}`, `{title}`, `{author}`, `{date}`.
    pub naming: String,
    pub rules: Vec<Rule>,
    pub sessions: Vec<Session>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ects: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade: Option<f64>,
    pub passed: bool,
    pub links: Vec<ModuleLink>,
    /// Weighted partial grades; when present the module grade is derived from them.
    pub components: Vec<GradeComponent>,
    /// Desired module grade, used to compute what the remaining components need.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_grade: Option<f64>,
}

impl Default for Module {
    fn default() -> Self {
        Self {
            notes_folder: None,
            id: new_id(),
            code: String::new(),
            name: String::new(),
            semester: String::new(),
            folder: String::new(),
            color: "blue".into(),
            naming: DEFAULT_NAMING.into(),
            rules: Vec::new(),
            sessions: Vec::new(),
            ects: None,
            grade: None,
            passed: false,
            links: Vec::new(),
            components: Vec::new(),
            target_grade: None,
        }
    }
}

impl Module {
    /// The module's mark for tight places: three characters at most, whatever the code says.
    pub fn badge(&self) -> String {
        let source = if self.code.trim().is_empty() { &self.name } else { &self.code };
        abbreviation(source)
    }

    pub fn display_name(&self) -> String {
        if self.name.is_empty() || self.name == self.code { self.code.clone() } else { format!("{} · {}", self.code, self.name) }
    }

    /// Average grade the ungraded components need to reach `target_grade`; `None` if not computable.
    pub fn required_for_target(&self) -> Option<f64> {
        let target = self.target_grade?;
        let weighted = self.components.iter().filter(|c| c.weight > 0.);
        let total: f64 = weighted.clone().map(|c| c.weight).sum();
        let open_weight: f64 = weighted.clone().filter(|c| c.grade.is_none()).map(|c| c.weight).sum();
        if open_weight <= 0. || total <= 0. {
            return None;
        }
        let earned: f64 = weighted.filter_map(|c| c.grade.map(|g| g * c.weight)).sum();
        Some((target * total - earned) / open_weight)
    }

    /// Grade from weighted components if any are graded, else the entered grade.
    pub fn effective_grade(&self) -> Option<f64> {
        let graded: Vec<_> = self.components.iter().filter(|c| c.grade.is_some() && c.weight > 0.).collect();
        let total: f64 = graded.iter().map(|c| c.weight).sum();
        if total <= 0. {
            return self.grade;
        }
        Some(graded.iter().map(|c| c.grade.unwrap_or(0.) * c.weight).sum::<f64>() / total)
    }

    /// The link used for uploads: the one marked for uploads, else the first.
    pub fn upload_link(&self) -> Option<&ModuleLink> {
        self.links.iter().find(|l| l.upload && l.url().is_some()).or_else(|| self.links.iter().find(|l| l.url().is_some()))
    }
}

/// Three characters at most for a module name: "Complex Analysis 1" is "CA1", "Mathe 2" is "MA2", "Datenbanken" is "DAT".
pub fn abbreviation(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    if name.chars().count() <= 3 {
        return name.to_string();
    }
    const SMALL: [&str; 16] = ["und", "and", "of", "the", "der", "die", "das", "des", "dem", "den", "für", "for", "in", "im", "zu", "to"];
    let mut spaced = String::new();
    let mut last: Option<char> = None;
    for c in name.chars() {
        if last.is_some_and(|l| (l.is_lowercase() && c.is_uppercase()) || (l.is_alphabetic() && c.is_ascii_digit())) {
            spaced.push(' ');
        }
        spaced.push(c);
        last = Some(c);
    }
    let mut words: Vec<&str> = Vec::new();
    let mut number = None;
    for word in spaced.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let roman = match word {
            "I" => Some("1"),
            "II" => Some("2"),
            "III" => Some("3"),
            "IV" => Some("4"),
            "V" => Some("5"),
            _ => None,
        };
        if word.chars().all(|c| c.is_ascii_digit()) {
            number = Some(word.to_string());
        } else if let Some(n) = roman.filter(|_| !words.is_empty()) {
            number = Some(n.to_string());
        } else if !SMALL.contains(&word.to_lowercase().as_str()) {
            words.push(word);
        }
    }
    let digits = number.map_or(String::new(), |n| n.chars().rev().take(1).collect());
    let room = 3 - digits.chars().count();
    let letters: String = match words.as_slice() {
        [] => String::new(),
        [only] => only.chars().take(room).collect(),
        many => many.iter().filter_map(|w| w.chars().next()).take(room).collect(),
    };
    let badge = format!("{}{digits}", letters.to_uppercase());
    if badge.is_empty() { name.chars().take(3).collect::<String>().to_uppercase() } else { badge.chars().take(3).collect() }
}

/// An abbreviation no other module has yet, still three characters at most.
pub fn unique_abbreviation(name: &str, taken: &[String]) -> String {
    let base = abbreviation(name);
    if !taken.contains(&base) {
        return base;
    }
    let letters: Vec<char> = name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_uppercase).collect();
    let first = base.chars().next().unwrap_or('M');
    for second in letters.iter().skip(1) {
        for third in letters.iter().skip(2) {
            let candidate: String = [first, *second, *third].iter().collect();
            if !taken.contains(&candidate) {
                return candidate;
            }
        }
    }
    (2..100).map(|n| format!("{}{n}", base.chars().take(1).collect::<String>())).find(|c| !taken.contains(c)).unwrap_or(base)
}

/// A recurring deadline, generated once per lecture week.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub title: String,
    pub short: String,
    /// ISO weekday, 1 = Monday … 7 = Sunday.
    pub weekday: u32,
    /// Due time, `HH:mm`.
    pub time: String,
    /// Weeks after the lecture week the deadline falls in.
    pub week_offset: i64,
    /// Optional deadlines are marked as skipped instead of overdue when nothing was handed in.
    pub optional: bool,
    /// Whether the deadline requires an upload confirmation.
    pub upload: bool,
    /// Folder relative to the module folder; `{w}`, `{kw}`, `{date}` make it per-week.
    pub folder: String,
    /// Glob a file must match to count, e.g. `*.pdf`.
    pub file_pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_week: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_week: Option<u32>,
    /// Every n-th lecture week, counted from `start_week`.
    pub interval: u32,
    /// When set, only these lecture weeks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weeks: Option<Vec<u32>>,
    /// Extra words that identify files for this rule, e.g. "Blatt", "Sheet".
    pub keywords: Vec<String>,
    /// Also count files in subfolders.
    pub recursive: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            id: new_id(),
            title: String::new(),
            short: String::new(),
            weekday: 1,
            time: "23:59".into(),
            week_offset: 0,
            optional: false,
            upload: true,
            folder: String::new(),
            file_pattern: "*".into(),
            start_week: None,
            end_week: None,
            interval: 1,
            weeks: None,
            keywords: Vec::new(),
            recursive: true,
        }
    }
}

impl Rule {
    /// A new rule with the defaults the settings editor starts from.
    pub fn new(title: &str) -> Self {
        Self { title: title.into(), time: "18:00".into(), ..Default::default() }
    }

    pub fn applies_to_week(&self, index: u32) -> bool {
        if let Some(weeks) = self.weeks.as_ref().filter(|w| !w.is_empty()) {
            return weeks.contains(&index);
        }
        let first = self.start_week.unwrap_or(1);
        if index < first || self.end_week.is_some_and(|e| index > e) {
            return false;
        }
        (index - first) % self.interval.max(1) == 0
    }

    pub fn label(&self) -> &str {
        if self.short.is_empty() { &self.title } else { &self.short }
    }

    pub fn is_per_week(&self) -> bool {
        ["{w}", "{kw}", "{date}"].iter().any(|t| self.folder.contains(t))
    }
}

/// A weekly class in the timetable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub id: String,
    pub title: String,
    /// ISO weekday, 1 = Monday … 7 = Sunday.
    pub weekday: u32,
    pub start: String,
    pub end: String,
    pub location: String,
}

impl Default for Session {
    fn default() -> Self {
        Self { id: new_id(), title: String::new(), weekday: 1, start: "09:00".into(), end: "10:00".into(), location: String::new() }
    }
}

/// A web link attached to a module, e.g. the Moodle course.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModuleLink {
    pub id: String,
    pub title: String,
    pub address: String,
    /// Opened by the upload flow.
    pub upload: bool,
}

impl Default for ModuleLink {
    fn default() -> Self {
        Self { id: new_id(), title: String::new(), address: String::new(), upload: false }
    }
}

impl ModuleLink {
    /// Only http(s) links are opened.
    pub fn url(&self) -> Option<url::Url> {
        let trimmed = self.address.trim();
        let with_scheme = if trimmed.contains("://") { trimmed.to_string() } else { format!("https://{trimmed}") };
        let u = url::Url::parse(&with_scheme).ok()?;
        (matches!(u.scheme(), "http" | "https") && u.host_str().is_some_and(|h| !h.is_empty())).then_some(u)
    }
}

/// A weighted part of a module grade, e.g. an exam or a project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GradeComponent {
    pub id: String,
    pub name: String,
    pub weight: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade: Option<f64>,
}

impl Default for GradeComponent {
    fn default() -> Self {
        Self { id: new_id(), name: String::new(), weight: 1., grade: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoauthorStyle {
    /// `Nachname-V`, e.g. `Mueller-C`.
    #[default]
    Initial,
    /// Any token without underscores.
    Free,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GradingSystem {
    /// 1.0 (best) – 5.0, passed up to 4.0.
    #[default]
    German,
    /// 0–100 %, passed from 50.
    Percent,
    /// 0.0–4.0 GPA (best 4.0), passed from 1.0.
    Gpa,
    /// Only passed / failed.
    PassFail,
}

impl GradingSystem {
    pub const ALL: [GradingSystem; 4] = [Self::German, Self::Percent, Self::Gpa, Self::PassFail];

    pub fn uses_grades(self) -> bool {
        self != Self::PassFail
    }

    pub fn passes(self, grade: f64) -> bool {
        match self {
            Self::German => grade <= 4.,
            Self::Percent => grade >= 50.,
            Self::Gpa => grade >= 1.,
            Self::PassFail => true,
        }
    }

    pub fn range(self) -> (f64, f64) {
        match self {
            Self::German => (1., 5.),
            Self::Percent => (0., 100.),
            Self::Gpa => (0., 4.),
            Self::PassFail => (0., 1.),
        }
    }

    /// Whether a higher number is better.
    pub fn higher_is_better(self) -> bool {
        self != Self::German
    }
}

/// User-made state stored as `.schrumbi-state.json` in the workspace root.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StudyState {
    pub overrides: BTreeMap<String, Override>,
    pub notes: BTreeMap<String, String>,
    pub manual: Vec<ManualAssignment>,
    /// Per-occurrence due dates that replace the rule's date.
    #[serde(with = "iso8601::map")]
    pub moved: BTreeMap<String, DateTime<Utc>>,
    /// Files that belong to a deadline, as paths relative to the workspace root.
    pub linked_files: BTreeMap<String, Vec<String>>,
    /// Set once the files older versions found by date in shared folders were turned into links.
    pub links_adopted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Override {
    pub status: Status,
    #[serde(with = "iso8601")]
    pub date: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    #[default]
    Submission,
    Task,
    Exam,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Submission, Kind::Task, Kind::Exam];
}

/// A one-off entry that is not generated by a rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ManualAssignment {
    pub id: String,
    pub module: String,
    pub title: String,
    #[serde(with = "iso8601")]
    pub due: DateTime<Utc>,
    pub kind: Kind,
    pub optional: bool,
    pub upload: bool,
    /// Folder relative to the module folder.
    pub folder: String,
    /// Calendar UID when imported, used to update instead of duplicate on re-import.
    #[serde(rename = "externalID", skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}

impl Default for ManualAssignment {
    fn default() -> Self {
        Self {
            id: new_id(),
            module: String::new(),
            title: String::new(),
            due: Utc::now(),
            kind: Kind::Submission,
            optional: false,
            upload: true,
            folder: String::new(),
            external_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Open,
    InProgress,
    Ready,
    Uploaded,
    Done,
    Skipped,
    Overdue,
}

impl Status {
    pub fn is_closed(self) -> bool {
        matches!(self, Status::Uploaded | Status::Done | Status::Skipped)
    }
}

/// A concrete deadline, generated from a rule or entered manually.
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub id: String,
    pub module_id: String,
    pub semester_id: String,
    pub rule_id: Option<String>,
    pub title: String,
    pub short: String,
    pub kind: Kind,
    pub due: DateTime<Local>,
    pub week: Option<u32>,
    pub iso_week: Option<u32>,
    pub optional: bool,
    pub upload: bool,
    pub folder: PathBuf,
    pub per_week_folder: bool,
    pub file_pattern: String,
    /// Start of the window in which a file in a shared folder counts (previous occurrence's due date).
    pub window_start: Option<DateTime<Local>>,
    pub recursive: bool,
    pub keywords: Vec<String>,
    pub is_moved: bool,
    /// The shared folder older versions searched by date, read once to turn what they found into links.
    pub legacy: Option<(PathBuf, bool)>,
}

impl Assignment {
    pub fn is_manual(&self) -> bool {
        self.rule_id.is_none()
    }

    pub fn tracks_files(&self) -> bool {
        self.kind == Kind::Submission
    }

    /// Files in its folder count without a link, because the folder belongs to this deadline alone.
    pub fn owns_folder(&self) -> bool {
        self.per_week_folder
    }
}

/// The computed status of an assignment after scanning its folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub status: Status,
    pub files: Vec<PathBuf>,
    pub misnamed: Vec<PathBuf>,
    pub is_override: bool,
    pub completed_on_time: Option<bool>,
    /// Task descriptions found next to the submission; they never count as handed in.
    pub briefs: Vec<PathBuf>,
    /// Linked files that are no longer where the link points.
    pub missing: Vec<PathBuf>,
    /// Unlinked files in the deadline's folder from around its time; they count only once linked.
    pub suggested: Vec<PathBuf>,
}

impl Evaluation {
    pub fn empty() -> Self {
        Self { status: Status::Open, files: Vec::new(), misnamed: Vec::new(), is_override: false, completed_on_time: None, briefs: Vec::new(), missing: Vec::new(), suggested: Vec::new() }
    }

    /// Ready but not confirmed as uploaded after the deadline.
    pub fn needs_confirmation(&self, a: &Assignment, now: DateTime<Local>) -> bool {
        self.status == Status::Ready && a.upload && now > a.due
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LectureWeek {
    pub index: u32,
    pub monday: NaiveDate,
    pub iso_week: u32,
}

impl LectureWeek {
    pub fn label(&self) -> String {
        format!("W{:02}", self.index)
    }
}

impl Status {
    pub fn label(self) -> String {
        super::i18n::tr(match self {
            Status::Open => "Offen",
            Status::InProgress => "In Arbeit",
            Status::Ready => "Bereit zum Hochladen",
            Status::Uploaded => "Hochgeladen",
            Status::Done => "Erledigt",
            Status::Skipped => "Entfällt",
            Status::Overdue => "Überfällig",
        })
    }

    /// Icon name in `assets/icons`.
    pub fn icon(self) -> &'static str {
        match self {
            Status::Open => "circle",
            Status::InProgress => "circle-dot",
            Status::Ready => "circle-check",
            Status::Uploaded | Status::Done => "circle-check",
            Status::Skipped => "circle-minus",
            Status::Overdue => "circle-alert",
        }
    }
}

impl Kind {
    pub fn label(self) -> String {
        super::i18n::tr(match self {
            Kind::Submission => "Abgabe",
            Kind::Task => "Aufgabe",
            Kind::Exam => "Prüfung",
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Submission => "file-up",
            Kind::Task => "list-checks",
            Kind::Exam => "graduation-cap",
        }
    }
}

impl GradingSystem {
    pub fn label(self) -> String {
        super::i18n::tr(match self {
            Self::German => "Deutsch (1,0–5,0)",
            Self::Percent => "Prozent (0–100)",
            Self::Gpa => "GPA (0,0–4,0)",
            Self::PassFail => "Bestanden / nicht bestanden",
        })
    }
}

#[cfg(test)]
mod abbreviation_tests {
    use super::{abbreviation, unique_abbreviation};

    #[test]
    fn stays_within_three_characters() {
        for (name, short) in [
            ("Complex Analysis 1", "CA1"),
            ("Mathe 2", "MA2"),
            ("Datenbanken", "DAT"),
            ("Software Engineering", "SE"),
            ("Lineare Algebra und Geometrie 2", "LA2"),
            ("Analysis II", "AN2"),
            ("UXD1", "UX1"),
            ("MCI", "MCI"),
            ("ComplexAnalysis1", "CA1"),
            ("WS25/Theoretische Informatik", "TI"),
            ("Übung", "ÜBU"),
            ("01 Analysis", "AN1"),
            ("Mechanik_Für-Ingenieure", "MI"),
        ] {
            assert_eq!(abbreviation(name), short, "{name}");
        }
        assert!(abbreviation("日本語の授業について").chars().count() <= 3);
    }

    #[test]
    fn avoids_the_marks_of_other_modules() {
        let taken = vec!["CA1".to_string()];
        let other = unique_abbreviation("Computer Architecture 1", &taken);
        assert_ne!(other, "CA1");
        assert!(other.chars().count() <= 3);
    }
}
