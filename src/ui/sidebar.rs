use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::act;
use super::drive::Mark;
use super::motion::Soft;
use super::theme::{Theme, font, space, iconsize, radius, control};
use super::widgets::{KeyNav, icon, tip};
use crate::core::i18n::{tr, trf};
use crate::core::library::{self, Library, Route};
use crate::core::prefs::{self, Shortcut};
use crate::core::shortcuts::ShortcutAction;

/// Height of the title band that holds the traffic lights.
pub const BAND: f32 = if super::chrome::CUSTOM { 8. } else { 52. };
/// Width of the sidebar when it shows only icons.
pub const RAIL: f32 = 48.;
/// Width of the sidebar with the names beside the icons.
pub const WIDE: f32 = 208.;

thread_local! {
    static MODULES_SCROLL: ScrollHandle = ScrollHandle::new();
}
const ROW: f32 = 44.;
const ROW_WIDE: f32 = 36.;
/// Height of the fade that shows the module list scrolls.
const FADE: f32 = 16.;

/// Window width below which the sidebar keeps only icons, so the page keeps its room.
const NAMES_MIN_WINDOW: f32 = 820.;

thread_local! {
    static NAMES: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Whether the user chose icons only; kept per device.
pub fn compact() -> bool {
    prefs::with(|p| p.sidebar_compact)
}

/// Whether the sidebar shows names at this window width.
pub fn names_shown(window_width: f32) -> bool {
    !compact() && window_width >= NAMES_MIN_WINDOW
}

/// The width of the sidebar at this window width.
pub fn rail_for(window_width: f32) -> f32 {
    if names_shown(window_width) { WIDE } else { RAIL }
}

fn names() -> bool {
    NAMES.get()
}

fn row() -> f32 {
    if names() { ROW_WIDE } else { ROW }
}

/// Switches between icons only and icons with names.
pub fn toggle_compact(cx: &mut App) {
    prefs::update(|p| p.sidebar_compact = !p.sidebar_compact);
    super::state::lib(cx).update(cx, |_, cx| cx.notify());
}


fn is_active(lib: &Library, route: &Route) -> bool {
    lib.route() == route || matches!((lib.route(), route), (Route::Doc(_), Route::Notes))
}

fn navigate(route: Route, cx: &mut App) {
    act(cx, |l| match route {
        Route::Notes => l.show_notes(),
        _ => l.set_route(route.clone()),
    });
}

struct Entry {
    /// Set on entries that can be hidden.
    key: Option<&'static str>,
    route: Route,
    icon: &'static str,
    label: String,
    shortcut: Option<ShortcutAction>,
    badge: usize,
    urgent: bool,
}

fn entries(lib: &Library) -> Vec<Entry> {
    vec![
        Entry { key: None, route: Route::Today, icon: "layout-grid", label: tr("Übersicht"), shortcut: Some(ShortcutAction::GoToday), badge: lib.action_items().len(), urgent: true },
        Entry { key: Some("semester"), route: Route::Grid, icon: "table", label: tr("Semester"), shortcut: Some(ShortcutAction::GoGrid), badge: 0, urgent: false },
        Entry { key: Some("record"), route: Route::Record, icon: "award", label: tr("Leistungen"), shortcut: Some(ShortcutAction::GoRecord), badge: 0, urgent: false },
        Entry { key: Some("inbox"), route: Route::Inbox, icon: "inbox", label: tr("Inbox"), shortcut: Some(ShortcutAction::GoInbox), badge: lib.inbox.len(), urgent: false },
        Entry { key: Some("notes"), route: Route::Notes, icon: "book-open", label: tr("Notizen"), shortcut: Some(ShortcutAction::GoNotes), badge: 0, urgent: false },
        Entry { key: Some("cards"), route: Route::Cards, icon: "layers", label: tr("Karten"), shortcut: Some(ShortcutAction::GoCards), badge: crate::core::cards::counts("", crate::core::cards::now()).total(), urgent: false },
    ]
}

/// Shows or hides a sidebar entry.
pub fn set_hidden(key: &str, hidden: bool, cx: &mut App) {
    prefs::update(|p| {
        p.hidden_tabs.retain(|k| k != key);
        if hidden {
            p.hidden_tabs.push(key.to_string());
        }
    });
    super::state::lib(cx).update(cx, |_, cx| cx.notify());
}

/// The first three modules answer to their own shortcuts, in sidebar order.
fn module_help(name: String, index: usize) -> String {
    match ShortcutAction::for_module(index).and_then(|a| a.active_shortcut()) {
        Some(s) => format!("{name} ({})", s.display()),
        None => name,
    }
}

fn shortcut_label(key: &str) -> String {
    Shortcut { key: key.into(), command: true, shift: false, option: false, control: false }.display()
}

fn badge(n: usize, tint: Hsla) -> Div {
    pill(n, tint).absolute().top(px(space::XXS)).right(px(0.))
}

fn pill(n: usize, tint: Hsla) -> Div {
    div()
        .min_w(px(iconsize::M))
        .h(px(iconsize::M))
        .px(px(space::XS))
        .rounded_full()
        .bg(tint)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(font::CAPTION2))
        .font_weight(FontWeight::BOLD)
        .text_color(white())
        .child(if n > 99 { "99+".to_string() } else { n.to_string() })
}

/// One row: the glyph with its count on it and, unless the sidebar is compact, the name beside it; the shortcut lives in the tooltip.
#[allow(clippy::too_many_arguments)]
fn tab(id: SharedString, active: bool, dim: bool, glyph: AnyElement, label: String, count: Option<(usize, Hsla)>, help: String, t: &Theme) -> Stateful<Div> {
    let (accent, selected, hover) = (t.accent, t.ink.opacity(0.08), t.ink.opacity(0.05));
    let wide = names();
    let faded = dim && !active;
    let shown = label.clone();
    div()
        .id(id)
        .mark("tab", label)
        .group("sidebar-tab")
        .relative()
        .w(px(if wide { WIDE } else { RAIL } - 2. * space::XS))
        .mx(px(space::XS))
        .h(px(row()))
        .flex_none()
        .flex()
        .items_center()
        .rounded(px(radius::CONTROL))
        .cursor_pointer().key_nav()
        .overflow_hidden()
        .when(active, |d| d.bg(selected).child(div().absolute().left_0().top(px(space::S)).bottom(px(space::S)).w(px(space::XXS)).rounded_r(px(space::XXS)).bg(accent)))
        .when(!active, |d| d.soft(hover, radius::CONTROL))
        .child(
            div()
                .relative()
                .w(px(RAIL - 2. * space::XS))
                .h_full()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(div().when(faded, |d| d.opacity(0.7).group_hover("sidebar-tab", |s| s.opacity(1.))).child(glyph))
                .when_some(count.filter(|_| !wide), |d, (n, tint)| d.child(badge(n, tint))),
        )
        .when(wide, |d| {
            d.child(div().flex_1().min_w_0().truncate().when(faded, |d| d.opacity(0.7).group_hover("sidebar-tab", |s| s.opacity(1.))).when(active, |d| d.font_weight(FontWeight::MEDIUM)).child(shown))
                .when_some(count, |d, (n, tint)| d.child(pill(n, tint).flex_none().mr(px(space::S))))
        })
        .tooltip(tip(help))
}

/// An invisible button over a tab, so the row can open a menu and still look like every other tab.
fn menu_overlay(id: &'static str) -> Button {
    Button::new(id).ghost().w_full().h(px(row()))
}

/// The sidebar: the pages, the modules and the settings, with their names unless the user chose icons only.
pub fn render(lib: &Library, update: Option<String>, window_width: f32, t: &Theme, cx: &App) -> Div {
    NAMES.set(names_shown(window_width));
    let fg = t.primary;
    let mut top = div().flex().flex_col();
    let hidden = prefs::get().hidden_tabs;
    for e in entries(lib) {
        let active = is_active(lib, &e.route);
        if !active && e.key.is_some_and(|k| hidden.iter().any(|h| h == k)) {
            continue;
        }
        let (hide_key, hide_label) = (e.key, e.label.clone());
        let help = match e.shortcut.and_then(|a| a.active_shortcut()) {
            Some(k) => format!("{} ({})", e.label, k.display()),
            None => e.label.clone(),
        };
        let route = e.route.clone();
        let is_inbox = e.route == Route::Inbox;
        let count = (e.badge > 0).then_some((e.badge, t.badge_fill(if e.urgent { t.gray } else { t.accent })));
        let row = tab(SharedString::from(format!("tab-{:?}", e.route)), active, true, icon(e.icon, iconsize::L, fg).into_any_element(), e.label, count, help, t).on_click(move |_, _, cx| navigate(route.clone(), cx));
        let Some(key) = hide_key else {
            top = top.child(row);
            continue;
        };
        let dir = lib.root.as_ref().zip(lib.config.as_ref()).map(|(r, c)| r.join(&c.inbox)).filter(|_| is_inbox);
        top = top.child(row.context_menu(move |mut menu, _, _| {
            if let Some(dir) = dir.clone() {
                menu = menu.item(PopupMenuItem::new(super::platform::reveal_label()).on_click(move |_, _, _| library::open_folder(&dir))).separator();
            }
            let label = hide_label.clone();
            menu.item(PopupMenuItem::new(tr("Aus der Seitenleiste ausblenden")).on_click(move |_, _, cx| {
                set_hidden(key, true, cx);
                let label = label.clone();
                act(cx, |l| {
                    l.notify(trf("%@ ausgeblendet", &[&label]), "eye-off", Some(tr("Einblenden")), Some(Box::new(move |_: &mut Library| prefs::update(|p| p.hidden_tabs.retain(|k| k != key)))));
                });
            }))
        }));
    }
    let modules = lib.semester_modules();
    let mut by_module: HashMap<&str, usize> = HashMap::new();
    for a in lib.action_items() {
        *by_module.entry(a.module_id.as_str()).or_default() += 1;
    }
    let mut mods = div().flex().flex_col();
    for (index, m) in modules.iter().enumerate() {
        let route = Route::Module(m.id.clone());
        let active = is_active(lib, &route);
        let color = t.palette(&m.color);
        let code = m.badge();
        let chip = div()
            .size(px(control::M))
            .rounded(px(radius::CONTROL))
            .overflow_hidden()
            .bg(color.opacity(if active { 0.9 } else { 0.22 }))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(if code.chars().count() > 2 { font::CAPTION2 } else { font::CAPTION }))
            .font_weight(FontWeight::BOLD)
            .text_color(if active { t.on_fill(color) } else { t.readable(color) })
            .child(code)
            .into_any_element();
        let count = by_module.get(m.id.as_str()).copied().filter(|n| *n > 0).map(|n| (n, t.badge_fill(t.gray)));
        let links: Vec<(String, String)> = m.links.iter().filter_map(|k| k.url().map(|u| (if k.title.is_empty() { tr("Link") } else { k.title.clone() }, u.to_string()))).collect();
        let folder = lib.root.as_ref().map(|r| r.join(&m.folder));
        let (outside, inside, edit_id) = (m.id.clone(), m.id.clone(), m.id.clone());
        mods = mods.child(
            tab(SharedString::from(format!("tab-module-{}", m.id)), active, false, chip, if m.name.is_empty() { m.code.clone() } else { m.name.clone() }, count, module_help(m.display_name(), index), t)
                .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(color.opacity(0.15)))
                .drag_over::<super::browser::DraggedFile>(move |s, _, _, _| s.bg(color.opacity(0.15)))
                .on_drop(move |paths: &ExternalPaths, _, cx| {
                    let paths = paths.paths().to_vec();
                    act(cx, |l| l.request_filing(paths, l.next_in_module(&outside)));
                })
                .on_drop(move |f: &super::browser::DraggedFile, _, cx| {
                    let path = f.0.clone();
                    act(cx, |l| l.request_filing(vec![path], l.next_in_module(&inside)));
                })
                .on_click(move |_, _, cx| navigate(route.clone(), cx))
                .context_menu(move |mut menu, _, _| {
                    for (title, url) in &links {
                        let url = url.clone();
                        menu = menu.item(PopupMenuItem::new(trf("%@ öffnen", &[title])).on_click(move |_, _, _| {
                            crate::core::library::open_path(&url);
                        }));
                    }
                    let edit = edit_id.clone();
                    menu = menu.item(PopupMenuItem::new(tr("Modul bearbeiten")).on_click(move |_, _, cx| super::settings::open_module(&edit, cx)));
                    if let Some(f) = folder.clone() {
                        menu = menu.item(PopupMenuItem::new(tr("Ordner öffnen")).on_click(move |_, _, _| library::open_folder(&f)));
                    }
                    menu
                }),
        );
    }
    let recents = prefs::get().recent_workspaces;
    let current = lib.root.clone();
    let name = lib.config.as_ref().map(|c| c.name.clone()).filter(|n| !n.is_empty());
    let workspace_help = name.as_ref().map_or_else(|| tr("Arbeitsordner"), |n| format!("{}: {n}", tr("Arbeitsordner")));
    let workspace = name.unwrap_or_else(|| tr("Arbeitsordner"));
    let semesters = lib.config.as_ref().map(|c| c.semesters.clone()).unwrap_or_default();
    let semester = lib.semester().map(|s| s.name.clone()).unwrap_or_default();
    let selected_id = lib.selected_semester_id.clone();
    let wide = !compact();
    let toggle_label = if wide { tr("Seitenleiste einklappen") } else { tr("Seitenleiste ausklappen") };
    let can_toggle = window_width >= NAMES_MIN_WINDOW;
    let bottom = div()
        .flex()
        .flex_col()
        .pb(px(space::S))
        .when_some(update, |d, version| {
            let label = trf("Update auf %@", &[&version]);
            d.child(tab("tab-update".into(), false, false, icon("download", iconsize::L, t.accent).into_any_element(), label.clone(), None, label, t).on_click(|_, window, cx| super::updates::show(window, cx)))
        })
        .when_some(sync_row(t, cx), |d, row| d.child(row))
        .when(semesters.len() > 1, |d| {
            d.child(tab("tab-semester-row".into(), false, true, icon("calendar-range", iconsize::L, fg).into_any_element(), semester.clone(), None, format!("{}: {semester}", tr("Semester")), t).child(div().absolute().inset_0().child(menu_overlay("tab-semester").dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, _| {
                for s in &semesters {
                    let id = s.id.clone();
                    menu = menu.item(PopupMenuItem::new(s.name.clone()).checked(selected_id.as_deref() == Some(s.id.as_str())).on_click(move |_, _, cx| {
                        act(cx, |l| {
                            l.selected_semester_id = Some(id.clone());
                            l.refresh();
                        })
                    }));
                }
                menu
            }))))
        })
        .child(tab("tab-workspace-row".into(), false, true, icon("folder-open", iconsize::L, fg).into_any_element(), workspace, None, workspace_help, t).child(div().absolute().inset_0().child(menu_overlay("tab-workspace").dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, _| {
            for (i, p) in recents.iter().enumerate() {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                menu = menu.item(PopupMenuItem::new(name).checked(current.as_ref() == Some(p)).on_click(move |_, window, cx| window.dispatch_action(Box::new(super::app::OpenRecent(i)), cx)));
            }
            menu.separator().item(PopupMenuItem::new(format!("{}{}", tr("Anderen Ordner öffnen…"), ShortcutAction::OpenOtherWorkspace.hint())).on_click(|_, window, cx| window.dispatch_action(Box::new(super::app::OpenOtherWorkspace), cx)))
        }))))
        .child(tab("tab-settings".into(), false, true, icon("settings", iconsize::L, fg).into_any_element(), tr("Einstellungen"), None, format!("{} ({})", tr("Einstellungen"), shortcut_label(",")), t).on_click(|_, _, cx| super::settings::open(cx)))
        .when(can_toggle, |d| d.child(tab("tab-sidebar-toggle".into(), false, true, icon(if wide { "panel-left-close" } else { "panel-left-open" }, iconsize::L, fg).into_any_element(), toggle_label.clone(), None, toggle_label, t).on_click(|_, _, cx| toggle_compact(cx))));
    let scroll = MODULES_SCROLL.with(Clone::clone);
    super::coach::scrolls("modules", &scroll);
    let (offset, reach) = (scroll.offset().y, scroll.max_offset().y);
    let more_above = offset < px(-1.);
    let more_below = reach > px(1.) && -offset < reach - px(1.);
    div()
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(top)
        .when(!modules.is_empty(), |d| d.child(div().mx(px(space::M)).my(px(space::S)).h(px(1.)).bg(t.separator)))
        .child(div().flex_1().min_h_0().relative().child(div().id("tab-modules").size_full().overflow_y_scroll().track_scroll(&scroll).child(mods)).vertical_scrollbar(&scroll).children(fade(true, more_above, t)).children(fade(false, more_below, t)))
        .child(bottom)
}

/// A fade at the top or bottom of the module list while more modules lie beyond it.
fn fade(top: bool, shown: bool, t: &Theme) -> Option<Div> {
    shown.then(|| div().absolute().left_0().right_0().h(px(FADE)).when(top, Styled::top_0).when(!top, Styled::bottom_0).bg(linear_gradient(if top { 180. } else { 0. }, linear_color_stop(t.sidebar, 0.), linear_color_stop(t.sidebar.opacity(0.), 1.))))
}

/// The sync state, always in the same place; the row opens what the state needs.
fn sync_row(t: &Theme, cx: &App) -> Option<AnyElement> {
    use super::sync_panel::RowKind;
    let row = super::sync_panel::row(t, cx)?;
    let fg = t.primary;
    let glyph = if row.working {
        super::motion::loading(iconsize::L, fg).into_any_element()
    } else {
        div().relative().child(icon(row.icon, iconsize::L, row.tint.unwrap_or(fg))).child(super::motion::done_pulse(("sync-done", row.finished), row.finished > 0, iconsize::L)).into_any_element()
    };
    let dim = row.tint.is_none() && row.count.is_none() && !row.working;
    let spoken = format!("{}: {}", tr("Synchronisierung"), row.label);
    let base = tab("tab-sync".into(), false, dim, glyph, row.label, row.count, row.help.clone(), t).aria_label(spoken);
    Some(match row.kind {
        RowKind::Offer => base
            .on_click(|_, _, cx| act(cx, |l| l.show_sync_setup = true))
            .context_menu(|menu, _, _| menu.item(PopupMenuItem::new(tr("Nicht mehr anbieten")).on_click(|_, _, cx| super::sync::hide_offer(cx))))
            .into_any_element(),
        RowKind::Conflicts => base.on_click(|_, _, cx| act(cx, |l| l.show_conflicts = true)).into_any_element(),
        RowKind::Panel => base
            .child(div().absolute().inset_0().child(super::sync_panel::overlay(row.help, cx)))
            .context_menu(|menu, _, _| {
                menu.item(PopupMenuItem::new(tr("Jetzt synchronisieren")).on_click(|_, _, cx| super::sync::run(None, true, cx)))
                    .item(PopupMenuItem::new(tr("Einstellungen")).on_click(|_, _, cx| super::settings::open_sync(cx)))
            })
            .into_any_element(),
    })
}
