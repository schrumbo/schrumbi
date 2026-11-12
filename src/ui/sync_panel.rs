use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::{self, Soft, loading};
use super::quick::Dismissed;
use super::state::{self, act};
use super::sync::{self, explain, step_label};
use super::theme::{Theme, control, font, hover, iconsize, radius, space};
use super::widgets::*;
use super::wizards::wizard_frame;
use crate::core::git::cmd::{Git, GitError};
use crate::core::git::conflict::{Choice, ConflictFile, ConflictKind};
use crate::core::git::setup::{self, Inspection};
use crate::core::git::status::{ChangeKind, ChangedFile};
use crate::core::i18n::{tr, trf};
use crate::core::prefs::GitMode;
use crate::core::shortcuts::ShortcutAction;

/// What the sidebar row shows.
pub struct Row {
    pub icon: &'static str,
    pub tint: Option<Hsla>,
    pub label: String,
    pub help: String,
    pub count: Option<(usize, Hsla)>,
    pub working: bool,
    pub kind: RowKind,
    pub finished: u64,
}

#[derive(Clone, Copy, PartialEq)]
pub enum RowKind {
    Offer,
    Panel,
    Conflicts,
}

/// The sidebar row for the current state; `None` when there is nothing to say.
pub fn row(t: &Theme, cx: &App) -> Option<Row> {
    let s = sync::get(cx);
    if s.offer {
        return Some(Row { icon: "git-branch", tint: None, label: tr("Sync einrichten"), help: tr("Diesen Ordner über Git sichern und abgleichen"), count: None, working: false, kind: RowKind::Offer, finished: 0 });
    }
    s.git.as_ref().or(s.root.as_ref().filter(|_| s.error.is_some()).map(|_| &NO_GIT))?;
    let finished = s.finished;
    let keys = ShortcutAction::Sync.hint();
    if let Some(step) = s.step {
        let label = step_label(step);
        return Some(Row { icon: "cloud", tint: None, help: label.clone(), label, count: None, working: true, kind: RowKind::Panel, finished });
    }
    if !s.conflicts.is_empty() {
        let n = s.conflicts.len();
        let label = if n == 1 { tr("1 Konflikt") } else { trf("%@ Konflikte", &[&n.to_string()]) };
        return Some(Row { icon: "git-merge", tint: Some(t.orange), help: label.clone(), label, count: None, working: false, kind: RowKind::Conflicts, finished });
    }
    if let Some(e) = &s.error {
        let (label, help) = match e {
            GitError::Auth => (tr("Anmeldung nötig"), explain(e).0),
            GitError::NoRemote => (tr("Online-Speicher fehlt"), explain(e).0),
            GitError::Missing | GitError::TooOld(_) => (tr("Git fehlt"), explain(e).0),
            _ => (tr("Sync-Problem"), explain(e).0),
        };
        return Some(Row { icon: "cloud-alert", tint: Some(t.orange), label, help, count: None, working: false, kind: RowKind::Panel, finished });
    }
    if s.offline {
        return Some(Row { icon: "cloud-off", tint: Some(t.tertiary), label: tr("Offline, lokal gesichert"), help: tr("Wird hochgeladen, sobald wieder eine Verbindung besteht"), count: None, working: false, kind: RowKind::Panel, finished });
    }
    let status = s.status.as_ref();
    let behind = status.map_or(0, |st| st.behind);
    if behind > 0 {
        let label = if behind == 1 { tr("1 neuer Stand") } else { trf("%@ neue Stände", &[&behind.to_string()]) };
        return Some(Row { icon: "arrow-down", tint: Some(t.accent), help: format!("{label}{keys}"), label, count: Some((behind, t.accent)), working: false, kind: RowKind::Panel, finished });
    }
    let automatic = sync::settings(cx).is_some_and(|g| g.mode == GitMode::Auto);
    let changed = status.map_or(0, |st| st.changed.len());
    if changed > 0 && !automatic {
        let label = if changed == 1 { tr("1 Änderung") } else { trf("%@ Änderungen", &[&changed.to_string()]) };
        return Some(Row { icon: "cloud", tint: None, help: format!("{label}{keys}"), label, count: Some((changed, t.gray)), working: false, kind: RowKind::Panel, finished });
    }
    let help = format!("{}{keys}", last_sync_text(cx));
    Some(Row { icon: "cloud", tint: None, label: if s.local_only { tr("Lokal gesichert") } else { tr("Gesichert") }, help, count: None, working: false, kind: RowKind::Panel, finished })
}

static NO_GIT: Git = Git { exe: PathBuf::new(), root: PathBuf::new() };

/// "Vor 3 Min. gesichert"; without an online storage or a finished sync, "Letzter lokaler Stand: vor 3 Min.".
pub fn last_sync_text(cx: &App) -> String {
    let s = sync::get(cx);
    let synced = sync::settings(cx).and_then(|g| g.last_sync).filter(|_| !s.local_only);
    let Some(at) = synced else {
        return s.last_commit.map_or_else(|| tr("Noch nicht gesichert"), |at| trf("Letzter lokaler Stand: %@", &[&ago(at)]));
    };
    let minutes = (chrono::Local::now().timestamp() - at).max(0) / 60;
    match minutes {
        0 => tr("Gerade eben gesichert"),
        1..=59 => trf("Vor %@ Min. gesichert", &[&minutes.to_string()]),
        60..=1439 => trf("Vor %@ Std. gesichert", &[&(minutes / 60).to_string()]),
        _ => trf("Vor %@ Tagen gesichert", &[&(minutes / 1440).to_string()]),
    }
}

fn ago(at: i64) -> String {
    let minutes = (chrono::Local::now().timestamp() - at).max(0) / 60;
    match minutes {
        0 => tr("gerade eben"),
        1..=59 => trf("vor %@ Min.", &[&minutes.to_string()]),
        60..=1439 => trf("vor %@ Std.", &[&(minutes / 60).to_string()]),
        _ => trf("vor %@ Tagen", &[&(minutes / 1440).to_string()]),
    }
}

/// The popover anchored on the row, opened by an invisible button over it.
pub fn overlay(help: String, cx: &App) -> impl IntoElement + use<> {
    let _ = cx;
    Popover::new("sync-popover")
        .anchor(Anchor::BottomLeft)
        .p_0()
        .trigger_style(StyleRefinement::default().w_full())
        .trigger(Button::new("sync-trigger").ghost().w_full().h(px(44.)).tooltip(help))
        .content(|_, window, cx| panel_view(window, cx))
}

fn panel_view(window: &mut Window, cx: &mut App) -> AnyElement {
    if let Some(view) = sync::get(cx).panel.clone() {
        return view.into_any_element();
    }
    let view = cx.new(|cx| SyncPanel::new(window, cx));
    let any: AnyView = view.into();
    cx.global_mut::<sync::Sync>().panel = Some(any.clone());
    any.into_any_element()
}

/// Status, changed files and the one action that matters.
pub struct SyncPanel {
    message: Entity<InputState>,
    show_details: bool,
    _subs: Vec<Subscription>,
}

impl SyncPanel {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Automatischer Text")).default_value(sync::get(cx).message.clone()));
        let subs = vec![cx.subscribe(&message, |this, input, e: &InputEvent, cx| match e {
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                cx.global_mut::<sync::Sync>().message = text;
            }
            InputEvent::PressEnter { .. } => this.start(cx),
            _ => {}
        })];
        Self { message, show_details: false, _subs: subs }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let text = sync::get(cx).message.clone();
        sync::run(Some(text), true, cx);
    }
}

fn kind_label(kind: &ChangeKind) -> String {
    tr(match kind {
        ChangeKind::New => "Neu",
        ChangeKind::Modified => "Geändert",
        ChangeKind::Deleted => "Gelöscht",
        ChangeKind::Renamed { .. } => "Umbenannt",
        ChangeKind::Conflicted => "Konflikt",
    })
}

fn file_row(file: &ChangedFile, root: Option<&PathBuf>, code: Option<String>, i: usize, t: &Theme) -> impl IntoElement + use<> {
    let path = root.map(|r| r.join(&file.path)).unwrap_or_else(|| PathBuf::from(&file.path));
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let gone = file.kind == ChangeKind::Deleted;
    let open = path.clone();
    let hover = t.ink.opacity(hover::ROW);
    div()
        .id(("sync-file", i))
        .h(px(control::M))
        .flex_none()
        .px(px(space::M))
        .flex()
        .items_center()
        .gap(px(space::S))
        .when(!gone, |d| d.cursor_pointer().key_nav().on_click(move |_, _, cx| act(cx, |l| l.open_file(&open))))
        .soft_row(hover)
        .child(file_icon(&path, iconsize::S))
        .child(div().flex_1().min_w_0().truncate().text_size(px(font::CALLOUT)).child(name))
        .when_some(code, |d, code| d.child(caption(code, t).flex_none()))
        .child(caption(kind_label(&file.kind), t).flex_none())
        .tip(file.path.clone())
}

impl Render for SyncPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let s = sync::get(cx);
        let status = s.status.clone().unwrap_or_default();
        let step = s.step;
        let automatic = sync::settings(cx).is_some_and(|g| g.mode == GitMode::Auto);
        let error = s.error.clone();
        let offline = s.offline;
        let too_large = s.too_large.clone();
        let conflicts = s.conflicts.len();
        let root = s.git.as_ref().map(|g| g.root.clone());
        let keys = ShortcutAction::Sync.active_shortcut().map(|k| k.display());
        let modules: Vec<(String, String)> = state::read(cx, |l| l.config.as_ref().map(|c| c.modules.iter().map(|m| (m.folder.clone(), m.code.clone())).collect()).unwrap_or_default());
        let code = |path: &str| modules.iter().filter(|(f, _)| path.starts_with(&format!("{f}/"))).max_by_key(|(f, _)| f.len()).map(|(_, c)| c.clone());
        let head = div()
            .px(px(space::M))
            .py(px(space::M))
            .flex()
            .flex_col()
            .gap(px(space::XXS))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .text_size(px(font::CALLOUT))
                    .font_weight(FontWeight::MEDIUM)
                    .child(icon("git-branch", iconsize::S, t.secondary))
                    .child(status.branch.clone().unwrap_or_else(|| tr("Kein Branch")))
                    .child(div().flex_1())
                    .when(status.ahead > 0, |d| d.child(div().flex().items_center().gap(px(space::XXS)).text_color(t.secondary).font_features(tabular()).child(icon("arrow-up", iconsize::XS, t.secondary)).child(status.ahead.to_string())))
                    .when(status.behind > 0, |d| d.child(div().flex().items_center().gap(px(space::XXS)).text_color(t.secondary).font_features(tabular()).child(icon("arrow-down", iconsize::XS, t.secondary)).child(status.behind.to_string()))),
            )
            .child(caption(super::sync_panel::last_sync_text(cx), &t));
        let count = status.changed.len();
        let list_height = (count.min(8) as f32) * control::M;
        let files = status.changed.clone();
        let list_root = root.clone();
        let codes: Vec<Option<String>> = files.iter().map(|f| code(&f.path)).collect();
        let list = if count == 0 {
            div()
                .px(px(space::M))
                .py(px(space::L))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(space::S))
                .text_color(t.secondary)
                .child(icon("check", iconsize::M, t.green))
                .child(tr("Alles gesichert"))
                .into_any_element()
        } else {
            div()
                .h(px(list_height))
                .child(
                    uniform_list("sync-files", count, move |range, _, cx| {
                        let t = Theme::get(cx);
                        range.map(|i| file_row(&files[i], list_root.as_ref(), codes[i].clone(), i, &t).into_any_element()).collect::<Vec<_>>()
                    })
                    .h_full(),
                )
                .into_any_element()
        };
        let problem = error.as_ref().map(|e| {
            let (text, action) = explain(e);
            let raw = if let GitError::Other(m) = e { Some(m.clone()) } else { None };
            let mut buttons = div().flex().gap(px(space::S));
            match action {
                Some("retry") => buttons = buttons.child(Button::new("sync-retry").small().label(tr("Erneut versuchen")).on_click(|_, _, cx| sync::run(None, true, cx))),
                Some("anyway") => buttons = buttons.child(Button::new("sync-anyway").small().label(tr("Trotzdem sichern")).on_click(|_, _, cx| sync::run_anyway(cx))),
                Some("setup") => buttons = buttons.child(Button::new("sync-setup").small().label(tr("Einrichten…")).on_click(|_, _, cx| act(cx, |l| l.show_sync_setup = true))),
                Some("reveal") => buttons = buttons.child(Button::new("sync-reveal").small().label(tr("Ordner öffnen")).on_click(|_, _, cx| {
                    if let Some(r) = state::read(cx, |l| l.root.clone()) {
                        crate::core::library::open_path(&r);
                    }
                })),
                Some("guide") => buttons = buttons.child(Button::new("sync-guide").small().label(tr("Anleitung öffnen")).on_click(|_, _, _| {
                    crate::core::library::open_path("https://git-scm.com/downloads");
                })),
                _ => {}
            }
            let has_raw = raw.is_some();
            let details = self.show_details;
            div()
                .p(px(space::S))
                .flex()
                .flex_col()
                .gap(px(space::S))
                .child(notice("cloud-alert", t.orange, text, None, buttons, &t))
                .when(has_raw, |d| {
                    d.child(link_button("sync-details", if details { tr("Details ausblenden") } else { tr("Details") }, &t).text_size(px(font::CAPTION)).px(px(space::S)).on_click(cx.listener(|p, _, _, cx| {
                        p.show_details = !p.show_details;
                        cx.notify();
                    })))
                    .when(details, |d| d.child(div().id("sync-raw").px(px(space::S)).max_h(px(120.)).overflow_y_scrollbar().text_size(px(font::CAPTION)).font_family(Theme::mono_family()).text_color(t.secondary).child(raw.clone().unwrap_or_default())))
                })
        });
        let working = step.is_some();
        let label = step.map_or_else(|| tr("Jetzt synchronisieren"), step_label);
        let percent = crate::core::git::cmd::UPLOAD_PERCENT.load(std::sync::atomic::Ordering::Relaxed);
        let upload = (step == Some(crate::core::git::sync::Step::Uploading) && percent > 0 && sync::get(cx).started.is_some_and(|at| at.elapsed() > std::time::Duration::from_secs(2))).then(|| div().px(px(space::M)).pb(px(space::S)).child(progress_bar(percent as f32 / 100., t.accent, &t)));
        let primary = conflicts == 0 && (count > 0 || status.ahead > 0 || status.behind > 0);
        let mut button = Button::new("sync-now").w_full().label(label).disabled(working).on_click(cx.listener(|p, _, _, cx| p.start(cx)));
        if primary {
            button = button.primary();
        }
        let action = div()
            .p(px(space::M))
            .flex()
            .flex_col()
            .gap(px(space::S))
            .children(upload)
            .when(!automatic && count > 0, |d| d.child(Input::new(&self.message).small()))
            .when(offline, |d| d.child(caption(tr("Offline: Stände bleiben lokal, bis wieder eine Verbindung besteht."), &t)))
            .when(!too_large.is_empty(), |d| {
                let n = too_large.len();
                d.child(caption(if n == 1 { tr("1 Datei ist zu groß zum Hochladen und bleibt lokal.") } else { trf("%@ Dateien sind zu groß zum Hochladen und bleiben lokal.", &[&n.to_string()]) }, &t))
            })
            .child(div().relative().child(button).when(working, |d| d.child(div().absolute().left(px(space::S)).top(px(space::XS)).child(loading(iconsize::S, t.secondary)))))
            .when_some(keys, |d, k| d.child(div().flex().justify_center().child(caption(k, &t))));
        let footer = div()
            .px(px(space::M))
            .py(px(space::S))
            .flex()
            .flex_col()
            .gap(px(space::S))
            .text_size(px(font::CALLOUT))
            .child(div().flex().items_center().gap(px(space::S)).child(div().flex_1().text_color(t.secondary).child(tr("App-Daten mitnehmen"))).child(app_data_switch(cx)))
            .child(link_button("sync-settings", tr("Einstellungen"), &t).on_click(|_, _, cx| super::settings::open_sync(cx)));
        div()
            .w(px(340.))
            .max_h(px(480.))
            .flex()
            .flex_col()
            .child(head)
            .child(divider(&t))
            .child(list)
            .children(problem)
            .child(divider(&t))
            .child(action)
            .child(divider(&t))
            .child(footer)
    }
}

// MARK: Setup

/// What the user can decide in the setup.
pub struct SyncSetup {
    git: Option<Git>,
    found: Option<Inspection>,
    step: SetupStep,
    use_ignore: bool,
    use_untrack: bool,
    use_nested: bool,
    mode: GitMode,
    url: Entity<InputState>,
    name: Entity<InputState>,
    email: Entity<InputState>,
    repo_name: Entity<InputState>,
    remote_state: RemoteState,
    error: Option<String>,
    working: bool,
    clean_state: Option<String>,
    show_details: bool,
    done: bool,
    focus: FocusHandle,
    needs_focus: bool,
    _subs: Vec<Subscription>,
}

#[derive(Clone, Copy, PartialEq)]
enum SetupStep {
    Check,
    CleanUp,
    Connect,
    Mode,
    Finish,
}

#[derive(Clone, PartialEq)]
enum RemoteState {
    Unchecked,
    Checking,
    Good,
    Bad(String),
}

impl EventEmitter<Dismissed> for SyncSetup {}

impl SyncSetup {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("git@github.com:name/studium.git"));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Dein Name")));
        let email = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Deine E-Mail")));
        let folder = state::read(cx, |l| l.root.as_ref().and_then(|r| r.file_name()).map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
        let repo_name = cx.new(|cx| InputState::new(window, cx).default_value(folder));
        let subs = vec![cx.subscribe(&url, |this, _, e: &InputEvent, cx| match e {
            InputEvent::Change => {
                this.remote_state = RemoteState::Unchecked;
                cx.notify();
            }
            InputEvent::Blur | InputEvent::PressEnter { .. } => this.check_remote(cx),
            _ => {}
        })];
        let mut this = Self {
            git: None,
            found: None,
            step: SetupStep::Check,
            use_ignore: true,
            use_untrack: true,
            use_nested: true,
            mode: GitMode::Auto,
            url,
            name,
            email,
            repo_name,
            remote_state: RemoteState::Unchecked,
            error: None,
            working: true,
            clean_state: None,
            show_details: false,
            done: false,
            focus: cx.focus_handle(),
            needs_focus: true,
            _subs: subs,
        };
        this.inspect(cx);
        this
    }

    fn inspect(&mut self, cx: &mut Context<Self>) {
        let Some(root) = state::read(cx, |l| l.root.clone()) else { return };
        let root = dunce::canonicalize(&root).unwrap_or(root);
        let preferred = crate::core::prefs::get().git_path;
        let task = cx.background_spawn(async move {
            let exe = crate::core::git::locate::find(preferred.as_deref())?;
            let git = Git::new(exe, &root);
            let found = setup::inspect(&git);
            Ok::<_, GitError>((git, found))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.working = false;
                match result {
                    Ok((git, found)) => {
                        if found.obsidian_git {
                            this.mode = GitMode::Manual;
                        }
                        this.git = Some(git);
                        this.found = Some(found);
                        this.step = this.first_step();
                    }
                    Err(e) => this.error = Some(explain(&e).0),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn blocked(&self) -> bool {
        self.found.as_ref().is_none_or(|f| f.outer_root.is_some())
    }

    fn first_step(&self) -> SetupStep {
        let Some(f) = &self.found else { return SetupStep::Check };
        if f.cloud.is_some() || f.outer_root.is_some() || !f.has_repo { SetupStep::Check } else { SetupStep::CleanUp }
    }

    /// Only steps with something to decide appear.
    fn steps(&self) -> Vec<SetupStep> {
        let Some(f) = &self.found else { return vec![SetupStep::Check] };
        let mut list = Vec::new();
        if f.cloud.is_some() || f.outer_root.is_some() || !f.has_repo {
            list.push(SetupStep::Check);
        }
        list.push(SetupStep::CleanUp);
        if f.remote.is_none() || !f.has_identity {
            list.push(SetupStep::Connect);
        }
        list.push(SetupStep::Mode);
        list
    }

    fn next_after(&self, step: SetupStep) -> SetupStep {
        let steps = self.steps();
        steps.iter().position(|s| *s == step).and_then(|i| steps.get(i + 1)).copied().unwrap_or(SetupStep::Finish)
    }

    fn previous(&self) -> Option<SetupStep> {
        let steps = self.steps();
        steps.iter().position(|s| *s == self.step).filter(|i| *i > 0).map(|i| steps[i - 1])
    }

    fn check_remote(&mut self, cx: &mut Context<Self>) {
        let url = self.url.read(cx).value().trim().to_string();
        let Some(git) = self.git.clone() else { return };
        if url.is_empty() || self.remote_state != RemoteState::Unchecked {
            return;
        }
        self.remote_state = RemoteState::Checking;
        cx.notify();
        let task = cx.background_spawn(async move { git.test_remote(&url) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.remote_state = match result {
                    Ok(()) => RemoteState::Good,
                    Err(e) => RemoteState::Bad(explain(&e).0),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn create_private(&mut self, cx: &mut Context<Self>) {
        let Some(git) = self.git.clone() else { return };
        let name = self.repo_name.read(cx).value().trim().to_string();
        self.working = true;
        self.error = None;
        cx.notify();
        let task = cx.background_spawn(async move {
            if !git.git_dir().exists() {
                git.init_repo()?;
            }
            git.create_private_remote(&name)?;
            git.run(&["remote", "get-url", "origin"]).map(|u| u.trim().to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.working = false;
                match result {
                    Ok(url) => {
                        if let Some(f) = this.found.as_mut() {
                            f.remote = Some(url);
                            f.has_repo = true;
                        }
                        this.remote_state = RemoteState::Good;
                    }
                    Err(e) => this.error = Some(explain(&e).0),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Everything happens here, after the user saw all of it.
    fn apply(&mut self, cx: &mut Context<Self>) {
        let (Some(git), Some(found)) = (self.git.clone(), self.found.clone()) else { return };
        let (use_ignore, use_untrack, use_nested) = (self.use_ignore, self.use_untrack, self.use_nested);
        let url = self.url.read(cx).value().trim().to_string();
        let (name, email) = (self.name.read(cx).value().trim().to_string(), self.email.read(cx).value().trim().to_string());
        let mode = self.mode;
        let message = tr("Sync eingerichtet");
        self.working = true;
        self.error = None;
        cx.notify();
        let task = cx.background_spawn(async move {
            if !found.has_repo {
                git.init_repo()?;
            }
            if !url.is_empty() && found.remote.is_none() {
                git.set_remote(&url)?;
            }
            if !found.has_identity && !name.is_empty() && !email.is_empty() {
                git.set_identity(&name, &email)?;
            }
            let lines = if use_ignore { found.missing_ignores.clone() } else { Vec::new() };
            let nested = if use_nested { found.nested.clone() } else { Vec::new() };
            git.clean_up(&lines, &nested, use_untrack && found.tracked_backups > 0, &message)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.working = false;
                match result {
                    Ok(state) => {
                        this.clean_state = state;
                        this.done = true;
                        this.step = SetupStep::Finish;
                        sync::enable(mode, cx);
                    }
                    Err(e) => this.error = Some(explain(&e).0),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn undo_clean_up(&mut self, cx: &mut Context<Self>) {
        let (Some(git), Some(state)) = (self.git.clone(), self.clean_state.take()) else { return };
        let task = cx.background_spawn(async move { git.undo_clean_up(&state) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.error = Some(explain(&e).0);
                }
                sync::refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn check_step(&self, t: &Theme) -> Div {
        let Some(f) = &self.found else { return div() };
        div()
            .flex()
            .flex_col()
            .gap(px(space::M))
            .child(div().text_color(t.secondary).child(tr("Sync sichert diesen Ordner und gleicht ihn über einen Online-Speicher mit deinen anderen Geräten ab, deine Dateien bleiben dabei, wo sie sind.")))
            .when_some(f.outer_root.clone(), |d, top| d.child(notice("triangle-alert", t.orange, tr("Dieser Ordner liegt in einem größeren Repository."), Some(trf("Sync geht nur, wenn das Repository genau hier beginnt. Es liegt bei %@.", &[&top.to_string_lossy()])), div(), t)))
            .when_some(f.cloud.clone(), |d, cloud| d.child(notice("triangle-alert", t.orange, trf("Dieser Ordner liegt in %@.", &[&cloud]), Some(tr("Git und Cloud-Ordner stören sich und können den Verlauf beschädigen. Besser liegt der Ordner außerhalb.")), div(), t)))
            .when(!f.has_repo && f.outer_root.is_none(), |d| d.child(caption(tr("Hier beginnt ein neuer Verlauf. Deine Dateien bleiben, wie sie sind."), t)))
    }

    fn option_row(id: &'static str, on: bool, title: String, detail: String, t: &Theme, click: impl Fn(&mut Self, &mut Context<Self>) + 'static, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(id)
            .px(px(space::M))
            .py(px(space::S))
            .flex()
            .items_center()
            .gap(px(space::M))
            .cursor_pointer()
            .key_nav()
            .soft_row(t.ink.opacity(hover::ROW))
            .child(div().size(px(iconsize::M)).flex_none().rounded(px(radius::SMALL)).border_1().border_color(if on { t.accent } else { t.separator }).when(on, |d| d.bg(t.accent)).flex().items_center().justify_center().when(on, |d| d.child(icon("check", iconsize::XS, t.on_accent))))
            .child(div().flex_1().min_w_0().child(title))
            .child(caption(detail, t).flex_none())
            .on_click(cx.listener(move |s, _, _, cx| {
                click(s, cx);
                cx.notify();
            }))
    }

    fn clean_step(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let Some(f) = &self.found else { return div() };
        let mut list = grouped(t);
        let mut any = false;
        if !f.missing_ignores.is_empty() {
            any = true;
            list = list.child(Self::option_row("su-ignore", self.use_ignore, tr("Zwischendateien ausnehmen"), tr("bleiben auf diesem Gerät"), t, |s, _| s.use_ignore = !s.use_ignore, cx));
        }
        if f.tracked_backups > 0 {
            any = true;
            let title = if f.tracked_backups == 1 { tr("1 Sicherungsdatei nicht mitnehmen") } else { trf("%@ Sicherungsdateien nicht mitnehmen", &[&f.tracked_backups.to_string()]) };
            list = list.child(Self::option_row("su-untrack", self.use_untrack, title, tr("bleiben auf der Platte"), t, |s, _| s.use_untrack = !s.use_untrack, cx));
        }
        for folder in &f.nested {
            any = true;
            list = list.child(Self::option_row("su-nested", self.use_nested, trf("%@ ausschließen", &[folder]), tr("verwaltet sich selbst"), t, |s, _| s.use_nested = !s.use_nested, cx));
        }
        let details = self.show_details;
        let lines = f.missing_ignores.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(space::M))
            .child(div().text_color(t.secondary).child(if any { tr("Das macht Schrumbi. Dateien auf der Platte bleiben unberührt.") } else { tr("Hier gibt es nichts aufzuräumen.") }))
            .when(any, |d| d.child(list))
            .when(!lines.is_empty(), |d| {
                d.child(link_button("su-details", if details { tr("Details ausblenden") } else { tr("Details") }, t).text_size(px(font::CAPTION)).on_click(cx.listener(|s, _, _, cx| {
                    s.show_details = !s.show_details;
                    cx.notify();
                })))
                .when(details, |d| d.child(div().px(px(space::S)).text_size(px(font::CAPTION)).font_family(Theme::mono_family()).text_color(t.secondary).children(lines.iter().map(|l| div().child(l.clone())))))
            })
            .child(caption(tr("Deine Aufgaben, Karten und Einstellungen werden mitgenommen."), t))
    }

    fn connect_step(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let Some(f) = &self.found else { return div() };
        let needs_remote = f.remote.is_none();
        let state = self.remote_state.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(space::L))
            .when(needs_remote, |d| {
                d.when(f.gh, |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::S))
                            .child(div().font_weight(FontWeight::MEDIUM).child(tr("Privat auf GitHub anlegen")))
                            .child(div().flex().gap(px(space::S)).child(div().flex_1().child(Input::new(&self.repo_name).small())).child(Button::new("su-create").small().primary().label(tr("Anlegen")).disabled(self.working).on_click(cx.listener(|s, _, _, cx| s.create_private(cx))))),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(space::S))
                        .child(div().font_weight(FontWeight::MEDIUM).child(tr("Adresse des Online-Speichers")))
                        .child(Input::new(&self.url).small())
                        .child(match state {
                            RemoteState::Unchecked => caption(tr("Wird beim Verlassen des Felds geprüft. Ohne Adresse bleibt alles lokal."), t).into_any_element(),
                            RemoteState::Checking => div().flex().items_center().gap(px(space::XS)).child(loading(iconsize::XS, t.secondary)).child(caption(tr("Wird geprüft …"), t)).into_any_element(),
                            RemoteState::Good => div().flex().items_center().gap(px(space::XS)).child(icon("check", iconsize::XS, t.green)).child(caption(tr("Verbindung steht"), t)).into_any_element(),
                            RemoteState::Bad(m) => div().flex().items_center().gap(px(space::XS)).child(icon("circle-alert", iconsize::XS, t.red)).child(div().text_size(px(font::CAPTION)).text_color(t.red_text).child(m)).into_any_element(),
                        }),
                )
            })
            .when(!f.has_identity, |d| {
                d.child(div().flex().flex_col().gap(px(space::S)).child(div().font_weight(FontWeight::MEDIUM).child(tr("Name und E-Mail für deine Stände"))).child(Input::new(&self.name).small()).child(Input::new(&self.email).small()).child(caption(tr("Gilt nur für diesen Ordner."), t)))
            })
    }

    fn mode_step(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let Some(f) = &self.found else { return div() };
        let plugin = f.obsidian_git;
        div()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .when(f.public == Some(true), |d| d.child(notice("shield-alert", t.red, tr("Dieses Repository ist öffentlich."), Some(tr("schrumbi.json kann deine Matrikelnummer enthalten, und alle Dateien wären für alle lesbar.")), div(), t)))
            .child(choice_card("su-auto", self.mode == GitMode::Auto, plugin, "cloud", tr("Automatisch"), tr("Sichert nach 5 Minuten Ruhe, beim Öffnen und Beenden. Du merkst nichts davon."), t).on_click(cx.listener(move |s, _, _, cx| {
                if !plugin {
                    s.mode = GitMode::Auto;
                    cx.notify();
                }
            })))
            .child(choice_card("su-manual", self.mode == GitMode::Manual, false, "refresh-cw", tr("Manuell"), trf("Du entscheidest selbst, wann gesichert wird%@.", &[&ShortcutAction::Sync.hint()]), t).on_click(cx.listener(|s, _, _, cx| {
                s.mode = GitMode::Manual;
                cx.notify();
            })))
            .when(plugin, |d| d.child(caption(tr("Das Obsidian-Plugin Git ist aktiv. Damit nicht zwei Programme synchronisieren, bleibt Automatisch aus."), t)))
    }

    fn finish_step(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let s = sync::get(cx);
        let working = s.step;
        let error = s.error.clone();
        let can_undo = self.clean_state.is_some();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(space::M))
            .py(px(space::L))
            .child(match working {
                Some(step) => div().flex().items_center().gap(px(space::S)).child(loading(iconsize::M, t.secondary)).child(step_label(step)).into_any_element(),
                None => match &error {
                    Some(e) => notice("cloud-alert", t.orange, explain(e).0, Some(tr("Die Einrichtung bleibt gültig.")), div(), t).into_any_element(),
                    None => div().relative().flex().items_center().gap(px(space::S)).child(div().relative().child(icon("check", iconsize::L, t.green)).child(motion::done_pulse("su-done", true, iconsize::L))).child(tr("Alles gesichert")).into_any_element(),
                },
            })
            .when(can_undo, |d| d.child(link_button("su-undo", tr("Aufräumen rückgängig machen"), t).text_size(px(font::CALLOUT)).on_click(cx.listener(|s, _, _, cx| s.undo_clean_up(cx)))))
    }
}

/// A selectable card, one of a set.
pub fn choice_card(id: &'static str, selected: bool, disabled: bool, icon_name: &str, title: String, detail: String, t: &Theme) -> Stateful<Div> {
    let tint = if selected { t.accent } else { t.separator };
    let spoken = format!("{title}{}", if selected { format!(", {}", tr("ausgewählt")) } else { String::new() });
    let card = div()
        .id(id)
        .aria_label(spoken)
        .p(px(space::M))
        .flex()
        .items_start()
        .gap(px(space::M))
        .rounded(px(radius::CONTROL))
        .border_1()
        .border_color(tint)
        .when(selected, |d| d.bg(t.accent.opacity(0.08)))
        .when(disabled, |d| d.opacity(0.5));
    let card = if disabled { card } else { card.cursor_pointer().key_nav().soft(t.ink.opacity(hover::ROW), radius::CONTROL) };
    card.child(icon(icon_name, iconsize::M, if selected { t.accent } else { t.secondary })).child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().font_weight(FontWeight::MEDIUM).child(title)).child(caption(detail, t)))
}

/// The height the sync wizard asks for; all steps share it so the buttons stay where they are.
const SYNC_SETUP_HEIGHT: f32 = 360.;

impl Render for SyncSetup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        if std::mem::take(&mut self.needs_focus) {
            window.focus(&self.focus, cx);
        }
        let me = cx.entity();
        let title = match self.step {
            SetupStep::Check => tr("Sync einrichten"),
            SetupStep::CleanUp => tr("Aufräumen"),
            SetupStep::Connect => tr("Verbinden"),
            SetupStep::Mode => tr("Sicherung"),
            SetupStep::Finish => tr("Fertig"),
        };
        let steps = self.steps();
        let position = steps.iter().position(|s| *s == self.step);
        let body: AnyElement = if self.found.is_none() {
            match self.error.clone() {
                Some(e) => notice("cloud-alert", t.orange, e, None, Button::new("su-guide").small().label(tr("Anleitung öffnen")).on_click(|_, _, _| {
                    crate::core::library::open_path("https://git-scm.com/downloads");
                }), &t).into_any_element(),
                None => div().flex().items_center().gap(px(space::S)).text_color(t.secondary).child(loading(iconsize::S, t.secondary)).child(tr("Ordner wird geprüft …")).into_any_element(),
            }
        } else {
            match self.step {
                SetupStep::Check => self.check_step(&t).into_any_element(),
                SetupStep::CleanUp => self.clean_step(&t, cx).into_any_element(),
                SetupStep::Connect => self.connect_step(&t, cx).into_any_element(),
                SetupStep::Mode => self.mode_step(&t, cx).into_any_element(),
                SetupStep::Finish => self.finish_step(&t, cx).into_any_element(),
            }
        };
        let finishing = self.step == SetupStep::Finish;
        let last = !finishing && self.next_after(self.step) == SetupStep::Finish;
        let blocked = self.blocked() || self.working;
        let previous = self.previous().filter(|_| !finishing);
        let without_remote = self.step == SetupStep::Connect && self.found.as_ref().is_some_and(|f| f.remote.is_none()) && self.url.read(cx).value().trim().is_empty();
        let next_label = if last {
            tr("Einrichten")
        } else if without_remote {
            tr("Ohne Online-Speicher weiter")
        } else {
            tr("Weiter")
        };
        let (close, leave) = (me.clone(), me.clone());
        let foot = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(space::S))
            .when(!finishing, |d| {
                let later = me.clone();
                d.child(Button::new("su-later").ghost().label(tr("Später einrichten")).on_click(move |_, _, cx| later.update(cx, |_, cx| cx.emit(Dismissed))))
            })
            .child(div().flex_1().min_w_0())
            .when_some(previous, |d, prev| d.child(Button::new("su-back").label(tr("Zurück")).on_click(cx.listener(move |s, _, _, cx| {
                s.step = prev;
                s.error = None;
                cx.notify();
            }))))
            .when(!finishing, |d| {
                d.child(Button::new("su-next").primary().label(next_label).disabled(blocked).on_click(cx.listener(move |s, _, _, cx| {
                    if s.next_after(s.step) == SetupStep::Finish {
                        s.apply(cx);
                    } else {
                        s.step = s.next_after(s.step);
                        cx.notify();
                    }
                })))
            })
            .when(finishing, |d| d.child(Button::new("su-close").primary().label(tr("Schließen")).on_click(move |_, _, cx| close.update(cx, |_, cx| cx.emit(Dismissed)))));
        let error = self.error.clone().filter(|_| self.found.is_some());
        let content = div()
            .flex()
            .flex_col()
            .gap(px(space::L))
            .child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::SEMIBOLD).child(title))
            .child(body)
            .when_some(error, |d, e| d.child(notice("cloud-alert", t.orange, e, None, div(), &t)));
        let at = if finishing { steps.len() } else { position.unwrap_or(0) };
        let leave_tip = if finishing { tr("Schließen") } else { tr("Später einrichten") };
        wizard_frame(&t, window, Some((at, steps.len() + 1)), leave_tip, move |_, _, cx| leave.update(cx, |_, cx| cx.emit(Dismissed)), content.into_any_element(), true, foot, SYNC_SETUP_HEIGHT)
            .id("sync-setup")
            .track_focus(&self.focus)
            .on_action(cx.listener(|s, _: &gpui_kit::base::actions::Confirm, window, cx| {
                if !s.focus.is_focused(window) && !super::sheets::typing(window) {
                    cx.propagate();
                } else if s.step == SetupStep::Finish {
                    cx.emit(Dismissed);
                } else if !(s.blocked() || s.working) {
                    if s.next_after(s.step) == SetupStep::Finish {
                        s.apply(cx);
                    } else {
                        s.step = s.next_after(s.step);
                        cx.notify();
                    }
                }
            }))
    }
}

/// The switch for taking the app's own files (tasks, cards, settings) along; shown on the sync page of the settings.
pub fn app_data_switch(cx: &App) -> Switch {
    let s = sync::get(cx);
    Switch::new("s-app-data").checked(!s.app_data_left_out).disabled(s.git.is_none() || s.step.is_some()).on_click(|v, _, cx| set_app_data(*v, cx))
}

/// Takes the app's own files in or out of sync; the files on disk are never touched.
pub fn set_app_data(on: bool, cx: &mut App) {
    if sync::get(cx).step.is_some() {
        return;
    }
    let Some(git) = sync::get(cx).git.clone() else { return };
    cx.global_mut::<sync::Sync>().app_data_left_out = !on;
    let message = if on { tr("App-Daten wieder dabei") } else { tr("App-Daten bleiben hier") };
    let task = cx.background_spawn(async move { git.set_app_data_synced(on, &message) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            match result {
                Ok(_) => {
                    let text = if on { tr("App-Daten werden mitgenommen") } else { tr("App-Daten bleiben auf diesem Gerät") };
                    act(cx, |l| l.notify(text, "check", None, None));
                }
                Err(GitError::Other(m)) if m == ".gitignore" => act(cx, |l| l.last_error = Some(tr("Die Datei .gitignore ist kein lesbarer Text und bleibt, wie sie ist."))),
                Err(e) => act(cx, |l| l.last_error = Some(explain(&e).0)),
            }
            sync::refresh(cx);
        });
    })
    .detach();
}

// MARK: Conflicts

/// Both versions of every file both devices changed; nothing happens until the user applies.
pub struct ConflictSheet {
    files: Vec<ConflictFile>,
    choices: HashMap<String, Choice>,
    index: usize,
    compare: Option<(usize, Entity<gpui_kit::component::diff::DiffState>)>,
    previews: HashMap<(usize, bool), PathBuf>,
    times: HashMap<usize, (Option<i64>, Option<i64>)>,
    working: bool,
    error: Option<String>,
    focus: FocusHandle,
    needs_focus: bool,
    open_after: Vec<(PathBuf, PathBuf)>,
}

impl EventEmitter<Dismissed> for ConflictSheet {}

impl ConflictSheet {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let files = sync::get(cx).conflicts.clone();
        let choices = files.iter().map(|f| (f.path.clone(), Choice::Both)).collect();
        let mut this = Self { files, choices, index: 0, compare: None, previews: HashMap::new(), times: HashMap::new(), working: false, error: None, focus: cx.focus_handle(), needs_focus: true, open_after: Vec::new() };
        this.load(cx);
        this
    }

    /// Fetches what the selected file needs in the background.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.files.get(self.index).cloned() else { return };
        let Some(git) = sync::get(cx).git.clone() else { return };
        let index = self.index;
        if !self.times.contains_key(&index) {
            let (worker, path) = (git.clone(), file.path.clone());
            let task = cx.background_spawn(async move { (worker.side_time("HEAD", &path), worker.side_time("@{u}", &path)) });
            cx.spawn(async move |this, cx| {
                let times = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.times.insert(index, times);
                    cx.notify();
                });
            })
            .detach();
        }
        if file.kind == ConflictKind::Text {
            if self.compare.as_ref().is_some_and(|(i, _)| *i == index) {
                return;
            }
            let name = file.path.clone();
            let task = cx.background_spawn(async move {
                let closed = |t: String| if t.ends_with('\n') { t } else { t + "\n" };
                let (ours, theirs) = (closed(git.text_of(file.ours.as_deref())), closed(git.text_of(file.theirs.as_deref())));
                let diff = similar::TextDiff::configure().timeout(std::time::Duration::from_secs(2)).diff_lines(&theirs, &ours);
                let patch = diff.unified_diff().header(&name, &name).to_string();
                gpui_kit::component::diff::DiffFile::parse(&patch).unwrap_or_default()
            });
            cx.spawn(async move |this, cx| {
                let files = task.await;
                let _ = this.update(cx, |this, cx| {
                    let state = cx.new(|cx| gpui_kit::component::diff::DiffState::new(files, cx).with_mode(gpui_kit::component::diff::DiffMode::Split));
                    this.compare = Some((index, state));
                    cx.notify();
                });
            })
            .detach();
        } else if file.kind == ConflictKind::Binary && !self.previews.contains_key(&(index, true)) {
            let dir = crate::core::prefs::data_dir().unwrap_or_else(std::env::temp_dir).join("conflict-previews");
            let ext = crate::core::naming::extension(&file.path).to_string();
            let task = cx.background_spawn(async move {
                let _ = std::fs::create_dir_all(&dir);
                let (mine, theirs) = (dir.join(format!("{index}-mine.{ext}")), dir.join(format!("{index}-theirs.{ext}")));
                if let Some(o) = &file.ours {
                    let _ = git.write_blob(o, &mine);
                }
                if let Some(o) = &file.theirs {
                    let _ = git.write_blob(o, &theirs);
                }
                (mine, theirs)
            });
            cx.spawn(async move |this, cx| {
                let (mine, theirs) = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.previews.insert((index, true), mine);
                    this.previews.insert((index, false), theirs);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.files.len() {
            self.index = index;
            self.load(cx);
            cx.notify();
        }
    }

    fn choose(&mut self, choice: Choice, cx: &mut Context<Self>) {
        if let Some(f) = self.files.get(self.index) {
            self.choices.insert(f.path.clone(), choice);
            cx.notify();
        }
    }

    /// Keeps both, then shows them side by side for working through by hand.
    fn by_hand(&mut self, cx: &mut Context<Self>) {
        self.choose(Choice::Both, cx);
        if let (Some(f), Some(git)) = (self.files.get(self.index), sync::get(cx).git.as_ref()) {
            let path = git.root.join(&f.path);
            self.open_after.push((path.clone(), path));
        }
        cx.notify();
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.working {
            return;
        }
        self.working = true;
        self.error = None;
        cx.notify();
        let me = cx.entity();
        let open: Vec<PathBuf> = self.open_after.iter().map(|(p, _)| p.clone()).collect();
        sync::resolve(self.choices.clone(), move |result, cx| {
            me.update(cx, |this, cx| {
                this.working = false;
                match result {
                    Ok(()) => {
                        for p in &open {
                            let p = p.clone();
                            act(cx, |l| l.open_file(&p));
                        }
                        cx.emit(Dismissed);
                    }
                    Err(e) => this.error = Some(explain(&e).0),
                }
                cx.notify();
            });
        }, cx);
    }

    fn side_card(&self, mine: bool, time: Option<i64>, t: &Theme) -> Div {
        let title = if mine { tr("Dieses Gerät") } else { tr("Anderes Gerät") };
        let when = time.and_then(|s| chrono::DateTime::from_timestamp(s, 0)).map(|d| d.with_timezone(&chrono::Local).format("%d.%m.%Y, %H:%M").to_string());
        let path = self.previews.get(&(self.index, mine)).cloned();
        let kind = self.files.get(self.index).map(|f| f.kind);
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .rounded(px(radius::CONTROL))
            .border_1()
            .border_color(t.separator)
            .overflow_hidden()
            .child(div().px(px(space::M)).py(px(space::S)).flex().flex_col().child(div().font_weight(FontWeight::MEDIUM).child(title)).when_some(when, |d, w| d.child(caption(w, t))))
            .child(divider(t))
            .child(
                div()
                    .h(px(180.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(t.secondary_bg)
                    .child(match (kind, path) {
                        (Some(ConflictKind::DeletedThere), _) if !mine => caption(tr("Hier gelöscht"), t).into_any_element(),
                        (Some(ConflictKind::DeletedHere), _) if mine => caption(tr("Hier gelöscht"), t).into_any_element(),
                        (_, Some(p)) if p.exists() && ["png", "jpg", "jpeg", "gif", "webp", "svg"].contains(&crate::core::naming::extension(&p.to_string_lossy()).to_lowercase().as_str()) => img(p).max_h(px(172.)).max_w_full().into_any_element(),
                        (_, Some(p)) => {
                            let open = p.clone();
                            div().flex().flex_col().items_center().gap(px(space::S)).child(file_icon(&p, iconsize::PREVIEW)).child(link_button(if mine { "c-open-mine" } else { "c-open-theirs" }, tr("Vorschau öffnen"), t).text_size(px(font::CALLOUT)).on_click(move |_, _, _| crate::core::library::open_path(&open))).into_any_element()
                        }
                        _ => loading(iconsize::M, t.secondary).into_any_element(),
                    }),
            )
    }
}

impl Render for ConflictSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        if std::mem::take(&mut self.needs_focus) {
            window.focus(&self.focus, cx);
        }
        let count = self.files.len();
        let current = self.files.get(self.index).cloned();
        let me = cx.entity();
        let mut list = div().id("conflict-list").w(px(208.)).flex_none().h_full().overflow_y_scrollbar().border_r_1().border_color(t.separator).py(px(space::XS));
        for (i, f) in self.files.iter().enumerate() {
            let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
            let choice = self.choices.get(&f.path).copied().unwrap_or(Choice::Both);
            let selected = i == self.index;
            let hover = t.ink.opacity(hover::ROW);
            list = list.child(
                div()
                    .id(("conflict-row", i))
                    .px(px(space::M))
                    .py(px(space::S))
                    .flex()
                    .flex_col()
                    .cursor_pointer()
                    .key_nav()
                    .when(selected, |d| d.child(motion::row_fill(t.ink.opacity(0.08))))
                    .soft_row(hover)
                    .child(div().truncate().child(name))
                    .child(caption(choice_label(choice), &t))
                    .tip(f.path.clone())
                    .on_click(cx.listener(move |s, _, _, cx| s.select(i, cx))),
            );
        }
        let times = self.times.get(&self.index).copied().unwrap_or((None, None));
        let choice = current.as_ref().and_then(|f| self.choices.get(&f.path)).copied().unwrap_or(Choice::Both);
        let detail = current.as_ref().map(|f| {
            let compare = self.compare.as_ref().filter(|(i, _)| *i == self.index).map(|(_, s)| s.clone());
            let preview: AnyElement = if f.kind == ConflictKind::Text {
                match compare {
                    Some(state) => div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .rounded(px(radius::CONTROL))
                        .border_1()
                        .border_color(t.separator)
                        .overflow_hidden()
                        .child(div().flex_none().py(px(space::S)).flex().border_b_1().border_color(t.separator).text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).text_color(t.secondary).child(div().flex_1().px(px(space::M)).child(tr("Anderes Gerät"))).child(div().flex_1().px(px(space::M)).child(tr("Dieses Gerät"))))
                        .child(div().flex_1().min_h_0().child(gpui_kit::component::diff::Diff::new(&state).header_visible(false).hunk_separator(gpui_kit::component::diff::DiffHunkSeparator::Simple)))
                        .into_any_element(),
                    None => div().flex_1().flex().items_center().justify_center().child(loading(iconsize::M, t.secondary)).into_any_element(),
                }
            } else {
                div().flex().gap(px(space::M)).child(self.side_card(true, times.0, &t)).child(self.side_card(false, times.1, &t)).into_any_element()
            };
            let hint = match (choice, f.kind) {
                (Choice::Both, ConflictKind::DeletedThere | ConflictKind::DeletedHere) => tr("Die vorhandene Version bleibt."),
                (Choice::Both, _) => tr("Deine Version behält den Namen, die andere wird eine Kopie daneben."),
                (Choice::Mine, _) | (Choice::Theirs, _) => tr("Die andere Version bleibt im Verlauf abrufbar."),
            };
            let segment = |id: &'static str, key: &'static str, label: String, value: Choice, cx: &mut Context<Self>| {
                let selected = choice == value;
                let mut b = Button::new(id).small().label(format!("{key}  {label}")).on_click(cx.listener(move |s, _, _, cx| s.choose(value, cx)));
                if selected {
                    b = b.primary();
                }
                b
            };
            let text = f.kind == ConflictKind::Text;
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .p(px(space::M))
                .flex()
                .flex_col()
                .gap(px(space::M))
                .child(div().font_weight(FontWeight::MEDIUM).truncate().child(f.path.clone()))
                .child(preview)
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(space::S))
                        .child(segment("c-both", "1", tr("Beide behalten"), Choice::Both, cx))
                        .child(segment("c-mine", "2", tr("Meine"), Choice::Mine, cx))
                        .child(segment("c-theirs", "3", tr("Andere"), Choice::Theirs, cx))
                        .when(text, |d| d.child(Button::new("c-hand").small().label(format!("4  {}", tr("Von Hand"))).on_click(cx.listener(|s, _, _, cx| s.by_hand(cx))))),
                )
                .child(caption(hint, &t))
        });
        let later = me.clone();
        let error = self.error.clone();
        let working = self.working;
        div()
            .id("conflicts")
            .track_focus(&self.focus)
            .w(px(760.))
            .h(px(540.))
            .flex()
            .flex_col()
            .on_action(cx.listener(|s, a: &gpui_kit::base::actions::Confirm, _, cx| {
                if a.secondary {
                    s.apply(cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, _, cx| {
                let key = e.keystroke.key.as_str();
                let modifiers = e.keystroke.modifiers;
                match key {
                    "up" if s.index > 0 => s.select(s.index - 1, cx),
                    "down" => s.select(s.index + 1, cx),
                    "1" => s.choose(Choice::Both, cx),
                    "2" => s.choose(Choice::Mine, cx),
                    "3" => s.choose(Choice::Theirs, cx),
                    "4" if s.files.get(s.index).is_some_and(|f| f.kind == ConflictKind::Text) => s.by_hand(cx),
                    "enter" if modifiers.platform || modifiers.control => s.apply(cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex_none()
                    .px(px(space::XL))
                    .py(px(space::L))
                    .flex()
                    .flex_col()
                    .gap(px(space::XXS))
                    .border_b_1()
                    .border_color(t.separator)
                    .child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::SEMIBOLD).child(if count == 1 { tr("1 Datei auf beiden Geräten geändert") } else { trf("%@ Dateien auf beiden Geräten geändert", &[&count.to_string()]) }))
                    .child(div().text_color(t.secondary).child(tr("Nichts geht verloren. Du entscheidest, was bleibt."))),
            )
            .child(div().flex_1().min_h_0().flex().child(list).children(detail))
            .when_some(error, |d, e| d.child(div().p(px(space::S)).child(notice("cloud-alert", t.orange, e, None, div(), &t))))
            .child(
                div()
                    .flex_none()
                    .p(px(space::M))
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .border_t_1()
                    .border_color(t.separator)
                    .child(Button::new("c-all").ghost().small().label(tr("Für alle: Beide behalten")).on_click(cx.listener(|s, _, _, cx| {
                        let all: Vec<String> = s.files.iter().map(|f| f.path.clone()).collect();
                        for p in all {
                            s.choices.insert(p, Choice::Both);
                        }
                        cx.notify();
                    })))
                    .child(div().flex_1())
                    .child(Button::new("c-later").label(tr("Später")).on_click(move |_, _, cx| later.update(cx, |_, cx| cx.emit(Dismissed))))
                    .child(Button::new("c-apply").primary().label(if count > 0 { trf("Übernehmen (%@)", &[&count.to_string()]) } else { tr("Übernehmen") }).disabled(working).on_click(cx.listener(|s, _, _, cx| s.apply(cx)))),
            )
    }
}

fn choice_label(choice: Choice) -> String {
    match choice {
        Choice::Both => tr("Beide behalten"),
        Choice::Mine => tr("Meine Version"),
        Choice::Theirs => tr("Andere Version"),
    }
}
