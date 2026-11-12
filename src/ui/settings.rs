use std::collections::{HashMap, HashSet};
use std::rc::Rc;

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
use super::time_field::Preset;
use super::state::{self, act, edit_config};
use super::motion::{self, Soft};
use super::theme::{self, Theme, font, hover, radius, space, iconsize, control, tint};
use super::widgets::*;
use crate::core::text::fold;
use crate::core::i18n::{ctrl_name, tr, trf};
use crate::core::ics;
use crate::core::model::*;
use crate::core::naming::Naming;
use crate::core::prefs::{self, NoteSaveMode, Shortcut};
use crate::core::themes;
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

const TABS: usize = 8;

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
    ("json", "Erscheinungsbild"),
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
    ("benachrichtigung", "Mitteilungen vor Abgaben"),
    ("notification", "Mitteilungen vor Abgaben"),
    ("deadline", "Mitteilungen vor Abgaben"),
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
/// One tab of the settings bar at full width.
const TAB: f32 = 76.;
/// The settings search field.
const SEARCH: f32 = 170.;
/// The window width below which the empty search folds to its icon so all tabs keep their labels.
const TABS_WITH_SEARCH: f32 = (if super::chrome::CUSTOM { space::M } else { 72. }) + 8. * (TAB + space::XXS) + space::S + SEARCH + space::M;

/// The window width below which the module editor stacks its colours under the code and narrows its fields.
const COMPACT_MODULE: f32 = 720.;

/// The window width below which a link of a module puts its buttons on a line of their own.
const COMPACT_LINKS: f32 = 760.;

/// The least width of a link's address, so it stays readable when the window is narrow.
const URL_MIN: f32 = 160.;

const NEEDS_CONVENTION: &[&str] = &["Matrikelnummer", "Mitautor:innen", "Muster", "Beispiel"];

/// Every labelled setting and the tab it lives on, for the search field.
const SEARCHABLE: &[(&str, Tab)] = &[
    ("Darstellung", Tab::General),
    ("Erscheinungsbild", Tab::General),
    ("Eigene Designs", Tab::General),
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
    ("Mitteilungen vor Abgaben", Tab::General),
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
    ("Für Fortgeschrittene", Tab::Ai),
    ("Für die Kommandozeile", Tab::Ai),
    ("Befehl", Tab::Ai),
    ("Im Terminal öffnen", Tab::Ai),
    ("Spalte öffnen", Tab::Ai),
    ("Spalte", Tab::Ai),
    ("Programm in der Spalte", Tab::Ai),
    ("Claude verbinden", Tab::Ai),
    ("Bei Änderungen zur Datei springen", Tab::Ai),
    ("Sitzung beenden nach", Tab::Ai),
    ("Eigener Befehl", Tab::Ai),
    ("Befehle ausführen erlauben", Tab::Ai),
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

/// What the appearance rows say about the last design that was loaded or removed.
enum ThemeNote {
    Problem(String),
    Removed { id: String, name: String, bytes: Vec<u8>, was_active: bool, stamp: u64 },
    Replaced { id: String, name: String, bytes: Vec<u8>, previous: String, stamp: u64 },
}

/// The settings window: Allgemein, Semester, Module, Notizen, Tastenkürzel.
pub struct Settings {
    tab: Tab,
    fields: Fields,
    semester: Option<String>,
    module: Option<String>,
    expanded_rule: Option<String>,
    advanced: HashSet<String>,
    search: Entity<InputState>,
    /// The search hit Enter opens.
    hit: usize,
    /// The scrolling page of each tab that scrolls as a whole, so a tab opens at its top.
    pages: [ScrollHandle; TABS],
    recording: Option<ShortcutAction>,
    rejected: Option<Rejected>,
    key_buttons: HashMap<&'static str, FocusHandle>,
    shortcut_scroll: UniformListScrollHandle,
    last_changed: Option<ShortcutAction>,
    notice: Option<Notice>,
    notices: u64,
    theme_note: Option<ThemeNote>,
    focus: FocusHandle,
    launch_at_login: bool,
    ai_rescanned: bool,
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
        window_min_size: Some(size(px(640.), px(500.))),
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

const SHORTCUT_ROW: f32 = control::M + space::S * 2.;

struct ShortcutRows {
    me: Entity<Settings>,
    modules: Vec<String>,
    clashes: HashMap<ShortcutAction, Vec<ShortcutAction>>,
    handles: HashMap<&'static str, FocusHandle>,
    recording: Option<ShortcutAction>,
    rejected: Option<Rejected>,
    reserved: bool,
    any_custom: bool,
    escape: String,
    delete: String,
}

enum ShortcutLine {
    Head(Group),
    Row(ShortcutAction, bool, bool),
}

fn shortcut_line(line: &ShortcutLine, c: &ShortcutRows, t: &Theme, cx: &mut App) -> AnyElement {
    let (a, first, last) = match line {
        ShortcutLine::Head(g) => {
            let key = g.label();
            return div().w_full().h(px(SHORTCUT_ROW)).flex().items_end().pb(px(space::S)).px(px(space::XS)).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(jump_target(&key, div().child(key.clone()), t)).into_any_element();
        }
        ShortcutLine::Row(a, first, last) => (*a, *first, *last),
    };
        let mut clashes: Vec<String> = c.clashes.get(&a).map(|others| others.iter().map(|o| o.named(&c.modules)).collect()).unwrap_or_default();
        clashes.extend(a.fixed_conflicts());
        let recording = c.recording == Some(a);
        let customized = a.is_customized();
        let rejected = c.rejected.clone().filter(|_| recording);
        let handle = c.handles.get(a.id()).cloned().unwrap_or_else(|| cx.focus_handle().tab_stop(true));
        let blocked_why = a.blocked();
        let blocked = blocked_why.as_ref().map(|why| trf("%@ – inaktiv", &[why]));
        let reason_chip: Option<String> = rejected.map(|why| match why {
            Rejected::NeedsModifier if cfg!(target_os = "macos") => tr("Dazu ⌘, ⌥ oder ⌃ halten"),
            Rejected::NeedsModifier => trf("Dazu %@ oder Alt halten", &[&ctrl_name()]),
            Rejected::Reserved(why) => why,
        });
        let reason: Option<String> = reason_chip.as_ref().map(|chip| if c.reserved { trf("%@ – andere Tasten drücken", &[chip]) } else { chip.clone() });
        let note = |text: String| div().min_w_0().truncate().text_size(px(font::CAPTION)).text_color(t.orange_text).child(text).into_any_element();
        let quiet = |text: String| {
            div().min_w_0().flex().px(px(space::XS)).rounded(px(radius::SMALL)).bg(t.ink.opacity(0.04)).border_1().border_color(t.ink.opacity(0.08)).text_size(px(font::CAPTION)).text_color(t.secondary).child(div().min_w_0().truncate().child(text)).child(div().flex_none().whitespace_nowrap().ml(px(space::XS)).child(tr("– inaktiv"))).into_any_element()
        };
        let dimmed = blocked.is_some() && reason.is_none() && !recording;
        let name = a.named(&c.modules);
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
        let hover = t.ink.opacity(hover::ROW);
        let help = reason.or(blocked).unwrap_or_else(|| if recording { trf("%@ bricht ab, %@ entfernt das Kürzel", &[&c.escape, &c.delete]) } else { tr("Klicken zum Ändern") });
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
            .tip(help)
            .on_click({ let me = c.me.clone(); move |_, window, cx| me.update(cx, |s, cx| s.start_recording(a, true, window, cx)) });
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
                .tip(full)
        });
    let row = div()
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
                        .on_click({ let me = c.me.clone(); move |_, window, cx| me.update(cx, |s, cx| s.start_recording(a, false, window, cx)) })
                        .child(div().min_w_0().truncate().when(dimmed, |d| d.text_color(t.secondary)).child(a.named(&c.modules)))
                        .children(clash),
                )
                .child(recorder)
                .child(div().flex_none().w(px(if c.any_custom { control::S + space::S + space::L } else { space::L })).pl(px(if c.any_custom { space::S } else { 0. })).pr(px(space::L)).when(customized, |d| {
                    d.child(icon_button(SharedString::from(format!("rst-{}", a.id())), "rotate-ccw", trf("Zurücksetzen auf %@", &[&default]), false, t, {
                        let this = c.me.clone();
                        move |_, cx| {
                            this.update(cx, |s, cx| {
                                let before = ShortcutAction::snapshot();
                                s.last_changed = Some(a);
                                a.reset();
                                super::app::bind_keys(cx);
                                let text = changed(a, cx);
                                s.offer_undo(before, text, cx);
                                cx.notify();
                            });
                        }
                    }))
                }))
                .into_any_element();
    div()
        .w_full()
        .h(px(SHORTCUT_ROW))
        .flex()
        .flex_col()
        .bg(t.secondary_bg)
        .border_x_1()
        .border_color(t.separator.opacity(0.6))
        .when(first, |d| d.border_t_1().rounded_tl(px(radius::CARD)).rounded_tr(px(radius::CARD)))
        .when(last, |d| d.border_b_1().rounded_bl(px(radius::CARD)).rounded_br(px(radius::CARD)))
        .when(!first, |d| d.child(divider(t)))
        .child(div().flex_1().min_h_0().flex().flex_col().justify_center().child(row))
        .into_any_element()
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
fn jump_target<E: Styled + ParentElement + IntoElement>(label: &str, content: E, t: &Theme) -> E {
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

const CLAUDE_INSTALL: &str = "https://code.claude.com/docs/en/setup";
const AI_ADVANCED: &str = "ai-advanced";
const ADVANCED_LABELS: &[&str] = &["Knopf in der Werkzeugleiste", "Für die Kommandozeile", "Befehl", "Im Terminal öffnen", "Spalte öffnen", "Spalte", "Programm in der Spalte", "Claude verbinden", "Bei Änderungen zur Datei springen", "Sitzung beenden nach", "Eigener Befehl"];

/// Whether a jump leads to one of these labels right now.
fn jumping_to(labels: &[&str]) -> bool {
    JUMP.with(|j| j.borrow().as_ref().is_some_and(|j| labels.contains(&j.label.as_str())))
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
        .child(div().flex_1().min_w_0().flex().items_center().gap(px(space::S)).text_color(t.primary).child(div().flex_none().child(indicator)).child(div().id("apple-cal-state").min_w_0().truncate().child(label.clone()).tip(label)))
        .children(button.map(|b| div().flex_none().max_w(px(control::ROW_CONTROL)).child(b)))
        .into_any_element()
}

fn row(label: impl Into<SharedString>, control: impl IntoElement, t: &Theme) -> AnyElement {
    let label: SharedString = label.into();
    let key = label.to_string();
    jump_target(&key, div().flex().items_center().gap(px(space::M)).px(px(space::L)).min_h(px(control::ROW)).py(px(space::S)).child(div().flex_1().min_w_0().text_color(t.primary).child(label)).child(div().flex_none().max_w(px(control::ROW_CONTROL)).child(control)), t).into_any_element()
}

fn switch_row(id: impl Into<SharedString>, label: impl Into<SharedString>, on: bool, disabled: bool, toggle: impl Fn(bool, &mut Window, &mut App) + 'static, t: &Theme) -> AnyElement {
    let (id, label): (SharedString, SharedString) = (id.into(), label.into());
    if disabled {
        return row(label, Switch::new(id).checked(on).disabled(true), t);
    }
    let key = label.to_string();
    let switch = Switch::new(SharedString::from(format!("{id}-switch"))).checked(on).tab_stop(false);
    jump_target(&key, toggle_row(id, label, switch, t, move |_, window, cx| toggle(!on, window, cx)).px(px(space::L)).min_h(px(control::ROW)).py(px(space::S)), t).into_any_element()
}

fn detail_switch_row(id: impl Into<SharedString>, label: impl Into<SharedString>, detail: String, on: bool, toggle: impl Fn(bool, &mut Window, &mut App) + 'static, t: &Theme) -> AnyElement {
    let (id, label): (SharedString, SharedString) = (id.into(), label.into());
    let key = label.to_string();
    let text = div().flex().flex_col().gap(px(space::XXS)).child(div().text_color(t.primary).child(label)).child(caption(detail, t));
    let switch = Switch::new(SharedString::from(format!("{id}-switch"))).checked(on).tab_stop(false);
    jump_target(&key, toggle_row(id, text, switch, t, move |_, window, cx| toggle(!on, window, cx)).px(px(space::L)).min_h(px(control::ROW)).py(px(space::S)), t).into_any_element()
}

/// A square icon button of the minimum hit size with the hover every icon control has.
fn icon_button(id: impl Into<ElementId>, name: &str, help: String, disabled: bool, t: &Theme, on_click: impl Fn(&mut Window, &mut App) + 'static) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(control::S))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(radius::INNER))
        .when(disabled, |d| d.opacity(tint::DISABLED))
        .when(!disabled, |d| d.cursor_pointer().key_nav().soft(t.ink.opacity(hover::CONTROL), radius::INNER).on_click(move |_, window, cx| on_click(window, cx)))
        .child(icon(name, iconsize::M, t.primary))
        .tip(help)
}

/// The line that adds an entry to a list: a link on the left, an optional second action on the right.
fn add_row(id: &'static str, label: String, secondary: Option<AnyElement>, t: &Theme, on_click: impl Fn(&mut App) + 'static) -> AnyElement {
    div()
        .flex()
        .items_center()
        .px(px(space::L))
        .py(px(space::S))
        .child(link_button(id, label, t).flex_row_reverse().gap(px(space::XS)).on_click(move |_, _, cx| on_click(cx)).child(icon("plus", iconsize::S, t.accent_text)))
        .child(div().flex_1())
        .children(secondary)
        .into_any_element()
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

fn naming_row(id: &'static str, c: &StudyConfig, t: &Theme) -> AnyElement {
    let on = c.naming_convention();
    detail_switch_row(id, tr("Nach einem Muster benennen"), naming_detail(on), on, |v, _, cx| edit_config(cx, |c| c.set_naming_convention(v)), t)
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
    let on = c.naming_convention();
    let mut naming = vec![naming_row("g-naming", c, t)];
    if on {
        naming.extend(extra);
    }
    let folders = vec![
        detail_row(tr("Abgabeordner"), tr("In jedem Modul. Hier landen abgelegte Dateien und neue Notizen für eine Abgabe. Leer: der Modulordner."), submissions, t),
        switch_row("g-create-folders", tr("Ordner anlegen"), c.create_folders, false, |v, _, cx| edit_config(cx, |c| c.create_folders = v), t),
    ];
    (
        section(Some(tr("Namensmuster")), on.then(|| tr("Das Muster steht in den Einstellungen jedes Moduls.")), naming, t),
        section(Some(tr("Ablegen und Ordner")), Some(tr("Schrumbi benennt und verschiebt nie etwas von selbst. Ohne „Ordner anlegen“ landen Dateien nur in Ordnern, die es schon gibt.")), folders, t),
    )
}

fn minus(id: impl Into<ElementId>, help: String, t: &Theme, f: impl Fn(&mut App) + 'static) -> Stateful<Div> {
    icon_button(id, "circle-minus", help, false, t, move |_, cx| f(cx))
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

fn theme_label(choice: prefs::ThemeChoice, file: &str, designs: &[themes::Installed]) -> String {
    designs.iter().find(|d| d.id == file && d.problem.is_none()).map_or_else(|| theme::choice_label(choice), |d| d.name.clone())
}

impl Settings {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Suchen")));
        let lib = state::lib(cx);
        let (me, handle) = (cx.weak_entity(), window.window_handle());
        let recorder = cx.intercept_keystrokes(move |e, window, cx| {
            let Some(settings) = me.upgrade().filter(|s| window.window_handle() == handle && s.read(cx).armed(window)) else { return };
            settings.update(cx, |s, cx| s.record_key(&e.keystroke, cx));
            cx.stop_propagation();
        });
        let activation = cx.observe_window_activation(window, |s, window, cx| {
            if window.is_window_active() {
                if let Some(m) = theme::recheck(window, cx) {
                    s.theme_note = Some(ThemeNote::Problem(m));
                }
                cx.notify();
                cx.spawn(async move |s, cx| {
                    cx.background_spawn(async { themes::rescan() }).await;
                    let _ = s.update(cx, |_, cx| cx.notify());
                })
                .detach();
            }
        });
        let subs = vec![recorder, activation, cx.observe(&lib, |_, _, cx| cx.notify()), cx.subscribe_in(&search, window, |s, _, e: &InputEvent, window, cx| {
            match e {
                InputEvent::PressEnter { .. } if s.tab != Tab::Shortcuts => {
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
        super::ai::recheck(cx);
        Self {
            tab: Tab::General,
            fields: Fields::default(),
            semester,
            module,
            expanded_rule: None,
            advanced: HashSet::new(),
            search,
            hit: 0,
            pages: std::array::from_fn(|_| ScrollHandle::new()),
            recording: None,
            rejected: None,
            key_buttons: HashMap::new(),
            shortcut_scroll: UniformListScrollHandle::new(),
            last_changed: None,
            notice: None,
            notices: 0,
            theme_note: None,
            focus: cx.focus_handle(),
            launch_at_login: super::platform::launches_at_login(),
            ai_rescanned: false,
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
        for action in ShortcutAction::ALL.into_iter().filter(|a| *a != ShortcutAction::ToggleAi || super::ai::active()) {
            let label = action.named(&modules);
            if fold(&label).contains(&query) || fold(&action.label()).contains(&query) {
                scored.push((3, Hit { label: label.clone(), place: tr("Tastenkürzel"), to: HitTarget::Shortcut(label) }));
            }
        }
        scored.sort_by(|a, b| a.0.cmp(&b.0));
        scored.into_iter().map(|(_, hit)| hit).take(40).collect()
    }

    fn show_module(&mut self, id: String) {
        self.go(Tab::Modules);
        self.module = Some(id);
    }

    fn page(&self) -> ScrollHandle {
        self.pages[self.tab as usize].clone()
    }

    fn go(&mut self, tab: Tab) {
        self.tab = tab;
    }

    fn open_result(&mut self, hit: Hit, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = None;
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        match hit.to {
            HitTarget::Setting(tab, label) => {
                self.go(tab);
                self.pages[tab as usize].set_offset(point(px(0.), px(0.)));
                self.light_up(label, window, cx);
            }
            HitTarget::Module(id) => self.show_module(id),
            HitTarget::Semester(id) => {
                self.go(Tab::Semester);
                self.semester = Some(id);
            }
            HitTarget::Shortcut(label) => {
                self.go(Tab::Shortcuts);
                self.search.update(cx, |s, cx| s.set_value(label, window, cx));
            }
        }
        cx.notify();
    }

    /// Scrolls to a setting and marks it until the eye has found it.
    fn light_up(&mut self, label: String, _: &mut Window, cx: &mut Context<Self>) {
        JUMP.with(|j| *j.borrow_mut() = Some(Jump { label, handle: self.page(), scrolled: false }));
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
                    .soft(t.ink.opacity(hover::ROW), radius::INNER)
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

    fn tab_bar(&self, window: &Window, t: &Theme, cx: &mut Context<Self>) -> Div {
        let searching = self.search.read(cx).focus_handle(cx).is_focused(window) || !self.search.read(cx).value().is_empty();
        let folded = f32::from(window.viewport_size().width) < TABS_WITH_SEARCH && !searching;
        let showing_results = self.tab != Tab::Shortcuts && !self.search.read(cx).value().trim().is_empty();
        let mut bar = div().flex_1().min_w_0().flex().justify_start().gap(px(space::XXS));
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
            let active = tab == self.tab && !showing_results;
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("tab-{label}")))
                    .flex_1()
                    .min_w_0()
                    .max_w(px(TAB))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(space::XS))
                    .py(px(space::S))
                    .rounded(px(radius::CONTROL))
                    .cursor_pointer().key_nav()
                    .when(active, |d| d.bg(t.accent.opacity(tint::SELECTED)))
                    .when(!active, |d| d.soft(t.ink.opacity(hover::ROW), radius::CONTROL))
                    .child(icon(icon_name, iconsize::L, if active { t.accent } else { t.secondary }))
                    .child(div().max_w_full().truncate().text_size(px(font::CAPTION)).text_color(if active { t.accent_text } else { t.primary }).child(label))
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.go(tab);
                        s.recording = None;
                        cx.notify();
                    })),
            );
        }
        let search = if folded {
            div().flex_none().child(Button::new("settings-search-fold").ghost().small().icon(menu_icon("search")).tooltip(format!("{} ({})", tr("Suchen"), Shortcut { key: "f".into(), command: true, shift: false, option: false, control: false }.display())).on_click(cx.listener(|s, _, window, cx| s.search.update(cx, |i, cx| i.focus(window, cx)))))
        } else {
            div().flex_none().w(px(SEARCH)).child(Input::new(&self.search).small().cleanable(true).prefix(icon("search", iconsize::XS, t.secondary)))
        };
        div().flex().items_center().gap(px(space::S)).pl(px(if super::chrome::CUSTOM { space::M } else { 72. })).pr(px(space::M)).pt(px(if super::chrome::CUSTOM { space::M } else { space::XXL })).pb(px(space::S)).border_b_1().border_color(t.separator).child(bar).child(search)
    }

    fn general(&mut self, c: &StudyConfig, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let mut f = std::mem::take(&mut self.fields);
        let text = |f: &mut Fields, key: &str, value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>, set: fn(&mut StudyConfig, String)| f.input(key, value, placeholder, window, cx, move |v, cx| edit_config(cx, |c| set(c, v)));
        let name = text(&mut f, "g-name", &c.name, "", window, cx, |c, v| c.name = v);
        let author = text(&mut f, "g-author", &c.author, &tr("Nachname-V"), window, cx, |c, v| c.author = v);
        let inbox = text(&mut f, "g-inbox", &c.inbox, "", window, cx, |c, v| if is_inner_folder(&v) { c.inbox = v });
        let submissions = text(&mut f, "g-submissions", &c.submission_folder, &tr("Modulordner"), window, cx, |c, v| if v.trim().is_empty() || is_inner_folder(&v) { c.submission_folder = v });
        let matrikel = text(&mut f, "g-matrikel", &c.matrikel, &tr("Matrikelnummer"), window, cx, |c, v| c.matrikel = v);
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
        let (theme_choice, theme_file) = (p.theme, p.theme_file.clone());
        let designs = themes::installed();
        let in_use = designs.iter().any(|d| d.id == theme_file && d.problem.is_none());
        let theme_name = theme_label(theme_choice, &theme_file, &designs);
        let menu_designs = designs.clone();
        let me = cx.weak_entity();
        let mut appearance_rows = vec![row(
                tr("Erscheinungsbild"),
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .child(picker("g-theme", theme_name, move |mut m| {
                        for choice in [prefs::ThemeChoice::System, prefs::ThemeChoice::Light, prefs::ThemeChoice::Dark, prefs::ThemeChoice::Nord, prefs::ThemeChoice::Dracula] {
                            m = m.item(PopupMenuItem::new(theme::choice_label(choice)).checked(!in_use && choice == theme_choice).on_click(move |_, window, cx| theme::choose(choice, window, cx)));
                        }
                        if !menu_designs.is_empty() {
                            m = m.separator();
                            for d in &menu_designs {
                                let id = d.id.clone();
                                m = m.item(PopupMenuItem::new(d.name.clone()).checked(d.id == theme_file).disabled(d.problem.is_some()).on_click(move |_, window, cx| theme::choose_file(&id, window, cx)));
                            }
                        }
                        m
                    }))
                    .child(Button::new("g-theme-load").small().label(tr("Design laden…")).on_click(move |_, window, cx| {
                        let me = me.clone();
                        theme::pick_and_load(window.window_handle(), cx, move |result, cx| {
                            let _ = me.update(cx, |s, cx| {
                                s.theme_note = match result {
                                    Ok(loaded) => loaded.replaced.map(|bytes| ThemeNote::Replaced { id: loaded.id, name: loaded.name, bytes, previous: loaded.previous, stamp: s.note_later(cx) }),
                                    Err(m) => Some(ThemeNote::Problem(m)),
                                };
                                cx.notify();
                            });
                        });
                    })),
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
            )];
        appearance_rows.extend(self.theme_note_row(t, cx));
        let language = section(Some(tr("Darstellung")), None, appearance_rows, t);
        let own_designs = (!designs.is_empty()).then(|| section(Some(tr("Eigene Designs")), None, designs.iter().enumerate().map(|(i, d)| self.design_row(i, d, t, cx)).collect(), t));
        let enabled = p.reminders_enabled;
        let hours = p.reminder_hours;
        let reminders = section(
            Some(tr("Erinnerungen")),
            None,
            vec![
                switch_row("g-rem", tr("Mitteilungen vor Abgaben"), enabled, false, |v, _, cx| {
                    prefs::update(|p| p.reminders_enabled = v);
                    state::sync_reminders(cx);
                }, t),
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
                switch_row("g-daybefore", tr("Zusätzlich am Vortag (Pflichtabgaben)"), p.reminder_day_before, !enabled, |v, _, cx| {
                    prefs::update(|p| p.reminder_day_before = v);
                    state::sync_reminders(cx);
                }, t),
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
            outgoing_rows.push(switch_row("g-apple-cal", tr("Mit Apple Kalender abgleichen"), sync_on, false, |v, _, cx| {
                prefs::update(|p| p.apple_calendar = v);
                if v {
                    super::apple_calendar::push(cx);
                } else {
                    super::apple_calendar::turn_off();
                }
                cx.refresh_windows();
            }, t));
            if sync_on {
                outgoing_rows.push(apple_calendar_row(t));
            }
        }
        outgoing_rows.push(switch_row("g-cal", tr("Kalenderdatei aktuell halten"), feed_on, false, |v, _, cx| {
            prefs::update(|p| p.calendar_feed = v);
            state::sync_reminders(cx);
            cx.refresh_windows();
        }, t));
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
        let this = cx.weak_entity();
        let system = section(
            Some(tr("System")),
            None,
            vec![
                switch_row("g-login", tr("Bei Anmeldung starten"), login, false, move |v, _, cx| {
                    let _ = this.update(cx, |s, cx| {
                        if super::platform::set_launch_at_login(v) {
                            s.launch_at_login = v;
                        } else {
                            act(cx, |l| l.last_error = Some(tr("Autostart konnte nicht geändert werden.")));
                        }
                        cx.notify();
                    });
                }, t),
                switch_row("g-update", tr("Automatisch nach Updates suchen"), p.update_check, false, |v, _, _| prefs::update(|p| p.update_check = v), t),
                switch_row("g-tray", tr("Symbol in der Menüleiste"), p.tray, !super::tray::AVAILABLE, |v, _, cx| {
                    prefs::update(|p| p.tray = v);
                    super::tray::sync(cx);
                }, t),
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
                    switch_row(format!("g-tab-{key}"), label, shown, false, move |v, _, cx| super::sidebar::set_hidden(key, !v, cx), t)
                })
                .collect(),
            t,
        );
        let setup_shown = !super::setup_card::hidden(cx);
        let hints = section(
            Some(tr("Hinweise")),
            Some(tr("Der Tipp des Tages erscheint einmal am Tag unten rechts und nennt nur, was du noch nicht benutzt hast. Die Liste zum Einrichten hakt sich von selbst ab.")),
            vec![
                switch_row("g-tips", tr("Tipp des Tages"), p.tips_enabled, false, |v, _, cx| {
                    prefs::update(|p| {
                        p.tips_enabled = v;
                        if v {
                            p.tips_last.clear();
                        }
                    });
                    cx.refresh_windows();
                }, t),
                switch_row("g-setup", tr("Einrichten in der Übersicht"), setup_shown, false, |v, _, cx| {
                    if v {
                        super::setup_card::restore(cx);
                    } else {
                        let root = state::read(cx, |l| l.root.clone()).unwrap_or_default();
                        prefs::update_setup(&root, |s| s.dismissed = true);
                        cx.refresh_windows();
                    }
                }, t),
            ],
            t,
        );
        div().flex().flex_col().gap(px(space::XL)).child(language).children(own_designs).child(studium).child(reminders).child(naming).child(filing).child(grades).child(sidebar).child(hints).child(incoming).child(calendar).child(system).child(search).child(storage)
    }
}

/// Starts a new instance and quits this one, so a language change applies.
impl Settings {
    fn master(&self, items: Vec<(String, String, String, Option<Hsla>)>, selected: Option<&str>, on_select: impl Fn(&mut Settings, String) + 'static + Clone, bar: Div, t: &Theme, cx: &mut Context<Self>) -> Div {
        let mut list = div().flex().flex_col().py(px(space::S));
        for (id, title, sub, dot) in items {
            if title.is_empty() {
                list = list.child(div().px(px(space::M)).pt(px(space::S)).pb(px(space::XXS)).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(sub));
                continue;
            }
            let active = selected == Some(id.as_str());
            let f = on_select.clone();
            let id2 = id.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("m-{id}")))
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .px(px(space::M))
                    .py(px(space::XS))
                    .cursor_pointer().key_nav()
                    .when(active, |d| d.child(motion::row_fill(t.ink.opacity(0.08))).child(div().absolute().left(px(hover::INSET)).top(px(space::S)).bottom(px(space::S)).w(px(space::XXS)).rounded_r(px(space::XXS)).bg(t.accent)))
                    .when(!active, |d| d.soft_row(t.ink.opacity(hover::ROW)))
                    .when_some(dot, |d, c| d.child(div().size(px(space::S)).flex_none().rounded_full().bg(c)))
                    .child(div().flex().flex_col().min_w_0().child(div().truncate().child(title)).child(div().truncate().text_size(px(font::CAPTION)).text_color(t.secondary).child(sub)))
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
            .child(icon_button("s-add", "plus", tr("Neues Semester"), false, t, move |_, cx| e1.update(cx, |s, cx| s.add_semester(false, cx))))
            .child(icon_button("s-del", "minus", tr("Semester löschen"), !can_delete, t, move |window, cx| {
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
            .child(icon_button("s-dup", "copy-plus", tr("Folgesemester mit denselben Modulen und Regeln anlegen"), sel.is_none(), t, move |_, cx| e3.update(cx, |s, cx| s.add_semester(true, cx))));
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
                            .child(minus(SharedString::from(format!("brk-rm-{i}")), tr("Pause entfernen"), t, move |cx| e(Box::new(move |s| if i < s.breaks.len() { s.breaks.remove(i); }))(cx))),
                    )
                    .child(div().text_size(px(font::CAPTION)).text_color(t.secondary).child(break_summary(b)))
                    .into_any_element(),
            );
        }
        let e = edit.clone();
        let start_date = s.start.clone();
        breaks.push(add_row("brk-add", tr("Pause hinzufügen"), None, t, move |cx| {
            let st = day::parse(&start_date).unwrap_or_else(|| Local::now().date_naive());
            e(Box::new(move |s| s.breaks.push(Break { name: tr("Pause"), start: day::format(st), end: day::format(st + Days::new(6)) })))(cx)
        }));
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
                    .child(minus(SharedString::from(format!("hol-rm-{i}")), tr("Feiertag entfernen"), t, move |cx| e(Box::new(move |s| if i < s.holidays.len() { s.holidays.remove(i); }))(cx)))
                    .into_any_element(),
            );
        }
        self.fields = fields;
        let (e1, e2) = (edit.clone(), edit.clone());
        let first = s.start.clone();
        let (sid, s2) = (s.id.clone(), s.clone());
        holidays.push(add_row(
            "hol-add",
            tr("Feiertag hinzufügen"),
            Some(
                Button::new("hol-ics").ghost().small().icon(menu_icon("calendar-plus")).label(tr("Aus Kalenderdatei (.ics)…")).on_click(move |_, _, cx| {
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
                })
                .into_any_element(),
            ),
            t,
            move |cx| {
                let f = first.clone();
                e1(Box::new(move |s| s.holidays.push(Holiday { name: tr("Feiertag"), date: f.clone() })))(cx)
            },
        ));
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
            .child(icon_button("m-add", "plus", tr("Neues Modul"), false, t, move |_, cx| {
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
            .child(icon_button("m-del", "minus", tr("Modul löschen"), sel.is_none(), t, move |window, cx| {
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
        let compact = f32::from(window.viewport_size().width) < COMPACT_MODULE;
        let narrow = f32::from(window.viewport_size().width) < COMPACT_LINKS;
        let mut dots = div().flex().gap(px(space::XS));
        for name in PALETTE {
            let selected = m.color == name;
            let e = edit.clone();
            dots = dots.child(
                div()
                    .id(SharedString::from(format!("color-{name}")))
                    .size(px(control::S))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer().key_nav()
                    .soft(t.ink.opacity(hover::CONTROL), radius::FULL)
                    .when(selected, |d| d.border_2().border_color(t.primary))
                    .child(div().size(px(iconsize::S)).rounded_full().bg(t.palette(name)))
                    .tip(name)
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
            row(tr("Kürzel"), div().flex().when(compact, |d| d.flex_col().items_end()).when(!compact, |d| d.items_center()).gap(px(if compact { space::S } else { space::M })).child(div().w(px(110.)).child(Input::new(&code))).child(dots), t),
            row(tr("Name"), div().w(px(if compact { 240. } else { 320. })).child(Input::new(&name)), t),
            row(tr("Semester"), sem_picker, t),
            row(
                tr("Ordner"),
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .child(div().w(px(if compact { 150. } else { 220. })).child(Input::new(&folder)))
                    .child(div().id("folder-state").child(icon(if exists { "circle-check" } else { "circle-alert" }, iconsize::S, if exists { t.green } else { t.orange })).tip(if exists { tr("Ordner existiert") } else { tr("Ordner existiert nicht") }))
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
            let upload = link.upload;
            let valid_glyph = div()
                .id(("lv", i))
                .size(px(control::S))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(icon(if valid { "circle-check" } else { "circle-alert" }, iconsize::S, if valid { t.green } else if link.address.is_empty() { t.secondary } else { t.orange }))
                .tip(if valid { tr("Gültige Adresse") } else { tr("Nur http(s)-Adressen") });
            let upload_toggle = toggle_row(("lu", i), tr("Upload"), Checkbox::new(("lu-box", i)).checked(upload).tab_stop(false), t, move |_, _, cx| {
                let on = !upload;
                e1(Box::new(move |m| {
                    for (j, l) in m.links.iter_mut().enumerate() {
                        l.upload = j == i && on;
                    }
                }))(cx)
            })
            .flex_row_reverse()
            .gap(px(space::S))
            .px(px(space::XS))
            .tip(tr("Wird beim Hochladen einer Abgabe geöffnet"));
            let open_button = icon_button(("lo", i), "square-arrow-out-up-right", tr("Öffnen"), !valid, t, move |_, _| {
                if let Some(u) = &url {
                    crate::core::library::open_path(u);
                }
            });
            let remove = minus(("lr", i), tr("Link entfernen"), t, move |cx| e2(Box::new(move |m| if i < m.links.len() { m.links.remove(i); }))(cx));
            let actions = div().flex().flex_none().items_center().gap(px(space::XS)).child(valid_glyph).child(upload_toggle).child(open_button).child(remove);
            let fields_line = div().flex().items_center().gap(px(space::S)).child(div().w(px(110.)).flex_none().child(Input::new(&title).small())).child(div().flex_1().min_w(px(URL_MIN)).child(Input::new(&addr).small()));
            let line = div().flex().gap(px(space::S)).px(px(space::L)).py(px(space::S));
            links.push(if narrow {
                line.flex_col().child(fields_line).child(div().flex().justify_end().child(actions)).into_any_element()
            } else {
                line.items_center().child(div().flex_1().min_w_0().child(fields_line)).child(actions).into_any_element()
            });
        }
        let e = edit.clone();
        links.push(add_row("link-add", tr("Link hinzufügen"), None, t, move |cx| {
            e(Box::new(|m| {
                let first = m.links.is_empty();
                m.links.push(ModuleLink { title: if first { "Moodle".into() } else { String::new() }, upload: first, ..Default::default() })
            }))(cx)
        }));
        let example = Naming::new(&m, semester.as_ref(), c).render(&tr("Titel"), Local::now().date_naive(), "pdf", &[], None, None, None);
        let naming_key = format!("naming-{}", m.id);
        let naming_open = self.advanced.contains(&naming_key);
        let naming_rows = if c.naming_convention() {
            let mut chips = div().flex().flex_wrap().justify_end().gap(px(space::XS));
            for (label, token) in [(tr("Kürzel"), "{code}"), (tr("Jahr"), "{yy}"), (tr("Titel"), "{title}"), (tr("Name"), "{author}"), (tr("Datum"), "{date}")] {
                let (e, input) = (edit.clone(), naming.clone());
                chips = chips.child(
                    div()
                        .id(SharedString::from(format!("chip-{token}")))
                        .h(px(control::S))
                        .px(px(space::S))
                        .flex()
                        .items_center()
                        .rounded(px(radius::CONTROL))
                        .border_1()
                        .border_color(t.separator)
                        .text_size(px(font::CALLOUT))
                        .cursor_pointer()
                        .key_nav()
                        .soft(t.ink.opacity(hover::CONTROL), radius::CONTROL)
                        .child(label)
                        .on_click(move |_, window, cx| {
                            let v = format!("{}{}", input.read(cx).value(), token);
                            input.update(cx, |i, cx| i.set_value(v.clone(), window, cx));
                            e(Box::new(move |m| m.naming = v.clone()))(cx)
                        }),
                );
            }
            let more = div()
                .px(px(space::L))
                .py(px(space::S))
                .child(
                    link_button(SharedString::from(format!("adv-{naming_key}")), tr("Erweitert"), t)
                        .flex()
                        .items_center()
                        .gap(px(space::XS))
                        .child(motion::chevron("naming-chevron", naming_open, t.accent))
                        .on_click(cx.listener({
                            let key = naming_key.clone();
                            move |s, _, _, cx| {
                                if !s.advanced.remove(&key) {
                                    s.advanced.insert(key.clone());
                                }
                                cx.notify();
                            }
                        })),
                )
                .child(motion::reveal("naming-help", naming_open, div().pt(px(space::S)).child(caption(tr("Platzhalter: {code} {yy} {yyyy} {title} {author} {matrikel} {date} {date:yyyy-MM-dd} {w} {kw}"), t))))
                .into_any_element();
            vec![
                row(tr("Muster"), div().w(px(if compact { 240. } else { 320. })).flex().flex_col().items_end().gap(px(space::S)).child(div().w_full().child(Input::new(&naming))).child(chips), t),
                row(tr("Beispiel"), div().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(example), t),
                more,
            ]
        } else {
            vec![naming_row("m-naming", c, t)]
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
            let day = picker(("sd", i), names[(s.weekday.clamp(1, 7) - 1) as usize].chars().take(2).collect(), move |mut menu| {
                for d in 1..=7u32 {
                    let e = e1.clone();
                    menu = menu.item(PopupMenuItem::new(names[(d - 1) as usize]).on_click(move |_, _, cx| e(Box::new(move |m| if let Some(s) = m.sessions.get_mut(i) { s.weekday = d }))(cx)));
                }
                menu
            }).small();
            let title = div().flex_1().min_w_0().child(Input::new(&title).small());
            let room = div().w(px(80.)).child(Input::new(&room).small());
            let remove = minus(("srm", i), tr("Termin entfernen"), t, move |cx| e4(Box::new(move |m| if i < m.sessions.len() { m.sessions.remove(i); }))(cx));
            let dash = div().text_color(t.secondary).child("–");
            let session = div().px(px(space::L)).py(px(space::S));
            sessions.push(if compact {
                session.flex().flex_col().gap(px(space::XS)).child(div().flex().items_center().gap(px(space::S)).child(title).child(room).child(remove)).child(div().flex().items_center().gap(px(space::S)).child(day).child(start).child(dash).child(end)).into_any_element()
            } else {
                session.flex().items_center().gap(px(space::S)).child(title).child(day).child(start).child(dash).child(end).child(room).child(remove).into_any_element()
            });
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
            .child(section(Some(tr("Namensmuster")), (!c.naming_convention()).then(|| tr("Gilt für alle Module. Eingeschaltet erscheint hier das Muster dieses Moduls.")), naming_rows, t))
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
            .soft(t.ink.opacity(hover::ROW), radius::INNER)
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
        let compact = f32::from(window.viewport_size().width) < COMPACT_MODULE;
        let field = |label: String, content: AnyElement| if compact {
            div().flex().flex_col().gap(px(space::XS)).when(!label.is_empty(), |d| d.child(div().text_color(t.secondary).child(label))).child(div().w_full().child(content))
        } else {
            div().flex().items_center().gap(px(space::M)).child(div().w(px(80.)).text_color(t.secondary).child(label)).child(div().flex_1().min_w_0().child(content))
        };
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
                    .flex_wrap()
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
                switch_row("s-open", tr("Beim Öffnen synchronisieren"), g.sync_on_open, !automatic, |v, _, cx| sync::update_settings(cx, |g| g.sync_on_open = v), t),
                switch_row("s-quit", tr("Beim Beenden synchronisieren"), g.sync_on_quit, !automatic, |v, _, cx| sync::update_settings(cx, |g| g.sync_on_quit = v), t),
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
        let picker = Button::new("vault-pick").small().label(tr("Obsidian-Ordner verbinden…")).dropdown_menu(move |mut menu, _, _| {
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
            [editor_row, markdown_row].into_iter().chain(super::spelling::SUPPORTED.then(|| switch_row("n-spell", tr("Rechtschreibung prüfen"), p.spell_check, false, |on, _, cx| {
                prefs::update(|p| p.spell_check = on);
                cx.refresh_windows();
            }, t))).chain([row(
                tr("Schriftgröße"),
                stepper(
                    "n-size",
                    format!("{} pt", size as i32),
                    Box::new(move |cx| super::documents::set_font_size((size - 1.).max(10.), cx)),
                    Box::new(move |cx| super::documents::set_font_size((size + 1.).min(24.), cx)),
                ),
                t,
            ), switch_row("n-full-width", tr("Text in voller Breite"), p.reader_full_width, false, |v, _, cx| super::documents::set_reader_full_width(v, cx), t)]).collect(),
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
                    .soft(t.ink.opacity(hover::CONTROL), radius::FULL)
                    .when(selected, |d| d.border_2().border_color(t.primary))
                    .child(div().size(px(iconsize::M)).rounded_full().bg(color))
                    .tip(tr(name))
                    .on_click(move |_, _, _| prefs::update(|p| p.note_accent = hex.into())),
            );
        }
        swatches = swatches.child(div().w(px(90.)).child(Input::new(&accent).small()));
        let pdf = section(
            Some(tr("PDF-Export")),
            Some(trf("Notizen werden als A4-PDF neben der Markdown-Datei gesichert (%@ im Notizfenster).", &[&ShortcutAction::ExportPdf.active_shortcut().map_or_else(String::new, |s| s.display())])),
            vec![
                row(tr("Akzentfarbe"), swatches, t),
                switch_row("n-header", tr("Kopfzeile mit Modul, Woche, Name und Datum"), p.pdf_header, false, |v, _, _| prefs::update(|p| p.pdf_header = v), t),
                switch_row("n-properties", tr("Eigenschaften über dem Text"), p.pdf_properties, false, |v, _, _| prefs::update(|p| p.pdf_properties = v), t),
                switch_row("n-pages", tr("Seitenzahlen"), p.pdf_page_numbers, false, |v, _, _| prefs::update(|p| p.pdf_page_numbers = v), t),
                switch_row("n-replace-ask", tr("Vor dem Ersetzen eines früheren Exports fragen"), p.pdf_replace_ask, false, |v, _, _| prefs::update(|p| p.pdf_replace_ask = v), t),
                div()
                    .flex()
                    .items_center()
                    .px(px(space::L))
                    .py(px(space::S))
                    .child(link_button("n-reset", tr("Standard wiederherstellen"), t).text_size(px(font::CALLOUT)).on_click(|_, _, _| {
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
        let dictionary = super::spelling::SUPPORTED.then(|| self.dictionary_section(&p.spell_words, t));
        div().flex().flex_col().gap(px(space::XL)).child(editor).children(dictionary).child(section(Some(tr("Sichern")), Some(save_footer), save_rows, t)).child(pdf).child(obsidian)
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
                        icon_button(("dict-remove", i), "x", tr("Aus dem Wörterbuch entfernen"), false, t, move |_, cx| super::spell_card::remove_word(&gone, cx)),
                        t,
                    )
                })
                .collect()
        };
        section(Some(tr("Eigenes Wörterbuch")), Some(tr("Diese Wörter markiert die Rechtschreibung nicht. Hinzufügen: in einer Notiz auf ein unterstrichenes Wort zeigen.")), rows, t)
    }

    fn ai_tab(&mut self, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        use super::ai::{self, Download, Registration};
        use crate::core::ai::agents::{self, Runner};
        use crate::core::ai::detect::{self, Agent};
        use crate::core::ai::install;
        let prefs = prefs::with(|p| p.ai.clone());
        let found = detect::found();
        let names = found.iter().map(|c| c.agent.label()).collect::<Vec<_>>().join(", ");
        let chosen = agents::chosen(&prefs);
        let signed_out = match chosen {
            Some(Runner::Known(a)) if a != Agent::OpenCode && ai::login_state(a, cx) == Some(false) => Some(a),
            _ => None,
        };
        let scanning = ai::scanning(cx);
        let nothing = found.is_empty() && prefs.custom_command.trim().is_empty();
        let status = if scanning {
            tr("Sucht …")
        } else if found.is_empty() {
            if self.ai_rescanned { tr("Noch nichts gefunden") } else { tr("Keins gefunden") }
        } else if self.ai_rescanned {
            trf("Gefunden: %@", &[&names])
        } else {
            names
        };
        let next: Option<AnyElement> = if nothing {
            Some(Button::new("ai-install").primary().label(tr("Claude Code installieren")).tooltip(tr("Öffnet die offizielle Anleitung im Browser")).on_click(|_, _, cx| {
                if !crate::core::drive::fenced("open", CLAUDE_INSTALL) {
                    cx.open_url(CLAUDE_INSTALL);
                }
            }).into_any_element())
        } else {
            signed_out.map(|a| {
                let tip = if a == Agent::Claude { tr("Öffnet die Anmeldung bei Claude im Terminal") } else { tr("Öffnet die Anmeldung bei ChatGPT im Terminal") };
                Button::new("ai-login").primary().label(tr("Anmeldung prüfen")).tooltip(tip).on_click(move |_, _, cx| ai::login(a, cx)).into_any_element()
            })
        };
        let rescan = Button::new("ai-rescan").label(tr("Erneut suchen")).loading(scanning).on_click(cx.listener(|s, _, _, cx| {
            s.ai_rescanned = true;
            ai::recheck(cx);
            cx.notify();
        }));
        let programs = div().flex().flex_wrap().justify_end().items_center().gap(px(space::M)).child(caption(status, t)).children(next).child(rescan);
        let mut sections = div().w_full().flex().flex_col().gap(px(space::L)).child(section(
            None,
            Some(if nothing {
                tr("Der KI-Assistent beantwortet Fragen zu deinen Notizen, PDFs und Fristen und schlägt Änderungen vor, die du erst bestätigst. Du brauchst ein Konto bei Claude oder ChatGPT.")
            } else {
                tr("Das Programm nutzt dein eigenes Konto dort; was du ihm zeigst, geht an diesen Anbieter. Lesen darf es in deinem Studienordner, ändern nur mit deinem Ja, und jede Änderung ist rückgängig.")
            }),
            vec![
                if nothing {
                    row(tr("KI-Assistent verwenden"), Switch::new("ai-on").checked(false).disabled(true).tooltip(tr("Erst ein KI-Programm installieren")), t)
                } else {
                    switch_row("ai-on", tr("KI-Assistent verwenden"), prefs.enabled, false, |v, _, cx| ai::set_enabled(v, cx), t)
                },
                row(tr("Programme"), programs, t),
            ],
            t,
        ));
        if !ai::active() {
            return sections;
        }
        sections = sections.child(section(
            None,
            Some(tr("Riskant: Ein Befehl kann Dateien ändern, ohne dass Schrumbi es rückgängig machen kann. Die KI fragt vor jedem Befehl.")),
            vec![switch_row("ai-execute", tr("Befehle ausführen erlauben"), prefs.allow_execute, false, |v, _, cx| {
                prefs::update(|p| p.ai.allow_execute = v);
                cx.refresh_windows();
            }, t)],
            t,
        ));
        let open = self.advanced.contains(AI_ADVANCED) || jumping_to(ADVANCED_LABELS);
        sections = sections.child(
            link_button("ai-advanced", tr("Für Fortgeschrittene"), t)
                .flex()
                .items_center()
                .gap(px(space::XS))
                .child(motion::chevron("ai-advanced-chevron", open, t.accent))
                .on_click(cx.listener(|s, _, _, cx| {
                    if !s.advanced.remove(AI_ADVANCED) {
                        s.advanced.insert(AI_ADVANCED.to_string());
                    }
                    cx.notify();
                })),
        );
        if !open {
            return sections;
        }
        sections = sections.child(section(
            None,
            None,
            vec![switch_row("ai-toolbar", tr("Knopf in der Werkzeugleiste"), prefs.toolbar_button, false, |v, _, cx| {
                prefs::update(|p| p.ai.toolbar_button = v);
                cx.refresh_windows();
            }, t)],
            t,
        ));
        let mut column = Vec::new();
        let runners = agents::runners(&prefs);
        if runners.len() > 1 {
            let label = chosen.map(|r| tr(r.label())).unwrap_or_default();
            let menu_runners = runners.clone();
            column.push(row(tr("Programm in der Spalte"), Button::new("ai-runner").label(label).dropdown_menu(move |mut menu, _, _| {
                for r in menu_runners.clone() {
                    menu = menu.item(PopupMenuItem::new(tr(r.label())).checked(Some(r) == chosen).on_click(move |_, _, cx| {
                        prefs::update(|p| p.ai.agent = r.key().to_string());
                        cx.refresh_windows();
                    }));
                }
                menu
            }), t));
        }
        if chosen == Some(Runner::Known(Agent::Claude)) {
            let ready = install::claude_adapter().is_some() || detect::which("npx").is_some();
            let control: AnyElement = match ai::download(cx) {
                Some(Download::Running(p)) => caption(trf("Lädt … %@", &[&format!("{} %", (p * 100.).round() as i64)]), t).into_any_element(),
                Some(Download::Failed(e)) => div().flex().items_center().gap(px(space::S)).child(caption(e, t).max_w(px(control::ROW_CONTROL / 2.)).truncate()).child(Button::new("ai-fetch").label(tr("Erneut versuchen")).on_click(|_, _, cx| ai::install_claude(cx))).into_any_element(),
                None if ready => caption(tr("Bereit"), t).into_any_element(),
                None if install::downloadable() => Button::new("ai-fetch").label(tr("Holen")).on_click(|_, _, cx| ai::install_claude(cx)).into_any_element(),
                None => caption(tr("Braucht Node.js"), t).into_any_element(),
            };
            column.push(detail_row(tr("Claude verbinden"), tr("Ein kleines Zusatzprogramm, das Claude Code mit Schrumbi sprechen lässt"), control, t));
        }
        column.push(row(tr("Spalte öffnen"), Button::new("ai-open").label(format!("{}{}", tr("Öffnen"), crate::core::shortcuts::ShortcutAction::ToggleAi.hint())).on_click(|_, _, cx| {
            ai::open_column(cx);
            super::main_window::show(cx);
        }), t));
        column.push(switch_row("ai-follow", tr("Bei Änderungen zur Datei springen"), prefs.follow_edits, false, |v, _, cx| {
            prefs::update(|p| p.ai.follow_edits = v);
            cx.refresh_windows();
        }, t));
        let idle = prefs.idle_minutes;
        column.push(row(tr("Sitzung beenden nach"), Button::new("ai-idle").label(trf("%lld Minuten", &[&idle])).dropdown_menu(move |mut menu, _, _| {
            for m in [10u32, 30, 60, 120] {
                menu = menu.item(PopupMenuItem::new(trf("%lld Minuten", &[&m])).checked(m == idle).on_click(move |_, _, cx| {
                    prefs::update(|p| p.ai.idle_minutes = m);
                    cx.refresh_windows();
                }));
            }
            menu
        }), t));
        sections = sections.child(section(Some(tr("Spalte")), Some(tr("Die Spalte rechts teilt sich den Platz mit Details. Pause beendet das Programm, die nächste Nachricht startet es neu.")), column, t));
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
        let shown_root = root.clone();
        rows.push(row(tr("Im Terminal öffnen"), Button::new("ai-terminal").label(tr("Öffnen")).disabled(root.is_none()).on_click(move |_, _, _| {
            if let Some(root) = root.clone()
                && !crate::core::drive::fenced("terminal", root.display())
            {
                let _ = detect::open_terminal(&root);
            }
        }), t));
        let command = self.fields.input("ai-command", &prefs.custom_command, &tr("z. B. opencode acp"), window, cx, |v, cx| {
            prefs::update(|p| p.ai.custom_command = v);
            ai::rescan(cx);
        });
        rows.push(detail_row(tr("Eigener Befehl"), tr("Startet ein anderes Programm, das ACP spricht, in der Spalte"), div().w(px(control::ROW_CONTROL)).child(Input::new(&command)), t));
        let reach = tr("Danach kann jedes Claude oder Codex, das du in deinem Studienordner startest, deine Notizen, PDFs und Fristen lesen, solange Schrumbi läuft. Jede Änderung fragt hier zuerst.");
        let reach = match &shown_root {
            Some(r) => format!("{reach} {}", trf("Studienordner gerade: %@", &[&r.display().to_string()])),
            None => reach,
        };
        sections = sections.child(section(Some(tr("Für die Kommandozeile")), Some(reach), rows, t));
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
            Some(tr("Ein höheres Ziel heißt mehr Wiederholungen: Über 95 % kostet viel Zeit und bringt wenig.")),
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
            Some(tr("In Notizen mit #flashcards oder #karten werden auch markierte Stellen zu Lücken. Eine einzelne Notiz lässt sich in ihren Eigenschaften davon ausnehmen. Code und Formeln ergeben nie eine Karte.")),
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

    fn design_row(&self, i: usize, d: &themes::Installed, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (id, name) = (d.id.clone(), d.name.clone());
        let (state, color) = match &d.problem {
            Some(_) => (tr("Fehler in der Datei"), t.orange_text),
            None if d.dark => (tr("Dunkel"), t.secondary),
            None => (tr("Hell"), t.secondary),
        };
        div()
            .flex()
            .items_center()
            .gap(px(space::M))
            .px(px(space::L))
            .min_h(px(control::ROW))
            .py(px(space::S))
            .child(div().id(("g-theme-name", i)).flex_1().min_w_0().truncate().text_color(t.primary).tip(d.name.clone()).child(d.name.clone()))
            .child(div().id(("g-theme-state", i)).flex_none().text_size(px(font::CAPTION)).text_color(color).when_some(d.problem.clone(), |e, p| e.tip(p)).child(state))
            .child(Button::new(("g-theme-rm", i)).small().label(tr("Entfernen")).on_click(cx.listener(move |s, _, window, cx| {
                let was_active = prefs::with(|p| p.theme_file == id);
                s.theme_note = match theme::remove_file(&id, window, cx) {
                    Ok(bytes) => {
                        let stamp = s.note_later(cx);
                        Some(ThemeNote::Removed { id: id.clone(), name: name.clone(), bytes, was_active, stamp })
                    }
                    Err(m) => Some(ThemeNote::Problem(m)),
                };
                cx.notify();
            })))
            .into_any_element()
    }

    fn note_later(&mut self, cx: &mut Context<Self>) -> u64 {
        self.notices += 1;
        let stamp = self.notices;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(motion::NOTICE).await;
            let _ = this.update(cx, |s, cx| {
                if matches!(&s.theme_note, Some(ThemeNote::Removed { stamp: n, .. } | ThemeNote::Replaced { stamp: n, .. }) if *n == stamp) {
                    s.theme_note = None;
                    cx.notify();
                }
            });
        })
        .detach();
        stamp
    }

    fn theme_note_row(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let line = div().id("theme-note").w_full().min_w_0().flex().items_center().gap(px(space::S)).px(px(space::L)).min_h(px(control::ROW)).py(px(space::S)).text_size(px(font::CAPTION)).text_color(t.secondary);
        Some(match self.theme_note.as_ref()? {
            ThemeNote::Problem(m) => line
                .aria_label(m.clone())
                .child(div().flex_none().child(icon("circle-alert", iconsize::XS, t.orange_text)))
                .child(div().id("theme-note-text").flex_1().min_w_0().truncate().tip(m.clone()).child(m.clone()))
                .child({
                    let this = cx.entity();
                    icon_button("theme-note-close", "x", tr("Schließen"), false, t, move |_, cx| {
                        this.update(cx, |s, cx| {
                            s.theme_note = None;
                            cx.notify();
                        })
                    })
                })
                .into_any_element(),
            ThemeNote::Replaced { name, .. } => {
                let text = trf("Design „%@“ ersetzt", &[name]);
                line.aria_label(text.clone())
                    .child(div().flex_none().child(icon("check", iconsize::XS, t.green_text)))
                    .child(div().id("theme-note-text").min_w_0().truncate().tip(text.clone()).child(text))
                    .child(div().flex_none().text_color(t.tertiary).child("·"))
                    .child(div().flex_none().child(link_button("theme-undo", tr("Rückgängig"), t).on_click(cx.listener(|s, _, window, cx| {
                        if let Some(ThemeNote::Replaced { id, bytes, previous, .. }) = s.theme_note.take() {
                            match theme::undo_load(&id, &bytes, &previous) {
                                Ok(()) => theme::follow_undo(window, cx),
                                Err(m) => s.theme_note = Some(ThemeNote::Problem(m)),
                            }
                        }
                        cx.notify();
                    }))))
                    .into_any_element()
            }
            ThemeNote::Removed { name, .. } => {
                let text = trf("Design „%@“ entfernt", &[name]);
                line.aria_label(text.clone())
                    .child(div().flex_none().child(icon("check", iconsize::XS, t.green_text)))
                    .child(div().id("theme-note-text").min_w_0().truncate().tip(text.clone()).child(text))
                    .child(div().flex_none().text_color(t.tertiary).child("·"))
                    .child(div().flex_none().child(link_button("theme-undo", tr("Rückgängig"), t).on_click(cx.listener(|s, _, window, cx| {
                        if let Some(ThemeNote::Removed { id, bytes, was_active, .. }) = s.theme_note.take() {
                            match themes::restore(&id, &bytes) {
                                Ok(()) if was_active => theme::choose_file(&id, window, cx),
                                Ok(()) => {}
                                Err(e) => s.theme_note = Some(ThemeNote::Problem(trf("Design konnte nicht gespeichert werden: %@", &[&e]))),
                            }
                        }
                        cx.notify();
                    }))))
                    .into_any_element()
            }
        })
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

    fn shortcuts(&mut self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let query = self.search.read(cx).value().trim().to_string();
        let filter = query.to_lowercase();
        let any_custom = ShortcutAction::any_customized();
        let reset_all = div()
            .flex()
            .justify_end()
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
                .child(div().id("sc-notice-text").min_w_0().truncate().tip(n.text.clone()).child(n.text.clone()))
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
                .child(icon_button("sc-notice-close", "x", format!("{} ({})", tr("Schließen"), escape), false, t, { let this = cx.entity(); move |window, cx| this.update(cx, |s, cx| s.close_notice(window, cx)) }))
                .into_any_element(),
            None => {
                let idle = trf("Klick auf ein Kürzel und drück die neuen Tasten. %@ bricht ab, %@ entfernt das Kürzel.", &[&escape, &delete]);
                caption(idle.clone(), t).id("sc-idle").w_full().min_w_0().truncate().tip(idle).into_any_element()
            }
        };
        let header = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .px(px(space::XL))
            .pt(px(space::M))
            .pb(px(space::M))
            .border_b_1()
            .border_color(t.separator)
            .child(div().h(px(control::S)).w_full().min_w_0().flex().items_center().gap(px(space::S)).child(div().flex_1().min_w_0().child(line)).child(reset_all));
        let modules = module_names(cx);
        let mut lines: Vec<ShortcutLine> = Vec::new();
        for g in Group::ALL {
            let actions: Vec<ShortcutAction> = ShortcutAction::ALL.into_iter().filter(|a| a.group() == g && (*a != ShortcutAction::ToggleAi || super::ai::active()) && (filter.is_empty() || shortcut_fits(*a, &modules, &filter))).collect();
            if actions.is_empty() {
                continue;
            }
            lines.push(ShortcutLine::Head(g));
            let count = actions.len();
            for (i, a) in actions.into_iter().enumerate() {
                lines.push(ShortcutLine::Row(a, i == 0, i + 1 == count));
            }
        }
        if lines.is_empty() {
            let search = self.search.clone();
            let empty = unavailable("search", trf("Keine Treffer für „%@“", &[&query]), "", t)
                .h(px(240.))
                .child(Button::new("sc-clear-filter").label(tr("Suche zurücksetzen")).on_click(cx.listener(move |_, _, window, cx| search.update(cx, |f, cx| f.set_value("", window, cx)))));
            let scroller = div().id("sc-scroll").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.page()).p(px(space::XL));
            return div().size_full().flex().flex_col().child(header).child(scroller.child(empty));
        }
        let handles: HashMap<&'static str, FocusHandle> = lines.iter().filter_map(|l| if let ShortcutLine::Row(a, ..) = l { Some((a.id(), self.key_button(*a, cx))) } else { None }).collect();
        let ctx = Rc::new(ShortcutRows {
            me: cx.entity(),
            modules,
            clashes: ShortcutAction::all_conflicts(),
            handles,
            recording: self.recording,
            rejected: self.rejected.clone(),
            reserved: self.rejected_is_reserved(),
            any_custom,
            escape,
            delete,
        });
        let stops: Vec<(usize, FocusHandle)> = lines.iter().enumerate().filter_map(|(at, l)| if let ShortcutLine::Row(a, ..) = l { ctx.handles.get(a.id()).map(|h| (at, h.clone())) } else { None }).collect();
        let lines = Rc::new(lines);
        let count = lines.len();
        let list = uniform_list("sc-list", count, move |range, _, cx| {
            let t = Theme::get(cx);
            range.map(|i| shortcut_line(&lines[i], &ctx, &t, cx)).collect::<Vec<_>>()
        })
        .track_scroll(&self.shortcut_scroll)
        .size_full();
        let scroll = self.shortcut_scroll.clone();
        let walk = move |s: &mut Self, back: bool, window: &mut Window, cx: &mut Context<Self>| {
            let here = stops.iter().position(|(_, h)| h.is_focused(window));
            let next = here.filter(|_| s.recording.is_none()).and_then(|here| if back { here.checked_sub(1) } else { Some(here + 1).filter(|n| *n < stops.len()) });
            let Some(next) = next else {
                cx.propagate();
                return;
            };
            let (at, handle) = &stops[next];
            let head = at.saturating_sub(1);
            scroll.scroll_to_item(if back || stops.get(next.wrapping_sub(1)).is_none_or(|(prev, _)| prev + 1 != *at) { head } else { *at }, ScrollStrategy::Nearest);
            window.focus(handle, cx);
            cx.notify();
        };
        let walk = Rc::new(walk);
        let back = walk.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(space::XL))
                    .pt(px(space::M))
                    .pb(px(space::XL))
                    .key_context("WalkItems")
                    .on_action(cx.listener(move |s, _: &super::canvas_view::NextItem, window, cx| walk(s, false, window, cx)))
                    .on_action(cx.listener(move |s, _: &super::canvas_view::PreviousItem, window, cx| back(s, true, window, cx)))
                    .child(list),
            )
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
            if let Some(why) = action.refused_for(&s) {
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
            self.go(Tab::Sync);
        }
        if let Some(label) = cx.try_global::<OpenLit>().and_then(|o| o.0.clone()) {
            cx.set_global(OpenLit(None));
            if let Some((_, tab)) = SEARCHABLE.iter().find(|(k, _)| *k == label) {
                self.go(*tab);
                self.search.update(cx, |s, cx| s.set_value("", window, cx));
                self.light_up(tr(&label), window, cx);
            }
        }
        if cx.try_global::<OpenOnAi>().is_some_and(|o| o.0) {
            cx.set_global(OpenOnAi(false));
            self.go(Tab::Ai);
        }
        let t = Theme::get(cx);
        let config = state::read(cx, |l| l.config.clone());
        let tabs = self.tab_bar(window, &t, cx);
        let searching = !self.search.read(cx).value().trim().is_empty();
        let page = self.page();
        let body: AnyElement = match (config, self.tab) {
            _ if searching && self.tab != Tab::Shortcuts => self.search_results(&t, cx),
            (_, Tab::Notes) => div().id("notes-scroll").size_full().overflow_y_scroll().track_scroll(&page).p(px(space::XL)).child(self.notes(window, cx, &t)).into_any_element(),
            (_, Tab::Cards) => div().id("cards-scroll").size_full().overflow_y_scroll().track_scroll(&page).p(px(space::XL)).child(self.cards_tab(&t)).into_any_element(),
            (_, Tab::Sync) => div().id("sync-scroll").size_full().overflow_y_scroll().track_scroll(&page).p(px(space::XL)).child(self.sync_tab(cx, &t)).into_any_element(),
            (_, Tab::Ai) => div().id("ai-scroll").size_full().overflow_y_scroll().track_scroll(&page).p(px(space::XL)).child(self.ai_tab(window, cx, &t)).into_any_element(),
            (_, Tab::Shortcuts) => self.shortcuts(&t, cx).into_any_element(),
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
                    if window.context_stack().iter().any(|c| c.contains("Input")) {
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
                    let listing = s.tab != Tab::Shortcuts;
                    match e.keystroke.key.as_str() {
                        "down" if listing => s.hit = (s.hit + 1).min(s.matches(cx).len().saturating_sub(1)),
                        "up" if listing => s.hit = s.hit.saturating_sub(1),
                        "escape" => s.search.update(cx, |f, cx| f.set_value("", window, cx)),
                        _ => return,
                    }
                    cx.stop_propagation();
                    cx.notify();
                } else if e.keystroke.key == "f" && e.keystroke.modifiers.secondary(cx) {
                    s.search.update(cx, |f, cx| f.focus(window, cx));
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
