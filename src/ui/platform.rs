use std::path::PathBuf;

use gpui_kit::{App, AppContext, PathPromptOptions};

use super::state::{self, act};
use crate::core::i18n::tr;

/// "Im Finder zeigen" / "Im Explorer zeigen" / "Im Dateimanager zeigen".
pub fn reveal_label() -> String {
    if cfg!(target_os = "macos") {
        tr("Im Finder zeigen")
    } else if cfg!(target_os = "windows") {
        tr("Im Explorer zeigen")
    } else {
        tr("Im Dateimanager zeigen")
    }
}

/// Desktop notification.
pub fn notify(title: &str, body: &str) {
    let _ = notify_rust::Notification::new().appname("Schrumbi").summary(title).body(body).show();
}

/// Shows an open panel and hands the chosen paths to `then`.
pub fn pick(options: PathPromptOptions, cx: &mut App, then: impl FnOnce(Vec<PathBuf>, &mut App) + 'static) {
    let rx = cx.prompt_for_paths(options);
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = rx.await {
            cx.update(|cx| then(paths, cx));
        }
    })
    .detach();
}

/// Shows a save panel and hands the chosen path to `then`.
pub fn save(directory: PathBuf, name: String, cx: &mut App, then: impl FnOnce(PathBuf, &mut App) + 'static) {
    let rx = cx.prompt_for_new_path(&directory, Some(&name));
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(path))) = rx.await {
            cx.update(|cx| then(path, cx));
        }
    })
    .detach();
}

fn files_only(multiple: bool) -> PathPromptOptions {
    PathPromptOptions { files: true, directories: false, multiple, prompt: None }
}

pub fn choose_folder(cx: &mut App, then: impl FnOnce(PathBuf, &mut App) + 'static) {
    pick(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(tr("Auswählen").into()) }, cx, move |mut p, cx| {
        if let Some(path) = p.pop() {
            then(path, cx);
        }
    });
}

pub fn pick_files_for_filing(assignment_id: Option<String>, cx: &mut App) {
    pick(files_only(true), cx, move |paths, cx| act(cx, |l| l.request_filing(paths, assignment_id)));
}

pub fn pick_link_files(id: &str, cx: &mut App) {
    let id = id.to_string();
    pick(files_only(true), cx, move |paths, cx| act(cx, |l| l.link(&paths, &id)));
}

pub fn pick_calendar_import(cx: &mut App) {
    pick(files_only(false), cx, |mut paths, cx| {
        if let Some(p) = paths.pop() {
            act(cx, |l| l.import_path = Some(p));
        }
    });
}

pub fn export_calendar(cx: &mut App) {
    let (Some(ics), name) = state::read(cx, |l| (l.ics(), l.ics_file_name())) else { return };
    save(dirs::download_dir().unwrap_or_default(), name, cx, move |path, cx| match std::fs::write(&path, ics) {
        Ok(()) => act(cx, |l| l.notify(tr("Kalender exportiert"), "calendar", Some(tr("Öffnen")), Some(Box::new(move |_| crate::core::library::open_path(&path))))),
        Err(e) => act(cx, |l| l.last_error = Some(e.to_string())),
    });
}

/// Zips the whole workspace off the main thread.
pub fn export_archive(cx: &mut App) {
    let (root, name, busy) = state::read(cx, |l| (l.root.clone(), format!("{}-{}.zip", l.config.as_ref().map_or("Schrumbi", |c| c.name.as_str()), crate::core::day::format(l.today())), l.is_exporting));
    let Some(root) = root.filter(|_| !busy) else { return };
    save(dirs::download_dir().unwrap_or_default(), name, cx, move |dest, cx| {
        if dest.starts_with(&root) {
            act(cx, |l| l.last_error = Some(tr("Bitte einen Ort außerhalb des Studienordners wählen, sonst landet die ZIP in der nächsten Sicherung.")));
            return;
        }
        act(cx, |l| {
            l.is_exporting = true;
            l.notify(tr("Studium wird als ZIP exportiert …"), "archive", None, None);
        });
        let (r, d) = (root.clone(), dest.clone());
        let task = cx.background_spawn(async move { crate::core::library::export_archive(&r, &d) });
        cx.spawn(async move |cx| {
            let result = task.await;
            cx.update(|cx| {
                act(cx, |l| {
                    l.is_exporting = false;
                    match result {
                        Ok(()) => {
                            let d = dest.clone();
                            l.notify(crate::core::i18n::trf("ZIP gespeichert: %@", &[&dest.file_name().unwrap_or_default().to_string_lossy()]), "archive", Some(tr("Zeigen")), Some(Box::new(move |_| crate::core::library::reveal(&d))));
                        }
                        Err(e) => l.last_error = Some(crate::core::i18n::trf("Export fehlgeschlagen: %@", &[&e])),
                    }
                })
            });
        })
        .detach();
    });
}

/// Starts or stops launching at login.
pub fn set_launch_at_login(on: bool) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let Ok(auto) = auto_launch::AutoLaunchBuilder::new().set_app_name("Schrumbi").set_app_path(&exe.to_string_lossy()).build() else { return false };
    if on { auto.enable().is_ok() } else { auto.disable().is_ok() }
}

pub fn launches_at_login() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    auto_launch::AutoLaunchBuilder::new().set_app_name("Schrumbi").set_app_path(&exe.to_string_lossy()).build().and_then(|a| a.is_enabled()).unwrap_or(false)
}

/// "Studienordner im Finder zeigen" and its Windows / Linux variants.
pub fn reveal_workspace_label() -> String {
    if cfg!(target_os = "macos") {
        tr("Studienordner im Finder zeigen")
    } else if cfg!(target_os = "windows") {
        tr("Studienordner im Explorer zeigen")
    } else {
        tr("Studienordner im Dateimanager zeigen")
    }
}
