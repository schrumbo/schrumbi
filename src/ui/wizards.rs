use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use chrono::Local;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::fields::Fields;
use super::inspector::{datetime_picker, picked};
use super::motion::Soft;
use super::quick::Dismissed;
use super::state::{self, act};
use super::theme::{Theme, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::ics::{self, CalendarEvent};
use crate::core::model::{CoauthorStyle, Kind, Module, StudyConfig};
use crate::core::naming::Naming;
use crate::core::{day, schedule};

/// Moves a single generated deadline, e.g. for a holiday or an announced extension.
pub struct Reschedule {
    id: String,
    date: Entity<DatePickerState>,
    _sub: Subscription,
}

impl EventEmitter<Dismissed> for Reschedule {}

impl Reschedule {
    pub fn new(id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let date = datetime_picker(window, cx);
        if let Some(due) = state::read(cx, |l| l.assignment(&id).map(|a| a.due)) {
            date.update(cx, |s, cx| s.set_date_time(day::wall(due), window, cx));
        }
        let sub = cx.subscribe(&date, |_, _, _: &DatePickerEvent, cx| cx.notify());
        Self { id, date, _sub: sub }
    }
}

impl Render for Reschedule {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let Some(a) = state::read(cx, |l| l.assignment(&self.id).cloned()) else { return div().into_any_element() };
        let me = cx.entity();
        let chosen = picked(&self.date, cx);
        let changed = chosen.is_some_and(|d| d != a.due.with_timezone(&chrono::Utc));
        let (reset, cancel, apply, id1, id2) = (me.clone(), me.clone(), me.clone(), self.id.clone(), self.id.clone());
        div()
            .w(px(400.))
            .flex()
            .flex_col()
            .gap(px(space::L))
            .p(px(space::XL))
            .child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::SEMIBOLD).child(tr("Termin verschieben")))
            .child(div().text_color(t.secondary).child(a.title.clone()))
            .child(div().flex().items_center().gap(px(space::S)).child(div().text_color(t.secondary).child(tr("Neuer Termin"))).child(div().flex_1().child(DatePicker::new(&self.date))))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .when(a.is_moved, |d| {
                        d.child(Button::new("rs-reset").ghost().label(tr("Ursprünglicher Termin")).on_click(move |_, _, cx| {
                            act(cx, |l| l.move_occurrence(&id1, None));
                            reset.update(cx, |_, cx| cx.emit(Dismissed));
                        }))
                    })
                    .child(div().flex_1())
                    .child(Button::new("rs-cancel").label(tr("Abbrechen")).on_click(move |_, _, cx| cancel.update(cx, |_, cx| cx.emit(Dismissed))))
                    .child(Button::new("rs-apply").primary().label(tr("Verschieben")).disabled(!changed).on_click(move |_, _, cx| {
                        if let Some(d) = chosen {
                            act(cx, |l| l.move_occurrence(&id2, Some(d)));
                        }
                        apply.update(cx, |_, cx| cx.emit(Dismissed));
                    })),
            )
            .into_any_element()
    }
}

/// Imports deadlines from an .ics file (e.g. the Moodle calendar export).
pub struct ImportCalendar {
    events: Vec<CalendarEvent>,
    selected: HashSet<String>,
    kinds: HashMap<String, Kind>,
    course_modules: HashMap<String, String>,
    include_past: bool,
}

impl EventEmitter<Dismissed> for ImportCalendar {}

impl ImportCalendar {
    pub fn new(path: &Path, cx: &mut Context<Self>) -> Self {
        let bytes = std::fs::read(path).unwrap_or_default();
        let text = String::from_utf8(bytes.clone()).unwrap_or_else(|_| bytes.iter().map(|b| *b as char).collect());
        let events = ics::parse(&text);
        let modules: Vec<Module> = state::read(cx, |l| l.semester_modules().into_iter().cloned().collect());
        let courses: BTreeSet<String> = events.iter().map(|e| e.course.clone()).collect();
        let course_modules = courses.into_iter().map(|c| (c.clone(), ics::module_for(&c, &modules).map(|m| m.id.clone()).unwrap_or_default())).collect();
        Self { selected: events.iter().map(|e| e.uid.clone()).collect(), events, kinds: HashMap::new(), course_modules, include_past: false }
    }

    fn visible(&self) -> Vec<&CalendarEvent> {
        let now = chrono::Utc::now();
        self.events.iter().filter(|e| self.include_past || e.due >= now).collect()
    }

    fn importable(&self) -> Vec<&CalendarEvent> {
        self.visible().into_iter().filter(|e| self.selected.contains(&e.uid) && self.course_modules.get(&e.course).is_some_and(|m| !m.is_empty())).collect()
    }
}

impl Render for ImportCalendar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let me = cx.entity();
        let (modules, known, now) = state::read(cx, |l| {
            (l.semester_modules().iter().map(|m| (m.id.clone(), format!("{} · {}", m.code, m.name))).collect::<Vec<_>>(), l.state.manual.iter().filter_map(|m| m.external_id.clone()).collect::<HashSet<_>>(), l.now)
        });
        let visible = self.visible();
        let courses: BTreeSet<String> = visible.iter().map(|e| e.course.clone()).collect();
        let mut list = div().flex().flex_col().px(px(space::L)).py(px(space::S));
        for course in courses {
            let current = self.course_modules.get(&course).cloned().unwrap_or_default();
            let label = modules.iter().find(|m| m.0 == current).map_or_else(|| tr("Nicht importieren"), |m| m.1.clone());
            let (e, mods, c2) = (me.clone(), modules.clone(), course.clone());
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .mt(px(space::S))
                    .py(px(space::XS))
                    .child(div().flex_1().truncate().font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(if course.is_empty() { tr("Ohne Kurs") } else { course.clone() }))
                    .child(Button::new(SharedString::from(format!("course-{course}"))).small().label(label).dropdown_caret(true).dropdown_menu(move |mut menu, _, _| {
                        let set = |id: String| {
                            let (e, c) = (e.clone(), c2.clone());
                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| e.update(cx, |i, cx| {
                                i.course_modules.insert(c.clone(), id.clone());
                                cx.notify();
                            })
                        };
                        menu = menu.item(PopupMenuItem::new(tr("Nicht importieren")).on_click(set(String::new())));
                        for (id, name) in &mods {
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(set(id.clone())));
                        }
                        menu
                    })),
            );
            for ev in visible.iter().filter(|e| e.course == course) {
                let kind = self.kinds.get(&ev.uid).copied().unwrap_or_else(|| ev.guessed_kind());
                let (e1, e2, uid1, uid2) = (me.clone(), me.clone(), ev.uid.clone(), ev.uid.clone());
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .py(px(space::XS))
                        .child(Checkbox::new(SharedString::from(format!("ev-{}", ev.uid))).checked(self.selected.contains(&ev.uid)).on_click(move |v, _, cx| {
                            let (v, uid) = (*v, uid1.clone());
                            e1.update(cx, |i, cx| {
                                if v { i.selected.insert(uid) } else { i.selected.remove(&uid) };
                                cx.notify();
                            })
                        }))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().truncate().child(ev.title())).child(caption(day::due(ics::local(ev.due), now), &t)))
                        .when(known.contains(&ev.uid), |d| d.child(caption(tr("bereits importiert"), &t)))
                        .child(Button::new(SharedString::from(format!("kind-{}", ev.uid))).small().label(kind.label()).dropdown_caret(true).dropdown_menu(move |mut menu, _, _| {
                            for k in Kind::ALL {
                                let (e, uid) = (e2.clone(), uid2.clone());
                                menu = menu.item(PopupMenuItem::new(k.label()).checked(k == kind).on_click(move |_, _, cx| e.update(cx, |i, cx| {
                                    i.kinds.insert(uid.clone(), k);
                                    cx.notify();
                                })));
                            }
                            menu
                        })),
                );
            }
        }
        let count = self.importable().len();
        let (past, cancel, import) = (me.clone(), me.clone(), me.clone());
        div()
            .w(px(640.))
            .h(px(560.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .p(px(space::XL))
                    .border_b_1()
                    .border_color(t.separator)
                    .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(tr("Kalender importieren")))
                    .child(div().text_size(px(font::CALLOUT)).text_color(t.secondary).child(tr("In Moodle: Kalender → Exportieren → alle Termine → .ics speichern. Bereits importierte Termine werden aktualisiert, nicht doppelt angelegt."))),
            )
            .child(if self.events.is_empty() {
                unavailable("calendar-x", tr("Keine Termine gefunden"), tr("Die Datei enthält keine lesbaren Termine."), &t).flex_1().into_any_element()
            } else {
                div().id("ics-list").flex_1().min_h_0().overflow_y_scrollbar().child(list).into_any_element()
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .p(px(space::L))
                    .border_t_1()
                    .border_color(t.separator)
                    .child(Checkbox::new("include-past").label(tr("Vergangene Termine zeigen")).checked(self.include_past).on_click(move |v, _, cx| {
                        let v = *v;
                        past.update(cx, |i, cx| {
                            i.include_past = v;
                            cx.notify();
                        })
                    }))
                    .child(div().flex_1())
                    .child(Button::new("ics-cancel").label(tr("Abbrechen")).on_click(move |_, _, cx| cancel.update(cx, |_, cx| cx.emit(Dismissed))))
                    .child(Button::new("ics-import").primary().label(trf("%lld importieren", &[&count])).disabled(count == 0).on_click(move |_, _, cx| {
                        let items: Vec<(CalendarEvent, String, Kind)> = import.read(cx).importable().into_iter().map(|e| (e.clone(), import.read(cx).course_modules.get(&e.course).cloned().unwrap_or_default(), import.read(cx).kinds.get(&e.uid).copied().unwrap_or_else(|| e.guessed_kind()))).collect();
                        act(cx, |l| l.import_events(items));
                        import.update(cx, |_, cx| cx.emit(Dismissed));
                    })),
            )
    }
}

/// First-run setup for a freshly chosen folder. Every step can be skipped.
pub struct Setup {
    step: usize,
    folders: Entity<super::folder_picker::FolderPicker>,
    is_vault: bool,
    focused: bool,
    draft: StudyConfig,
    fields: Fields,
    start: Entity<DatePickerState>,
    end: Entity<DatePickerState>,
    /// A weekly deadline for each module that gets one: its title and weekday.
    weekly: std::collections::HashMap<String, (String, u32)>,
    calendar: String,
    new_module: String,
    /// Holds the keyboard on pages without a field, so Enter still goes on.
    focus: FocusHandle,
    focused_step: Option<usize>,
    _subs: Vec<Subscription>,
}

/// The last page: nothing to fill in, only what to do next.
const SETUP_DONE: usize = 4;

impl EventEmitter<Dismissed> for Setup {}

impl Setup {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut draft = state::read(cx, |l| l.config.clone()).unwrap_or_default();
        let root = state::read(cx, |l| l.root.clone()).unwrap_or_default();
        let is_vault = root.join(".obsidian").is_dir();
        let found = crate::core::folders::all_folders(&root);
        let chosen = crate::core::folders::suggested_modules(&found, is_vault);
        let semester = draft.semesters.first().map(|s| s.id.clone()).unwrap_or_default();
        draft.modules.clear();
        let mut taken = Vec::new();
        for (i, folder) in chosen.iter().enumerate() {
            draft.modules.push(crate::core::workspace::module_for_folder(folder, &semester, i, &mut taken));
        }
        let folders = cx.new(|cx| super::folder_picker::FolderPicker::new(&root, chosen, window, cx));
        let picker = |window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| DatePickerState::new(window, cx).date_format(if crate::core::i18n::lang() == crate::core::i18n::Lang::En { "%m/%d/%Y" } else { "%d.%m.%Y" }));
        let (start, end) = (picker(window, cx), picker(window, cx));
        if let Some(s) = draft.semesters.first() {
            if let Some(d) = s.start_date() {
                start.update(cx, |p, cx| p.set_date(d, window, cx));
            }
            if let Some(d) = s.end_date() {
                end.update(cx, |p, cx| p.set_date(d, window, cx));
            }
        }
        let subs = vec![
            cx.subscribe(&folders, Self::folders_changed),
            cx.subscribe(&start, |this, _, e: &DatePickerEvent, cx| {
                let DatePickerEvent::Change(v) = e;
                if let (Some(d), Some(s)) = (v.start(), this.draft.semesters.first_mut()) {
                    s.start = day::format(d.date());
                }
                cx.notify();
            }),
            cx.subscribe(&end, |this, _, e: &DatePickerEvent, cx| {
                let DatePickerEvent::Change(v) = e;
                if let (Some(d), Some(s)) = (v.start(), this.draft.semesters.first_mut()) {
                    s.end = day::format(d.date());
                }
                cx.notify();
            }),
        ];
        Self { step: 0, folders, is_vault, focused: false, draft, fields: Fields::default(), start, end, weekly: Default::default(), calendar: String::new(), new_module: String::new(), focus: cx.focus_handle(), focused_step: None, _subs: subs }
    }

    fn folders_changed(&mut self, picker: Entity<super::folder_picker::FolderPicker>, _: &super::folder_picker::Changed, cx: &mut Context<Self>) {
        let chosen = picker.read(cx).selected.clone();
        let semester = self.draft.semesters.first().map(|s| s.id.clone()).unwrap_or_default();
        self.draft.modules.retain(|m| chosen.contains(&m.folder));
        let mut taken: Vec<String> = self.draft.modules.iter().map(|m| m.code.clone()).collect();
        for folder in &chosen {
            if !self.draft.modules.iter().any(|m| m.folder == *folder) {
                let position = self.draft.modules.len();
                self.draft.modules.push(crate::core::workspace::module_for_folder(folder, &semester, position, &mut taken));
            }
        }
        cx.notify();
    }

    /// Makes a folder for a module that has none yet and takes it into the list.
    fn add_module(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = crate::core::text::safe_file_stem(&self.new_module);
        let root = state::read(cx, |l| l.root.clone()).unwrap_or_default();
        if name.is_empty() || std::fs::create_dir_all(root.join(&name)).is_err() {
            return;
        }
        let mut chosen: Vec<String> = self.folders.read(cx).selected.iter().cloned().collect();
        if !chosen.contains(&name) {
            chosen.push(name);
        }
        let folders = cx.new(|cx| super::folder_picker::FolderPicker::new(&root, chosen, window, cx));
        self._subs.push(cx.subscribe(&folders, Self::folders_changed));
        self.folders = folders.clone();
        self.new_module.clear();
        window.focus(&self.focus, cx);
        self.folders_changed(folders, &super::folder_picker::Changed, cx);
    }

    /// What keeps this step from going on, said in a sentence.
    fn problem(&self) -> Option<String> {
        if self.step == 1 {
            let semester = self.draft.semesters.first()?;
            return match (semester.start_date(), semester.end_date()) {
                (Some(a), Some(b)) if a < b => None,
                (Some(_), Some(_)) => Some(tr("Das Ende muss nach dem Beginn liegen.")),
                _ => Some(tr("Wähle Beginn und Ende der Vorlesungszeit.")),
            };
        }
        if self.step == 3 {
            let address = self.calendar.trim();
            if !(address.is_empty() || address.starts_with("https://") || address.starts_with("http://") || address.starts_with("webcal://")) {
                return Some(tr("Die Adresse muss mit https:// oder webcal:// beginnen."));
            }
        }
        if self.step == 2 {
            let mut codes: Vec<String> = self.draft.modules.iter().map(|m| m.code.trim().to_lowercase()).collect();
            if codes.iter().any(String::is_empty) {
                return Some(tr("Jedes Modul braucht ein Kürzel."));
            }
            codes.sort();
            if codes.windows(2).any(|w| w[0] == w[1]) {
                return Some(tr("Zwei Module haben dasselbe Kürzel."));
            }
        }
        None
    }

    fn go_on(&mut self, cx: &mut Context<Self>) {
        if self.problem().is_some() {
            return;
        }
        if self.step == SETUP_DONE {
            self.finish(true, cx);
        } else {
            self.step += 1;
            cx.notify();
        }
    }

    fn finish(&mut self, save: bool, cx: &mut Context<Self>) {
        if save {
            for module in &mut self.draft.modules {
                if let Some((title, weekday)) = self.weekly.get(&module.id).filter(|(title, _)| !title.trim().is_empty()) {
                    let short: String = title.trim().chars().take(2).collect::<String>().to_uppercase();
                    let today = Local::now().date_naive();
                    let start_week = self.draft.semesters.first().and_then(|semester| schedule::lecture_weeks(semester).into_iter().find(|w| w.monday + chrono::Days::new(*weekday as u64 - 1) >= today).map(|w| w.index));
                    module.rules.push(crate::core::model::Rule { title: title.trim().to_string(), short, weekday: *weekday, time: "23:59".into(), upload: true, start_week, ..Default::default() });
                }
            }
            let c = self.draft.clone();
            act(cx, |l| l.save_config(c));
        }
        act(cx, |l| l.needs_setup = false);
        cx.emit(Dismissed);
        let address = self.calendar.trim().to_string();
        if save && !address.is_empty() {
            let root = state::read(cx, |l| l.root.clone()).unwrap_or_default().to_string_lossy().to_string();
            crate::core::prefs::update(|p| {
                p.calendar_sources.insert(root, address);
            });
            state::refresh_calendar(true, cx);
        }
    }
}

fn order_choice(id: &'static str, icon_name: &'static str, title: String, detail: String, selected: bool, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, t: &Theme) -> Stateful<Div> {
    let (fill, hover) = if selected { (t.accent.opacity(0.1), t.accent.opacity(0.16)) } else { (t.ink.opacity(0.03), t.ink.opacity(0.07)) };
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .flex()
        .items_start()
        .gap(px(space::M))
        .p(px(space::M))
        .rounded(px(radius::CARD))
        .border(px(2.))
        .border_color(if selected { t.accent } else { t.separator })
        .bg(fill)
        .soft(hover, radius::CARD - 2.)
        .cursor_pointer()
        .key_nav()
        .on_click(on_click)
        .child(icon(icon_name, iconsize::M, if selected { t.accent_text } else { t.secondary }))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().font_weight(FontWeight::SEMIBOLD).child(title)).child(caption(detail, t)))
        .child(div().flex_none().size(px(iconsize::S)).when(selected, |d| d.child(icon("check", iconsize::S, t.accent_text))))
}

fn form_row(label: String, content: impl IntoElement, t: &Theme) -> Div {
    div().flex().items_center().gap(px(space::M)).px(px(space::L)).py(px(space::S)).child(div().w(px(170.)).text_color(t.secondary).child(label)).child(div().flex_1().child(content))
}

impl Render for Setup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let me = cx.entity();
        let mut fields = std::mem::take(&mut self.fields);
        let (e1, e2) = (me.clone(), me.clone());
        if self.step != 0 && self.focused_step != Some(self.step) {
            let focus = self.focus.clone();
            window.on_next_frame(move |window, cx| window.focus(&focus, cx));
        }
        self.focused_step = Some(self.step);
        let body: Div = {
            match self.step {
            0 => {
                let first_field = std::mem::replace(&mut self.focused, true);
                let name = fields.input("setup-name", &self.draft.name, &tr("z. B. Informatik"), window, cx, move |v, cx| e1.update(cx, |s, cx| {
                    s.draft.name = v;
                    cx.notify();
                }));
                if !first_field {
                    name.update(cx, |n, cx| n.focus(window, cx));
                }
                let author = fields.input("setup-author", &self.draft.author, &tr("Nachname-V, z. B. Muster-M"), window, cx, move |v, cx| e2.update(cx, |s, cx| {
                    s.draft.author = v;
                    cx.notify();
                }));
                let sample_module = Module { code: self.draft.modules.first().map_or("ABC".into(), |m| m.code.clone()), ..Default::default() };
                let author_text = if self.draft.author.is_empty() { tr("Nachname-V") } else { self.draft.author.clone() };
                let folders = self.draft.create_folders;
                let naming = self.draft.naming_convention();
                let (help, own, named) = (me.clone(), me.clone(), me.clone());
                let help = move |_: &ClickEvent, _: &mut Window, cx: &mut App| help.update(cx, |s, cx| {
                    s.draft.create_folders = true;
                    cx.notify();
                });
                let own = move |_: &ClickEvent, _: &mut Window, cx: &mut App| own.update(cx, |s, cx| {
                    s.draft.create_folders = false;
                    cx.notify();
                });
                let vault = state::read(cx, |l| l.is_vault);
                let naming_switch = Switch::new("setup-naming").checked(naming).on_click(move |v, _, cx| {
                    let v = *v;
                    named.update(cx, |s, cx| {
                        s.draft.set_naming_convention(v);
                        cx.notify();
                    });
                });
                let example = Naming::with_author(&sample_module, self.draft.semesters.first(), &author_text, "", CoauthorStyle::Initial).render(&tr("Titel"), Local::now().date_naive(), "pdf", &[], None, None, None);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::L))
                    .child(div().flex().items_center().gap(px(space::M)).child(super::onboarding::snail(54., &t)).child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(tr("Willkommen"))))
                    .child(div().text_color(t.secondary).child(if self.is_vault { tr("Deine Notizen bleiben normale Dateien. Du kannst sie hier schreiben und in Obsidian, auch gleichzeitig.") } else { tr("Vier kurze Schritte, danach kennt die App deine Module und Abgaben.") }))
                    .child(grouped(&t).child(form_row(tr("Studiengang"), Input::new(&name), &t)))
                    .when(!vault, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(space::S))
                                .child(section_title(tr("Namenskonvention")))
                                .child(
                                    grouped(&t)
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(space::M))
                                                .px(px(space::L))
                                                .py(px(space::S))
                                                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().font_weight(FontWeight::SEMIBOLD).child(tr("Nach einem Muster benennen"))).child(caption(super::settings::naming_detail(naming), &t)))
                                                .child(naming_switch),
                                        )
                                        .when(naming, |d| d.child(divider(&t)).child(form_row(tr("Kürzel in Dateinamen"), Input::new(&author), &t)).child(divider(&t)).child(form_row(tr("Beispiel"), div().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).text_color(t.secondary).child(example), &t))),
                                ),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::S))
                            .child(section_title(tr("Ordner")))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(space::M))
                                    .child(order_choice("setup-order-help", "wand-sparkles", tr("Schrumbi hilft beim Ordnen"), tr("Legt in jedem Modul einen Abgabeordner an, sobald du etwas ablegst, und benennt nach Muster."), folders, help, &t))
                                    .child(order_choice("setup-order-own", "folder-tree", tr("Meine eigene Struktur"), tr("Schrumbi liest nur. Dateien landen dort, wo du sie hinlegst."), !folders, own, &t)),
                            )
                            .child(caption(tr("Lässt sich jederzeit in den Einstellungen ändern, auch einzeln."), &t)),
                    )
            }
            1 => {
                let mut col = div().flex().flex_col().gap(px(space::L)).child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(tr("Semester"))).child(div().text_color(t.secondary).child(tr("Vorlesungszeitraum und Pausen. Pausen zählen nicht als Vorlesungswochen.")));
                if let Some(s) = self.draft.semesters.first().cloned() {
                    let e = me.clone();
                    let name = fields.input("setup-semester", &s.name, "", window, cx, move |v, cx| e.update(cx, |st, cx| {
                        if let Some(s) = st.draft.semesters.first_mut() {
                            s.name = v;
                        }
                        cx.notify();
                    }));
                    let breaks = if s.breaks.is_empty() { tr("keine, in den Einstellungen ergänzen") } else { s.breaks.iter().map(|b| b.name.clone()).collect::<Vec<_>>().join(", ") };
                    col = col.child(
                        grouped(&t)
                            .child(form_row(tr("Name"), Input::new(&name), &t))
                            .child(divider(&t))
                            .child(form_row(tr("Vorlesungsbeginn"), DatePicker::new(&self.start), &t))
                            .child(divider(&t))
                            .child(form_row(tr("Vorlesungsende"), DatePicker::new(&self.end), &t))
                            .child(divider(&t))
                            .child(form_row(tr("Pausen"), div().text_color(t.secondary).child(breaks), &t))
                            .child(divider(&t))
                            .child(form_row(tr("Vorlesungswochen"), div().child(schedule::lecture_weeks(&s).len().to_string()), &t)),
                    );
                }
                col
            }
            3 => {
                let typed = me.clone();
                let calendar = fields.input("setup-calendar", &self.calendar, "https://…", window, cx, move |v, cx| typed.update(cx, |s, cx| {
                    s.calendar = v;
                    cx.notify();
                }));
                let mut rows = div().flex().flex_col();
                for (i, m) in self.draft.modules.clone().into_iter().enumerate() {
                    let (typed, picked, id, id2) = (me.clone(), me.clone(), m.id.clone(), m.id.clone());
                    let current = self.weekly.get(&m.id).cloned().unwrap_or_else(|| (String::new(), 5));
                    let title = fields.input(&format!("setup-weekly-{}", m.id), &current.0, &tr("z. B. Übungsblatt"), window, cx, move |v, cx| typed.update(cx, |s, cx| {
                        s.weekly.entry(id.clone()).or_insert_with(|| (String::new(), 5)).0 = v;
                        cx.notify();
                    }));
                    if i > 0 {
                        rows = rows.child(divider(&t));
                    }
                    rows = rows.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S))
                            .px(px(space::M))
                            .py(px(space::S))
                            .child(div().flex_none().w(px(44.)).font_weight(FontWeight::SEMIBOLD).text_color(t.palette(&m.color)).child(m.code.clone()))
                            .child(div().flex_1().min_w_0().child(Input::new(&title).small()))
                            .child(Button::new(SharedString::from(format!("setup-day-{}", m.id))).small().ghost().label(day::weekday_names().get(current.1 as usize - 1).copied().unwrap_or_default().to_string()).dropdown_caret(true).dropdown_menu(move |mut menu, _, _| {
                                for (n, name) in day::weekday_names().iter().enumerate() {
                                    let (p, id) = (picked.clone(), id2.clone());
                                    menu = menu.item(PopupMenuItem::new(name.to_string()).checked(current.1 as usize == n + 1).on_click(move |_, _, cx| p.update(cx, |s, cx| {
                                        s.weekly.entry(id.clone()).or_insert_with(|| (String::new(), 5)).1 = n as u32 + 1;
                                        cx.notify();
                                    })));
                                }
                                menu
                            })),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::L))
                    .child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(tr("Abgaben")))
                    .child(div().text_color(t.secondary).child(tr("Beides ist freiwillig und geht auch später in den Einstellungen.")))
                    .child(div().flex().flex_col().gap(px(space::S)).child(section_title(tr("Aus Moodle"))).child(caption(tr("Kalender → Exportieren → Kalender-URL abrufen. Neue Abgaben kommen dann von selbst."), &t)).child(Input::new(&calendar)))
                    .child(div().flex().flex_col().gap(px(space::S)).child(section_title(tr("Jede Woche"))).child(caption(tr("Hat ein Modul jede Woche eine Abgabe? Trage ihren Namen ein, sie ist dann jede Vorlesungswoche um 23:59 fällig."), &t)).when(!self.draft.modules.is_empty(), |d| d.child(grouped(&t).child(rows))).when(self.draft.modules.is_empty(), |d| d.child(caption(tr("Noch kein Modul gewählt."), &t))))
            }
            SETUP_DONE => {
                let command = if cfg!(target_os = "macos") { "⌘" } else { "Strg+" };
                let weekly = self.weekly.values().filter(|(title, _)| !title.trim().is_empty()).count();
                let modules = self.draft.modules.len();
                let tip = |name: &'static str, title: String, detail: String| div().flex().items_start().gap(px(space::M)).child(div().size(px(36.)).flex_none().rounded(px(radius::INNER)).bg(t.accent.opacity(0.14)).flex().items_center().justify_center().child(icon(name, iconsize::M, t.accent_text))).child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().font_weight(FontWeight::SEMIBOLD).child(title)).child(caption(detail, &t)));
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::L))
                    .child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(tr("Alles bereit")))
                    .child(div().text_color(t.secondary).child(trf("%@ Module, %@ wöchentliche Abgaben. Das meiste geht von hier aus so:", &[&modules.to_string(), &weekly.to_string()])))
                    .child(tip("inbox", tr("Dateien ablegen"), if self.draft.rename_on_filing { tr("Ziehe Dateien ins Fenster. Die App schlägt Modul, Abgabe und Namen vor, Enter legt ab.") } else { tr("Ziehe Dateien ins Fenster. Die App schlägt Modul und Abgabe vor, Enter legt ab.") }))
                    .child(tip("square-pen", tr("Notizen schreiben"), trf("%@N legt eine an, ihr Dateiname folgt der ersten Überschrift.", &[&command])))
                    .child(tip("layers", tr("Karten lernen"), tr("Schreibe „Frage :: Antwort“ in eine Notiz, die Karte erscheint im Tab Karten.")))
                    .child(tip("search", tr("Alles finden"), trf("%@K zeigt jeden Befehl, / sucht in Dateien und Notizen.", &[&command])))
            }
            _ => {
                let mut list = div().flex().flex_col();
                for (i, m) in self.draft.modules.clone().into_iter().enumerate() {
                    let (e1, e2, e3) = (me.clone(), me.clone(), me.clone());
                    let (id1, id2, folder) = (m.id.clone(), m.id.clone(), m.folder.clone());
                    let code = fields.input(&format!("setup-code-{}", m.id), &m.code, &tr("Kürzel"), window, cx, move |v, cx| e1.update(cx, |s, cx| {
                        if let Some(m) = s.draft.module_mut(&id1) {
                            m.code = v;
                        }
                        cx.notify();
                    }));
                    let name = fields.input(&format!("setup-mname-{}", m.id), &m.name, &tr("Name"), window, cx, move |v, cx| e2.update(cx, |s, cx| {
                        if let Some(m) = s.draft.module_mut(&id2) {
                            m.name = v;
                        }
                        cx.notify();
                    }));
                    if i > 0 {
                        list = list.child(divider(&t));
                    }
                    list = list.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::XS))
                            .px(px(space::M))
                            .py(px(space::S))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S))
                                    .child(div().flex_none().size(px(space::S + space::XXS)).rounded_full().bg(t.palette(&m.color)))
                                    .child(div().flex_none().w(px(56.)).child(Input::new(&code).small()))
                                    .child(div().flex_1().min_w_0().child(Input::new(&name).small()))
                                    .child(Button::new(SharedString::from(format!("setup-rm-{}", m.id))).ghost().small().icon(menu_icon("circle-minus")).tooltip(tr("Kein Modul")).on_click(move |_, _, cx| e3.update(cx, |s, cx| {
                                        s.folders.update(cx, |p, cx| {
                                            p.selected.remove(&folder);
                                            cx.emit(super::folder_picker::Changed);
                                            cx.notify();
                                        });
                                    }))),
                            )
                            .child(caption(m.folder.replace('/', " › "), &t).truncate()),
                    );
                }
                let empty = self.draft.modules.is_empty();
                let picked = self.draft.modules.len();
                let typed = me.clone();
                let add_field = fields.input("setup-new-module", &self.new_module, &tr("Modul, das noch keinen Ordner hat"), window, cx, move |v, cx| typed.update(cx, |s, cx| {
                    s.new_module = v;
                    cx.notify();
                }));
                let adder = me.clone();
                let can_add = !self.new_module.trim().is_empty();
                let creates = self.draft.create_folders;
                div()
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap(px(space::M))
                    .child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(tr("Module")))
                    .child(div().text_color(t.secondary).child(tr("Hake die Ordner an, die Module sind, egal wie tief sie liegen. Alles andere bleibt einfach Notizen.")))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .gap(px(space::M))
                            .child(div().flex_1().min_w_0().h_full().flex().flex_col().gap(px(space::S)).child(div().flex_1().min_h_0().rounded(px(radius::CARD)).border_1().border_color(t.separator).overflow_hidden().child(self.folders.clone())).when(creates, |d| d.child(div().flex_none().flex().gap(px(space::S)).child(div().flex_1().min_w_0().child(Input::new(&add_field).small())).child(Button::new("setup-add").small().icon(menu_icon("plus")).label(tr("Anlegen")).disabled(!can_add).on_click(move |_, window, cx| adder.update(cx, |s, cx| s.add_module(window, cx)))))))
                            .child(
                                div()
                                    .w(px(320.))
                                    .flex_none()
                                    .h_full()
                                    .flex()
                                    .flex_col()
                                    .rounded(px(radius::CARD))
                                    .border_1()
                                    .border_color(t.separator)
                                    .overflow_hidden()
                                    .child(div().flex_none().px(px(space::M)).py(px(space::S)).border_b_1().border_color(t.separator).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(if picked == 1 { tr("1 Modul") } else { trf("%lld Module", &[&picked]) }))
                                    .child(if empty {
                                        unavailable("library", tr("Noch kein Modul"), if creates { tr("Hake links einen Ordner an oder lege unten einen an. Geht auch später: Rechtsklick auf einen Ordner in den Notizen.") } else { tr("Hake links einen Ordner an. Geht auch später: Rechtsklick auf einen Ordner in den Notizen.") }, &t).into_any_element()
                                    } else {
                                        div().id("setup-modules").flex_1().min_h_0().overflow_y_scrollbar().child(list).into_any_element()
                                    }),
                            ),
                    )
            }
            }
        };
        fields.prune();
        self.fields = fields;
        let step = self.step;
        let problem = self.problem();
        let blocked = problem.is_some();
        let (later, back, next) = (me.clone(), me.clone(), me.clone());
        div()
            .w_full()
            .h(px(600.))
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "enter" && s.focus.is_focused(window) {
                    s.go_on(cx);
                }
            }))
            .capture_action(cx.listener(|s, a: &gpui_kit::component::input::Enter, window, cx| {
                if !a.secondary {
                    if s.step == 2 && s.draft.create_folders && !s.new_module.trim().is_empty() {
                        s.add_module(window, cx);
                    } else {
                        s.go_on(cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .child(div().flex().gap(px(space::S)).px(px(space::XL)).pt(px(space::XL)).children((0..4).map(|i| div().flex_1().h(px(4.)).rounded_full().bg(if i <= step { t.accent } else { t.secondary.opacity(0.25) }))))
            .child(if step == 2 { div().id("setup-body").flex_1().min_h_0().p(px(space::XL)).child(body).into_any_element() } else { div().id("setup-body").flex_1().min_h_0().overflow_y_scrollbar().p(px(space::XL)).child(body).into_any_element() })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .p(px(space::L))
                    .border_t_1()
                    .border_color(t.separator)
                    .when(step < SETUP_DONE, |d| d.child(Button::new("setup-later").ghost().label(tr("Später einrichten")).on_click(move |_, _, cx| later.update(cx, |s, cx| s.finish(false, cx)))))
                    .child(div().flex_1())
                    .children(problem.map(|text| div().text_size(px(font::CALLOUT)).text_color(t.orange).child(text)))
                    .when(step > 0, |d| {
                        d.child(Button::new("setup-back").label(tr("Zurück")).on_click(move |_, _, cx| back.update(cx, |s, cx| {
                            s.step -= 1;
                            cx.notify();
                        })))
                    })
                    .child(Button::new("setup-next").primary().disabled(blocked).label(if step == SETUP_DONE { tr("Los geht's") } else { tr("Weiter") }).on_click(move |_, _, cx| next.update(cx, |s, cx| s.go_on(cx)))),
            )
            .rounded(px(radius::CARD))
    }
}
