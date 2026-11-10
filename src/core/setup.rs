use std::collections::BTreeSet;

/// One thing the app can be set up with, in the order the setup list shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    Modules,
    Semester,
    Calendar,
    Naming,
    Editor,
    Cards,
    Sync,
}

impl Item {
    pub const ALL: [Item; 7] = [Item::Modules, Item::Semester, Item::Calendar, Item::Naming, Item::Editor, Item::Cards, Item::Sync];

    /// The stable name stored in the settings.
    pub fn key(self) -> &'static str {
        match self {
            Item::Modules => "modules",
            Item::Semester => "semester",
            Item::Calendar => "calendar",
            Item::Naming => "naming",
            Item::Editor => "editor",
            Item::Cards => "cards",
            Item::Sync => "sync",
        }
    }
}

/// What the workspace and the settings say right now; the list ticks itself from these.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    pub modules: usize,
    /// The semester has dates, and they are not the app's own guess.
    pub semester_set: bool,
    pub calendar: bool,
    pub naming_on: bool,
    /// The naming pattern needs no author, or the author is filled in.
    pub author_ok: bool,
    pub editor_chosen: bool,
    pub cards: usize,
    pub sync: bool,
}

/// Whether an item is done by what the facts say, before skips and confirmations.
pub fn done(item: Item, f: &Facts) -> bool {
    match item {
        Item::Modules => f.modules > 0,
        Item::Semester => f.semester_set,
        Item::Calendar => f.calendar,
        Item::Naming => f.naming_on && f.author_ok,
        Item::Editor => f.editor_chosen,
        Item::Cards => f.cards > 0,
        Item::Sync => f.sync,
    }
}

/// Whether an item is settled: done, skipped or its guess confirmed.
pub fn settled(item: Item, f: &Facts, skipped: &BTreeSet<String>, confirmed: &BTreeSet<String>) -> bool {
    done(item, f) || skipped.contains(item.key()) || confirmed.contains(item.key())
}

/// Settled items and the total.
pub fn progress(f: &Facts, skipped: &BTreeSet<String>, confirmed: &BTreeSet<String>) -> (usize, usize) {
    (Item::ALL.iter().filter(|i| settled(**i, f, skipped, confirmed)).count(), Item::ALL.len())
}

/// Items still waiting, in list order.
pub fn open_items(f: &Facts, skipped: &BTreeSet<String>, confirmed: &BTreeSet<String>) -> Vec<Item> {
    Item::ALL.into_iter().filter(|i| !settled(*i, f, skipped, confirmed)).collect()
}

/// Whether the list has something to show: an open item, or a complete list that has not said so yet.
pub fn wanted(f: &Facts, skipped: &BTreeSet<String>, confirmed: &BTreeSet<String>, dismissed: bool, completed: bool) -> bool {
    if dismissed || completed {
        return false;
    }
    let (settled, total) = progress(f, skipped, confirmed);
    settled < total || !completed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn a_fresh_workspace_has_everything_open() {
        let f = Facts::default();
        assert_eq!(open_items(&f, &set(&[]), &set(&[])), Item::ALL.to_vec());
        assert_eq!(progress(&f, &set(&[]), &set(&[])), (0, 7));
    }

    #[test]
    fn items_tick_from_real_state_not_clicks() {
        let f = Facts { modules: 2, semester_set: true, calendar: true, naming_on: true, author_ok: true, editor_chosen: true, cards: 3, sync: true };
        assert!(open_items(&f, &set(&[]), &set(&[])).is_empty());
        assert_eq!(progress(&f, &set(&[]), &set(&[])), (7, 7));
        let half = Facts { modules: 1, naming_on: true, ..Facts::default() };
        assert!(!done(Item::Naming, &half), "a pattern with an author placeholder waits for the author");
        assert_eq!(open_items(&half, &set(&[]), &set(&[])), vec![Item::Semester, Item::Calendar, Item::Naming, Item::Editor, Item::Cards, Item::Sync]);
    }

    #[test]
    fn skipped_and_confirmed_count_as_settled() {
        let f = Facts { modules: 1, ..Facts::default() };
        let skipped = set(&["calendar", "naming", "editor", "cards", "sync"]);
        let confirmed = set(&["semester"]);
        assert!(open_items(&f, &skipped, &confirmed).is_empty());
        assert_eq!(progress(&f, &skipped, &confirmed), (7, 7));
        assert!(!done(Item::Semester, &f), "confirming does not change the facts");
    }

    #[test]
    fn the_list_hides_once_dismissed_or_completed() {
        let f = Facts::default();
        assert!(wanted(&f, &set(&[]), &set(&[]), false, false));
        assert!(!wanted(&f, &set(&[]), &set(&[]), true, false));
        let all = Facts { modules: 1, semester_set: true, calendar: true, naming_on: true, author_ok: true, editor_chosen: true, cards: 1, sync: true };
        assert!(wanted(&all, &set(&[]), &set(&[]), false, false), "a complete list shows its completion once");
        assert!(!wanted(&all, &set(&[]), &set(&[]), false, true));
    }
}
