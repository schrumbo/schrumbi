use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::tools;
use crate::core::i18n::trf;
use crate::core::library::Library;

const BUSY: &str = "Schrumbi is syncing or moving files right now. Try again in a moment.";

/// Writes of one session this close together count as one turn when nothing else happened in between.
pub const GAP: Duration = Duration::from_secs(120);

struct Turn {
    recorded: u64,
    top: String,
    at: Instant,
    labels: Vec<String>,
}

/// Folds the writes of one agent turn into one undo step, named „KI: …“.
#[derive(Default)]
pub struct Turns {
    open: HashMap<String, Turn>,
}

impl Turns {
    /// Runs a tool for `session`. Its steps join the turn's step while that is still the latest one.
    pub fn call(&mut self, lib: &mut Library, session: &str, name: &str, args: &Value) -> Result<Value, String> {
        if tools::is_write(name) && lib.busy() {
            return Err(BUSY.into());
        }
        let before = lib.recorded_steps();
        let joins = self.open.get(session).is_some_and(|t| t.at.elapsed() < GAP && t.recorded == before && lib.undo_label() == Some(t.top.as_str()));
        let result = tools::call(lib, name, args);
        let added = usize::try_from(lib.recorded_steps() - before).unwrap_or(usize::MAX);
        if added > 0 {
            let changed = result.as_ref().ok().and_then(|o| o.changed.clone()).or_else(|| lib.undo_label().map(String::from)).unwrap_or_default();
            let mut labels = if joins { self.open.remove(session).map(|t| t.labels).unwrap_or_default() } else { Vec::new() };
            labels.push(changed);
            let label = if labels.len() == 1 { trf("KI: %@", &[&labels[0]]) } else { trf("KI: %lld Änderungen", &[&labels.len()]) };
            lib.join_latest(added + usize::from(joins), "pencil", label.clone());
            self.open.insert(session.to_string(), Turn { recorded: lib.recorded_steps(), top: label, at: Instant::now(), labels });
        }
        result.map(|o| o.value)
    }

    /// Ends the turn of `session`: its next write starts a new undo step.
    pub fn end(&mut self, session: &str) {
        self.open.remove(session);
    }
}
