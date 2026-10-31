use std::collections::HashSet;

use chrono::{Days, Local};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::fields::Fields;
use super::state::{self, act, edit_config};
use super::motion::{self, Soft};
use super::theme::{self, Theme, font, radius, space, iconsize, control};
use super::widgets::*;
use crate::core::text::fold;
use crate::core::i18n::{tr, trf};
use crate::core::ics;
use crate::core::model::*;
use crate::core::naming::Naming;
use crate::core::prefs::{self, NoteSaveMode, Shortcut};
use crate::core::shortcuts::{Group, ShortcutAction};
use crate::core::{day, schedule};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    General,
    Semester,
    Modules,
    Notes,
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

/// Every labelled setting and the tab it lives on, for the search field.
const SEARCHABLE: &[(&str, Tab)] = &[
    ("Studium", Tab::General),
    ("Name", Tab::General),
    ("Kürzel in Dateinamen", Tab::General),
    ("Inbox-Ordner", Tab::General),
    ("Matrikelnummer", Tab::General),
    ("Mitautor:innen", Tab::General),
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
    ("System", Tab::General),
    ("Bei Anmeldung starten", Tab::General),
    ("Automatisch nach Updates suchen", Tab::General),
    ("Symbol in der Menüleiste", Tab::General),
    ("Seitenleiste", Tab::General),
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
    ("Dateinamen", Tab::Modules),
    ("Wiederkehrende Deadlines", Tab::Modules),
    ("Stundenplan", Tab::Modules),
    ("Editor", Tab::Notes),
    ("Schriftgröße", Tab::Notes),
    ("Sichern", Tab::Notes),
    ("Pause", Tab::Notes),
    ("PDF-Export", Tab::Notes),
    ("Akzentfarbe", Tab::Notes),
    ("Kopfzeile mit Modul, Woche, Name und Datum", Tab::Notes),
    ("Seitenzahlen", Tab::Notes),
];

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
    recording: Option<ShortcutAction>,
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
                window.activate_window();
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
    let _ = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| {
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
    let _ = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| About));
}

struct About;

impl Render for About {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                let _ = opener::open(env!("CARGO_PKG_REPOSITORY"));
            }))
            .child(div().text_size(px(font::CAPTION)).text_color(t.tertiary).child("Lucide (ISC) · Material Icon Theme (MIT)"));
        super::chrome::frame(tr("Über Schrumbi"), content, &t)
    }
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
        .when_some(title, |d, s| d.child(div().px(px(space::XS)).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(s)))
        .child(g)
        .when_some(footer, |d, f| d.child(div().px(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.secondary).child(f)))
}

fn row(label: impl Into<SharedString>, control: impl IntoElement, t: &Theme) -> AnyElement {
    div().flex().items_center().gap(px(space::M)).px(px(space::L)).min_h(px(40.)).py(px(space::S)).child(div().flex_1().min_w_0().text_color(t.primary).child(label.into())).child(div().flex_none().max_w(px(460.)).child(control)).into_any_element()
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

fn minus(id: impl Into<ElementId>, help: String, f: impl Fn(&mut App) + 'static) -> Button {
    Button::new(id).ghost().small().icon(menu_icon("circle-minus")).tooltip(help).on_click(move |_, _, cx| f(cx))
}

fn time_options() -> Vec<String> {
    (0..24).flat_map(|h| [0, 15, 30, 45].map(move |m| format!("{h:02}:{m:02}"))).chain(["23:59".to_string()]).collect()
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

impl Settings {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Befehl suchen")));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Suchen")));
        let lib = state::lib(cx);
        let subs = vec![cx.observe(&lib, |_, _, cx| cx.notify()), cx.subscribe(&filter, |_, _, _: &InputEvent, cx| cx.notify()), cx.subscribe_in(&search, window, |s, _, e: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = e
                && let Some(tab) = s.matches(cx).first().map(|(_, tab)| *tab)
            {
                s.open_result(tab, window, cx);
            }
            cx.notify();
        })];
        let (semester, module) = state::read(cx, |l| (l.config.as_ref().and_then(|c| c.current_semester(l.now)).map(|s| s.id.clone()), l.config.as_ref().and_then(|c| c.modules.first()).map(|m| m.id.clone())));
        Self {
            tab: Tab::General,
            fields: Fields::default(),
            semester,
            module,
            expanded_rule: None,
            advanced: HashSet::new(),
            filter,
            search,
            recording: None,
            focus: cx.focus_handle(),
            launch_at_login: super::platform::launches_at_login(),
            markdown_default: super::platform::markdown_is_default(),
            _subs: subs,
        }
    }

    fn matches(&self, cx: &App) -> Vec<(String, Tab)> {
        let query = fold(self.search.read(cx).value().trim());
        if query.is_empty() {
            return Vec::new();
        }
        SEARCHABLE.iter().map(|(key, tab)| (tr(key), *tab)).filter(|(label, _)| fold(label).contains(&query)).collect()
    }

    fn show_module(&mut self, id: String) {
        self.tab = Tab::Modules;
        self.module = Some(id);
    }

    fn open_result(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = tab;
        self.recording = None;
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    fn search_results(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let results = self.matches(cx);
        if results.is_empty() {
            return unavailable("search", tr("Keine Treffer"), "", t).into_any_element();
        }
        let tab_name = |tab: Tab| match tab {
            Tab::General => tr("Allgemein"),
            Tab::Semester => tr("Semester"),
            Tab::Modules => tr("Module"),
            Tab::Notes => tr("Notizen"),
            Tab::Shortcuts => tr("Tastenkürzel"),
        };
        let mut list = div().w_full().max_w(px(560.)).flex().flex_col().gap(px(space::XXS));
        for (i, (label, tab)) in results.into_iter().enumerate() {
            list = list.child(
                row_button(("settings-result", i), t)
                    .flex()
                    .items_center()
                    .px(px(space::M))
                    .py(px(space::S))
                    .rounded(px(radius::INNER))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(caption(tab_name(tab), t))
                    .on_click(cx.listener(move |s, _, window, cx| s.open_result(tab, window, cx))),
            );
        }
        div().id("settings-results").size_full().overflow_y_scrollbar().p(px(space::XL)).flex().justify_center().child(list).into_any_element()
    }

    fn tab_bar(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let mut bar = div().flex().justify_center().gap(px(space::XS)).pt(px(if super::chrome::CUSTOM { space::M } else { space::XXL })).pb(px(space::S)).border_b_1().border_color(t.separator);
        for (tab, label, icon_name) in [
            (Tab::General, tr("Allgemein"), "settings"),
            (Tab::Semester, tr("Semester"), "calendar"),
            (Tab::Modules, tr("Module"), "library"),
            (Tab::Notes, tr("Notizen"), "square-pen"),
            (Tab::Shortcuts, tr("Tastenkürzel"), "command"),
        ] {
            let active = tab == self.tab;
            let hover = t.ink.opacity(0.06);
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("tab-{label}")))
                    .w(px(84.))
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
        div().relative().child(bar).child(div().absolute().top(px(if super::chrome::CUSTOM { space::M } else { space::XXL })).right(px(space::M)).w(px(130.)).child(Input::new(&self.search).small().prefix(icon("search", iconsize::XS, t.secondary))))
    }

    fn general(&mut self, c: &StudyConfig, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let mut f = std::mem::take(&mut self.fields);
        let text = |f: &mut Fields, key: &str, value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>, set: fn(&mut StudyConfig, String)| f.input(key, value, placeholder, window, cx, move |v, cx| edit_config(cx, |c| set(c, v)));
        let name = text(&mut f, "g-name", &c.name, "", window, cx, |c, v| c.name = v);
        let author = text(&mut f, "g-author", &c.author, &tr("Nachname-V"), window, cx, |c, v| c.author = v);
        let inbox = text(&mut f, "g-inbox", &c.inbox, "", window, cx, |c, v| if is_inner_folder(&v) { c.inbox = v });
        let matrikel = text(&mut f, "g-matrikel", &c.matrikel, &tr("für {matrikel} in Dateinamen"), window, cx, |c, v| c.matrikel = v);
        let credit = text(&mut f, "g-credit", &c.credit_label, &tr("ECTS, CP, Credits"), window, cx, |c, v| c.credit_label = v);
        let exclude = text(&mut f, "g-exclude", &c.search_exclude.join(", "), "", window, cx, |c, v| c.search_exclude = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
        self.fields = f;
        let p = prefs::get();
        let (root, index_count, config_path, exporting) = state::read(cx, |l| (l.root.clone(), l.index_count, l.config_path(), l.is_exporting));
        let style = c.coauthor_style;
        let grading = c.grading;

        let studium = section(
            Some(tr("Studium")),
            None,
            vec![
                row(tr("Name"), div().w(px(300.)).child(Input::new(&name)), t),
                row(tr("Kürzel in Dateinamen"), div().w(px(300.)).child(Input::new(&author)), t),
                row(tr("Inbox-Ordner"), div().w(px(300.)).child(Input::new(&inbox)), t),
                row(tr("Matrikelnummer"), div().w(px(300.)).child(Input::new(&matrikel)), t),
                row(
                    tr("Mitautor:innen"),
                    picker("g-coauthor", if style == CoauthorStyle::Initial { tr("Nachname-V (Mueller-C)") } else { tr("Frei (beliebiger Name)") }, move |m| {
                        m.item(PopupMenuItem::new(tr("Nachname-V (Mueller-C)")).checked(style == CoauthorStyle::Initial).on_click(|_, _, cx| edit_config(cx, |c| c.coauthor_style = CoauthorStyle::Initial)))
                            .item(PopupMenuItem::new(tr("Frei (beliebiger Name)")).checked(style == CoauthorStyle::Free).on_click(|_, _, cx| edit_config(cx, |c| c.coauthor_style = CoauthorStyle::Free)))
                    }),
                    t,
                ),
            ],
            t,
        );
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
                                state::close_workspace(cx);
                                state::open_workspace(&p, cx);
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
        let language = section(
            None,
            None,
            vec![row(
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
        let calendar = section(
            None,
            Some(tr("Hält die Datei „Schrumbi-Kalender.ics“ im Studienordner aktuell. Abonniere sie in Apple Kalender, Outlook oder Thunderbird, dann erscheinen die Deadlines dort und auf dem Handy.")),
            vec![row(tr("Kalenderdatei aktuell halten"), Switch::new("g-cal").checked(p.calendar_feed).on_click(|v, _, cx| {
                let v = *v;
                prefs::update(|p| p.calendar_feed = v);
                state::sync_reminders(cx);
            }), t)],
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
            [("semester", tr("Semester")), ("record", tr("Leistungen")), ("inbox", tr("Inbox")), ("search", tr("Suche")), ("checks", tr("Aufräumen"))]
                .into_iter()
                .map(|(key, label)| {
                    let shown = !hidden.iter().any(|k| k == key);
                    row(label, Switch::new(SharedString::from(format!("g-tab-{key}"))).checked(shown).on_click(move |v, _, cx| super::sidebar::set_hidden(key, !*v, cx)), t)
                })
                .collect(),
            t,
        );
        div().flex().flex_col().gap(px(space::XL)).child(studium).child(grades).child(search).child(sidebar).child(storage).child(language).child(reminders).child(calendar).child(system)
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
    use gpui_kit::component::WindowExt;
    let ok = std::rc::Rc::new(ok);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let ok = ok.clone();
        alert.title(title.clone()).description(message.clone()).show_cancel(true).on_ok(move |_, _, cx| {
            ok(cx);
            true
        })
    });
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
                            let _ = opener::open(u);
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
        let naming_rows = vec![row(tr("Muster"), div().w(px(320.)).child(Input::new(&naming)), t), row(tr("Beispiel"), div().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(example), t)];
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
            menu.item(PopupMenuItem::new(tr("Abgabe in Wochenordner (Mo 18:00, Folgewoche)")).on_click(add(base("Hausübung", "HÜ", 1, "18:00", 1, "W{w}_KW{kw}/Hausuebung"))))
                .item(PopupMenuItem::new(tr("Abgabe in Sammelordner (Mo 10:00)")).on_click(add(base("Abgabe", "Abgabe", 1, "10:00", 0, "Abgaben"))))
                .item(PopupMenuItem::new(tr("Übung am selben Tag (optional, 22:00)")).on_click(add(Rule { optional: true, ..base("Tagesübung", "TÜ", 3, "22:00", 0, "W{w}_KW{kw}/Tagesuebung") })))
                .item(PopupMenuItem::new(tr("Wöchentliche Aufgabe zum Abhaken")).on_click(add(Rule { upload: false, file_pattern: "-".into(), ..base("Wochenaufgabe", "Aufgabe", 5, "18:00", 0, "") })))
                .separator()
                .item(PopupMenuItem::new(tr("Leere Regel")).on_click(add(base("Abgabe", "Abgabe", 1, "18:00", 0, "Abgaben"))))
        })).into_any_element());
        let mut sessions: Vec<AnyElement> = Vec::new();
        let names = day::weekday_names();
        for (i, s) in m.sessions.iter().enumerate() {
            let e = edit.clone();
            let title = fields.input(&format!("st-{}-{i}", m.id), &s.title, &tr("Vorlesung"), window, cx, move |v, cx| e(Box::new(move |m| if let Some(s) = m.sessions.get_mut(i) { s.title = v.clone() }))(cx));
            let e = edit.clone();
            let room = fields.input(&format!("sr-{}-{i}", m.id), &s.location, &tr("Raum"), window, cx, move |v, cx| e(Box::new(move |m| if let Some(s) = m.sessions.get_mut(i) { s.location = v.clone() }))(cx));
            let (e1, e2, e3, e4) = (edit.clone(), edit.clone(), edit.clone(), edit.clone());
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
                    .child(time_picker(("sf", i), s.start.clone(), move |v, cx| e2(Box::new(move |m| if let Some(s) = m.sessions.get_mut(i) { s.start = v.clone() }))(cx)))
                    .child(div().text_color(t.secondary).child("–"))
                    .child(time_picker(("se", i), s.end.clone(), move |v, cx| e3(Box::new(move |m| if let Some(s) = m.sessions.get_mut(i) { s.end = v.clone() }))(cx)))
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
            .child(section(Some(tr("Dateinamen")), Some(tr("Platzhalter: {code} {yy} {yyyy} {title} {author} {matrikel} {date} {date:yyyy-MM-dd} {w} {kw}")), naming_rows, t))
            .child(section(Some(tr("Wiederkehrende Deadlines")), Some(tr("Wird für jede Vorlesungswoche erzeugt. Ordner-Platzhalter: {w} Vorlesungswoche, {kw} Kalenderwoche. Ohne Platzhalter zählt eine Datei, deren Datum in den 7 Tagen vor der Deadline liegt.")), rules, t))
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
        let summary = format!("{wd}{when}, {} · {}", r.time, if r.folder.is_empty() { m.folder.clone() } else { r.folder.clone() });
        let rid = r.id.clone();
        let header = div()
            .id(SharedString::from(format!("rule-{}", r.id)))
            .flex()
            .items_center()
            .gap(px(space::S))
            .cursor_pointer().key_nav()
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
                    .child(time_picker(("rtm", i), r.time.clone(), move |v, cx| s2(Box::new(move |r| r.time = v.clone()))(cx)))
                    .child(picker(("roff", i), offset_label(offset), move |mut menu| {
                        for o in 0..=2i64 {
                            let s = s3.clone();
                            menu = menu.item(PopupMenuItem::new(offset_label(o)).checked(o == offset).on_click(move |_, _, cx| s(Box::new(move |r| r.week_offset = o))(cx)));
                        }
                        menu
                    }).small())
                    .into_any_element(),
            ))
            .child(field(tr("Ordner"), div().flex().flex_col().gap(px(space::XS)).child(Input::new(&folder).small()).when_some(example, |d, ex| d.child(div().font_family(Theme::mono_family()).text_size(px(font::CAPTION)).text_color(t.secondary).child(trf("z. B. %@/%@", &[&m.folder, &ex])))).into_any_element()))
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

fn time_picker(id: impl Into<ElementId>, value: String, set: impl Fn(String, &mut App) + 'static) -> Picker {
    let set = std::rc::Rc::new(set);
    picker(id, value.clone(), move |mut menu| {
        menu = menu.max_h(px(320.)).scrollable(true);
        for t in time_options() {
            let s = set.clone();
            let v = t.clone();
            menu = menu.item(PopupMenuItem::new(t.clone()).checked(t == value).on_click(move |_, _, cx| s(v.clone(), cx)));
        }
        menu
    })
    .small()
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
    fn notes(&mut self, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let p = prefs::get();
        let mut fields = std::mem::take(&mut self.fields);
        let accent = fields.input("n-accent", &p.note_accent, "#0EA5E9", window, cx, |v, _| {
            if v.len() == 7 && v.starts_with('#') && u32::from_str_radix(&v[1..], 16).is_ok() {
                prefs::update(|p| p.note_accent = v.to_uppercase());
            }
        });
        self.fields = fields;
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
        let editor = section(
            Some(tr("Editor")),
            Some(tr("Doppelklick auf .md-Dateien in Schrumbi öffnet immer den eigenen Editor.")),
            vec![markdown_row, row(
                tr("Schriftgröße"),
                stepper(
                    "n-size",
                    format!("{} pt", size as i32),
                    Box::new(move |cx| super::documents::set_font_size((size - 1.).max(10.), cx)),
                    Box::new(move |cx| super::documents::set_font_size((size + 1.).min(22.), cx)),
                ),
                t,
            )],
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
                    .when(selected, |d| d.border_2().border_color(t.primary))
                    .child(div().size(px(iconsize::M)).rounded_full().bg(color))
                    .tooltip(tip(tr(name)))
                    .on_click(move |_, _, _| prefs::update(|p| p.note_accent = hex.into())),
            );
        }
        swatches = swatches.child(div().w(px(90.)).child(Input::new(&accent).small()));
        let pdf = section(
            Some(tr("PDF-Export")),
            Some(trf("Notizen werden als A4-PDF neben der Markdown-Datei gesichert (%@ im Notizfenster).", &[&ShortcutAction::ExportPdf.shortcut().map_or_else(String::new, |s| s.display())])),
            vec![
                row(tr("Akzentfarbe"), swatches, t),
                row(tr("Kopfzeile mit Modul, Woche, Name und Datum"), Switch::new("n-header").checked(p.pdf_header).on_click(|v, _, _| {
                    let v = *v;
                    prefs::update(|p| p.pdf_header = v);
                }), t),
                row(tr("Seitenzahlen"), Switch::new("n-pages").checked(p.pdf_page_numbers).on_click(|v, _, _| {
                    let v = *v;
                    prefs::update(|p| p.pdf_page_numbers = v);
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
        div().flex().flex_col().gap(px(space::XL)).child(editor).child(section(Some(tr("Sichern")), Some(save_footer), save_rows, t)).child(pdf)
    }

    fn shortcuts(&mut self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let filter = self.filter.read(cx).value().to_lowercase();
        let any_custom = ShortcutAction::ALL.iter().any(|a| a.is_customized());
        let head = div()
            .flex()
            .items_center()
            .gap(px(space::S))
            .child(div().flex_1().child(Input::new(&self.filter).prefix(icon("search", iconsize::S, t.secondary)).cleanable(true)))
            .child(Button::new("sc-reset-all").label(tr("Alle zurücksetzen")).disabled(!any_custom).on_click(cx.listener(|_, _, _, cx| {
                ShortcutAction::reset_all();
                super::app::bind_keys(cx);
                cx.notify();
            })));
        let mut col = div().flex().flex_col().gap(px(space::XL)).child(div().flex().flex_col().gap(px(space::S)).child(head).child(caption(tr("Klick auf ein Kürzel und drück die neuen Tasten. ⎋ bricht ab, ⌫ entfernt das Kürzel."), t)));
        for g in Group::ALL {
            let actions: Vec<ShortcutAction> = ShortcutAction::ALL.into_iter().filter(|a| a.group() == g && (filter.is_empty() || a.label().to_lowercase().contains(&filter))).collect();
            if actions.is_empty() {
                continue;
            }
            let mut rows: Vec<AnyElement> = Vec::new();
            for a in actions {
                let conflicts = a.conflicts();
                let recording = self.recording == Some(a);
                let customized = a.is_customized();
                let caps: AnyElement = if recording {
                    div().text_size(px(font::CALLOUT)).text_color(t.accent_text).child(tr("Tasten drücken …")).into_any_element()
                } else if let Some(s) = a.shortcut() {
                    div().flex().gap(px(space::XS)).children(Shortcut::parts(&s).into_iter().map(|c| div().min_w(px(iconsize::M)).h(px(iconsize::M)).px(px(space::XS)).flex().items_center().justify_center().rounded(px(radius::SMALL)).bg(t.ink.opacity(0.07)).border_1().border_color(t.ink.opacity(0.12)).text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).child(c))).into_any_element()
                } else {
                    div().text_size(px(font::CALLOUT)).text_color(t.tertiary).child(tr("Kein Kürzel")).into_any_element()
                };
                let hover = t.ink.opacity(0.06);
                let recorder = div()
                    .id(SharedString::from(format!("rec-{}", a.id())))
                    .min_w(px(120.))
                    .h(px(control::M))
                    .px(px(space::S))
                    .flex()
                    .items_center()
                    .justify_end()
                    .rounded(px(radius::CONTROL))
                    .cursor_pointer().key_nav()
                    .soft(hover, radius::CONTROL)
                    .when(recording, |d| d.border(px(1.5)).border_color(t.accent))
                    .child(caps)
                    .tooltip(tip(if recording { tr("⎋ bricht ab, ⌫ entfernt das Kürzel") } else { tr("Klicken zum Ändern") }))
                    .on_click(cx.listener(move |s, _, window, cx| {
                        s.recording = if s.recording == Some(a) { None } else { Some(a) };
                        s.focus.focus(window, cx);
                        cx.notify();
                    }));
                let default = a.default_shortcut().display();
                rows.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .px(px(space::L))
                        .py(px(space::S))
                        .child(div().flex_1().child(a.label()))
                        .when(!conflicts.is_empty(), |d| {
                            d.child(div().id(SharedString::from(format!("dup-{}", a.id()))).flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.orange).child(icon("triangle-alert", iconsize::XS, t.orange)).child(tr("Doppelt")).tooltip(tip(trf("Auch belegt: %@", &[&conflicts.iter().map(|c| c.label()).collect::<Vec<_>>().join(", ")]))))
                        })
                        .child(recorder)
                        .child(div().w(px(26.)).when(customized, |d| {
                            d.child(Button::new(SharedString::from(format!("rst-{}", a.id()))).ghost().small().icon(menu_icon("rotate-ccw")).tooltip(trf("Zurücksetzen auf %@", &[&default])).on_click(cx.listener(move |_, _, _, cx| {
                                a.reset();
                                super::app::bind_keys(cx);
                                cx.notify();
                            })))
                        }))
                        .into_any_element(),
                );
            }
            col = col.child(section(Some(g.label()), None, rows, t));
        }
        col
    }

    fn record_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(action) = self.recording else { return };
        let k = &e.keystroke;
        let m = &k.modifiers;
        let plain = !m.platform && !m.alt && !m.control && !m.shift;
        let key = match k.key.as_str() {
            "enter" => "return".to_string(),
            "backspace" => "delete".to_string(),
            "space" => "space".to_string(),
            other => other.to_string(),
        };
        if key == "escape" && plain {
            self.recording = None;
        } else if key == "delete" && plain {
            action.set(None);
            self.recording = None;
            super::app::bind_keys(cx);
        } else if ["shift", "control", "alt", "platform", "function"].contains(&key.as_str()) {
            return;
        } else {
            let mac = cfg!(target_os = "macos");
            let s = Shortcut { key, command: if mac { m.platform } else { m.control }, shift: m.shift, option: m.alt, control: if mac { m.control } else { false } };
            let special = ["up", "down", "left", "right", "tab", "return"].contains(&s.key.as_str());
            if s.command || s.option || s.control || (special && s.shift) {
                action.set(Some(s));
                self.recording = None;
                super::app::bind_keys(cx);
            } else {
                return;
            }
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        let t = Theme::get(cx);
        let config = state::read(cx, |l| l.config.clone());
        let tabs = self.tab_bar(&t, cx);
        let searching = !self.search.read(cx).value().trim().is_empty();
        let body: AnyElement = match (config, self.tab) {
            _ if searching => self.search_results(&t, cx),
            (_, Tab::Notes) => div().id("notes-scroll").size_full().overflow_y_scrollbar().p(px(space::XL)).child(self.notes(window, cx, &t)).into_any_element(),
            (_, Tab::Shortcuts) => div().id("sc-scroll").size_full().overflow_y_scrollbar().p(px(space::XL)).child(self.shortcuts(&t, cx)).into_any_element(),
            (None, _) => unavailable("folder", tr("Kein Ordner gewählt"), tr("Wähle zuerst im Hauptfenster deinen Studienordner."), &t).into_any_element(),
            (Some(c), Tab::General) => div().id("general-scroll").size_full().overflow_y_scrollbar().p(px(space::XL)).child(self.general(&c, window, cx, &t)).into_any_element(),
            (Some(c), Tab::Semester) => self.semesters(&c, window, cx, &t).into_any_element(),
            (Some(c), Tab::Modules) => self.modules(&c, window, cx, &t).into_any_element(),
        };
        let mut fields = std::mem::take(&mut self.fields);
        fields.prune();
        self.fields = fields;
        let content = div()
            .id("settings")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, window, cx| {
                if s.recording.is_none() && e.keystroke.key == "escape" {
                    if window.context_stack().iter().any(|c| c.contains("Input")) {
                        window.focus(&s.focus, cx);
                    } else {
                        window.remove_window();
                    }
                    cx.stop_propagation();
                    return;
                }
                s.record_key(e, cx)
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(t.content)
            .font_family(Theme::font_family())
            .text_size(px(font::BODY))
            .line_height(relative(1.22))
            .text_color(t.primary)
            .child(tabs)
            .child(div().flex_1().min_h_0().child(body));
        super::chrome::frame(tr("Einstellungen"), content, &t)
    }
}

pub fn apply_theme(window: &mut Window, cx: &mut App) {
    theme::apply(theme::is_dark(window.appearance()), Some(window), cx);
}
