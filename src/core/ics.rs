use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;

use super::model::{Assignment, Kind, Module, StudyConfig};
use super::day;

/// An event read from an iCalendar file, e.g. a Moodle calendar export.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarEvent {
    pub uid: String,
    pub summary: String,
    pub course: String,
    pub due: DateTime<Utc>,
}

impl CalendarEvent {
    /// Summary without "ist fällig" / "is due" style suffixes.
    pub fn title(&self) -> String {
        static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
            [
                r"(?i)\s+(ist|sind)\s+fällig\.?$",
                r"(?i)\s+is\s+due\.?$",
                r"(?i)\s+soll(en)?\s+abgeschlossen\s+sein\.?$",
                r"(?i)\s+(öffnet|opens|schließt|closes)\.?$",
                r"(?i)^(abgabe|fällig|due)\s*:\s*",
            ]
            .iter()
            .map(|p| Regex::new(p).unwrap())
            .collect()
        });
        PATTERNS.iter().fold(self.summary.clone(), |t, re| re.replace(&t, "").into_owned()).trim().to_string()
    }

    pub fn guessed_kind(&self) -> Kind {
        let s = self.summary.to_lowercase();
        if ["klausur", "prüfung", "exam", "test", "testat"].iter().any(|w| s.contains(w)) {
            Kind::Exam
        } else if ["fällig", "due", "abgabe", "einreich", "submission", "assignment", "hausübung", "hausarbeit", "upload"].iter().any(|w| s.contains(w)) {
            Kind::Submission
        } else {
            Kind::Task
        }
    }
}

pub fn parse(text: &str) -> Vec<CalendarEvent> {
    let unfolded = text.replace("\r\n", "\n").replace("\n ", "").replace("\n\t", "");
    let mut events = Vec::new();
    let mut fields: Option<HashMap<String, (String, String)>> = None;
    for line in unfolded.split('\n').filter(|l| !l.is_empty()) {
        match line {
            "BEGIN:VEVENT" | "BEGIN:VTODO" => {
                fields = Some(HashMap::new());
                continue;
            }
            "END:VEVENT" | "END:VTODO" => {
                if let Some(f) = fields.take() {
                    events.extend(occurrences(&f));
                }
                continue;
            }
            _ => {}
        }
        let (Some(f), Some(colon)) = (fields.as_mut(), line.find(':')) else { continue };
        let (head, value) = (&line[..colon], &line[colon + 1..]);
        let mut parts = head.splitn(2, ';');
        let Some(name) = parts.next().filter(|n| !n.is_empty()) else { continue };
        f.entry(name.to_uppercase()).or_insert_with(|| (parts.next().unwrap_or("").to_string(), unescape(value)));
    }
    events.sort_by_key(|e| e.due);
    events
}

const MAX_OCCURRENCES: usize = 366;
const OPEN_ENDED_OCCURRENCES: usize = 60;
const MAX_INTERVAL: i64 = 1000;

#[derive(Clone, Copy)]
enum Zone {
    Utc,
    Named(Tz),
    Workspace,
}

#[derive(Clone, Copy)]
struct Stamp {
    local: NaiveDateTime,
    zone: Zone,
    all_day: bool,
}

struct Times {
    start: Stamp,
    due: Stamp,
}

fn stamp(value: &str, params: &str) -> Option<Stamp> {
    static TZID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"TZID=([^;:]+)").unwrap());
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 8 {
        return None;
    }
    let d = NaiveDate::from_ymd_opt(digits[..4].parse().ok()?, digits[4..6].parse().ok()?, digits[6..8].parse().ok()?)?;
    let all_day = digits.len() < 12;
    let t = if all_day { NaiveTime::from_hms_opt(23, 59, 0)? } else { NaiveTime::from_hms_opt(digits[8..10].parse().ok()?, digits[10..12].parse().ok()?, 0)? };
    let zone = if value.ends_with('Z') {
        Zone::Utc
    } else {
        TZID.captures(params).and_then(|c| c[1].parse::<Tz>().ok()).map_or(Zone::Workspace, Zone::Named)
    };
    Some(Stamp { local: d.and_time(t), zone, all_day })
}

fn instant(local: NaiveDateTime, zone: Zone) -> Option<DateTime<Utc>> {
    match zone {
        Zone::Utc => Some(Utc.from_utc_datetime(&local)),
        Zone::Named(z) => day::resolve(&z, local).map(|x| x.with_timezone(&Utc)),
        Zone::Workspace => day::at(local.date(), local.time()).map(|x| x.with_timezone(&Utc)),
    }
}

fn times(f: &HashMap<String, (String, String)>) -> Option<Times> {
    let at = |name: &str| f.get(name).and_then(|r| stamp(&r.1, &r.0));
    let start = at("DTSTART");
    let mut due = at("DUE").or_else(|| at("DTEND")).or(start)?;
    if f.get("DUE").is_none() && due.all_day && start.is_some_and(|s| s.all_day && s.local.date() < due.local.date()) {
        due.local -= Duration::days(1);
    }
    Some(Times { start: start.unwrap_or(due), due })
}

fn event_at(f: &HashMap<String, (String, String)>, due: DateTime<Utc>) -> Option<CalendarEvent> {
    let summary = f.get("SUMMARY").map(|v| v.1.clone()).filter(|s| !s.is_empty())?;
    Some(CalendarEvent {
        uid: f.get("UID").map(|v| v.1.clone()).unwrap_or_else(|| format!("{summary}-{}", due.timestamp())),
        summary,
        course: f.get("CATEGORIES").map(|v| v.1.clone()).unwrap_or_default(),
        due,
    })
}

fn event(f: &HashMap<String, (String, String)>) -> Option<CalendarEvent> {
    let t = times(f)?;
    event_at(f, instant(t.due.local, t.due.zone)?)
}

fn weekday_offset(code: &str) -> Option<i64> {
    let letters: String = code.chars().filter(char::is_ascii_alphabetic).collect();
    ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|d| *d == letters).map(|i| i as i64)
}

/// An event with all its dates: `RRULE` by day or week with `BYDAY`, `COUNT` and `UNTIL`, what a course calendar uses.
/// Every date is a wall clock time in the event's own zone, so a lecture keeps its hour across the clock change. A cancelled event has none.
fn occurrences(f: &HashMap<String, (String, String)>) -> Vec<CalendarEvent> {
    if f.get("STATUS").is_some_and(|s| s.1.eq_ignore_ascii_case("CANCELLED")) {
        return Vec::new();
    }
    let Some(t) = times(f) else { return Vec::new() };
    let Some(first) = instant(t.due.local, t.due.zone).and_then(|due| event_at(f, due)) else { return Vec::new() };
    let Some(rule) = f.get("RRULE").map(|r| r.1.to_uppercase()) else { return vec![first] };
    let part = |name: &str| rule.split(';').find_map(|p| p.strip_prefix(&format!("{name}=")).map(str::to_string));
    let weekly = match part("FREQ").as_deref() {
        Some("DAILY") => false,
        Some("WEEKLY") => true,
        _ => return vec![first],
    };
    let interval = part("INTERVAL").and_then(|i| i.parse::<i64>().ok()).unwrap_or(1).clamp(1, MAX_INTERVAL);
    let until = part("UNTIL").and_then(|u| date(&u, ""));
    let count = part("COUNT").and_then(|c| c.parse::<usize>().ok()).map_or(if until.is_some() { MAX_OCCURRENCES } else { OPEN_ENDED_OCCURRENCES }, |c| c.min(MAX_OCCURRENCES));
    let skipped: Vec<i64> = f.get("EXDATE").map_or(Vec::new(), |e| e.1.split(',').filter_map(|d| date(d, &e.0)).map(|d| d.timestamp()).collect());
    let first_day = t.start.local.date();
    let days: Vec<NaiveDate> = if weekly {
        let mut wanted: Vec<i64> = part("BYDAY").map_or(Vec::new(), |b| b.split(',').filter_map(weekday_offset).collect());
        if wanted.is_empty() {
            wanted.push(i64::from(first_day.weekday().num_days_from_monday()));
        }
        wanted.sort_unstable();
        wanted.dedup();
        let monday = day::monday(first_day);
        (0..MAX_OCCURRENCES as i64)
            .flat_map(|week| wanted.iter().map(move |o| (week, *o)))
            .filter_map(|(week, o)| monday.checked_add_signed(Duration::days(week * 7 * interval + o)))
            .filter(|d| *d >= first_day)
            .collect()
    } else {
        (0..MAX_OCCURRENCES as i64).filter_map(|n| first_day.checked_add_signed(Duration::days(n * interval))).collect()
    };
    let span = (t.due.local.date() - first_day).num_days();
    let mut out = Vec::new();
    for d in days {
        let Some(start) = instant(d.and_time(t.start.local.time()), t.start.zone) else { continue };
        if until.is_some_and(|u| start > u) {
            break;
        }
        if out.len() >= count {
            break;
        }
        let Some(due) = d.checked_add_signed(Duration::days(span)).and_then(|dd| instant(dd.and_time(t.due.local.time()), t.due.zone)) else { continue };
        out.push((start, due));
    }
    out.into_iter()
        .filter(|(start, due)| !skipped.contains(&start.timestamp()) && !skipped.contains(&due.timestamp()))
        .map(|(_, due)| CalendarEvent { uid: format!("{}#{}", first.uid, due.timestamp()), due, ..first.clone() })
        .collect()
}

pub fn date(value: &str, params: &str) -> Option<DateTime<Utc>> {
    let s = stamp(value, params)?;
    instant(s.local, s.zone)
}

fn unescape(s: &str) -> String {
    s.replace("\\n", " ").replace("\\N", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// Best module for a calendar course name: code as a word, then name contained either way.
pub fn module_for<'a>(course: &str, modules: &'a [Module]) -> Option<&'a Module> {
    let c = course.to_lowercase();
    if c.is_empty() {
        return None;
    }
    modules
        .iter()
        .find(|m| Regex::new(&format!(r"\b{}\b", regex::escape(&m.code.to_lowercase()))).is_ok_and(|r| r.is_match(&c)))
        .or_else(|| modules.iter().find(|m| c.contains(&m.name.to_lowercase()) || m.name.to_lowercase().contains(&c)))
}

/// iCalendar export of deadlines with a display alarm `alarm_hours` before each.
pub fn export(assignments: &[&Assignment], config: &StudyConfig, alarm_hours: i64) -> String {
    let stamp = utc(Utc::now());
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//Schrumbi//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:PUBLISH".into(),
        format!("X-WR-CALNAME:{}", escape(&config.name)),
        "REFRESH-INTERVAL;VALUE=DURATION:PT1H".into(),
        "X-PUBLISHED-TTL:PT1H".into(),
    ];
    for a in assignments {
        let title = [config.module(&a.module_id).map(|m| m.code.clone()), Some(a.title.clone())].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let due = a.due.with_timezone(&Utc);
        lines.extend([
            "BEGIN:VEVENT".to_string(),
            format!("UID:{}@schrumbi.local", a.id),
            format!("DTSTAMP:{stamp}"),
            format!("DTSTART:{}", utc(due - Duration::minutes(30))),
            format!("DTEND:{}", utc(due)),
            format!("SUMMARY:{}", escape(&title)),
            format!("DESCRIPTION:{}", escape(&config.module(&a.module_id).map_or(String::new(), |m| m.name.clone()))),
            "BEGIN:VALARM".into(),
            "ACTION:DISPLAY".into(),
            format!("DESCRIPTION:{}", escape(&title)),
            format!("TRIGGER:-PT{}H", alarm_hours.max(0)),
            "END:VALARM".into(),
            "END:VEVENT".into(),
        ]);
    }
    for semester in &config.semesters {
        for module in config.modules.iter().filter(|m| m.semester == semester.id) {
            lines.extend(session_events(module, semester, &stamp));
        }
    }
    lines.push("END:VCALENDAR".into());
    lines.iter().map(|l| fold(l)).collect::<Vec<_>>().join("\r\n") + "\r\n"
}

/// A module's weekly sessions for its semester, at the same wall clock time all along, without breaks and holidays.
fn session_events(module: &Module, semester: &super::model::Semester, stamp: &str) -> Vec<String> {
    let (Some(start), Some(end)) = (day::parse(&semester.start), day::parse(&semester.end)) else { return Vec::new() };
    let mut out = Vec::new();
    for session in &module.sessions {
        let offset = (session.weekday as i64 - start.weekday().number_from_monday() as i64).rem_euclid(7);
        let first = start + Duration::days(offset);
        if first > end {
            continue;
        }
        let clock = |t: &str| t.replace(':', "");
        let free: Vec<String> = std::iter::successors(Some(first), |d| Some(*d + Duration::days(7)))
            .take_while(|d| *d <= end)
            .filter(|d| {
                let text = d.format("%Y-%m-%d").to_string();
                semester.holidays.iter().any(|h| h.date == text) || semester.breaks.iter().any(|b| b.start <= text && text <= b.end)
            })
            .map(|d| format!("{}T{}00", d.format("%Y%m%d"), clock(&session.start)))
            .collect();
        let title = if session.title.is_empty() { module.code.clone() } else { format!("{} {}", module.code, session.title) };
        out.extend([
            "BEGIN:VEVENT".to_string(),
            format!("UID:{}.{}@schrumbi.local", module.id, session.id),
            format!("DTSTAMP:{stamp}"),
            format!("DTSTART:{}T{}00", first.format("%Y%m%d"), clock(&session.start)),
            format!("DTEND:{}T{}00", first.format("%Y%m%d"), clock(&session.end)),
            format!("RRULE:FREQ=WEEKLY;UNTIL={}T235959", end.format("%Y%m%d")),
        ]);
        if !free.is_empty() {
            out.push(format!("EXDATE:{}", free.join(",")));
        }
        out.push(format!("SUMMARY:{}", escape(&title)));
        if !session.location.is_empty() {
            out.push(format!("LOCATION:{}", escape(&session.location)));
        }
        out.push("END:VEVENT".into());
    }
    out
}

/// Lines longer than 75 bytes continue on the next one; some calendars insist on it.
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let mut used = 0;
    for c in line.chars() {
        if used + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            used = 1;
        }
        out.push(c);
        used += c.len_utf8();
    }
    out
}

fn utc(d: DateTime<Utc>) -> String {
    d.format("%Y%m%dT%H%M%SZ").to_string()
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n")
}

pub fn local(d: DateTime<Utc>) -> DateTime<Local> {
    d.with_timezone(&Local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_and_cancellations_are_read() {
        let ics = "BEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Blatt\r\nDTSTART:20261102T170000Z\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nEXDATE:20261109T170000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Weg\r\nSTATUS:CANCELLED\r\nDTSTART:20261102T170000Z\r\nEND:VEVENT\r\n";
        let found = parse(ics);
        assert_eq!(found.len(), 2);
        assert_ne!(found[0].uid, found[1].uid);
        assert_eq!((found[1].due - found[0].due).num_days(), 14);
    }

    #[test]
    fn long_lines_fold_and_read_back() {
        let line = format!("SUMMARY:{}", "ä".repeat(60));
        let folded = fold(&line);
        assert!(folded.split("\r\n").all(|l| l.len() <= 75));
        assert_eq!(folded.replace("\r\n ", ""), line);
    }

    #[test]
    fn sessions_become_weekly_events_without_free_days() {
        let mut config = StudyConfig::default();
        let semester = crate::core::model::Semester { id: "s".into(), start: "2026-10-05".into(), end: "2026-11-30".into(), holidays: vec![crate::core::model::Holiday { name: "x".into(), date: "2026-10-13".into() }], ..Default::default() };
        let module = Module { id: "m".into(), code: "CA1".into(), semester: "s".into(), sessions: vec![crate::core::model::Session { id: "v".into(), title: "VL".into(), weekday: 2, start: "10:15".into(), end: "11:45".into(), location: "HS, 1".into() }], ..Default::default() };
        config.semesters.push(semester);
        config.modules.push(module);
        let out = export(&[], &config, 24).replace("\r\n ", "");
        assert!(out.contains("DTSTART:20261006T101500"), "{out}");
        assert!(out.contains("RRULE:FREQ=WEEKLY;UNTIL=20261130T235959"));
        assert!(out.contains("EXDATE:20261013T101500"));
        assert!(out.contains("LOCATION:HS\\, 1"));
    }

    #[test]
    fn parses_moodle_events() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:42\r\nSUMMARY:Hausübung 3 ist fällig\r\nDTSTART:20261102T170000Z\r\nCATEGORIES:P1 Projekt\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let e = parse(ics);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].title(), "Hausübung 3");
        assert_eq!(e[0].guessed_kind(), Kind::Submission);
        assert_eq!(e[0].due, Utc.with_ymd_and_hms(2026, 11, 2, 17, 0, 0).unwrap());
        let m = Module { code: "P1".into(), ..Default::default() };
        assert!(module_for(&e[0].course, std::slice::from_ref(&m)).is_some());
    }

    fn event_text(lines: &[&str]) -> String {
        format!("BEGIN:VEVENT\r\nUID:v\r\nSUMMARY:VL\r\n{}\r\nEND:VEVENT\r\n", lines.join("\r\n"))
    }

    fn berlin(due: DateTime<Utc>) -> NaiveDateTime {
        due.with_timezone(&"Europe/Berlin".parse::<Tz>().unwrap()).naive_local()
    }

    fn wall(month: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, month, d).unwrap().and_hms_opt(h, 0, 0).unwrap()
    }

    #[test]
    fn weekly_lectures_keep_their_wall_clock_hour_across_the_clock_change() {
        let found = parse(&event_text(&["DTSTART;TZID=Europe/Berlin:20261019T100000", "DTEND;TZID=Europe/Berlin:20261019T100000", "RRULE:FREQ=WEEKLY;COUNT=3"]));
        assert_eq!(found.iter().map(|e| berlin(e.due)).collect::<Vec<_>>(), vec![wall(10, 19, 10), wall(10, 26, 10), wall(11, 2, 10)]);
        assert_eq!(found[0].due, Utc.with_ymd_and_hms(2026, 10, 19, 8, 0, 0).unwrap());
        assert_eq!(found[1].due, Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap());
    }

    #[test]
    fn byday_lists_every_weekday_of_each_week() {
        let found = parse(&event_text(&["DTSTART;TZID=Europe/Berlin:20261019T100000", "RRULE:FREQ=WEEKLY;BYDAY=WE,MO;COUNT=4"]));
        assert_eq!(found.iter().map(|e| berlin(e.due)).collect::<Vec<_>>(), vec![wall(10, 19, 10), wall(10, 21, 10), wall(10, 26, 10), wall(10, 28, 10)]);
    }

    #[test]
    fn byday_before_the_first_day_of_the_series_is_skipped() {
        let found = parse(&event_text(&["DTSTART;TZID=Europe/Berlin:20261021T100000", "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=3"]));
        assert_eq!(found.iter().map(|e| berlin(e.due)).collect::<Vec<_>>(), vec![wall(10, 21, 10), wall(10, 26, 10), wall(10, 28, 10)]);
    }

    #[test]
    fn count_up_to_366_and_until_are_honoured() {
        let rule = |r: &str| parse(&event_text(&["DTSTART:20261019T100000Z", &format!("RRULE:{r}")]));
        assert_eq!(rule("FREQ=DAILY;COUNT=100").len(), 100);
        assert_eq!(rule("FREQ=DAILY;COUNT=5000").len(), 366);
        assert_eq!(rule("FREQ=WEEKLY").len(), 60);
        assert_eq!(rule("FREQ=WEEKLY;UNTIL=20261130T235959Z").len(), 7);
        assert_eq!(rule("FREQ=DAILY;INTERVAL=99999999999999999;COUNT=3").len(), 3);
    }

    #[test]
    fn all_day_events_end_on_the_last_day() {
        let found = |lines: &[&str]| parse(&event_text(lines)).iter().map(|e| day::local_date(local(e.due))).collect::<Vec<_>>();
        let d = |m, dd| NaiveDate::from_ymd_opt(2026, m, dd).unwrap();
        assert_eq!(found(&["DTSTART;VALUE=DATE:20261224", "DTEND;VALUE=DATE:20261227"]), vec![d(12, 26)]);
        assert_eq!(found(&["DTSTART;VALUE=DATE:20261225", "DTEND;VALUE=DATE:20261226"]), vec![d(12, 25)]);
        assert_eq!(found(&["DTSTART;VALUE=DATE:20261224"]), vec![d(12, 24)]);
    }

    #[test]
    fn exdates_remove_the_start_of_an_occurrence() {
        let found = parse(&event_text(&["DTSTART;TZID=Europe/Berlin:20261019T101500", "DTEND;TZID=Europe/Berlin:20261019T114500", "RRULE:FREQ=WEEKLY;COUNT=3", "EXDATE;TZID=Europe/Berlin:20261026T101500"]));
        assert_eq!(found.len(), 2);
        assert_eq!(berlin(found[1].due), wall(11, 2, 11) + Duration::minutes(45));
    }
}
