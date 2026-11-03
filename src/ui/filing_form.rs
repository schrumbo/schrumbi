use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Local, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::{self, act};
use super::motion::Soft;
use super::theme::{Theme, font, radius, space, iconsize};
use super::widgets::*;
use crate::core::day;
use crate::core::filing::{self, BRIEF_TAG, FileFacts, FileRole, Suggestion};
use crate::core::i18n::{tr, trf};
use crate::core::library::{Library, MaterialTarget};
use crate::core::model::{Assignment, CoauthorStyle, StudyConfig};
use crate::core::naming::{Naming, extension};
use crate::core::prefs;

/// Where a file goes: an assignment's folder or any folder.
#[derive(Clone, PartialEq)]
pub enum Target {
    Assignment(String),
    Folder(PathBuf),
}

pub enum FilingEvent {
    Finished,
}

impl EventEmitter<FilingEvent> for FilingForm {}

/// Shared filing flow: say what the file is, pick target, title and date; preview the final name; file one or many.
pub struct FilingForm {
    paths: Vec<PathBuf>,
    preset: Option<String>,
    index: usize,
    facts: Option<FileFacts>,
    thumbnail: Option<Arc<Image>>,
    ranked: Vec<Assignment>,
    places: Vec<MaterialTarget>,
    titles: Vec<Suggestion>,
    role: FileRole,
    guessed: Option<FileRole>,
    target: Option<Target>,
    title: Entity<InputState>,
    byline: Entity<InputState>,
    coauthors: Vec<String>,
    draft: Entity<InputState>,
    date: Entity<DatePickerState>,
    date_value: NaiveDate,
    rename: bool,
    renames_by_default: bool,
    convention: bool,
    grab_focus: bool,
    details: bool,
    /// Width of the form, measured while painting.
    width: f32,
    focus: FocusHandle,
    load: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// "Anna Beispiel" → "Beispiel A"; ignores tool names and user handles.
pub fn byline_from(author: Option<&str>) -> String {
    let Some(author) = author else { return String::new() };
    let words: Vec<&str> = author.split(' ').filter(|w| !w.is_empty()).collect();
    let ok = (2..=3).contains(&words.len()) && words.iter().all(|w| w.chars().all(|c| c.is_alphabetic() || c == '-') && w.chars().next().is_some_and(char::is_uppercase));
    if !ok {
        return String::new();
    }
    format!("{} {}", words[words.len() - 1], words[0].chars().next().unwrap_or_default())
}

impl FilingForm {
    pub fn new(paths: Vec<PathBuf>, preset: Option<String>, grab_focus: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder(tr("z. B. Persona-Board")));
        let byline = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Dozent:in, z. B. Beispiel A (optional)")));
        let draft = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Gruppenarbeit? Nachname V")));
        let date = cx.new(|cx| DatePickerState::new(window, cx).date_format(if crate::core::i18n::lang() == crate::core::i18n::Lang::En { "%m/%d/%Y" } else { "%d.%m.%Y" }));
        let subs = vec![
            cx.subscribe_in(&title, window, |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.commit(window, cx),
                _ => {}
            }),
            cx.subscribe(&byline, |_, _, e: &InputEvent, cx| if matches!(e, InputEvent::Change) { cx.notify() }),
            cx.subscribe_in(&draft, window, |this, s, e: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = e {
                    let raw = s.read(cx).value().to_string();
                    this.add_coauthor(&raw, cx);
                    s.update(cx, |s, cx| s.set_value("", window, cx));
                }
            }),
            cx.subscribe(&date, |this, _, e: &DatePickerEvent, cx| {
                let DatePickerEvent::Change(v) = e;
                if let Some(d) = v.start() {
                    this.date_value = d.date();
                    cx.notify();
                }
            }),
        ];
        let renames = renames_by_default(cx);
        let mut form = Self {
            paths,
            preset,
            index: 0,
            facts: None,
            thumbnail: None,
            ranked: Vec::new(),
            places: Vec::new(),
            titles: Vec::new(),
            role: FileRole::Work,
            guessed: None,
            target: None,
            title,
            byline,
            coauthors: Vec::new(),
            draft,
            date,
            date_value: Local::now().date_naive(),
            rename: renames,
            renames_by_default: renames,
            convention: has_convention(cx),
            grab_focus,
            details: false,
            width: 560.,
            focus: cx.focus_handle(),
            load: None,
            _subs: subs,
        };
        form.load_current(window, cx);
        form
    }

    /// The file currently being filed.
    pub fn path(&self) -> Option<&PathBuf> {
        self.paths.get(self.index)
    }

    /// Enter from outside the form: files the suggestion, or moves into the fields when they are open.
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.details {
            self.refocus(window, cx);
        } else {
            self.commit(window, cx);
        }
    }

    fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.details {
            self.title.update(cx, |s, cx| s.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
    }

    fn set_details(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.details = open;
        self.refocus(window, cx);
        cx.notify();
    }

    /// Targets offered next to the current suggestion, best first.
    fn alternatives(&self) -> Vec<Target> {
        let all: Vec<Target> = if self.role == FileRole::Material { self.places.iter().map(|p| Target::Folder(p.path.clone())).collect() } else { self.ranked.iter().map(|a| Target::Assignment(a.id.clone())).collect() };
        all.into_iter().filter(|t| Some(t) != self.target.as_ref()).take(3).collect()
    }

    /// Name, tint and context line of a target.
    fn describe(&self, target: &Target, l: &Library, t: &Theme) -> (String, Hsla, String) {
        match target {
            Target::Assignment(id) => {
                let a = self.ranked.iter().find(|a| &a.id == id).cloned().or_else(|| l.assignment(id).cloned());
                let m = a.as_ref().and_then(|a| l.module(a));
                (a.as_ref().map_or_else(String::new, |a| a.title.clone()), t.palette(m.map_or("blue", |m| m.color.as_str())), a.as_ref().map_or_else(String::new, |a| format!("{} · {}", m.map_or("", |m| m.code.as_str()), day::day_label(a.due, l.now))))
            }
            Target::Folder(f) => {
                let m = l.module_containing(f);
                (name_of(f), t.palette(m.map_or("blue", |m| m.color.as_str())), l.relative_path(f))
            }
        }
    }

    fn style(cx: &App) -> CoauthorStyle {
        state::read(cx, |l| l.config.as_ref().map(|c| c.coauthor_style).unwrap_or_default())
    }

    fn add_coauthor(&mut self, raw: &str, cx: &mut Context<Self>) {
        let n = Naming::clean_author(raw.trim(), Self::style(cx));
        if !n.is_empty() && !self.coauthors.contains(&n) {
            self.coauthors.push(n);
        }
        cx.notify();
    }

    fn load_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.path().cloned() else { return };
        self.facts = None;
        self.thumbnail = None;
        self.details = false;
        let p = path.clone();
        let task = cx.background_spawn(async move {
            let facts = FileFacts::read(&p);
            let thumb = if extension(&name_of(&p)).eq_ignore_ascii_case("pdf") {
                crate::core::pdf_render::PdfDoc::open(&p).and_then(|d| d.render_png(0, 1.0)).map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)))
            } else {
                None
            };
            (facts, thumb)
        });
        self.load = Some(cx.spawn_in(window, async move |this, cx| {
            let (facts, thumb) = task.await;
            let _ = this.update_in(cx, |f, window, cx| f.apply_facts(facts, thumb, window, cx));
        }));
        cx.notify();
    }

    fn apply_facts(&mut self, facts: FileFacts, thumb: Option<Arc<Image>>, window: &mut Window, cx: &mut Context<Self>) {
        let (ranked, places, titles, parsed, author) = state::read(cx, |l| {
            let namings = l.namings();
            (l.rank_targets(&facts), l.material_targets(), filing::title_suggestions(&facts, &namings, &l.title_ignore_words()), namings.iter().find_map(|n| n.parse(facts.name())), l.config.as_ref().map(|c| c.author.clone()).unwrap_or_default())
        });
        self.ranked = ranked;
        self.places = places;
        self.titles = titles;
        self.coauthors = parsed.as_ref().map(|p| p.coauthors.clone()).unwrap_or_default();
        let t = self.titles.first().map(|s| s.title.clone()).unwrap_or_default();
        self.title.update(cx, |s, cx| s.set_value(t, window, cx));
        let b = byline_from(facts.pdf_author.as_deref());
        self.byline.update(cx, |s, cx| s.set_value(b, window, cx));
        let g = FileRole::guess(&facts, &author);
        let guess = if self.preset.is_some() && g == FileRole::Material { FileRole::Work } else { g };
        self.guessed = Some(guess);
        self.role = guess;
        self.thumbnail = thumb;
        let initial = facts.name_date.unwrap_or_else(|| day::local_date(facts.created));
        self.set_date(initial, window, cx);
        if let Some(preset) = self.preset.clone() {
            self.select(Target::Assignment(preset), window, cx);
        } else if self.target.is_none() || self.paths.len() > 1 {
            self.select_default(window, cx);
        }
        if let Some(d) = parsed.and_then(|p| p.date) {
            self.set_date(d, window, cx);
        }
        self.facts = Some(facts);
        self.details = state::read(cx, |l| self.folder(l).is_none()) || (self.rename && self.title.read(cx).value().trim().is_empty());
        if self.grab_focus {
            self.refocus(window, cx);
        }
        cx.notify();
    }

    fn set_date(&mut self, d: NaiveDate, window: &mut Window, cx: &mut Context<Self>) {
        self.date_value = d;
        self.date.update(cx, |s, cx| s.set_date(d, window, cx));
    }

    fn target_assignment(&self, l: &Library) -> Option<Assignment> {
        match &self.target {
            Some(Target::Assignment(id)) => l.assignment(id).cloned(),
            _ => None,
        }
    }

    fn folder(&self, l: &Library) -> Option<PathBuf> {
        match &self.target {
            Some(Target::Assignment(_)) => self.target_assignment(l).and_then(|a| l.filing_folder(&a)),
            Some(Target::Folder(f)) => Some(f.clone()),
            None => None,
        }
    }

    fn module_id(&self, l: &Library) -> Option<String> {
        self.target_assignment(l).map(|a| a.module_id).or_else(|| self.folder(l).and_then(|f| l.module_containing(&f).map(|m| m.id.clone())))
    }

    fn final_name(&self, l: &Library, cx: &App) -> String {
        let Some(path) = self.path() else { return String::new() };
        let name = name_of(path);
        let Some(naming) = self.module_id(l).and_then(|m| l.naming(&m)).filter(|_| self.rename) else { return name };
        let a = self.target_assignment(l);
        let week = a.as_ref().and_then(|a| a.week).or_else(|| l.current_week().map(|w| w.index));
        let iso = a.as_ref().and_then(|a| a.iso_week);
        let title = self.title.read(cx).value().to_string();
        let ext = extension(&name);
        match self.role {
            FileRole::Work => naming.render(&title, self.date_value, ext, &self.coauthors, week, iso, None),
            FileRole::Brief => naming.render(&title, self.date_value, ext, &[], week, iso, Some(BRIEF_TAG)),
            FileRole::Material => naming.render(&title, self.date_value, ext, &[], week, None, Some(&self.byline.read(cx).value())),
        }
    }

    fn set_role(&mut self, r: FileRole, window: &mut Window, cx: &mut Context<Self>) {
        if r == self.role {
            return;
        }
        self.role = r;
        let keep = r != FileRole::Material && matches!(self.target, Some(Target::Assignment(_)));
        if keep {
            self.rename = self.renames_for_role(cx);
        } else {
            self.select_default(window, cx);
        }
        cx.notify();
    }

    /// Renaming is on when the workspace wants it, and, while there is a convention, for a task sheet whose name would otherwise count as the submission.
    fn renames_for_role(&self, cx: &App) -> bool {
        let author = state::read(cx, |l| l.config.as_ref().map(|c| c.author.clone()).unwrap_or_default());
        renames_by_default(cx) || (has_convention(cx) && self.role == FileRole::Brief && self.path().is_some_and(|p| !FileRole::is_brief(&name_of(p), &author)))
    }

    fn follow_rename_setting(&mut self, cx: &App) {
        let now = renames_by_default(cx);
        let convention = has_convention(cx);
        if now == self.renames_by_default && convention == self.convention {
            return;
        }
        self.renames_by_default = now;
        self.convention = convention;
        self.rename = match &self.target {
            Some(Target::Folder(f)) => now && state::read(cx, |l| l.module_containing(f).is_some()),
            _ => self.renames_for_role(cx),
        };
        if self.rename && self.facts.is_some() && self.title.read(cx).value().trim().is_empty() {
            self.details = true;
        }
    }

    fn select_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let t = if self.role == FileRole::Material { self.places.first().map(|p| Target::Folder(p.path.clone())) } else { self.ranked.first().map(|a| Target::Assignment(a.id.clone())) };
        match t {
            Some(t) => self.select(t, window, cx),
            None => self.target = None,
        }
    }

    fn select(&mut self, t: Target, window: &mut Window, cx: &mut Context<Self>) {
        match &t {
            Target::Assignment(id) => {
                self.rename = self.renames_for_role(cx);
                if self.role == FileRole::Work
                    && let Some(a) = state::read(cx, |l| l.assignment(id).cloned())
                {
                    self.set_date(day::local_date(a.due.min(Local::now())), window, cx);
                }
            }
            Target::Folder(f) => {
                let (in_module, near) = state::read(cx, |l| (l.module_containing(f).is_some(), l.bylines_near(f)));
                self.rename = in_module && renames_by_default(cx);
                if self.role == FileRole::Material {
                    let from_pdf = byline_from(self.facts.as_ref().and_then(|f| f.pdf_author.as_deref()));
                    let b = if from_pdf.is_empty() { near.first().cloned().unwrap_or_default() } else { from_pdf };
                    self.byline.update(cx, |s, cx| s.set_value(b, window, cx));
                }
            }
        }
        self.target = Some(t);
        cx.notify();
    }

    fn can_file(&self, l: &Library, cx: &App) -> bool {
        self.facts.is_some() && self.folder(l).is_some() && (!self.rename || !Naming::clean(&self.title.read(cx).value()).is_empty())
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.draft.read(cx).value().to_string();
        if !pending.trim().is_empty() {
            self.add_coauthor(&pending, cx);
            self.draft.update(cx, |s, cx| s.set_value("", window, cx));
        }
        let lib = state::lib(cx);
        let (folder, name) = {
            let l = lib.read(cx);
            (self.folder(l), self.final_name(l, cx))
        };
        let (Some(path), Some(folder)) = (self.path().cloned(), folder) else { return };
        if !self.can_file(lib.read(cx), cx) {
            return;
        }
        let assignment = self.target_assignment(lib.read(cx)).map(|a| a.id);
        act(cx, |l| l.file_for(&path, &folder, &name, assignment.as_deref(), true));
        if self.role == FileRole::Work {
            prefs::remember_coauthors(&self.coauthors);
        }
        self.advance(window, cx);
    }

    /// Files the current and all following files into the chosen folder with their best title.
    fn commit_remaining(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.facts.is_none() {
            return;
        }
        let Some(current) = self.path().cloned() else { return };
        let lib = state::lib(cx);
        let (folder, current_name, naming, namings, ignore, week, assignment) = {
            let l = lib.read(cx);
            (self.folder(l), self.final_name(l, cx), self.module_id(l).and_then(|m| l.naming(&m)).filter(|_| self.rename), l.namings(), l.title_ignore_words(), self.target_assignment(l).and_then(|a| a.week).or_else(|| l.current_week().map(|w| w.index)), self.target_assignment(l).map(|a| a.id))
        };
        let Some(folder) = folder else { return };
        let rest: Vec<PathBuf> = self.paths[self.index..].to_vec();
        let (role, byline) = (self.role, self.byline.read(cx).value().to_string());
        let shared_name = (role == FileRole::Work && naming.is_some()).then(|| current_name.clone());
        let numbers_files = shared_name.is_some();
        let task = cx.background_spawn(async move {
            rest.iter()
                .enumerate()
                .skip(1)
                .map(|(i, p)| {
                    let name = name_of(p);
                    if let Some(shared) = &shared_name {
                        return (p.clone(), numbered(shared, i + 1, extension(&name)));
                    }
                    let Some(naming) = &naming else { return (p.clone(), name) };
                    let f = FileFacts::read(p);
                    let t = filing::title_suggestions(&f, &namings, &ignore).first().map(|s| s.title.clone()).unwrap_or_else(|| Naming::guess_title(&name));
                    let d = f.name_date.unwrap_or_else(|| day::local_date(f.created));
                    let by = if role == FileRole::Brief { BRIEF_TAG.to_string() } else { byline.clone() };
                    (p.clone(), naming.render(&t, d, extension(&name), &[], week, None, Some(&by)))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |this, cx| {
            let others = task.await;
            let _ = this.update(cx, |_, cx| {
                let first = if numbers_files { numbered(&current_name, 1, extension(&current_name)) } else { current_name };
                let mut items = vec![(current, folder.clone(), first)];
                items.extend(others.into_iter().map(|(p, n)| (p, folder.clone(), n)));
                act(cx, |l| l.file_batch(items, assignment.as_deref()));
                cx.emit(FilingEvent::Finished);
            });
        })
        .detach();
    }

    fn advance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.index + 1 < self.paths.len() {
            self.index += 1;
            self.load_current(window, cx);
        } else {
            self.index = self.paths.len();
            cx.emit(FilingEvent::Finished);
        }
    }
}

impl FilingForm {
    /// The suggestion as one decision: where the file goes and what it will be called.
    #[allow(clippy::too_many_arguments)]
    fn summary(&self, me: Entity<Self>, thumb: AnyElement, preview: PathBuf, final_name: &str, valid: bool, can_file: bool, l: &Library, t: &Theme) -> Div {
        const CHOICE_MIN: f32 = 300.;
        let room = self.width - 2. * space::XL - space::XL - CHOICE_MIN;
        let stacked = room < 140.;
        let preview_w = if stacked { (self.width - 2. * space::XL).min(320.) } else { room.min(480.) };
        let preview_h = if stacked { 160. } else { preview_w * 1.375 };
        let source = self.path().map(|p| name_of(p)).unwrap_or_default();
        let picture = div().flex().flex_col().gap(px(space::S)).w(px(preview_w)).flex_none().child(
            div()
                .id("filing-thumb")
                .h(px(preview_h))
                .rounded(px(radius::CONTROL))
                .overflow_hidden()
                .bg(t.secondary_bg)
                .border_1()
                .border_color(t.separator)
                .hover({ let c = t.accent.opacity(0.5); move |s| s.border_color(c) })
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer().key_nav()
                .tooltip(tip(tr("Vorschau (Klick)")))
                .on_click(move |_, _, cx| super::preview::show(&preview, cx))
                .child(thumb),
        )
        .child(caption(source, t));

        let (role_label, role_icon) = match self.role {
            FileRole::Work => (tr("Meine Abgabe"), "file-up"),
            FileRole::Brief => (tr("Aufgabenstellung"), "clipboard-list"),
            FileRole::Material => (tr("Unterlagen"), "library"),
        };
        let described = self.target.as_ref().map(|target| self.describe(target, l, t));
        let folder = self.folder(l);
        let ready = self.facts.is_some();
        let multi = self.paths.len() > 1;
        let last = self.index + 1 >= self.paths.len();
        let (commit, change) = (me.clone(), me.clone());
        let mut choice = div().flex_1().min_w_0().flex().flex_col().gap(px(space::M));
        if multi {
            choice = choice.child(section_label(trf("%lld von %lld", &[&(self.index + 1), &self.paths.len()]), t));
        }
        choice = choice
            .child(div().flex().items_center().gap(px(space::XS)).text_size(px(font::CALLOUT)).text_color(t.secondary).child(icon(role_icon, iconsize::XS, t.secondary)).child(role_label))
            .children(described.map(|(name, tint, context)| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::BOLD).line_clamp(2).child(name))
                    .child(div().flex().items_center().gap(px(space::S)).text_size(px(font::CALLOUT)).text_color(t.secondary).child(div().size(px(space::S)).flex_none().rounded_full().bg(tint)).child(div().truncate().child(context)))
            }))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(space::S))
                    .p(px(space::M))
                    .rounded(px(radius::CONTROL))
                    .bg(t.secondary_bg)
                    .child(icon(if valid { "badge-check" } else { "triangle-alert" }, iconsize::S, if valid { t.green } else { t.orange }))
                    .child(div().flex_1().min_w_0().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).line_clamp(2).child(if ready { final_name.to_string() } else { "…".to_string() })),
            )
            .children(folder.map(|f| div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.secondary).child(icon("folder", iconsize::XS, t.secondary)).child(div().truncate().child(l.relative_path(&f)))))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .child(Button::new("file-commit").primary().label(if multi && !last { tr("Ablegen & weiter") } else { tr("Ablegen") }).disabled(!can_file).on_click(move |_, window, cx| commit.update(cx, |f, cx| f.commit(window, cx))))
                    .child(kbd("↩", t))
                    .child(Button::new("filing-change").ghost().label(tr("Ändern")).on_click(move |_, window, cx| change.update(cx, |f, cx| f.set_details(true, window, cx)))),
            );

        let others = self.alternatives();
        if !others.is_empty() {
            let mut rows = div().flex().flex_col().gap(px(space::XS));
            for (i, target) in others.into_iter().enumerate() {
                let (name, tint, context) = self.describe(&target, l, t);
                let e = me.clone();
                rows = rows.child(
                    div()
                        .id(SharedString::from(format!("filing-other-{i}")))
                        .w_full()
                        .soft(t.ink.opacity(0.06), radius::INNER)
                        .cursor_pointer().key_nav()
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .px(px(space::S))
                        .py(px(space::XS))
                        .rounded(px(radius::INNER))
                        .child(div().size(px(space::S)).flex_none().rounded_full().bg(tint))
                        .child(div().flex_1().min_w_0().truncate().text_size(px(font::CALLOUT)).child(name))
                        .child(div().flex_none().max_w(px(160.)).truncate().text_size(px(font::CAPTION)).text_color(t.secondary).child(context))
                        .child(kbd((i + 1).to_string(), t))
                        .on_click(move |_, window, cx| e.update(cx, |f, cx| f.select(target.clone(), window, cx))),
                );
            }
            choice = choice.child(div().flex().flex_col().gap(px(space::XS)).pt(px(space::S)).child(section_label(tr("Andere Ziele"), t)).child(rows));
        }
        div().flex().when(stacked, |d| d.flex_col()).items_start().gap(px(space::XL)).p(px(space::XL)).child(picture).child(choice.w_full().max_w(px(640.)))
    }

    fn footer(&self, me: Entity<Self>, has_folder: bool, can_file: bool, primary: bool, t: &Theme) -> Option<Div> {
        let last = self.index + 1 >= self.paths.len();
        let multi = self.paths.len() > 1;
        if !multi && !primary {
            return None;
        }
        let (e1, e2, e3) = (me.clone(), me.clone(), me);
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(space::S))
                .flex_none()
                .h(px(super::theme::control::FOOTBAR))
                .px(px(space::L))
                .border_t_1()
                .border_color(t.separator)
                .when(multi, |d| {
                    d.child(Button::new("skip").label(tr("Überspringen")).on_click(move |_, window, cx| e1.update(cx, |f, cx| f.advance(window, cx)))).when(!last, |d| {
                        d.child(
                            Button::new("file-rest")
                                .label(trf("Alle %lld hier ablegen", &[&(self.paths.len() - self.index)]))
                                .tooltip(tr("Restliche Dateien mit ihren erkannten Titeln in denselben Ordner legen"))
                                .disabled(!has_folder || self.facts.is_none())
                                .on_click(move |_, window, cx| e2.update(cx, |f, cx| f.commit_remaining(window, cx))),
                        )
                    })
                })
                .child(div().flex_1())
                .when(primary, |d| d.child(Button::new("file-commit").primary().label(if multi && !last { tr("Ablegen & weiter") } else { tr("Ablegen") }).disabled(!can_file).on_click(move |_, window, cx| e3.update(cx, |f, cx| f.commit(window, cx))))),
        )
    }

    /// The short form's keys: `e` opens all fields, `1` to `3` pick another target. False when nothing happened.
    pub fn quick_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.details {
            return false;
        }
        match key {
            "e" => self.set_details(true, window, cx),
            "1" | "2" | "3" => {
                let index = key.parse::<usize>().unwrap_or(1) - 1;
                let Some(target) = self.alternatives().into_iter().nth(index) else { return false };
                self.select(target, window, cx);
            }
            _ => return false,
        }
        true
    }

    fn shell(&self, body: Div, footer: Option<Div>, cx: &mut Context<Self>) -> Div {
        div()
            .capture_action(cx.listener(|f, _: &gpui_kit::component::input::Escape, window, cx| {
                if f.details {
                    f.set_details(false, window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|f, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                if k.key == "right" && k.modifiers.alt && (k.modifiers.platform || k.modifiers.control) && f.paths.len() > 1 {
                    f.advance(window, cx);
                    cx.stop_propagation();
                    return;
                }
                if f.details || !f.focus.is_focused(window) || e.is_held || k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
                    return;
                }
                match k.key.as_str() {
                    "enter" => f.commit(window, cx),
                    key => {
                        if !f.quick_key(key, window, cx) {
                            return;
                        }
                    }
                }
                cx.stop_propagation();
            }))
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .key_context("FilingForm")
            .child({
                let me = cx.entity();
                let known = self.width;
                canvas(
                    move |bounds, _, cx| {
                        let width = f32::from(bounds.size.width);
                        if (width - known).abs() > 1. {
                            me.update(cx, |f, cx| {
                                f.width = width;
                                cx.notify();
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            })
            .on_action(cx.listener(|f, _: &CommitFiling, window, cx| f.commit(window, cx)))
            .child(div().id("filing-scroll").flex_1().min_h_0().overflow_y_scrollbar().child(body))
            .children(footer)
    }
}

fn section_label(s: String, t: &Theme) -> Div {
    div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(s)
}

fn chip(id: SharedString, selected: bool, t: &Theme) -> Stateful<Div> {
    let (fill, hover) = if selected { (t.accent.opacity(0.15), t.accent.opacity(0.2)) } else { (t.ink.opacity(0.05), t.ink.opacity(0.1)) };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(space::S))
        .px(px(space::M))
        .py(px(space::S))
        .rounded(px(radius::CONTROL))
        .bg(fill)
        .soft(hover, radius::CONTROL)
        .cursor_pointer().key_nav()
        .when(selected, |d| d.border(px(2.)).border_color(t.accent))
}

fn suggestion(id: SharedString, label: String, selected: bool, t: &Theme) -> Stateful<Div> {
    let (fill, hover) = if selected { (t.accent.opacity(0.2), t.accent.opacity(0.28)) } else { (t.ink.opacity(0.06), t.ink.opacity(0.1)) };
    div().id(id).px(px(space::S)).py(px(space::XS)).rounded_full().bg(fill).soft(hover, radius::FULL).cursor_pointer().key_nav().text_size(px(font::CAPTION)).text_color(if selected { t.accent } else { t.primary }).child(label)
}

fn renames_by_default(cx: &App) -> bool {
    state::read(cx, |l| l.config.as_ref().is_none_or(|c| c.rename_on_filing))
}

fn has_convention(cx: &App) -> bool {
    state::read(cx, |l| l.config.as_ref().is_none_or(StudyConfig::naming_convention))
}

impl Render for FilingForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.follow_rename_setting(cx);
        let t = Theme::get(cx);
        let Some(path) = self.path().cloned() else {
            return unavailable("circle-check", tr("Fertig"), tr("Alle Dateien wurden abgelegt."), &t).into_any_element();
        };
        let me = cx.entity();
        let lib = state::lib(cx);
        let l = lib.read(cx);
        let folder = self.folder(l);
        let module_id = self.module_id(l);
        let final_name = self.final_name(l, cx);
        let valid = !(self.rename && self.role == FileRole::Work) || module_id.as_ref().and_then(|m| l.naming(m)).is_none_or(|n| n.is_valid(&final_name));
        let can_file = self.can_file(l, cx);
        let title_value = Naming::clean(&self.title.read(cx).value());

        let preview_path = path.clone();
        let thumb: AnyElement = match &self.thumbnail {
            Some(i) => img(i.clone()).size_full().object_fit(ObjectFit::Contain).into_any_element(),
            None if ["png", "jpg", "jpeg", "gif", "webp"].contains(&extension(&name_of(&path)).to_lowercase().as_str()) => img(path.clone()).size_full().object_fit(ObjectFit::Cover).into_any_element(),
            None => file_icon(&path, iconsize::HERO).into_any_element(),
        };
        if !self.details {
            let body = self.summary(me.clone(), thumb, preview_path, &final_name, valid, can_file, l, &t);
            let footer = self.footer(me, folder.is_some(), can_file, false, &t);
            return self.shell(body, footer, cx).into_any_element();
        }
        let p2 = path.clone();
        let less = me.clone();
        let header = div()
            .flex()
            .items_start()
            .gap(px(space::M))
            .child(
                div()
                    .id("filing-thumb")
                    .w(px(64.))
                    .h(px(80.))
                    .flex_none()
                    .rounded(px(radius::CONTROL))
                    .overflow_hidden()
                    .bg(t.secondary_bg)
                    .border_1()
                    .border_color(t.separator)
                    .hover({ let c = t.accent.opacity(0.5); move |s| s.border_color(c) })
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer().key_nav()
                    .tooltip(tip(tr("Vorschau (Klick)")))
                    .on_click(move |_, _, cx| super::preview::show(&preview_path, cx))
                    .child(thumb),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .when(self.paths.len() > 1, |d| d.child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(trf("%lld von %lld", &[&(self.index + 1), &self.paths.len()]))))
                    .child(div().font_weight(FontWeight::BOLD).line_clamp(2).child(name_of(&path)))
                    .when_some(self.facts.as_ref(), |d, f| d.child(caption(trf("Erstellt %@", &[&day::due(f.created, l.now)]), &t)))
                    .child(link_button("filing-preview", tr("Vorschau"), &t).flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).child(icon("eye", iconsize::XS, t.accent)).on_click(move |_, _, cx| super::preview::show(&p2, cx))),
            )
            .when(folder.is_some(), |d| d.child(link_button("filing-less", tr("Weniger"), &t).text_size(px(font::CAPTION)).on_click(move |_, window, cx| less.update(cx, |f, cx| f.set_details(false, window, cx)))));

        let role_hint = match self.role {
            FileRole::Work => tr("Zählt als erledigt, sobald sie im Abgabeordner liegt."),
            FileRole::Brief => tr("Liegt bei der Abgabe, zählt aber nicht als erledigt."),
            FileRole::Material => tr("Folien, Skripte, Handouts – Vorschläge nach deinen letzten Kursen."),
        };
        let mut roles = div().flex().rounded(px(radius::INNER)).bg(t.ink.opacity(0.06)).p(px(space::XXS));
        for r in FileRole::ALL {
            let (label, icon_name) = match r {
                FileRole::Work => (tr("Meine Abgabe"), "file-up"),
                FileRole::Brief => (tr("Aufgabenstellung"), "clipboard-list"),
                FileRole::Material => (tr("Unterlagen"), "library"),
            };
            let active = r == self.role;
            let e = me.clone();
            roles = roles.child(
                div()
                    .id(SharedString::from(format!("role-{label}")))
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .px(px(space::M))
                    .py(px(space::XS))
                    .rounded(px(radius::SMALL))
                    .when(!active, |d| d.soft(t.ink.opacity(0.06), radius::SMALL))
                    .cursor_pointer().key_nav()
                    .text_size(px(font::CALLOUT))
                    .when(active, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm().font_weight(FontWeight::MEDIUM))
                    .child(icon(icon_name, iconsize::XS, if active { t.primary } else { t.secondary }))
                    .child(label)
                    .on_click(move |_, window, cx| e.update(cx, |f, cx| f.set_role(r, window, cx))),
            );
        }
        let role_section = div()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .child(section_label(tr("Was ist das?"), &t))
                    .when(self.guessed == Some(self.role), |d| d.child(div().id("guessed").flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.secondary).child(icon("sparkles", iconsize::XS, t.secondary)).child(tr("erkannt")).tooltip(tip(tr("Aus Dateiname und Inhalt geschätzt"))))),
            )
            .child(div().flex().child(roles))
            .child(caption(role_hint, &t));

        let mut chips = div().flex().flex_wrap().gap(px(space::S));
        if self.role == FileRole::Material {
            for p in self.places.iter().take(4) {
                let selected = self.target == Some(Target::Folder(p.path.clone()));
                let module_root = l.root.as_ref().map(|r| r.join(&p.module.folder)).unwrap_or_default();
                let inner = p.path.strip_prefix(&module_root).map(|r| r.iter().map(|c| c.to_string_lossy().to_string()).collect::<Vec<_>>()).unwrap_or_default();
                let label = if inner.is_empty() { name_of(&p.path) } else { inner[inner.len().saturating_sub(2)..].join(" › ") };
                let (e, f) = (me.clone(), p.path.clone());
                let ctx = p.context.clone();
                chips = chips.child(
                    chip(SharedString::from(format!("place-{}", p.path.display())), selected, &t)
                        .tooltip(tip(l.relative_path(&p.path)))
                        .child(div().size(px(space::S)).rounded_full().bg(t.palette(&p.module.color)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).truncate().max_w(px(220.)).child(label))
                                .child(div().flex().gap(px(space::XS)).text_size(px(font::CAPTION2)).text_color(t.secondary).child(p.module.code.clone()).when_some(ctx, |d, c| d.child("·").child(div().text_color(if c.days_ago == 0 { t.accent } else { t.secondary }).child(c.day_label())))),
                        )
                        .when(selected, |d| d.child(icon("check", iconsize::XS, t.accent)))
                        .on_click(move |_, window, cx| e.update(cx, |form, cx| form.select(Target::Folder(f.clone()), window, cx))),
                );
            }
        } else {
            for a in self.ranked.iter().take(4) {
                let selected = self.target == Some(Target::Assignment(a.id.clone()));
                let m = l.module(a);
                let (e, id) = (me.clone(), a.id.clone());
                chips = chips.child(
                    chip(SharedString::from(format!("target-{}", a.id)), selected, &t)
                        .child(div().size(px(space::S)).rounded_full().bg(t.palette(m.map_or("blue", |m| m.color.as_str()))))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).truncate().max_w(px(220.)).child(a.title.clone()))
                                .child(div().text_size(px(font::CAPTION2)).text_color(t.secondary).child(format!("{} · {}", m.map_or("", |m| m.code.as_str()), day::day_label(a.due, l.now)))),
                        )
                        .when(selected, |d| d.child(icon("check", iconsize::XS, t.accent)))
                        .on_click(move |_, window, cx| e.update(cx, |form, cx| form.select(Target::Assignment(id.clone()), window, cx))),
                );
            }
        }
        let other_label = match &self.target {
            Some(Target::Folder(f)) if !self.places.iter().take(if self.role == FileRole::Material { 4 } else { 0 }).any(|p| &p.path == f) => name_of(f),
            Some(Target::Assignment(id)) if !self.ranked.iter().take(4).any(|a| &a.id == id) => l.assignment(id).map(|a| a.title.clone()).unwrap_or_default(),
            _ => tr("Andere…"),
        };
        let (material, ranked, places) = (self.role == FileRole::Material, self.ranked.clone(), self.places.clone());
        let modules: Vec<(String, String)> = l.semester_modules().iter().map(|m| (m.id.clone(), m.name.clone())).collect();
        let folders: Vec<(PathBuf, String)> = l.suggested_folders(None).into_iter().map(|f| (f.clone(), l.relative_path(&f))).collect();
        let relative: Vec<(PathBuf, String)> = places.iter().map(|p| (p.path.clone(), l.relative_path(&p.path))).collect();
        let now = l.now;
        let e = me.clone();
        chips = chips.child(Button::new("other-target").small().icon(menu_icon("ellipsis")).label(other_label).dropdown_menu(move |mut menu, _, _| {
            let pick = |t: Target| {
                let e = e.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| e.update(cx, |f, cx| f.select(t.clone(), window, cx))
            };
            for (mid, mname) in &modules {
                if material {
                    let items: Vec<_> = places.iter().zip(&relative).filter(|(p, _)| &p.module.id == mid).collect();
                    if items.is_empty() {
                        continue;
                    }
                    menu = menu.label(mname.clone());
                    for (p, (_, rel)) in items {
                        menu = menu.item(PopupMenuItem::new(rel.clone()).on_click(pick(Target::Folder(p.path.clone()))));
                    }
                } else {
                    let items: Vec<_> = ranked.iter().filter(|a| &a.module_id == mid).collect();
                    if items.is_empty() {
                        continue;
                    }
                    menu = menu.label(mname.clone());
                    for a in items {
                        menu = menu.item(PopupMenuItem::new(format!("{} · {}", a.title, day::day_label(a.due, now))).on_click(pick(Target::Assignment(a.id.clone()))));
                    }
                }
            }
            if !folders.is_empty() {
                menu = menu.separator().label(tr("Ordner"));
                for (f, rel) in &folders {
                    menu = menu.item(PopupMenuItem::new(rel.clone()).on_click(pick(Target::Folder(f.clone()))));
                }
            }
            let e2 = e.clone();
            menu.separator().item(PopupMenuItem::new(tr("Anderen Ordner wählen…")).on_click(move |_, _, cx| {
                let e = e2.clone();
                super::platform::choose_folder(cx, move |p, cx| {
                    if let Some(w) = cx.active_window() {
                        let _ = w.update(cx, |_, window, cx| e.update(cx, |f, cx| f.select(Target::Folder(p), window, cx)));
                    }
                })
            }))
        }));
        let target_section = div()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .child(section_label(tr("Wohin?"), &t))
            .child(chips)
            .child(match &folder {
                Some(f) => div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.secondary).child(icon("folder", iconsize::XS, t.secondary)).child(div().truncate().child(l.relative_path(f))),
                None if self.target_assignment(l).is_some() => div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.orange).child(icon("folder-search", iconsize::XS, t.orange)).child(tr("Der Ordner dieser Abgabe fehlt. Wähle einen Ordner.")),
                None => caption(if material { tr("Wähle einen Ordner.") } else { tr("Wähle eine Abgabe oder einen Ordner.") }, &t),
            });

        let mut body = div().flex().flex_col().gap(px(space::XL)).p(px(space::XL)).child(header).child(role_section).child(target_section);

        if folder.is_some() {
            let e = me.clone();
            let convention = has_convention(cx);
            let mut name = div().flex().flex_col().gap(px(space::M)).child(
                div().flex().items_center().child(section_label(tr("Name"), &t)).child(div().flex_1()).when(convention, |d| d.child(
                    div()
                        .id("rename-switch")
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .text_size(px(font::CAPTION))
                        .child(tr("Nach Konvention benennen"))
                        .child(Switch::new("rename").small().checked(self.rename && module_id.is_some()).on_click(move |v, _, cx| {
                            let v = *v;
                            e.update(cx, |f, cx| {
                                f.rename = v;
                                cx.notify();
                            })
                        }))
                        .tooltip(tip(if module_id.is_none() { tr("Ordner gehört zu keinem Modul") } else { tr("Muster aus den Moduleinstellungen") })),
                )),
            );
            if self.rename && module_id.is_some() {
                let mut sugg = div().flex().flex_wrap().gap(px(space::S));
                for (i, s) in self.titles.iter().enumerate() {
                    let (e, value) = (me.clone(), s.title.clone());
                    sugg = sugg.child(suggestion(SharedString::from(format!("title-{i}")), s.title.clone(), title_value == s.title, &t).tooltip(tip(tr(s.source))).on_click(move |_, window, cx| {
                        let v = value.clone();
                        e.update(cx, |f, cx| f.title.update(cx, |s, cx| s.set_value(v, window, cx)))
                    }));
                }
                name = name.child(div().flex().flex_col().gap(px(space::S)).child(Input::new(&self.title)).when(self.titles.iter().any(|s| s.title != title_value), |d| d.child(sugg)));
                match self.role {
                    FileRole::Work => {
                        let mut row = div().flex().flex_wrap().items_center().gap(px(space::S));
                        for (i, n) in self.coauthors.iter().enumerate() {
                            let (e, n2) = (me.clone(), n.clone());
                            row = row.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::XS))
                                    .px(px(space::S))
                                    .py(px(space::XS))
                                    .rounded_full()
                                    .bg(t.accent.opacity(0.15))
                                    .text_size(px(font::CAPTION))
                                    .child(n.clone())
                                    .child(div().id(("rm-co", i)).rounded_full().soft(t.ink.opacity(0.1), radius::FULL).cursor_pointer().key_nav().child(icon("circle-x", iconsize::XS, t.secondary)).tooltip(tip(trf("%@ entfernen", &[&n]))).on_click(move |_, _, cx| e.update(cx, |f, cx| {
                                        f.coauthors.retain(|c| *c != n2);
                                        cx.notify();
                                    }))),
                            );
                        }
                        row = row.child(div().w(px(200.)).child(Input::new(&self.draft).small()));
                        let recent: Vec<String> = prefs::get().recent_coauthors.into_iter().filter(|c| !self.coauthors.contains(c)).take(4).collect();
                        for (i, n) in recent.into_iter().enumerate() {
                            let e = me.clone();
                            row = row.child(suggestion(SharedString::from(format!("recent-co-{i}")), format!("+ {n}"), false, &t).on_click(move |_, _, cx| e.update(cx, |f, cx| {
                                f.coauthors.push(n.clone());
                                cx.notify();
                            })));
                        }
                        name = name.child(row);
                    }
                    FileRole::Material => {
                        let known = folder.as_ref().map(|f| l.bylines_near(f)).unwrap_or_default();
                        let current = self.byline.read(cx).value().to_string();
                        let mut sugg = div().flex().flex_wrap().gap(px(space::S));
                        for (i, b) in known.into_iter().take(4).enumerate() {
                            let selected = current == b || Naming::clean_author(&current, CoauthorStyle::Initial) == b;
                            let (e, v) = (me.clone(), b.clone());
                            sugg = sugg.child(suggestion(SharedString::from(format!("byline-{i}")), b, selected, &t).tooltip(tip(tr("So heißen andere Dateien in diesem Ordner"))).on_click(move |_, window, cx| {
                                let v = v.clone();
                                e.update(cx, |f, cx| f.byline.update(cx, |s, cx| s.set_value(v, window, cx)))
                            }));
                        }
                        name = name.child(div().flex().flex_col().gap(px(space::S)).child(div().max_w(px(320.)).child(Input::new(&self.byline))).child(sugg));
                    }
                    FileRole::Brief => {}
                }
                let mut dates = div().flex().items_center().gap(px(space::S)).child(div().w(px(150.)).child(DatePicker::new(&self.date).small()));
                let today = l.today();
                let e = me.clone();
                dates = dates.child(suggestion("date-today".into(), tr("Heute"), self.date_value == today, &t).on_click(move |_, window, cx| e.update(cx, |f, cx| f.set_date(today, window, cx))));
                if let Some(a) = self.target_assignment(l) {
                    let (e, d) = (me.clone(), day::local_date(a.due));
                    dates = dates.child(suggestion("date-due".into(), tr("Fälligkeit"), self.date_value == d, &t).on_click(move |_, window, cx| e.update(cx, |f, cx| f.set_date(d, window, cx))));
                }
                if let Some(f) = &self.facts {
                    let (e, d) = (me.clone(), f.name_date.unwrap_or_else(|| day::local_date(f.created)));
                    dates = dates.child(suggestion("date-file".into(), tr("Datei"), self.date_value == d, &t).on_click(move |_, window, cx| e.update(cx, |f, cx| f.set_date(d, window, cx))));
                }
                name = name.child(dates);
            }
            name = name.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(space::S))
                    .p(px(space::M))
                    .rounded(px(radius::CONTROL))
                    .bg(t.secondary_bg)
                    .child(icon(if valid { "badge-check" } else { "triangle-alert" }, iconsize::S, if valid { t.green } else { t.orange }))
                    .child(div().flex_1().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).line_clamp(2).child(final_name)),
            );
            body = body.child(name);
        }

        let footer = self.footer(me, folder.is_some(), can_file, true, &t);
        self.shell(body, footer, cx).into_any_element()
    }
}

actions!(schrumbi_filing, [CommitFiling]);

/// A file of a series that belongs together: the shared name with a running number, e.g. `Foto_261102_2.jpg`.
fn numbered(name: &str, number: usize, ext: &str) -> String {
    let current = extension(name).to_string();
    let stem = name.strip_suffix(&format!(".{current}")).unwrap_or(name);
    if ext.is_empty() { format!("{stem}_{number}") } else { format!("{stem}_{number}.{ext}") }
}
