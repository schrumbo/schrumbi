use std::cell::Cell;
use std::path::PathBuf;

thread_local! {
    static CAUGHT: Cell<bool> = const { Cell::new(false) };
}

fn log_path() -> Option<PathBuf> {
    super::prefs::data_dir().map(|d| d.join("crash.log"))
}

/// Writes every panic to `crash.log`, because a release build has no console to show it on.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if CAUGHT.get() {
            return;
        }
        if let Some(path) = log_path() {
            let thread = std::thread::current();
            let report = format!(
                "Schrumbi {} on {} {}\nZeit: {}\nThread: {}\n\n{info}\n\n{}\n",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                chrono::Local::now().to_rfc3339(),
                thread.name().unwrap_or("?"),
                std::backtrace::Backtrace::force_capture(),
            );
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, report);
        }
        previous(info);
    }));
}

/// Runs work that may panic on a broken file; such a panic is expected, so it is neither logged nor printed.
pub fn caught<R>(work: impl FnOnce() -> R) -> Option<R> {
    let before = CAUGHT.replace(true);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).ok();
    CAUGHT.set(before);
    result
}

/// A thread pool whose panics count as caught, for libraries that spread work over rayon.
pub fn caught_pool() -> Option<rayon::ThreadPool> {
    rayon::ThreadPoolBuilder::new().start_handler(|_| CAUGHT.set(true)).build().ok()
}

/// The log of a crash in the previous run, once; it is kept as `crash-last.log` for sending it on.
pub fn take_pending() -> Option<PathBuf> {
    let path = log_path().filter(|p| p.exists())?;
    let kept = path.with_file_name("crash-last.log");
    std::fs::rename(&path, &kept).ok()?;
    Some(kept)
}

#[cfg(test)]
mod tests {
    use super::{install, log_path, take_pending};

    #[test]
    #[ignore]
    fn crash_child() {
        println!("LOG={}", log_path().unwrap().display());
        install();
        panic!("Testabsturz");
    }

    #[test]
    fn writes_a_log_when_the_app_panics() {
        let out = std::process::Command::new(std::env::current_exe().unwrap()).args(["--ignored", "--exact", "--nocapture", "core::crash::tests::crash_child"]).output().unwrap();
        assert!(!out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        let path = stdout.lines().find_map(|l| l.split("LOG=").nth(1)).unwrap().to_string();
        let log = std::fs::read_to_string(&path).unwrap();
        assert!(log.contains("Testabsturz"), "{log}");
        assert!(log.contains("crash.rs"), "{log}");
        let _ = std::fs::remove_file(path);
        assert!(take_pending().is_none());
    }
}
