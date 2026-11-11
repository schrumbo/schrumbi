use chrono::{Datelike, Days, NaiveDate, NaiveTime, Weekday};

/// A title with the date words at its end taken out: "Referat morgen 18 Uhr".
#[derive(Debug, PartialEq)]
pub struct NaturalDate {
    pub title: String,
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
}

const CONNECTORS: [&str; 10] = ["am", "um", "bis", "ab", "nächsten", "nächste", "kommenden", "next", "at", "on"];

fn weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "montag" | "monday" => Weekday::Mon,
        "dienstag" | "tuesday" => Weekday::Tue,
        "mittwoch" | "wednesday" => Weekday::Wed,
        "donnerstag" | "thursday" => Weekday::Thu,
        "freitag" | "friday" => Weekday::Fri,
        "samstag" | "saturday" => Weekday::Sat,
        "sonntag" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

const MAX_AHEAD_DAYS: u64 = 3650;

fn ahead(today: NaiveDate, days: u64) -> Option<NaiveDate> {
    today.checked_add_days(Days::new(days))
}

fn next_weekday(today: NaiveDate, target: Weekday) -> Option<NaiveDate> {
    let days = (7 + target.num_days_from_monday() as u64 - today.weekday().num_days_from_monday() as u64) % 7;
    ahead(today, if days == 0 { 7 } else { days })
}

fn clock(word: &str) -> Option<NaiveTime> {
    let (h, m) = word.split_once([':', '.'])?;
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

/// "12.11." or "12.11.2026"; the closing dot keeps "Kapitel 3.5" a title.
fn calendar_date(word: &str, today: NaiveDate) -> Option<NaiveDate> {
    let parts: Vec<&str> = word.split('.').collect();
    let (d, m, y) = match parts[..] {
        [d, m, ""] => (d, m, None),
        [d, m, y] if y.len() == 4 => (d, m, Some(y.parse().ok()?)),
        _ => return None,
    };
    let (d, m): (u32, u32) = (d.parse().ok()?, m.parse().ok()?);
    let date = NaiveDate::from_ymd_opt(y.unwrap_or(today.year()), m, d)?;
    if y.is_none() && date < today { NaiveDate::from_ymd_opt(today.year() + 1, m, d) } else { Some(date) }
}

/// Reads date and time words from the end of `input`; `None` when there are none.
pub fn parse(input: &str, today: NaiveDate) -> Option<NaturalDate> {
    let words: Vec<&str> = input.split_whitespace().collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let (mut end, mut date, mut time) = (words.len(), None, None);
    loop {
        let last = lower[..end].last().map(String::as_str);
        let before = end.checked_sub(2).map(|i| lower[i].as_str());
        let before2 = end.checked_sub(3).map(|i| lower[i].as_str());
        match (before2, before, last) {
            (_, Some(hour), Some("uhr")) if time.is_none() => {
                time = hour.parse().ok().and_then(|h| NaiveTime::from_hms_opt(h, 0, 0)).or_else(|| clock(hour));
                if time.is_none() {
                    break;
                }
                end -= 2;
            }
            (Some("in"), Some(n), Some("tag" | "tagen" | "day" | "days")) if date.is_none() => {
                date = Some(ahead(today, n.parse::<u64>().ok().filter(|n| *n <= MAX_AHEAD_DAYS)?)?);
                end -= 3;
            }
            (Some("in"), Some(n), Some("woche" | "wochen" | "week" | "weeks")) if date.is_none() => {
                date = Some(ahead(today, n.parse::<u64>().ok()?.checked_mul(7).filter(|d| *d <= MAX_AHEAD_DAYS)?)?);
                end -= 3;
            }
            (_, _, Some(word)) => {
                if time.is_none() && let Some(t) = clock(word).filter(|_| word.contains(':')) {
                    time = Some(t);
                } else if date.is_none() {
                    date = match word {
                        "heute" | "today" => Some(today),
                        "morgen" | "tomorrow" => ahead(today, 1),
                        "übermorgen" => ahead(today, 2),
                        _ => weekday(word).and_then(|w| next_weekday(today, w)).or_else(|| calendar_date(word, today)),
                    };
                    if date.is_none() {
                        break;
                    }
                } else {
                    break;
                }
                end -= 1;
            }
            _ => break,
        }
        while end > 0 && CONNECTORS.contains(&lower[end - 1].as_str()) {
            end -= 1;
        }
    }
    (date.is_some() || time.is_some()).then(|| NaturalDate { title: words[..end].join(" "), date, time })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saturday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()
    }

    fn day(m: u32, d: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(2026, m, d)
    }

    fn at(h: u32, m: u32) -> Option<NaiveTime> {
        NaiveTime::from_hms_opt(h, m, 0)
    }

    #[test]
    fn reads_words_and_times() {
        assert_eq!(parse("Referat morgen 18 Uhr", saturday()), Some(NaturalDate { title: "Referat".into(), date: day(11, 1), time: at(18, 0) }));
        assert_eq!(parse("Essay bis Freitag um 12:30", saturday()), Some(NaturalDate { title: "Essay".into(), date: day(11, 6), time: at(12, 30) }));
        assert_eq!(parse("Lesen in 2 Wochen", saturday()), Some(NaturalDate { title: "Lesen".into(), date: day(11, 14), time: None }));
        assert_eq!(parse("Klausur am 12.11.", saturday()), Some(NaturalDate { title: "Klausur".into(), date: day(11, 12), time: None }));
    }

    #[test]
    fn same_weekday_means_next_week_and_past_dates_next_year() {
        assert_eq!(parse("X Samstag", saturday()).unwrap().date, day(11, 7));
        assert_eq!(parse("X 3.2.", saturday()).unwrap().date, NaiveDate::from_ymd_opt(2027, 2, 3));
    }

    #[test]
    fn plain_titles_stay_untouched() {
        for title in ["Kapitel 3.5", "Montag Protokoll lesen", "Referat", "", "Blatt 12 Uhrwerk"] {
            assert_eq!(parse(title, saturday()), None, "{title}");
        }
    }

    #[test]
    fn nonsense_numbers_are_not_dates() {
        assert_eq!(parse("X 45.13.", saturday()), None);
        assert_eq!(parse("X 25 Uhr", saturday()), None);
    }

    #[test]
    fn huge_offsets_are_not_dates() {
        for title in ["X in 100000000 Tagen", "X in 99999999999 Tagen", "X in 3000000000000000000 Wochen", "X in 3651 days", "X in 18446744073709551615 Wochen"] {
            assert_eq!(parse(title, saturday()), None, "{title}");
        }
        assert!(parse("X in 3650 Tagen", saturday()).is_some());
        assert!(parse("X morgen", NaiveDate::MAX).is_none());
        assert!(parse("X Montag", NaiveDate::MAX).is_none());
    }
}
