use chrono::{DateTime, Datelike, Local, Timelike};

const MONTHS_DE: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
const MONTHS_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS_DE: [&str; 7] = ["Sonntag", "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag"];
const DAYS_EN: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

fn short(name: &str, n: usize) -> String {
    name.chars().take(n).collect()
}

/// Formats a date with a Moment.js pattern, as Obsidian's daily notes and templates use them. `[text]` stays literal.
pub fn format(pattern: &str, at: DateTime<Local>, english: bool) -> String {
    let (months, days) = if english { (MONTHS_EN, DAYS_EN) } else { (MONTHS_DE, DAYS_DE) };
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' {
            let close = chars[i..].iter().position(|x| *x == ']');
            if let Some(end) = close {
                out.extend(&chars[i + 1..i + end]);
                i += end + 1;
                continue;
            }
        }
        let run = chars[i..].iter().take_while(|x| **x == c).count();
        let hour12 = match at.hour() % 12 {
            0 => 12,
            h => h,
        };
        let weekday = at.weekday().num_days_from_sunday() as usize;
        let month = at.month0() as usize;
        let piece: Option<String> = match (c, run) {
            ('Y', 4) => Some(format!("{:04}", at.year())),
            ('Y', 2) => Some(format!("{:02}", at.year() % 100)),
            ('M', 4) => Some(months[month].to_string()),
            ('M', 3) => Some(short(months[month], 3)),
            ('M', 2) => Some(format!("{:02}", at.month())),
            ('M', 1) => Some(at.month().to_string()),
            ('D', 4) => Some(format!("{:03}", at.ordinal())),
            ('D', 3) => Some(at.ordinal().to_string()),
            ('D', 2) => Some(format!("{:02}", at.day())),
            ('D', 1) => Some(at.day().to_string()),
            ('d', 4) => Some(days[weekday].to_string()),
            ('d', 3) => Some(short(days[weekday], 3)),
            ('d', 2) => Some(short(days[weekday], 2)),
            ('d', 1) => Some(weekday.to_string()),
            ('H', 2) => Some(format!("{:02}", at.hour())),
            ('H', 1) => Some(at.hour().to_string()),
            ('h', 2) => Some(format!("{hour12:02}")),
            ('h', 1) => Some(hour12.to_string()),
            ('m', 2) => Some(format!("{:02}", at.minute())),
            ('m', 1) => Some(at.minute().to_string()),
            ('s', 2) => Some(format!("{:02}", at.second())),
            ('s', 1) => Some(at.second().to_string()),
            ('A', 1) => Some(if at.hour() < 12 { "AM" } else { "PM" }.to_string()),
            ('a', 1) => Some(if at.hour() < 12 { "am" } else { "pm" }.to_string()),
            ('W', 2) => Some(format!("{:02}", at.iso_week().week())),
            ('W', 1) => Some(at.iso_week().week().to_string()),
            ('G', 4) => Some(format!("{:04}", at.iso_week().year())),
            ('Q', 1) => Some(((at.month0() / 3) + 1).to_string()),
            ('X', 1) => Some(at.timestamp().to_string()),
            _ => None,
        };
        match piece {
            Some(p) => {
                out.push_str(&p);
                i += run;
            }
            None => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Fills `{{title}}`, `{{date}}`, `{{time}}` and `{{date:FORMAT}}` in a template.
pub fn fill_template(template: &str, title: &str, at: DateTime<Local>, date_format: &str, time_format: &str, english: bool) -> String {
    let date = if date_format.is_empty() { "YYYY-MM-DD" } else { date_format };
    let time = if time_format.is_empty() { "HH:mm" } else { time_format };
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = rest[start + 2..start + end].trim();
        let (name, custom) = inner.split_once(':').map_or((inner, None), |(n, f)| (n.trim(), Some(f.trim())));
        match name.to_lowercase().as_str() {
            "title" => out.push_str(title),
            "date" => out.push_str(&format(custom.unwrap_or(date), at, english)),
            "time" => out.push_str(&format(custom.unwrap_or(time), at, english)),
            _ => out.push_str(&rest[start..start + end + 2]),
        }
        rest = &rest[start + end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 3, 5, 14, 7, 9).unwrap()
    }

    #[test]
    fn formats_common_patterns() {
        assert_eq!(format("YYYY-MM-DD", at(), false), "2026-03-05");
        assert_eq!(format("YYYY/MMMM/YYYY-MMM-DD", at(), true), "2026/March/2026-Mar-05");
        assert_eq!(format("dddd, D. MMMM YY", at(), false), "Donnerstag, 5. März 26");
        assert_eq!(format("HH:mm:ss", at(), false), "14:07:09");
        assert_eq!(format("h:mm A", at(), false), "2:07 PM");
        assert_eq!(format("[Woche] WW", at(), false), "Woche 10");
        assert_eq!(format("Qx", at(), false), "1x");
    }

    #[test]
    fn templates_fill_their_variables() {
        let out = fill_template("# {{title}}\n{{date}} {{time}}\n{{date:DD.MM.}} {{TIME:HH}}\n{{unbekannt}} {{offen", "Notiz", at(), "", "", false);
        assert_eq!(out, "# Notiz\n2026-03-05 14:07\n05.03. 14\n{{unbekannt}} {{offen");
        assert_eq!(fill_template("{{date}}", "", at(), "YY", "", false), "26");
    }
}
