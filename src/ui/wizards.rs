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
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::fields::Fields;
use super::inspector::{datetime_picker, picked};
use super::quick::Dismissed;
use super::state::{self, act};
use super::theme::{Theme, font, radius, space};
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
            .child(div().flex().items_center().gap(px(space::S)).child(div().text_color(t.secondary).child(tr("Neuer Eintrag"))).child(div().flex_1().child(DatePicker::new(&self.date))))
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
    draft: StudyConfig,
    fields: Fields,
    start: Entity<DatePickerState>,
    end: Entity<DatePickerState>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<Dismissed> for Setup {}

impl Setup {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = state::read(cx, |l| l.config.clone()).unwrap_or_default();
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
        Self { step: 0, draft, fields: Fields::default(), start, end, _subs: subs }
    }

    fn finish(&mut self, save: bool, cx: &mut Context<Self>) {
        if save {
            let c = self.draft.clone();
            act(cx, |l| l.save_config(c));
        }
        act(cx, |l| l.needs_setup = false);
        cx.emit(Dismissed);
    }
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
        let body: Div = match self.step {
            0 => {
                let name = fields.input("setup-name", &self.draft.name, &tr("z. B. Informatik"), window, cx, move |v, cx| e1.update(cx, |s, cx| {
                    s.draft.name = v;
                    cx.notify();
                }));
                let author = fields.input("setup-author", &self.draft.author, &tr("Nachname-V, z. B. Muster-M"), window, cx, move |v, cx| e2.update(cx, |s, cx| {
                    s.draft.author = v;
                    cx.notify();
                }));
                let sample_module = Module { code: self.draft.modules.first().map_or("ABC".into(), |m| m.code.clone()), ..Default::default() };
                let author_text = if self.draft.author.is_empty() { tr("Nachname-V") } else { self.draft.author.clone() };
                let example = Naming::with_author(&sample_module, self.draft.semesters.first(), &author_text, "", CoauthorStyle::Initial).render(&tr("Titel"), Local::now().date_naive(), "pdf", &[], None, None, None);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::L))
                    .child(div().flex().items_center().gap(px(space::M)).child(super::onboarding::snail(54., &t)).child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(tr("Willkommen"))))
                    .child(div().text_color(t.secondary).child(tr("Drei kurze Schritte, danach erkennt die App deine Abgaben automatisch.")))
                    .child(
                        grouped(&t)
                            .child(form_row(tr("Studiengang"), Input::new(&name), &t))
                            .child(divider(&t))
                            .child(form_row(tr("Kürzel in Dateinamen"), Input::new(&author), &t))
                            .child(divider(&t))
                            .child(form_row(tr("Beispiel"), div().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).text_color(t.secondary).child(example), &t)),
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
            _ => {
                let mut list = grouped(&t);
                for (i, m) in self.draft.modules.clone().into_iter().enumerate() {
                    let (e1, e2, e3) = (me.clone(), me.clone(), me.clone());
                    let id1 = m.id.clone();
                    let id2 = m.id.clone();
                    let id3 = m.id.clone();
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
                            .items_center()
                            .gap(px(space::S))
                            .px(px(space::L))
                            .py(px(space::S))
                            .child(div().size(px(10.)).rounded_full().bg(t.palette(&m.color)))
                            .child(div().w(px(70.)).child(Input::new(&code).small()))
                            .child(div().flex_1().child(Input::new(&name).small()))
                            .child(caption(m.folder.clone(), &t))
                            .child(Button::new(SharedString::from(format!("setup-rm-{}", m.id))).ghost().small().icon(menu_icon("circle-minus")).tooltip(tr("Kein Modul")).on_click(move |_, _, cx| e3.update(cx, |s, cx| {
                                s.draft.modules.retain(|x| x.id != id3);
                                cx.notify();
                            }))),
                    );
                }
                let empty = self.draft.modules.is_empty();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::L))
                    .child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(tr("Module")))
                    .child(div().text_color(t.secondary).child(tr("Aus deinen Ordnern erkannt. Danach: Termine aus Moodle über „Ablage → Kalender importieren“ übernehmen oder wiederkehrende Abgaben unter Einstellungen → Module anlegen.")))
                    .child(if empty { unavailable("folder", tr("Keine Ordner gefunden"), tr("Lege für jedes Modul einen Ordner an oder füge Module später hinzu."), &t).h(px(200.)).into_any_element() } else { div().id("setup-modules").max_h(px(260.)).overflow_y_scrollbar().child(list).into_any_element() })
            }
        };
        fields.prune();
        self.fields = fields;
        let step = self.step;
        let (later, back, next) = (me.clone(), me.clone(), me.clone());
        div()
            .w(px(600.))
            .h(px(520.))
            .flex()
            .flex_col()
            .child(div().flex().gap(px(space::S)).px(px(space::XL)).pt(px(space::XL)).children((0..3).map(|i| div().flex_1().h(px(4.)).rounded_full().bg(if i <= step { t.accent } else { t.secondary.opacity(0.25) }))))
            .child(div().id("setup-body").flex_1().min_h_0().overflow_y_scrollbar().p(px(space::XL)).child(body))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .p(px(space::L))
                    .border_t_1()
                    .border_color(t.separator)
                    .child(Button::new("setup-later").ghost().label(tr("Später einrichten")).on_click(move |_, _, cx| later.update(cx, |s, cx| s.finish(false, cx))))
                    .child(div().flex_1())
                    .when(step > 0, |d| {
                        d.child(Button::new("setup-back").label(tr("Zurück")).on_click(move |_, _, cx| back.update(cx, |s, cx| {
                            s.step -= 1;
                            cx.notify();
                        })))
                    })
                    .child(Button::new("setup-next").primary().label(if step == 2 { tr("Fertig") } else { tr("Weiter") }).on_click(move |_, _, cx| next.update(cx, |s, cx| {
                        if s.step == 2 {
                            s.finish(true, cx);
                        } else {
                            s.step += 1;
                            cx.notify();
                        }
                    }))),
            )
            .rounded(px(radius::CARD))
    }
}
