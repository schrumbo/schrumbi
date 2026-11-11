use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::{App, AppContext, Entity, Global, ScrollHandle, Task};
use notify::{EventKind, RecursiveMode, Watcher};

use crate::core::i18n::{tr, trf};
use crate::core::library::{Library, Reminder};
use crate::core::highlights;
use crate::core::workspace::{CONFIG_NAME, STATE_NAME};

/// Many apps empty a file before writing it; reading in between would show an empty note.
const RELOAD_SETTLE: u64 = 150;

const RESCAN_TICKS: u32 = 20;

/// The shared library plus the background work that keeps it current.
pub struct Store {
    pub lib: Entity<Library>,
    watcher: Option<notify::RecommendedWatcher>,
    index_again: bool,
    watch_task: Option<Task<()>>,
    reminder_task: Option<Task<()>>,
    reminders: Vec<Reminder>,
    pending_refresh: Option<Task<()>>,
    pending_index: Option<Task<()>>,
    pending_config: Option<Task<()>>,
    pending_notes: Option<Task<()>>,
    tab_scroll: ScrollHandle,
    page_wants_focus: bool,
    shown_tab: std::cell::RefCell<Option<std::path::PathBuf>>,
    pending_reload: Option<Task<()>>,
    ticks: u32,
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
        index_again: false,
        watch_task: None,
        reminder_task: None,
        reminders: Vec::new(),
        pending_refresh: None,
        pending_index: None,
        pending_config: None,
        pending_notes: None,
        tab_scroll: ScrollHandle::new(),
        page_wants_focus: false,
        shown_tab: Default::default(),
        pending_reload: None,
        ticks: 0,
    });
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(30)).await;
            cx.update(|cx| tick(cx));
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(3600)).await;
            cx.update(|cx| refresh_calendar(false, cx));
        }
    })
    .detach();
}

/// Moves the library's clock on and redraws once a minute or when a status changed; the folders are rescanned every ten minutes.
fn tick(cx: &mut App) {
    let lib = lib(cx);
    let changed = lib.update(cx, |l, cx| {
        let changed = l.tick();
        if changed {
            cx.notify();
        }
        changed
    });
    let store = cx.global_mut::<Store>();
    store.ticks += 1;
    let rescan = store.ticks % RESCAN_TICKS == 0;
    if rescan {
        scan(cx);
    }
    if changed {
        sync_reminders(cx);
    }
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
    let (undo_resolution, soon, touched) = lib.update(cx, |l, _| (std::mem::take(&mut l.sync_undo_requested), std::mem::take(&mut l.sync_soon), std::mem::take(&mut l.notes_touched)));
    if touched {
        reindex(cx);
    }
    if undo_resolution {
        super::sync::undo_resolution(cx);
    }
    if soon {
        super::sync::soon(cx);
    }
    time_toast(before, cx);
    scan(cx);
    sync_reminders(cx);
    out
}

fn time_toast(before: Option<u64>, cx: &mut App) {
    let lib = lib(cx);
    let toast = lib.read(cx).toast.as_ref().map(|t| (t.id, t.undo.is_some()));
    if let Some((id, undoable)) = toast
        && Some(id) != before
    {
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

/// Moves files to the trash off the main thread; they leave every list in the same frame.
pub fn trash(paths: Vec<PathBuf>, cx: &mut App) {
    if paths.is_empty() {
        return;
    }
    let paths = act(cx, |l| l.trashing(&paths));
    if paths.is_empty() {
        return;
    }
    let task = cx.background_spawn(async move { crate::core::library::trash_files(&paths) });
    cx.spawn(async move |cx| {
        let results = task.await;
        cx.update(|cx| {
            act(cx, |l| l.trashed(results));
            reindex(cx);
        });
    })
    .detach();
}

/// Updates the search index and catalog in the background.
pub fn reindex(cx: &mut App) {
    let lib = lib(cx);
    let Some(job) = lib.update(cx, |l, _| l.index_job()) else {
        cx.global_mut::<Store>().index_again = true;
        return;
    };
    let task = cx.background_spawn(async move { job.run() });
    cx.spawn(async move |cx| {
        let (count, catalog) = task.await;
        cx.update(|cx| {
            let before = lib.read(cx).toast.as_ref().map(|t| t.id);
            lib.update(cx, |l, cx| {
                l.apply_index(count, catalog);
                cx.notify();
            });
            time_toast(before, cx);
            if std::mem::take(&mut cx.global_mut::<Store>().index_again) {
                reindex(cx);
            }
        });
    })
    .detach();
}

/// Writes the notes still open in PDF views. When one cannot be written it stays open, the user is told, and the caller must not leave the folder or quit.
pub fn notes_settled(cx: &mut App) -> bool {
    if super::pdf_view::flush_notes(cx) {
        return true;
    }
    act(cx, |l| l.last_error = Some(tr("Eine PDF-Notiz ist noch offen und lässt sich gerade nicht speichern. Mit Esc verwirfst du sie.")));
    false
}

/// Opens a workspace, starts watching it and indexes it.
pub fn open_workspace(path: &Path, cx: &mut App) {
    if !path.is_dir() || !notes_settled(cx) {
        return;
    }
    act(cx, |l| l.open(path));
    watch(path.to_path_buf(), cx);
    reindex(cx);
    refresh_calendar(false, cx);
    super::sync::workspace_opened(path, cx);
}

/// Closes the open folder; `false` and nothing closed while a PDF note cannot be written.
pub fn close_workspace(cx: &mut App) -> bool {
    if !notes_settled(cx) {
        return false;
    }
    flush_pending(cx);
    let store = cx.global_mut::<Store>();
    store.watcher = None;
    store.watch_task = None;
    crate::core::vault::index::reset();
    super::sync::workspace_closed(cx);
    act(cx, |l| l.close());
    true
}

#[derive(Default, Clone, Copy)]
struct Change {
    structural: bool,
    content: bool,
    config: bool,
    history: bool,
    /// Obsidian's own settings changed, which only the vault index reads.
    vault_settings: bool,
}

fn classify(event: &notify::Event, roots: &[PathBuf]) -> Change {
    let mut c = Change::default();
    for p in &event.paths {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let text = p.to_string_lossy();
        if text.contains("/.git/") && (name == "HEAD" || text.contains("/.git/refs/")) {
            c.history = true;
            continue;
        }
        if name == CONFIG_NAME || name == STATE_NAME || name == highlights::FILE_NAME || name == highlights::ANNOTATIONS_NAME {
            c.config = true;
            continue;
        }
        let inside = roots.iter().find_map(|r| p.strip_prefix(r).ok()).unwrap_or(p);
        if inside.starts_with(".obsidian") && name.ends_with(".json") && !name.starts_with("workspace") {
            c.vault_settings = true;
            continue;
        }
        if inside.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) || name.starts_with('~') {
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
    let vault = read(cx, |l| l.external_vault());
    let mut roots = vec![dunce::canonicalize(&root).unwrap_or_else(|_| root.clone())];
    roots.extend(vault.iter().map(|v| dunce::canonicalize(v).unwrap_or_else(|_| v.clone())));
    roots.push(root.clone());
    roots.extend(vault.clone());
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(e) = res {
            let c = classify(&e, &roots);
            if c.structural || c.content || c.config || c.history || c.vault_settings {
                let _ = tx.send_blocking(c);
            }
        }
    });
    let Ok(mut watcher) = watcher else { return };
    if watcher.watch(&root, RecursiveMode::Recursive).is_err() {
        return;
    }
    if let Some(vault) = &vault {
        let _ = watcher.watch(vault, RecursiveMode::Recursive);
    }
    let task = cx.spawn(async move |cx| {
        while let Ok(change) = rx.recv().await {
            let mut c = change;
            while let Ok(more) = rx.try_recv() {
                c.structural |= more.structural;
                c.content |= more.content;
                c.config |= more.config;
                c.history |= more.history;
                c.vault_settings |= more.vault_settings;
            }
            cx.update(|cx| changed(c, cx));
        }
    });
    let store = cx.global_mut::<Store>();
    store.watcher = Some(watcher);
    store.watch_task = Some(task);
}

/// Watches a newly connected or removed vault too.
pub fn watch_vault(cx: &mut App) {
    if let Some(root) = read(cx, |l| l.root.clone()) {
        watch(root, cx);
    }
}

/// Coming back from another app: what happened there shows right away.
pub fn came_back(cx: &mut App) {
    reload_buffers(cx);
    cx.global_mut::<Store>().pending_index = None;
    reindex(cx);
    super::apple_calendar::recheck();
}

fn changed(c: Change, cx: &mut App) {
    if c.structural || c.content || c.config {
        super::sync::touched(cx);
    } else if c.history && super::sync::get(cx).step.is_none() {
        super::sync::refresh(cx);
    }
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
    if c.structural || c.content || c.vault_settings {
        if cx.global::<Store>().pending_index.is_some() {
            return;
        }
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            cx.update(|cx| {
                cx.global_mut::<Store>().pending_index = None;
                reindex(cx);
            });
        });
        cx.global_mut::<Store>().pending_index = Some(task);
    }
}

/// Picks up note edits made in other apps.
fn reload_buffers(cx: &mut App) {
    let lib = lib(cx);
    lib.update(cx, |l, cx| {
        if l.follow_outside_moves() {
            cx.notify();
        }
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

/// A closing dialog takes the keyboard back to where it was when it opened, also after its exit animation.
/// So the page asks for it twice: right away, and once the animation is over.
pub fn return_focus_to_page(cx: &mut App) {
    cx.global_mut::<Store>().page_wants_focus = true;
    lib(cx).update(cx, |_, cx| cx.notify());
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_millis(300)).await;
        cx.update(|cx| {
            cx.global_mut::<Store>().page_wants_focus = true;
            lib(cx).update(cx, |_, cx| cx.notify());
        });
    })
    .detach();
}

pub fn take_page_focus_request(cx: &mut App) -> bool {
    std::mem::take(&mut cx.global_mut::<Store>().page_wants_focus)
}

/// The tab bar's scroll position; scrolls to the tab at `index` when `current` just became the shown one.
pub fn tab_scroll(current: &std::path::Path, index: usize, cx: &App) -> ScrollHandle {
    let store = cx.global::<Store>();
    if store.shown_tab.borrow().as_deref() != Some(current) {
        *store.shown_tab.borrow_mut() = Some(current.to_path_buf());
        store.tab_scroll.scroll_to_item(index);
    }
    store.tab_scroll.clone()
}

/// Writes what is still waiting for its delay, before quitting or leaving a workspace.
pub fn flush_pending(cx: &mut App) {
    crate::core::cards::flush();
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

/// The calendar address this workspace fetches deadlines from.
pub fn calendar_source(cx: &App) -> Option<String> {
    let root = read(cx, |l| l.root.clone())?;
    crate::core::prefs::get().calendar_sources.get(root.to_string_lossy().as_ref()).filter(|u| !u.trim().is_empty()).cloned()
}

fn fetch_calendar(address: &str) -> Result<String, String> {
    let address = address.trim();
    let address = address.strip_prefix("webcal://").map_or_else(|| address.to_string(), |rest| format!("https://{rest}"));
    if !(address.starts_with("https://") || address.starts_with("http://")) {
        return Err(tr("Die Adresse muss mit https:// oder webcal:// beginnen."));
    }
    let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build());
    let text = agent.get(&address).call().map_err(|e| e.to_string())?.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if !text.contains("BEGIN:VCALENDAR") {
        return Err(tr("Unter dieser Adresse liegt kein Kalender."));
    }
    Ok(text)
}

/// Fetches the subscribed calendar. Quietly in the background; when asked, it opens the import to choose and reports errors.
pub fn refresh_calendar(asked: bool, cx: &mut App) {
    let Some(address) = calendar_source(cx) else { return };
    let task = cx.background_spawn(async move { fetch_calendar(&address) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(text) if asked => {
                let file = std::env::temp_dir().join(format!("schrumbi-kalender-{}.ics", std::process::id()));
                if std::fs::write(&file, text).is_ok() {
                    act(cx, |l| l.import_path = Some(file));
                }
            }
            Ok(text) => {
                act(cx, |l| l.refresh_events(&text));
            }
            Err(e) if asked => act(cx, |l| l.last_error = Some(trf("Kalender konnte nicht geladen werden: %@", &[&e]))),
            Err(_) => {}
        });
    })
    .detach();
}

pub fn sync_reminders(cx: &mut App) {
    if crate::core::prefs::get().calendar_feed
        && let (Some(root), Some(ics)) = read(cx, |l| (l.root.clone(), l.ics()))
    {
        let path = root.join("Schrumbi-Kalender.ics");
        crate::core::calendar_server::serve(path.clone());
        let strip = |s: &str| s.lines().filter(|l| !l.starts_with("DTSTAMP:")).collect::<Vec<_>>().join("\n");
        if std::fs::read_to_string(&path).map_or(true, |old| strip(&old) != strip(&ics)) {
            let _ = std::fs::write(&path, ics);
        }
    }
    super::apple_calendar::push(cx);
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

thread_local! {
    /// The input field being edited, so undo groups typing per field and not across fields.
    static EDITED_FIELD: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

pub fn set_edited_field(key: &str) {
    EDITED_FIELD.with(|f| key.clone_into(&mut f.borrow_mut()));
}

/// Changes the config in memory right away and writes it to disk shortly after.
pub fn edit_config(cx: &mut App, f: impl FnOnce(&mut crate::core::model::StudyConfig)) {
    let Some(mut c) = read(cx, |l| l.config.clone()) else { return };
    f(&mut c);
    let changed = lib(cx).update(cx, |l, cx| {
        let before = l.config_snapshot();
        let changed = l.update_config(c);
        if let (true, Some(before)) = (changed, before) {
            let field = EDITED_FIELD.with(|f| f.borrow().clone());
            l.record_edit(&format!("config:{field}"), crate::core::i18n::tr("Einstellung geändert"), before);
        }
        cx.notify();
        changed
    });
    if changed {
        schedule_config_save(cx);
    }
}
