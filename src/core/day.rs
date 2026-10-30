use std::sync::{LazyLock, RwLock};

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use chrono_tz::Tz;
use regex::Regex;

static ZONE: RwLock<Option<Tz>> = RwLock::new(None);

/// Sets the workspace time zone; an empty or unknown name uses the system zone.
pub fn set_time_zone(name: &str) {
    *ZONE.write().unwrap() = name.parse::<Tz>().ok();
}

fn zone() -> Option<Tz> {
    *ZONE.read().unwrap()
}

/// Wall-clock date and time of an instant in the workspace zone.
pub fn wall(t: DateTime<Local>) -> NaiveDateTime {
    match zone() {
        Some(z) => t.with_timezone(&z).naive_local(),
        None => t.naive_local(),
    }
}

/// Calendar day of an instant in the workspace zone.
pub fn local_date(t: DateTime<Local>) -> NaiveDate {
    wall(t).date()
}

const WEEKDAYS: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];
const WEEKDAYS_SHORT: [&str; 7] = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
const MONTHS: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
const WEEKDAYS_EN: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const WEEKDAYS_SHORT_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn en() -> bool {
    super::i18n::lang() == super::i18n::Lang::En
}

/// Parses `yyyy-MM-dd`.
pub fn parse(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

pub fn format(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Parses `HH:mm`, falling back to 23:59 like the Swift app.
pub fn time(s: &str) -> NaiveTime {
    let mut parts = s.split(':').filter_map(|p| p.parse::<u32>().ok());
    let h = parts.next().unwrap_or(23);
    let m = parts.next().unwrap_or(59);
    NaiveTime::from_hms_opt(h, m, 0).unwrap_or(NaiveTime::from_hms_opt(23, 59, 0).unwrap())
}

/// The instant of wall-clock `t` on day `d` in the workspace zone.
pub fn at(d: NaiveDate, t: NaiveTime) -> Option<DateTime<Local>> {
    match zone() {
        Some(z) => z.from_local_datetime(&d.and_time(t)).earliest().map(|x| x.with_timezone(&Local)),
        None => Local.from_local_datetime(&d.and_time(t)).earliest(),
    }
}

pub fn monday(d: NaiveDate) -> NaiveDate {
    d - Days::new(d.weekday().num_days_from_monday() as u64)
}

pub fn iso_week(d: NaiveDate) -> u32 {
    d.iso_week().week()
}

pub fn yymmdd(d: NaiveDate) -> String {
    d.format("%y%m%d").to_string()
}

pub fn is_weekend(d: NaiveDate) -> bool {
    d.weekday().number_from_monday() >= 6
}

static ISO_IN_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\D)(20\d{2})-(\d{2})-(\d{2})(?:\D|$)").unwrap());
static SIX_DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").unwrap());

/// Parses a `yyMMdd` or `yyyy-MM-dd` date embedded in a file name.
pub fn date_in_name(name: &str) -> Option<NaiveDate> {
    if let Some(c) = ISO_IN_NAME.captures(name) {
        return NaiveDate::from_ymd_opt(c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?);
    }
    SIX_DIGITS.find_iter(name).filter(|m| m.as_str().len() == 6).collect::<Vec<_>>().into_iter().rev().find_map(|m| {
        let s = m.as_str();
        let (y, mo, d) = (s[0..2].parse::<i32>().ok()?, s[2..4].parse::<u32>().ok()?, s[4..6].parse::<u32>().ok()?);
        if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
            return None;
        }
        NaiveDate::from_ymd_opt(2000 + y, mo, d)
    })
}

/// "Donnerstag, 29. Oktober" / "Thursday, October 29".
pub fn long(d: NaiveDate) -> String {
    let w = d.weekday().num_days_from_monday() as usize;
    if en() { format!("{}, {} {}", WEEKDAYS_EN[w], MONTHS_EN[d.month0() as usize], d.day()) } else { format!("{}, {}. {}", WEEKDAYS[w], d.day(), MONTHS[d.month0() as usize]) }
}

/// "Donnerstag".
pub fn weekday_long(d: NaiveDate) -> String {
    let w = d.weekday().num_days_from_monday() as usize;
    (if en() { WEEKDAYS_EN[w] } else { WEEKDAYS[w] }).to_string()
}

/// "Do" / "Thu".
pub fn weekday_short(d: NaiveDate) -> &'static str {
    let w = d.weekday().num_days_from_monday() as usize;
    if en() { WEEKDAYS_SHORT_EN[w] } else { WEEKDAYS_SHORT[w] }
}

/// Weekday names starting Monday, in the app language.
pub fn weekday_names() -> [&'static str; 7] {
    if en() { WEEKDAYS_EN } else { WEEKDAYS }
}

/// "1.11." / "11/1".
pub fn short_date(d: NaiveDate) -> String {
    if en() { format!("{}/{}", d.month(), d.day()) } else { format!("{}.{}.", d.day(), d.month()) }
}

pub fn hm(t: DateTime<Local>) -> String {
    let w = wall(t);
    if en() {
        let h = w.hour() % 12;
        format!("{}:{:02} {}", if h == 0 { 12 } else { h }, w.minute(), if w.hour() < 12 { "AM" } else { "PM" })
    } else {
        format!("{:02}:{:02}", w.hour(), w.minute())
    }
}

fn relative_day(d: NaiveDate, today: NaiveDate) -> Option<String> {
    match (d - today).num_days() {
        0 => Some(super::i18n::tr("Heute")),
        1 => Some(super::i18n::tr("Morgen")),
        -1 => Some(super::i18n::tr("Gestern")),
        _ => None,
    }
}

fn day_only(d: NaiveDate) -> String {
    if en() { format!("{}, {}", weekday_short(d), short_date(d)) } else { format!("{}. {}", weekday_short(d), short_date(d)) }
}

/// "Morgen, 18:00" or "So. 1.11., 18:00".
pub fn due(t: DateTime<Local>, now: DateTime<Local>) -> String {
    let d = local_date(t);
    match relative_day(d, local_date(now)) {
        Some(r) => format!("{r}, {}", hm(t)),
        None => format!("{}, {}", day_only(d), hm(t)),
    }
}

/// "Heute" or "So. 1.11.".
pub fn day_label(t: DateTime<Local>, now: DateTime<Local>) -> String {
    let d = local_date(t);
    relative_day(d, local_date(now)).unwrap_or_else(|| day_only(d))
}

/// "Heute · So. 1.11." or the long date.
pub fn day_header(d: NaiveDate, today: NaiveDate) -> String {
    match relative_day(d, today) {
        Some(r) => format!("{r} · {}", day_only(d)),
        None => long(d),
    }
}

/// "in 2 Tagen", "vor 3 Std.".
pub fn countdown(to: DateTime<Local>, now: DateTime<Local>) -> String {
    let secs = (to - now).num_seconds();
    let span = duration(secs.unsigned_abs());
    super::i18n::trf(if secs < 0 { "vor %@" } else { "in %@" }, &[&span])
}

/// "3 Tage", "5 Std.", "überfällig".
pub fn compact(to: DateTime<Local>, now: DateTime<Local>) -> String {
    use super::i18n::{tr, trf};
    let s = (to - now).num_seconds();
    match s {
        ..0 => tr("überfällig"),
        172_800.. => trf("%lld Tage", &[&(s / 86400)]),
        86_400.. => tr("1 Tag"),
        3600.. => trf("%lld Std.", &[&(s / 3600)]),
        _ => trf("%lld Min.", &[&(s / 60).max(1)]),
    }
}

/// "2T", "5h", "12m" for the tray.
pub fn menu_bar(to: DateTime<Local>, now: DateTime<Local>) -> String {
    let s = (to - now).num_seconds();
    match s {
        86_400.. => super::i18n::trf("%lldT", &[&(s / 86400)]),
        3600.. => format!("{}h", s / 3600),
        _ => format!("{}m", (s / 60).max(0)),
    }
}

fn duration(s: u64) -> String {
    use super::i18n::{tr, trf};
    let (days, hours, minutes) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60);
    match (days, hours) {
        (2.., _) => trf("%lld Tagen", &[&days]),
        (1, 0) => tr("1 Tag"),
        (1, h) => trf("1 Tag, %lld Std.", &[&h]),
        (0, h) if h > 0 && minutes > 0 && h < 3 => trf("%lld Std. %lld Min.", &[&h, &minutes]),
        (0, h) if h > 0 => trf("%lld Std.", &[&h]),
        _ => trf("%lld Min.", &[&minutes.max(1)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dates_in_names() {
        assert_eq!(date_in_name("P1_26_Radar_Muster-M_261028.md"), NaiveDate::from_ymd_opt(2026, 10, 28));
        assert_eq!(date_in_name("notes 2026-11-02.pdf"), NaiveDate::from_ymd_opt(2026, 11, 2));
        assert_eq!(date_in_name("P1_26_x.pdf"), None);
    }

    #[test]
    fn formats_long_dates() {
        assert_eq!(long(NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()), "Donnerstag, 29. Oktober");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(duration(2 * 86400 + 5), "2 Tagen");
        assert_eq!(duration(86400 + 3 * 3600), "1 Tag, 3 Std.");
        assert_eq!(duration(3600 + 120), "1 Std. 2 Min.");
        assert_eq!(duration(10), "1 Min.");
    }
}
