use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::markdown_view;
use super::quick::Dismissed;
use super::state::act;
use super::theme::{Theme, font};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::prefs;
use crate::core::update::{self, Release};

/// How often the app looks for a new version while running.
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Default)]
struct Updates {
    available: Option<Release>,
}

impl Global for Updates {}

/// The version the sidebar should offer, unless the user skipped it.
pub fn available(cx: &App) -> Option<String> {
    cx.try_global::<Updates>().and_then(|u| u.available.as_ref()).map(|r| r.version.clone())
}

fn apply(result: Result<Option<Release>, String>, manual: bool, cx: &mut App) {
    match result {
        Ok(Some(release)) => {
            let skipped = !manual && prefs::get().update_skip == release.version;
            let version = release.version.clone();
            if !skipped {
                cx.global_mut::<Updates>().available = Some(release);
            }
            if manual {
                act(cx, |l| l.notify(trf("Version %@ ist verfügbar", &[&version]), "download", None, None));
            }
            cx.refresh_windows();
        }
        Ok(None) => {
            cx.global_mut::<Updates>().available = None;
            if manual {
                act(cx, |l| l.notify(tr("Du hast die neueste Version."), "circle-check", None, None));
            }
        }
        Err(_) if manual => act(cx, |l| l.notify(tr("Konnte nicht nach Updates suchen."), "triangle-alert", None, None)),
        Err(_) => {}
    }
}

/// Looks for updates now; a manual check always reports its result.
pub fn check(manual: bool, cx: &mut App) {
    let task = cx.background_spawn(async { update::check() });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| apply(result, manual, cx));
    })
    .detach();
}

/// Starts the daily check, if the user wants it.
pub fn init(cx: &mut App) {
    cx.set_global(Updates::default());
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_secs(5)).await;
        loop {
            if prefs::get().update_check {
                cx.update(|cx| check(false, cx));
            }
            cx.background_executor().timer(INTERVAL).await;
        }
    })
    .detach();
}

/// Opens the dialog for the available update.
pub fn show(window: &mut Window, cx: &mut App) {
    let Some(release) = cx.try_global::<Updates>().and_then(|u| u.available.clone()) else { return };
    let view = cx.new(|_| UpdateSheet { notes: Rc::new(markdown_view::parse(&release.notes)), release, phase: Phase::Offer, task: None });
    let handle = window.window_handle();
    cx.subscribe(&view, move |_, _: &Dismissed, cx| {
        let _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
    })
    .detach();
    use gpui_kit::component::WindowExt;
    window.open_dialog(cx, move |d, _, _| d.w(px(480.)).p_0().child(view.clone()));
}

enum Phase {
    Offer,
    Downloading(f32),
    Failed(String),
}

struct UpdateSheet {
    release: Release,
    notes: Rc<markdown_view::ParsedNote>,
    phase: Phase,
    task: Option<Task<()>>,
}

impl EventEmitter<Dismissed> for UpdateSheet {}

impl UpdateSheet {
    fn start(&mut self, cx: &mut Context<Self>) {
        let Some((url, checksum)) = self.release.installer.clone() else { return };
        self.phase = Phase::Downloading(0.);
        let (tx, rx) = async_channel::unbounded::<f32>();
        let download = cx.background_spawn(async move { update::download(&url, &checksum, |p| drop(tx.try_send(p))) });
        self.task = Some(cx.spawn(async move |this, cx| {
            while let Ok(p) = rx.recv().await {
                let _ = this.update(cx, |s, cx| {
                    s.phase = Phase::Downloading(p);
                    cx.notify();
                });
            }
            let result = download.await;
            let _ = this.update(cx, |s, cx| {
                match result {
                    Ok(path) => s.install(path, cx),
                    Err(e) => s.phase = Phase::Failed(e),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Asks about unsaved notes, replaces the app and quits; the new version starts by itself.
    fn install(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        let all = super::state::read(cx, |l| l.buffers.keys().cloned().collect());
        super::documents::resolve_unsaved(all, cx, move |cx| match update::install(&path) {
            Ok(()) => cx.quit(),
            Err(e) => {
                let _ = me.update(cx, |s, cx| {
                    s.phase = Phase::Failed(e);
                    cx.notify();
                });
            }
        });
    }

    fn skip(&mut self, cx: &mut Context<Self>) {
        let version = self.release.version.clone();
        prefs::update(|p| p.update_skip = version);
        cx.global_mut::<Updates>().available = None;
        cx.emit(Dismissed);
    }
}

impl Render for UpdateSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let me = cx.entity();
        let page = self.release.page.clone();
        let (later, skip, install) = (me.clone(), me.clone(), me.clone());
        let automatic = self.release.installer.is_some();
        let page_button = |id: &'static str| {
            let page = page.clone();
            Button::new(id).label(tr("Release-Seite öffnen")).on_click(move |_, _, _| drop(opener::open(page.as_str())))
        };
        let footer: AnyElement = match &self.phase {
            Phase::Offer => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(Button::new("update-skip").ghost().label(tr("Diese Version überspringen")).on_click(move |_, _, cx| skip.update(cx, |s, cx| s.skip(cx))))
                .child(div().flex_1())
                .child(Button::new("update-later").label(tr("Später")).on_click(move |_, _, cx| later.update(cx, |_, cx| cx.emit(Dismissed))))
                .when(automatic, |d| d.child(Button::new("update-now").primary().label(tr("Jetzt aktualisieren")).on_click(move |_, _, cx| install.update(cx, |s, cx| s.start(cx)))))
                .when(!automatic, |d| d.child(page_button("update-page")))
                .into_any_element(),
            Phase::Downloading(p) => div().flex().flex_col().gap(px(6.)).child(progress_bar(*p, t.accent, &t)).child(caption(tr("Wird geladen und geprüft …"), &t)).into_any_element(),
            Phase::Failed(_) => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().child(text(tr("Das Update konnte nicht installiert werden."), font::CALLOUT, t.red)))
                .child(page_button("update-page"))
                .into_any_element(),
        };
        let notes: AnyElement = if self.release.notes.trim().is_empty() {
            caption(tr("Keine Versionshinweise."), &t).into_any_element()
        } else {
            markdown_view::render(&self.notes, None, None, &t).into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(20.))
            .text_size(px(font::BODY))
            .text_color(t.primary)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(font::TITLE3)).child(trf("Version %@ ist verfügbar", &[&self.release.version])))
                    .child(caption(trf("Installiert: %@", &[&env!("CARGO_PKG_VERSION")]), &t)),
            )
            .child(div().id("update-notes").max_h(px(280.)).overflow_y_scroll().child(notes))
            .child(footer)
    }
}
