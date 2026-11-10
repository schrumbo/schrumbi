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

/// The seven days of the week holding `anchor`, Monday first.
pub fn week_days(anchor: NaiveDate) -> [NaiveDate; 7] {
    let monday = day::monday(anchor);
    std::array::from_fn(|i| monday + Days::new(i as u64))
}

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

/// Whether the page holding `anchor` shows `today`.
pub fn shows(view: CalendarView, anchor: NaiveDate, today: NaiveDate) -> bool {
    match view {
        CalendarView::Week => day::monday(anchor) == day::monday(today),
        CalendarView::Month => (anchor.year(), anchor.month()) == (today.year(), today.month()),
    }
}

/// Which way the page holding `to` lies from the page holding `from`: -1 earlier, 1 later, 0 the same page.
pub fn direction(view: CalendarView, from: NaiveDate, to: NaiveDate) -> i8 {
    if shows(view, from, to) {
        0
    } else if to < from {
        -1
    } else {
        1
    }
}

/// The month a page is named after; for a week its Thursday decides, as ISO week numbering does.
pub fn named_month(view: CalendarView, anchor: NaiveDate) -> NaiveDate {
    match view {
        CalendarView::Week => day::monday(anchor) + Days::new(3),
        CalendarView::Month => month_start(anchor),
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
        let days = week_days(ymd(2026, 11, 10));
        assert_eq!(days[0], ymd(2026, 11, 9));
        assert_eq!(days[6], ymd(2026, 11, 15));
        assert_eq!(week_days(ymd(2026, 11, 15))[0], ymd(2026, 11, 9));
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
    fn a_page_knows_whether_today_is_on_it() {
        let today = ymd(2026, 11, 10);
        assert!(shows(CalendarView::Week, ymd(2026, 11, 15), today));
        assert!(!shows(CalendarView::Week, ymd(2026, 11, 16), today));
        assert!(shows(CalendarView::Month, ymd(2026, 11, 30), today));
        assert!(!shows(CalendarView::Month, ymd(2026, 12, 1), today));
        assert_eq!(direction(CalendarView::Week, ymd(2026, 11, 23), today), -1);
        assert_eq!(direction(CalendarView::Week, ymd(2026, 10, 1), today), 1);
        assert_eq!(direction(CalendarView::Month, ymd(2026, 11, 1), today), 0);
    }

    #[test]
    fn a_week_is_named_after_the_month_of_its_thursday() {
        assert_eq!(named_month(CalendarView::Week, ymd(2026, 11, 30)), ymd(2026, 12, 3));
        assert_eq!(named_month(CalendarView::Week, ymd(2027, 1, 1)), ymd(2026, 12, 31));
        assert_eq!(named_month(CalendarView::Week, ymd(2025, 12, 29)), ymd(2026, 1, 1));
        assert_eq!(named_month(CalendarView::Month, ymd(2026, 11, 30)), ymd(2026, 11, 1));
    }
}
