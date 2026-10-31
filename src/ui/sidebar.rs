use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::act;
use super::motion::Soft;
use super::theme::{Theme, font, space, iconsize, radius, control};
use super::widgets::{KeyNav, icon, tip};
use crate::core::i18n::{tr, trf};
use crate::core::library::{self, Library, Route};
use crate::core::prefs;

/// Height of the title band that holds the traffic lights.
pub const BAND: f32 = if super::chrome::CUSTOM { 8. } else { 52. };
/// Width of the sidebar when collapsed: just the icons.
pub const RAIL: f32 = 48.;
/// Width of the sidebar with labels.
pub const WIDTH: f32 = 220.;
const ROW: f32 = 44.;

fn is_active(lib: &Library, route: &Route) -> bool {
    lib.route() == route || matches!((lib.route(), route), (Route::Doc(_), Route::Doc(_)))
}

fn navigate(route: Route, cx: &mut App) {
    act(cx, |l| if let Route::Doc(_) = route { l.show_documents() } else { l.set_route(route.clone()) });
}

/// Clicking the selected tab shows or hides the labels, any other tab just navigates.
fn tab_click(route: Route, window: &mut Window, cx: &mut App) {
    if super::state::read(cx, |l| is_active(l, &route)) {
        super::main_window::toggle_sidebar(window, cx);
    } else {
        navigate(route, cx);
    }
}

struct Entry {
    route: Route,
    icon: &'static str,
    label: String,
    shortcut: Option<&'static str>,
    badge: usize,
    urgent: bool,
}

fn entries(lib: &Library) -> Vec<Entry> {
    let issues = lib.misnamed().len() + usize::from(lib.missing_folder_count > 0);
    let mut v = vec![
        Entry { route: Route::Today, icon: "layout-grid", label: tr("Übersicht"), shortcut: Some("1"), badge: lib.action_items().len(), urgent: true },
        Entry { route: Route::Grid, icon: "table", label: tr("Semester"), shortcut: Some("2"), badge: 0, urgent: false },
        Entry { route: Route::Record, icon: "award", label: tr("Leistungen"), shortcut: Some("3"), badge: 0, urgent: false },
        Entry { route: Route::Inbox, icon: "inbox", label: tr("Inbox"), shortcut: Some("4"), badge: lib.inbox.len(), urgent: false },
        Entry { route: Route::Search, icon: "search", label: tr("Suche"), shortcut: None, badge: 0, urgent: false },
    ];
    if let Some(doc) = lib.active_document.clone().or_else(|| lib.open_documents.first().cloned()) {
        v.push(Entry { route: Route::Doc(doc), icon: "files", label: tr("Dokumente"), shortcut: None, badge: lib.open_documents.len(), urgent: false });
    }
    v.push(Entry { route: Route::Checks, icon: "list-checks", label: tr("Prüfen"), shortcut: Some("5"), badge: issues, urgent: false });
    v
}

fn shortcut_label(key: &str) -> String {
    if cfg!(target_os = "macos") { format!("⌘{key}") } else { format!("Ctrl+{key}") }
}

fn badge(n: usize, tint: Hsla) -> Div {
    div()
        .absolute()
        .bottom(px(space::XS))
        .right(px(space::XXS))
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

fn count_pill(n: usize, tint: Hsla) -> Div {
    div().flex_none().min_w(px(iconsize::M)).px(px(space::XS)).rounded_full().bg(tint).flex().items_center().justify_center().text_size(px(font::CAPTION2)).font_weight(FontWeight::BOLD).text_color(white()).child(if n > 99 { "99+".to_string() } else { n.to_string() })
}

fn label_view(label: String, trailing: Option<Div>) -> Div {
    div().w(px(WIDTH - RAIL - space::M)).flex_none().flex().items_center().gap(px(space::S)).text_size(px(font::BODY)).child(div().flex_1().min_w_0().truncate().child(label)).children(trailing)
}

/// One row; the count sits behind the label when there is room, otherwise on the glyph.
#[allow(clippy::too_many_arguments)]
fn tab(id: SharedString, active: bool, dim: bool, glyph: AnyElement, label: String, count: Option<(usize, Hsla)>, wide: bool, help: String, t: &Theme) -> Stateful<Div> {
    let (accent, selected, hover) = (t.accent, t.ink.opacity(0.08), t.ink.opacity(0.05));
    div()
        .id(id)
        .relative()
        .w_full()
        .h(px(ROW))
        .flex_none()
        .flex()
        .items_center()
        .cursor_pointer().key_nav()
        .overflow_hidden()
        .when(active, |d| d.bg(selected).child(div().absolute().left_0().top(px(space::M)).bottom(px(space::M)).w(px(space::XXS)).rounded_r(px(space::XXS)).bg(accent)))
        .when(!active, |d| d.soft(hover, 0.).hover(|s| s.opacity(1.)))
        .child(
            div()
                .relative()
                .w(px(RAIL))
                .h_full()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(glyph)
                .when_some(count.filter(|_| !wide), |d, (n, tint)| d.child(badge(n, tint))),
        )
        .child(label_view(label, count.filter(|_| wide).map(|(n, tint)| count_pill(n, tint))))
        .when(dim && !active, |d| d.opacity(0.65))
        .tooltip(tip(help))
}

/// An invisible button over a tab, so the row can open a menu and still look like every other tab.
fn menu_overlay(id: &'static str, help: String) -> Button {
    Button::new(id).ghost().w_full().h(px(ROW)).tooltip(help)
}

/// The sidebar: one column of tabs. The icons always show, the labels sit beside them when wide enough.
pub fn render(lib: &Library, update: Option<String>, wide: bool, t: &Theme) -> Div {
    let fg = t.primary;
    let mut top = div().flex().flex_col();
    for e in entries(lib) {
        let active = is_active(lib, &e.route);
        let help = match e.shortcut {
            Some(k) => format!("{}  {}", e.label, shortcut_label(k)),
            None => e.label.clone(),
        };
        let route = e.route.clone();
        let is_inbox = e.route == Route::Inbox;
        let count = (e.badge > 0).then_some((e.badge, if e.urgent { t.red } else { t.accent }));
        let row = tab(SharedString::from(format!("tab-{:?}", e.route)), active, true, icon(e.icon, iconsize::L, fg).into_any_element(), e.label, count, wide, help, t).on_click(move |_, window, cx| tab_click(route.clone(), window, cx));
        if is_inbox {
            let dir = lib.root.as_ref().zip(lib.config.as_ref()).map(|(r, c)| r.join(&c.inbox));
            top = top.child(row.context_menu(move |menu, _, _| {
                let dir = dir.clone();
                menu.item(PopupMenuItem::new(super::platform::reveal_label()).on_click(move |_, _, _| {
                    if let Some(d) = &dir {
                        library::open_path(d);
                    }
                }))
            }));
        } else {
            top = top.child(row);
        }
    }
    let modules = lib.semester_modules();
    let mut by_module: HashMap<&str, usize> = HashMap::new();
    for a in lib.action_items() {
        *by_module.entry(a.module_id.as_str()).or_default() += 1;
    }
    let mut mods = div().flex().flex_col();
    for m in &modules {
        let route = Route::Module(m.id.clone());
        let active = is_active(lib, &route);
        let color = t.palette(&m.color);
        let code: String = m.code.chars().take(3).collect();
        let chip = div()
            .size(px(control::S))
            .rounded(px(radius::CONTROL))
            .bg(color.opacity(if active { 0.9 } else { 0.22 }))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(if code.chars().count() > 2 { font::CAPTION2 } else { font::CAPTION }))
            .font_weight(FontWeight::BOLD)
            .text_color(if active { white() } else { color })
            .child(code)
            .into_any_element();
        let count = by_module.get(m.id.as_str()).copied().filter(|n| *n > 0).map(|n| (n, t.red));
        let links: Vec<(String, String)> = m.links.iter().filter_map(|k| k.url().map(|u| (if k.title.is_empty() { tr("Link") } else { k.title.clone() }, u.to_string()))).collect();
        let folder = lib.root.as_ref().map(|r| r.join(&m.folder));
        mods = mods.child(
            tab(SharedString::from(format!("tab-module-{}", m.id)), active, false, chip, m.display_name(), count, wide, m.display_name(), t)
                .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(color.opacity(0.15)))
                .drag_over::<super::browser::DraggedFile>(move |s, _, _, _| s.bg(color.opacity(0.15)))
                .on_drop(|paths: &ExternalPaths, _, cx| {
                    let paths = paths.paths().to_vec();
                    act(cx, |l| l.request_filing(paths, None));
                })
                .on_drop(|f: &super::browser::DraggedFile, _, cx| {
                    let path = f.0.clone();
                    act(cx, |l| l.request_filing(vec![path], None));
                })
                .on_click(move |_, window, cx| tab_click(route.clone(), window, cx))
                .context_menu(move |mut menu, _, _| {
                    for (title, url) in &links {
                        let url = url.clone();
                        menu = menu.item(PopupMenuItem::new(trf("%@ öffnen", &[title])).on_click(move |_, _, _| {
                            let _ = opener::open(&url);
                        }));
                    }
                    if let Some(f) = folder.clone() {
                        menu = menu.item(PopupMenuItem::new(tr("Ordner öffnen")).on_click(move |_, _, _| library::open_path(&f)));
                    }
                    menu
                }),
        );
    }
    let recents = prefs::get().recent_workspaces;
    let current = lib.root.clone();
    let workspace = lib.config.as_ref().map(|c| c.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| tr("Arbeitsordner"));
    let semesters = lib.config.as_ref().map(|c| c.semesters.clone()).unwrap_or_default();
    let semester = lib.semester().map(|s| s.name.clone()).unwrap_or_default();
    let selected_id = lib.selected_semester_id.clone();
    let bottom = div()
        .flex()
        .flex_col()
        .pb(px(space::S))
        .when_some(update, |d, version| {
            let label = trf("Update auf %@", &[&version]);
            d.child(tab("tab-update".into(), false, false, icon("download", iconsize::L, t.accent).into_any_element(), label.clone(), None, wide, label, t).on_click(|_, window, cx| super::updates::show(window, cx)))
        })
        .when(semesters.len() > 1, |d| {
            d.child(tab("tab-semester-row".into(), false, true, icon("calendar-range", iconsize::L, fg).into_any_element(), semester.clone(), None, wide, semester.clone(), t).child(div().absolute().inset_0().child(menu_overlay("tab-semester", semester.clone()).dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, _| {
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
        .child(tab("tab-workspace-row".into(), false, true, icon("folder-open", iconsize::L, fg).into_any_element(), workspace, None, wide, tr("Arbeitsordner"), t).child(div().absolute().inset_0().child(menu_overlay("tab-workspace", tr("Arbeitsordner")).dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, _| {
            for (i, p) in recents.iter().enumerate() {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                menu = menu.item(PopupMenuItem::new(name).checked(current.as_ref() == Some(p)).on_click(move |_, window, cx| window.dispatch_action(Box::new(super::app::OpenRecent(i)), cx)));
            }
            menu.separator().item(PopupMenuItem::new(tr("Anderen Ordner öffnen…")).on_click(|_, window, cx| window.dispatch_action(Box::new(super::app::OpenOtherWorkspace), cx)))
        }))))
        .child(tab("tab-settings".into(), false, true, icon("settings", iconsize::L, fg).into_any_element(), tr("Einstellungen"), None, wide, format!("{}  {}", tr("Einstellungen"), shortcut_label(",")), t).on_click(|_, _, cx| super::settings::open(cx)));
    div()
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(top)
        .when(!modules.is_empty(), |d| d.child(div().mx(px(space::M)).my(px(space::S)).h(px(1.)).bg(t.separator)))
        .child(div().id("tab-modules").flex_1().min_h_0().overflow_y_scrollbar().child(mods))
        .child(bottom)
}
