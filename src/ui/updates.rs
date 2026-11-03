use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::markdown_view;
use super::quick::Dismissed;
use super::state::act;
use super::motion::Soft;
use super::theme::{Theme, control, font, iconsize, radius, space};
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
    let view = cx.new(|_| UpdateSheet { highlights: highlights(&release.notes).map(|h| Rc::new(markdown_view::parse(&h))), release, phase: Phase::Offer, task: None });
    let handle = window.window_handle();
    cx.subscribe(&view, move |_, _: &Dismissed, cx| {
        let _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
    })
    .detach();
    use gpui_kit::component::WindowExt;
    window.open_dialog(cx, move |d, _, _| d.entrance(gpui_kit::component::dialog::DialogEntrance::FadeSlide).w(px(480.)).p_0().child(view.clone()));
}

enum Phase {
    Offer,
    Downloading(f32),
    Installing,
    Failed(String),
}

/// The hand-written part on top of the release notes, without the full list of changes below it.
fn highlights(notes: &str) -> Option<String> {
    let (top, _) = notes.split_once("\n## All changes")?;
    let body = top.trim().strip_prefix("## ").map_or(top.trim(), |rest| rest.split_once('\n').map_or("", |(_, b)| b)).trim();
    (!body.is_empty()).then(|| body.to_string())
}

struct UpdateSheet {
    release: Release,
    highlights: Option<Rc<markdown_view::ParsedNote>>,
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
        cx.defer(move |cx| super::documents::resolve_unsaved(all, cx, move |cx| {
            let _ = me.update(cx, |s, cx| {
                s.phase = Phase::Installing;
                cx.notify();
            });
            let task = cx.background_spawn(async move { update::install(&path) });
            cx.spawn(async move |cx| {
                let result = task.await;
                cx.update(|cx| match result {
                    Ok(()) => cx.quit(),
                    Err(e) => {
                        let _ = me.update(cx, |s, cx| {
                            s.phase = Phase::Failed(e);
                            cx.notify();
                        });
                    }
                });
            })
            .detach();
        }));
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
            Button::new(id).label(tr("Release-Seite öffnen")).on_click(move |_, _, _| crate::core::library::open_path(page.as_str()))
        };
        let footer: AnyElement = match &self.phase {
            Phase::Offer => div()
                .flex()
                .items_center()
                .gap(px(space::S))
                .child(Button::new("update-skip").ghost().label(tr("Diese Version überspringen")).on_click(move |_, _, cx| skip.update(cx, |s, cx| s.skip(cx))))
                .child(div().flex_1())
                .child(Button::new("update-later").label(tr("Später")).on_click(move |_, _, cx| later.update(cx, |_, cx| cx.emit(Dismissed))))
                .when(automatic, |d| d.child(Button::new("update-now").primary().label(tr("Jetzt aktualisieren")).on_click(move |_, _, cx| install.update(cx, |s, cx| s.start(cx)))))
                .when(!automatic, |d| d.child(page_button("update-page")))
                .into_any_element(),
            Phase::Downloading(p) => div().flex().flex_col().gap(px(space::S)).child(progress_bar(*p, t.accent, &t)).child(caption(tr("Wird geladen und geprüft …"), &t)).into_any_element(),
            Phase::Installing => div().flex().items_center().gap(px(space::S)).child(super::motion::spinner(super::theme::iconsize::S, t.secondary)).child(caption(tr("Wird installiert, Schrumbi startet gleich neu …"), &t)).into_any_element(),
            Phase::Failed(_) => div()
                .flex()
                .items_center()
                .gap(px(space::S))
                .child(div().flex_1().child(text(tr("Das Update konnte nicht installiert werden."), font::CALLOUT, t.red)))
                .child(page_button("update-page"))
                .into_any_element(),
        };
        let notes_page = self.release.page.clone();
        let link = div()
            .id("update-notes-link")
            .flex()
            .items_center()
            .gap(px(space::XS))
            .h(px(control::S))
            .px(px(space::S))
            .mx(px(-space::S))
            .cursor_pointer()
            .key_nav()
            .soft(t.ink.opacity(0.05), radius::INNER)
            .text_size(px(font::CALLOUT))
            .text_color(t.accent_text)
            .tooltip(tip(tr("Öffnet sich im Browser")))
            .on_click(move |_, _, _| crate::core::library::open_path(notes_page.as_str()))
            .child(tr("Alle Änderungen auf der Release-Seite"))
            .child(icon("arrow-up-right", iconsize::XS, t.accent_text));
        let notes = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(space::S))
            .children(self.highlights.as_ref().map(|h| div().id("update-notes").w_full().max_h(px(240.)).overflow_y_scrollbar().child(markdown_view::render(h, None, None, &t))))
            .when(automatic, |d| d.child(link));
        div()
            .flex()
            .flex_col()
            .gap(px(space::M))
            .p(px(space::XL))
            .text_size(px(font::BODY))
            .text_color(t.primary)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::XXS))
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(font::TITLE3)).child(trf("Version %@ ist verfügbar", &[&self.release.version])))
                    .child(caption(trf("Installiert: %@", &[&env!("CARGO_PKG_VERSION")]), &t)),
            )
            .child(notes)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_written_part_is_shown() {
        assert_eq!(super::highlights("## Schrumbi 0.3.0\n\nNeu: Office.\n\n## All changes\n\n### Reader\n\n- x\n").as_deref(), Some("Neu: Office."));
        assert_eq!(super::highlights("## Schrumbi 0.3.0\n\n### Reader\n\n- x\n"), None);
    }
}
