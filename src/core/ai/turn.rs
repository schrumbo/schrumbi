use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::tools::{self, Outcome};
use crate::core::i18n::trf;
use crate::core::library::Library;

const BUSY: &str = "Schrumbi is syncing or moving files right now. Try again in a moment.";

struct Turn {
    recorded: u64,
    top: String,
    labels: Vec<String>,
}

/// Folds the writes of one agent turn into one undo step, named „KI: …“.
#[derive(Default)]
pub struct Turns {
    open: HashMap<String, Turn>,
    /// Sessions whose turns have real boundaries; any other session's writes are one undo step each.
    bounded: HashSet<String>,
}

impl Turns {
    /// Runs a tool for `session`. Its steps join the turn's step while that is still the latest one.
    pub fn call(&mut self, lib: &mut Library, session: &str, name: &str, args: &Value) -> Result<Value, String> {
        if tools::is_write(name) && lib.busy() {
            return Err(BUSY.into());
        }
        self.run(lib, session, |lib| tools::call(lib, name, args))
    }

    /// Runs any change for `session` and folds the steps it records into the turn.
    pub fn run(&mut self, lib: &mut Library, session: &str, change: impl FnOnce(&mut Library) -> Result<Outcome, String>) -> Result<Value, String> {
        let before = lib.recorded_steps();
        let joins = self.bounded.contains(session) && self.open.get(session).is_some_and(|t| t.recorded == before && lib.undo_label() == Some(t.top.as_str()));
        let result = change(lib);
        let added = usize::try_from(lib.recorded_steps() - before).unwrap_or(usize::MAX);
        if added > 0 {
            let changed = result.as_ref().ok().and_then(|o| o.changed.clone()).or_else(|| lib.undo_label().map(String::from)).unwrap_or_default();
            let mut labels = if joins { self.open.remove(session).map(|t| t.labels).unwrap_or_default() } else { Vec::new() };
            labels.push(changed);
            let label = if labels.len() == 1 { trf("KI: %@", &[&labels[0]]) } else { trf("KI: %lld Änderungen", &[&labels.len()]) };
            lib.join_latest(added + usize::from(joins), "pencil", label.clone());
            self.open.insert(session.to_string(), Turn { recorded: lib.recorded_steps(), top: label, labels });
        }
        result.map(|o| o.value)
    }

    /// Starts a turn of `session` with a real boundary: its writes join until `end`, however long it takes.
    pub fn begin(&mut self, session: &str) {
        self.open.remove(session);
        self.bounded.insert(session.to_string());
    }

    /// Whether `session` is a column session with real turns, not a program in the terminal.
    pub fn is_bounded(&self, session: &str) -> bool {
        self.bounded.contains(session)
    }

    /// Ends the turn of `session`: its next write starts a new undo step.
    pub fn end(&mut self, session: &str) {
        self.open.remove(session);
    }

    /// Forgets `session`, e.g. when its agent ended.
    pub fn close(&mut self, session: &str) {
        self.open.remove(session);
        self.bounded.remove(session);
    }
}
