use chrono::{Days, Local, NaiveTime};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::inspector::{datetime_picker, picked};
use std::path::{Path as FilePath, PathBuf};

use crate::core::vault::index;
use super::state::{self, act};
use super::motion::Soft;
use super::theme::{Theme, font, radius, space, iconsize};
use super::widgets::*;
use crate::core::natural_date;
use crate::core::catalog::{self, CatalogFile, FileKind};
use crate::core::day;
use crate::core::i18n::{tr, trf};
use crate::core::model::{Kind, ManualAssignment};
use crate::core::text::fold;

/// Emitted when a sheet is done, so its dialog closes.
pub struct Dismissed;

/// Adds a one-off task, submission or exam.
pub struct QuickAdd {
    module: String,
    kind: Kind,
    title: Entity<InputState>,
    /// The title without the date words found in it.
    clean_title: Option<String>,
    due: Entity<DatePickerState>,
    folder: String,
    optional: bool,
    focused: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<Dismissed> for QuickAdd {}

fn default_time(kind: Kind) -> NaiveTime {
    match kind {
        Kind::Task => NaiveTime::from_hms_opt(18, 0, 0),
        Kind::Submission => NaiveTime::from_hms_opt(23, 59, 0),
        Kind::Exam => NaiveTime::from_hms_opt(10, 0, 0),
    }
    .unwrap_or_default()
}

impl QuickAdd {
    pub fn new(module: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder(tr("z. B. Kapitel 3 lesen")));
        let due = datetime_picker(window, cx);
        let tomorrow = Local::now().date_naive() + Days::new(1);
        due.update(cx, |s, cx| s.set_date_time(tomorrow.and_time(default_time(Kind::Task)), window, cx));
        let module = module.or_else(|| state::read(cx, |l| l.semester_modules().first().map(|m| m.id.clone()))).unwrap_or_default();
        let subs = vec![
            cx.subscribe_in(&title, window, |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::PressEnter { .. } => this.add(cx),
                InputEvent::Change => {
                    this.read_date_words(window, cx);
                    cx.notify();
                }
                _ => {}
            }),
            cx.subscribe(&due, |_, _, _: &DatePickerEvent, cx| cx.notify()),
        ];
        Self { module, kind: Kind::Task, title, clean_title: None, due, folder: String::new(), optional: false, focused: false, _subs: subs }
    }

    fn set_kind(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        self.kind = kind;
        let placeholder = match kind {
            Kind::Task => tr("z. B. Kapitel 3 lesen"),
            Kind::Submission => tr("z. B. Essay Entwurf"),
            Kind::Exam => tr("z. B. Klausur Theorie 1"),
        };
        self.title.update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        if let Some(d) = self.due.read(cx).date_time().start() {
            self.due.update(cx, |s, cx| s.set_date_time(d.date().and_time(default_time(kind)), window, cx));
        }
        cx.notify();
    }

    /// Lets "Referat morgen 18 Uhr" set the date field.
    fn read_date_words(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.title.read(cx).value().to_string();
        let found = natural_date::parse(&text, Local::now().date_naive()).filter(|n| !n.title.is_empty());
        self.clean_title = found.as_ref().map(|n| n.title.clone());
        if let Some(n) = found {
            let date = n.date.or_else(|| self.due.read(cx).date_time().start().map(|d| d.date()));
            if let Some(date) = date {
                let time = n.time.unwrap_or_else(|| default_time(self.kind));
                self.due.update(cx, |s, cx| s.set_date_time(date.and_time(time), window, cx));
            }
        }
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        let title = self.clean_title.clone().unwrap_or_else(|| self.title.read(cx).value().trim().to_string());
        let Some(due) = picked(&self.due, cx) else { return };
        if title.is_empty() || self.module.is_empty() {
            return;
        }
        let m = ManualAssignment {
            module: self.module.clone(),
            title,
            due,
            kind: self.kind,
            optional: self.kind == Kind::Submission && self.optional,
            upload: self.kind == Kind::Submission,
            folder: if self.kind == Kind::Submission { self.folder.clone() } else { String::new() },
            ..Default::default()
        };
        act(cx, |l| l.add_manual(m));
        cx.emit(Dismissed);
    }
}

fn subfolders(module: &str, cx: &App) -> Vec<String> {
    let Some(base) = state::read(cx, |l| l.root.clone().zip(l.module_by_id(module).map(|m| m.folder.clone())).map(|(r, f)| r.join(f))) else { return Vec::new() };
    let re = regex::Regex::new(r"^W\d{2}").unwrap();
    let mut v: Vec<String> = std::fs::read_dir(base)
        .map(|d| d.filter_map(Result::ok).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| !n.starts_with('.') && !re.is_match(n)).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn row(label: String, content: impl IntoElement, t: &Theme) -> Div {
    div().flex().items_start().gap(px(space::M)).child(div().w(px(70.)).pt(px(space::XS)).text_color(t.secondary).child(label)).child(div().flex_1().child(content))
}

impl Render for QuickAdd {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            self.focused = true;
            self.title.update(cx, |s, cx| s.focus(window, cx));
        }
        let t = Theme::get(cx);
        let me = cx.entity();
        let modules: Vec<(String, String)> = state::read(cx, |l| l.semester_modules().iter().map(|m| (m.id.clone(), m.display_name())).collect());
        let current = modules.iter().find(|m| m.0 == self.module).map(|m| m.1.clone()).unwrap_or_default();
        let mut kinds = div().flex().rounded(px(radius::INNER)).bg(t.ink.opacity(0.06)).p(px(space::XXS));
        for k in Kind::ALL {
            let active = k == self.kind;
            let e = me.clone();
            kinds = kinds.child(
                div()
                    .id(SharedString::from(format!("kind-{}", k.label())))
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(space::XS))
                    .py(px(space::XS))
                    .rounded(px(radius::INNER))
                    .cursor_pointer().key_nav()
                    .when(active, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm().font_weight(FontWeight::MEDIUM))
                    .child(icon(k.icon(), iconsize::S, if active { t.primary } else { t.secondary }))
                    .child(k.label())
                    .on_click(move |_, window, cx| e.update(cx, |q, cx| q.set_kind(k, window, cx))),
            );
        }
        let e = me.clone();
        let module_menu = Button::new("qa-module").label(current).dropdown_caret(true).dropdown_menu(move |mut menu, _, _| {
            for (id, name) in &modules {
                let (e, id) = (e.clone(), id.clone());
                menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| e.update(cx, |q, cx| {
                    q.module = id.clone();
                    q.folder.clear();
                    cx.notify();
                })));
            }
            menu
        });
        let today = Local::now().date_naive();
        let next_monday = day::monday(today) + Days::new(7);
        let quick: Vec<(String, chrono::NaiveDate)> = vec![(tr("Heute"), today), (tr("Morgen"), today + Days::new(1)), (tr("Nächster Mo"), next_monday), (tr("In 1 Woche"), today + Days::new(7))];
        let mut chips = div().flex().gap(px(space::S));
        for (i, (label, d)) in quick.into_iter().enumerate() {
            let e = me.clone();
            chips = chips.child(Button::new(("qa-date", i)).small().label(label).on_click(move |_, window, cx| e.update(cx, |q, cx| {
                let t = default_time(q.kind);
                q.due.update(cx, |s, cx| s.set_date_time(d.and_time(t), window, cx));
                cx.notify();
            })));
        }
        let countdown = picked(&self.due, cx).map(|d| day::countdown(d.with_timezone(&Local), Local::now())).unwrap_or_default();
        let mut grid = div()
            .flex()
            .flex_col()
            .gap(px(space::M))
            .child(row(tr("Modul"), module_menu, &t))
            .child(row(if self.kind == Kind::Exam { tr("Termin") } else { tr("Fällig") }, div().flex().flex_col().gap(px(space::S)).child(div().w(px(240.)).child(DatePicker::new(&self.due))).child(chips).child(caption(countdown, &t)), &t));
        if self.kind == Kind::Submission {
            let folders = subfolders(&self.module, cx);
            let label = if self.folder.is_empty() { tr("Modulordner") } else { self.folder.clone() };
            let (e1, e2) = (me.clone(), me.clone());
            grid = grid
                .child(row(
                    tr("Ordner"),
                    Button::new("qa-folder").label(label).dropdown_caret(true).dropdown_menu(move |mut menu, _, _| {
                        let e = e1.clone();
                        menu = menu.item(PopupMenuItem::new(tr("Modulordner")).on_click(move |_, _, cx| e.update(cx, |q, cx| {
                            q.folder.clear();
                            cx.notify();
                        })));
                        for f in &folders {
                            let (e, f) = (e1.clone(), f.clone());
                            menu = menu.item(PopupMenuItem::new(f.clone()).on_click(move |_, _, cx| e.update(cx, |q, cx| {
                                q.folder = f.clone();
                                cx.notify();
                            })));
                        }
                        menu
                    }),
                    &t,
                ))
                .child(row(String::new(), Switch::new("qa-optional").label(tr("Optional")).checked(self.optional).on_click(move |v, _, cx| {
                    let v = *v;
                    e2.update(cx, |q, cx| {
                        q.optional = v;
                        cx.notify();
                    })
                }), &t));
        }
        let hint = match self.kind {
            Kind::Exam => tr("Erscheint mit Countdown in der Übersicht."),
            Kind::Task => tr("Zum Abhaken, ohne Datei."),
            Kind::Submission => tr("Status wird über den Ordner erkannt."),
        };
        let can_add = !self.title.read(cx).value().trim().is_empty() && !self.module.is_empty();
        let (cancel, add) = (me.clone(), me.clone());
        div()
            .w(px(520.))
            .flex()
            .flex_col()
            .gap(px(space::L))
            .p(px(space::XL))
            .child(kinds)
            .child(Input::new(&self.title))
            .child(grid)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .child(div().flex_1().child(caption(hint, &t)))
                    .child(Button::new("qa-cancel").label(tr("Abbrechen")).on_click(move |_, _, cx| cancel.update(cx, |_, cx| cx.emit(Dismissed))))
                    .child(Button::new("qa-add").primary().label(tr("Hinzufügen")).disabled(!can_add).on_click(move |_, _, cx| add.update(cx, |q, cx| q.add(cx)))),
            )
    }
}

/// ⌘P: find any file by name or folder and open it; open tabs come first.
pub struct QuickOpen {
    query: Entity<InputState>,
    index: usize,
    focused: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<Dismissed> for QuickOpen {}

/// A hit only a vault knows: alias, tag, heading, block or a note yet to be created.
struct VaultHit {
    label: String,
    detail: String,
    icon: &'static str,
    open: Option<(PathBuf, String)>,
    create: Option<String>,
}

fn vault_hits(query: &str) -> Vec<VaultHit> {
    let query = query.trim();
    let vault = index::current();
    if query.is_empty() || vault.root.as_os_str().is_empty() {
        return Vec::new();
    }
    let relative = |p: &FilePath| p.strip_prefix(&vault.root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    let mut hits = Vec::new();
    if let Some(tag) = query.strip_prefix('#') {
        let tag = tag.to_lowercase();
        for (name, _) in vault.tag_tree().into_iter().filter(|(n, _)| n.starts_with(&tag)) {
            for path in vault.notes_with_tag(&name).into_iter().take(20) {
                hits.push(VaultHit { label: path.file_stem().unwrap_or_default().to_string_lossy().to_string(), detail: format!("#{name}"), icon: "tag", open: Some((path, String::new())), create: None });
            }
        }
        hits.truncate(40);
        return hits;
    }
    if let Some((name, sub)) = query.split_once(['#', '^']) {
        let block = query.as_bytes()[name.len()] == b'^';
        let Some(path) = vault.resolve(name, None) else { return hits };
        let Some(facts) = vault.notes.get(&path) else { return hits };
        let sub = sub.to_lowercase();
        if block {
            hits.extend(facts.blocks.iter().filter(|(id, _)| id.to_lowercase().contains(&sub)).map(|(id, _)| VaultHit { label: format!("^{id}"), detail: relative(&path), icon: "pilcrow", open: Some((path.clone(), format!("#^{id}"))), create: None }));
        } else {
            hits.extend(facts.headings.iter().filter(|(_, h, _)| h.to_lowercase().contains(&sub)).map(|(_, h, _)| VaultHit { label: h.clone(), detail: relative(&path), icon: "heading", open: Some((path.clone(), format!("#{h}"))), create: None }));
        }
        return hits;
    }
    let lower = query.to_lowercase();
    for (path, facts) in &vault.notes {
        if let Some(alias) = facts.aliases.iter().find(|a| a.to_lowercase().contains(&lower)) {
            hits.push(VaultHit { label: alias.clone(), detail: relative(path), icon: "corner-down-right", open: Some((path.clone(), String::new())), create: None });
        }
    }
    hits.sort_by(|a, b| a.label.cmp(&b.label));
    hits.truncate(20);
    if vault.resolve(query, None).is_none() && !query.contains(['|', '#', '^', '[', ']', '\\', ':', '*', '?', '"', '<', '>']) {
        hits.push(VaultHit { label: trf("Neue Notiz „%@“ anlegen", &[&query.to_string()]), detail: String::new(), icon: "file-plus", open: None, create: Some(query.to_string()) });
    }
    hits
}

actions!(schrumbi_quick, [QuickUp, QuickDown]);

impl QuickOpen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Dateiname, Ordner, „w3“ …")));
        let subs = vec![cx.subscribe(&query, |this, _, e: &InputEvent, cx| match e {
            InputEvent::Change => {
                this.index = 0;
                cx.notify();
            }
            InputEvent::PressEnter { secondary: true, .. } => this.hand_over(cx),
            InputEvent::PressEnter { .. } => this.open(cx),
            _ => {}
        })];
        Self { query, index: 0, focused: false, _subs: subs }
    }

    fn results(&self, cx: &App) -> Vec<(CatalogFile, bool)> {
        let q = self.query.read(cx).value().to_string();
        state::read(cx, |l| {
            let terms: Vec<String> = fold(&q).split(' ').filter(|t| !t.is_empty()).map(String::from).collect();
            let current = l.current_week().map(|w| w.index);
            let open: Vec<&std::path::PathBuf> = l.open_documents.iter().collect();
            let files: Vec<&CatalogFile> = l.catalog.iter().filter(|f| !f.in_project || !terms.is_empty()).collect();
            if terms.is_empty() {
                let tabs = l.open_documents.iter().filter_map(|p| files.iter().find(|f| f.path() == p.as_path()).map(|f| ((*f).clone(), true)));
                let mut recent: Vec<&CatalogFile> = files.iter().filter(|f| !open.contains(&&f.file.path) && FileKind::Documents.matches(f.name())).copied().collect();
                recent.sort_by(|a, b| b.date.cmp(&a.date));
                return tabs.chain(recent.into_iter().map(|f| (f.clone(), false))).take(12).collect();
            }
            let mut scored: Vec<(&CatalogFile, f64)> = files.iter().filter_map(|f| catalog::score(f, &terms, current).map(|s| (*f, s + if open.contains(&&f.file.path) { 3. } else { 0. }))).collect();
            scored.sort_by(|a, b| if a.1 == b.1 { b.0.date.cmp(&a.0.date) } else { b.1.total_cmp(&a.1) });
            scored.into_iter().take(40).map(|(f, _)| (f.clone(), open.contains(&&f.file.path))).collect()
        })
    }

    /// ⌘↩: straight to Obsidian, for the file that would have opened here.
    fn hand_over(&mut self, cx: &mut Context<Self>) {
        let files = self.results(cx);
        let target = files.get(self.index).map(|(f, _)| f.file.path.clone()).or_else(|| {
            let q = self.query.read(cx).value().to_string();
            vault_hits(&q).get(self.index.saturating_sub(files.len())).and_then(|h| h.open.as_ref().map(|(p, _)| p.clone()))
        });
        if let Some(path) = target.filter(|p| super::vault_tools::in_vault(p, cx)) {
            cx.emit(Dismissed);
            super::vault_tools::open_in_obsidian(&path, cx);
        }
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let files = self.results(cx);
        if let Some((f, _)) = files.get(self.index) {
            let p = f.file.path.clone();
            cx.emit(Dismissed);
            act(cx, |l| l.open_file(&p));
            return;
        }
        let q = self.query.read(cx).value().to_string();
        let hits = vault_hits(&q);
        let Some(hit) = hits.get(self.index - files.len()) else { return };
        cx.emit(Dismissed);
        if let Some((path, sub)) = &hit.open {
            let destination = format!("{}{sub}", path.display());
            act(cx, |l| {
                if sub.is_empty() || !l.open_link(&destination, None) {
                    l.open_file(path);
                }
            });
        } else if let Some(name) = &hit.create {
            act(cx, |l| l.create_link_target(name, None));
            state::reindex(cx);
        }
    }
}

impl Render for QuickOpen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            self.focused = true;
            self.query.update(cx, |s, cx| s.focus(window, cx));
        }
        let t = Theme::get(cx);
        let items = self.results(cx);
        let extra = vault_hits(&self.query.read(cx).value());
        self.index = self.index.min((items.len() + extra.len()).saturating_sub(1));
        let me = cx.entity();
        let mut list = div().flex().flex_col().gap(px(space::XXS)).p(px(space::S));
        for (i, (f, is_open)) in items.iter().enumerate() {
            let selected = i == self.index;
            let fg = if selected { white() } else { t.primary };
            let sub = if selected { white().opacity(0.85) } else { t.secondary };
            let module = f.module_id.as_deref().and_then(|id| state::read(cx, |l| l.module_by_id(id).map(|m| (m.code.clone(), m.color.clone()))));
            let e = me.clone();
            list = list.child(
                div()
                    .id(("qo", i))
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .px(px(space::M))
                    .py(px(space::S))
                    .rounded(px(radius::INNER))
                    .cursor_pointer().key_nav()
                    .text_color(fg)
                    .when(selected, |d| d.bg(t.accent))
                    .child(file_icon(f.path(), iconsize::M))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(space::XXS))
                            .child(div().truncate().child(f.name().to_string()))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(space::S))
                                    .text_size(px(font::CAPTION))
                                    .text_color(sub)
                                    .when_some(module, |d, (code, color)| d.child(module_tag(&code, t.palette(&color))))
                                    .when_some(f.week, |d, w| d.child(format!("W{w:02}")))
                                    .when(!f.place.is_empty(), |d| d.child(div().truncate().child(f.place.join(" › ")))),
                            ),
                    )
                    .when(*is_open, |d| d.child(div().text_size(px(font::CAPTION2)).font_weight(FontWeight::SEMIBOLD).text_color(sub).child(tr("Geöffnet"))))
                    .on_click(move |_, _, cx| e.update(cx, |q, cx| {
                        q.index = i;
                        q.open(cx);
                    })),
            );
        }
        for (n, hit) in extra.iter().enumerate() {
            let i = items.len() + n;
            let selected = i == self.index;
            let e = me.clone();
            list = list.child(
                div()
                    .id(("qo", i))
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .px(px(space::M))
                    .py(px(space::S))
                    .rounded(px(radius::INNER))
                    .cursor_pointer().key_nav()
                    .text_color(if selected { white() } else { t.primary })
                    .when(selected, |d| d.bg(t.accent))
                    .child(icon(hit.icon, iconsize::M, if selected { white() } else { t.secondary }))
                    .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().truncate().child(hit.label.clone())).when(!hit.detail.is_empty(), |d| d.child(div().truncate().text_size(px(font::CAPTION)).text_color(if selected { white().opacity(0.85) } else { t.secondary }).child(hit.detail.clone()))))
                    .on_click(move |_, _, cx| e.update(cx, |q, cx| {
                        q.index = i;
                        q.open(cx);
                    })),
            );
        }
        let any = !items.is_empty() || !extra.is_empty();
        let hint = |icon_name: &str, label: String| div().flex().items_center().gap(px(space::XS)).child(icon(icon_name, iconsize::XS, t.secondary)).child(label);
        div()
            .w(px(620.))
            .flex()
            .flex_col()
            .key_context("QuickOpen")
            .on_action(cx.listener(|q, _: &QuickDown, _, cx| {
                q.index += 1;
                cx.notify();
            }))
            .on_action(cx.listener(|q, _: &QuickUp, _, cx| {
                q.index = q.index.saturating_sub(1);
                cx.notify();
            }))
            .child(div().flex().items_center().gap(px(space::S)).p(px(space::L)).child(icon("search", iconsize::M, t.secondary)).child(div().flex_1().child(Input::new(&self.query).appearance(false).large())))
            .child(div().h(px(1.)).bg(t.separator))
            .child(if !any {
                unavailable("search", tr("Keine Treffer"), "", &t).h(px(360.)).into_any_element()
            } else {
                div().id("qo-list").h(px(360.)).overflow_y_scrollbar().child(list).into_any_element()
            })
            .child(div().h(px(1.)).bg(t.separator))
            .child(
                div()
                    .flex()
                    .gap(px(space::M))
                    .px(px(space::L))
                    .py(px(space::S))
                    .text_size(px(font::CAPTION))
                    .text_color(t.secondary)
                    .child(hint("corner-down-left", tr("Öffnen")))
                    .when(state::read(cx, |l| l.is_vault), |d| d.child(hint("external-link", tr("⌘↩ In Obsidian"))))
                    .child(hint("arrow-down-up", tr("Auswählen")))
                    .child(hint("x", tr("Schließen"))),
            )
    }
}

#[derive(Clone, Copy, PartialEq)]
enum NewKind {
    Note,
    Text,
    Folder,
}

/// ⇧⌘N: a new Markdown note, text file or folder in a suggested place.
pub struct NewNote {
    kind: NewKind,
    title: Entity<InputState>,
    places: Vec<crate::core::library::MaterialTarget>,
    preset: Option<std::path::PathBuf>,
    folder: Option<std::path::PathBuf>,
    focused: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<Dismissed> for NewNote {}

impl NewNote {
    pub fn new(module: Option<String>, preset: Option<std::path::PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder(tr("z. B. Mitschrift Interviews")));
        let places = state::read(cx, |l| {
            let prefer = module.clone().or_else(|| preset.as_ref().and_then(|p| l.module_containing(p).map(|m| m.id.clone())));
            l.note_targets(prefer.as_deref())
        });
        let folder = preset.clone().or_else(|| places.first().map(|p| p.path.clone()));
        let subs = vec![cx.subscribe(&title, |this, _, e: &InputEvent, cx| match e {
            InputEvent::PressEnter { .. } => this.create(cx),
            InputEvent::Change => cx.notify(),
            _ => {}
        })];
        Self { kind: NewKind::Note, title, places, preset, folder, focused: false, _subs: subs }
    }

    fn valid(&self, cx: &App) -> bool {
        let t = self.title.read(cx).value().trim().to_string();
        self.folder.is_some() && if self.kind == NewKind::Folder { !t.is_empty() && !t.contains(['/', '\\']) } else { !crate::core::naming::Naming::clean(&t).is_empty() }
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        if !self.valid(cx) {
            return;
        }
        let Some(folder) = self.folder.clone() else { return };
        let title = self.title.read(cx).value().trim().to_string();
        match self.kind {
            NewKind::Folder => {
                if act(cx, |l| l.create_folder(&title, &folder)).is_some() {
                    cx.emit(Dismissed);
                }
            }
            NewKind::Note | NewKind::Text => {
                let ext = if self.kind == NewKind::Note { "md" } else { "txt" };
                if let Some(p) = act(cx, |l| l.create_note(&title, &folder, ext)) {
                    cx.emit(Dismissed);
                    act(cx, |l| l.open_file(&p));
                }
            }
        }
    }
}

impl Render for NewNote {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            self.focused = true;
            self.title.update(cx, |s, cx| s.focus(window, cx));
        }
        let t = Theme::get(cx);
        let me = cx.entity();
        let mut kinds = div().flex().rounded(px(radius::INNER)).bg(t.ink.opacity(0.06)).p(px(space::XXS));
        for (k, label, icon_name) in [(NewKind::Note, tr("Notiz"), "square-pen"), (NewKind::Text, tr("Textdatei"), "file-text"), (NewKind::Folder, tr("Ordner"), "folder-plus")] {
            let active = k == self.kind;
            let e = me.clone();
            kinds = kinds.child(
                div()
                    .id(SharedString::from(format!("nk-{label}")))
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .px(px(space::M))
                    .py(px(space::XS))
                    .rounded(px(radius::INNER))
                    .cursor_pointer().key_nav()
                    .when(active, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm().font_weight(FontWeight::MEDIUM))
                    .child(icon(icon_name, iconsize::XS, if active { t.primary } else { t.secondary }))
                    .child(label)
                    .on_click(move |_, window, cx| e.update(cx, |n, cx| {
                        n.kind = k;
                        let p = if k == NewKind::Folder { tr("z. B. Recherche") } else { tr("z. B. Mitschrift Interviews") };
                        n.title.update(cx, |s, cx| s.set_placeholder(p, window, cx));
                        cx.notify();
                    })),
            );
        }
        let mut chips = div().flex().flex_wrap().gap(px(space::S));
        if let Some(preset) = self.preset.clone().filter(|p| !self.places.iter().take(6).any(|x| &x.path == p)) {
            let selected = self.folder.as_ref() == Some(&preset);
            let e = me.clone();
            let p2 = preset.clone();
            chips = chips.child(Button::new("nn-preset").small().icon(menu_icon(if selected { "check" } else { "folder" })).label(preset.file_name().unwrap_or_default().to_string_lossy().to_string()).when(selected, |b| b.primary()).on_click(move |_, _, cx| e.update(cx, |n, cx| {
                n.folder = Some(p2.clone());
                cx.notify();
            })));
        }
        let (now_root, rel) = state::read(cx, |l| (l.root.clone(), self.folder.as_ref().map(|f| l.relative_path(f))));
        for p in self.places.iter().take(6) {
            let selected = self.folder.as_ref() == Some(&p.path);
            let module_root = now_root.as_ref().map(|r| r.join(&p.module.folder)).unwrap_or_default();
            let inner: Vec<String> = p.path.strip_prefix(&module_root).map(|r| r.iter().map(|c| c.to_string_lossy().to_string()).collect()).unwrap_or_default();
            let label = if inner.is_empty() { p.path.file_name().unwrap_or_default().to_string_lossy().to_string() } else { inner[inner.len().saturating_sub(2)..].join(" › ") };
            let (e, path) = (me.clone(), p.path.clone());
            let ctx = p.context.clone();
            let (fill, hover) = if selected { (t.accent.opacity(0.15), t.accent.opacity(0.2)) } else { (t.ink.opacity(0.05), t.ink.opacity(0.1)) };
            chips = chips.child(
                div()
                    .id(SharedString::from(format!("nn-place-{}", p.path.display())))
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
                    .child(div().size(px(space::S)).rounded_full().bg(t.palette(&p.module.color)))
                    .child(div().flex().flex_col().child(div().text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).child(label)).child(div().flex().gap(px(space::XS)).text_size(px(font::CAPTION2)).text_color(t.secondary).child(p.module.code.clone()).when_some(ctx, |d, c| d.child("·").child(c.day_label()))))
                    .when(selected, |d| d.child(icon("check", iconsize::XS, t.accent)))
                    .on_click(move |_, _, cx| e.update(cx, |n, cx| {
                        n.folder = Some(path.clone());
                        cx.notify();
                    })),
            );
        }
        let e = me.clone();
        chips = chips.child(Button::new("nn-other").small().label(tr("Anderer Ordner…")).on_click(move |_, _, cx| {
            let e = e.clone();
            super::platform::choose_folder(cx, move |p, cx| e.update(cx, |n, cx| {
                n.folder = Some(p);
                cx.notify();
            }))
        }));
        let valid = self.valid(cx);
        let title = self.title.read(cx).value().trim().to_string();
        let name = (valid).then(|| match (self.kind, &self.folder) {
            (NewKind::Folder, _) => format!("{title}/"),
            (k, Some(f)) => state::read(cx, |l| l.note_name(&title, f, if k == NewKind::Note { "md" } else { "txt" })),
            _ => String::new(),
        });
        let (cancel, create) = (me.clone(), me.clone());
        div()
            .w(px(540.))
            .flex()
            .flex_col()
            .gap(px(space::L))
            .p(px(space::XL))
            .child(div().flex().items_center().child(div().flex_1().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(tr("Neu"))).child(kinds))
            .child(Input::new(&self.title))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S))
                    .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(tr("Wohin?")))
                    .child(chips)
                    .when_some(rel, |d, r| d.child(div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.secondary).child(icon("folder", iconsize::XS, t.secondary)).child(div().truncate().child(r)))),
            )
            .when_some(name, |d, n| d.child(div().truncate().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).text_color(t.secondary).child(n)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(space::S))
                    .child(Button::new("nn-cancel").label(tr("Abbrechen")).on_click(move |_, _, cx| cancel.update(cx, |_, cx| cx.emit(Dismissed))))
                    .child(Button::new("nn-create").primary().label(tr("Erstellen")).disabled(!valid).on_click(move |_, _, cx| create.update(cx, |n, cx| n.create(cx)))),
            )
    }
}

/// Names the open tabs, so they can be opened together later.
pub struct SaveLayout {
    name: Entity<InputState>,
    focused: bool,
    _sub: Subscription,
}

impl EventEmitter<Dismissed> for SaveLayout {}

impl SaveLayout {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Name")));
        let sub = cx.subscribe(&name, |this, field, e: &InputEvent, cx| {
            if matches!(e, InputEvent::PressEnter { .. }) {
                let text = field.read(cx).value().trim().to_string();
                this.save(text, cx);
            }
        });
        Self { name, focused: false, _sub: sub }
    }

    fn save(&mut self, text: String, cx: &mut Context<Self>) {
        if !text.is_empty() {
            state::act(cx, |l| l.save_layout(&text));
        }
        cx.emit(Dismissed);
    }
}

impl Render for SaveLayout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !std::mem::replace(&mut self.focused, true) {
            self.name.update(cx, |s, cx| s.focus(window, cx));
        }
        div().flex().flex_col().gap(px(space::S)).p(px(space::L)).child(div().font_weight(FontWeight::SEMIBOLD).child(tr("Arbeitsbereich sichern"))).child(Input::new(&self.name))
    }
}
