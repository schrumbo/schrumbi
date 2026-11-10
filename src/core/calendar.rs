use chrono::{DateTime, Datelike, Days, Local, Months, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

use super::day;

/// How far the overview calendar looks: one week in columns, or a month as a grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CalendarView {
    #[default]
    Week,
    Month,
}

/// Rows of the month grid; always six, so paging never changes its height.
pub const ROWS: usize = 6;

/// One month as rows of weeks, Monday first.
pub type Grid = [[NaiveDate; 7]; ROWS];

/// The first day of the month holding `d`.
pub fn month_start(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

/// The grid of the month holding `anchor`: from the Monday on or before its first day, six weeks on.
pub fn month_grid(anchor: NaiveDate) -> Grid {
    let first = day::monday(month_start(anchor));
    std::array::from_fn(|r| std::array::from_fn(|c| first + Days::new((r * 7 + c) as u64)))
}

/// The ISO week number of each grid row.
pub fn week_numbers(grid: &Grid) -> [u32; ROWS] {
    std::array::from_fn(|r| day::iso_week(grid[r][0]))
}

/// `anchor` moved by `pages` weeks or months; a month page keeps the day of the month as far as the month has it.
pub fn shift(view: CalendarView, anchor: NaiveDate, pages: i32) -> NaiveDate {
    let moved = match view {
        CalendarView::Week => anchor.checked_add_signed(chrono::Duration::days(7 * i64::from(pages))),
        CalendarView::Month if pages >= 0 => anchor.checked_add_months(Months::new(pages.unsigned_abs())),
        CalendarView::Month => anchor.checked_sub_months(Months::new(pages.unsigned_abs())),
    };
    moved.unwrap_or(anchor)
}

/// The first day of the week page of `anchor`: today on the page that follows today, else the Monday of its week.
pub fn page_start(anchor: Option<NaiveDate>, today: NaiveDate) -> NaiveDate {
    anchor.map_or(today, day::monday)
}

/// The seven days of the week page of `anchor`: today and the six after it when `None`, else Monday to Sunday.
pub fn page_days(anchor: Option<NaiveDate>, today: NaiveDate) -> [NaiveDate; 7] {
    let first = page_start(anchor, today);
    std::array::from_fn(|i| first + Days::new(i as u64))
}

/// The anchor of the week page showing `d`: the page that follows today when it has `d`.
pub fn anchor_for(d: NaiveDate, today: NaiveDate) -> Option<NaiveDate> {
    (!page_days(None, today).contains(&d)).then_some(d)
}

fn position(anchor: Option<NaiveDate>, today: NaiveDate) -> i64 {
    let monday = day::monday(today);
    match anchor {
        None => i64::from(today != monday),
        Some(d) => 2 * ((day::monday(d) - monday).num_days() / 7),
    }
}

/// The anchor `pages` pages on from `anchor`, where `None` follows today. The week page following today sits between the week holding today and the next one, and replaces the week holding today when today is a Monday.
pub fn turn(view: CalendarView, anchor: Option<NaiveDate>, today: NaiveDate, pages: i32) -> Option<NaiveDate> {
    if view == CalendarView::Month {
        return Some(shift(view, anchor.unwrap_or(today), pages));
    }
    let rolling = position(None, today);
    let step = i64::from(pages.signum());
    let mut at = position(anchor, today);
    for _ in 0..pages.unsigned_abs() {
        at += step;
        if at % 2 != 0 && at != rolling {
            at += step;
        }
    }
    if at == rolling {
        return None;
    }
    let base = anchor.unwrap_or(today);
    let weeks = (at - position(Some(base), today)) / 2;
    Some(i32::try_from(weeks).map_or(base, |w| shift(view, base, w)))
}

/// Whether the page of `anchor` is the one "Heute" brings back: the week following today, or the month holding today.
pub fn at_home(view: CalendarView, anchor: Option<NaiveDate>, today: NaiveDate) -> bool {
    match view {
        CalendarView::Week => page_start(anchor, today) == today,
        CalendarView::Month => anchor.is_none_or(|a| (a.year(), a.month()) == (today.year(), today.month())),
    }
}

/// Which way the page "Heute" brings back lies from the page of `anchor`: -1 earlier, 1 later, 0 the same page.
pub fn toward_home(view: CalendarView, anchor: Option<NaiveDate>, today: NaiveDate) -> i8 {
    if at_home(view, anchor, today) {
        return 0;
    }
    let later = match view {
        CalendarView::Week => position(None, today) > position(anchor, today),
        CalendarView::Month => anchor.is_some_and(|a| a < today),
    };
    if later { 1 } else { -1 }
}

/// The month a page is named after: the middle of its seven days for a week, as ISO week numbering does with the Thursday.
pub fn named_month(view: CalendarView, anchor: Option<NaiveDate>, today: NaiveDate) -> NaiveDate {
    match view {
        CalendarView::Week => page_start(anchor, today) + Days::new(3),
        CalendarView::Month => month_start(anchor.unwrap_or(today)),
    }
}

/// The instant with the wall-clock time of `due` on `date`, so a deadline moved across a clock change keeps its hour.
pub fn same_time_on(due: DateTime<Local>, date: NaiveDate) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&date.and_time(due.time())).earliest()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveTime, Timelike};

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_week_runs_monday_to_sunday_around_its_anchor() {
        let today = ymd(2026, 11, 10);
        let days = page_days(Some(ymd(2026, 11, 10)), today);
        assert_eq!(days[0], ymd(2026, 11, 9));
        assert_eq!(days[6], ymd(2026, 11, 15));
        assert_eq!(page_days(Some(ymd(2026, 11, 15)), today)[0], ymd(2026, 11, 9));
    }

    #[test]
    fn the_month_grid_starts_on_a_monday_and_has_six_rows() {
        let grid = month_grid(ymd(2026, 11, 10));
        assert_eq!(grid[0][0], ymd(2026, 10, 26));
        assert_eq!(grid[0][6], ymd(2026, 11, 1));
        assert_eq!(grid[5][6], ymd(2026, 12, 6));
        assert_eq!(week_numbers(&grid), [44, 45, 46, 47, 48, 49]);
        let june = month_grid(ymd(2026, 6, 15));
        assert_eq!(june[0][0], ymd(2026, 6, 1));
        assert_eq!(june[5][6], ymd(2026, 7, 12));
    }

    #[test]
    fn week_53_shows_in_the_december_grid_and_week_1_follows_it() {
        let grid = month_grid(ymd(2026, 12, 1));
        assert_eq!(grid[0][0], ymd(2026, 11, 30));
        assert_eq!(week_numbers(&grid), [49, 50, 51, 52, 53, 1]);
        assert_eq!(week_numbers(&month_grid(ymd(2027, 1, 1))), [53, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn week_pages_cross_the_year_boundary_in_both_directions() {
        let kw53 = ymd(2026, 12, 29);
        assert_eq!(shift(CalendarView::Week, kw53, 1), ymd(2027, 1, 5));
        assert_eq!(day::iso_week(shift(CalendarView::Week, kw53, 1)), 1);
        assert_eq!(shift(CalendarView::Week, ymd(2027, 1, 5), -1), kw53);
        assert_eq!(day::iso_week(kw53), 53);
        assert_eq!(shift(CalendarView::Week, ymd(2026, 11, 10), -3), ymd(2026, 10, 20));
    }

    #[test]
    fn month_pages_keep_the_day_as_far_as_the_month_has_it() {
        assert_eq!(shift(CalendarView::Month, ymd(2026, 1, 31), 1), ymd(2026, 2, 28));
        assert_eq!(shift(CalendarView::Month, ymd(2026, 3, 31), -1), ymd(2026, 2, 28));
        assert_eq!(shift(CalendarView::Month, ymd(2026, 12, 10), 1), ymd(2027, 1, 10));
        assert_eq!(shift(CalendarView::Month, ymd(2027, 1, 10), -1), ymd(2026, 12, 10));
        assert_eq!(shift(CalendarView::Month, ymd(2026, 11, 10), 0), ymd(2026, 11, 10));
    }

    #[test]
    fn a_week_across_the_clock_change_is_still_seven_days_apart() {
        let before = ymd(2026, 10, 20);
        let after = shift(CalendarView::Week, before, 1);
        assert_eq!(after, ymd(2026, 10, 27));
        assert_eq!((after - before).num_days(), 7);
        assert_eq!(shift(CalendarView::Week, ymd(2026, 3, 24), 1), ymd(2026, 3, 31));
    }

    #[test]
    fn a_moved_deadline_keeps_its_wall_clock_time() {
        let due = Local.from_local_datetime(&ymd(2026, 10, 20).and_time(NaiveTime::from_hms_opt(18, 0, 0).unwrap())).earliest().unwrap();
        let moved = same_time_on(due, ymd(2026, 10, 27)).unwrap();
        assert_eq!(moved.date_naive(), ymd(2026, 10, 27));
        assert_eq!((moved.hour(), moved.minute()), (18, 0));
        let spring = same_time_on(due, ymd(2026, 3, 29)).unwrap();
        assert_eq!((spring.date_naive(), spring.hour()), (ymd(2026, 3, 29), 18));
    }

    #[test]
    fn the_current_week_page_rolls_from_today_and_pages_into_monday_weeks() {
        let today = ymd(2026, 11, 10);
        assert_eq!(page_days(None, today)[0], today);
        assert_eq!(page_days(None, today)[6], ymd(2026, 11, 16));
        let next = turn(CalendarView::Week, None, today, 1);
        assert_eq!(page_days(next, today)[0], ymd(2026, 11, 16));
        assert_eq!(turn(CalendarView::Week, next, today, -1), None);
        let this = turn(CalendarView::Week, None, today, -1);
        assert_eq!(page_days(this, today)[0], ymd(2026, 11, 9));
        assert_eq!(page_days(this, today)[6], ymd(2026, 11, 15));
        assert_eq!(turn(CalendarView::Week, this, today, 1), None);
        assert_eq!(page_start(turn(CalendarView::Week, this, today, -1), today), ymd(2026, 11, 2));
        assert_eq!(page_start(turn(CalendarView::Week, None, today, 2), today), ymd(2026, 11, 23));
        assert_eq!(page_start(turn(CalendarView::Week, None, today, -2), today), ymd(2026, 11, 2));
    }

    #[test]
    fn on_a_monday_the_rolling_page_is_the_week_itself() {
        let monday = ymd(2026, 11, 9);
        assert_eq!(page_days(None, monday), page_days(Some(ymd(2026, 11, 12)), monday));
        assert_eq!(page_start(turn(CalendarView::Week, None, monday, -1), monday), ymd(2026, 11, 2));
        assert_eq!(page_start(turn(CalendarView::Week, None, monday, 1), monday), ymd(2026, 11, 16));
        assert_eq!(turn(CalendarView::Week, Some(ymd(2026, 11, 4)), monday, 1), None);
        assert!(at_home(CalendarView::Week, Some(ymd(2026, 11, 12)), monday));
    }

    #[test]
    fn on_a_sunday_the_next_page_starts_the_next_morning() {
        let sunday = ymd(2026, 11, 15);
        assert_eq!(page_days(None, sunday)[6], ymd(2026, 11, 21));
        assert_eq!(page_start(turn(CalendarView::Week, None, sunday, 1), sunday), ymd(2026, 11, 16));
        let this = turn(CalendarView::Week, None, sunday, -1);
        assert_eq!(page_days(this, sunday)[6], sunday);
        assert!(!at_home(CalendarView::Week, this, sunday));
    }

    #[test]
    fn the_rolling_page_crosses_the_year_boundary() {
        let today = ymd(2026, 12, 30);
        assert_eq!(page_days(None, today)[6], ymd(2027, 1, 5));
        let next = turn(CalendarView::Week, None, today, 1);
        assert_eq!(page_start(next, today), ymd(2027, 1, 4));
        assert_eq!(day::iso_week(page_start(next, today)), 1);
        let this = turn(CalendarView::Week, None, today, -1);
        assert_eq!(day::iso_week(page_start(this, today)), 53);
        assert_eq!(page_start(turn(CalendarView::Week, this, today, -1), today), ymd(2026, 12, 21));
        let sunday = ymd(2027, 1, 3);
        assert_eq!(page_start(turn(CalendarView::Week, None, sunday, -1), sunday), ymd(2026, 12, 28));
        assert_eq!(page_start(turn(CalendarView::Week, None, sunday, 1), sunday), ymd(2027, 1, 4));
    }

    #[test]
    fn a_page_knows_whether_it_is_home_and_which_way_home_lies() {
        let today = ymd(2026, 11, 10);
        assert!(at_home(CalendarView::Week, None, today));
        assert!(!at_home(CalendarView::Week, Some(today), today));
        assert!(at_home(CalendarView::Month, Some(ymd(2026, 11, 30)), today));
        assert!(!at_home(CalendarView::Month, Some(ymd(2026, 12, 1)), today));
        assert_eq!(toward_home(CalendarView::Week, Some(ymd(2026, 11, 23)), today), -1);
        assert_eq!(toward_home(CalendarView::Week, Some(ymd(2026, 11, 16)), today), -1);
        assert_eq!(toward_home(CalendarView::Week, Some(today), today), 1);
        assert_eq!(toward_home(CalendarView::Week, Some(ymd(2026, 10, 1)), today), 1);
        assert_eq!(toward_home(CalendarView::Month, Some(ymd(2026, 11, 1)), today), 0);
        assert_eq!(toward_home(CalendarView::Month, Some(ymd(2027, 1, 1)), today), -1);
    }

    #[test]
    fn a_day_outside_the_rolling_page_opens_its_own_week() {
        let today = ymd(2026, 11, 10);
        assert_eq!(anchor_for(ymd(2026, 11, 16), today), None);
        assert_eq!(anchor_for(ymd(2026, 11, 9), today), Some(ymd(2026, 11, 9)));
        assert_eq!(anchor_for(ymd(2026, 11, 17), today), Some(ymd(2026, 11, 17)));
    }

    #[test]
    fn a_week_is_named_after_the_month_of_its_middle_day() {
        assert_eq!(named_month(CalendarView::Week, Some(ymd(2026, 11, 30)), ymd(2026, 11, 10)), ymd(2026, 12, 3));
        assert_eq!(named_month(CalendarView::Week, Some(ymd(2027, 1, 1)), ymd(2026, 11, 10)), ymd(2026, 12, 31));
        assert_eq!(named_month(CalendarView::Week, Some(ymd(2025, 12, 29)), ymd(2026, 11, 10)), ymd(2026, 1, 1));
        assert_eq!(named_month(CalendarView::Week, None, ymd(2026, 11, 28)), ymd(2026, 12, 1));
        assert_eq!(named_month(CalendarView::Month, Some(ymd(2026, 11, 30)), ymd(2026, 11, 10)), ymd(2026, 11, 1));
    }
}
