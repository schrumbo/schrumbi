use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::date_picker::DatePicker;
use gpui_kit::component::input::{Input, InputEvent};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::fields::Fields;
use super::motion::{self, Soft};
use super::state::{self, act, edit_config};
use super::theme::{Theme, font, hints, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::library::{Library, Route};
use crate::core::model::{CoauthorStyle, Module, Semester, StudyConfig};
use crate::core::naming::Naming;
use crate::core::prefs::{self, NoteEditor, SetupPrefs};
use crate::core::setup::{self, Facts, Item};
use crate::core::{day, schedule, workspace};

/// The list at the top of the overview that ticks itself as the workspace gets set up.
pub struct SetupCard {
    fields: Fields,
    /// The item whose inline form is unfolded.
    open: Option<Item>,
    /// The list had open items this session, so finishing them deserves a word.
    had_open: bool,
    celebrating: Option<Task<()>>,
    calendar: Entity<CalendarAsk>,
    /// The form just unfolded; its first field takes the keyboard on the next frame.
    focus_field: bool,
    /// The steps after the first one are unfolded.
    more: bool,
    _subs: Vec<Subscription>,
}

/// Asks for the Moodle calendar address in place: a button first, then the field and Abrufen.
pub struct CalendarAsk {
    fields: Fields,
    /// The address as typed; it is saved by Abrufen or Enter, not by each key.
    draft: Option<String>,
    enter: Option<Subscription>,
    /// The field shows; a button-first ask unfolds on its button.
    expanded: bool,
    /// Folds back to its button once the address is saved.
    folds: bool,
    focus: bool,
}

/// The calendar address was saved and is being fetched.
pub struct Fetched;

impl EventEmitter<Fetched> for CalendarAsk {}

impl CalendarAsk {
    /// `expanded` shows the field at once; otherwise a button unfolds it.
    pub fn new(expanded: bool) -> Self {
        Self { fields: Fields::default(), draft: None, enter: None, expanded, folds: !expanded, focus: false }
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.draft.clone().map(|v| v.trim().to_string()).filter(|v| v.starts_with("http")) else { return };
        self.draft = None;
        self.expanded = !self.folds;
        let root = root(cx).to_string_lossy().to_string();
        prefs::update(|p| {
            p.calendar_sources.insert(root, url);
        });
        state::refresh_calendar(true, cx);
        cx.emit(Fetched);
        cx.refresh_windows();
    }
}

impl Render for CalendarAsk {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        if !self.expanded && state::calendar_source(cx).is_some() {
            return div().flex().items_center().gap(px(space::S)).child(icon("circle-check", iconsize::S, t.green)).child(caption(tr("Verbunden, wird jede Stunde abgerufen"), &t)).child(link_button("calendar-ask-change", tr("Ändern"), &t).text_size(px(font::CAPTION)).on_click(cx.listener(|this, _, _, cx| {
                this.expanded = true;
                this.focus = true;
                cx.notify();
            })));
        }
        if !self.expanded {
            return div().flex().child(Button::new("calendar-ask-open").small().label(tr("Moodle-Kalender verbinden")).on_click(cx.listener(|this, _, _, cx| {
                this.expanded = true;
                this.focus = true;
                cx.notify();
            })));
        }
        let value = self.draft.clone().or_else(|| state::calendar_source(cx)).unwrap_or_default();
        let me = cx.entity().downgrade();
        let field = self.fields.input("calendar-ask", &value, "https://…", window, cx, move |v, cx| {
            let _ = me.update(cx, |s, cx| {
                s.draft = Some(v);
                cx.notify();
            });
        });
        if self.enter.is_none() {
            let me = cx.entity().downgrade();
            let app: &mut App = cx;
            self.enter = Some(app.subscribe(&field, move |_, e: &InputEvent, cx| {
                if matches!(e, InputEvent::PressEnter { .. }) {
                    let _ = me.update(cx, |s, cx| s.fetch(cx));
                }
            }));
        }
        if std::mem::take(&mut self.focus) {
            let f = field.clone();
            cx.defer_in(window, move |_, window, cx| f.update(cx, |s, cx| s.focus(window, cx)));
        }
        let ready = value.trim().starts_with("http");
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .child(caption(tr("In Moodle: Kalender → Exportieren → Kalender-URL abrufen, dann die Adresse hier einfügen."), &t))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .child(div().flex_1().min_w_0().max_w(px(hints::URL_WIDTH)).child(Input::new(&field).small()))
                    .child(Button::new("calendar-ask-fetch").primary().small().label(tr("Abrufen")).disabled(!ready).on_click(cx.listener(|this, _, _, cx| this.fetch(cx)))),
            )
    }
}

/// What the rows read from the library, copied so the forms can change it while drawing.
struct Snap {
    config: Option<StudyConfig>,
    semester: Option<Semester>,
    modules: Vec<Module>,
}

/// What the facts say for this workspace right now.
pub fn facts(l: &Library, cx: &App) -> Facts {
    let config = l.config.as_ref();
    let naming_on = config.is_some_and(|c| c.naming_convention());
    let needs_author = config.is_some_and(|c| c.modules.is_empty() || c.modules.iter().any(|m| m.naming.contains("{author}")));
    Facts {
        modules: l.semester_modules().len(),
        semester_set: l.semester().is_some_and(|s| s.start_date().is_some() && s.end_date().is_some() && !workspace::is_guessed(s)),
        calendar: state::calendar_source(cx).is_some(),
        naming_on,
        author_ok: !needs_author || config.is_some_and(|c| !c.author.trim().is_empty()),
        editor_chosen: prefs::with(|p| p.note_editor_chosen || p.note_editor == NoteEditor::Live),
        cards: crate::core::cards::all_cards().len(),
        sync: super::sync::enabled_here(cx),
    }
}

fn root(cx: &App) -> PathBuf {
    state::read(cx, |l| l.root.clone()).unwrap_or_default()
}

fn marks(cx: &App) -> SetupPrefs {
    prefs::setup_of(&root(cx))
}

/// Finished and closed as the workspace remembers them for every device.
fn shared(cx: &App) -> (bool, bool) {
    state::read(cx, |l| (l.state.setup_completed, l.state.setup_dismissed))
}

/// Records the list as finished or closed, or brings it back, on this device and in the workspace.
fn settle(cx: &mut App, completed: bool, dismissed: bool) {
    prefs::update_setup(&root(cx), |s| {
        s.completed = completed;
        s.dismissed = dismissed;
    });
    act(cx, |l| l.mark_setup(completed, dismissed));
    cx.refresh_windows();
}

/// The item of this device still open after the list was finished on another device; offered one at a time.
fn device_offer(facts: &Facts, marks: &SetupPrefs) -> Option<Item> {
    [Item::Calendar, Item::Editor, Item::Sync].into_iter().find(|i| !setup::settled(*i, facts, &marks.skipped, &marks.confirmed))
}

/// Ticks what the guided setup just settled: the semester dates were shown and the naming was decided there.
pub fn wizard_done(cx: &mut App) {
    remember(cx, |s| {
        s.confirmed.insert(Item::Semester.key().to_string());
        s.confirmed.insert(Item::Naming.key().to_string());
    });
}

fn remember(cx: &mut App, f: impl FnOnce(&mut SetupPrefs)) {
    let root = root(cx);
    prefs::update_setup(&root, f);
    cx.refresh_windows();
}

fn skip(item: Item, cx: &mut App) {
    remember(cx, |s| {
        s.skipped.insert(item.key().to_string());
    });
}

fn confirm(item: Item, cx: &mut App) {
    remember(cx, |s| {
        s.confirmed.insert(item.key().to_string());
    });
}

fn unskip(item: Item, cx: &mut App) {
    remember(cx, |s| {
        s.skipped.remove(item.key());
        s.confirmed.remove(item.key());
    });
}

/// Brings a dismissed or finished list back; from the settings.
pub fn restore(cx: &mut App) {
    settle(cx, false, false);
}

/// Whether the list is dismissed or finished for this workspace, here or on another device.
pub fn hidden(cx: &App) -> bool {
    let m = marks(cx);
    let (completed, dismissed) = shared(cx);
    m.dismissed || m.completed || completed || dismissed
}

fn title(item: Item) -> String {
    match item {
        Item::Modules => tr("Module"),
        Item::Semester => tr("Vorlesungszeit"),
        Item::Calendar => tr("Moodle-Kalender"),
        Item::Naming => tr("Dateinamen"),
        Item::Editor => tr("Notizen schreiben"),
        Item::Cards => tr("Karten"),
        Item::Sync => tr("Sichern & Abgleichen"),
    }
}

fn icon_of(item: Item) -> &'static str {
    match item {
        Item::Modules => "library",
        Item::Semester => "calendar-range",
        Item::Calendar => "calendar-plus",
        Item::Naming => "wand-sparkles",
        Item::Editor => "square-pen",
        Item::Cards => "layers",
        Item::Sync => "cloud",
    }
}

/// What an open item asks for, in one line.
fn ask(item: Item) -> String {
    match item {
        Item::Modules => tr("Welche Ordner sind deine Module? Sie bekommen Abgaben, Notizen und ein Kürzel."),
        Item::Semester => tr("Die Vorlesungszeit ist geschätzt. Stimmt sie, zählt die App die Wochen richtig."),
        Item::Calendar => tr("Mit der Kalenderadresse aus Moodle kommen Abgaben von selbst in die Übersicht."),
        Item::Naming => tr("Abgelegte Dateien nach Muster benennen, z. B. ANA_26_Blatt03_Muster-M_261110.pdf."),
        Item::Editor => tr("Formatiert sehen, während du tippst, oder den Text mit Zeilennummern?"),
        Item::Cards => tr("Lernkarten aus Anki übernehmen oder als „Frage :: Antwort“ in Notizen schreiben."),
        Item::Sync => tr("Notizen und Dateien sichern und auf mehreren Geräten abgleichen."),
    }
}

/// What a settled item shows, from the facts.
fn summary(item: Item, f: &Facts, snap: &Snap, marks: &SetupPrefs) -> String {
    if marks.skipped.contains(item.key()) && !setup::done(item, f) {
        return tr("Übersprungen");
    }
    match item {
        Item::Modules => if f.modules == 1 { tr("1 Modul") } else { trf("%lld Module", &[&f.modules]) },
        Item::Semester => snap.semester.as_ref().map(|s| format!("{} · {} – {}", s.name, s.start_date().map(day::short_date).unwrap_or_default(), s.end_date().map(day::short_date).unwrap_or_default())).unwrap_or_default(),
        Item::Calendar => tr("Abonniert, wird jede Stunde abgerufen"),
        Item::Naming => if f.naming_on { tr("Nach Muster") } else { tr("Namen bleiben, wie du sie gibst") },
        Item::Editor => if prefs::with(|p| p.note_editor == NoteEditor::Live) { tr("Formatiert") } else { tr("Text mit Zeilennummern") },
        Item::Cards => if f.cards == 1 { tr("1 Karte") } else { trf("%lld Karten", &[&f.cards]) },
        Item::Sync => tr("Eingerichtet"),
    }
}

fn action_label(item: Item) -> String {
    match item {
        Item::Modules => tr("Ordner wählen…"),
        Item::Semester => tr("Prüfen"),
        Item::Calendar => tr("Adresse eintragen"),
        Item::Naming => tr("Einrichten"),
        Item::Editor => tr("Wählen"),
        Item::Cards => tr("Aus Anki importieren…"),
        Item::Sync => tr("Einrichten…"),
    }
}

fn pick_modules(cx: &mut App) {
    act(cx, |l| {
        l.setup_only = Some(super::wizards::SETUP_MODULES);
        l.needs_setup = true;
    });
}

/// Where "Ändern" on a settled item leads.
fn change(item: Item, cx: &mut App) {
    match item {
        Item::Modules => pick_modules(cx),
        Item::Semester => super::settings::open_lit("Vorlesungsbeginn", cx),
        Item::Calendar => super::settings::open_lit("Kalender abonnieren", cx),
        Item::Naming => super::settings::open_lit("Nach einem Muster benennen", cx),
        Item::Editor => super::settings::open_lit("Notizen schreiben", cx),
        Item::Cards => act(cx, |l| l.set_route(Route::Cards)),
        Item::Sync if super::sync::enabled_here(cx) => super::settings::open_sync(cx),
        Item::Sync => act(cx, |l| l.show_sync_setup = true),
    }
}

fn label(text: String, t: &Theme) -> Div {
    div().w(px(hints::FORM_LABEL)).flex_none().text_size(px(font::CALLOUT)).text_color(t.secondary).child(text)
}

impl SetupCard {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let lib = state::lib(cx);
        let calendar = cx.new(|_| CalendarAsk::new(true));
        let subs = vec![
            cx.observe(&lib, |_, _, cx| cx.notify()),
            cx.subscribe(&calendar, |this, _, _: &Fetched, cx| {
                this.open = None;
                cx.notify();
            }),
        ];
        Self { fields: Fields::default(), open: None, had_open: false, celebrating: None, calendar, focus_field: false, more: false, _subs: subs }
    }

    /// Whether the overview should show the list at all right now.
    pub fn wanted(&self, cx: &App) -> bool {
        let marks = marks(cx);
        let facts = state::read(cx, |l| facts(l, cx));
        if marks.dismissed || marks.completed {
            return false;
        }
        let (completed, dismissed) = shared(cx);
        if completed || dismissed {
            return device_offer(&facts, &marks).is_some();
        }
        setup::wanted(&facts, &marks.skipped, &marks.confirmed, marks.dismissed, marks.completed) && (self.had_open || !setup::open_items(&facts, &marks.skipped, &marks.confirmed).is_empty())
    }

    /// Whether the list currently offers the Moodle calendar, so the overview need not say it twice.
    pub fn offers_calendar(&self, cx: &App) -> bool {
        let marks = marks(cx);
        let facts = state::read(cx, |l| facts(l, cx));
        self.wanted(cx) && !marks.collapsed && !setup::settled(Item::Calendar, &facts, &marks.skipped, &marks.confirmed) && (!hidden(cx) || device_offer(&facts, &marks) == Some(Item::Calendar))
    }

    fn primary(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        match item {
            Item::Semester | Item::Calendar | Item::Naming | Item::Editor => {
                self.open = if self.open == Some(item) { None } else { Some(item) };
                self.focus_field = self.open.is_some();
                cx.notify();
            }
            Item::Modules => pick_modules(cx),
            Item::Cards => super::cards_page::choose_import(window, cx),
            Item::Sync => act(cx, |l| l.show_sync_setup = true),
        }
    }

    fn semester_form(&mut self, snap: &Snap, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let Some(s) = snap.semester.clone() else { return div() };
        let (id1, id2) = (s.id.clone(), s.id.clone());
        let start = self.fields.date("setup-start", s.start_date(), window, cx, move |d, cx| edit_config(cx, |c| if let Some(s) = c.semesters.iter_mut().find(|s| s.id == id1) { s.start = day::format(d) }));
        let end = self.fields.date("setup-end", s.end_date(), window, cx, move |d, cx| edit_config(cx, |c| if let Some(s) = c.semesters.iter_mut().find(|s| s.id == id2) { s.end = day::format(d) }));
        let weeks = schedule::lecture_weeks(&s).len();
        let breaks = if s.breaks.is_empty() { tr("keine Pausen") } else { s.breaks.iter().map(|b| b.name.clone()).collect::<Vec<_>>().join(", ") };
        div()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .child(div().flex().items_center().gap(px(space::M)).child(label(tr("Vorlesungsbeginn"), t)).child(div().w(px(hints::DATE_WIDTH)).child(DatePicker::new(&start))))
            .child(div().flex().items_center().gap(px(space::M)).child(label(tr("Vorlesungsende"), t)).child(div().w(px(hints::DATE_WIDTH)).child(DatePicker::new(&end))))
            .child(div().flex().items_center().gap(px(space::M)).child(label(s.name.clone(), t)).child(caption(trf("%lld Vorlesungswochen · %@", &[&weeks, &breaks]), t)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::M))
                    .child(Button::new("setup-semester-ok").primary().small().label(tr("Stimmt so")).on_click(cx.listener(|this, _, _, cx| {
                        this.open = None;
                        confirm(Item::Semester, cx);
                    })))
                    .child(link_button("setup-semester-more", tr("Pausen und Feiertage…"), t).text_size(px(font::CAPTION)).on_click(|_, _, cx| super::settings::open_lit("Pausen", cx))),
            )
    }

    fn calendar_form(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if std::mem::take(&mut self.focus_field) {
            self.calendar.update(cx, |c, _| c.focus = true);
        }
        self.calendar.clone().into_any_element()
    }

    fn naming_form(&mut self, snap: &Snap, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let Some(c) = snap.config.as_ref() else { return div() };
        let on = c.naming_convention();
        let author = self.fields.input("setup-author", &c.author, &tr("Nachname-V, z. B. Muster-M"), window, cx, |v, cx| edit_config(cx, |c| c.author = v.trim().to_string()));
        let sample = snap.modules.first().cloned().unwrap_or_else(|| Module { code: "ABC".into(), ..Default::default() });
        let shown_author = if c.author.trim().is_empty() { tr("Nachname-V") } else { c.author.trim().to_string() };
        let example = Naming::with_author(&sample, snap.semester.as_ref(), &shown_author, "", CoauthorStyle::Initial).render(&tr("Titel"), chrono::Local::now().date_naive(), "pdf", &[], None, None, None);
        div()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .child(div().flex().items_center().gap(px(space::M)).child(label(tr("Nach Muster benennen"), t)).child(Switch::new("setup-naming").checked(on).on_click(|v, _, cx| {
                let v = *v;
                edit_config(cx, |c| c.set_naming_convention(v));
            })))
            .when(on, |d| {
                d.child(div().flex().items_center().gap(px(space::M)).child(label(tr("Dein Kürzel"), t)).child(div().w(px(hints::DATE_WIDTH)).child(Input::new(&author).small())))
                    .child(div().flex().items_center().gap(px(space::M)).child(label(tr("Beispiel"), t)).child(div().min_w_0().truncate().font_family(Theme::mono_family()).text_size(px(font::CAPTION)).text_color(t.secondary).child(example)))
            })
            .child(div().flex().child(Button::new("setup-naming-done").small().label(tr("Fertig")).on_click(cx.listener(|this, _, _, cx| {
                this.open = None;
                cx.notify();
            }))))
    }

    fn editor_form(&self, cx: &mut Context<Self>, t: &Theme) -> Div {
        let current = prefs::with(|p| p.note_editor);
        let me = cx.entity().downgrade();
        let choose = |kind: NoteEditor| {
            let me = me.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                prefs::update(|p| {
                    p.note_editor = kind;
                    p.note_editor_chosen = true;
                });
                let _ = me.update(cx, |s, _| s.open = None);
                super::app::rebuild_menus(cx);
                cx.refresh_windows();
            }
        };
        div()
            .flex()
            .gap(px(space::M))
            .child(super::wizards::order_choice("setup-editor-live", "sparkles", tr("Formatiert"), tr("Überschriften, Listen und Formeln erscheinen beim Tippen."), current == NoteEditor::Live, choose(NoteEditor::Live), t))
            .child(super::wizards::order_choice("setup-editor-code", "code", tr("Text mit Zeilennummern"), tr("Der ganze Text als Quelle, mit Vorschau daneben."), current == NoteEditor::Code, choose(NoteEditor::Code), t))
    }

    #[allow(clippy::too_many_arguments)]
    fn row(&mut self, item: Item, first: bool, f: &Facts, snap: &Snap, marks: &SetupPrefs, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let open = self.open == Some(item);
        let settled = setup::settled(item, f, &marks.skipped, &marks.confirmed) && !open;
        let me = cx.entity();
        let glyph = if setup::done(item, f) || marks.confirmed.contains(item.key()) {
            div().flex_none().size(px(iconsize::L)).rounded_full().bg(t.green).flex().items_center().justify_center().child(icon("check", iconsize::XS, white()))
        } else if settled {
            div().flex_none().size(px(iconsize::L)).rounded_full().border_2().border_color(t.tertiary).flex().items_center().justify_center().child(icon("minus", iconsize::XS, t.tertiary))
        } else {
            div().flex_none().size(px(iconsize::L)).rounded_full().border_2().border_color(t.accent.opacity(0.6))
        };
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(space::XXS))
            .child(div().font_weight(FontWeight::MEDIUM).when(settled, |d| d.text_color(t.secondary)).child(title(item)))
            .child(caption(if settled { summary(item, f, snap, marks) } else { ask(item) }, t));
        let mut head = div().flex().items_center().gap(px(space::M)).child(glyph).child(icon(icon_of(item), iconsize::S, if settled { t.tertiary } else { t.secondary })).child(text);
        if settled {
            let skipped = marks.skipped.contains(item.key()) && !setup::done(item, f);
            head = head.child(if skipped {
                link_button(SharedString::from(format!("setup-unskip-{}", item.key())), tr("Doch einrichten"), t).text_size(px(font::CAPTION)).on_click(move |_, _, cx| unskip(item, cx))
            } else {
                link_button(SharedString::from(format!("setup-change-{}", item.key())), tr("Ändern"), t).text_size(px(font::CAPTION)).text_color(t.secondary).on_click(move |_, _, cx| change(item, cx))
            });
        } else {
            head = head
                .child(Button::new(SharedString::from(format!("setup-do-{}", item.key()))).small().when(first, |b| b.primary()).label(action_label(item)).on_click(move |_, window, cx| me.update(cx, |s, cx| s.primary(item, window, cx))))
                .child(link_button(SharedString::from(format!("setup-skip-{}", item.key())), tr("Überspringen"), t).text_size(px(font::CAPTION)).text_color(t.secondary).tooltip(tip(tr("Geht jederzeit später, auch aus den Einstellungen"))).on_click(move |_, _, cx| skip(item, cx)));
        }
        let form = match (open, item) {
            (true, Item::Semester) => Some(self.semester_form(snap, window, cx, t)),
            (true, Item::Calendar) => Some(div().w_full().child(self.calendar_form(cx))),
            (true, Item::Naming) => Some(self.naming_form(snap, window, cx, t)),
            (true, Item::Editor) => Some(self.editor_form(cx, t)),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .px(px(space::L))
            .py(px(space::M))
            .child(head)
            .child(div().w_full().child(motion::reveal(SharedString::from(format!("setup-form-{}", item.key())), open, div().w_full().pl(px(iconsize::L + space::M + iconsize::S + space::M)).pt(px(space::M)).children(form))))
    }

    fn celebration(&mut self, cx: &mut Context<Self>, t: &Theme) -> Div {
        if self.celebrating.is_none() {
            let root = root(cx);
            self.celebrating = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(hints::DONE_FOR).await;
                let _ = this.update(cx, |s, cx| {
                    s.celebrating = None;
                    if self::root(cx) == root {
                        settle(cx, true, false);
                    }
                });
            }));
        }
        let travel = motion::timed(motion::SLOW, cx);
        div().child(
            banner(t.green)
                .flex()
                .items_center()
                .gap(px(space::L))
                .child(div().flex_none().size(px(iconsize::HERO)).rounded_full().bg(t.green).flex().items_center().justify_center().child(icon("check", iconsize::L, white())))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().font_weight(FontWeight::BOLD).child(tr("Alles eingerichtet"))).child(caption(tr("Die App kennt dein Semester. Von hier an zeigt die Übersicht, was dran ist."), t)))
                .with_animation("setup-done", Animation::new(travel).with_easing(ease_out_quint()), |d, p| d.opacity(p)),
        )
    }
}

impl Render for SetupCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let root_now = root(cx);
        let key = prefs::workspace_key(&root_now);
        if !prefs::with(|p| p.setup.contains_key(&key)) {
            let busy = state::read(cx, |l| !l.semester_assignments().is_empty());
            prefs::update_setup(&root_now, |s| s.collapsed = busy);
        }
        let marks = marks(cx);
        if marks.dismissed || marks.completed {
            return div();
        }
        let (facts, snap) = state::read(cx, |l| (facts(l, cx), Snap { config: l.config.clone(), semester: l.semester().cloned(), modules: l.semester_modules().into_iter().cloned().collect() }));
        let (done_elsewhere, closed_elsewhere) = shared(cx);
        if done_elsewhere || closed_elsewhere {
            let Some(item) = device_offer(&facts, &marks) else { return div() };
            let row = self.row(item, true, &facts, &snap, &marks, window, cx, &t);
            self.fields.prune();
            return grouped(&t).child(div().px(px(space::L)).pt(px(space::M)).child(caption(tr("Auf diesem Gerät noch offen"), &t))).child(row);
        }
        let open_items = setup::open_items(&facts, &marks.skipped, &marks.confirmed);
        if open_items.is_empty() {
            if !self.had_open {
                cx.defer(|cx| settle(cx, true, false));
                return div();
            }
            return self.celebration(cx, &t);
        }
        self.had_open = true;
        self.celebrating = None;
        let (settled, total) = setup::progress(&facts, &marks.skipped, &marks.confirmed);
        let collapsed = marks.collapsed;
        let (fold_root, close_root) = (root(cx), root(cx));
        let header = div()
            .flex()
            .items_center()
            .gap(px(space::M))
            .px(px(space::L))
            .py(px(space::M))
            .child(
                div()
                    .id("setup-fold")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(space::M))
                    .mx(px(-space::S))
                    .px(px(space::S))
                    .py(px(space::XS))
                    .rounded(px(radius::CONTROL))
                    .cursor_pointer()
                    .key_nav()
                    .soft(t.ink.opacity(0.05), radius::CONTROL)
                    .tooltip(tip(if collapsed { tr("Aufklappen") } else { tr("Zuklappen") }))
                    .on_click(move |_, _, cx| {
                        prefs::update_setup(&fold_root, |s| s.collapsed = !s.collapsed);
                        cx.refresh_windows();
                    })
                    .child(motion::chevron("setup-chevron", !collapsed, t.secondary))
                    .child(div().font_weight(FontWeight::BOLD).child(tr("Einrichten")))
                    .child(div().font_features(tabular()).text_color(t.secondary).child(format!("{settled}/{total}")))
                    .child(div().flex_1().max_w(px(hints::PROGRESS_WIDTH)).child(progress_bar(settled as f32 / total.max(1) as f32, t.accent, &t)))
                    .when(collapsed, |d| d.child(caption(if open_items.len() == 1 { tr("1 Punkt offen") } else { trf("%lld Punkte offen", &[&open_items.len()]) }, &t))),
            )
            .child(
                div()
                    .id("setup-close")
                    .flex_none()
                    .size(px(space::XL))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .key_nav()
                    .soft(t.ink.opacity(0.1), radius::FULL)
                    .tooltip(tip(tr("Ausblenden; die Einstellungen zeigen die Liste wieder")))
                    .on_click(move |_, _, cx| {
                        settle(cx, false, true);
                        let undo_root = close_root.clone();
                        act(cx, |l| {
                            l.notify(
                                tr("Einrichten ausgeblendet"),
                                "eye-off",
                                Some(tr("Einblenden")),
                                Some(Box::new(move |l: &mut Library| {
                                    prefs::update_setup(&undo_root, |s| s.dismissed = false);
                                    l.mark_setup(false, false);
                                })),
                            )
                        });
                    })
                    .child(icon("x", iconsize::S, t.secondary)),
            );
        let first = open_items[0];
        let waiting = open_items.len() - 1;
        let mut body = div().flex().flex_col().child(self.row(first, true, &facts, &snap, &marks, window, cx, &t));
        let more = self.more;
        let me = cx.entity();
        let all = open_items.clone();
        body = body.child(divider(&t)).child(
            div()
                .flex()
                .items_center()
                .gap(px(space::M))
                .px(px(space::L))
                .py(px(space::S))
                .child(
                    div()
                        .id("setup-more")
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .mx(px(-space::S))
                        .px(px(space::S))
                        .py(px(space::XS))
                        .rounded(px(radius::CONTROL))
                        .cursor_pointer()
                        .key_nav()
                        .soft(t.ink.opacity(0.05), radius::CONTROL)
                        .on_click(move |_, _, cx| {
                            me.update(cx, |s, cx| {
                                s.more = !s.more;
                                cx.notify();
                            })
                        })
                        .child(motion::chevron("setup-more-chevron", more, t.secondary))
                        .child(caption(
                            match waiting {
                                0 => tr("Erledigte Schritte"),
                                1 => tr("Noch 1 Schritt"),
                                n => trf("Noch %@ Schritte", &[&n.to_string()]),
                            },
                            &t,
                        )),
                )
                .child(div().flex_1())
                .child(link_button("setup-skip-all", tr("Alle überspringen"), &t).text_size(px(font::CAPTION)).text_color(t.secondary).tooltip(tip(tr("Geht jederzeit später, auch aus den Einstellungen"))).on_click(move |_, _, cx| {
                    remember(cx, |s| s.skipped.extend(all.iter().map(|i| i.key().to_string())));
                })),
        );
        if more {
            for item in Item::ALL.into_iter().filter(|i| *i != first) {
                let row = self.row(item, false, &facts, &snap, &marks, window, cx, &t);
                body = body.child(divider(&t)).child(row);
            }
        }
        self.fields.prune();
        let footer = div()
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::L))
            .py(px(space::M))
            .border_t_1()
            .border_color(t.separator)
            .child(icon("list-checks", iconsize::S, t.tertiary))
            .child(caption(tr("Alles ist freiwillig. Die Liste hakt sich von selbst ab."), &t))
            .child(div().flex_1())
            .child(link_button("setup-guided", tr("Geführt einrichten…"), &t).text_size(px(font::CAPTION)).tooltip(tip(tr("Alle Schritte nacheinander in einem Fenster"))).on_click(|_, _, cx| act(cx, |l| l.needs_setup = true)));
        grouped(&t).child(header).child(div().w_full().child(motion::reveal("setup-body", !collapsed, div().w_full().flex().flex_col().child(divider(&t)).child(body).child(footer))))
    }
}
