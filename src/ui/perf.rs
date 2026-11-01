use std::cell::Cell;
use std::sync::LazyLock;
use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::*;

static ENABLED: LazyLock<bool> = LazyLock::new(|| std::env::var_os("SCHRUMBI_PERF").is_some());

thread_local! {
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Marks the start of a frame when `SCHRUMBI_PERF` is set.
pub fn frame_start() {
    if *ENABLED {
        STARTED.set(Some(Instant::now()));
    }
}

/// Adds the element painted last, which prints the frame's CPU time as `PERF <ms>` to stderr.
pub fn frame_end<E: ParentElement>(mut root: E) -> E {
    if *ENABLED {
        let last = canvas(|_, _, _| {}, |_, _, _, _| {
            if let Some(started) = STARTED.take() {
                eprintln!("PERF {:.2}", started.elapsed().as_secs_f64() * 1000.);
            }
        });
        root.extend([last.absolute().size_0().into_any_element()]);
    }
    root
}
