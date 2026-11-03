use std::cell::Cell;
use std::sync::LazyLock;
use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::*;

static PRINT: LazyLock<bool> = LazyLock::new(|| std::env::var_os("SCHRUMBI_PERF").is_some());
static ENABLED: LazyLock<bool> = LazyLock::new(|| *PRINT || crate::core::drive::on());

thread_local! {
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Marks the start of a frame when `SCHRUMBI_PERF` is set or an agent drives the app.
pub fn frame_start() {
    if *ENABLED {
        STARTED.set(Some(Instant::now()));
    }
}

/// Adds the element painted last, which prints the frame's CPU time as `PERF <ms>` to stderr and hands it to the driving agent.
pub fn frame_end<E: ParentElement>(mut root: E) -> E {
    if *ENABLED {
        let last = canvas(|_, _, _| {}, |_, _, window, _| {
            let ms = STARTED.take().map(|started| started.elapsed().as_secs_f64() * 1000.);
            if *PRINT && let Some(ms) = ms {
                eprintln!("PERF {ms:.2}");
            }
            super::drive::frame(window, ms.map(|ms| ms as f32));
        });
        root.extend([last.absolute().size_0().into_any_element()]);
    }
    root
}

/// A secondary window's root, timed like the main window's when measuring.
pub fn window(root: AnyElement) -> AnyElement {
    if *ENABLED { frame_end(div().size_full().child(root)).into_any_element() } else { root }
}
