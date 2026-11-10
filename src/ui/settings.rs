use std::collections::{HashMap, HashSet};

use chrono::{Days, Local};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable, ThemeStyled};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::fields::Fields;
use super::time_field::Preset;
use super::state::{self, act, edit_config};
use super::motion::{self, Soft};
use super::theme::{self, Theme, font, radius, space, iconsize, control};
use super::widgets::*;
use crate::core::text::fold;
use crate::core::i18n::{ctrl_name, tr, trf};
use crate::core::ics;
use crate::core::model::*;
use crate::core::naming::Naming;
use crate::core::prefs::{self, NoteSaveMode, Shortcut};
use crate::core::shortcuts::{Customized, Group, ShortcutAction};
use crate::core::{clock, day, schedule};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    General,
    Semester,
    Modules,
    Notes,
    Cards,
    Sync,
    Ai,
    Shortcuts,
}

const ACCENTS: [(&str, &str); 8] = [
    ("#0EA5E9", "Himmelblau"),
    ("#2563EB", "Blau"),
    ("#0D9488", "Petrol"),
    ("#16A34A", "Grün"),
    ("#D97706", "Bernstein"),
    ("#DC2626", "Rot"),
    ("#7C3AED", "Violett"),
    ("#334155", "Schiefer"),
];

/// Other words people look for a setting with.
const SYNONYMS: &[(&str, &str)] = &[
    ("theme", "Erscheinungsbild"),
    ("design", "Erscheinungsbild"),
    ("dark", "Erscheinungsbild"),
    ("dunkel", "Erscheinungsbild"),
    ("light", "Erscheinungsbild"),
    ("hell", "Erscheinungsbild"),
    ("nord", "Erscheinungsbild"),
    ("dracula", "Erscheinungsbild"),
    ("farbe", "Erscheinungsbild"),
    ("color", "Erscheinungsbild"),
    ("font", "Schriftgröße"),
    ("schrift", "Schriftgröße"),
    ("text", "Schriftgröße"),
    ("zoom", "Schriftgröße"),
    ("breite", "Text in voller Breite"),
    ("width", "Text in voller Breite"),
    ("spalte", "Text in voller Breite"),
    ("lesebreite", "Text in voller Breite"),
    ("zentriert", "Text in voller Breite"),
    ("language", "Sprache"),
    ("english", "Sprache"),
    ("deutsch", "Sprache"),
    ("autostart", "Bei Anmeldung starten"),
    ("tray", "Symbol in der Menüleiste"),
    ("git", "Sync"),
    ("github", "Online-Speicher"),
    ("backup", "Sicherung"),
    ("zip", "Sicherung"),
    ("workspace", "Ordner"),
    ("vault", "Obsidian"),
    ("live", "Notizen schreiben"),
    ("markdown", "Notizen schreiben"),
    ("speichern", "Sichern"),
    ("autosave", "Sichern"),
    ("benachrichtigung", "Mitteilungen vor Deadlines"),
    ("notification", "Mitteilungen vor Deadlines"),
    ("ics", "Kalenderdatei aktuell halten"),
    ("apple", "Mit Apple Kalender abgleichen"),
    ("icloud", "Mit Apple Kalender abgleichen"),
    ("iphone", "Mit Apple Kalender abgleichen"),
    ("outlook", "Im Kalender zeigen"),
    ("thunderbird", "Im Kalender zeigen"),
    ("google", "Im Kalender zeigen"),
    ("moodle", "Kalender abonnieren"),
    ("spell", "Rechtschreibung prüfen"),
    ("dictionary", "Eigenes Wörterbuch"),
    ("wörterbuch", "Eigenes Wörterbuch"),
    ("ects", "Bezeichnung der Punkte"),
    ("naming", "Namensmuster"),
    ("dateiname", "Namensmuster"),
    ("umbenennen", "Namensmuster"),
    ("muster", "Namensmuster"),
    ("note", "Notensystem"),
    ("flashcard", "Lernen"),
    ("karteikarten", "Lernen"),
    ("fsrs", "Erinnerungsziel"),
    ("algorithmus", "Erinnerungsziel"),
    ("retention", "Erinnerungsziel"),
    ("shortcut", "Tastenkürzel"),
    ("hotkey", "Tastenkürzel"),
    ("keybinding", "Tastenkürzel"),
    ("ki", "KI-Assistent verwenden"),
    ("ai", "KI-Assistent verwenden"),
    ("assistent", "KI-Assistent verwenden"),
    ("claude", "Programme"),
    ("codex", "Programme"),
    ("opencode", "Programme"),
    ("mcp", "Für die Kommandozeile"),
    ("terminal", "Im Terminal öffnen"),
];

/// Settings shown only while there is a naming convention; search leads to its switch instead.
const NEEDS_CONVENTION: &[&str] = &["Matrikelnummer", "Mitautor:innen", "Muster", "Beispiel"];

/// Every labelled setting and the tab it lives on, for the search field.
const SEARCHABLE: &[(&str, Tab)] = &[
    ("Darstellung", Tab::General),
    ("Erscheinungsbild", Tab::General),
    ("Studium", Tab::General),
    ("Name", Tab::General),
    ("Kürzel in Dateinamen", Tab::General),
    ("Inbox-Ordner", Tab::General),
    ("Matrikelnummer", Tab::General),
    ("Mitautor:innen", Tab::General),
    ("Ablegen und Ordner", Tab::General),
    ("Abgabeordner", Tab::General),
    ("Namensmuster", Tab::General),
    ("Nach einem Muster benennen", Tab::General),
    ("Ordner anlegen", Tab::General),
    ("Noten", Tab::General),
    ("Notensystem", Tab::General),
    ("Bezeichnung der Punkte", Tab::General),
    ("Suche", Tab::General),
    ("Ausgeschlossene Ordner", Tab::General),
    ("Indexierte Dateien", Tab::General),
    ("Speicherort", Tab::General),
    ("Ordner", Tab::General),
    ("Sicherung", Tab::General),
    ("Konfiguration", Tab::General),
    ("Sprache", Tab::General),
    ("Erinnerungen", Tab::General),
    ("Mitteilungen vor Deadlines", Tab::General),
    ("Erinnern", Tab::General),
    ("Zusätzlich am Vortag (Pflichtabgaben)", Tab::General),
    ("Kalenderdatei aktuell halten", Tab::General),
    ("Mit Apple Kalender abgleichen", Tab::General),
    ("Adresse", Tab::General),
    ("Kalender abonnieren", Tab::General),
    ("Im Kalender zeigen", Tab::General),
    ("Abonnieren oder importieren", Tab::General),
    ("System", Tab::General),
    ("Bei Anmeldung starten", Tab::General),
    ("Automatisch nach Updates suchen", Tab::General),
    ("Symbol in der Menüleiste", Tab::General),
    ("Seitenleiste", Tab::General),
    ("Hinweise", Tab::General),
    ("Tipp des Tages", Tab::General),
    ("Einrichten in der Übersicht", Tab::General),
    ("Semester", Tab::Semester),
    ("Vorlesungsbeginn", Tab::Semester),
    ("Vorlesungsende", Tab::Semester),
    ("Vorlesungswochen", Tab::Semester),
    ("Module", Tab::Semester),
    ("Pausen", Tab::Semester),
    ("Feiertage", Tab::Semester),
    ("Kürzel", Tab::Modules),
    ("Muster", Tab::Modules),
    ("Beispiel", Tab::Modules),
    ("Modul", Tab::Modules),
    ("Links", Tab::Modules),
    ("Wiederkehrende Deadlines", Tab::Modules),
    ("Stundenplan", Tab::Modules),
    ("Sync", Tab::Sync),
    ("Jetzt synchronisieren", Tab::Sync),
    ("Wann sichern", Tab::Sync),
    ("Nach Ruhe", Tab::Sync),
    ("Beim Öffnen synchronisieren", Tab::Sync),
    ("Beim Beenden synchronisieren", Tab::Sync),
    ("Online-Speicher", Tab::Sync),
    ("Git-Pfad", Tab::Sync),
    ("Sync ausschalten", Tab::Sync),
    ("Status", Tab::Sync),
    ("Branch", Tab::Sync),
    ("Letzter Stand", Tab::Sync),
    ("Automatik", Tab::Sync),
    ("Ausgeschlossen", Tab::Sync),
    ("Erweitert", Tab::Sync),
    ("Version", Tab::Sync),
    ("Einrichtung erneut prüfen", Tab::Sync),
    ("Anhangsordner", Tab::Notes),
    ("Linkformat", Tab::Notes),
    ("Vorlagen", Tab::Notes),
    ("Verbindung", Tab::Notes),
    ("Vault", Tab::Notes),
    ("Tastenkürzel", Tab::Shortcuts),
    ("KI-Assistent verwenden", Tab::Ai),
    ("Programme", Tab::Ai),
    ("Knopf in der Werkzeugleiste", Tab::Ai),
    ("Für die Kommandozeile", Tab::Ai),
    ("Befehl", Tab::Ai),
    ("Im Terminal öffnen", Tab::Ai),
    ("Lernen", Tab::Cards),
    ("Erinnerungsziel", Tab::Cards),
    ("Neue Karten pro Tag", Tab::Cards),
    ("Wiederholungen pro Tag", Tab::Cards),
    ("Karten schreiben", Tab::Cards),
    ("Editor", Tab::Notes),
    ("Notizen schreiben", Tab::Notes),
    ("Rechtschreibung prüfen", Tab::Notes),
    ("Eigenes Wörterbuch", Tab::Notes),
    ("Obsidian", Tab::Notes),
    ("Schriftgröße", Tab::Notes),
    ("Text in voller Breite", Tab::Notes),
    ("Sichern", Tab::Notes),
    ("Pause", Tab::Notes),
    ("PDF-Export", Tab::Notes),
    ("Akzentfarbe", Tab::Notes),
    ("Kopfzeile mit Modul, Woche, Name und Datum", Tab::Notes),
    ("Seitenzahlen", Tab::Notes),
    ("Vor dem Ersetzen eines früheren Exports fragen", Tab::Notes),
    ("Eigenschaften über dem Text", Tab::Notes),
];

/// Why the keys just pressed were not taken as a shortcut.
#[derive(Clone)]
enum Rejected {
    NeedsModifier,
    Reserved(String),
}

/// The inline offer to take back the last shortcut change, with the shortcuts as they were.
struct Notice {
    id: u64,
    text: String,
    before: Customized,
}

/// The settings window: Allgemein, Semester, Module, Notizen, Tastenkürzel.
pub struct Settings {
    tab: Tab,
    fields: Fields,
    semester: Option<String>,
    module: Option<String>,
    expanded_rule: Option<String>,
    advanced: HashSet<String>,
    filter: Entity<InputState>,
    search: Entity<InputState>,
    /// The search hit Enter opens.
    hit: usize,
    /// The scrolling page of the tabs that scroll as a whole.
    page: ScrollHandle,
    recording: Option<ShortcutAction>,
    rejected: Option<Rejected>,
    key_buttons: HashMap<&'static str, FocusHandle>,
    last_changed: Option<ShortcutAction>,
    notice: Option<Notice>,
    notices: u64,
    focus: FocusHandle,
    launch_at_login: bool,
    markdown_default: Option<bool>,
    _subs: Vec<Subscription>,
}

/// Opens the settings window, or brings the open one to the front.
pub fn open(cx: &mut App) {
    show(None, cx);
}

/// Opens the settings on the page of one module.
pub fn open_module(id: &str, cx: &mut App) {
    show(Some(id.to_string()), cx);
}

fn show(module: Option<String>, cx: &mut App) {
    for w in cx.windows() {
        if let Some(h) = w.downcast::<gpui_kit::base::Root>()
            && h.update(cx, |root, window, cx| {
                let Ok(view) = root.view().clone().downcast::<Settings>() else { return false };
                super::drive::activate_window(window);
                if let Some(id) = module.clone() {
                    view.update(cx, |s, cx| {
                        s.show_module(id);
                        cx.notify();
                    });
                }
                true
            })
            .unwrap_or(false)
        {
            return;
        }
    }
    let bounds = Bounds::centered(None, size(px(880.), px(640.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(760.), px(520.))),
        titlebar: Some(TitlebarOptions { title: Some(tr("Einstellungen").into()), appears_transparent: true, traffic_light_position: Some(point(px(14.), px(14.))) }),
        ..Default::default()
    };
    let options = super::chrome::window_options(options, tr("Einstellungen"));
    let _ = super::chrome::open_window(options, cx, |window, cx| cx.new(|cx| {
        let mut settings = Settings::new(window, cx);
        if let Some(id) = module {
            settings.show_module(id);
        }
        settings
    }));
}

/// Whether a typed folder stays inside the workspace; anything else keeps the old value.
fn is_inner_folder(folder: &str) -> bool {
    let path = std::path::Path::new(folder.trim());
    !folder.trim().is_empty() && path.is_relative() && !path.components().any(|c| matches!(c, std::path::Component::ParentDir))
}

fn module_names(cx: &App) -> Vec<String> {
    state::read(cx, |l| l.semester_modules().iter().take(ShortcutAction::MODULES.len()).map(|m| m.display_name()).collect())
}

/// What the notice after a shortcut change says: the action and the keys it has now.
fn changed(action: ShortcutAction, cx: &App) -> String {
    let keys = action.shortcut().map_or_else(|| tr("Kein Kürzel"), |s| s.display());
    trf("%@: %@", &[&action.named(&module_names(cx)), &keys])
}

fn shortcut_fits(action: ShortcutAction, modules: &[String], filter: &str) -> bool {
    action.named(modules).to_lowercase().contains(filter) || action.label().to_lowercase().contains(filter)
}

fn rules_and_sessions(rules: usize, sessions: usize) -> String {
    let rules = if rules == 1 { tr("1 Regel") } else { trf("%lld Regeln", &[&rules]) };
    let sessions = if sessions == 1 { tr("1 Termin") } else { trf("%lld Termine", &[&sessions]) };
    format!("{rules} · {sessions}")
}

/// The about panel: logo, version and licenses.
pub fn open_about(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(360.), px(300.)), cx);
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), is_resizable: false, titlebar: Some(TitlebarOptions { title: Some(tr("Über Schrumbi").into()), appears_transparent: true, traffic_light_position: Some(point(px(14.), px(14.))) }), ..Default::default() };
    let options = super::chrome::window_options(options, tr("Über Schrumbi"));
    let _ = super::chrome::open_window(options, cx, |_, cx| cx.new(|_| About));
}

struct About;

impl Render for About {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        super::perf::frame_start();
        let t = Theme::get(cx);
        let content = div()
            .size_full()
            .bg(t.content)
            .font_family(Theme::font_family())
            .text_color(t.primary)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(space::S))
            .child(super::onboarding::logo(40., &t))
            .child(div().text_color(t.secondary).child(trf("Version %@", &[&env!("CARGO_PKG_VERSION")])))
            .child(div().text_size(px(font::CAPTION)).text_color(t.secondary).child("© 2026 Schrumbo · GNU AGPL-3.0"))
            .child(link_button("about-source", tr("Quellcode"), &t).text_size(px(font::CAPTION)).on_click(|_, _, _| {
                crate::core::library::open_path(env!("CARGO_PKG_REPOSITORY"));
            }))
            .child(div().text_size(px(font::CAPTION)).text_color(t.tertiary).child("Lucide (ISC) · Material Icon Theme (MIT)"));
        super::chrome::frame(tr("Über Schrumbi"), content, &t)
    }
}

/// The setting a search result led to: it scrolls into view once and lights up for a moment.
struct Jump {
    label: String,
    handle: ScrollHandle,
    scrolled: bool,
}

thread_local! {
    static JUMP: std::cell::RefCell<Option<Jump>> = const { std::cell::RefCell::new(None) };
}

/// The row or heading with this label, marked and brought into view while a jump leads to it.
fn jump_target(label: &str, content: Div, t: &Theme) -> Div {
    let hit = JUMP.with(|j| j.borrow().as_ref().filter(|j| j.label == label).map(|j| (j.handle.clone(), !j.scrolled)));
    let Some((handle, scroll)) = hit else { return content };
    if scroll {
        JUMP.with(|j| {
            if let Some(j) = j.borrow_mut().as_mut() {
                j.scrolled = true;
            }
        });
    }
    content.relative().bg(t.accent.opacity(0.14)).rounded(px(radius::INNER)).when(scroll, |d| {
        d.child(
            canvas(
                move |bounds, window, _| {
                    let handle = handle.clone();
                    window.on_next_frame(move |window, _| {
                        let view = handle.bounds();
                        let now = handle.offset();
                        handle.set_offset(point(now.x, now.y - (bounds.top() - view.top() - px(64.))));
                        window.refresh();
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    })
}

fn section(title: Option<String>, footer: Option<String>, rows: Vec<AnyElement>, t: &Theme) -> Div {
    let mut g = grouped(t);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            g = g.child(divider(t));
        }
        g = g.child(r);
    }
    div()
        .flex()
        .flex_col()
        .gap(px(space::S))
        .when_some(title, |d, s| {
            let key = s.clone();
            d.child(jump_target(&key, div().px(px(space::XS)).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(s), t))
        })
        .child(g)
        .when_some(footer, |d, f| d.child(div().px(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.secondary).child(f)))
}

/// How the Apple calendar stands, with the one button that helps.
fn apple_calendar_row(t: &Theme) -> AnyElement {
    use super::apple_calendar::{self as apple, State};
    let open = || Some(Button::new("apple-cal-open").small().label(tr("Kalender öffnen")).on_click(|_, _, _| apple::open_calendar_app()));
    let settings = |id: &'static str| Some(Button::new(id).small().label(tr("Systemeinstellungen öffnen")).on_click(|_, _, _| apple::open_privacy_settings()));
    let (mark, label, button) = match apple::state() {
        State::Off => (Some(("circle-dashed", t.secondary)), tr("Wartet auf deinen Studienordner"), None),
        State::Working => (None, tr("Gleicht ab …"), None),
        State::Asking => (None, tr("Warte auf deine Erlaubnis …"), None),
        State::Synced(0) => (Some(("circle-check", t.green)), tr("Noch keine Termine in diesem Semester"), None),
        State::Synced(1) => (Some(("circle-check", t.green)), tr("1 Termin im Kalender „Schrumbi“"), open()),
        State::Synced(n) => (Some(("circle-check", t.green)), trf("%lld Termine im Kalender „Schrumbi“", &[&n]), open()),
        State::Denied => (Some(("circle-alert", t.orange)), tr("Kein Zugriff auf deine Kalender"), settings("apple-cal-allow")),
        State::Restricted => (Some(("circle-alert", t.orange)), tr("Kalenderzugriff ist auf diesem Mac gesperrt"), settings("apple-cal-restricted")),
        State::Unavailable => (Some(("circle-alert", t.secondary)), tr("Nur in der installierten Schrumbi-App"), None),
        State::Failed(e) => (Some(("circle-alert", t.orange)), trf("Abgleich fehlgeschlagen: %@", &[&e]), Some(Button::new("apple-cal-retry").small().label(tr("Erneut versuchen")).on_click(|_, _, _| apple::recheck()))),
    };
    let indicator = match mark {
        Some((name, color)) => icon(name, iconsize::S, color).into_any_element(),
        None => motion::loading(iconsize::S, t.secondary).into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .gap(px(space::M))
        .px(px(space::L))
        .min_h(px(control::ROW))
        .py(px(space::S))
        .child(div().flex_1().min_w_0().flex().items_center().gap(px(space::S)).text_color(t.primary).child(div().flex_none().child(indicator)).child(div().id("apple-cal-state").min_w_0().truncate().child(label.clone()).tooltip(tip(label))))
        .children(button.map(|b| div().flex_none().max_w(px(control::ROW_CONTROL)).child(b)))
        .into_any_element()
}

fn row(label: impl Into<SharedString>, control: impl IntoElement, t: &Theme) -> AnyElement {
    let label: SharedString = label.into();
    let key = label.to_string();
    jump_target(&key, div().flex().items_center().gap(px(space::M)).px(px(space::L)).min_h(px(control::ROW)).py(px(space::S)).child(div().flex_1().min_w_0().text_color(t.primary).child(label)).child(div().flex_none().max_w(px(control::ROW_CONTROL)).child(control)), t).into_any_element()
}

/// A pop-up button like SwiftUI's menu-style `Picker`.
#[derive(IntoElement)]
struct Picker {
    button: Button,
    build: std::rc::Rc<dyn Fn(PopupMenu) -> PopupMenu>,
}

impl Picker {
    fn small(mut self) -> Self {
        self.button = self.button.small();
        self
    }

    fn disabled(mut self, disabled: bool) -> Self {
        self.button = self.button.disabled(disabled);
        self
    }
}

impl RenderOnce for Picker {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let build = self.build;
        self.button.dropdown_menu(move |menu, _, _| build(menu))
    }
}

fn picker(id: impl Into<ElementId>, label: String, build: impl Fn(PopupMenu) -> PopupMenu + 'static) -> Picker {
    Picker { button: Button::new(id).label(label).dropdown_caret(true), build: std::rc::Rc::new(build) }
}

fn detail_row(label: impl Into<SharedString>, detail: String, control: impl IntoElement, t: &Theme) -> AnyElement {
    let label: SharedString = label.into();
    let key = label.to_string();
    jump_target(&key, div().flex().items_center().gap(px(space::M)).px(px(space::L)).min_h(px(control::ROW)).py(px(space::S)).child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().text_color(t.primary).child(label)).child(caption(detail, t))).child(div().flex_none().child(control)), t).into_any_element()
}

fn naming_switch(id: &'static str, c: &StudyConfig) -> Switch {
    Switch::new(id).checked(c.naming_convention()).on_click(|v, _, cx| {
        let v = *v;
        edit_config(cx, |c| c.set_naming_convention(v));
    })
}

pub(crate) fn naming_detail(on: bool) -> String {
    if on { tr("Abgelegte Dateien und Notizen in Abgabeordnern heißen nach dem Muster des Moduls.") } else { tr("Dateien und Notizen behalten die Namen, die du ihnen gibst.") }
}

fn sub_row(label: impl Into<SharedString>, control: impl IntoElement, t: &Theme) -> AnyElement {
    let label: SharedString = label.into();
    let key = label.to_string();
    jump_target(&key, div().flex().items_center().gap(px(space::M)).pl(px(space::L + space::XL)).pr(px(space::L)).min_h(px(control::ROW)).py(px(space::S)).child(div().flex_1().min_w_0().text_color(t.primary).child(label)).child(div().flex_none().max_w(px(control::ROW_CONTROL)).child(control)), t).into_any_element()
}

fn filing_sections(c: &StudyConfig, extra: Vec<AnyElement>, submissions: AnyElement, t: &Theme) -> (Div, Div) {
    let switch = |id: &'static str, on: bool, set: fn(&mut StudyConfig, bool)| Switch::new(id).checked(on).on_click(move |v, _, cx| {
        let v = *v;
        edit_config(cx, |c| set(c, v));
    });
    let on = c.naming_convention();
    let mut naming = vec![detail_row(tr("Nach einem Muster benennen"), naming_detail(on), naming_switch("g-naming", c), t)];
    if on {
        naming.extend(extra);
    }
    let folders = vec![
        detail_row(tr("Abgabeordner"), tr("In jedem Modul. Hier landen abgelegte Dateien und neue Notizen für eine Abgabe. Leer: der Modulordner."), submissions, t),
        row(tr("Ordner anlegen"), switch("g-create-folders", c.create_folders, |c, v| c.create_folders = v), t),
    ];
    (
        section(Some(tr("Namensmuster")), on.then(|| tr("Das Muster steht in den Einstellungen jedes Moduls.")), naming, t),
        section(Some(tr("Ablegen und Ordner")), Some(tr("Schrumbi benennt und verschiebt nie etwas von selbst. Ohne „Ordner anlegen“ landen Dateien nur in Ordnern, die es schon gibt.")), folders, t),
    )
}

fn minus(id: impl Into<ElementId>, help: String, f: impl Fn(&mut App) + 'static) -> Button {
    Button::new(id).ghost().small().icon(menu_icon("circle-minus")).tooltip(help).on_click(move |_, _, cx| f(cx))
}

fn break_summary(b: &Break) -> String {
    let (Some(start), Some(end)) = (day::parse(&b.start), day::parse(&b.end)) else { return String::new() };
    if start > end {
        return String::new();
    }
    let mut weeks = Vec::new();
    let mut monday = day::monday(start);
    while monday <= end {
        if b.contains(monday) {
            weeks.push(day::iso_week(monday));
        }
        monday = monday + Days::new(7);
    }
    match (weeks.first(), weeks.last()) {
        (Some(f), Some(l)) => {
            let range = if f == l { format!("KW{f:02}") } else { format!("KW{f:02}–KW{l:02}") };
            if weeks.len() == 1 { trf("1 Woche · %@", &[&range]) } else { trf("%lld Wochen · %@", &[&weeks.len(), &range]) }
        }
        _ => tr("keine volle Woche"),
    }
}

/// Where a search hit leads.
#[derive(Clone)]
enum HitTarget {
    Setting(Tab, String),
    Module(String),
    Semester(String),
    Shortcut(String),
}

#[derive(Clone)]
struct Hit {
    label: String,
    place: String,
    to: HitTarget,
}

fn tab_name(tab: Tab) -> String {
    match tab {
        Tab::General => tr("Allgemein"),
        Tab::Semester => tr("Semester"),
        Tab::Modules => tr("Module"),
        Tab::Notes => tr("Notizen"),
        Tab::Cards => tr("Karten"),
        Tab::Sync => tr("Sync"),
        Tab::Ai => tr("KI"),
        Tab::Shortcuts => tr("Tastenkürzel"),
    }
}

fn theme_label(choice: prefs::ThemeChoice) -> String {
    match choice {
        prefs::ThemeChoice::System => tr("System"),
        prefs::ThemeChoice::Light => tr("Hell"),
        prefs::ThemeChoice::Dark => tr("Dunkel"),
        prefs::ThemeChoice::Nord => "Nord".to_string(),
        prefs::ThemeChoice::Dracula => "Dracula".to_string(),
    }
}

impl Settings {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Kürzel suchen")));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Suchen")));
        let lib = state::lib(cx);
        let (me, handle) = (cx.weak_entity(), window.window_handle());
        let recorder = cx.intercept_keystrokes(move |e, window, cx| {
            let Some(settings) = me.upgrade().filter(|s| window.window_handle() == handle && s.read(cx).armed(window)) else { return };
            settings.update(cx, |s, cx| s.record_key(&e.keystroke, cx));
            cx.stop_propagation();
        });
        let subs = vec![recorder, cx.observe(&lib, |_, _, cx| cx.notify()), cx.subscribe(&filter, |_, _, _: &InputEvent, cx| cx.notify()), cx.subscribe_in(&search, window, |s, _, e: &InputEvent, window, cx| {
            match e {
                InputEvent::PressEnter { .. } => {
                    let found = s.matches(cx);
                    if let Some(hit) = found.get(s.hit.min(found.len().saturating_sub(1))).cloned() {
                        s.open_result(hit, window, cx);
                    }
                }
                InputEvent::Change => s.hit = 0,
                _ => {}
            }
            cx.notify();
        })];
        let (semester, module) = state::read(cx, |l| (l.config.as_ref().and_then(|c| c.current_semester(l.now)).map(|s| s.id.clone()), l.config.as_ref().and_then(|c| c.modules.first()).map(|m| m.id.clone())));
        super::ai::rescan(cx);
        Self {
            tab: Tab::General,
            fields: Fields::default(),
            semester,
            module,
            expanded_rule: None,
            advanced: HashSet::new(),
            filter,
            search,
            hit: 0,
            page: ScrollHandle::new(),
            recording: None,
            rejected: None,
            key_buttons: HashMap::new(),
            last_changed: None,
            notice: None,
            notices: 0,
            focus: cx.focus_handle(),
            launch_at_login: super::platform::launches_at_login(),
            markdown_default: super::platform::markdown_is_default(),
            _subs: subs,
        }
    }

    /// Settings, modules, semesters and shortcuts that fit what is typed, the best first.
    fn matches(&self, cx: &App) -> Vec<Hit> {
        let query = fold(self.search.read(cx).value().trim());
        if query.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(u8, Hit)> = Vec::new();
        let convention = state::read(cx, |l| l.config.as_ref().is_none_or(StudyConfig::naming_convention));
        for (key, tab) in SEARCHABLE {
            if !super::apple_calendar::AVAILABLE && *key == "Mit Apple Kalender abgleichen" {
                continue;
            }
            let label = tr(key);
            let folded = fold(&label);
            let score = if folded.starts_with(&query) {
                0
            } else if folded.contains(&query) {
                1
            } else if SYNONYMS.iter().any(|(word, target)| target == key && word.starts_with(&query)) {
                2
            } else {
                continue;
            };
            let target = if !convention && NEEDS_CONVENTION.contains(key) { "Nach einem Muster benennen" } else { key };
            scored.push((score, Hit { label, place: tab_name(*tab), to: HitTarget::Setting(*tab, tr(target)) }));
        }
        state::read(cx, |l| {
            if let Some(c) = &l.config {
                for m in &c.modules {
                    if fold(&m.code).contains(&query) || fold(&m.name).contains(&query) {
                        scored.push((1, Hit { label: m.display_name(), place: tr("Module"), to: HitTarget::Module(m.id.clone()) }));
                    }
                }
                for semester in &c.semesters {
                    if fold(&semester.name).contains(&query) {
                        scored.push((1, Hit { label: semester.name.clone(), place: tr("Semester"), to: HitTarget::Semester(semester.id.clone()) }));
                    }
                }
            }
        });
        let modules = module_names(cx);
        for action in ShortcutAction::ALL {
            let label = action.named(&modules);
            if fold(&label).contains(&query) || fold(&action.label()).contains(&query) {
                scored.push((3, Hit { label: label.clone(), place: tr("Tastenkürzel"), to: HitTarget::Shortcut(label) }));
            }
        }
        scored.sort_by(|a, b| a.0.cmp(&b.0));
        scored.into_iter().map(|(_, hit)| hit).take(40).collect()
    }

    fn show_module(&mut self, id: String) {
        self.tab = Tab::Modules;
        self.module = Some(id);
    }

    fn open_result(&mut self, hit: Hit, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = None;
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        match hit.to {
            HitTarget::Setting(tab, label) => {
                self.tab = tab;
                self.light_up(label, window, cx);
            }
            HitTarget::Module(id) => self.show_module(id),
            HitTarget::Semester(id) => {
                self.tab = Tab::Semester;
                self.semester = Some(id);
            }
            HitTarget::Shortcut(label) => {
                self.tab = Tab::Shortcuts;
                self.filter.update(cx, |f, cx| f.set_value(label, window, cx));
            }
        }
        cx.notify();
    }

    /// Scrolls to a setting and marks it until the eye has found it.
    fn light_up(&mut self, label: String, _: &mut Window, cx: &mut Context<Self>) {
        JUMP.with(|j| *j.borrow_mut() = Some(Jump { label, handle: self.page.clone(), scrolled: false }));
        cx.spawn(async |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(1800)).await;
            JUMP.with(|j| *j.borrow_mut() = None);
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn search_results(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let results = self.matches(cx);
        if results.is_empty() {
            return unavailable("search", tr("Keine Treffer"), tr("Versuch ein anderes Wort, zum Beispiel Theme, Schrift oder Sync."), t).into_any_element();
        }
        let selected = self.hit.min(results.len() - 1);
        let mut list = div().w_full().max_w(px(560.)).flex().flex_col().gap(px(space::XXS));
        for (i, hit) in results.into_iter().enumerate() {
            let (label, place, open) = (hit.label.clone(), hit.place.clone(), hit);
            list = list.child(
                div()
                    .id(("settings-result", i))
                    .w_full()
                    .cursor_pointer()
                    .key_nav()
                    .soft(t.ink.opacity(0.06), radius::INNER)
                    .flex()
                    .items_center()
                    .px(px(space::M))
                    .py(px(space::S))
                    .rounded(px(radius::INNER))
                    .when(i == selected, |d| d.bg(t.ink.opacity(0.09)))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(caption(place, t))
                    .on_click(cx.listener(move |s, _, window, cx| s.open_result(open.clone(), window, cx))),
            );
        }
        div().id("settings-results").size_full().overflow_y_scrollbar().p(px(space::XL)).flex().justify_center().child(list).into_any_element()
    }

    fn tab_bar(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let mut bar = div().flex().justify_start().pl(px(if super::chrome::CUSTOM { space::M } else { 72. })).gap(px(space::XXS)).pt(px(if super::chrome::CUSTOM { space::M } else { space::XXL })).pb(px(space::S)).border_b_1().border_color(t.separator);
        for (tab, label, icon_name) in [
            (Tab::General, tr("Allgemein"), "settings"),
            (Tab::Semester, tr("Semester"), "calendar"),
            (Tab::Modules, tr("Module"), "library"),
            (Tab::Notes, tr("Notizen"), "square-pen"),
            (Tab::Cards, tr("Karten"), "layers"),
            (Tab::Sync, tr("Sync"), "cloud"),
            (Tab::Ai, tr("KI"), "bot"),
            (Tab::Shortcuts, tr("Tastenkürzel"), "command"),
        ] {
            let active = tab == self.tab;
            let hover = t.ink.opacity(0.06);
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("tab-{label}")))
                    .w(px(76.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(space::XS))
                    .py(px(space::S))
                    .rounded(px(radius::CONTROL))
                    .cursor_pointer().key_nav()
                    .when(active, |d| d.bg(t.ink.opacity(0.09)))
                    .when(!active, |d| d.soft(hover, radius::CONTROL))
                    .child(icon(icon_name, iconsize::L, if active { t.accent } else { t.secondary }))
                    .child(div().text_size(px(font::CAPTION)).text_color(if active { t.accent_text } else { t.primary }).child(label))
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.tab = tab;
                        s.recording = None;
                        cx.notify();
                    })),
            );
        }
        div().relative().child(bar).child(div().absolute().top(px(if super::chrome::CUSTOM { space::M } else { space::XXL })).right(px(space::M)).w(px(170.)).child(Input::new(&self.search).small().cleanable(true).prefix(icon("search", iconsize::XS, t.secondary))))
    }

    fn general(&mut self, c: &StudyConfig, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let mut f = std::mem::take(&mut self.fields);
        let text = |f: &mut Fields, key: &str, value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>, set: fn(&mut StudyConfig, String)| f.input(key, value, placeholder, window, cx, move |v, cx| edit_config(cx, |c| set(c, v)));
        let name = text(&mut f, "g-name", &c.name, "", window, cx, |c, v| c.name = v);
        let author = text(&mut f, "g-author", &c.author, &tr("Nachname-V"), window, cx, |c, v| c.author = v);
        let inbox = text(&mut f, "g-inbox", &c.inbox, "", window, cx, |c, v| if is_inner_folder(&v) { c.inbox = v });
        let submissions = text(&mut f, "g-submissions", &c.submission_folder, &tr("Modulordner"), window, cx, |c, v| if v.trim().is_empty() || is_inner_folder(&v) { c.submission_folder = v });
        let matrikel = text(&mut f, "g-matrikel", &c.matrikel, &tr("für {matrikel} in Dateinamen"), window, cx, |c, v| c.matrikel = v);
        let credit = text(&mut f, "g-credit", &c.credit_label, &tr("ECTS, CP, Credits"), window, cx, |c, v| c.credit_label = v);
        let exclude = text(&mut f, "g-exclude", &c.search_exclude.join(", "), "", window, cx, |c, v| c.search_exclude = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
        self.fields = f;
        let p = prefs::get();
        let (root, index_count, config_path, exporting) = state::read(cx, |l| (l.root.clone(), l.index_count, l.config_path(), l.is_exporting));
        let style = c.coauthor_style;
        let grading = c.grading;

        let studium_rows = vec![
            row(tr("Name"), div().w(px(300.)).child(Input::new(&name)), t),
            row(tr("Inbox-Ordner"), div().w(px(300.)).child(Input::new(&inbox)), t),
        ];
        let naming_extra = vec![
            sub_row(tr("Kürzel in Dateinamen"), div().w(px(300.)).child(Input::new(&author)), t),
            sub_row(tr("Matrikelnummer"), div().w(px(300.)).child(Input::new(&matrikel)), t),
            sub_row(
                tr("Mitautor:innen"),
                picker("g-coauthor", if style == CoauthorStyle::Initial { tr("Nachname-V (Mueller-C)") } else { tr("Frei (beliebiger Name)") }, move |m| {
                    m.item(PopupMenuItem::new(tr("Nachname-V (Mueller-C)")).checked(style == CoauthorStyle::Initial).on_click(|_, _, cx| edit_config(cx, |c| c.coauthor_style = CoauthorStyle::Initial)))
                        .item(PopupMenuItem::new(tr("Frei (beliebiger Name)")).checked(style == CoauthorStyle::Free).on_click(|_, _, cx| edit_config(cx, |c| c.coauthor_style = CoauthorStyle::Free)))
                }),
                t,
            ),
        ];
        let studium = section(Some(tr("Studium")), None, studium_rows, t);
        let (naming, filing) = filing_sections(c, naming_extra, div().w(px(300.)).child(Input::new(&submissions)).into_any_element(), t);
        let grades = section(
            Some(tr("Noten")),
            None,
            vec![
                row(
                    tr("Notensystem"),
                    picker("g-grading", grading.label(), move |mut m| {
                        for g in GradingSystem::ALL {
                            m = m.item(PopupMenuItem::new(g.label()).checked(g == grading).on_click(move |_, _, cx| edit_config(cx, |c| c.grading = g)));
                        }
                        m
                    }),
                    t,
                ),
                row(tr("Bezeichnung der Punkte"), div().w(px(200.)).child(Input::new(&credit)), t),
            ],
            t,
        );
        let search = section(
            Some(tr("Suche")),
            Some(tr("Kommagetrennte Ordnernamen, die Suche und Dateiliste überspringen. Dateien über 60 MB und nicht geladene iCloud-Dateien werden nur mit Namen indexiert.")),
            vec![
                row(tr("Ausgeschlossene Ordner"), div().w(px(300.)).child(Input::new(&exclude)), t),
                row(tr("Indexierte Dateien"), div().text_color(t.secondary).child(index_count.to_string()), t),
                row(
                    "",
                    Button::new("g-reindex").label(tr("Index neu aufbauen")).on_click(|_, _, cx| {
                        state::read(cx, |l| l.search_index.reset());
                        state::reindex(cx);
                    }),
                    t,
                ),
            ],
            t,
        );
        let (r1, c1, c2) = (root.clone(), config_path.clone(), config_path.clone());
        let storage = section(
            Some(tr("Speicherort")),
            Some(tr("Änderungen werden automatisch in schrumbi.json gespeichert.")),
            vec![
                row(
                    tr("Ordner"),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .child(div().max_w(px(220.)).truncate().text_color(t.secondary).child(root.as_ref().map_or("—".into(), |r| r.display().to_string())))
                        .when_some(r1, |d, r| d.child(Button::new("g-show-root").small().label(tr("Zeigen")).on_click(move |_, _, _| crate::core::library::reveal(&r))))
                        .child(Button::new("g-change-root").small().label(tr("Ändern…")).on_click(|_, _, cx| {
                            super::platform::choose_folder(cx, |p, cx| {
                                if state::close_workspace(cx) {
                                    state::open_workspace(&p, cx);
                                }
                            })
                        })),
                    t,
                ),
                row(tr("Sicherung"), Button::new("g-zip").label(tr("Studium als ZIP exportieren…")).disabled(exporting).on_click(|_, _, cx| super::platform::export_archive(cx)), t),
                row(
                    tr("Konfiguration"),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .child(div().text_color(t.secondary).child("schrumbi.json"))
                        .child(Button::new("g-show-config").small().label(tr("Zeigen")).on_click(move |_, _, _| {
                            if let Some(p) = &c1 {
                                crate::core::library::reveal(p);
                            }
                        }))
                        .child(Button::new("g-reload-config").small().label(tr("Neu laden")).on_click(move |_, _, cx| {
                            let _ = &c2;
                            act(cx, |l| {
                                l.load_config();
                                l.refresh();
                            })
                        })),
                    t,
                ),
            ],
            t,
        );
        let lang = p.language.clone();
        let label = match lang.as_str() {
            "de" => "Deutsch".to_string(),
            "en" => "English".to_string(),
            _ => tr("Systemsprache"),
        };
        let theme_choice = p.theme;
        let language = section(
            Some(tr("Darstellung")),
            None,
            vec![row(
                tr("Erscheinungsbild"),
                picker("g-theme", theme_label(theme_choice), move |mut m| {
                    for choice in [prefs::ThemeChoice::System, prefs::ThemeChoice::Light, prefs::ThemeChoice::Dark, prefs::ThemeChoice::Nord, prefs::ThemeChoice::Dracula] {
                        m = m.item(PopupMenuItem::new(theme_label(choice)).checked(choice == theme_choice).on_click(move |_, window, cx| theme::choose(choice, window, cx)));
                    }
                    m
                }),
                t,
            ), row(
                tr("Sprache"),
                picker("g-lang", label, move |m| {
                    let set = |code: &'static str| {
                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            prefs::update(|p| p.language = code.into());
                            crate::core::i18n::init(code);
                            super::app::rebuild_menus(cx);
                            cx.refresh_windows();
                        }
                    };
                    m.item(PopupMenuItem::new(tr("Systemsprache")).checked(lang.is_empty()).on_click(set("")))
                        .item(PopupMenuItem::new("Deutsch").checked(lang == "de").on_click(set("de")))
                        .item(PopupMenuItem::new("English").checked(lang == "en").on_click(set("en")))
                }),
                t,
            )]
            .into_iter()
            .collect(),
            t,
        );
        let enabled = p.reminders_enabled;
        let hours = p.reminder_hours;
        let reminders = section(
            Some(tr("Erinnerungen")),
            None,
            vec![
                row(tr("Mitteilungen vor Deadlines"), Switch::new("g-rem").checked(enabled).on_click(|v, _, cx| {
                    let v = *v;
                    prefs::update(|p| p.reminders_enabled = v);
                    state::sync_reminders(cx);
                }), t),
                row(
                    tr("Erinnern"),
                    picker("g-hours", if hours == 0 { tr("Nicht") } else { trf("%lld Std. vorher", &[&hours]) }, move |mut m| {
                        for h in [0i64, 1, 2, 3, 6, 12] {
                            m = m.item(PopupMenuItem::new(if h == 0 { tr("Nicht") } else { trf("%lld Std. vorher", &[&h]) }).checked(h == hours).on_click(move |_, _, cx| {
                                prefs::update(|p| p.reminder_hours = h);
                                state::sync_reminders(cx);
                            }));
                        }
                        m
                    })
                    .disabled(!enabled),
                    t,
                ),
                row(tr("Zusätzlich am Vortag (Pflichtabgaben)"), Switch::new("g-daybefore").checked(p.reminder_day_before).disabled(!enabled).on_click(|v, _, cx| {
                    let v = *v;
                    prefs::update(|p| p.reminder_day_before = v);
                    state::sync_reminders(cx);
                }), t),
            ],
            t,
        );
        let source_root = state::read(cx, |l| l.root.clone()).unwrap_or_default().to_string_lossy().to_string();
        let source_value = p.calendar_sources.get(&source_root).cloned().unwrap_or_default();
        let mut fields = std::mem::take(&mut self.fields);
        let source_field = fields.input("calendar-source", &source_value, "https://…", window, cx, move |v, _| {
            let (root, v) = (source_root.clone(), v.trim().to_string());
            prefs::update(|p| {
                if v.is_empty() {
                    p.calendar_sources.remove(&root);
                } else {
                    p.calendar_sources.insert(root, v);
                }
            });
        });
        self.fields = fields;
        let incoming = section(
            Some(tr("Kalender abonnieren")),
            Some(tr("Trage die Kalenderadresse von Moodle ein (Kalender → Exportieren → Kalender-URL abrufen). Neue und geänderte Abgaben kommen dann von selbst, beim Start und jede Stunde. „Abrufen“ zeigt alles und ordnet Kurse zu.")),
            vec![row(tr("Adresse"), div().flex().items_center().gap(px(space::S)).child(div().w(px(280.)).child(Input::new(&source_field).small())).child(Button::new("calendar-fetch").small().label(tr("Abrufen")).disabled(source_value.trim().is_empty()).on_click(|_, _, cx| state::refresh_calendar(true, cx))), t)],
            t,
        );
        let feed_on = p.calendar_feed;
        let feed_file = state::read(cx, |l| l.root.clone()).map(|r| r.join("Schrumbi-Kalender.ics")).unwrap_or_default();
        let how = if cfg!(target_os = "macos") {
            tr("Apple Kalender: Ablage → Neues Kalenderabonnement, Adresse einfügen. Outlook für Mac kann die Datei einmalig importieren.")
        } else if cfg!(target_os = "windows") {
            tr("Outlook (klassisch): Datei → Kontoeinstellungen → Internetkalender → Neu, Adresse einfügen. Thunderbird: Neuer Kalender → Im Netzwerk. Neues Outlook und die Windows-Kalender-App können nur einmalig importieren.")
        } else {
            tr("Thunderbird: Neuer Kalender → Im Netzwerk, Adresse einfügen. GNOME Kalender und Evolution: Neuer Kalender → Aus dem Web. KDE Merkuro: Kalender hinzufügen → iCal-Datei, dort die Datei wählen.")
        };
        let mut outgoing_rows = Vec::new();
        if super::apple_calendar::AVAILABLE {
            let sync_on = p.apple_calendar;
            outgoing_rows.push(row(tr("Mit Apple Kalender abgleichen"), Switch::new("g-apple-cal").checked(sync_on).on_click(|v, _, cx| {
                let v = *v;
                prefs::update(|p| p.apple_calendar = v);
                if v {
                    super::apple_calendar::push(cx);
                } else {
                    super::apple_calendar::turn_off();
                }
                cx.refresh_windows();
            }), t));
            if sync_on {
                outgoing_rows.push(apple_calendar_row(t));
            }
        }
        outgoing_rows.push(row(tr("Kalenderdatei aktuell halten"), Switch::new("g-cal").checked(feed_on).on_click(|v, _, cx| {
            let v = *v;
            prefs::update(|p| p.calendar_feed = v);
            state::sync_reminders(cx);
            cx.refresh_windows();
        }), t));
        if feed_on {
            let (open_file, reveal_file) = (feed_file.clone(), feed_file.clone());
            outgoing_rows.push(row(tr("Abonnieren oder importieren"), div().flex().gap(px(space::S))
                .child(Button::new("calendar-copy").small().label(tr("Adresse kopieren")).on_click(|_, _, cx| {
                    if let Some(address) = crate::core::calendar_server::address() {
                        super::drive::copy(address.to_string(), cx);
                        state::act(cx, |l| l.notify(tr("Adresse kopiert"), "link", None, None));
                    }
                }))
                .child(Button::new("calendar-once").small().ghost().label(tr("Einmalig importieren")).on_click(move |_, _, _| {
                    crate::core::library::open_path(&open_file);
                }))
                .child(Button::new("calendar-reveal").small().ghost().icon(menu_icon("folder")).tooltip(super::platform::reveal_label()).on_click(move |_, _, _| crate::core::library::reveal(&reveal_file))), t));
        }
        let calendar = section(
            Some(tr("Im Kalender zeigen")),
            Some(if super::apple_calendar::AVAILABLE {
                format!("{} {} {} {}", tr("Abgaben, Prüfungen und Termine erscheinen in deiner Kalender-App."), tr("Der Abgleich trägt sie in den Kalender „Schrumbi“ ein, über iCloud auch auf dem iPhone; deine eigenen Termine bleiben unberührt."), how, tr("Die Adresse gilt auf diesem Computer, solange Schrumbi läuft."))
            } else {
                format!("{} {} {}", tr("Abgaben, Prüfungen und Termine erscheinen in deiner Kalender-App."), how, tr("Die Adresse gilt auf diesem Computer, solange Schrumbi läuft; die App behält den letzten Stand. Fürs Handy oder Google Kalender braucht es eine Adresse im Netz, etwa den Freigabelink der Datei aus Dropbox."))
            }),
            outgoing_rows,
            t,
        );
        let login = self.launch_at_login;
        let system = section(
            Some(tr("System")),
            None,
            vec![
                row(tr("Bei Anmeldung starten"), Switch::new("g-login").checked(login).on_click(cx.listener(|s, v: &bool, _, cx| {
                    if super::platform::set_launch_at_login(*v) {
                        s.launch_at_login = *v;
                    } else {
                        act(cx, |l| l.last_error = Some(tr("Autostart konnte nicht geändert werden.")));
                    }
                    cx.notify();
                })), t),
                row(tr("Automatisch nach Updates suchen"), Switch::new("g-update").checked(p.update_check).on_click(|v, _, _| {
                    let v = *v;
                    prefs::update(|p| p.update_check = v);
                }), t),
                row(tr("Symbol in der Menüleiste"), Switch::new("g-tray").checked(p.tray).disabled(!super::tray::AVAILABLE).on_click(|v, _, cx| {
                    let v = *v;
                    prefs::update(|p| p.tray = v);
                    super::tray::sync(cx);
                }), t),
                div().flex().items_center().gap(px(space::S)).px(px(space::L)).py(px(space::S)).text_color(t.secondary).child(icon("shield-check", iconsize::S, t.secondary)).child(tr("Sendet keine Daten · nur der gewählte Ordner wird gelesen")).into_any_element(),
            ],
            t,
        );
        let hidden = p.hidden_tabs.clone();
        let sidebar = section(
            Some(tr("Seitenleiste")),
            Some(tr("Ausgeblendete Einträge bleiben über Kürzel und Befehlspalette erreichbar. Rechtsklick auf einen Eintrag blendet ihn direkt aus.")),
            [("semester", tr("Semester")), ("record", tr("Leistungen")), ("inbox", tr("Inbox")), ("notes", tr("Notizen")), ("cards", tr("Karten"))]
                .into_iter()
                .map(|(key, label)| {
                    let shown = !hidden.iter().any(|k| k == key);
                    row(label, Switch::new(SharedString::from(format!("g-tab-{key}"))).checked(shown).on_click(move |v, _, cx| super::sidebar::set_hidden(key, !*v, cx)), t)
                })
                .collect(),
            t,
        );
        let setup_shown = !super::setup_card::hidden(cx);
        let hints = section(
            Some(tr("Hinweise")),
            Some(tr("Der Tipp des Tages erscheint einmal am Tag unten rechts und nennt nur, was du noch nicht benutzt hast. Die Liste zum Einrichten hakt sich von selbst ab.")),
            vec![
                row(tr("Tipp des Tages"), Switch::new("g-tips").checked(p.tips_enabled).on_click(|v, _, cx| {
                    let v = *v;
                    prefs::update(|p| {
                        p.tips_enabled = v;
                        if v {
                            p.tips_last.clear();
                        }
                    });
                    cx.refresh_windows();
                }), t),
                row(tr("Einrichten in der Übersicht"), Switch::new("g-setup").checked(setup_shown).on_click(|v, _, cx| {
                    if *v {
                        super::setup_card::restore(cx);
                    } else {
                        let root = state::read(cx, |l| l.root.clone()).unwrap_or_default();
                        prefs::update_setup(&root, |s| s.dismissed = true);
                        cx.refresh_windows();
                    }
                }), t),
            ],
            t,
        );
        div().flex().flex_col().gap(px(space::XL)).child(language).child(studium).child(naming).child(filing).child(grades).child(sidebar).child(hints).child(reminders).child(incoming).child(calendar).child(system).child(search).child(storage)
    }
}

/// Starts a new instance and quits this one, so a language change applies.
impl Settings {
    fn master(&self, items: Vec<(String, String, String, Option<Hsla>)>, selected: Option<&str>, on_select: impl Fn(&mut Settings, String) + 'static + Clone, bar: Div, t: &Theme, cx: &mut Context<Self>) -> Div {
        let mut list = div().flex().flex_col().p(px(space::S));
        for (id, title, sub, dot) in items {
            if title.is_empty() {
                list = list.child(div().px(px(space::S)).pt(px(space::S)).pb(px(space::XXS)).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(sub));
                continue;
            }
            let active = selected == Some(id.as_str());
            let hover = t.ink.opacity(0.05);
            let f = on_select.clone();
            let id2 = id.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("m-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .px(px(space::S))
                    .py(px(space::XS))
                    .rounded(px(radius::INNER))
                    .cursor_pointer().key_nav()
                    .when(active, |d| d.bg(t.selection).text_color(white()))
                    .when(!active, |d| d.soft(hover, radius::INNER))
                    .when_some(dot, |d, c| d.child(div().size(px(space::S)).rounded_full().bg(if active { white() } else { c })))
                    .child(div().flex().flex_col().min_w_0().child(div().truncate().child(title)).child(div().truncate().text_size(px(font::CAPTION)).text_color(if active { white().opacity(0.8) } else { t.secondary }).child(sub)))
                    .on_click(cx.listener(move |s, _, _, cx| {
                        f(s, id2.clone());
                        cx.notify();
                    })),
            );
        }
        div().w(px(210.)).flex_none().h_full().flex().flex_col().border_r_1().border_color(t.separator).child(div().id("master").flex_1().min_h_0().overflow_y_scrollbar().child(list)).child(bar.border_t_1().border_color(t.separator).p(px(space::S)))
    }

    fn semesters(&mut self, c: &StudyConfig, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let mut sorted = c.semesters.clone();
        sorted.sort_by(|a, b| b.start.cmp(&a.start));
        let items = sorted.iter().map(|s| (s.id.clone(), s.name.clone(), format!("{} – {}", s.start_date().map(day::short_date).unwrap_or(s.start.clone()), s.end_date().map(|d| d.format("%d.%m.%Y").to_string()).unwrap_or(s.end.clone())), None)).collect();
        let sel = self.semester.clone();
        let can_delete = sel.is_some() && c.semesters.len() > 1;
        let me = cx.entity();
        let (e1, e2, e3) = (me.clone(), me.clone(), me.clone());
        let bar = div()
            .flex()
            .gap(px(space::XXS))
            .child(Button::new("s-add").ghost().small().icon(menu_icon("plus")).tooltip(tr("Neues Semester")).on_click(move |_, _, cx| e1.update(cx, |s, cx| s.add_semester(false, cx))))
            .child(Button::new("s-del").ghost().small().icon(menu_icon("minus")).tooltip(tr("Semester löschen")).disabled(!can_delete).on_click(move |_, window, cx| {
                let e = e2.clone();
                confirm(window, cx, tr("Semester löschen?"), tr("Module dieses Semesters bleiben erhalten und erscheinen unter „Ohne Semester“."), move |cx| {
                    e.update(cx, |s, cx| {
                        let id = s.semester.clone();
                        edit_config(cx, |c| c.semesters.retain(|x| Some(&x.id) != id.as_ref()));
                        s.semester = state::read(cx, |l| l.config.as_ref().and_then(|c| c.semesters.first()).map(|s| s.id.clone()));
                        cx.notify();
                    })
                })
            }))
            .child(Button::new("s-dup").ghost().small().icon(menu_icon("copy-plus")).tooltip(tr("Folgesemester mit denselben Modulen und Regeln anlegen")).disabled(sel.is_none()).on_click(move |_, _, cx| e3.update(cx, |s, cx| s.add_semester(true, cx))));
        let master = self.master(items, sel.as_deref(), |s, id| s.semester = Some(id), bar, t, cx);
        let detail = match c.semesters.iter().find(|s| Some(&s.id) == sel.as_ref()).cloned() {
            Some(s) => self.semester_editor(c, s, window, cx, t).into_any_element(),
            None => unavailable("calendar", tr("Semester wählen"), "", t).into_any_element(),
        };
        div().size_full().flex().child(master).child(div().id("s-detail").flex_1().min_w_0().h_full().overflow_y_scrollbar().p(px(space::XL)).child(div().w_full().child(detail)))
    }

    fn add_semester(&mut self, copy: bool, cx: &mut Context<Self>) {
        let last = state::read(cx, |l| l.config.as_ref().and_then(|c| c.semesters.iter().filter_map(|s| s.end_date()).max())).unwrap_or_else(|| Local::now().date_naive());
        let start = day::monday(last + Days::new(14));
        let end = start + Days::new(7 * 15 - 3);
        let id = format!("s{}", chrono::Utc::now().timestamp());
        let s = Semester { id: id.clone(), name: tr("Neues Semester"), start: day::format(start), end: day::format(end), ..Default::default() };
        let source = self.semester.clone();
        edit_config(cx, |c| {
            if copy && let Some(src) = &source {
                let copies: Vec<Module> = c.modules.iter().filter(|m| &m.semester == src).map(|m| Module { id: format!("{}-{}", m.id, id), semester: id.clone(), ..m.clone() }).collect();
                c.modules.extend(copies);
            }
            c.semesters.push(s);
        });
        self.semester = Some(id);
    }

    fn semester_editor(&mut self, c: &StudyConfig, s: Semester, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let id = s.id.clone();
        let edit = move |f: Box<dyn Fn(&mut Semester)>| {
            let id = id.clone();
            move |cx: &mut App| edit_config(cx, |c| if let Some(s) = c.semesters.iter_mut().find(|x| x.id == id) { f(s) })
        };
        let mut fields = std::mem::take(&mut self.fields);
        let e = edit.clone();
        let name = fields.input(&format!("sem-name-{}", s.id), &s.name, "", window, cx, move |v, cx| e(Box::new(move |s| s.name = v.clone()))(cx));
        let e = edit.clone();
        let start = fields.date(&format!("sem-start-{}", s.id), s.start_date(), window, cx, move |d, cx| e(Box::new(move |s| s.start = day::format(d)))(cx));
        let e = edit.clone();
        let end = fields.date(&format!("sem-end-{}", s.id), s.end_date(), window, cx, move |d, cx| e(Box::new(move |s| s.end = day::format(d)))(cx));
        let modules: Vec<String> = c.modules.iter().filter(|m| m.semester == s.id).map(|m| m.code.clone()).collect();
        let weeks = schedule::lecture_weeks(&s).len();
        let mut breaks: Vec<AnyElement> = Vec::new();
        for (i, b) in s.breaks.iter().enumerate() {
            let e = edit.clone();
            let n = fields.input(&format!("brk-n-{}-{i}", s.id), &b.name, &tr("Name"), window, cx, move |v, cx| e(Box::new(move |s| if let Some(b) = s.breaks.get_mut(i) { b.name = v.clone() }))(cx));
            let e = edit.clone();
            let from = fields.date(&format!("brk-s-{}-{i}", s.id), day::parse(&b.start), window, cx, move |d, cx| e(Box::new(move |s| if let Some(b) = s.breaks.get_mut(i) { b.start = day::format(d) }))(cx));
            let e = edit.clone();
            let to = fields.date(&format!("brk-e-{}-{i}", s.id), day::parse(&b.end), window, cx, move |d, cx| e(Box::new(move |s| if let Some(b) = s.breaks.get_mut(i) { b.end = day::format(d) }))(cx));
            let e = edit.clone();
            breaks.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .px(px(space::L))
                    .py(px(space::S))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S))
                            .child(div().flex_1().min_w_0().child(Input::new(&n).small()))
                            .child(div().w(px(120.)).flex_none().child(gpui_kit::component::date_picker::DatePicker::new(&from).small()))
                            .child(div().text_color(t.secondary).child(tr("bis")))
                            .child(div().w(px(120.)).flex_none().child(gpui_kit::component::date_picker::DatePicker::new(&to).small()))
                            .child(minus(SharedString::from(format!("brk-rm-{i}")), tr("Pause entfernen"), move |cx| e(Box::new(move |s| if i < s.breaks.len() { s.breaks.remove(i); }))(cx))),
                    )
                    .child(div().text_size(px(font::CAPTION)).text_color(t.secondary).child(break_summary(b)))
                    .into_any_element(),
            );
        }
        let e = edit.clone();
        let start_date = s.start.clone();
        breaks.push(div().px(px(space::L)).py(px(space::S)).child(Button::new("brk-add").ghost().small().icon(menu_icon("plus")).label(tr("Pause hinzufügen")).on_click(move |_, _, cx| {
            let st = day::parse(&start_date).unwrap_or_else(|| Local::now().date_naive());
            e(Box::new(move |s| s.breaks.push(Break { name: tr("Pause"), start: day::format(st), end: day::format(st + Days::new(6)) })))(cx)
        })).into_any_element());
        let mut holidays: Vec<AnyElement> = Vec::new();
        for (i, h) in s.holidays.iter().enumerate() {
            let e = edit.clone();
            let n = fields.input(&format!("hol-n-{}-{i}", s.id), &h.name, &tr("Name"), window, cx, move |v, cx| e(Box::new(move |s| if let Some(h) = s.holidays.get_mut(i) { h.name = v.clone() }))(cx));
            let e = edit.clone();
            let d = fields.date(&format!("hol-d-{}-{i}", s.id), day::parse(&h.date), window, cx, move |d, cx| e(Box::new(move |s| if let Some(h) = s.holidays.get_mut(i) { h.date = day::format(d) }))(cx));
            let e = edit.clone();
            holidays.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .px(px(space::L))
                    .py(px(space::S))
                    .child(div().w(px(220.)).child(Input::new(&n).small()))
                    .child(div().flex_1())
                    .child(div().w(px(130.)).child(gpui_kit::component::date_picker::DatePicker::new(&d).small()))
                    .child(minus(SharedString::from(format!("hol-rm-{i}")), tr("Feiertag entfernen"), move |cx| e(Box::new(move |s| if i < s.holidays.len() { s.holidays.remove(i); }))(cx)))
                    .into_any_element(),
            );
        }
        self.fields = fields;
        let (e1, e2) = (edit.clone(), edit.clone());
        let first = s.start.clone();
        let (sid, s2) = (s.id.clone(), s.clone());
        holidays.push(
            div()
                .flex()
                .px(px(space::L))
                .py(px(space::S))
                .child(Button::new("hol-add").ghost().small().icon(menu_icon("plus")).label(tr("Feiertag hinzufügen")).on_click(move |_, _, cx| {
                    let f = first.clone();
                    e1(Box::new(move |s| s.holidays.push(Holiday { name: tr("Feiertag"), date: f.clone() })))(cx)
                }))
                .child(div().flex_1())
                .child(Button::new("hol-ics").ghost().small().icon(menu_icon("calendar-plus")).label(tr("Aus Kalenderdatei (.ics)…")).on_click(move |_, _, cx| {
                    let (e, s, _) = (e2.clone(), s2.clone(), sid.clone());
                    super::platform::pick(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None }, cx, move |paths, cx| {
                        let Some(text) = paths.first().and_then(|p| std::fs::read_to_string(p).ok()) else { return };
                        let (Some(start), Some(end)) = (s.start_date(), s.end_date()) else { return };
                        let found: Vec<Holiday> = ics::parse(&text)
                            .into_iter()
                            .filter_map(|ev| {
                                let d = day::local_date(ics::local(ev.due));
                                (d >= start && d <= end).then(|| Holiday { name: ev.title(), date: day::format(d) })
                            })
                            .collect();
                        e(Box::new(move |s| {
                            for h in &found {
                                if !s.holidays.iter().any(|x| x.date == h.date) {
                                    s.holidays.push(h.clone());
                                }
                            }
                            s.holidays.sort_by(|a, b| a.date.cmp(&b.date));
                        }))(cx)
                    });
                }))
                .into_any_element(),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(space::XL))
            .child(section(
                Some(tr("Semester")),
                None,
                vec![
                    row(tr("Name"), div().w(px(260.)).child(Input::new(&name)), t),
                    row(tr("Vorlesungsbeginn"), div().w(px(160.)).child(gpui_kit::component::date_picker::DatePicker::new(&start)), t),
                    row(tr("Vorlesungsende"), div().w(px(160.)).child(gpui_kit::component::date_picker::DatePicker::new(&end)), t),
                    row(tr("Vorlesungswochen"), div().text_color(t.secondary).child(weeks.to_string()), t),
                    row(tr("Module"), div().text_color(t.secondary).child(if modules.is_empty() { "–".into() } else { modules.join(", ") }), t),
                ],
                t,
            ))
            .child(section(Some(tr("Pausen")), Some(tr("In Pausen werden keine Vorlesungswochen gezählt und keine Deadlines erzeugt.")), breaks, t))
            .child(section(Some(tr("Feiertage")), Some(tr("Fällt eine Deadline auf einen Feiertag, rückt sie auf den nächsten Tag. Feiertagskalender für dein Land oder deine Hochschule gibt es als .ics zum Download.")), holidays, t))
    }
}

/// Asks before a destructive change.
fn confirm(window: &mut Window, cx: &mut App, title: String, message: String, ok: impl Fn(&mut App) + 'static) {
    super::sheets::ask(window, cx, title, message, tr("Löschen"), true, ok);
}

impl Settings {
    fn modules(&mut self, c: &StudyConfig, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let mut sems = c.semesters.clone();
        sems.sort_by(|a, b| b.start.cmp(&a.start));
        let mut items: Vec<(String, String, String, Option<Hsla>)> = Vec::new();
        for s in &sems {
            items.push((String::new(), String::new(), s.name.clone(), None));
            for m in c.modules.iter().filter(|m| m.semester == s.id) {
                items.push((m.id.clone(), m.code.clone(), rules_and_sessions(m.rules.len(), m.sessions.len()), Some(t.palette(&m.color))));
            }
        }
        let orphans: Vec<&Module> = c.modules.iter().filter(|m| !c.semesters.iter().any(|s| s.id == m.semester)).collect();
        if !orphans.is_empty() {
            items.push((String::new(), String::new(), tr("Ohne Semester"), None));
            for m in orphans {
                items.push((m.id.clone(), m.code.clone(), String::new(), Some(t.palette(&m.color))));
            }
        }
        let me = cx.entity();
        let (e1, e2) = (me.clone(), me.clone());
        let sel = self.module.clone();
        let bar = div()
            .flex()
            .gap(px(space::XXS))
            .child(Button::new("m-add").ghost().small().icon(menu_icon("plus")).tooltip(tr("Neues Modul")).on_click(move |_, _, cx| {
                let (current, count) = state::read(cx, |l| (l.semester().map(|s| s.id.clone()).unwrap_or_default(), l.config.as_ref().map_or(0, |c| c.modules.len())));
                let current = e1.read(cx).semester.clone().unwrap_or(current);
                let id = format!("m{}", chrono::Utc::now().timestamp());
                let m = Module { id: id.clone(), code: "NEU".into(), name: tr("Neues Modul"), semester: current, folder: "NEU".into(), color: PALETTE[count % PALETTE.len()].into(), ..Default::default() };
                edit_config(cx, |c| c.modules.push(m));
                e1.update(cx, |s, cx| {
                    s.module = Some(id);
                    cx.notify();
                });
            }))
            .child(Button::new("m-del").ghost().small().icon(menu_icon("minus")).tooltip(tr("Modul löschen")).disabled(sel.is_none()).on_click(move |_, window, cx| {
                let e = e2.clone();
                confirm(window, cx, tr("Modul löschen?"), tr("Regeln, Stundenplan und Noten des Moduls werden entfernt. Dateien im Ordner bleiben erhalten."), move |cx| {
                    e.update(cx, |s, cx| {
                        let id = s.module.take();
                        edit_config(cx, |c| c.modules.retain(|m| Some(&m.id) != id.as_ref()));
                        cx.notify();
                    })
                })
            }));
        let master = self.master(items, sel.as_deref(), |s, id| s.module = Some(id), bar, t, cx);
        let detail = match c.modules.iter().find(|m| Some(&m.id) == sel.as_ref()).cloned() {
            Some(m) => self.module_editor(c, m, window, cx, t).into_any_element(),
            None => unavailable("library", tr("Modul wählen"), "", t).into_any_element(),
        };
        div().size_full().flex().child(master).child(div().id("m-detail").flex_1().min_w_0().h_full().overflow_y_scrollbar().p(px(space::XL)).child(div().w_full().child(detail)))
    }

    fn module_editor(&mut self, c: &StudyConfig, m: Module, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let mid = m.id.clone();
        let edit = move |f: Box<dyn Fn(&mut Module)>| -> Box<dyn Fn(&mut App)> {
            let id = mid.clone();
            Box::new(move |cx: &mut App| edit_config(cx, |c| if let Some(m) = c.module_mut(&id) { f(m) }))
        };
        let semester = c.semester(&m.semester).cloned();
        let mut fields = std::mem::take(&mut self.fields);
        let k = |s: &str| format!("{s}-{}", m.id);
        let e = edit.clone();
        let code = fields.input(&k("code"), &m.code, &tr("Kürzel"), window, cx, move |v, cx| e(Box::new(move |m| m.code = v.clone()))(cx));
        let e = edit.clone();
        let name = fields.input(&k("name"), &m.name, &tr("Name"), window, cx, move |v, cx| e(Box::new(move |m| m.name = v.clone()))(cx));
        let e = edit.clone();
        let folder = fields.input(&k("folder"), &m.folder, &tr("Ordner"), window, cx, move |v, cx| if is_inner_folder(&v) { e(Box::new(move |m| m.folder = v.clone()))(cx) });
        let e = edit.clone();
        let naming = fields.input(&k("naming"), &m.naming, &tr("Muster"), window, cx, move |v, cx| e(Box::new(move |m| m.naming = v.clone()))(cx));
        let root = state::read(cx, |l| l.root.clone());
        let exists = root.as_ref().is_some_and(|r| r.join(&m.folder).is_dir());
        let mut dots = div().flex().gap(px(space::XS));
        for name in PALETTE {
            let selected = m.color == name;
            let e = edit.clone();
            dots = dots.child(
                div()
                    .id(SharedString::from(format!("color-{name}")))
                    .size(px(20.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer().key_nav()
                    .hover(|s| s.opacity(0.75))
                    .when(selected, |d| d.border_2().border_color(t.primary))
                    .child(div().size(px(14.)).rounded_full().bg(t.palette(name)))
                    .tooltip(tip(name))
                    .on_click(move |_, _, cx| e(Box::new(move |m| m.color = name.into()))(cx)),
            );
        }
        let sems: Vec<(String, String)> = c.semesters.iter().map(|s| (s.id.clone(), s.name.clone())).collect();
        let current_sem = semester.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| tr("Ohne Semester"));
        let e = edit.clone();
        let sem_picker = picker("m-sem", current_sem, move |mut menu| {
            for (id, n) in &sems {
                let (e, id) = (e.clone(), id.clone());
                menu = menu.item(PopupMenuItem::new(n.clone()).on_click(move |_, _, cx| {
                    let id = id.clone();
                    e(Box::new(move |m| m.semester = id.clone()))(cx)
                }));
            }
            menu
        });
        let (e, base) = (edit.clone(), root.clone());
        let module_rows = vec![
            row(tr("Kürzel"), div().flex().items_center().gap(px(space::M)).child(div().w(px(110.)).child(Input::new(&code))).child(dots), t),
            row(tr("Name"), div().w(px(320.)).child(Input::new(&name)), t),
            row(tr("Semester"), sem_picker, t),
            row(
                tr("Ordner"),
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .child(div().w(px(220.)).child(Input::new(&folder)))
                    .child(div().id("folder-state").child(icon(if exists { "circle-check" } else { "circle-alert" }, iconsize::S, if exists { t.green } else { t.orange })).tooltip(tip(if exists { tr("Ordner existiert") } else { tr("Ordner existiert nicht") })))
                    .child(Button::new("m-pick-folder").small().label(tr("Wählen…")).on_click(move |_, _, cx| {
                        let (e, base) = (e.clone(), base.clone());
                        super::platform::choose_folder(cx, move |p, cx| {
                            let rel = base.as_ref().and_then(|b| p.strip_prefix(b).ok()).map(|r| crate::core::text::slash_path(r).to_string()).unwrap_or_else(|| p.to_string_lossy().to_string());
                            e(Box::new(move |m| m.folder = rel.clone()))(cx)
                        })
                    })),
                t,
            ),
        ];
        let mut links: Vec<AnyElement> = Vec::new();
        for (i, link) in m.links.iter().enumerate() {
            let e = edit.clone();
            let title = fields.input(&format!("lt-{}-{i}", m.id), &link.title, "Moodle", window, cx, move |v, cx| e(Box::new(move |m| if let Some(l) = m.links.get_mut(i) { l.title = v.clone() }))(cx));
            let e = edit.clone();
            let addr = fields.input(&format!("la-{}-{i}", m.id), &link.address, "https://moodle.example.de/course/view.php?id=…", window, cx, move |v, cx| e(Box::new(move |m| if let Some(l) = m.links.get_mut(i) { l.address = v.clone() }))(cx));
            let url = link.url().map(|u| u.to_string());
            let valid = url.is_some();
            let (e1, e2) = (edit.clone(), edit.clone());
            links.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .px(px(space::L))
                    .py(px(space::S))
                    .child(div().w(px(110.)).child(Input::new(&title).small()))
                    .child(div().flex_1().child(Input::new(&addr).small()))
                    .child(div().id(("lv", i)).child(icon(if valid { "circle-check" } else { "circle-alert" }, iconsize::S, if valid { t.green } else if link.address.is_empty() { t.secondary } else { t.orange })).tooltip(tip(if valid { tr("Gültige Adresse") } else { tr("Nur http(s)-Adressen") })))
                    .child(Checkbox::new(("lu", i)).label(tr("Upload")).checked(link.upload).tooltip(tr("Wird beim Hochladen einer Abgabe geöffnet")).on_click(move |v, _, cx| {
                        let v = *v;
                        e1(Box::new(move |m| {
                            for (j, l) in m.links.iter_mut().enumerate() {
                                l.upload = j == i && v;
                            }
                        }))(cx)
                    }))
                    .child(Button::new(("lo", i)).ghost().small().icon(menu_icon("square-arrow-out-up-right")).tooltip(tr("Öffnen")).disabled(!valid).on_click(move |_, _, _| {
                        if let Some(u) = &url {
                            crate::core::library::open_path(u);
                        }
                    }))
                    .child(minus(("lr", i), tr("Link entfernen"), move |cx| e2(Box::new(move |m| if i < m.links.len() { m.links.remove(i); }))(cx)))
                    .into_any_element(),
            );
        }
        let e = edit.clone();
        links.push(div().px(px(space::L)).py(px(space::S)).child(Button::new("link-add").ghost().small().icon(menu_icon("plus")).label(tr("Link hinzufügen")).on_click(move |_, _, cx| {
            e(Box::new(|m| {
                let first = m.links.is_empty();
                m.links.push(ModuleLink { title: if first { "Moodle".into() } else { String::new() }, upload: first, ..Default::default() })
            }))(cx)
        })).into_any_element());
        let example = Naming::new(&m, semester.as_ref(), c).render(&tr("Titel"), Local::now().date_naive(), "pdf", &[], None, None, None);
        let naming_rows = if c.naming_convention() {
            vec![row(tr("Muster"), div().w(px(320.)).child(Input::new(&naming)), t), row(tr("Beispiel"), div().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(example), t)]
        } else {
            vec![detail_row(tr("Nach einem Muster benennen"), naming_detail(false), naming_switch("m-naming", c), t)]
        };
        let mut rules: Vec<AnyElement> = Vec::new();
        for (i, r) in m.rules.iter().enumerate() {
            rules.push(self.rule_card(&m, semester.as_ref(), i, r, &mut fields, edit.clone(), window, cx, t));
        }
        let e = edit.clone();
        let me = cx.entity();
        rules.push(div().px(px(space::L)).py(px(space::S)).child(Button::new("rule-add").ghost().small().icon(menu_icon("plus")).label(tr("Regel hinzufügen")).dropdown_menu(move |menu, _, _| {
            let add = |rule: Rule| {
                let (e, me) = (e.clone(), me.clone());
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    let id = format!("r{}", chrono::Utc::now().timestamp());
                    let r = Rule { id: id.clone(), ..rule.clone() };
                    e(Box::new(move |m| m.rules.push(r.clone())))(cx);
                    me.update(cx, |s, cx| {
                        s.expanded_rule = Some(id.clone());
                        cx.notify();
                    });
                }
            };
            let base = |title: &str, short: &str, weekday: u32, time: &str, offset: i64, folder: &str| Rule { title: tr(title), short: tr(short), weekday, time: time.into(), week_offset: offset, folder: folder.into(), ..Default::default() };
            menu.item(PopupMenuItem::new(tr("Abgabe (Mo 18:00, Folgewoche)")).on_click(add(base("Hausübung", "HÜ", 1, "18:00", 1, ""))))
                .item(PopupMenuItem::new(tr("Abgabe am selben Tag (optional, 22:00)")).on_click(add(Rule { optional: true, ..base("Tagesübung", "TÜ", 3, "22:00", 0, "") })))
                .item(PopupMenuItem::new(tr("Wöchentliche Aufgabe zum Abhaken")).on_click(add(Rule { upload: false, file_pattern: "-".into(), ..base("Wochenaufgabe", "Aufgabe", 5, "18:00", 0, "") })))
                .item(PopupMenuItem::new(tr("Abgabe mit eigenem Ordner pro Woche")).on_click(add(base("Hausübung", "HÜ", 1, "18:00", 1, "W{w}_KW{kw}/Hausuebung"))))
                .separator()
                .item(PopupMenuItem::new(tr("Leere Regel")).on_click(add(base("Abgabe", "Abgabe", 1, "18:00", 0, ""))))
        })).into_any_element());
        let mut sessions: Vec<AnyElement> = Vec::new();
        let names = day::weekday_names();
        for (i, s) in m.sessions.iter().enumerate() {
            let e = edit.clone();
            let title = fields.input(&format!("st-{}-{i}", m.id), &s.title, &tr("Vorlesung"), window, cx, move |v, cx| e(Box::new(move |m| if let Some(s) = m.sessions.get_mut(i) { s.title = v.clone() }))(cx));
            let e = edit.clone();
            let room = fields.input(&format!("sr-{}-{i}", m.id), &s.location, &tr("Raum"), window, cx, move |v, cx| e(Box::new(move |m| if let Some(s) = m.sessions.get_mut(i) { s.location = v.clone() }))(cx));
            let (e1, e2, e3, e4) = (edit.clone(), edit.clone(), edit.clone(), edit.clone());
            let start = fields.time(&format!("sf-{}-{i}", m.id), &s.start, start_presets(), window, cx, move |v, cx| e2(Box::new(move |m| if let Some(s) = m.sessions.get_mut(i) { s.end = clock::keep_length(&s.start, &s.end, &v); s.start = v.clone(); }))(cx));
            let end = fields.time(&format!("se-{}-{i}", m.id), &s.end, end_presets(&s.start), window, cx, move |v, cx| e3(Box::new(move |m| if let Some(s) = m.sessions.get_mut(i) { s.end = v.clone() }))(cx));
            let begins = clock::parse(&s.start);
            end.update(cx, |f, cx| f.after(begins, cx));
            start.update(cx, |f, cx| f.before(Some(clock::last_minute()), cx));
            sessions.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .px(px(space::L))
                    .py(px(space::S))
                    .child(div().flex_1().child(Input::new(&title).small()))
                    .child(picker(("sd", i), names[(s.weekday.clamp(1, 7) - 1) as usize].chars().take(2).collect(), move |mut menu| {
                        for d in 1..=7u32 {
                            let e = e1.clone();
                            menu = menu.item(PopupMenuItem::new(names[(d - 1) as usize]).on_click(move |_, _, cx| e(Box::new(move |m| if let Some(s) = m.sessions.get_mut(i) { s.weekday = d }))(cx)));
                        }
                        menu
                    }).small())
                    .child(start)
                    .child(div().text_color(t.secondary).child("–"))
                    .child(end)
                    .child(div().w(px(80.)).child(Input::new(&room).small()))
                    .child(minus(("srm", i), tr("Termin entfernen"), move |cx| e4(Box::new(move |m| if i < m.sessions.len() { m.sessions.remove(i); }))(cx)))
                    .into_any_element(),
            );
        }
        let (e, code_text) = (edit.clone(), m.code.clone());
        sessions.push(div().px(px(space::L)).py(px(space::S)).child(Button::new("session-add").ghost().small().icon(menu_icon("plus")).label(tr("Termin hinzufügen")).on_click(move |_, _, cx| {
            let c = code_text.clone();
            e(Box::new(move |m| m.sessions.push(Session { title: c.clone(), ..Default::default() })))(cx)
        })).into_any_element());
        self.fields = fields;
        div()
            .flex()
            .flex_col()
            .gap(px(space::XL))
            .child(section(Some(tr("Modul")), None, module_rows, t))
            .child(section(Some(tr("Links")), Some(tr("Der Upload-Link öffnet sich beim Hochladen einer Abgabe, daneben erscheinen die Dateien zum Hineinziehen.")), links, t))
            .child(section(Some(tr("Namensmuster")), Some(if c.naming_convention() { tr("Platzhalter: {code} {yy} {yyyy} {title} {author} {matrikel} {date} {date:yyyy-MM-dd} {w} {kw}") } else { tr("Gilt für alle Module. Eingeschaltet erscheint hier das Muster dieses Moduls.") }), naming_rows, t))
            .child(section(Some(tr("Wiederkehrende Deadlines")), Some(tr("Wird für jede Vorlesungswoche erzeugt. Eine Datei gehört dazu, wenn du sie dort ablegst, dafür eine Notiz anlegst oder sie zuordnest.")), rules, t))
            .child(section(Some(tr("Stundenplan")), Some(tr("Erscheint im 7-Tage-Ausblick der Übersicht, nicht in Pausen.")), sessions, t))
    }

    #[allow(clippy::too_many_arguments)]
    fn rule_card(&mut self, m: &Module, semester: Option<&Semester>, i: usize, r: &Rule, fields: &mut Fields, edit: impl Fn(Box<dyn Fn(&mut Module)>) -> Box<dyn Fn(&mut App)> + Clone + 'static, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> AnyElement {
        let expanded = self.expanded_rule.as_deref() == Some(r.id.as_str());
        let names = day::weekday_names();
        let wd = names[(r.weekday.clamp(1, 7) - 1) as usize];
        let when = match r.week_offset {
            0 => String::new(),
            1 => tr(" der Folgewoche"),
            n => trf(" +%lld Wochen", &[&n]),
        };
        let summary = if r.is_per_week() { format!("{wd}{when}, {} · {}", r.time, r.folder) } else { format!("{wd}{when}, {}", r.time) };
        let rid = r.id.clone();
        let header = div()
            .id(SharedString::from(format!("rule-{}", r.id)))
            .flex()
            .items_center()
            .gap(px(space::S))
            .rounded(px(radius::INNER))
            .cursor_pointer().key_nav()
            .soft(t.ink.opacity(0.045), radius::INNER)
            .child(motion::chevron("chevron", expanded, t.secondary))
            .child(div().flex_1().flex().flex_col().child(div().font_weight(FontWeight::MEDIUM).child(r.title.clone())).child(caption(summary, t)))
            .when(r.optional, |d| d.child(div().text_size(px(font::CAPTION2)).text_color(t.secondary).child(tr("optional"))))
            .on_click(cx.listener(move |s, _, _, cx| {
                s.expanded_rule = if s.expanded_rule.as_deref() == Some(rid.as_str()) { None } else { Some(rid.clone()) };
                cx.notify();
            }));
        let shell = div().flex().flex_col().px(px(space::L)).py(px(space::S)).child(header);
        let body_id = SharedString::from(format!("rule-body-{}", r.id));
        if !expanded {
            return shell.child(motion::reveal(body_id, false, div())).into_any_element();
        }
        let mut card = div().flex().flex_col().gap(px(space::S)).pt(px(space::S));
        let edit_rules = edit.clone();
        let set = move |f: Box<dyn Fn(&mut Rule)>| {
            let e = edit.clone();
            e(Box::new(move |m| if let Some(r) = m.rules.get_mut(i) { f(r) }))
        };
        let key = |s: &str| format!("{s}-{}-{}", m.id, r.id);
        let s1 = set.clone();
        let title = fields.input(&key("rt"), &r.title, &tr("Titel"), window, cx, move |v, cx| s1(Box::new(move |r| r.title = v.clone()))(cx));
        let s1 = set.clone();
        let short = fields.input(&key("rs"), &r.short, &tr("Kurzname"), window, cx, move |v, cx| s1(Box::new(move |r| r.short = v.clone()))(cx));
        let s1 = set.clone();
        let folder = fields.input(&key("rf"), &r.folder, "W{w}_KW{kw}/Abgabe", window, cx, move |v, cx| s1(Box::new(move |r| r.folder = v.clone()))(cx));
        let folder_open = r.is_per_week();
        let s1 = set.clone();
        let pattern = fields.input(&key("rp"), &r.file_pattern, &tr("* oder *.pdf"), window, cx, move |v, cx| s1(Box::new(move |r| r.file_pattern = v.clone()))(cx));
        let s1 = set.clone();
        let weeks_text = r.weeks.as_ref().map(|w| w.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        let weeks = fields.input(&key("rw"), &weeks_text, &tr("nur Wochen, z. B. 1, 3, 7"), window, cx, move |v, cx| {
            let list: Vec<u32> = v.split([',', ' ']).filter_map(|x| x.trim().parse().ok()).collect();
            s1(Box::new(move |r| r.weeks = (!list.is_empty()).then(|| list.clone())))(cx)
        });
        let s1 = set.clone();
        let keywords = fields.input(&key("rk"), &r.keywords.join(", "), &tr("z. B. Blatt, Sheet, Übungszettel"), window, cx, move |v, cx| {
            let k: Vec<String> = v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
            s1(Box::new(move |r| r.keywords = k.clone()))(cx)
        });
        let field = |label: String, content: AnyElement| div().flex().items_center().gap(px(space::M)).child(div().w(px(80.)).text_color(t.secondary).child(label)).child(div().flex_1().child(content));
        let (s1, s2, s3, s4) = (set.clone(), set.clone(), set.clone(), set.clone());
        let offset = r.week_offset;
        let offset_label = |o: i64| match o {
            0 => tr("in der Vorlesungswoche"),
            1 => tr("in der Folgewoche"),
            _ => tr("2 Wochen später"),
        };
        let example = (r.is_per_week())
            .then(|| semester.and_then(|s| schedule::lecture_weeks(s).first().copied()).and_then(|w| schedule::due_date(r, &w).map(|d| schedule::resolve(&r.folder, &w, d))))
            .flatten();
        card = card
            .child(field(tr("Titel"), div().flex().gap(px(space::S)).child(div().flex_1().child(Input::new(&title).small())).child(div().w(px(110.)).child(Input::new(&short).small())).into_any_element()))
            .child(field(
                tr("Fällig"),
                div()
                    .flex()
                    .gap(px(space::S))
                    .child(picker(("rwd", i), wd.to_string(), move |mut menu| {
                        for d in 1..=7u32 {
                            let s = s1.clone();
                            menu = menu.item(PopupMenuItem::new(names[(d - 1) as usize]).on_click(move |_, _, cx| s(Box::new(move |r| r.weekday = d))(cx)));
                        }
                        menu
                    }).small())
                    .child(fields.time(&format!("rtm-{}-{i}", m.id), &r.time, due_presets(), window, cx, move |v, cx| s2(Box::new(move |r| r.time = v.clone()))(cx)))
                    .child(picker(("roff", i), offset_label(offset), move |mut menu| {
                        for o in 0..=2i64 {
                            let s = s3.clone();
                            menu = menu.item(PopupMenuItem::new(offset_label(o)).checked(o == offset).on_click(move |_, _, cx| s(Box::new(move |r| r.week_offset = o))(cx)));
                        }
                        menu
                    }).small())
                    .into_any_element(),
            ))
            .when(folder_open, |c| c.child(field(tr("Wochenordner"), div().flex().flex_col().gap(px(space::XS)).child(Input::new(&folder).small()).when_some(example.clone(), |d, ex| d.child(div().font_family(Theme::mono_family()).text_size(px(font::CAPTION)).text_color(t.secondary).child(trf("z. B. %@/%@", &[&m.folder, &ex])))).into_any_element())))
            .child(field(String::new(), Switch::new(("ropt", i)).label(tr("Optional")).checked(r.optional).on_click(move |v, _, cx| {
                let v = *v;
                s4(Box::new(move |r| r.optional = v))(cx)
            }).into_any_element()));
        let adv_open = self.advanced.contains(&r.id);
        let rid = r.id.clone();
        card = card.child(
            link_button(SharedString::from(format!("adv-{}", r.id)), tr("Erweitert"), t)
                .flex()
                .items_center()
                .gap(px(space::XS))
                .child(motion::chevron("chevron", adv_open, t.accent))
                .on_click(cx.listener(move |s, _, _, cx| {
                    if !s.advanced.remove(&rid) {
                        s.advanced.insert(rid.clone());
                    }
                    cx.notify();
                })),
        );
        if adv_open {
            let (s1, s2, s3, s4, s5) = (set.clone(), set.clone(), set.clone(), set.clone(), set.clone());
            let interval = r.interval;
            let has_weeks = r.weeks.as_ref().is_some_and(|w| !w.is_empty());
            let interval_label = |n: u32| if n == 1 { tr("jede Woche") } else { trf("jede %lld. Woche", &[&n]) };
            card = card
                .when(!folder_open, |c| c.child(field(tr("Wochenordner"), div().flex().flex_col().gap(px(space::XS)).child(Input::new(&folder).small()).child(caption(tr("Nur wenn jede Woche einen eigenen Ordner hat, z. B. W{w}_KW{kw}/Hausuebung. Was darin liegt, zählt dann ohne Zuordnen."), t)).into_any_element())))
                .child(field(tr("Dateien"), div().w(px(140.)).child(Input::new(&pattern).small()).into_any_element()))
                .child(field(String::new(), Switch::new(("rup", i)).label(tr("Upload bestätigen")).checked(r.upload).tooltip(tr("Aus: zählt als erledigt, sobald eine Datei da ist")).on_click(move |v, _, cx| {
                    let v = *v;
                    s1(Box::new(move |r| r.upload = v))(cx)
                }).into_any_element()))
                .child(field(
                    tr("Rhythmus"),
                    div()
                        .flex()
                        .gap(px(space::M))
                        .child(picker(("rint", i), interval_label(interval), move |mut menu| {
                            for n in 1..=4u32 {
                                let s = s2.clone();
                                menu = menu.item(PopupMenuItem::new(interval_label(n)).checked(n == interval).on_click(move |_, _, cx| s(Box::new(move |r| r.interval = n))(cx)));
                            }
                            menu
                        }).small().disabled(has_weeks))
                        .child(div().w(px(180.)).child(Input::new(&weeks).small()))
                        .into_any_element(),
                ))
                .child(field(tr("Zeitraum"), div().flex().gap(px(space::L)).child(week_limit(("rsw", i), tr("ab"), r.start_week, move |v, cx| s3(Box::new(move |r| r.start_week = v))(cx), t)).child(week_limit(("rew", i), tr("bis"), r.end_week, move |v, cx| s4(Box::new(move |r| r.end_week = v))(cx), t)).into_any_element()))
                .child(field(
                    tr("Erkennung"),
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(space::S))
                        .child(Switch::new(("rrec", i)).label(tr("Auch Unterordner durchsuchen")).checked(r.recursive).on_click(move |v, _, cx| {
                            let v = *v;
                            s5(Box::new(move |r| r.recursive = v))(cx)
                        }))
                        .child(Input::new(&keywords).small())
                        .into_any_element(),
                ));
        }
        if let Some(s) = semester {
            let now = Local::now();
            let next: Vec<String> = schedule::lecture_weeks(s)
                .iter()
                .filter(|w| r.applies_to_week(w.index))
                .filter_map(|w| schedule::due_date(r, w))
                .filter(|d| s.break_at(day::local_date(*d)).is_none() && *d > now)
                .take(3)
                .map(|d| day::due(d, now))
                .collect();
            if !next.is_empty() {
                card = card.child(caption(trf("Nächste: %@", &[&next.join(" · ")]), t));
            }
        }
        let e = edit_rules;
        let card = card.child(div().flex().justify_end().child(Button::new(("rdel", i)).ghost().danger().small().label(tr("Regel löschen")).on_click(move |_, _, cx| e(Box::new(move |m| if i < m.rules.len() { m.rules.remove(i); }))(cx))));
        shell.child(motion::reveal(body_id, true, card)).into_any_element()
    }
}

fn due_presets() -> Vec<Preset> {
    ["08:00", "12:00", "14:00", "18:00", "23:59"].map(Preset::plain).to_vec()
}

fn start_presets() -> Vec<Preset> {
    ["08:00", "09:45", "11:30", "13:30", "15:15", "17:00"].map(Preset::plain).to_vec()
}

fn end_presets(start: &str) -> Vec<Preset> {
    let Some(from) = clock::parse(start) else { return Vec::new() };
    [45, 90, 120].iter().map(|&minutes| (minutes, from + chrono::Duration::minutes(minutes))).filter(|(_, to)| *to > from).map(|(minutes, to)| {
        let time = clock::format(to);
        Preset { label: trf("%@ · %@ Min.", &[&time, &minutes.to_string()]), time }
    }).collect()
}

fn week_limit(id: impl Into<ElementId>, label: String, value: Option<u32>, set: impl Fn(Option<u32>, &mut App) + 'static, t: &Theme) -> Div {
    let set = std::rc::Rc::new(set);
    let id: ElementId = id.into();
    let (s1, s2, s3) = (set.clone(), set.clone(), set);
    div()
        .flex()
        .items_center()
        .gap(px(space::XS))
        .child(Checkbox::new(id.clone()).label(label).checked(value.is_some()).on_click(move |v, _, cx| s1(if *v { Some(1) } else { None }, cx)))
        .when_some(value, |d, v| {
            d.child(Button::new(ElementId::Name(format!("{id:?}-dec").into())).ghost().small().icon(menu_icon("minus")).tooltip(tr("Weniger")).on_click(move |_, _, cx| s2(Some(v.saturating_sub(1).max(1)), cx)))
                .child(div().w(px(32.)).text_center().text_color(t.primary).child(format!("W{v:02}")))
                .child(Button::new(ElementId::Name(format!("{id:?}-inc").into())).ghost().small().icon(menu_icon("plus")).tooltip(tr("Mehr")).on_click(move |_, _, cx| s3(Some((v + 1).min(30)), cx)))
        })
}

impl Settings {
    fn sync_tab(&mut self, cx: &mut Context<Self>, t: &Theme) -> Div {
        use super::sync;
        use crate::core::prefs::GitMode;
        let on = sync::settings(cx).is_some_and(|g| g.enabled) && sync::get(cx).git.is_some();
        if !on {
            let has_root = state::read(cx, |l| l.root.is_some());
            return div().w_full().flex().flex_col().gap(px(space::L)).child(section(
                None,
                Some(tr("Sichert deine Dateien über Git und gleicht sie mit anderen Geräten ab. Zugangsdaten bleiben bei Git, Schrumbi speichert keine.")),
                vec![row(tr("Sync"), Button::new("s-setup").primary().label(tr("Einrichten…")).disabled(!has_root).on_click(|_, _, cx| act(cx, |l| l.show_sync_setup = true)), t)],
                t,
            ));
        }
        let g = sync::settings(cx).unwrap_or_default();
        let s = sync::get(cx);
        let status = s.status.clone().unwrap_or_default();
        let remote = s.git.as_ref().and_then(|git| git.run(&["remote", "get-url", "origin"]).ok()).map(|u| u.trim().to_string());
        let (exe, version) = (s.git.as_ref().map(|git| git.exe.to_string_lossy().to_string()).unwrap_or_default(), s.git.as_ref().and_then(|git| git.run(&["--version"]).ok()).map(|v| v.trim().to_string()).unwrap_or_default());
        let excluded: Vec<String> = s.nested.iter().chain(s.too_large.iter()).cloned().collect();
        let mode_label = |m: GitMode| if m == GitMode::Auto { tr("Automatisch") } else { tr("Manuell") };
        let idle = g.idle_minutes;
        let automatic = g.mode == GitMode::Auto;
        let mut sections = div().w_full().flex().flex_col().gap(px(space::L)).child(section(
            Some(tr("Status")),
            None,
            vec![
                row(tr("Online-Speicher"), caption(remote.unwrap_or_else(|| tr("Keiner")), t).truncate(), t),
                row(tr("Branch"), caption(status.branch.unwrap_or_else(|| tr("Kein Branch")), t), t),
                row(tr("Letzter Stand"), caption(super::sync_panel::last_sync_text(cx), t), t),
                row(tr("Jetzt synchronisieren"), Button::new("s-now").label(tr("Synchronisieren")).on_click(|_, _, cx| sync::run(None, true, cx)), t),
            ],
            t,
        ));
        sections = sections.child(section(
            Some(tr("Automatik")),
            Some(if automatic { tr("Sichert im Hintergrund und meldet sich nur bei Konflikten oder Anmeldeproblemen.") } else { tr("Es wird nur gesichert, wenn du es auslöst.") }),
            vec![
                row(tr("Wann sichern"), picker("s-mode", mode_label(g.mode), move |m| {
                    m.item(PopupMenuItem::new(tr("Automatisch")).checked(automatic).on_click(|_, _, cx| sync::update_settings(cx, |g| g.mode = GitMode::Auto)))
                        .item(PopupMenuItem::new(tr("Manuell")).checked(!automatic).on_click(|_, _, cx| sync::update_settings(cx, |g| g.mode = GitMode::Manual)))
                }), t),
                row(tr("Nach Ruhe"), picker("s-idle", trf("%@ Min.", &[&idle.to_string()]), move |mut m| {
                    for minutes in [2u32, 5, 15] {
                        m = m.item(PopupMenuItem::new(trf("%@ Min.", &[&minutes.to_string()])).checked(idle == minutes).on_click(move |_, _, cx| sync::update_settings(cx, |g| g.idle_minutes = minutes)));
                    }
                    m
                }).disabled(!automatic), t),
                row(tr("Beim Öffnen synchronisieren"), Switch::new("s-open").checked(g.sync_on_open).disabled(!automatic).on_click(|v, _, cx| {
                    let v = *v;
                    sync::update_settings(cx, |g| g.sync_on_open = v);
                }), t),
                row(tr("Beim Beenden synchronisieren"), Switch::new("s-quit").checked(g.sync_on_quit).disabled(!automatic).on_click(|v, _, cx| {
                    let v = *v;
                    sync::update_settings(cx, |g| g.sync_on_quit = v);
                }), t),
            ],
            t,
        ));
        if !excluded.is_empty() {
            sections = sections.child(section(Some(tr("Ausgeschlossen")), Some(tr("Eigene Repositories und zu große Dateien bleiben nur auf diesem Gerät.")), excluded.into_iter().map(|e| row(e, div(), t)).collect(), t));
        }
        sections.child(section(
            Some(tr("Erweitert")),
            Some(tr("Ausschalten beendet nur den Abgleich. Das Repository und dein Verlauf bleiben unangetastet.")),
            vec![
                row(tr("Git-Pfad"), caption(exe, t).truncate(), t),
                row(tr("Version"), caption(version, t), t),
                row(tr("Einrichtung erneut prüfen"), Button::new("s-resetup").label(tr("Prüfen…")).on_click(|_, _, cx| act(cx, |l| l.show_sync_setup = true)), t),
                row(tr("Sync ausschalten"), Button::new("s-off").danger().label(tr("Ausschalten")).on_click(|_, _, cx| sync::disable(cx)), t),
            ],
            t,
        ))
    }

    fn obsidian_section(&mut self, fields: &mut Fields, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        use crate::core::vault::config::{self, LinkFormat, VaultConfig};
        let (root, external, modules) = state::read(cx, |l| (l.root.clone(), l.external_vault(), l.config.as_ref().map_or(Vec::new(), |c| c.modules.clone())));
        let Some(root) = root else { return div() };
        let own = root.join(".obsidian").is_dir();
        if own || external.is_some() {
            let vault = external.clone().unwrap_or_else(|| root.clone());
            let read = VaultConfig::read(&vault);
            let format = match read.new_link_format {
                LinkFormat::Shortest => tr("Kürzester Pfad"),
                LinkFormat::Relative => tr("Relativer Pfad"),
                LinkFormat::Absolute => tr("Absoluter Pfad"),
            };
            let style = if read.use_markdown_links { "[text](pfad)" } else { "[[pfad]]" };
            let value = |s: String| div().text_color(t.secondary).child(s).into_any_element();
            let title = if own { tr("Dieser Ordner ist ein Obsidian-Vault") } else { trf("Verbunden mit %@", &[&vault.display().to_string()]) };
            let mut rows = vec![
                row(title, icon("circle-check", iconsize::M, t.green), t),
                row(tr("Anhangsordner"), value(read.attachment_folder_path.clone()), t),
                row(tr("Linkformat"), value(format!("{format} · {style}")), t),
                row(tr("Vorlagen"), value(if read.templates.folder.is_empty() { tr("Kein Ordner") } else { read.templates.folder.clone() }), t),
            ];
            if external.is_some() {
                for module in modules {
                    let id = module.id.clone();
                    let input = fields.input(&format!("vault-{id}"), module.notes_folder.as_deref().unwrap_or(""), &tr("Ordner im Vault"), window, cx, move |v, cx| {
                        let id = id.clone();
                        act(cx, |l| {
                            let Some(mut updated) = l.config.clone() else { return };
                            if let Some(m) = updated.modules.iter_mut().find(|m| m.id == id) {
                                m.notes_folder = (!v.trim().is_empty()).then(|| v.trim().to_string());
                            }
                            l.save_config(updated);
                        });
                    });
                    rows.push(row(module.code.clone(), div().w(px(200.)).child(Input::new(&input).small()), t));
                }
                rows.push(row(tr("Verbindung"), Button::new("vault-disconnect").small().ghost().label(tr("Trennen")).on_click(|_, _, cx| {
                    act(cx, |l| l.connect_vault(None));
                    state::watch_vault(cx);
                    state::reindex(cx);
                }), t));
            }
            return section(Some(tr("Obsidian")), Some(tr("Diese Werte kommen aus Obsidian und lassen sich dort ändern.")), rows, t);
        }
        let connect = |path: std::path::PathBuf, cx: &mut App| {
            act(cx, |l| l.connect_vault(Some(path)));
            state::watch_vault(cx);
            state::reindex(cx);
        };
        let known = config::known_vaults();
        let picker = Button::new("vault-pick").small().label(tr("Vault verbinden…")).dropdown_menu(move |mut menu, _, _| {
            for vault in &known {
                let path = vault.path.clone();
                menu = menu.item(PopupMenuItem::new(vault.name.clone()).on_click(move |_, _, cx| connect(path.clone(), cx)));
            }
            menu.separator().item(PopupMenuItem::new(tr("Ordner wählen…")).on_click(move |_, _, cx| super::platform::choose_folder(cx, move |path, cx| connect(path, cx))))
        });
        section(Some(tr("Obsidian")), Some(tr("Optional. Ein Vault außerhalb dieses Ordners wird mitdurchsucht und synchronisiert. Schrumbi verändert seine Einstellungen nicht.")), vec![row(tr("Vault"), picker, t)], t)
    }

    fn notes(&mut self, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let p = prefs::get();
        let mut fields = std::mem::take(&mut self.fields);
        let accent = fields.input("n-accent", &p.note_accent, "#0EA5E9", window, cx, |v, _| {
            if v.len() == 7 && v.starts_with('#') && u32::from_str_radix(&v[1..], 16).is_ok() {
                prefs::update(|p| p.note_accent = v.to_uppercase());
            }
        });
        let size = p.editor_font_size;
        let mode = p.note_save_mode;
        let delay = p.note_save_delay;
        let mode_label = |m: NoteSaveMode| match m {
            NoteSaveMode::Focus => tr("Beim Fokuswechsel"),
            NoteSaveMode::Delay => tr("Nach einer Tipp-Pause"),
        };
        let save_footer = match mode {
            NoteSaveMode::Focus => tr("Gesichert wird, sobald du den Editor, das Fenster oder die App verlässt oder den Tab wechselst. ⌘S sichert sofort."),
            NoteSaveMode::Delay => tr("Zusätzlich wird nach der Pause automatisch gesichert. ⌘S sichert sofort."),
        } + " " + &tr("Schließen sichert immer.");
        let stepper = |id: &'static str, label: String, dec: Box<dyn Fn(&mut App)>, inc: Box<dyn Fn(&mut App)>| {
            div()
                .flex()
                .items_center()
                .gap(px(space::S))
                .child(Button::new(SharedString::from(format!("{id}-dec"))).small().icon(menu_icon("minus")).tooltip(tr("Weniger")).on_click(move |_, _, cx| dec(cx)))
                .child(div().w(px(110.)).text_center().child(label))
                .child(Button::new(SharedString::from(format!("{id}-inc"))).small().icon(menu_icon("plus")).tooltip(tr("Mehr")).on_click(move |_, _, cx| inc(cx)))
        };
        let me = cx.entity();
        let markdown_row = match self.markdown_default {
            Some(true) => row(super::platform::markdown_label(), div().flex().items_center().gap(px(space::S)).text_color(t.secondary).child(icon("circle-check", iconsize::S, t.green)).child(tr("Öffnet in Schrumbi")), t),
            Some(false) => row(
                super::platform::markdown_label(),
                Button::new("n-default").small().label(tr("Schrumbi als Standard festlegen")).on_click(move |_, _, cx| {
                    super::platform::set_markdown_default();
                    me.update(cx, |s, cx| {
                        s.markdown_default = super::platform::markdown_is_default();
                        cx.notify();
                    });
                }),
                t,
            ),
            None => div().into_any_element(),
        };
        let editor_kind = prefs::get().note_editor;
        let editor_label = |k: prefs::NoteEditor| if k == prefs::NoteEditor::Live { tr("Live-Ansicht: Formatierung beim Schreiben sichtbar") } else { tr("Quelltext: Text mit Zeilennummern") };
        let editor_row = row(
            tr("Notizen schreiben"),
            picker("n-editor", editor_label(editor_kind), move |mut menu| {
                for k in [prefs::NoteEditor::Code, prefs::NoteEditor::Live] {
                    menu = menu.item(PopupMenuItem::new(editor_label(k)).checked(k == editor_kind).on_click(move |_, _, cx| {
                        prefs::update(|p| {
                            p.note_editor = k;
                            p.note_editor_chosen = true;
                        });
                        super::app::rebuild_menus(cx);
                        cx.refresh_windows();
                    }));
                }
                menu
            }),
            t,
        );
        let editor = section(
            Some(tr("Editor")),
            Some(tr("Doppelklick auf .md-Dateien in Schrumbi öffnet immer den eigenen Editor.")),
            vec![editor_row, markdown_row, row(tr("Rechtschreibung prüfen"), Switch::new("n-spell").checked(p.spell_check).on_click(|v, _, cx| {
                let on = *v;
                prefs::update(|p| p.spell_check = on);
                cx.refresh_windows();
            }), t), row(
                tr("Schriftgröße"),
                stepper(
                    "n-size",
                    format!("{} pt", size as i32),
                    Box::new(move |cx| super::documents::set_font_size((size - 1.).max(10.), cx)),
                    Box::new(move |cx| super::documents::set_font_size((size + 1.).min(24.), cx)),
                ),
                t,
            ), row(tr("Text in voller Breite"), Switch::new("n-full-width").checked(p.reader_full_width).on_click(|v, _, cx| super::documents::set_reader_full_width(*v, cx)), t)],
            t,
        );
        let mut save_rows = vec![row(
            tr("Sichern"),
            picker("n-mode", mode_label(mode), move |mut menu| {
                for m in [NoteSaveMode::Focus, NoteSaveMode::Delay] {
                    menu = menu.item(PopupMenuItem::new(mode_label(m)).checked(m == mode).on_click(move |_, _, _| prefs::update(|p| p.note_save_mode = m)));
                }
                menu
            }),
            t,
        )];
        if mode == NoteSaveMode::Delay {
            save_rows.push(row(
                tr("Pause"),
                stepper(
                    "n-delay",
                    trf("%lld s ohne Eingabe", &[&(delay as i64)]),
                    Box::new(move |_| prefs::update(|p| p.note_save_delay = (delay - 1.).max(1.))),
                    Box::new(move |_| prefs::update(|p| p.note_save_delay = (delay + 1.).min(60.))),
                ),
                t,
            ));
        }
        let current = p.note_accent.to_uppercase();
        let mut swatches = div().flex().items_center().gap(px(space::S));
        for (hex, name) in ACCENTS {
            let selected = current == hex;
            let color: Hsla = rgb(u32::from_str_radix(&hex[1..], 16).unwrap_or(0)).into();
            swatches = swatches.child(
                div()
                    .id(SharedString::from(format!("acc-{hex}")))
                    .size(px(control::S))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer().key_nav()
                    .hover(|s| s.opacity(0.75))
                    .when(selected, |d| d.border_2().border_color(t.primary))
                    .child(div().size(px(iconsize::M)).rounded_full().bg(color))
                    .tooltip(tip(tr(name)))
                    .on_click(move |_, _, _| prefs::update(|p| p.note_accent = hex.into())),
            );
        }
        swatches = swatches.child(div().w(px(90.)).child(Input::new(&accent).small()));
        let pdf = section(
            Some(tr("PDF-Export")),
            Some(trf("Notizen werden als A4-PDF neben der Markdown-Datei gesichert (%@ im Notizfenster).", &[&ShortcutAction::ExportPdf.active_shortcut().map_or_else(String::new, |s| s.display())])),
            vec![
                row(tr("Akzentfarbe"), swatches, t),
                row(tr("Kopfzeile mit Modul, Woche, Name und Datum"), Switch::new("n-header").checked(p.pdf_header).on_click(|v, _, _| {
                    let v = *v;
                    prefs::update(|p| p.pdf_header = v);
                }), t),
                row(tr("Eigenschaften über dem Text"), Switch::new("n-properties").checked(p.pdf_properties).on_click(|v, _, _| {
                    let v = *v;
                    prefs::update(|p| p.pdf_properties = v);
                }), t),
                row(tr("Seitenzahlen"), Switch::new("n-pages").checked(p.pdf_page_numbers).on_click(|v, _, _| {
                    let v = *v;
                    prefs::update(|p| p.pdf_page_numbers = v);
                }), t),
                row(tr("Vor dem Ersetzen eines früheren Exports fragen"), Switch::new("n-replace-ask").checked(p.pdf_replace_ask).on_click(|v, _, _| {
                    let v = *v;
                    prefs::update(|p| p.pdf_replace_ask = v);
                }), t),
                div()
                    .flex()
                    .px(px(space::L))
                    .py(px(space::S))
                    .child(Button::new("n-reset").ghost().label(tr("Standard wiederherstellen")).on_click(|_, _, _| {
                        prefs::update(|p| {
                            p.note_accent = prefs::DEFAULT_ACCENT.into();
                            p.pdf_header = true;
                            p.pdf_page_numbers = true;
                        })
                    }))
                    .child(div().flex_1())
                    .child(Button::new("n-sample").label(tr("Beispiel-PDF ansehen")).on_click(|_, _, cx| super::pdf::sample(cx)))
                    .into_any_element(),
            ],
            t,
        );
        let obsidian = self.obsidian_section(&mut fields, window, cx, t);
        self.fields = fields;
        let dictionary = self.dictionary_section(&p.spell_words, t);
        div().flex().flex_col().gap(px(space::XL)).child(editor).child(dictionary).child(section(Some(tr("Sichern")), Some(save_footer), save_rows, t)).child(pdf).child(obsidian)
    }

    fn dictionary_section(&self, words: &[String], t: &Theme) -> Div {
        let rows = if words.is_empty() {
            vec![div().px(px(space::L)).py(px(space::M)).text_color(t.secondary).child(tr("Noch keine Wörter")).into_any_element()]
        } else {
            words
                .iter()
                .enumerate()
                .map(|(i, word)| {
                    let gone = word.clone();
                    row(
                        word.clone(),
                        Button::new(("dict-remove", i)).ghost().small().icon(menu_icon("x")).tooltip(tr("Aus dem Wörterbuch entfernen")).on_click(move |_, _, cx| super::spell_card::remove_word(&gone, cx)),
                        t,
                    )
                })
                .collect()
        };
        section(Some(tr("Eigenes Wörterbuch")), Some(tr("Diese Wörter markiert die Rechtschreibung nicht. Hinzufügen: in einer Notiz auf ein unterstrichenes Wort zeigen.")), rows, t)
    }

    fn ai_tab(&mut self, cx: &mut Context<Self>, t: &Theme) -> Div {
        use super::ai::{self, Registration};
        use crate::core::ai::detect::{self, Agent};
        let prefs = prefs::with(|p| p.ai.clone());
        let found = detect::found();
        let names = found.iter().map(|c| c.agent.label()).collect::<Vec<_>>().join(", ");
        let rescan = Button::new("ai-rescan").label(tr("Erneut suchen")).on_click(|_, _, cx| ai::rescan(cx));
        let programs = div().flex().items_center().gap(px(space::M)).child(caption(if found.is_empty() { tr("Keins gefunden") } else { names }, t)).child(rescan);
        let mut sections = div().w_full().flex().flex_col().gap(px(space::L)).child(section(
            None,
            Some(if found.is_empty() {
                tr("Schrumbi arbeitet mit Claude Code, Codex oder OpenCode zusammen, sobald eins davon installiert ist.")
            } else {
                tr("Das Programm nutzt dein eigenes Konto dort; was du ihm zeigst, geht an diesen Anbieter. Lesen darf es in deinem Studienordner, ändern nur mit deinem Ja, und jede Änderung ist rückgängig.")
            }),
            vec![
                row(tr("KI-Assistent verwenden"), Switch::new("ai-on").checked(prefs.enabled).on_click(|v, _, cx| ai::set_enabled(*v, cx)), t),
                row(tr("Programme"), programs, t),
                row(tr("Knopf in der Werkzeugleiste"), Switch::new("ai-toolbar").checked(prefs.toolbar_button).disabled(!prefs.enabled).on_click(|v, _, cx| {
                    let v = *v;
                    prefs::update(|p| p.ai.toolbar_button = v);
                    cx.refresh_windows();
                }), t),
            ],
            t,
        ));
        if !ai::active() {
            return sections;
        }
        let Some(exe) = detect::exe() else { return sections };
        let mut rows = Vec::new();
        for cli in found {
            let text = detect::register_text(cli.agent, &exe);
            let copy_text = text.clone();
            let copy = Button::new(SharedString::from(format!("ai-copy-{:?}", cli.agent))).ghost().icon(menu_icon("copy")).tooltip(tr("Kopieren")).on_click(move |_, _, cx| super::drive::copy(copy_text.clone(), cx));
            let mut controls = div().flex().items_center().gap(px(space::S)).child(copy);
            let mut detail = text;
            if detect::register_args(cli.agent, &exe).is_some() {
                let state = ai::registration(cli.agent, cx);
                if let Some(Registration::Failed(e)) = &state {
                    detail = e.clone();
                }
                let label = match state {
                    Some(Registration::Running) => tr("Trägt ein …"),
                    Some(Registration::Done) => tr("Eingetragen"),
                    Some(Registration::Failed(_)) => tr("Erneut versuchen"),
                    None => tr("Eintragen"),
                };
                let busy = state == Some(Registration::Running);
                let target = cli.clone();
                controls = controls.child(Button::new(SharedString::from(format!("ai-register-{:?}", cli.agent))).label(label).loading(busy).disabled(busy || state == Some(Registration::Done)).on_click(move |_, _, cx| ai::register(target.clone(), cx)));
            } else if cli.agent == Agent::OpenCode {
                detail = format!("{} {detail}", tr("In opencode.json ergänzen:"));
            }
            rows.push(detail_row(cli.agent.label(), detail, controls, t));
        }
        let server = detect::server_text(&exe);
        let server_copy = server.clone();
        rows.push(detail_row(tr("Befehl"), server, Button::new("ai-copy-server").ghost().icon(menu_icon("copy")).tooltip(tr("Kopieren")).on_click(move |_, _, cx| super::drive::copy(server_copy.clone(), cx)), t));
        let root = state::read(cx, |l| l.root.clone());
        rows.push(row(tr("Im Terminal öffnen"), Button::new("ai-terminal").label(tr("Öffnen")).disabled(root.is_none()).on_click(move |_, _, _| {
            if let Some(root) = root.clone()
                && !crate::core::drive::fenced("terminal", root.display())
            {
                let _ = detect::open_terminal(&root);
            }
        }), t));
        sections = sections.child(section(Some(tr("Für die Kommandozeile")), Some(tr("Danach kann dein Programm im Terminal deine Notizen, PDFs und Fristen nutzen, solange Schrumbi läuft. Jede Änderung fragt hier zuerst.")), rows, t));
        sections
    }

    fn cards_tab(&mut self, t: &Theme) -> Div {
        let p = prefs::get();
        let (retention, new, reviews) = (p.card_retention, p.card_new_per_day, p.card_reviews_per_day);
        let step = |id: &'static str, value: String, dec: Box<dyn Fn(&mut App)>, inc: Box<dyn Fn(&mut App)>| {
            div()
                .flex()
                .items_center()
                .gap(px(space::S))
                .child(Button::new(SharedString::from(format!("{id}-dec"))).small().icon(menu_icon("minus")).tooltip(tr("Weniger")).on_click(move |_, _, cx| {
                    dec(cx);
                    cx.refresh_windows();
                }))
                .child(div().w(px(64.)).text_center().font_features(tabular()).child(value))
                .child(Button::new(SharedString::from(format!("{id}-inc"))).small().icon(menu_icon("plus")).tooltip(tr("Mehr")).on_click(move |_, _, cx| {
                    inc(cx);
                    cx.refresh_windows();
                }))
        };
        let learning = section(
            Some(tr("Lernen")),
            Some(tr("Geplant wird mit FSRS-6, dem aus Millionen echter Wiederholungen ermittelten Verfahren, das auch Anki nutzt. Ein höheres Ziel heißt mehr Wiederholungen: Über 95 % kostet viel Zeit und bringt wenig.")),
            vec![
                row(tr("Erinnerungsziel"), step("c-retention", format!("{:.0} %", retention * 100.), Box::new(move |_| prefs::update(|p| p.card_retention = (retention - 0.01).max(0.7))), Box::new(move |_| prefs::update(|p| p.card_retention = (retention + 0.01).min(0.97)))), t),
                row(tr("Neue Karten pro Tag"), step("c-new", new.to_string(), Box::new(move |_| prefs::update(|p| p.card_new_per_day = new.saturating_sub(5))), Box::new(move |_| prefs::update(|p| p.card_new_per_day = new + 5))), t),
                row(tr("Wiederholungen pro Tag"), step("c-reviews", reviews.to_string(), Box::new(move |_| prefs::update(|p| p.card_reviews_per_day = reviews.saturating_sub(50))), Box::new(move |_| prefs::update(|p| p.card_reviews_per_day = reviews + 50))), t),
            ],
            t,
        );
        let example = |text: &str, meaning: String| div().flex().items_center().gap(px(space::M)).px(px(space::L)).py(px(space::S)).child(div().flex_1().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(text.to_string())).child(div().flex_none().text_color(t.secondary).child(meaning)).into_any_element();
        let syntax = section(
            Some(tr("Karten schreiben")),
            Some(tr("In Notizen mit #flashcards oder #karten zählen auch {{Lücke}} und ==Markiertes== als Lücken. cards: false in den Eigenschaften nimmt eine Notiz aus. Code und Formeln ergeben nie eine Karte.")),
            vec![example("Frage :: Antwort", tr("Eine Karte")), example("Begriff ::: Erklärung", tr("In beide Richtungen")), example("Lange Frage\n?\nAntwort", tr("Über mehrere Zeilen")), example("Der {{c1::Satz}} gilt", tr("Lückentext"))],
            t,
        );
        div().flex().flex_col().gap(px(space::XL)).child(learning).child(syntax)
    }

    /// Whether a shortcut is being recorded and its key button, which only records while it has the focus.
    fn armed(&self, window: &Window) -> bool {
        self.recording.and_then(|a| self.key_buttons.get(a.id())).is_some_and(|h| h.is_focused(window))
    }

    fn rejected_is_reserved(&self) -> bool {
        matches!(self.rejected, Some(Rejected::Reserved(_)))
    }

    fn key_button(&mut self, a: ShortcutAction, cx: &mut Context<Self>) -> FocusHandle {
        self.key_buttons.entry(a.id()).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

    fn close_notice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.notice = None;
        let back = self.last_changed.and_then(|a| self.key_buttons.get(a.id())).unwrap_or(&self.focus).clone();
        window.focus(&back, cx);
        cx.notify();
    }

    /// Offers to take the change back unless the shortcuts are as they were; the latest offer replaces the earlier one.
    fn offer_undo(&mut self, before: Customized, text: String, cx: &mut Context<Self>) {
        if ShortcutAction::snapshot() == before {
            return;
        }
        self.notices += 1;
        let id = self.notices;
        self.notice = Some(Notice { id, text, before });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(motion::NOTICE).await;
            let _ = this.update(cx, |s, cx| {
                if s.notice.as_ref().is_some_and(|n| n.id == id) {
                    s.notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn start_recording(&mut self, a: ShortcutAction, toggle: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = if toggle && self.recording == Some(a) { None } else { Some(a) };
        self.rejected = None;
        let handle = self.key_button(a, cx);
        handle.focus(window, cx);
        cx.notify();
    }

    fn shortcuts(&mut self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        let query = self.filter.read(cx).value().trim().to_string();
        let filter = query.to_lowercase();
        let any_custom = ShortcutAction::any_customized();
        let head = div()
            .flex()
            .items_center()
            .gap(px(space::S))
            .child(div().flex_1().child(Input::new(&self.filter).prefix(icon("search", iconsize::S, t.secondary)).cleanable(true)))
            .child(Button::new("sc-reset-all").label(tr("Alle zurücksetzen")).disabled(!any_custom).on_click(cx.listener(|s, _, _, cx| {
                let before = ShortcutAction::reset_all();
                s.last_changed = None;
                s.recording = None;
                s.rejected = None;
                super::app::bind_keys(cx);
                s.offer_undo(before, tr("Alle Kürzel zurückgesetzt"), cx);
                cx.notify();
            })));
        let (escape, delete) = (Shortcut { key: "escape".into(), command: false, shift: false, option: false, control: false }.display(), Shortcut { key: "delete".into(), command: false, shift: false, option: false, control: false }.display());
        let line: AnyElement = match &self.notice {
            Some(n) => div()
                .id("sc-notice")
                .aria_label(n.text.clone())
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(space::S))
                .text_size(px(font::CAPTION))
                .text_color(t.secondary)
                .child(div().flex_none().child(icon("check", iconsize::XS, t.green_text)))
                .child(div().id("sc-notice-text").min_w_0().truncate().tooltip(tip(n.text.clone())).child(n.text.clone()))
                .child(div().flex_none().text_color(t.tertiary).child("·"))
                .child(div().flex_none().child(link_button("sc-undo", tr("Rückgängig"), t).on_click(cx.listener(|s, _, _, cx| {
                    if let Some(n) = s.notice.take() {
                        ShortcutAction::restore(n.before);
                        s.recording = None;
                        s.rejected = None;
                        super::app::bind_keys(cx);
                    }
                    cx.notify();
                }))))
                .child(Button::new("sc-notice-close").ghost().small().icon(menu_icon("x")).tooltip(format!("{} ({})", tr("Schließen"), escape)).on_click(cx.listener(|s, _, window, cx| s.close_notice(window, cx))))
                .into_any_element(),
            None => {
                let idle = trf("Klick auf ein Kürzel und drück die neuen Tasten. %@ bricht ab, %@ entfernt das Kürzel.", &[&escape, &delete]);
                caption(idle.clone(), t).id("sc-idle").w_full().min_w_0().truncate().tooltip(tip(idle)).into_any_element()
            }
        };
        let header = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .px(px(space::XL))
            .pt(px(space::XL))
            .pb(px(space::M))
            .border_b_1()
            .border_color(t.separator)
            .child(head)
            .child(div().h(px(control::S)).w_full().min_w_0().flex().items_center().child(line));
        let mut col = div().flex().flex_col().gap(px(space::XL));
        let modules = module_names(cx);
        let all_clashes = ShortcutAction::all_conflicts();
        let mut shown = false;
        for g in Group::ALL {
            let actions: Vec<ShortcutAction> = ShortcutAction::ALL.into_iter().filter(|a| a.group() == g && (filter.is_empty() || shortcut_fits(*a, &modules, &filter))).collect();
            if actions.is_empty() {
                continue;
            }
            shown = true;
            let mut rows: Vec<AnyElement> = Vec::new();
            for a in actions {
                let mut clashes: Vec<String> = all_clashes.get(&a).map(|c| c.iter().map(|o| o.named(&modules)).collect()).unwrap_or_default();
                clashes.extend(a.fixed_conflicts());
                let recording = self.recording == Some(a);
                let customized = a.is_customized();
                let rejected = self.rejected.clone().filter(|_| recording);
                let handle = self.key_button(a, cx);
                let blocked_why = a.blocked();
                let blocked = blocked_why.as_ref().map(|why| trf("%@ – inaktiv", &[why]));
                let reason_chip: Option<String> = rejected.map(|why| match why {
                    Rejected::NeedsModifier if cfg!(target_os = "macos") => tr("Dazu ⌘, ⌥ oder ⌃ halten"),
                    Rejected::NeedsModifier => trf("Dazu %@ oder Alt halten", &[&ctrl_name()]),
                    Rejected::Reserved(why) => why,
                });
                let reason: Option<String> = reason_chip.as_ref().map(|chip| if self.rejected_is_reserved() { trf("%@ – andere Tasten drücken", &[chip]) } else { chip.clone() });
                let note = |text: String| div().min_w_0().truncate().text_size(px(font::CAPTION)).text_color(t.orange_text).child(text).into_any_element();
                let quiet = |text: String| {
                    div().min_w_0().flex().px(px(space::XS)).rounded(px(radius::SMALL)).bg(t.ink.opacity(0.04)).border_1().border_color(t.ink.opacity(0.08)).text_size(px(font::CAPTION)).text_color(t.secondary).child(div().min_w_0().truncate().child(text)).child(div().flex_none().whitespace_nowrap().ml(px(space::XS)).child(tr("– inaktiv"))).into_any_element()
                };
                let dimmed = blocked.is_some() && reason.is_none() && !recording;
                let name = a.named(&modules);
                let state = if let Some(text) = reason.clone().or(blocked.clone()) {
                    text
                } else if recording {
                    tr("Tasten drücken…")
                } else {
                    a.shortcut().map_or_else(|| tr("Kein Kürzel"), |s| s.display())
                };
                let caps: AnyElement = if let Some(text) = reason_chip.clone() {
                    note(text)
                } else if recording {
                    div().text_size(px(font::CALLOUT)).text_color(t.accent_text).child(tr("Tasten drücken…")).into_any_element()
                } else if let Some(text) = blocked_why.clone() {
                    quiet(text)
                } else if let Some(s) = a.shortcut() {
                    div().flex().flex_none().gap(px(space::XS)).children(Shortcut::parts(&s).into_iter().map(|c| div().min_w(px(iconsize::M)).h(px(iconsize::M)).px(px(space::XS)).flex().items_center().justify_center().rounded(px(radius::SMALL)).bg(t.ink.opacity(0.07)).border_1().border_color(t.ink.opacity(0.12)).text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).child(c))).into_any_element()
                } else {
                    div().text_size(px(font::CALLOUT)).text_color(t.tertiary).child(tr("Kein Kürzel")).into_any_element()
                };
                let hover = t.ink.opacity(0.06);
                let help = reason.or(blocked).unwrap_or_else(|| if recording { trf("%@ bricht ab, %@ entfernt das Kürzel", &[&escape, &delete]) } else { tr("Klicken zum Ändern") });
                let help = if clashes.is_empty() { help } else { format!("{help}\n{}", trf("Auch belegt: %@", &[&clashes.join(", ")])) };
                let recorder = div()
                    .id(SharedString::from(format!("rec-{}", a.id())))
                    .track_focus(&handle)
                    .w(px(control::KEYS))
                    .h(px(control::M))
                    .px(px(space::S))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .rounded(px(radius::CONTROL))
                    .border(px(theme::ring::BORDER))
                    .border_color(if recording { t.accent } else { gpui::transparent_black() })
                    .when(recording, |d| d.bg(t.accent.opacity(theme::ring::ARMED)))
                    .cursor_pointer()
                    .key_nav()
                    .soft(hover, radius::CONTROL)
                    .aria_label(trf("%@: %@", &[&name, &state]))
                    .when(!clashes.is_empty(), |d| d.aria_description(format!("{} · {}", tr("Doppelt"), trf("Auch belegt: %@", &[&clashes.join(", ")]))))
                    .child(caps)
                    .tooltip(tip(help))
                    .on_click(cx.listener(move |s, _, window, cx| s.start_recording(a, true, window, cx)));
                let default = a.default_shortcut().map_or_else(|| tr("Kein Kürzel"), |d| d.display());
                let clash = (!clashes.is_empty()).then(|| {
                    let full = trf("Auch belegt: %@", &[&clashes.join(", ")]);
                    div()
                        .id(SharedString::from(format!("dup-{}", a.id())))
                        .flex()
                        .min_w_0()
                        .max_w(px(control::CLASH))
                        .items_center()
                        .gap(px(space::XS))
                        .rounded(px(radius::SMALL))
                        .text_size(px(font::CAPTION))
                        .text_color(t.orange_text)
                        .aria_label(format!("{} · {}", tr("Doppelt"), full))
                        .child(icon("triangle-alert", iconsize::XS, t.orange_text))
                        .child(div().flex_none().child(tr("Doppelt")))
                        .child(div().min_w_0().truncate().child(format!("· {}", clashes.join(", "))))
                        .tooltip(tip(full))
                });
                rows.push(
                    div()
                        .flex()
                        .items_center()
                        .min_h(px(control::M + space::S * 2.))
                        .child(
                            div()
                                .id(SharedString::from(format!("lbl-{}", a.id())))
                                .flex_1()
                                .min_w_0()
                                .self_stretch()
                                .flex()
                                .items_center()
                                .gap(px(space::S))
                                .pl(px(space::L))
                                .pr(px(space::S))
                                .cursor_pointer()
                                .soft(hover, radius::SMALL)
                                .on_click(cx.listener(move |s, _, window, cx| s.start_recording(a, false, window, cx)))
                                .child(div().min_w_0().truncate().when(dimmed, |d| d.text_color(t.secondary)).child(a.named(&modules)))
                                .children(clash),
                        )
                        .child(recorder)
                        .child(div().flex_none().w(px(control::ICON_BUTTON + space::S + space::L)).pl(px(space::S)).pr(px(space::L)).when(customized, |d| {
                            d.child(Button::new(SharedString::from(format!("rst-{}", a.id()))).ghost().small().icon(menu_icon("rotate-ccw")).tooltip(trf("Zurücksetzen auf %@", &[&default])).on_click(cx.listener(move |s, _, _, cx| {
                                let before = ShortcutAction::snapshot();
                                s.last_changed = Some(a);
                                a.reset();
                                super::app::bind_keys(cx);
                                let text = changed(a, cx);
                                s.offer_undo(before, text, cx);
                                cx.notify();
                            })))
                        }))
                        .into_any_element(),
                );
            }
            col = col.child(section(Some(g.label()), None, rows, t));
        }
        let scroller = div().id("sc-scroll").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.page).p(px(space::XL));
        if !shown {
            let filter_field = self.filter.clone();
            col = col.child(
                unavailable("search", trf("Keine Treffer für „%@“", &[&query]), "", t)
                    .h(px(240.))
                    .child(Button::new("sc-clear-filter").label(tr("Filter zurücksetzen")).on_click(cx.listener(move |_, _, window, cx| filter_field.update(cx, |f, cx| f.set_value("", window, cx))))),
            );
        }
        div().size_full().flex().flex_col().child(header).child(scroller.child(col))
    }

    fn record_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) {
        let Some(action) = self.recording else { return };
        let m = &k.modifiers;
        let plain = !m.platform && !m.alt && !m.control && !m.shift;
        let key = match k.key.as_str() {
            "enter" => "return".to_string(),
            "backspace" => "delete".to_string(),
            "space" => "space".to_string(),
            other => other.to_string(),
        };
        if ["shift", "control", "alt", "platform", "function"].contains(&key.as_str()) {
            return;
        }
        if key == "escape" && plain {
            self.recording = None;
        } else if key == "delete" && plain {
            let before = ShortcutAction::snapshot();
            action.set(None);
            self.last_changed = Some(action);
            self.recording = None;
            super::app::bind_keys(cx);
            let text = changed(action, cx);
            self.offer_undo(before, text, cx);
        } else {
            let mac = cfg!(target_os = "macos");
            let s = Shortcut { key, command: if mac { m.platform } else { m.control }, shift: m.shift, option: m.alt, control: if mac { m.control } else { false } };
            if let Some(why) = action.reserved_for(&s) {
                self.rejected = Some(Rejected::Reserved(why));
                cx.notify();
                return;
            }
            let function = s.key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n));
            if s.command || s.option || s.control || function {
                let before = ShortcutAction::snapshot();
                action.set(Some(s));
                self.last_changed = Some(action);
                self.recording = None;
                super::app::bind_keys(cx);
                let text = changed(action, cx);
                self.offer_undo(before, text, cx);
            } else {
                self.rejected = Some(Rejected::NeedsModifier);
                cx.notify();
                return;
            }
        }
        self.rejected = None;
        cx.notify();
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        super::perf::frame_start();
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        if self.recording.is_some() && !self.armed(window) {
            self.recording = None;
            self.rejected = None;
        }
        if cx.try_global::<OpenOnSync>().is_some_and(|o| o.0) {
            cx.set_global(OpenOnSync(false));
            self.tab = Tab::Sync;
        }
        if let Some(label) = cx.try_global::<OpenLit>().and_then(|o| o.0.clone()) {
            cx.set_global(OpenLit(None));
            if let Some((_, tab)) = SEARCHABLE.iter().find(|(k, _)| *k == label) {
                self.tab = *tab;
                self.search.update(cx, |s, cx| s.set_value("", window, cx));
                self.light_up(tr(&label), window, cx);
            }
        }
        if cx.try_global::<OpenOnAi>().is_some_and(|o| o.0) {
            cx.set_global(OpenOnAi(false));
            self.tab = Tab::Ai;
        }
        let t = Theme::get(cx);
        let config = state::read(cx, |l| l.config.clone());
        let tabs = self.tab_bar(&t, cx);
        let searching = !self.search.read(cx).value().trim().is_empty();
        let page = self.page.clone();
        let body: AnyElement = match (config, self.tab) {
            _ if searching => self.search_results(&t, cx),
            (_, Tab::Notes) => div().id("notes-scroll").size_full().overflow_y_scroll().track_scroll(&page).p(px(space::XL)).child(self.notes(window, cx, &t)).into_any_element(),
            (_, Tab::Cards) => div().id("cards-scroll").size_full().overflow_y_scroll().track_scroll(&page).p(px(space::XL)).child(self.cards_tab(&t)).into_any_element(),
            (_, Tab::Sync) => div().id("sync-scroll").size_full().overflow_y_scroll().track_scroll(&page).p(px(space::XL)).child(self.sync_tab(cx, &t)).into_any_element(),
            (_, Tab::Ai) => div().id("ai-scroll").size_full().overflow_y_scroll().track_scroll(&page).p(px(space::XL)).child(self.ai_tab(cx, &t)).into_any_element(),
            (_, Tab::Shortcuts) => self.shortcuts(&t, window, cx).into_any_element(),
            (None, _) => unavailable("folder", tr("Kein Ordner gewählt"), tr("Wähle zuerst im Hauptfenster deinen Studienordner."), &t).into_any_element(),
            (Some(c), Tab::General) => div().id("general-scroll").size_full().overflow_y_scroll().track_scroll(&page).p(px(space::XL)).child(self.general(&c, window, cx, &t)).into_any_element(),
            (Some(c), Tab::Semester) => self.semesters(&c, window, cx, &t).into_any_element(),
            (Some(c), Tab::Modules) => self.modules(&c, window, cx, &t).into_any_element(),
        };
        let mut fields = std::mem::take(&mut self.fields);
        fields.prune();
        self.fields = fields;
        let content = div()
            .id("settings")
            .track_focus(&self.focus)
            .on_action(|_: &super::app::CloseWindow, window, _| window.remove_window())
            .on_action(|_: &super::app::CloseTab, window, _| window.remove_window())
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, window, cx| {
                if s.recording.is_none() && e.keystroke.key == "escape" {
                    let filter_focused = s.filter.read(cx).focus_handle(cx).is_focused(window);
                    if filter_focused && !s.filter.read(cx).value().is_empty() {
                        s.filter.update(cx, |f, cx| f.set_value("", window, cx));
                    } else if window.context_stack().iter().any(|c| c.contains("Input")) {
                        window.focus(&s.focus, cx);
                    } else if s.tab == Tab::Shortcuts && s.notice.is_some() {
                        s.close_notice(window, cx);
                    } else {
                        window.remove_window();
                    }
                    cx.stop_propagation();
                    return;
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(t.content)
            .font_family(Theme::font_family())
            .text_size(px(font::BODY))
            .line_height(relative(1.22))
            .text_color(t.primary)
            .capture_key_down(cx.listener(|s, e: &KeyDownEvent, window, cx| {
                if !s.search.read(cx).value().trim().is_empty() {
                    match e.keystroke.key.as_str() {
                        "down" => s.hit = (s.hit + 1).min(s.matches(cx).len().saturating_sub(1)),
                        "up" => s.hit = s.hit.saturating_sub(1),
                        "escape" => s.search.update(cx, |f, cx| f.set_value("", window, cx)),
                        _ => return,
                    }
                    cx.stop_propagation();
                    cx.notify();
                } else if e.keystroke.key == "f" && e.keystroke.modifiers.secondary(cx) {
                    let field = if s.tab == Tab::Shortcuts { s.filter.clone() } else { s.search.clone() };
                    field.update(cx, |f, cx| f.focus(window, cx));
                    cx.stop_propagation();
                }
            }))
            .child(tabs)
            .child(div().flex_1().min_h_0().child(body));
        super::chrome::frame(tr("Einstellungen"), content, &t)
    }
}

/// Opens the settings on the sync tab.
pub fn open_sync(cx: &mut App) {
    open(cx);
    cx.set_global(OpenOnSync(true));
    cx.refresh_windows();
}

/// Opens the settings at the row or section with this German label, lit up for a moment.
pub fn open_lit(label: &str, cx: &mut App) {
    open(cx);
    cx.set_global(OpenLit(Some(label.to_string())));
    cx.refresh_windows();
}

/// Asks the settings window to switch to the sync tab on its next frame.
struct OpenOnSync(bool);

impl Global for OpenOnSync {}

/// Asks the settings window to jump to a labelled setting on its next frame.
struct OpenLit(Option<String>);

impl Global for OpenLit {}

/// Opens the settings on the assistant's tab.
pub fn open_ai(cx: &mut App) {
    open(cx);
    cx.set_global(OpenOnAi(true));
    cx.refresh_windows();
}

struct OpenOnAi(bool);

impl Global for OpenOnAi {}

#[cfg(test)]
mod tests {
    use super::{SEARCHABLE, SYNONYMS};

    #[test]
    fn every_setting_can_be_found_by_search() {
        let source = include_str!("settings.rs");
        let label = regex::Regex::new(r#"(?:row\(\s*tr\(|section\(\s*Some\(tr\()"([^"]+)"\)"#).unwrap();
        let missing: Vec<String> = label.captures_iter(source).map(|c| c[1].to_string()).filter(|l| !SEARCHABLE.iter().any(|(k, _)| k == l)).collect();
        assert!(missing.is_empty(), "not searchable: {missing:?}");
        for (word, target) in SYNONYMS {
            assert!(SEARCHABLE.iter().any(|(k, _)| k == target), "{word} points nowhere");
        }
    }
}
