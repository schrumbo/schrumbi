use chrono::{NaiveTime, Timelike};

const DAY: i64 = 24 * 60;

/// The last minute of the day, which `24`, `24:00` and `24 Uhr` read as.
pub fn last_minute() -> NaiveTime {
    NaiveTime::from_hms_opt(23, 59, 0).unwrap_or(NaiveTime::MIN)
}

/// Reads a typed time of day: `18`, `1830`, `18:30`, `6:30`, `18.30`, `18h30`, `14:00h`, `18 Uhr`, `6pm`, `6:30 am`.
pub fn parse(input: &str) -> Option<NaiveTime> {
    let mut s: String = input.trim().to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    s = s.trim_end_matches("uhr").to_string();
    if let Some(rest) = s.strip_suffix('h') {
        s = rest.to_string();
    }
    let meridiem = if let Some(rest) = s.strip_suffix("a.m.").or_else(|| s.strip_suffix("am")).or_else(|| s.strip_suffix('a')) {
        let rest = rest.to_string();
        s = rest;
        Some(false)
    } else if let Some(rest) = s.strip_suffix("p.m.").or_else(|| s.strip_suffix("pm")).or_else(|| s.strip_suffix('p')) {
        let rest = rest.to_string();
        s = rest;
        Some(true)
    } else {
        None
    };
    let parts: Vec<&str> = s.split([':', '.', 'h', ',']).collect();
    let digits = |p: &str| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit());
    let (h, m): (u32, u32) = match parts.as_slice() {
        [one] if !one.is_empty() && one.len() <= 4 && one.chars().all(|c| c.is_ascii_digit()) => {
            let n: u32 = one.parse().ok()?;
            if one.len() <= 2 { (n, 0) } else { (n / 100, n % 100) }
        }
        [h, ""] if digits(h) => (h.parse().ok()?, 0),
        [h, m] if digits(h) && digits(m) => (h.parse().ok()?, m.parse().ok()?),
        _ => return None,
    };
    let h = match meridiem {
        Some(pm) if (1..=12).contains(&h) => h % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None => h,
    };
    if (h, m) == (24, 0) && meridiem.is_none() {
        return Some(last_minute());
    }
    NaiveTime::from_hms_opt(h, m, 0)
}

/// `HH:mm`, the form times are stored in.
pub fn format(t: NaiveTime) -> String {
    t.format("%H:%M").to_string()
}

/// Moves by `minutes` to the next multiple of it, so 18:07 goes up to 18:15 and down to 18:00; wraps at midnight.
pub fn step(t: NaiveTime, minutes: i64) -> NaiveTime {
    let size = minutes.abs().max(1);
    let now = i64::from(t.hour() * 60 + t.minute());
    let next = if minutes >= 0 { (now / size + 1) * size } else { (now + size - 1) / size * size - size };
    let next = next.rem_euclid(DAY) as u32;
    NaiveTime::from_hms_opt(next / 60, next % 60, 0).unwrap_or(t)
}

/// The end that keeps a span's length when its start moves to `new_start`, at most 23:59; unchanged when the span is empty, inverted or unreadable.
pub fn keep_length(start: &str, end: &str, new_start: &str) -> String {
    let (Some(from), Some(to), Some(now)) = (parse(start), parse(end), parse(new_start)) else { return end.to_string() };
    let length = to - from;
    let last = last_minute();
    let ends = now + length;
    match () {
        _ if length <= chrono::Duration::zero() => end.to_string(),
        _ if ends > now => format(ends),
        _ if now < last => format(last),
        _ => end.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{format, keep_length, parse, step};
    use chrono::NaiveTime;

    fn normalize(input: &str) -> Option<String> {
        parse(input).map(format)
    }

    fn at(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn reads_what_people_type() {
        for (typed, wanted) in [
            ("18", "18:00"),
            ("6", "06:00"),
            ("0", "00:00"),
            ("1830", "18:30"),
            ("630", "06:30"),
            ("0915", "09:15"),
            ("18:30", "18:30"),
            ("6:30", "06:30"),
            (" 6:30 ", "06:30"),
            ("18.30", "18:30"),
            ("18,30", "18:30"),
            ("18h30", "18:30"),
            ("18h", "18:00"),
            ("18:", "18:00"),
            ("18 Uhr", "18:00"),
            ("18:30 Uhr", "18:30"),
            ("23:59", "23:59"),
            ("24", "23:59"),
            ("24:00", "23:59"),
            ("24 Uhr", "23:59"),
            ("14:00h", "14:00"),
            ("14.00 h", "14:00"),
            ("14 h", "14:00"),
            ("6pm", "18:00"),
            ("6:30 pm", "18:30"),
            ("12am", "00:00"),
            ("12pm", "12:00"),
            ("9a", "09:00"),
            ("7 p.m.", "19:00"),
        ] {
            assert_eq!(normalize(typed).as_deref(), Some(wanted), "{typed}");
        }
    }

    #[test]
    fn refuses_what_is_no_time() {
        for typed in ["", " ", "24:01", "25:00", "18:60", "1860", "12345", "abc", "18:30:00", "13pm", "0am", ":30", "18:300", "-1", "6 30x"] {
            assert_eq!(parse(typed), None, "{typed}");
        }
    }

    #[test]
    fn steps_to_the_next_mark_and_wraps_at_midnight() {
        assert_eq!(step(at(18, 7), 15), at(18, 15));
        assert_eq!(step(at(18, 7), -15), at(18, 0));
        assert_eq!(step(at(18, 15), 15), at(18, 30));
        assert_eq!(step(at(18, 15), -15), at(18, 0));
        assert_eq!(step(at(23, 59), 15), at(0, 0));
        assert_eq!(step(at(23, 59), -15), at(23, 45));
        assert_eq!(step(at(0, 0), -15), at(23, 45));
        assert_eq!(step(at(23, 30), 60), at(0, 0));
        assert_eq!(step(at(18, 30), -60), at(18, 0));
        assert_eq!(step(at(18, 30), 60), at(19, 0));
    }

    #[test]
    fn a_moved_start_keeps_the_length() {
        assert_eq!(keep_length("10:15", "11:45", "14:00"), "15:30");
        assert_eq!(keep_length("10:15", "11:45", "08:00"), "09:30");
        assert_eq!(keep_length("10:15", "11:45", "23:00"), "23:59", "stops at the end of the day");
        assert_eq!(keep_length("10:15", "11:45", "23:59"), "11:45", "no later time left");
        assert_eq!(keep_length("12:00", "11:00", "13:00"), "11:00", "an inverted span stays as it was");
        assert_eq!(keep_length("", "11:00", "13:00"), "11:00");
    }

    #[test]
    fn a_moved_start_leaves_the_end_after_it_except_at_the_last_minute() {
        for start in ["00:00", "09:00", "11:45", "12:00", "22:30", "23:30", "23:58"] {
            let end = keep_length("10:15", "11:45", start);
            assert!(parse(&end) > parse(start), "{start} -> {end}");
        }
        assert_eq!(keep_length("10:15", "11:45", "23:58"), "23:59");
        assert_eq!(keep_length("10:15", "11:45", "24"), "11:45", "no later minute, the start field refuses it");
        assert_eq!(parse("24"), Some(super::last_minute()), "so a start field that must stay before it refuses it");
    }
}
