use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

static DIR: LazyLock<Option<PathBuf>> = LazyLock::new(|| std::env::var_os("SCHRUMBI_DRIVE").map(PathBuf::from));
static EVENTS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static ANSWERS: Mutex<VecDeque<Option<Vec<PathBuf>>>> = Mutex::new(VecDeque::new());
const KEPT_EVENTS: usize = 500;

/// Whether an agent drives this instance; `SCHRUMBI_DRIVE` names its scratch folder.
pub fn on() -> bool {
    DIR.is_some()
}

/// The driven instance's folder.
pub fn dir() -> Option<&'static Path> {
    DIR.as_deref()
}

/// Refuses to drive an instance that would use the real settings.
pub fn check() {
    if on() && std::env::var_os("SCHRUMBI_HOME").is_none() {
        eprintln!("SCHRUMBI_DRIVE needs SCHRUMBI_HOME, so a driven instance never touches the real settings");
        std::process::exit(2);
    }
}

/// Records something the driving agent should know about.
pub fn note(event: String) {
    if on() {
        let mut events = EVENTS.lock().unwrap();
        if events.len() == KEPT_EVENTS {
            events.pop_front();
        }
        events.push_back(event);
    }
}

/// True when `what` must not reach the system, a browser or another app; it is only recorded.
pub fn fenced(what: &str, target: impl std::fmt::Display) -> bool {
    if on() {
        note(format!("{what} {target}"));
    }
    on()
}

/// Everything recorded since the last call.
pub fn take_events() -> Vec<String> {
    EVENTS.lock().unwrap().drain(..).collect()
}

/// The answer for the next file dialog: the paths, or `None` to cancel it.
pub fn queue_answer(paths: Option<Vec<PathBuf>>) {
    ANSWERS.lock().unwrap().push_back(paths);
}

/// Answers a file dialog without showing it; with nothing queued the dialog is cancelled.
pub fn answer(kind: &str) -> Option<Vec<PathBuf>> {
    let answer = ANSWERS.lock().unwrap().pop_front().flatten();
    note(format!("dialog {kind} -> {}", answer.as_ref().map_or("cancel".to_string(), |p| p.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))));
    answer
}
