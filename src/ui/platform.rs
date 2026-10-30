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

/// The file manager's name for settings labels.
pub fn markdown_label() -> String {
    if cfg!(target_os = "macos") { tr("Markdown im Finder") } else { tr("Markdown-Dateien") }
}

#[cfg(target_os = "macos")]
mod markdown_default {
    use std::ffi::c_void;

    use objc2_foundation::{NSBundle, NSString};

    const MARKDOWN: &str = "net.daringfireball.markdown";
    const ALL_ROLES: u32 = 0xFFFF_FFFF;

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyDefaultRoleHandlerForContentType(content_type: *const c_void, role: u32) -> *const NSString;
        fn LSSetDefaultRoleHandlerForContentType(content_type: *const c_void, role: u32, handler: *const c_void) -> i32;
    }

    /// `None` outside an app bundle, where there is nothing to register.
    pub fn get() -> Option<bool> {
        let own = NSBundle::mainBundle().bundleIdentifier()?;
        let kind = NSString::from_str(MARKDOWN);
        let current = unsafe { LSCopyDefaultRoleHandlerForContentType((&*kind as *const NSString).cast(), ALL_ROLES) };
        Some(unsafe { current.as_ref() }.is_some_and(|c| c.to_string().eq_ignore_ascii_case(&own.to_string())))
    }

    pub fn set() -> bool {
        let Some(own) = NSBundle::mainBundle().bundleIdentifier() else { return false };
        let kind = NSString::from_str(MARKDOWN);
        unsafe { LSSetDefaultRoleHandlerForContentType((&*kind as *const NSString).cast(), ALL_ROLES, (&*own as *const NSString).cast()) == 0 }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod markdown_default {
    /// `None` when `xdg-mime` is missing.
    pub fn get() -> Option<bool> {
        let out = std::process::Command::new("xdg-mime").args(["query", "default", "text/markdown"]).output().ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim() == "schrumbi.desktop")
    }

    pub fn set() -> bool {
        std::process::Command::new("xdg-mime").args(["default", "schrumbi.desktop", "text/markdown"]).status().is_ok_and(|s| s.success())
    }
}

#[cfg(windows)]
mod markdown_default {
    /// Windows doesn't let apps read or set this; the button opens the system dialog instead.
    pub fn get() -> Option<bool> {
        Some(false)
    }

    pub fn set() -> bool {
        let _ = opener::open("ms-settings:defaultapps");
        false
    }
}

pub use markdown_default::{get as markdown_is_default, set as set_markdown_default};

/// The app that opens this file by default, for labels like „In Preview öffnen“.
#[cfg(target_os = "macos")]
pub fn default_app_name(path: &std::path::Path) -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;
    let (workspace, url_class, manager) = (AnyClass::get(c"NSWorkspace")?, AnyClass::get(c"NSURL")?, AnyClass::get(c"NSFileManager")?);
    let path = NSString::from_str(&path.to_string_lossy());
    unsafe {
        let shared: *mut AnyObject = msg_send![workspace, sharedWorkspace];
        let url: *mut AnyObject = msg_send![url_class, fileURLWithPath: &*path];
        let app: *mut AnyObject = msg_send![shared, URLForApplicationToOpenURL: url];
        if app.is_null() {
            return None;
        }
        let app_path: *mut NSString = msg_send![app, path];
        let files: *mut AnyObject = msg_send![manager, defaultManager];
        let name: *mut NSString = msg_send![files, displayNameAtPath: app_path];
        name.as_ref().map(|n| n.to_string().trim_end_matches(".app").to_string())
    }
}

/// The app that opens this file by default; only macOS tells apps its name.
#[cfg(not(target_os = "macos"))]
pub fn default_app_name(_: &std::path::Path) -> Option<String> {
    None
}
