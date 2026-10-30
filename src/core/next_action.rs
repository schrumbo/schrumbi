/// The single thing the overview recommends doing now.
#[derive(Debug, PartialEq)]
pub enum NextAction<T> {
    Overdue(T),
    Unconfirmed(T),
    Due(T),
    Inbox(usize),
    Clear,
}

/// Picks the next action: missed work first, then work about to be missed, the inbox, and finally the next deadline.
pub fn choose<T>(overdue: Option<T>, unconfirmed: Option<T>, next: Option<T>, due_soon: bool, inbox: usize) -> NextAction<T> {
    match (overdue, unconfirmed, next) {
        (Some(a), _, _) => NextAction::Overdue(a),
        (None, Some(a), _) => NextAction::Unconfirmed(a),
        (None, None, Some(a)) if due_soon || inbox == 0 => NextAction::Due(a),
        _ if inbox > 0 => NextAction::Inbox(inbox),
        _ => NextAction::Clear,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missed_work_comes_first() {
        assert_eq!(choose(Some("a"), Some("b"), Some("c"), true, 3), NextAction::Overdue("a"));
        assert_eq!(choose(None, Some("b"), Some("c"), true, 3), NextAction::Unconfirmed("b"));
    }

    #[test]
    fn inbox_waits_only_for_urgent_deadlines() {
        assert_eq!(choose(None, None, Some("c"), true, 3), NextAction::Due("c"));
        assert_eq!(choose(None, None, Some("c"), false, 3), NextAction::Inbox(3));
        assert_eq!(choose(None, None, Some("c"), false, 0), NextAction::Due("c"));
    }

    #[test]
    fn nothing_left_is_clear() {
        assert_eq!(choose::<&str>(None, None, None, false, 2), NextAction::Inbox(2));
        assert_eq!(choose::<&str>(None, None, None, false, 0), NextAction::Clear);
    }
}
