use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui_kit::*;

use super::motion;
use super::state::{self, act};
use crate::core::git::cmd::{Git, GitError};
use crate::core::git::conflict::{Choice, ConflictFile, Resolved};
use crate::core::git::status::RepoStatus;
use crate::core::git::sync::{Report, Step};
use crate::core::git::{locate, message};
use crate::core::i18n::{tr, trf};
use crate::core::prefs::{self, GitMode};

/// How often the remote is looked at while a window is active, and in the background.
const FETCH_ACTIVE: Duration = Duration::from_secs(5 * 60);
const FETCH_IDLE: Duration = Duration::from_secs(30 * 60);
/// Waits between attempts while offline.
const BACKOFF: [u64; 4] = [60, 120, 300, 900];
/// How long a spinner stays once it appeared, so nothing flickers.
pub const MIN_WORK: Duration = Duration::from_millis(400);

/// Everything sync knows about the open workspace.
#[derive(Default)]
pub struct Sync {
    pub root: Option<PathBuf>,
    pub git: Option<Git>,
    /// A `.git` folder exists but sync was not set up here.
    pub offer: bool,
    pub status: Option<RepoStatus>,
    pub step: Option<Step>,
    pub error: Option<GitError>,
    pub offline: bool,
    /// The workspace has no remote, which is how it was set up: states are only saved here.
    pub local_only: bool,
    /// When the newest state was made, as a Unix time.
    pub last_commit: Option<i64>,
    pub conflicts: Vec<ConflictFile>,
    pub automatic: Vec<(String, String)>,
    pub too_large: Vec<String>,
    pub nested: Vec<String>,
    /// Text typed for the next state; survives closing the panel.
    pub message: String,
    /// Bumped when a manual sync ended well, for the confirmation at the row.
    pub finished: u64,
    /// Bumped after every sync that ended well, so lists of saved states can load again.
    pub saved: u64,
    /// When the running sync started, for showing progress only on long uploads.
    pub started: Option<Instant>,
    /// The user confirmed that many files gone at once may be recorded by the next sync.
    pub deletions_allowed: bool,
    /// The last resolution, for taking it back.
    pub resolved: Option<Resolved>,
    pending: Option<Option<String>>,
    last_change: Option<Instant>,
    last_fetch: Option<Instant>,
    retry_at: Option<Instant>,
    attempts: usize,
    announced: Option<GitError>,
    generation: u64,
    refresh_task: Option<Task<()>>,
    pub panel: Option<AnyView>,
}

impl Global for Sync {}

pub fn get(cx: &App) -> &Sync {
    cx.global::<Sync>()
}

fn enabled(root: &Path) -> bool {
    prefs::get().git.get(&root.to_string_lossy().to_string()).is_some_and(|g| g.enabled)
}

/// This workspace's settings, by value.
pub fn settings(cx: &App) -> Option<prefs::GitPrefs> {
    let root = get(cx).root.as_ref()?;
    prefs::get().git.get(&root.to_string_lossy().to_string()).cloned()
}

pub fn update_settings(cx: &mut App, f: impl FnOnce(&mut prefs::GitPrefs)) {
    let Some(root) = get(cx).root.clone() else { return };
    prefs::update(|p| f(p.git.entry(root.to_string_lossy().to_string()).or_default()));
    cx.refresh_windows();
}

pub fn init(cx: &mut App) {
    cx.set_global(Sync::default());
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(20)).await;
            cx.update(tick);
        }
    })
    .detach();
}

/// Decides what is due: a retry, an automatic sync after a quiet time, or a look at the remote.
fn tick(cx: &mut App) {
    let s = get(cx);
    if s.git.is_none() || s.step.is_some() {
        return;
    }
    let Some(settings) = settings(cx) else { return };
    let s = get(cx);
    let automatic = settings.mode == GitMode::Auto;
    let now = Instant::now();
    if s.offline && s.retry_at.is_some_and(|at| now >= at) {
        run(None, false, cx);
        return;
    }
    let quiet = Duration::from_secs(settings.idle_minutes.max(1) as u64 * 60);
    if automatic && s.error == Some(GitError::Locked) && s.started.is_some_and(|at| now.duration_since(at) > Duration::from_secs(20)) {
        run(None, false, cx);
        return;
    }
    if automatic && s.conflicts.is_empty() && s.error.is_none() && s.last_change.is_some_and(|c| now.duration_since(c) >= quiet) {
        run(None, false, cx);
        return;
    }
    let active = cx.active_window().is_some();
    let every = if active { FETCH_ACTIVE } else { FETCH_IDLE };
    if !s.offline && s.last_fetch.is_none_or(|f| now.duration_since(f) >= every) {
        fetch_only(cx);
    }
}

/// Starts following a workspace. Git only ever runs here after the user set sync up.
pub fn workspace_opened(path: &Path, cx: &mut App) {
    let root = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let is_on = enabled(path);
    let hidden = prefs::get().git.get(&path.to_string_lossy().to_string()).is_some_and(|g| g.offer_hidden);
    let has_repo = root.join(".git").exists();
    let on_open = settings_for(path).is_some_and(|g| g.sync_on_open && g.mode == GitMode::Auto);
    let s = cx.global_mut::<Sync>();
    *s = Sync { root: Some(path.to_path_buf()), offer: has_repo && !is_on && !hidden, panel: s.panel.take(), ..Default::default() };
    if !is_on {
        cx.refresh_windows();
        return;
    }
    connect(root, on_open, cx);
}

fn settings_for(path: &Path) -> Option<prefs::GitPrefs> {
    prefs::get().git.get(&path.to_string_lossy().to_string()).cloned()
}

/// Finds Git in the background, then reads the status and optionally syncs.
fn connect(root: PathBuf, sync_after: bool, cx: &mut App) {
    let preferred = prefs::get().git_path;
    let task = cx.background_spawn(async move { locate::find(preferred.as_deref()) });
    cx.spawn(async move |cx| {
        let found = task.await;
        cx.update(|cx| match found {
            Ok(exe) => {
                cx.global_mut::<Sync>().git = Some(Git::new(exe, &root));
                cx.global_mut::<Sync>().error = None;
                if sync_after {
                    run(None, false, cx);
                } else {
                    refresh(cx);
                }
            }
            Err(e) => {
                cx.global_mut::<Sync>().error = Some(e);
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

pub fn workspace_closed(cx: &mut App) {
    let panel = cx.global_mut::<Sync>().panel.take();
    cx.set_global(Sync { panel, ..Default::default() });
}

/// Turns sync on for the open workspace.
pub fn enable(mode: GitMode, cx: &mut App) {
    update_settings(cx, |g| {
        g.enabled = true;
        g.mode = mode;
        g.offer_hidden = false;
    });
    let Some(root) = get(cx).root.clone() else { return };
    let root = dunce::canonicalize(&root).unwrap_or(root);
    cx.global_mut::<Sync>().offer = false;
    connect(root, true, cx);
}

/// Stops syncing; the repository stays as it is.
pub fn disable(cx: &mut App) {
    update_settings(cx, |g| g.enabled = false);
    let s = cx.global_mut::<Sync>();
    s.git = None;
    s.status = None;
    s.error = None;
    s.offline = false;
    s.conflicts.clear();
    s.offer = s.root.as_ref().is_some_and(|r| r.join(".git").exists());
    cx.refresh_windows();
}

pub fn hide_offer(cx: &mut App) {
    update_settings(cx, |g| g.offer_hidden = true);
    cx.global_mut::<Sync>().offer = false;
    cx.refresh_windows();
}

/// Reads the status again in the background; what is shown stays until the answer is there.
pub fn refresh(cx: &mut App) {
    let Some(git) = get(cx).git.clone() else { return };
    let generation = get(cx).generation;
    let task = cx.background_spawn(async move { git.status().map(|status| (status, git.last_commit_time(), git.has_remote())) });
    let job = cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let s = cx.global_mut::<Sync>();
            if s.generation != generation || s.step.is_some() {
                return;
            }
            match result {
                Ok((status, last_commit, remote)) => {
                    s.status = Some(status);
                    s.last_commit = last_commit;
                    s.local_only = !remote;
                }
                Err(e) => s.error = Some(e),
            }
            cx.refresh_windows();
        });
    });
    cx.global_mut::<Sync>().refresh_task = Some(job);
}

/// A file changed: remembers when, for the automatic sync, and updates the list soon.
pub fn touched(cx: &mut App) {
    if get(cx).git.is_none() {
        return;
    }
    cx.global_mut::<Sync>().last_change = Some(Instant::now());
    let job = cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_secs(2)).await;
        cx.update(refresh);
    });
    cx.global_mut::<Sync>().refresh_task = Some(job);
}

/// Something important was filed: with automatic sync it goes up right away.
pub fn soon(cx: &mut App) {
    if get(cx).git.is_some() && settings(cx).is_some_and(|g| g.mode == GitMode::Auto) {
        run(None, false, cx);
    }
}

fn modules(cx: &App) -> Vec<(String, String)> {
    state::read(cx, |l| l.config.as_ref().map(|c| c.modules.iter().map(|m| (m.folder.clone(), m.code.clone())).collect()).unwrap_or_default())
}

fn fetch_only(cx: &mut App) {
    let Some(git) = get(cx).git.clone() else { return };
    cx.global_mut::<Sync>().last_fetch = Some(Instant::now());
    let task = cx.background_spawn(async move { git.run_network(&["fetch", "-q"]).map(|_| ()) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            match result {
                Ok(()) => cx.global_mut::<Sync>().offline = false,
                Err(GitError::Offline | GitError::Timeout) => cx.global_mut::<Sync>().offline = true,
                Err(_) => {}
            }
            refresh(cx);
        });
    })
    .detach();
}

/// Syncs once and records many files gone at once, after the user confirmed it.
pub fn run_anyway(cx: &mut App) {
    if get(cx).git.is_none() {
        return;
    }
    cx.global_mut::<Sync>().deletions_allowed = true;
    run(None, true, cx);
}

/// One sync. A second request while one runs is remembered and follows it.
pub fn run(message: Option<String>, manual: bool, cx: &mut App) {
    let Some(git) = get(cx).git.clone() else { return };
    if get(cx).step.is_some() {
        cx.global_mut::<Sync>().pending = Some(message);
        return;
    }
    act(cx, |l| l.begin_sync());
    let modules = modules(cx);
    let s = cx.global_mut::<Sync>();
    let allowed = std::mem::take(&mut s.deletions_allowed);
    s.step = Some(Step::Saving);
    s.started = Some(Instant::now());
    crate::core::git::cmd::UPLOAD_PERCENT.store(0, std::sync::atomic::Ordering::Relaxed);
    s.error = None;
    s.last_change = None;
    s.retry_at = None;
    s.generation += 1;
    let started = Instant::now();
    let (tx, rx) = async_channel::unbounded::<Step>();
    let custom = message.filter(|m| !m.trim().is_empty());
    let vault = state::read(cx, |l| l.external_vault()).filter(|v| v.join(".git").exists()).map(|v| git.for_folder(&v));
    let vault_modules = modules.clone();
    let task = cx.background_spawn(async move {
        let result = git.sync(
            &|files| custom.clone().unwrap_or_else(|| message::auto_message(files, &modules, chrono::Local::now())),
            &|step| {
                let _ = tx.send_blocking(step);
            },
            allowed,
        );
        let conflicts = if matches!(result, Err(GitError::Conflicts(_))) { git.conflicts().ok() } else { None };
        let vault_error = vault.and_then(|v| v.sync(&|files| message::auto_message(files, &vault_modules, chrono::Local::now()), &|_| {}, allowed).err());
        (result, conflicts, vault_error)
    });
    cx.refresh_windows();
    cx.spawn(async move |cx| {
        while let Ok(step) = rx.recv().await {
            cx.update(|cx| {
                cx.global_mut::<Sync>().step = Some(step);
                cx.refresh_windows();
            });
        }
        let (result, conflicts, vault_error) = task.await;
        if let Some(e) = vault_error {
            cx.update(|cx| act(cx, |l| l.last_error = Some(trf("Der Vault konnte nicht synchronisiert werden: %@", &[&explain(&e).0]))));
        }
        let spent = started.elapsed();
        if spent > Duration::from_millis(300) && spent < MIN_WORK {
            cx.background_executor().timer(MIN_WORK - spent).await;
        }
        cx.update(|cx| finished(result, conflicts, manual, cx));
    })
    .detach();
}

fn finished(result: Result<Report, GitError>, conflicts: Option<(Vec<ConflictFile>, Vec<(String, String)>)>, manual: bool, cx: &mut App) {
    let pulled = result.as_ref().map(|r| r.pulled.clone()).unwrap_or_default();
    motion::arm_rows(cx);
    act(cx, |l| l.finish_sync(&pulled));
    if !pulled.is_empty() {
        state::reindex(cx);
    }
    let s = cx.global_mut::<Sync>();
    s.step = None;
    match &result {
        Ok(report) => {
            s.saved += 1;
            s.error = None;
            s.announced = None;
            s.conflicts.clear();
            s.automatic.clear();
            s.too_large = report.too_large.clone();
            s.nested = report.nested.clone();
            s.offline = report.offline;
            s.local_only = report.local_only;
            s.last_fetch = Some(Instant::now());
            if report.committed > 0 {
                s.message.clear();
            }
            if report.offline {
                s.retry_at = Some(Instant::now() + Duration::from_secs(BACKOFF[s.attempts.min(BACKOFF.len() - 1)]));
                s.attempts += 1;
            } else {
                s.attempts = 0;
                if manual {
                    s.finished += 1;
                }
            }
        }
        Err(GitError::Conflicts(_)) => {
            let (files, automatic) = conflicts.unwrap_or_default();
            s.conflicts = files;
            s.automatic = automatic;
            announce(GitError::Conflicts(Vec::new()), tr("Konflikte beim Synchronisieren"), trf("%@ Dateien wurden auf beiden Geräten geändert.", &[&get(cx).conflicts.len().to_string()]), cx);
        }
        Err(e) => {
            s.error = Some(e.clone());
            if *e == GitError::Auth {
                announce(GitError::Auth, tr("Anmeldung nötig"), tr("Der Online-Speicher lehnt die Anmeldung ab. Deine Änderungen sind lokal gesichert."), cx);
            }
        }
    }
    stamp(&result, cx);
    report_to_user(&result, &pulled, manual, cx);
    refresh(cx);
    if let Some(next) = cx.global_mut::<Sync>().pending.take() {
        run(next, manual, cx);
    }
    cx.refresh_windows();
}

fn stamp(result: &Result<Report, GitError>, cx: &mut App) {
    if result.as_ref().is_ok_and(|r| !r.offline) {
        let now = chrono::Local::now().timestamp();
        update_settings(cx, |g| g.last_sync = Some(now));
    }
}

/// Quiet by default: automatic syncs only speak up when something visible changed.
fn report_to_user(result: &Result<Report, GitError>, pulled: &[PathBuf], manual: bool, cx: &mut App) {
    let Ok(report) = result else { return };
    let open = state::read(cx, |l| l.active_document.clone());
    if let Some(open) = open
        && pulled.iter().any(|p| *p == open)
    {
        act(cx, |l| l.notify(tr("Aktualisiert vom anderen Gerät"), "cloud", None, None));
    } else if manual && !pulled.is_empty() {
        let text = if pulled.len() == 1 { tr("1 Datei übernommen") } else { trf("%@ Dateien übernommen", &[&pulled.len().to_string()]) };
        act(cx, |l| l.notify(text, "arrow-down", None, None));
    } else if manual && report.committed > 0 {
        let text = if report.committed == 1 { tr("Gesichert · 1 Datei") } else { trf("Gesichert · %@ Dateien", &[&report.committed.to_string()]) };
        act(cx, |l| l.notify(text, "check", None, None));
    } else if manual && !report.pushed && !report.offline {
        act(cx, |l| l.notify(tr("Nichts Neues"), "check", None, None));
    }
}

/// One system message per problem, never for being offline.
fn announce(error: GitError, title: String, body: String, cx: &mut App) {
    if get(cx).announced.as_ref() == Some(&error) {
        return;
    }
    cx.global_mut::<Sync>().announced = Some(error);
    if cx.active_window().is_some() || crate::core::drive::fenced("notification", &title) {
        return;
    }
    cx.background_spawn(async move {
        let _ = notify_rust::Notification::new().appname("Schrumbi").summary(&title).body(&body).show();
    })
    .detach();
}

/// The repository a file belongs to: the workspace's or the connected vault's.
pub fn git_for(path: &Path, cx: &App) -> Option<Git> {
    let git = get(cx).git.clone()?;
    let full = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if full.starts_with(&git.root) {
        return Some(git);
    }
    let vault = state::read(cx, |l| l.external_vault())?;
    let vault = dunce::canonicalize(&vault).unwrap_or(vault);
    (full.starts_with(&vault) && vault.join(".git").exists()).then(|| git.for_folder(&vault))
}

/// Whether this workspace syncs at all.
pub fn enabled_here(cx: &App) -> bool {
    get(cx).git.is_some()
}

/// What the tray should say about syncing, only when it needs attention: conflicts, or nothing uploaded for over a day.
pub fn tray_warning(cx: &App) -> Option<String> {
    let s = get(cx);
    if !s.conflicts.is_empty() {
        return Some(if s.conflicts.len() == 1 { tr("1 Konflikt") } else { trf("%@ Konflikte", &[&s.conflicts.len().to_string()]) });
    }
    let status = s.status.as_ref()?;
    let waiting = status.ahead > 0 || !status.changed.is_empty();
    let stale = settings(cx).and_then(|g| g.last_sync).is_some_and(|t| (chrono::Local::now().timestamp() - t) > 86_400);
    (s.offline && waiting && stale).then(|| tr("Seit über einem Tag nicht hochgeladen"))
}

/// Files with changes not yet saved as a state; shown only when syncing by hand.
pub fn unsaved_files(cx: &App) -> std::collections::HashSet<PathBuf> {
    let s = get(cx);
    let (Some(status), Some(root)) = (s.status.as_ref(), s.root.as_ref()) else { return Default::default() };
    if settings_for(root).is_none_or(|g| g.mode != GitMode::Manual) {
        return Default::default();
    }
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.clone());
    status.changed.iter().map(|c| root.join(&c.path)).collect()
}

/// Applies the choices for all conflicts and syncs on.
pub fn resolve(choices: std::collections::HashMap<String, Choice>, done: impl FnOnce(Result<(), GitError>, &mut App) + 'static, cx: &mut App) {
    let Some(git) = get(cx).git.clone() else { return };
    let automatic = get(cx).automatic.clone();
    act(cx, |l| l.begin_sync());
    cx.global_mut::<Sync>().step = Some(Step::Merging);
    cx.refresh_windows();
    let message = tr("Konflikte gelöst");
    let task = cx.background_spawn(async move { git.merge_resolved(&choices, &automatic, &message) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let pulled = result.as_ref().map(|r| r.pulled.clone()).unwrap_or_default();
            motion::arm_rows(cx);
            act(cx, |l| l.finish_sync(&pulled));
            state::reindex(cx);
            let s = cx.global_mut::<Sync>();
            s.step = None;
            match result {
                Ok(resolved) => {
                    let copies = resolved.copies.len();
                    s.conflicts.clear();
                    s.automatic.clear();
                    s.announced = None;
                    s.resolved = Some(resolved);
                    let text = match copies {
                        0 => tr("Konflikte gelöst"),
                        1 => tr("Konflikte gelöst, 1 Kopie angelegt"),
                        n => trf("Konflikte gelöst, %@ Kopien angelegt", &[&n.to_string()]),
                    };
                    let undo: crate::core::library::Undo = Box::new(|l| l.sync_undo_requested = true);
                    act(cx, |l| l.notify(text, "git-merge", Some(tr("Rückgängig")), Some(undo)));
                    done(Ok(()), cx);
                    run(None, false, cx);
                }
                Err(e) => done(Err(e), cx),
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Takes the last resolution back as a new state and uploads that.
pub fn undo_resolution(cx: &mut App) {
    let Some(git) = get(cx).git.clone() else { return };
    let Some(resolved) = cx.global_mut::<Sync>().resolved.take() else { return };
    act(cx, |l| l.begin_sync());
    let message = tr("Konfliktlösung rückgängig");
    let paths: Vec<PathBuf> = resolved.touched.iter().map(|p| git.root.join(p)).collect();
    let worker = git.clone();
    let task = cx.background_spawn(async move { worker.undo_resolution(&resolved, &message) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            act(cx, |l| l.finish_sync(&paths));
            match result {
                Ok(()) => run(None, false, cx),
                Err(e) => cx.global_mut::<Sync>().error = Some(e),
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Called from the app's quit path: one last save, bounded so quitting never hangs.
pub fn quit_after(done: impl FnOnce(&mut App) + 'static, cx: &mut App) {
    let wants = get(cx).git.is_some() && settings(cx).is_some_and(|g| g.mode == GitMode::Auto && g.sync_on_quit) && get(cx).status.as_ref().is_some_and(|s| !s.changed.is_empty() || s.ahead > 0);
    if !wants {
        done(cx);
        return;
    }
    run(None, false, cx);
    cx.spawn(async move |cx| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) && cx.update(|cx| get(cx).step.is_some()) {
            cx.background_executor().timer(Duration::from_millis(50)).await;
        }
        cx.update(done);
    })
    .detach();
}

/// What to say about an error: what happened and what helps.
pub fn explain(error: &GitError) -> (String, Option<&'static str>) {
    match error {
        GitError::Offline => (tr("Keine Verbindung. Deine Änderungen sind lokal gesichert und werden später hochgeladen."), Some("retry")),
        GitError::Auth => (tr("Der Online-Speicher lehnt die Anmeldung ab. Prüfe deinen SSH-Schlüssel oder deine Zugangsdaten."), Some("retry")),
        GitError::NoRemote => (tr("Dieser Workspace hat noch keinen Online-Speicher."), Some("setup")),
        GitError::NoIdentity => (tr("Git braucht einen Namen und eine E-Mail für deine Stände."), Some("setup")),
        GitError::Locked => (tr("Ein anderes Programm arbeitet gerade am Verlauf. Versuche es gleich noch einmal. Bleibt das so, liegt noch die Datei .git/index.lock im Ordner, die du dann löschen kannst."), Some("retry")),
        GitError::Rejected => (tr("Ein anderes Gerät war gerade schneller. Versuche es noch einmal."), Some("retry")),
        GitError::Detached | GitError::ExternalMerge => (tr("Der Verlauf wird gerade anders bearbeitet, zum Beispiel im Terminal. Schrumbi wartet, bis das abgeschlossen ist."), Some("reveal")),
        GitError::Missing => (tr("Für den Sync wird Git benötigt. Installiere es und starte Schrumbi neu."), Some("guide")),
        GitError::TooOld(v) => (trf("Git %@ ist zu alt, nötig ist mindestens 2.38.", &[v]), Some("guide")),
        GitError::Timeout => (tr("Der Online-Speicher antwortet nicht. Deine Änderungen sind lokal gesichert."), Some("retry")),
        GitError::Conflicts(_) => (tr("Dateien wurden auf beiden Geräten geändert."), None),
        GitError::ManyDeleted(n) => (trf("%@ Dateien fehlen auf einmal, deshalb pausiert das Sichern. War das gewollt, klicke auf „Trotzdem sichern“.", &[&n.to_string()]), Some("anyway")),
        GitError::Other(m) if m.contains("dubious ownership") => (tr("Git vertraut diesem Ordner noch nicht, oft bei externen Laufwerken. Erlaube ihn einmal mit „git config --global --add safe.directory“ und dem Ordnerpfad."), Some("retry")),
        GitError::Other(_) => (tr("Das Synchronisieren hat nicht geklappt."), Some("retry")),
    }
}

/// The name of what is being done right now.
pub fn step_label(step: Step) -> String {
    tr(match step {
        Step::Saving => "Sichert…",
        Step::Fetching => "Holt…",
        Step::Merging => "Übernimmt…",
        Step::Uploading => "Lädt hoch…",
    })
}
