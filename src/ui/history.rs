use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::diff::{Diff, DiffFile, DiffHunkSeparator, DiffMode, DiffState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::{Soft, loading};
use super::preview::FilePreview;
use super::state::{self, act};
use super::sync;
use super::theme::{Theme, font, iconsize, space};
use super::widgets::*;
use crate::core::git::history::Version;
use crate::core::i18n::{tr, trf};
use crate::core::markdown;
use crate::core::naming::extension;

/// How many versions are fetched at a time.
const PAGE: usize = 100;

/// An earlier version of a document, shown in place of the document itself.
pub struct Viewed {
    pub path: PathBuf,
    pub version: Version,
    pub temp: PathBuf,
}

/// The version on screen, if any.
#[derive(Default)]
pub struct History {
    pub viewed: Option<Viewed>,
    generation: u64,
}

impl Global for History {}

pub fn init(cx: &mut App) {
    cx.set_global(History::default());
}

pub fn viewed_path(cx: &App) -> Option<PathBuf> {
    cx.global::<History>().viewed.as_ref().map(|v| v.path.clone())
}

fn folder() -> PathBuf {
    crate::core::prefs::data_dir().unwrap_or_else(std::env::temp_dir).join("versions")
}

/// Shows a version in the document's place; its bytes are fetched in the background.
pub fn show(path: PathBuf, version: Version, cx: &mut App) {
    let Some(git) = sync::get(cx).git.clone() else { return };
    cx.global_mut::<History>().generation += 1;
    let generation = cx.global::<History>().generation;
    let job = version.clone();
    let ext = extension(&path.file_name().unwrap_or_default().to_string_lossy()).to_string();
    let task = cx.background_spawn(async move {
        let bytes = git.version_bytes(&job).ok()?;
        let dir = folder();
        std::fs::create_dir_all(&dir).ok()?;
        let temp = dir.join(format!("{}.{ext}", job.rev));
        std::fs::write(&temp, bytes).ok()?;
        Some(temp)
    });
    cx.spawn(async move |cx| {
        let temp = task.await;
        cx.update(|cx| {
            if cx.global::<History>().generation != generation {
                return;
            }
            let Some(temp) = temp else {
                act(cx, |l| l.notify(tr("Diese Version konnte nicht geladen werden."), "triangle-alert", None, None));
                return;
            };
            let old = cx.global_mut::<History>().viewed.replace(Viewed { path, version, temp: temp.clone() });
            if let Some(old) = old.filter(|o| o.temp != temp) {
                let _ = std::fs::remove_file(old.temp);
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Back to the current document; nothing was changed.
pub fn close(cx: &mut App) {
    let h = cx.global_mut::<History>();
    h.generation += 1;
    if let Some(v) = h.viewed.take() {
        let _ = std::fs::remove_file(v.temp);
    }
    cx.refresh_windows();
}

/// Makes the shown version the current one; the current text stays one undo away.
pub fn restore(cx: &mut App) {
    let Some(v) = cx.global::<History>().viewed.as_ref().map(|v| (v.path.clone(), v.temp.clone())) else { return };
    let Ok(bytes) = std::fs::read(&v.1) else { return };
    act(cx, |l| l.restore_version(&v.0, bytes));
    close(cx);
    sync::touched(cx);
}

fn when_label(time: i64) -> (String, String) {
    let Some(moment) = chrono::DateTime::from_timestamp(time, 0).map(|d| d.with_timezone(&chrono::Local)) else { return (String::new(), String::new()) };
    let today = chrono::Local::now().date_naive();
    (crate::core::day::day_header(moment.date_naive(), today), moment.format("%H:%M").to_string())
}

/// The versions of the open document, in the details column.
pub struct VersionList {
    path: Option<PathBuf>,
    versions: Vec<Version>,
    loading: bool,
    more: bool,
    selected: Option<usize>,
    error: Option<String>,
    focus: FocusHandle,
    generation: u64,
}

impl VersionList {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { path: None, versions: Vec::new(), loading: false, more: false, selected: None, error: None, focus: cx.focus_handle(), generation: 0 }
    }

    fn relative(path: &Path, cx: &App) -> Option<String> {
        let root = sync::get(cx).git.as_ref()?.root.clone();
        let full = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        Some(full.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/"))
    }

    fn load(&mut self, skip: usize, cx: &mut Context<Self>) {
        let (Some(path), Some(git)) = (self.path.clone(), sync::get(cx).git.clone()) else { return };
        let Some(rel) = Self::relative(&path, cx) else { return };
        self.loading = true;
        self.generation += 1;
        let generation = self.generation;
        let task = cx.background_spawn(async move { git.versions(&rel, skip, PAGE) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(list) => {
                        this.more = list.len() == PAGE;
                        this.versions.extend(list);
                    }
                    Err(e) => this.error = Some(sync::explain(&e).0),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn sync_path(&mut self, cx: &mut Context<Self>) {
        let current = state::read(cx, |l| l.active_document.clone());
        if current != self.path {
            self.path = current;
            self.versions.clear();
            self.selected = None;
            self.error = None;
            self.more = false;
            if self.path.is_some() {
                self.load(0, cx);
            }
        }
    }

    /// The newest state is what is on disk unless something is unsaved; older ones open in place.
    fn select(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        self.selected = index;
        match index.and_then(|i| self.versions.get(i).cloned().map(|v| (i, v))) {
            Some((_, version)) => {
                if let Some(path) = self.path.clone() {
                    show(path, version, cx);
                }
            }
            None => close(cx),
        }
        cx.notify();
    }

    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        let next = match (self.selected, down) {
            (None, true) => Some(0),
            (None, false) => None,
            (Some(0), false) => None,
            (Some(i), false) => Some(i - 1),
            (Some(i), true) => Some((i + 1).min(self.versions.len().saturating_sub(1))),
        };
        if next == Some(self.versions.len().saturating_sub(1)) && self.more && !self.loading {
            let skip = self.versions.len();
            self.load(skip, cx);
        }
        if !self.versions.is_empty() {
            self.select(next, cx);
        }
    }
}

impl Render for VersionList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_path(cx);
        let t = Theme::get(cx);
        if sync::get(cx).git.is_none() {
            return unavailable("history", tr("Noch kein Verlauf"), tr("Er beginnt, sobald der Sync eingerichtet ist."), &t).into_any_element();
        }
        if self.versions.is_empty() {
            if self.loading {
                return div().p(px(space::L)).flex().items_center().gap(px(space::S)).text_color(t.secondary).child(loading(iconsize::S, t.secondary)).child(tr("Verlauf wird geladen …")).into_any_element();
            }
            if let Some(e) = self.error.clone() {
                return div().p(px(space::S)).child(notice("cloud-alert", t.orange, e, None, div(), &t)).into_any_element();
            }
            return unavailable("history", tr("Noch kein Verlauf"), tr("Er beginnt mit dem ersten Sichern."), &t).into_any_element();
        }
        let today = chrono::Local::now().date_naive();
        let hover = t.ink.opacity(0.05);
        let mut list = div().flex().flex_col().py(px(space::S));
        let current_selected = self.selected.is_none();
        list = list.child(
            div()
                .id("version-now")
                .px(px(space::L))
                .py(px(space::S))
                .flex()
                .items_center()
                .gap(px(space::M))
                .cursor_pointer()
                .key_nav()
                .when(current_selected, |d| d.bg(t.ink.opacity(0.08)))
                .when(!current_selected, |d| d.soft(hover, 0.))
                .child(div().size(px(space::S)).rounded_full().bg(t.accent))
                .child(div().font_weight(FontWeight::MEDIUM).child(tr("Aktuell")))
                .on_click(cx.listener(|s, _, _, cx| s.select(None, cx))),
        );
        let mut last_day = None;
        for (i, v) in self.versions.iter().enumerate() {
            let day = chrono::DateTime::from_timestamp(v.time, 0).map(|d| d.with_timezone(&chrono::Local).date_naive());
            if day != last_day {
                last_day = day;
                if let Some(d) = day {
                    list = list.child(div().px(px(space::L)).pt(px(space::M)).pb(px(space::XS)).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(crate::core::day::day_header(d, today)));
                }
            }
            let selected = self.selected == Some(i);
            let (_, time) = when_label(v.time);
            list = list.child(
                div()
                    .id(("version", i))
                    .px(px(space::L))
                    .py(px(space::S))
                    .flex()
                    .items_start()
                    .gap(px(space::M))
                    .cursor_pointer()
                    .key_nav()
                    .when(selected, |d| d.bg(t.ink.opacity(0.08)))
                    .when(!selected, |d| d.soft(hover, 0.))
                    .child(div().mt(px(space::XS)).size(px(space::S)).rounded_full().border_1().border_color(t.tertiary))
                    .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().child(v.subject.clone())).child(caption(time, &t).font_features(tabular())))
                    .tooltip(tip(v.subject.clone()))
                    .on_click(cx.listener(move |s, _, _, cx| s.select(Some(i), cx))),
            );
        }
        if self.more {
            list = list.child(div().p(px(space::M)).child(Button::new("version-more").small().w_full().label(tr("Ältere laden")).disabled(self.loading).on_click(cx.listener(|s, _, _, cx| {
                let skip = s.versions.len();
                s.load(skip, cx);
            }))));
        }
        div()
            .id("version-list")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, _, cx| match e.keystroke.key.as_str() {
                "up" => s.step(false, cx),
                "down" => s.step(true, cx),
                "escape" => s.select(None, cx),
                "enter" if s.selected.is_some() => restore(cx),
                _ => return,
            }))
            .child(list)
            .into_any_element()
    }
}

/// The document's place while an earlier version is shown: read-only, with a way back.
pub struct VersionPane {
    version: Entity<FilePreview>,
    current: Entity<FilePreview>,
    shown: Option<PathBuf>,
    compare: bool,
    diff: Option<(PathBuf, Entity<DiffState>)>,
}

impl VersionPane {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { version: cx.new(|_| FilePreview::new(false)), current: cx.new(|_| FilePreview::new(false)), shown: None, compare: false, diff: None }
    }

    fn make_diff(&mut self, viewed: PathBuf, path: PathBuf, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move {
            let read = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
            let closed = |t: String| if t.ends_with('\n') { t } else { t + "\n" };
            let (old, new) = (closed(read(&viewed)), closed(read(&path)));
            let diff = similar::TextDiff::configure().timeout(std::time::Duration::from_secs(2)).diff_lines(&old, &new);
            let patch = diff.unified_diff().header("a", "a").to_string();
            (viewed, DiffFile::parse(&patch).unwrap_or_default())
        });
        cx.spawn(async move |this, cx| {
            let (viewed, files) = task.await;
            let _ = this.update(cx, |this, cx| {
                let state = cx.new(|cx| DiffState::new(files, cx).with_mode(DiffMode::Split));
                this.diff = Some((viewed, state));
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for VersionPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let Some((temp, path, time)) = cx.global::<History>().viewed.as_ref().map(|v| (v.temp.clone(), v.path.clone(), v.version.time)) else { return div().into_any_element() };
        if self.shown.as_ref() != Some(&temp) {
            self.shown = Some(temp.clone());
            let (a, b) = (temp.clone(), path.clone());
            self.version.update(cx, |p, cx| p.set_path(Some(a), cx));
            self.current.update(cx, |p, cx| p.set_path(Some(b), cx));
            self.diff = None;
        }
        let text = markdown::is_markdown(&path) || matches!(extension(&path.to_string_lossy()).to_lowercase().as_str(), "txt" | "csv" | "html" | "json");
        if self.compare && text && self.diff.as_ref().is_none_or(|(p, _)| *p != temp) {
            self.make_diff(temp.clone(), path.clone(), cx);
        }
        let (day, clock) = when_label(time);
        let bar = div()
            .flex_none()
            .px(px(space::L))
            .py(px(space::S))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::M))
            .bg(t.accent.opacity(0.08))
            .border_b_1()
            .border_color(t.separator)
            .child(icon("history", iconsize::M, t.accent))
            .child(div().flex_1().min_w(px(160.)).child(trf("Version vom %@, %@ · nur lesen", &[&day, &clock])))
            .child(
                div()
                    .flex()
                    .gap(px(space::S))
                    .child(Button::new("version-compare").small().label(if self.compare { tr("Vergleich ausblenden") } else { tr("Vergleichen") }).on_click(cx.listener(|p, _, _, cx| {
                        p.compare = !p.compare;
                        cx.notify();
                    })))
                    .child(Button::new("version-restore").small().primary().label(tr("Diese Version wiederherstellen")).on_click(|_, _, cx| restore(cx)))
                    .child(Button::new("version-close").small().label(tr("Schließen")).on_click(|_, _, cx| close(cx))),
            );
        let body: AnyElement = if self.compare && text {
            match self.diff.as_ref().filter(|(p, _)| *p == temp) {
                Some((_, state)) => div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().flex_none().py(px(space::S)).flex().border_b_1().border_color(t.separator).text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).text_color(t.secondary).child(div().flex_1().px(px(space::L)).child(tr("Diese Version"))).child(div().flex_1().px(px(space::L)).child(tr("Aktuell"))))
                    .child(div().flex_1().min_h_0().child(Diff::new(state).header_visible(false).hunk_separator(DiffHunkSeparator::Simple)))
                    .into_any_element(),
                None => div().size_full().flex().items_center().justify_center().child(loading(iconsize::M, t.secondary)).into_any_element(),
            }
        } else if self.compare {
            div().size_full().flex().child(div().flex_1().min_w_0().h_full().border_r_1().border_color(t.separator).child(self.version.clone())).child(div().flex_1().min_w_0().h_full().child(self.current.clone())).into_any_element()
        } else {
            self.version.clone().into_any_element()
        };
        div().size_full().flex().flex_col().child(bar).child(div().flex_1().min_h_0().child(body)).into_any_element()
    }
}
