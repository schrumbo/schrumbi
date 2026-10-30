use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone, Utc};
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
                if let Some(e) = fields.take().as_ref().and_then(event) {
                    events.push(e);
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

fn event(f: &HashMap<String, (String, String)>) -> Option<CalendarEvent> {
    let summary = f.get("SUMMARY").map(|v| v.1.clone()).filter(|s| !s.is_empty())?;
    let raw = f.get("DUE").or_else(|| f.get("DTEND")).or_else(|| f.get("DTSTART"))?;
    let due = date(&raw.1, &raw.0)?;
    Some(CalendarEvent {
        uid: f.get("UID").map(|v| v.1.clone()).unwrap_or_else(|| format!("{summary}-{}", due.timestamp())),
        summary,
        course: f.get("CATEGORIES").map(|v| v.1.clone()).unwrap_or_default(),
        due,
    })
}

pub fn date(value: &str, params: &str) -> Option<DateTime<Utc>> {
    static TZID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"TZID=([^;:]+)").unwrap());
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 8 {
        return None;
    }
    let d = NaiveDate::from_ymd_opt(digits[..4].parse().ok()?, digits[4..6].parse().ok()?, digits[6..8].parse().ok()?)?;
    let t = if digits.len() >= 12 { NaiveTime::from_hms_opt(digits[8..10].parse().ok()?, digits[10..12].parse().ok()?, 0)? } else { NaiveTime::from_hms_opt(23, 59, 0)? };
    let naive = d.and_time(t);
    if value.ends_with('Z') {
        return Some(Utc.from_utc_datetime(&naive));
    }
    if let Some(z) = TZID.captures(params).and_then(|c| c[1].parse::<Tz>().ok()) {
        return z.from_local_datetime(&naive).earliest().map(|x| x.with_timezone(&Utc));
    }
    day::at(d, t).map(|x| x.with_timezone(&Utc))
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
        format!("X-WR-CALNAME:{} Abgaben", escape(&config.name)),
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
            format!("DESCRIPTION:{}", escape(&a.folder.to_string_lossy())),
            "BEGIN:VALARM".into(),
            "ACTION:DISPLAY".into(),
            format!("DESCRIPTION:{}", escape(&title)),
            format!("TRIGGER:-PT{}H", alarm_hours.max(0)),
            "END:VALARM".into(),
            "END:VEVENT".into(),
        ]);
    }
    lines.push("END:VCALENDAR".into());
    lines.join("\r\n") + "\r\n"
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
}
