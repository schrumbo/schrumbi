use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::{App, AppContext, Entity, Global, Task};
use notify::{EventKind, RecursiveMode, Watcher};

use crate::core::library::{Library, Reminder};
use crate::core::workspace::{CONFIG_NAME, STATE_NAME};

/// Many apps empty a file before writing it; reading in between would show an empty note.
const RELOAD_SETTLE: u64 = 150;

/// The shared library plus the background work that keeps it current.
pub struct Store {
    pub lib: Entity<Library>,
    watcher: Option<notify::RecommendedWatcher>,
    watch_task: Option<Task<()>>,
    reminder_task: Option<Task<()>>,
    reminders: Vec<Reminder>,
    pending_refresh: Option<Task<()>>,
    pending_index: Option<Task<()>>,
    pending_config: Option<Task<()>>,
    pending_notes: Option<Task<()>>,
    pending_reload: Option<Task<()>>,
}

impl Global for Store {}

pub fn lib(cx: &App) -> Entity<Library> {
    cx.global::<Store>().lib.clone()
}

pub fn init(cx: &mut App) {
    let lib = cx.new(|_| Library::new());
    cx.set_global(Store {
        lib,
        watcher: None,
        watch_task: None,
        reminder_task: None,
        reminders: Vec::new(),
        pending_refresh: None,
        pending_index: None,
        pending_config: None,
        pending_notes: None,
        pending_reload: None,
    });
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(30)).await;
            cx.update(|cx| act(cx, |l| l.tick()));
        }
    })
    .detach();
}

/// Runs a change on the library, then rescans, schedules the toast timeout and syncs reminders.
pub fn act<R>(cx: &mut App, f: impl FnOnce(&mut Library) -> R) -> R {
    let lib = lib(cx);
    let before = lib.read(cx).toast.as_ref().map(|t| t.id);
    let out = lib.update(cx, |l, cx| {
        let out = f(l);
        cx.notify();
        out
    });
    if lib.update(cx, |l, _| l.take_rows_moved()) {
        super::motion::arm_rows(cx);
    }
    let toast = lib.read(cx).toast.as_ref().map(|t| (t.id, t.undo.is_some()));
    if let Some((id, undoable)) = toast
        && Some(id) != before
    {
        let lib = lib.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(if undoable { 6 } else { 3 })).await;
            while cx.update(|cx| lib.read(cx).toast_hovered) {
                cx.background_executor().timer(Duration::from_millis(500)).await;
            }
            cx.update(|cx| lib.update(cx, |l, cx| {
                l.dismiss_toast(id);
                cx.notify();
            }));
        })
        .detach();
    }
    scan(cx);
    sync_reminders(cx);
    out
}

/// Reads without changing anything.
pub fn read<R>(cx: &App, f: impl FnOnce(&Library) -> R) -> R {
    f(lib(cx).read(cx))
}

/// Lists assignment folders and the inbox off the main thread, then re-evaluates.
pub fn scan(cx: &mut App) {
    let lib = lib(cx);
    let Some(job) = lib.update(cx, |l, _| l.scan_job()) else { return };
    let task = cx.background_spawn(async move { job.run() });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            lib.update(cx, |l, cx| {
                l.apply_scan(result);
                cx.notify();
            });
            sync_reminders(cx);
        });
    })
    .detach();
}

/// Updates the search index and catalog in the background.
pub fn reindex(cx: &mut App) {
    let lib = lib(cx);
    let Some(job) = lib.update(cx, |l, _| l.index_job()) else { return };
    let task = cx.background_spawn(async move { job.run() });
    cx.spawn(async move |cx| {
        let (count, catalog) = task.await;
        cx.update(|cx| {
            lib.update(cx, |l, cx| {
                l.apply_index(count, catalog);
                cx.notify();
            })
        });
    })
    .detach();
}

/// Opens a workspace, starts watching it and indexes it.
pub fn open_workspace(path: &Path, cx: &mut App) {
    if !path.is_dir() {
        return;
    }
    act(cx, |l| l.open(path));
    watch(path.to_path_buf(), cx);
    reindex(cx);
}

pub fn close_workspace(cx: &mut App) {
    flush_pending(cx);
    let store = cx.global_mut::<Store>();
    store.watcher = None;
    store.watch_task = None;
    act(cx, |l| l.close());
}

#[derive(Default, Clone, Copy)]
struct Change {
    structural: bool,
    content: bool,
    config: bool,
}

fn classify(event: &notify::Event) -> Change {
    let mut c = Change::default();
    for p in &event.paths {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name == CONFIG_NAME || name == STATE_NAME {
            c.config = true;
            continue;
        }
        if p.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) || name.starts_with('~') {
            continue;
        }
        match event.kind {
            EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_)) => c.structural = true,
            EventKind::Modify(_) => c.content = true,
            _ => {}
        }
    }
    c
}

/// Reacts to disk changes: new, removed or renamed files re-evaluate deadlines; edits to existing files
/// (e.g. note autosaves) only refresh the search index, later and without touching deadlines.
fn watch(root: PathBuf, cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<Change>();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(e) = res {
            let c = classify(&e);
            if c.structural || c.content || c.config {
                let _ = tx.send_blocking(c);
            }
        }
    });
    let Ok(mut watcher) = watcher else { return };
    if watcher.watch(&root, RecursiveMode::Recursive).is_err() {
        return;
    }
    let task = cx.spawn(async move |cx| {
        while let Ok(change) = rx.recv().await {
            let mut c = change;
            while let Ok(more) = rx.try_recv() {
                c.structural |= more.structural;
                c.content |= more.content;
                c.config |= more.config;
            }
            cx.update(|cx| changed(c, cx));
        }
    });
    let store = cx.global_mut::<Store>();
    store.watcher = Some(watcher);
    store.watch_task = Some(task);
}

fn changed(c: Change, cx: &mut App) {
    if c.structural || c.config {
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            cx.update(|cx| {
                let store = cx.global::<Store>();
                let busy = store.pending_config.is_some() || store.pending_notes.is_some();
                if c.config && !busy {
                    act(cx, |l| l.reload_external());
                }
                if c.structural {
                    act(cx, |l| l.refresh());
                }
                reload_buffers(cx);
            });
        });
        cx.global_mut::<Store>().pending_refresh = Some(task);
    } else if c.content {
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(RELOAD_SETTLE)).await;
            cx.update(reload_buffers);
        });
        cx.global_mut::<Store>().pending_reload = Some(task);
    }
    if c.structural || c.content {
        let delay = if c.structural { 3 } else { 10 };
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(delay)).await;
            cx.update(reindex);
        });
        cx.global_mut::<Store>().pending_index = Some(task);
    }
}

/// Picks up note edits made in other apps.
fn reload_buffers(cx: &mut App) {
    let lib = lib(cx);
    lib.update(cx, |l, cx| {
        if l.buffers.values_mut().fold(false, |any, b| b.reload_if_changed() || any) {
            cx.notify();
        }
    });
}

/// Writes the config shortly after the last settings change.
pub fn schedule_config_save(cx: &mut App) {
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_millis(250)).await;
        cx.update(|cx| act(cx, |l| l.refresh()));
        cx.background_executor().timer(Duration::from_millis(350)).await;
        cx.update(|cx| {
            act(cx, |l| l.write_config());
            cx.global_mut::<Store>().pending_config = None;
        });
    });
    cx.global_mut::<Store>().pending_config = Some(task);
}

/// Saves assignment notes 0.7 s after the last keystroke.
pub fn schedule_notes_save(cx: &mut App) {
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_millis(700)).await;
        cx.update(|cx| {
            lib(cx).update(cx, |l, _| l.save_notes());
            cx.global_mut::<Store>().pending_notes = None;
        });
    });
    cx.global_mut::<Store>().pending_notes = Some(task);
}

/// Writes what is still waiting for its delay, before quitting or leaving a workspace.
pub fn flush_pending(cx: &mut App) {
    if cx.global_mut::<Store>().pending_notes.take().is_some() {
        lib(cx).update(cx, |l, _| l.save_notes());
    }
    if cx.global_mut::<Store>().pending_config.take().is_some() {
        lib(cx).update(cx, |l, _| l.write_config());
    }
}

/// Keeps one timer for the next due reminder; rebuilt only when the list changes.
/// Also refreshes the subscribable calendar file when that is turned on.
/// Schedules all reminders again, e.g. after the system allowed notifications.
pub fn reschedule_reminders(cx: &mut App) {
    cx.global_mut::<Store>().reminders.clear();
    sync_reminders(cx);
}

pub fn sync_reminders(cx: &mut App) {
    if crate::core::prefs::get().calendar_feed
        && let (Some(root), Some(ics)) = read(cx, |l| (l.root.clone(), l.ics()))
    {
        let path = root.join("Schrumbi-Kalender.ics");
        let strip = |s: &str| s.lines().filter(|l| !l.starts_with("DTSTAMP:")).collect::<Vec<_>>().join("\n");
        if std::fs::read_to_string(&path).map_or(true, |old| strip(&old) != strip(&ics)) {
            let _ = std::fs::write(&path, ics);
        }
    }
    let reminders = read(cx, |l| l.reminders());
    if reminders == cx.global::<Store>().reminders {
        return;
    }
    let pending = crate::ui::notifications::schedule(&reminders);
    let task = cx.spawn(async move |cx| {
        for r in pending {
            let wait = (r.fire - chrono::Local::now()).to_std().unwrap_or_default();
            cx.background_executor().timer(wait).await;
            cx.update(|cx| crate::ui::notifications::show(r, cx));
        }
    });
    let store = cx.global_mut::<Store>();
    store.reminders = reminders;
    store.reminder_task = Some(task);
}

/// Changes the config in memory right away and writes it to disk shortly after.
pub fn edit_config(cx: &mut App, f: impl FnOnce(&mut crate::core::model::StudyConfig)) {
    let Some(mut c) = read(cx, |l| l.config.clone()) else { return };
    f(&mut c);
    let changed = lib(cx).update(cx, |l, cx| {
        let before = l.config_snapshot();
        let changed = l.update_config(c);
        if let (true, Some(before)) = (changed, before) {
            l.record_edit("config", crate::core::i18n::tr("Einstellung geändert"), before);
        }
        cx.notify();
        changed
    });
    if changed {
        schedule_config_save(cx);
    }
}
