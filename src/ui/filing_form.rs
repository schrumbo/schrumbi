use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Local, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::{self, act};
use super::theme::{Theme, font, radius};
use super::widgets::*;
use crate::core::day;
use crate::core::filing::{self, BRIEF_TAG, FileFacts, FileRole, Suggestion};
use crate::core::i18n::{tr, trf};
use crate::core::library::{Library, MaterialTarget};
use crate::core::model::{Assignment, CoauthorStyle};
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
    pub fn new(paths: Vec<PathBuf>, preset: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            rename: true,
            load: None,
            _subs: subs,
        };
        form.load_current(window, cx);
        form
    }

    fn path(&self) -> Option<&PathBuf> {
        self.paths.get(self.index)
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
        let p = path.clone();
        let task = cx.background_spawn(async move {
            let facts = FileFacts::read(&p);
            let thumb = if extension(&name_of(&p)).eq_ignore_ascii_case("pdf") {
                crate::core::pdf_render::PdfDoc::open(&p).and_then(|d| d.render_png(0, 0.5)).map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)))
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
        self.title.update(cx, |s, cx| s.focus(window, cx));
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
            Some(Target::Assignment(_)) => self.target_assignment(l).map(|a| a.folder),
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
            self.rename = true;
        } else {
            self.select_default(window, cx);
        }
        cx.notify();
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
                self.rename = true;
                if self.role == FileRole::Work
                    && let Some(a) = state::read(cx, |l| l.assignment(id).cloned())
                {
                    self.set_date(day::local_date(a.due.min(Local::now())), window, cx);
                }
            }
            Target::Folder(f) => {
                let (in_module, near) = state::read(cx, |l| (l.module_containing(f).is_some(), l.bylines_near(f)));
                self.rename = in_module;
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
        self.folder(l).is_some() && (!self.rename || !Naming::clean(&self.title.read(cx).value()).is_empty())
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
        act(cx, |l| l.file(&path, &folder, &name, true));
        if self.role == FileRole::Work {
            prefs::remember_coauthors(&self.coauthors);
        }
        self.advance(window, cx);
    }

    /// Files the current and all following files into the chosen folder with their best title.
    fn commit_remaining(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lib = state::lib(cx);
        let (folder, current_name, naming, namings, ignore, week) = {
            let l = lib.read(cx);
            (self.folder(l), self.final_name(l, cx), self.module_id(l).and_then(|m| l.naming(&m)).filter(|_| self.rename), l.namings(), l.title_ignore_words(), self.target_assignment(l).and_then(|a| a.week).or_else(|| l.current_week().map(|w| w.index)))
        };
        let Some(folder) = folder else { return };
        let rest: Vec<PathBuf> = self.paths[self.index..].to_vec();
        let (role, byline) = (self.role, self.byline.read(cx).value().to_string());
        let task = cx.background_spawn(async move {
            rest.iter()
                .skip(1)
                .map(|p| {
                    let name = name_of(p);
                    let Some(naming) = &naming else { return (p.clone(), name) };
                    let f = FileFacts::read(p);
                    let t = filing::title_suggestions(&f, &namings, &ignore).first().map(|s| s.title.clone()).unwrap_or_else(|| Naming::guess_title(&name));
                    let d = f.name_date.unwrap_or_else(|| day::local_date(f.created));
                    let by = if role == FileRole::Brief { BRIEF_TAG.to_string() } else { byline.clone() };
                    (p.clone(), naming.render(&t, d, extension(&name), &[], week, None, Some(&by)))
                })
                .collect::<Vec<_>>()
        });
        let current = self.paths[self.index].clone();
        cx.spawn_in(window, async move |this, cx| {
            let others = task.await;
            let _ = this.update(cx, |_, cx| {
                let mut items = vec![(current, folder.clone(), current_name)];
                items.extend(others.into_iter().map(|(p, n)| (p, folder.clone(), n)));
                act(cx, |l| l.file_batch(items));
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
            cx.emit(FilingEvent::Finished);
        }
    }
}

fn section_label(s: String, t: &Theme) -> Div {
    div().text_size(px(font::SUBHEADLINE)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(s)
}

fn chip(id: SharedString, selected: bool, t: &Theme) -> Stateful<Div> {
    let (fill, hover) = if selected { (t.accent.opacity(0.15), t.accent.opacity(0.2)) } else { (t.ink.opacity(0.05), t.ink.opacity(0.1)) };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(radius::CONTROL))
        .bg(fill)
        .hover(move |s| s.bg(hover))
        .cursor_pointer()
        .when(selected, |d| d.border(px(2.)).border_color(t.accent))
}

fn suggestion(id: SharedString, label: String, selected: bool, t: &Theme) -> Stateful<Div> {
    let (fill, hover) = if selected { (t.accent.opacity(0.2), t.accent.opacity(0.28)) } else { (t.ink.opacity(0.06), t.ink.opacity(0.1)) };
    div().id(id).px(px(8.)).py(px(3.)).rounded_full().bg(fill).hover(move |s| s.bg(hover)).cursor_pointer().text_size(px(font::CAPTION)).text_color(if selected { t.accent } else { t.primary }).child(label)
}

impl Render for FilingForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            None => file_icon(&path, 36.).into_any_element(),
        };
        let p2 = path.clone();
        let header = div()
            .flex()
            .items_start()
            .gap(px(14.))
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
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
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
                    .gap(px(4.))
                    .when(self.paths.len() > 1, |d| d.child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(trf("%lld von %lld", &[&(self.index + 1), &self.paths.len()]))))
                    .child(div().font_weight(FontWeight::BOLD).line_clamp(2).child(name_of(&path)))
                    .when_some(self.facts.as_ref(), |d, f| d.child(caption(trf("Erstellt %@", &[&day::due(f.created, l.now)]), &t)))
                    .child(link_button("filing-preview", tr("Vorschau"), &t).flex().items_center().gap(px(3.)).text_size(px(font::CAPTION)).child(icon("eye", 11., t.accent)).on_click(move |_, _, cx| super::preview::show(&p2, cx))),
            );

        let role_hint = match self.role {
            FileRole::Work => tr("Zählt als erledigt, sobald sie im Abgabeordner liegt."),
            FileRole::Brief => tr("Liegt bei der Abgabe, zählt aber nicht als erledigt."),
            FileRole::Material => tr("Folien, Skripte, Handouts – Vorschläge nach deinen letzten Kursen."),
        };
        let mut roles = div().flex().rounded(px(6.)).bg(t.ink.opacity(0.06)).p(px(2.));
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
                    .gap(px(5.))
                    .px(px(10.))
                    .py(px(3.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(font::CALLOUT))
                    .when(active, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm().font_weight(FontWeight::MEDIUM))
                    .child(icon(icon_name, 12., if active { t.primary } else { t.secondary }))
                    .child(label)
                    .on_click(move |_, window, cx| e.update(cx, |f, cx| f.set_role(r, window, cx))),
            );
        }
        let role_section = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(section_label(tr("Was ist das?"), &t))
                    .when(self.guessed == Some(self.role), |d| d.child(div().id("guessed").flex().items_center().gap(px(3.)).text_size(px(font::CAPTION)).text_color(t.secondary).child(icon("sparkles", 11., t.secondary)).child(tr("erkannt")).tooltip(tip(tr("Aus Dateiname und Inhalt geschätzt"))))),
            )
            .child(div().flex().child(roles))
            .child(caption(role_hint, &t));

        let mut chips = div().flex().flex_wrap().gap(px(6.));
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
                        .child(div().size(px(7.)).rounded_full().bg(t.palette(&p.module.color)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).truncate().max_w(px(220.)).child(label))
                                .child(div().flex().gap(px(4.)).text_size(px(font::CAPTION2)).text_color(t.secondary).child(p.module.code.clone()).when_some(ctx, |d, c| d.child("·").child(div().text_color(if c.days_ago == 0 { t.accent } else { t.secondary }).child(c.day_label())))),
                        )
                        .when(selected, |d| d.child(icon("check", 12., t.accent)))
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
                        .child(div().size(px(7.)).rounded_full().bg(t.palette(m.map_or("blue", |m| m.color.as_str()))))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).truncate().max_w(px(220.)).child(a.title.clone()))
                                .child(div().text_size(px(font::CAPTION2)).text_color(t.secondary).child(format!("{} · {}", m.map_or("", |m| m.code.as_str()), day::day_label(a.due, l.now)))),
                        )
                        .when(selected, |d| d.child(icon("check", 12., t.accent)))
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
            .gap(px(10.))
            .child(section_label(tr("Wohin?"), &t))
            .child(chips)
            .child(match &folder {
                Some(f) => div().flex().items_center().gap(px(4.)).text_size(px(font::CAPTION)).text_color(t.secondary).child(icon("folder", 11., t.secondary)).child(div().truncate().child(l.relative_path(f))),
                None => caption(if material { tr("Wähle einen Ordner.") } else { tr("Wähle eine Abgabe oder einen Ordner.") }, &t),
            });

        let mut body = div().flex().flex_col().gap(px(20.)).p(px(22.)).child(header).child(role_section).child(target_section);

        if folder.is_some() {
            let e = me.clone();
            let mut name = div().flex().flex_col().gap(px(14.)).child(
                div().flex().items_center().child(section_label(tr("Name"), &t)).child(div().flex_1()).child(
                    div()
                        .id("rename-switch")
                        .flex()
                        .items_center()
                        .gap(px(6.))
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
                ),
            );
            if self.rename && module_id.is_some() {
                let mut sugg = div().flex().flex_wrap().gap(px(6.));
                for (i, s) in self.titles.iter().enumerate() {
                    let (e, value) = (me.clone(), s.title.clone());
                    sugg = sugg.child(suggestion(SharedString::from(format!("title-{i}")), s.title.clone(), title_value == s.title, &t).tooltip(tip(tr(s.source))).on_click(move |_, window, cx| {
                        let v = value.clone();
                        e.update(cx, |f, cx| f.title.update(cx, |s, cx| s.set_value(v, window, cx)))
                    }));
                }
                name = name.child(div().flex().flex_col().gap(px(6.)).child(Input::new(&self.title)).when(!self.titles.is_empty(), |d| d.child(sugg)));
                match self.role {
                    FileRole::Work => {
                        let mut row = div().flex().flex_wrap().items_center().gap(px(6.));
                        for (i, n) in self.coauthors.iter().enumerate() {
                            let (e, n2) = (me.clone(), n.clone());
                            row = row.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .px(px(8.))
                                    .py(px(3.))
                                    .rounded_full()
                                    .bg(t.accent.opacity(0.15))
                                    .text_size(px(font::CAPTION))
                                    .child(n.clone())
                                    .child(div().id(("rm-co", i)).cursor_pointer().child(icon("circle-x", 12., t.secondary)).tooltip(tip(trf("%@ entfernen", &[&n]))).on_click(move |_, _, cx| e.update(cx, |f, cx| {
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
                        let mut sugg = div().flex().flex_wrap().gap(px(6.));
                        for (i, b) in known.into_iter().take(4).enumerate() {
                            let selected = current == b || Naming::clean_author(&current, CoauthorStyle::Initial) == b;
                            let (e, v) = (me.clone(), b.clone());
                            sugg = sugg.child(suggestion(SharedString::from(format!("byline-{i}")), b, selected, &t).tooltip(tip(tr("So heißen andere Dateien in diesem Ordner"))).on_click(move |_, window, cx| {
                                let v = v.clone();
                                e.update(cx, |f, cx| f.byline.update(cx, |s, cx| s.set_value(v, window, cx)))
                            }));
                        }
                        name = name.child(div().flex().flex_col().gap(px(6.)).child(div().max_w(px(320.)).child(Input::new(&self.byline))).child(sugg));
                    }
                    FileRole::Brief => {}
                }
                let mut dates = div().flex().items_center().gap(px(8.)).child(div().w(px(150.)).child(DatePicker::new(&self.date).small()));
                let today = l.today();
                let e = me.clone();
                dates = dates.child(suggestion("date-today".into(), tr("Heute"), false, &t).on_click(move |_, window, cx| e.update(cx, |f, cx| f.set_date(today, window, cx))));
                if let Some(a) = self.target_assignment(l) {
                    let (e, d) = (me.clone(), day::local_date(a.due));
                    dates = dates.child(suggestion("date-due".into(), tr("Fälligkeit"), false, &t).on_click(move |_, window, cx| e.update(cx, |f, cx| f.set_date(d, window, cx))));
                }
                if let Some(f) = &self.facts {
                    let (e, d) = (me.clone(), f.name_date.unwrap_or_else(|| day::local_date(f.created)));
                    dates = dates.child(suggestion("date-file".into(), tr("Datei"), false, &t).on_click(move |_, window, cx| e.update(cx, |f, cx| f.set_date(d, window, cx))));
                }
                name = name.child(dates);
            }
            name = name.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(radius::CONTROL))
                    .bg(t.secondary_bg)
                    .child(icon(if valid { "badge-check" } else { "triangle-alert" }, 14., if valid { t.green } else { t.orange }))
                    .child(div().flex_1().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).line_clamp(2).child(final_name)),
            );
            body = body.child(name);
        }

        let last = self.index + 1 >= self.paths.len();
        let multi = self.paths.len() > 1;
        let (e1, e2, e3) = (me.clone(), me.clone(), me.clone());
        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .p(px(14.))
            .border_t_1()
            .border_color(t.separator)
            .when(multi, |d| {
                d.child(Button::new("skip").label(tr("Überspringen")).on_click(move |_, window, cx| e1.update(cx, |f, cx| f.advance(window, cx)))).when(self.role != FileRole::Work && !last, |d| {
                    d.child(
                        Button::new("file-rest")
                            .label(trf("Alle %lld hier ablegen", &[&(self.paths.len() - self.index)]))
                            .tooltip(tr("Restliche Dateien mit ihren erkannten Titeln in denselben Ordner legen"))
                            .disabled(folder.is_none())
                            .on_click(move |_, window, cx| e2.update(cx, |f, cx| f.commit_remaining(window, cx))),
                    )
                })
            })
            .child(div().flex_1())
            .child(Button::new("file-commit").primary().label(if multi && !last { tr("Ablegen & weiter") } else { tr("Ablegen") }).disabled(!can_file).on_click(move |_, window, cx| e3.update(cx, |f, cx| f.commit(window, cx))));

        div()
            .on_key_down(cx.listener(|f, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                if k.key == "right" && k.modifiers.alt && (k.modifiers.platform || k.modifiers.control) && f.paths.len() > 1 {
                    f.advance(window, cx);
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .key_context("FilingForm")
            .on_action(cx.listener(|f, _: &CommitFiling, window, cx| f.commit(window, cx)))
            .child(div().id("filing-scroll").flex_1().min_h_0().overflow_y_scroll().child(body))
            .child(footer)
            .into_any_element()
    }
}

actions!(schrumbi_filing, [CommitFiling]);
