use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Days, Local};

use super::day;
use super::model::{Assignment, Kind, LectureWeek, Rule, Semester, StudyConfig, StudyState};

/// Lecture weeks of a semester; weeks whose Monday lies in a break are skipped and not counted.
pub fn lecture_weeks(semester: &Semester) -> Vec<LectureWeek> {
    let (Some(start), Some(end)) = (semester.start_date(), semester.end_date()) else { return Vec::new() };
    let mut weeks = Vec::new();
    let mut monday = day::monday(start);
    let mut index = 0;
    while monday <= end {
        if semester.break_at(monday).is_none() {
            index += 1;
            weeks.push(LectureWeek { index, monday, iso_week: day::iso_week(monday) });
        }
        monday = monday + Days::new(7);
    }
    weeks
}

pub fn due_date(rule: &Rule, week: &LectureWeek) -> Option<DateTime<Local>> {
    let offset = rule.week_offset * 7 + (rule.weekday.clamp(1, 7) as i64 - 1);
    let d = week.monday.checked_add_signed(chrono::Duration::days(offset))?;
    day::at(d, day::time(&rule.time))
}

pub fn resolve(template: &str, week: &LectureWeek, due: DateTime<Local>) -> String {
    template
        .replace("{w}", &format!("{:02}", week.index))
        .replace("{kw}", &format!("{:02}", week.iso_week))
        .replace("{date}", &day::yymmdd(day::local_date(due)))
}

/// Moves a due date off holidays to the next regular day, keeping the time.
fn shift_off_holidays(d: DateTime<Local>, semester: &Semester) -> DateTime<Local> {
    let wall = day::wall(d);
    let mut date = wall.date();
    for _ in 0..14 {
        if !semester.is_holiday(date) {
            break;
        }
        date = date + Days::new(1);
    }
    day::at(date, wall.time()).unwrap_or(d)
}

/// Expands rules and manual entries into concrete assignments, sorted by due date.
pub fn assignments(config: &StudyConfig, state: &StudyState, root: &Path) -> Vec<Assignment> {
    let mut result = Vec::new();
    for module in &config.modules {
        let Some(semester) = config.semester(&module.semester) else { continue };
        let module_dir = root.join(&module.folder);
        let weeks = lecture_weeks(semester);
        for rule in &module.rules {
            let mut previous_due = None;
            for week in weeks.iter().filter(|w| rule.applies_to_week(w.index)) {
                let Some(planned) = due_date(rule, week) else { continue };
                if semester.break_at(day::local_date(planned)).is_some() {
                    continue;
                }
                let id = format!("{}.{}.w{}", module.id, rule.id, week.index);
                let moved = state.moved.get(&id).map(|d| d.with_timezone(&Local));
                let due = moved.unwrap_or_else(|| shift_off_holidays(planned, semester));
                let folder = if rule.folder.is_empty() { module_dir.clone() } else { module_dir.join(resolve(&rule.folder, week, planned)) };
                result.push(Assignment {
                    id,
                    module_id: module.id.clone(),
                    semester_id: semester.id.clone(),
                    rule_id: Some(rule.id.clone()),
                    title: rule.title.clone(),
                    short: rule.label().to_string(),
                    kind: Kind::Submission,
                    due,
                    week: Some(week.index),
                    iso_week: Some(week.iso_week),
                    optional: rule.optional,
                    upload: rule.upload,
                    folder,
                    per_week_folder: rule.is_per_week(),
                    file_pattern: rule.file_pattern.clone(),
                    window_start: previous_due,
                    recursive: rule.recursive,
                    keywords: rule.keywords.clone(),
                    is_moved: moved.is_some(),
                });
                previous_due = Some(due);
            }
        }
    }
    for m in &state.manual {
        let Some(module) = config.module(&m.module) else { continue };
        let module_dir = root.join(&module.folder);
        let due = m.due.with_timezone(&Local);
        let week = config.semester(&module.semester).and_then(|s| lecture_weeks(s).into_iter().rev().find(|w| w.monday <= day::local_date(due)));
        result.push(Assignment {
            id: format!("manual.{}", m.id),
            module_id: module.id.clone(),
            semester_id: module.semester.clone(),
            rule_id: None,
            title: m.title.clone(),
            short: m.title.clone(),
            kind: m.kind,
            due,
            week: week.map(|w| w.index),
            iso_week: Some(day::iso_week(day::local_date(due))),
            optional: m.optional,
            upload: m.kind == Kind::Submission && m.upload,
            folder: if m.folder.is_empty() { module_dir } else { module_dir.join(&m.folder) },
            per_week_folder: false,
            file_pattern: "*".into(),
            window_start: None,
            recursive: false,
            keywords: Vec::new(),
            is_moved: false,
        });
    }
    result.sort_by_key(|a| a.due);
    result
}

/// Folders that rules expect to exist for a semester.
pub fn expected_folders(config: &StudyConfig, semester: &Semester, root: &Path) -> Vec<PathBuf> {
    let mut urls = BTreeSet::new();
    let weeks = lecture_weeks(semester);
    for module in config.modules.iter().filter(|m| m.semester == semester.id) {
        let module_dir = root.join(&module.folder);
        urls.insert(module_dir.clone());
        for week in &weeks {
            for rule in module.rules.iter().filter(|r| !r.folder.is_empty() && r.applies_to_week(week.index)) {
                if let Some(due) = due_date(rule, week) {
                    urls.insert(module_dir.join(resolve(&rule.folder, week, due)));
                }
            }
        }
    }
    urls.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Break;

    #[test]
    fn skips_break_weeks_without_counting_them() {
        let s = Semester {
            id: "s".into(),
            start: "2026-10-12".into(),
            end: "2026-11-08".into(),
            breaks: vec![Break { name: "Pause".into(), start: "2026-10-19".into(), end: "2026-10-25".into() }],
            ..Default::default()
        };
        let weeks = lecture_weeks(&s);
        assert_eq!(weeks.iter().map(|w| w.index).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(weeks[1].monday, day::parse("2026-10-26").unwrap());
        assert_eq!(weeks[1].iso_week, 44);
    }
}
