//! What Schrumbi keeps in a calendar of the user's own and the steps that bring it there. Only events Schrumbi made and still knows are ever changed or removed.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{Duration, NaiveTime};
use serde::{Deserialize, Serialize};

use super::day;
use super::model::{Assignment, Module, Semester, StudyConfig};

/// One event as Schrumbi wants it in the calendar. Times are Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub location: String,
    pub notes: String,
    /// Minutes before the start for a reminder.
    pub alarm: Option<i64>,
}

/// An event Schrumbi made, under the calendar's identifiers, with what it last wrote.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Link {
    pub id: String,
    pub external: String,
    pub entry: Entry,
}

/// Everything Schrumbi made in the calendar, by event key, saved per computer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Links {
    pub calendar: String,
    /// The account the calendar lives in, so a signed-out account is told apart from a deleted calendar.
    pub source: String,
    pub events: BTreeMap<String, Link>,
}

/// A deadline as the calendar shows it; skipped ones are left out by the caller.
pub struct Deadline<'a> {
    pub assignment: &'a Assignment,
    pub done: bool,
}

/// The given deadlines and every single lecture day of the semester's timetable, without breaks and holidays. Open deadlines alert `alarm_hours` before they are due.
pub fn wanted(deadlines: &[Deadline], config: &StudyConfig, semester: &Semester, alarm_hours: Option<i64>) -> BTreeMap<String, Entry> {
    let mut out = BTreeMap::new();
    for d in deadlines {
        let a = d.assignment;
        let module = config.module(&a.module_id);
        let title = [module.map(|m| m.code.clone()), Some(a.title.clone())].into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        let end = a.due.timestamp();
        out.insert(
            format!("a:{}", a.id),
            Entry {
                title: if d.done { format!("✓ {title}") } else { title },
                start: end - 30 * 60,
                end,
                location: String::new(),
                notes: module.map_or(String::new(), |m| m.name.clone()),
                alarm: alarm_hours.filter(|h| *h > 0 && !d.done).map(|h| (h * 60 - 30).max(0)),
            },
        );
    }
    for module in config.modules.iter().filter(|m| m.semester == semester.id) {
        sessions(module, semester, &mut out);
    }
    out
}

fn sessions(module: &Module, semester: &Semester, out: &mut BTreeMap<String, Entry>) {
    use chrono::Datelike;
    let (Some(first), Some(last)) = (semester.start_date(), semester.end_date()) else { return };
    let clock = |t: &str| NaiveTime::parse_from_str(t, "%H:%M").ok();
    for session in &module.sessions {
        let (Some(from), Some(to)) = (clock(&session.start), clock(&session.end)) else { continue };
        let offset = (session.weekday as i64 - first.weekday().number_from_monday() as i64).rem_euclid(7);
        let days = std::iter::successors(Some(first + Duration::days(offset)), |d| Some(*d + Duration::days(7))).take_while(|d| *d <= last);
        let title = if session.title.is_empty() { module.code.clone() } else { format!("{} {}", module.code, session.title) };
        for d in days.filter(|d| !semester.is_holiday(*d) && semester.break_at(*d).is_none()) {
            let (Some(start), Some(end)) = (day::at(d, from), day::at(d, to)) else { continue };
            out.insert(
                format!("s:{}:{}T{}-{}:{}:{}", module.id, day::format(d), session.start, session.end, session.title, session.location),
                Entry { title: title.clone(), start: start.timestamp(), end: end.timestamp().max(start.timestamp()), location: session.location.clone(), notes: module.name.clone(), alarm: None },
            );
        }
    }
}

/// One change to the calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Create(String),
    /// Key and the event's current identifier.
    Update(String, String),
    Delete(String, String),
    /// The event is gone or left Schrumbi's calendar; only the link is dropped.
    Forget(String),
}

/// Steps from what Schrumbi made to what it wants. `find` gives a linked event's current identifier while it is still in Schrumbi's calendar.
pub fn plan(wanted: &BTreeMap<String, Entry>, links: &Links, mut find: impl FnMut(&Link) -> Option<String>) -> Vec<Step> {
    let mut steps = Vec::new();
    for (key, link) in &links.events {
        match (find(link), wanted.get(key)) {
            (Some(id), Some(entry)) if *entry != link.entry || id != link.id => steps.push(Step::Update(key.clone(), id)),
            (Some(_), Some(_)) => {}
            (Some(id), None) => steps.push(Step::Delete(key.clone(), id)),
            (None, Some(_)) => steps.push(Step::Create(key.clone())),
            (None, None) => steps.push(Step::Forget(key.clone())),
        }
    }
    steps.extend(wanted.keys().filter(|k| !links.events.contains_key(*k)).map(|k| Step::Create(k.clone())));
    steps
}

const MARK: &str = "schrumbi://event/";

/// The address every event Schrumbi makes carries, so it knows its own events even without links.
pub fn mark(key: &str) -> String {
    format!("{MARK}{}", url::form_urlencoded::byte_serialize(key.as_bytes()).collect::<String>())
}

/// The event key in a mark; `None` for any other address.
pub fn marked_key(address: &str) -> Option<String> {
    let encoded = address.strip_prefix(MARK)?;
    url::form_urlencoded::parse(format!("k={encoded}").as_bytes()).next().map(|(_, k)| k.into_owned()).filter(|k| !k.is_empty())
}

/// A calendar Schrumbi writes into.
pub trait Calendar {
    /// The linked event's current identifier, if it still is in Schrumbi's calendar.
    fn find(&mut self, link: &Link) -> Option<String>;
    /// Key, identifier, title and times of every event in Schrumbi's calendar that carries Schrumbi's mark.
    fn marked(&mut self) -> Vec<(String, String, Entry)>;
    /// Stages a new event with its mark, or changes the one with `id`.
    fn save(&mut self, id: Option<&str>, key: &str, entry: &Entry) -> Result<(), String>;
    fn remove(&mut self, id: &str) -> Result<(), String>;
    /// Writes what was staged and returns the identifier and external identifier of each saved event, in order.
    fn commit(&mut self) -> Result<Vec<(String, String)>, String>;
}

/// Takes back Schrumbi's own events that lost their link, e.g. after a crash right after writing. A second one for the same key with the same title and times is a duplicate to remove; a changed copy the user made stays.
fn adopt(calendar: &mut impl Calendar, wanted: &BTreeMap<String, Entry>, links: &mut Links) -> Vec<String> {
    let mut duplicates = Vec::new();
    let mut taken = std::collections::BTreeSet::new();
    for (key, id, seen) in calendar.marked() {
        let found = match links.events.get(&key) {
            Some(l) if l.id == id => Some(id.clone()),
            Some(l) => calendar.find(l),
            None => None,
        };
        let same = |e: &Entry| e.title == seen.title && e.start == seen.start && e.end == seen.end;
        if found.as_deref() == Some(id.as_str()) {
            if let Some(l) = links.events.get_mut(&key) {
                l.id = id;
            }
            taken.insert(key);
        } else if taken.contains(&key) || found.is_some() {
            if wanted.get(&key).is_some_and(same) || links.events.get(&key).is_some_and(|l| same(&l.entry)) {
                duplicates.push(id);
            }
        } else {
            links.events.insert(key.clone(), Link { id, external: String::new(), entry: Entry::default() });
            taken.insert(key);
        }
    }
    duplicates
}

/// Brings the calendar to `wanted` and the links along with it. Links change only once the calendar has the change, so a failure never loses track of an event.
pub fn apply(calendar: &mut impl Calendar, wanted: &BTreeMap<String, Entry>, links: &mut Links) -> Result<(), String> {
    let duplicates = adopt(calendar, wanted, links);
    let steps = plan(wanted, links, |l| calendar.find(l));
    let (mut saved, mut removed, mut failed) = (Vec::new(), Vec::new(), None);
    for id in &duplicates {
        if let Err(e) = calendar.remove(id) {
            failed.get_or_insert(e);
        }
    }
    for step in &steps {
        let result = match step {
            Step::Create(key) => calendar.save(None, key, &wanted[key]).map(|()| saved.push(key)),
            Step::Update(key, id) => calendar.save(Some(id), key, &wanted[key]).map(|()| saved.push(key)),
            Step::Delete(key, id) => calendar.remove(id).map(|()| removed.push(key)),
            Step::Forget(key) => {
                removed.push(key);
                Ok(())
            }
        };
        if let Err(e) = result {
            failed.get_or_insert(e);
        }
    }
    if saved.is_empty() && removed.is_empty() && duplicates.is_empty() {
        return failed.map_or(Ok(()), Err);
    }
    let ids = calendar.commit()?;
    for (key, (id, external)) in saved.into_iter().zip(ids) {
        links.events.insert(key.clone(), Link { id, external, entry: wanted[key].clone() });
    }
    for key in removed {
        links.events.remove(key);
    }
    failed.map_or(Ok(()), Err)
}

fn file() -> Option<PathBuf> {
    super::prefs::config_dir().map(|d| d.join("apple-calendar.json"))
}

/// The saved links; an unreadable file is kept as `apple-calendar.defekt.json`, and the marks find the events again.
pub fn load() -> Links {
    let Some(path) = file() else { return Links::default() };
    let Ok(bytes) = std::fs::read(&path) else { return Links::default() };
    serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        let _ = std::fs::copy(&path, path.with_extension("defekt.json"));
        Links::default()
    })
}

pub fn save(links: &Links) -> std::io::Result<()> {
    let Some(path) = file() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    super::note_buffer::write_atomic(&path, &serde_json::to_vec_pretty(links).map_err(std::io::Error::other)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Break, Holiday, ManualAssignment, Session, StudyState};
    use chrono::{Local, TimeZone, Utc};

    #[derive(Default)]
    struct Fake {
        events: BTreeMap<String, (String, Entry)>,
        marks: BTreeMap<String, String>,
        staged: Vec<(Option<String>, String, Entry)>,
        removing: Vec<String>,
        next: usize,
        fail_commit: bool,
    }

    impl Fake {
        fn ours(&self) -> usize {
            self.events.values().filter(|e| e.0 == "Schrumbi").count()
        }
    }

    impl Calendar for Fake {
        fn find(&mut self, link: &Link) -> Option<String> {
            let ours = |id: &String| self.events.get(id).is_some_and(|e| e.0 == "Schrumbi");
            if ours(&link.id) {
                return Some(link.id.clone());
            }
            self.events.keys().find(|id| format!("x-{id}") == link.external && ours(id)).cloned()
        }

        fn marked(&mut self) -> Vec<(String, String, Entry)> {
            self.marks
                .iter()
                .filter_map(|(id, m)| {
                    let e = self.events.get(id).filter(|e| e.0 == "Schrumbi")?;
                    Some((marked_key(m)?, id.clone(), Entry { title: e.1.title.clone(), start: e.1.start, end: e.1.end, ..Default::default() }))
                })
                .collect()
        }

        fn save(&mut self, id: Option<&str>, key: &str, entry: &Entry) -> Result<(), String> {
            self.staged.push((id.map(str::to_string), key.to_string(), entry.clone()));
            Ok(())
        }

        fn remove(&mut self, id: &str) -> Result<(), String> {
            self.removing.push(id.into());
            Ok(())
        }

        fn commit(&mut self) -> Result<Vec<(String, String)>, String> {
            if self.fail_commit {
                self.staged.clear();
                self.removing.clear();
                return Err("offline".into());
            }
            for id in std::mem::take(&mut self.removing) {
                self.events.remove(&id);
            }
            let mut ids = Vec::new();
            for (id, key, entry) in std::mem::take(&mut self.staged) {
                let id = id.unwrap_or_else(|| {
                    self.next += 1;
                    format!("ev{}", self.next)
                });
                self.marks.insert(id.clone(), mark(&key));
                self.events.insert(id.clone(), ("Schrumbi".into(), entry));
                ids.push((id.clone(), format!("x-{id}")));
            }
            Ok(ids)
        }
    }

    fn entry(title: &str, start: i64) -> Entry {
        Entry { title: title.into(), start, end: start + 60, ..Default::default() }
    }

    fn want(items: &[(&str, Entry)]) -> BTreeMap<String, Entry> {
        items.iter().map(|(k, e)| (k.to_string(), e.clone())).collect()
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let (mut cal, mut links) = (Fake::default(), Links::default());
        let w = want(&[("a:1", entry("Blatt 1", 100)), ("a:2", entry("Blatt 2", 200))]);
        apply(&mut cal, &w, &mut links).unwrap();
        assert_eq!(cal.ours(), 2);
        assert!(plan(&w, &links, |l| cal.find(l)).is_empty());
        apply(&mut cal, &w, &mut links).unwrap();
        assert_eq!(cal.events.len(), 2);
    }

    #[test]
    fn moved_done_and_deleted_deadlines_follow() {
        let (mut cal, mut links) = (Fake::default(), Links::default());
        apply(&mut cal, &want(&[("a:1", entry("Blatt 1", 100)), ("a:2", entry("Blatt 2", 200))]), &mut links).unwrap();
        let id = links.events["a:1"].id.clone();
        apply(&mut cal, &want(&[("a:1", entry("✓ Blatt 1", 150))]), &mut links).unwrap();
        assert_eq!(cal.events.len(), 1);
        assert_eq!(cal.events[&id].1.start, 150);
        assert_eq!(links.events["a:1"].id, id);
        assert!(!links.events.contains_key("a:2"));
    }

    #[test]
    fn events_the_user_made_are_never_touched() {
        let (mut cal, mut links) = (Fake::default(), Links::default());
        cal.events.insert("mine".into(), ("Schrumbi".into(), entry("Lerngruppe", 50)));
        cal.events.insert("work".into(), ("Arbeit".into(), entry("Schicht", 60)));
        apply(&mut cal, &want(&[("a:1", entry("Blatt 1", 100))]), &mut links).unwrap();
        apply(&mut cal, &BTreeMap::new(), &mut links).unwrap();
        assert!(cal.events.contains_key("mine") && cal.events.contains_key("work"));
        assert_eq!(cal.events.len(), 2);
        assert!(links.events.is_empty());
    }

    #[test]
    fn an_event_moved_to_another_calendar_is_left_there_and_made_again() {
        let (mut cal, mut links) = (Fake::default(), Links::default());
        let w = want(&[("a:1", entry("Blatt 1", 100))]);
        apply(&mut cal, &w, &mut links).unwrap();
        let id = links.events["a:1"].id.clone();
        cal.events.get_mut(&id).unwrap().0 = "Privat".into();
        apply(&mut cal, &w, &mut links).unwrap();
        assert_eq!(cal.events[&id].0, "Privat");
        assert_eq!(cal.ours(), 1);
        assert_ne!(links.events["a:1"].id, id);
        apply(&mut cal, &BTreeMap::new(), &mut links).unwrap();
        assert!(cal.events.contains_key(&id));
    }

    #[test]
    fn a_deleted_event_comes_back_and_a_vanished_one_is_forgotten() {
        let (mut cal, mut links) = (Fake::default(), Links::default());
        apply(&mut cal, &want(&[("a:1", entry("Blatt 1", 100)), ("a:2", entry("Blatt 2", 200))]), &mut links).unwrap();
        cal.events.clear();
        let steps = plan(&want(&[("a:1", entry("Blatt 1", 100))]), &links, |l| cal.find(l));
        assert_eq!(steps, vec![Step::Create("a:1".into()), Step::Forget("a:2".into())]);
        apply(&mut cal, &want(&[("a:1", entry("Blatt 1", 100))]), &mut links).unwrap();
        assert_eq!(cal.ours(), 1);
        assert_eq!(links.events.len(), 1);
    }

    #[test]
    fn a_failed_commit_keeps_every_link() {
        let (mut cal, mut links) = (Fake::default(), Links::default());
        apply(&mut cal, &want(&[("a:1", entry("Blatt 1", 100))]), &mut links).unwrap();
        let before = links.clone();
        cal.fail_commit = true;
        assert!(apply(&mut cal, &BTreeMap::new(), &mut links).is_err());
        assert!(apply(&mut cal, &want(&[("a:2", entry("Blatt 2", 1))]), &mut links).is_err());
        assert_eq!(links, before);
        cal.fail_commit = false;
        apply(&mut cal, &BTreeMap::new(), &mut links).unwrap();
        assert_eq!(cal.ours(), 0);
    }

    #[test]
    fn lost_links_find_schrumbis_events_by_their_mark_without_duplicates() {
        let (mut cal, mut links) = (Fake::default(), Links::default());
        let w = want(&[("a:1", entry("Blatt 1", 100)), ("s:m:2026-10-06T10:15-11:45:VL:", entry("CA1 VL", 200))]);
        apply(&mut cal, &w, &mut links).unwrap();
        cal.events.insert("mine".into(), ("Schrumbi".into(), entry("Blatt 1", 100)));
        let id = links.events["a:1"].id.clone();
        cal.marks.insert("copy".into(), mark("a:1"));
        cal.events.insert("copy".into(), ("Schrumbi".into(), entry("Blatt 1", 100)));
        let mut lost = Links::default();
        apply(&mut cal, &w, &mut lost).unwrap();
        assert_eq!(cal.events.len(), 3);
        assert!(cal.events.contains_key("mine"));
        assert_eq!(lost.events.len(), 2);
        assert!([id.as_str(), "copy"].contains(&lost.events["a:1"].id.as_str()));
        apply(&mut cal, &BTreeMap::new(), &mut lost).unwrap();
        assert_eq!(cal.events.keys().collect::<Vec<_>>(), vec!["mine"]);
    }

    #[test]
    fn an_event_with_a_new_identifier_stays_and_a_changed_copy_is_the_users() {
        let (mut cal, mut links) = (Fake::default(), Links::default());
        let w = want(&[("a:1", entry("Blatt 1", 100))]);
        apply(&mut cal, &w, &mut links).unwrap();
        let id = links.events["a:1"].id.clone();
        links.events.get_mut("a:1").unwrap().id = "stale".into();
        cal.marks.insert("lerngruppe".into(), mark("a:1"));
        cal.events.insert("lerngruppe".into(), ("Schrumbi".into(), entry("Lerngruppe", 900)));
        cal.marks.insert("twin".into(), mark("a:1"));
        cal.events.insert("twin".into(), ("Schrumbi".into(), entry("Blatt 1", 100)));
        apply(&mut cal, &w, &mut links).unwrap();
        assert_eq!(links.events["a:1"].id, id);
        assert!(cal.events.contains_key(&id) && cal.events.contains_key("lerngruppe"));
        assert!(!cal.events.contains_key("twin"));
        assert!(plan(&w, &links, |l| cal.find(l)).is_empty());
    }

    #[test]
    fn marks_carry_any_key_and_ignore_other_addresses() {
        for key in ["a:manual.1", "s:m:2026-10-06T10:15:Übung & Tutorium/2"] {
            assert_eq!(marked_key(&mark(key)).as_deref(), Some(key));
        }
        assert_eq!(marked_key("https://moodle.example.org/ca1"), None);
        assert_eq!(marked_key("schrumbi://event/"), None);
    }

    #[test]
    fn links_survive_a_restart() {
        let links = Links { calendar: "cal".into(), source: "icloud".into(), events: BTreeMap::from([("a:1".into(), Link { id: "ev1".into(), external: "x".into(), entry: entry("Blatt", 5) })]) };
        save(&links).unwrap();
        assert_eq!(load(), links);
    }

    #[test]
    fn timetable_skips_free_days_and_deadlines_lose_their_alarm_when_done() {
        let semester = Semester {
            id: "s".into(),
            start: "2026-10-05".into(),
            end: "2026-11-02".into(),
            holidays: vec![Holiday { name: "x".into(), date: "2026-10-13".into() }],
            breaks: vec![Break { start: "2026-10-19".into(), end: "2026-10-25".into(), ..Default::default() }],
            ..Default::default()
        };
        let module = Module { id: "m".into(), code: "CA1".into(), name: "Computer Architecture".into(), semester: "s".into(), sessions: vec![Session { id: "v".into(), title: "VL".into(), weekday: 2, start: "10:15".into(), end: "11:45".into(), location: "HS 1".into() }], ..Default::default() };
        let other = Module { id: "o".into(), semester: "old".into(), sessions: module.sessions.clone(), ..Default::default() };
        let config = StudyConfig { modules: vec![module, other], semesters: vec![semester.clone()], ..Default::default() };
        let due = Local.with_ymd_and_hms(2026, 10, 30, 23, 59, 0).unwrap().with_timezone(&Utc);
        let manual = |id: &str, title: &str| ManualAssignment { id: id.into(), module: "m".into(), title: title.into(), due, ..Default::default() };
        let state = StudyState { manual: vec![manual("1", "Blatt 1"), manual("2", "Blatt 2")], ..Default::default() };
        let all = crate::core::schedule::assignments(&config, &state, std::path::Path::new("root"));
        let w = wanted(&[Deadline { assignment: &all[0], done: false }, Deadline { assignment: &all[1], done: true }], &config, &semester, Some(3));
        let days: Vec<&str> = w.keys().filter_map(|k| k.strip_prefix("s:m:")).collect();
        assert_eq!(days, vec!["2026-10-06T10:15-11:45:VL:HS 1", "2026-10-27T10:15-11:45:VL:HS 1"]);
        assert!(!w.keys().any(|k| k.starts_with("s:o:")));
        let lecture = &w["s:m:2026-10-06T10:15-11:45:VL:HS 1"];
        assert_eq!((lecture.title.as_str(), lecture.location.as_str(), lecture.end - lecture.start), ("CA1 VL", "HS 1", 90 * 60));
        let (first, second) = (&w[&format!("a:{}", all[0].id)], &w[&format!("a:{}", all[1].id)]);
        assert_eq!(first.title, format!("CA1 · {}", all[0].title));
        assert_eq!((first.start, first.end), (due.timestamp() - 30 * 60, due.timestamp()));
        assert_eq!(first.alarm, Some(150));
        assert!(second.title.starts_with("✓ CA1 · "));
        assert_eq!(second.alarm, None);
        for hours in [None, Some(0)] {
            let quiet = wanted(&[Deadline { assignment: &all[0], done: false }], &config, &semester, hours);
            assert_eq!(quiet[&format!("a:{}", all[0].id)].alarm, None);
        }
    }
}
