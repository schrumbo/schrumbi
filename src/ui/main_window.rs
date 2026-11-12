use gpui_kit::component::{Disableable, Sizable};
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::WindowExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::bar_fit;
use super::inspector::Inspector;
use super::motion::Soft;
use super::motion;
use super::pages::Pages;
use super::state::{self, act};
use super::theme::{self, Theme, crumb, font, hover, space, iconsize, radius, control};
use super::widgets::*;
use super::{sheets, sidebar, today};
use crate::core::i18n::{tr, trf};
use crate::core::library::{Library, NewNoteRequest, Route};
use crate::core::shortcuts::ShortcutAction;

actions!(schrumbi_main, [Dismiss, FocusSearch, SearchUp, SearchDown]);

/// The main window: icon rail, toolbar, the routed page and the inspector.
pub struct MainWindow {
    search: Entity<InputState>,
    inspector: Entity<Inspector>,
    pages: Entity<Pages>,
    today: Entity<TodayView>,
    save_bounds: Option<Task<()>>,
    syncing_search: bool,
    /// What the search field says while empty: where it searches.
    search_placeholder: String,
    focus: FocusHandle,
    focused_once: bool,
    page_refocused: Option<std::time::Instant>,
    shown_route: Option<Route>,
    slides: Option<(std::path::PathBuf, Entity<super::vault_tools::Slides>)>,
    error_shown: bool,
    /// The selection count and place when a press on the page began, if nothing inside claimed it.
    press_stamp: Option<(u64, Point<Pixels>)>,
    ai_panel: Option<Entity<super::ai_panel::AiPanel>>,
    /// The right panel's width while its edge is dragged.
    resizing: Option<f32>,
    _subs: Vec<Subscription>,
}

const PANEL_WIDTH: f32 = 360.;
const PANEL_MIN: f32 = 280.;
const PANEL_MAX: f32 = 640.;
const PAGE_KEEPS: f32 = 480.;
const FLOAT_GAP: f32 = 48.;
const EDGE_GRIP: f32 = 6.;

/// The edge of the right panel being dragged.
#[derive(Clone)]
struct PanelEdge;

impl Render for PanelEdge {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn wanted_width() -> f32 {
    crate::core::prefs::with(|p| p.inspector_width.or(p.ai.width)).unwrap_or(PANEL_WIDTH)
}

fn docked_width(window_width: f32) -> f32 {
    wanted_width().clamp(PANEL_MIN, PANEL_MAX.min((window_width - sidebar::rail_for(window_width) - PAGE_KEEPS).max(PANEL_MIN)))
}

/// Whether the right panel floats over the page because beside it the page would get too narrow.
fn panel_floats(window_width: f32) -> bool {
    window_width - sidebar::rail_for(window_width) - docked_width(window_width) < PAGE_KEEPS
}

fn panel_max(window_width: f32) -> f32 {
    let keeps = if panel_floats(window_width) { FLOAT_GAP } else { PAGE_KEEPS };
    PANEL_MAX.min((window_width - sidebar::rail_for(window_width) - keeps).max(PANEL_MIN))
}

/// How wide the right panel is, for Details and KI alike: what the user dragged it to, within what the window leaves.
fn panel_width(window_width: f32) -> f32 {
    wanted_width().clamp(PANEL_MIN, panel_max(window_width))
}

impl MainWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        super::setup_card::register_undo(cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Suchen")));
        let inspector = cx.new(|cx| Inspector::new(window, cx));
        let pages = cx.new(|cx| Pages::new(window, cx));
        let today = cx.new(TodayView::new);
        let lib = state::lib(cx);
        let focus = cx.focus_handle();
        let subs = vec![
            cx.on_focus_lost(window, |this, window, cx| {
                if !window.has_active_dialog(cx) {
                    window.focus(&this.focus, cx);
                    if this.page_refocused.is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(1)) {
                        this.page_refocused = Some(std::time::Instant::now());
                        state::return_focus_to_page(cx);
                    }
                }
            }),
            cx.observe_in(&lib, window, |this, lib, window, cx| {
                this.sync_search(window, cx);
                sheets::present(&lib, window, cx);
                cx.notify();
            }),
            cx.subscribe(&search, |this, s, e: &InputEvent, cx| match e {
                InputEvent::Change if !this.syncing_search => {
                    let text = s.read(cx).value().to_string();
                    if !text.is_empty() {
                        crate::core::prefs::mark_used("search");
                    }
                    act(cx, |l| {
                        if let Route::Module(_) = l.route() {
                            l.module_query = text;
                        } else {
                            l.search_query = text.clone();
                            if !text.is_empty() && *l.route() != Route::Search {
                                l.set_route(Route::Search);
                            }
                        }
                    });
                }
                InputEvent::PressEnter { .. } => {
                    let pages = this.pages.clone();
                    pages.update(cx, |p, cx| p.open_search_selection(cx));
                }
                _ => {}
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                if let WindowBounds::Windowed(b) = window.window_bounds() {
                    let saved = [f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)];
                    this.save_bounds = Some(cx.spawn(async move |_, cx| {
                        cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
                        crate::core::prefs::update(|p| p.main_window = Some(saved));
                    }));
                }
            }),
            cx.observe_window_activation(window, |_, window, cx| {
                if window.is_window_active() {
                    state::came_back(cx);
                    if let Some(m) = theme::recheck(window, cx) {
                        act(cx, |l| l.notify(m, "circle-alert", None, None));
                    }
                }
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                theme::refresh(window, cx);
                cx.notify();
            }),
        ];
        Self {
            search,
            inspector,
            pages,
            today,
            save_bounds: None,
            press_stamp: None,
            error_shown: false,
            syncing_search: false,
            search_placeholder: tr("Suchen"),
            focus,
            focused_once: false,
            page_refocused: None,
            shown_route: None,
            slides: None,
            ai_panel: None,
            resizing: None,
            _subs: subs,
        }
    }

    /// One search field for the whole window, so the toolbar never changes between pages:
    /// on a module page it filters that module's files, everywhere else it searches all files.
    fn sync_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = state::read(cx, |l| if let Route::Module(_) = l.route() { l.module_query.clone() } else { l.search_query.clone() });
        if self.search.read(cx).value().as_ref() != wanted {
            self.syncing_search = true;
            self.search.update(cx, |s, cx| s.set_value(wanted, window, cx));
            self.syncing_search = false;
        }
        let placeholder = state::read(cx, |l| if let Route::Module(id) = l.route() { l.module_by_id(id).map(|m| trf("In %@ suchen", &[&m.code])) } else { None }).unwrap_or_else(|| tr("Suchen"));
        if self.search_placeholder != placeholder {
            self.search_placeholder = placeholder.clone();
            self.search.update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
    }

    /// Esc in the search field clears it first; on an empty field it hands the keyboard back to the page.
    fn leave_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search.read(cx).value().is_empty() {
            act(cx, |l| if let Route::Module(_) = l.route() { l.module_query.clear() } else { l.search_query.clear() });
            self.search.update(cx, |s, cx| s.set_value("", window, cx));
            return;
        }
        match self.pages.read(cx).module.focus.clone().filter(|_| state::read(cx, |l| matches!(l.route(), Route::Module(_)))) {
            Some(page) => window.focus(&page, cx),
            None => {
                window.focus(&self.focus, cx);
                state::return_focus_to_page(cx);
            }
        }
    }

    fn step_results(&mut self, step: i64, cx: &mut Context<Self>) {
        if state::read(cx, |l| *l.route() == Route::Search) {
            self.pages.update(cx, |p, cx| p.move_search_selection(step, cx));
        }
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.focus(window, cx));
    }

    fn title(lib: &Library) -> String {
        match lib.route() {
            Route::Today => tr("Übersicht"),
            Route::Grid => lib.semester().map_or_else(|| tr("Semester"), |s| s.name.clone()),
            Route::Record => tr("Leistungen"),
            Route::Inbox => tr("Inbox"),
            Route::Search => tr("Suche"),
            Route::Notes => tr("Notizen"),
            Route::Graph => tr("Graph"),
            Route::Cards => tr("Karten"),
            Route::Module(id) => lib.module_by_id(id).map_or(id.clone(), |m| m.code.clone()),
            Route::Doc(p) => lib.note_title(p),
        }
    }

    /// Where the open note lives: each folder leads into the explorer, the name renames it.
    /// Narrow rows fold the folders into one "…" first, then the deadline gives way, the name last.
    fn crumbs(lib: &Library, path: &std::path::Path, room: f32, window: &Window, t: &Theme) -> Div {
        let over = t.ink.opacity(hover::CONTROL);
        let vault = crate::core::vault::index::current();
        let folders: Vec<String> = path.parent().and_then(|p| p.strip_prefix(&vault.root).ok()).map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect()).unwrap_or_default();
        let skipped = folders.len().saturating_sub(3);
        let name = lib.note_title(path);
        let linked = lib.deadlines_of(path);
        let has_pill = !linked.is_empty() || lib.module_containing(path).is_some();
        let label = linked.first().map(|a| {
            let more = if linked.len() > 1 { format!(" +{}", linked.len() - 1) } else { String::new() };
            format!("{}{more} · {}", crate::core::library::deadline_label(a), crate::core::day::countdown(a.due, lib.now))
        });

        let width = |text: &str, size: f32, weight: FontWeight| bar_fit::text_width(text, size, weight, window);
        let slash = width("/", font::CALLOUT, FontWeight::NORMAL) + space::XS;
        let crumb_w = |text: &str| 2. * space::XS + width(text, font::CALLOUT, FontWeight::NORMAL) + space::XS + slash;
        let shown_w: f32 = folders.iter().skip(skipped).map(|n| crumb_w(n).min(crumb::FOLDER_MAX)).sum::<f32>() + if skipped > 0 { crumb_w("…") } else { 0. };
        let collapsed_w = if folders.is_empty() { 0. } else { crumb_w("…") };
        let name_w = width(&name, font::TITLE3, FontWeight::BOLD) + 2. * space::XS + crumb::NAME_SLACK + space::XS;
        let caret = iconsize::XS + space::XS;
        let pill_natural = if has_pill {
            space::S + (match &label { Some(l) => bar_fit::button_width(l, window), None => 2. * space::S + iconsize::S } + caret).min(crumb::PILL_MAX)
        } else {
            0.
        };
        let plan = bar_fit::crumb_widths(room, shown_w, collapsed_w, name_w, pill_natural, if has_pill { space::S + crumb::PILL_MIN } else { 0. }, crumb::NAME_MIN);

        let mut row = div().flex_1().min_w_0().flex().items_center().gap(px(space::XS)).overflow_hidden();
        let folder = |i: usize, name: &str, rel: String| {
            div()
                .id(("crumb", i))
                .flex_none()
                .max_w(px(crumb::FOLDER_MAX))
                .truncate()
                .h(px(control::S))
                .line_height(px(control::S))
                .px(px(space::XS))
                .rounded(px(radius::SMALL))
                .cursor_pointer()
                .key_nav()
                .text_size(px(font::CALLOUT))
                .text_color(t.secondary)
                .soft(over, radius::SMALL)
                .tip(rel.clone())
                .on_click(move |_, _, cx| {
                    crate::core::prefs::update(|p| p.notes_explorer_hidden = false);
                    act(cx, |l| l.reveal_folder = Some(rel.clone()));
                })
                .child(name.to_string())
        };
        let separator = || div().flex_none().text_color(t.tertiary).child("/");
        if plan.folders {
            if skipped > 0 {
                let hidden: Vec<String> = folders[..skipped].to_vec();
                let all = folders.clone();
                let more = Button::new("crumb-hidden").ghost().small().min_w(px(control::S)).rounded(px(radius::SMALL)).cursor_pointer().tooltip(trf("Ausgeblendete Ordner: %@", &[&hidden.join("/")])).child(div().text_size(px(font::CALLOUT)).text_color(t.secondary).child("…")).dropdown_menu(move |mut menu, _, _| {
                    for (i, name) in hidden.iter().enumerate() {
                        let rel = all[..=i].join("/");
                        menu = menu.item(PopupMenuItem::new(crate::core::text::end_ellipsis(name, 40)).icon(menu_icon("folder")).on_click(move |_, _, cx| {
                            crate::core::prefs::update(|p| p.notes_explorer_hidden = false);
                            act(cx, |l| l.reveal_folder = Some(rel.clone()));
                        }));
                    }
                    menu
                });
                row = row.child(div().flex_none().text_color(t.tertiary).child(more)).child(separator());
            }
            for (i, name) in folders.iter().enumerate().skip(skipped) {
                row = row.child(folder(i, name, folders[..=i].join("/"))).child(separator());
            }
        } else if !folders.is_empty() {
            row = row.child(folder(folders.len() - 1, "…", folders.join("/"))).child(separator());
        }
        let rename = path.to_path_buf();
        row = row.child(
            div()
                .id("crumb-name")
                .flex_none()
                .w(px(plan.name))
                .truncate()
                .h(px(control::S))
                .line_height(px(control::S))
                .px(px(space::XS))
                .rounded(px(radius::SMALL))
                .cursor_pointer()
                .key_nav()
                .text_size(px(font::TITLE3))
                .font_weight(FontWeight::BOLD)
                .soft(over, radius::SMALL)
                .tip(format!("{name}\n{}{}", tr("Klicken zum Umbenennen"), ShortcutAction::RenameNote.hint()))
                .on_click(move |_, _, cx| super::app::rename_file(rename.clone(), cx))
                .child(name),
        );
        if has_pill && plan.pill > 0. {
            let menu_path = path.to_path_buf();
            let shown = linked.first().map(|a| (a.module_id.clone(), a.id.clone()));
            let pill = Button::new("note-deadline").ghost().small().min_w_0().w(px(plan.pill - space::S)).icon(menu_icon("link"));
            let belongs = linked.iter().map(|a| a.title.as_str()).collect::<Vec<_>>().join(", ");
            let pill = match label {
                Some(full) => pill.label(full.clone()).tooltip(format!("{}\n{full}", trf("Gehört zu %@", &[&belongs]))),
                None => pill.tooltip(tr("Einer Abgabe zuordnen")),
            };
            row = row.child(div().flex().flex_none().w(px(plan.pill - space::S)).overflow_hidden().ml(px(space::S)).child(pill.dropdown_menu(move |menu, _, cx| {
                let mut menu = deadline_items(menu.label(tr("Gehört zu")), &menu_path, cx);
                if let Some((module, id)) = shown.clone() {
                    menu = menu.separator().item(PopupMenuItem::new(tr("Abgabe anzeigen")).icon(menu_icon("arrow-right")).on_click(move |_, _, cx| {
                        act(cx, |l| {
                            l.set_route(Route::Module(module.clone()));
                            l.select(Some(id.clone()));
                        })
                    }));
                }
                menu
            })));
        }
        row
    }

    fn toolbar(&self, lib: &Library, left: f32, page_width: f32, total: f32, search_open: bool, window: &Window, t: &Theme) -> Div {
        let show = lib.show_inspector() && !super::ai::column_open();
        let offered = lib.inspector_kind().is_some();
        let module = if let Route::Module(id) = lib.route() { Some(id.clone()) } else { None };
        let module2 = module.clone();
        let toolbar_padding = (if super::chrome::CUSTOM { space::M } else { (92. - left).max(space::M) }) + space::M;
        let folded = total < control::SEARCH_FOLD && !search_open;
        let search_width = if folded { control::M } else if page_width < 760. { 200. } else { 325. };
        let ai_room = if super::ai::toolbar_button_shown() { control::M + space::S } else { 0. };
        div()
            .h(px(control::BAR))
            .flex_none()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(t.separator)
            .gap(px(space::S))
            .pl(px(toolbar_padding - space::M))
            .pr(px(space::M))
            .child(Button::new("go-back").ghost().size(px(control::M)).rounded_full().icon(menu_icon("arrow-left")).disabled(!lib.can_go_back()).tooltip(format!("{}{}", tr("Zurück"), ShortcutAction::GoBack.hint())).on_click(|_, _, cx| act(cx, |l| l.go_back())))
            .child(Button::new("go-forward").ghost().size(px(control::M)).rounded_full().icon(menu_icon("arrow-right")).disabled(!lib.can_go_forward()).tooltip(format!("{}{}", tr("Vorwärts"), ShortcutAction::GoForward.hint())).on_click(|_, _, cx| act(cx, |l| l.go_forward())))
            
            .child(match lib.route() {
                Route::Doc(shown) => Self::crumbs(lib, lib.focused_document().unwrap_or(shown), total - toolbar_padding - crumb::TOOLBAR_BUTTONS * control::M - search_width - crumb::TOOLBAR_GAPS * space::S - ai_room, window, t),
                _ => div().flex_1().min_w(px(80.)).truncate().text_size(px(font::TITLE3)).font_weight(FontWeight::BOLD).child(Self::title(lib)),
            })
            .child(super::coach::anchor(div().id("coach-new").rounded_full()).child(Button::new("new-menu").ghost().size(px(control::M)).rounded_full().icon(menu_icon("plus")).tooltip(format!("{}{}", tr("Neu"), ShortcutAction::NewEntry.hint())).dropdown_menu(move |menu, _, _| {
                let m = module2.clone();
                menu.item(PopupMenuItem::new(tr("Neuer Eintrag…")).icon(menu_icon("calendar-plus")).action(Box::new(super::app::NewEntry)).on_click(|_, _, cx| act(cx, |l| l.show_quick_add = true)))
                    .item(PopupMenuItem::new(tr("Neue Notiz, Datei oder Ordner…")).icon(menu_icon("file-plus")).action(Box::new(super::app::NewFile)).on_click(move |_, _, cx| {
                        let m = m.clone();
                        act(cx, |l| l.new_note = Some(NewNoteRequest { module_id: m, folder: None }))
                    }))
            })))
            .children(super::ai::toolbar_button(t))
            .child(
                Button::new("toggle-inspector")
                    .ghost()
                    .size(px(control::M))
                    .rounded_full()
                    .when(offered && show, |b| b.bg(t.ink.opacity(0.14)))
                    .icon(menu_icon("panel-right"))
                    .disabled(!offered)
                    .tooltip(if offered { format!("{}{}", if show { tr("Details ausblenden") } else { tr("Details einblenden") }, ShortcutAction::ToggleInspector.hint()) } else { tr("Keine Details auf dieser Seite") })
                    .on_click(|_, _, cx| super::ai::toggle_details(cx)),
            )
            .child(super::coach::anchor(div().id("coach-search")).w(px(search_width)).flex_none().key_context("SearchField").map(|d| if folded {
                d.child(Button::new("search-fold").ghost().size(px(control::M)).rounded_full().icon(menu_icon("search")).tooltip(format!("{} (/)", self.search_placeholder)).on_click(|_, window, cx| window.dispatch_action(Box::new(FocusSearch), cx)))
            } else {
                d.child(Input::new(&self.search).h(px(control::M)).prefix(icon("search", iconsize::M, t.secondary)).suffix(kbd("/", t)).cleanable(true))
            }))
    }
}

/// The overview page in its own view, so changes elsewhere in the window, like a streaming answer beside it, do not lay it out again.
pub struct TodayView {
    setup: Entity<super::setup_card::SetupCard>,
    moodle: Entity<super::setup_card::CalendarAsk>,
    scroll: ScrollHandle,
    width: f32,
    _subs: Vec<Subscription>,
}

impl TodayView {
    fn new(cx: &mut Context<Self>) -> Self {
        let lib = state::lib(cx);
        Self {
            setup: cx.new(super::setup_card::SetupCard::new),
            moodle: cx.new(|_| super::setup_card::CalendarAsk::new(false)),
            scroll: ScrollHandle::new(),
            width: 0.,
            _subs: vec![cx.observe(&lib, |_, _, cx| cx.notify())],
        }
    }
}

impl Render for TodayView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let setup_shown = self.setup.read(cx).wanted(cx);
        let covers_calendar = setup_shown && self.setup.read(cx).offers_calendar(cx);
        let setup_card = setup_shown.then(|| self.setup.clone().into_any_element());
        super::coach::scrolls("today", &self.scroll);
        let lib = state::lib(cx);
        let l = lib.read(cx);
        div().relative().size_full().child(div().id("today-scroll").size_full().overflow_y_scroll().track_scroll(&self.scroll).child(today::render(l, self.width, setup_card, covers_calendar, self.moodle.clone().into_any_element(), &t))).vertical_scrollbar(&self.scroll)
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        super::perf::frame_start();
        super::widgets::note_drag(cx.has_active_drag());
        if !std::mem::replace(&mut self.focused_once, true) && window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        let t = Theme::get(cx);
        let lib = state::lib(cx);
        let wanted_slides = lib.read(cx).slides.clone();
        if wanted_slides.as_ref() != self.slides.as_ref().map(|(p, _)| p) {
            self.slides = wanted_slides.map(|path| {
                let view = cx.new(|cx| super::vault_tools::Slides::new(&path, cx));
                (path, view)
            });
        }
        let slides = self.slides.as_ref().map(|(_, v)| v.clone());
        let has_error = lib.read(cx).last_error.is_some();
        if has_error && !self.error_shown {
            window.focus(&self.focus, cx);
        }
        self.error_shown = has_error;
        let route = lib.read(cx).route().clone();
        if self.shown_route.as_ref() != Some(&route) {
            let typing = self.search.read(cx).focus_handle(cx).is_focused(window);
            if self.shown_route.is_some() && !typing && !window.has_active_dialog(cx) && !state::read(cx, |l| l.quiet_open) {
                window.focus(&self.focus, cx);
            }
            motion::forget_rows(cx);
            if route == Route::Search && state::read(cx, |l| l.search_query.is_empty()) && !window.has_active_dialog(cx) {
                let search = self.search.clone();
                window.on_next_frame(move |window, cx| search.update(cx, |s, cx| s.focus(window, cx)));
            }
            self.shown_route = Some(route);
        }
        let l = lib.read(cx);
        let width = f32::from(window.viewport_size().width);
        let search_open = self.search.read(cx).focus_handle(cx).is_focused(window) || !self.search.read(cx).value().is_empty();
        let column = super::ai::column_open() && super::ai::active();
        let inspector_open = l.show_inspector() && !column;

        let base = super::coach::root(div().child(gpui_kit::base::TextSelectionLayer).id("main"))
            .capture_any_mouse_down(cx.listener(|this, e: &MouseDownEvent, _, cx| {
                this.press_stamp = None;
                if e.button == MouseButton::Right {
                    pause_tooltips(cx);
                } else {
                    super::widgets::clicked(cx);
                }
            }))
            .track_focus(&self.focus)
            .key_context("MainWindow")
            .size_full()
            .relative()
            .bg(t.content)
            .font_family(Theme::font_family())
            .text_size(px(font::BODY))
            .line_height(relative(font::LINE))
            .text_color(t.primary)
            .on_action(|_: &super::find_bar::FindInNote, _, cx| {
                if !super::app::in_active_note(super::find_bar::FindInNote, cx) && !super::app::find_in_pane(cx) {
                    cx.propagate();
                }
            })
            .on_action(|_: &super::find_bar::ReplaceInNote, _, cx| {
                if !super::app::in_active_note(super::find_bar::ReplaceInNote, cx) {
                    cx.propagate();
                }
            })
            .on_action(|_: &super::documents::ToggleWrap, _, cx| {
                if !super::app::in_active_note(super::documents::ToggleWrap, cx) {
                    cx.propagate();
                }
            })
            .on_action(|_: &super::app::RenameNote, window, cx| {
                if super::app::typing_in_field(window) {
                    return;
                }
                super::sheets::close_popovers(window, cx);
                match state::read(cx, super::app::rename_target) {
                    Some(path) => super::app::rename_file(path, cx),
                    None => cx.propagate(),
                }
            })
            .on_action(cx.listener(|this, _: &super::app::NewFile, window, cx| {
                if !this.pages.update(cx, |p, cx| p.new_note_in_explorer(window, cx)) {
                    cx.propagate();
                }
            }))
            .on_action(|_: &super::app::CloseTab, _, cx| {
                match state::read(cx, |l| if let Route::Doc(p) = l.route() { Some(p.clone()) } else { None }) {
                    Some(path) => super::documents::close_tab(&path, cx),
                    None => cx.propagate(),
                }
            })
            .capture_action(|_: &Dismiss, _, cx| {
                if super::coach::stop(cx) {
                    cx.stop_propagation();
                    return;
                }
                if state::read(cx, |l| l.last_error.is_some()) {
                    act(cx, |l| l.last_error = None);
                    cx.stop_propagation();
                }
            })
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                if window.context_stack().iter().any(|c| c.contains("NoteEditor")) {
                    return;
                }
                if this.search.read(cx).focus_handle(cx).is_focused(window) {
                    this.leave_search(window, cx);
                    return;
                }
                let typing = window.context_stack().iter().any(|c| c.contains("Input"));
                if !typing {
                    act(cx, Library::dismiss_inspector);
                }
                window.focus(&this.focus, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_action(|_: &gpui_kit::component::input::Copy, window, cx| {
                let text = gpui_kit::base::TextSelection::selected_text(window, cx);
                if text.is_empty() {
                    cx.propagate();
                    return;
                }
                super::drive::copy(super::selectable::without_room(&text).to_string(), cx);
            })
            .on_action(cx.listener(|this, _: &SearchUp, _, cx| this.step_results(-1, cx)))
            .on_action(cx.listener(|this, _: &SearchDown, _, cx| this.step_results(1, cx)))
            .on_action(|_: &super::app::Minimize, window, _| window.minimize_window())
            .on_action(|_: &super::app::Zoom, window, _| window.zoom_window())
            .on_action(|_: &super::app::ToggleFullscreen, window, _| window.toggle_fullscreen())
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                if matches!(k.key.as_str(), "enter" | "escape") && state::read(cx, |l| l.last_error.is_some()) {
                    act(cx, |l| l.last_error = None);
                    cx.stop_propagation();
                    return;
                }
                let slash = k.key == "/" || k.key_char.as_deref() == Some("/");
                let typing = window.context_stack().iter().any(|c| c.contains("Input"));
                let plain = !k.modifiers.platform && !k.modifiers.control && !k.modifiers.alt;
                if slash && plain && !typing && !window.has_active_dialog(cx) {
                    this.focus_search(window, cx);
                    cx.stop_propagation();
                    return;
                }
                let menu = window.context_stack().iter().any(|c| c.contains("PopupMenu"));
                if plain && !typing && !menu && !window.has_active_dialog(cx) && state::read(cx, |l| matches!(l.route(), Route::Today)) && super::board::key(&k.key, cx) {
                    cx.stop_propagation();
                }
            }));

        if l.root.is_none() {
            let error = l.last_error.clone();
            let page = base.child(super::onboarding::render(error, &t));
            if super::chrome::CUSTOM {
                let bar = super::chrome::main_bar(cx);
                return div().size_full().flex().flex_col().child(bar).child(div().flex_1().min_h_0().relative().child(page)).into_any_element();
            }
            return page.into_any_element();
        }

        let update = super::updates::available(cx);
        let left = motion::follow("sidebar-width", sidebar::rail_for(width), motion::MEDIUM, window, cx);
        if column && self.ai_panel.is_none() {
            self.ai_panel = Some(cx.new(|cx| super::ai_panel::AiPanel::new(window, cx)));
        }
        if self.resizing.is_some() && !cx.has_active_drag() {
            if let Some(w) = self.resizing.take() {
                crate::core::prefs::update(|p| {
                    p.inspector_width = Some(w);
                    p.ai.width = None;
                });
            }
        }
        let panel_open = column || inspector_open;
        let full = self.resizing.unwrap_or_else(|| panel_width(width));
        let inspector_target = if panel_open { full } else { 0. };
        let floating = panel_open && panel_floats(width);
        let reserved = if floating { 0. } else { inspector_target };
        let slot_w = motion::follow("right-panel-width", inspector_target, if self.resizing.is_some() { std::time::Duration::ZERO } else { motion::MEDIUM }, window, cx);
        let page_width = width - left - reserved;
        self.pages.update(cx, |p, _| p.width = page_width);
        let dialog_open = window.has_active_dialog(cx);
        self.today.update(cx, |v, _| v.width = page_width);
        let l = lib.read(cx);
        let page = match l.route() {
            Route::Today => {
                AnyView::from(self.today.clone()).cached(StyleRefinement::default().size_full()).into_any_element()
            }
            _ => self.pages.clone().into_any_element(),
        };
        let page = if matches!(l.route(), Route::Today | Route::Grid | Route::Record) {
            let accent = t.accent;
            div()
                .id("drop-page")
                .size_full()
                .relative()
                .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(accent.opacity(0.08)))
                .on_drop(|paths: &ExternalPaths, _, cx| {
                    let paths = paths.paths().to_vec();
                    act(cx, |l| l.request_filing(paths, None));
                })
                .child(page)
                .into_any_element()
        } else {
            page
        };
        let page_key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            l.route().hash(&mut hasher);
            hasher.finish()
        };
        let page_motion = motion::timed(motion::FAST, cx);
        let toast_id = l.toast.as_ref().map_or(0, |toast| toast.id);
        let toast = l.toast.as_ref().map(|toast| toast_view(toast.message.clone(), toast.icon, toast.undo.is_some().then(|| toast.action_title.clone()), &t));
        let notice_room = if matches!(l.route(), Route::Doc(_)) {
            [l.active_document.as_ref(), l.split_document.as_ref()]
                .into_iter()
                .flatten()
                .filter(|d| l.save_problems.contains(d) && l.buffers.get(*d).is_some_and(|b| b.dirty))
                .map(|d| l.save_notice_height.get(d).copied().unwrap_or(control::SAVE_NOTICE))
                .fold(0., f32::max)
        } else {
            0.
        };
        let bar_room = if matches!(l.route(), Route::Doc(_)) && l.page_bars.values().any(|p| l.active_document.as_ref() == Some(p) || l.split_document.as_ref() == Some(p)) { control::SUBBAR } else { 0. };
        let working = (!l.exporting_pdfs.is_empty() && l.toast.is_none()).then(|| toast_view(tr("PDF wird erstellt …"), "loader", None, &t));
        let error = l.last_error.clone();
        let broken = l.config_broken;
        let config_path = l.config_path();

        let mut root = base
            .child(div().absolute().top_0().left_0().bottom_0().w(px(left)).bg(t.sidebar))
            .child(
                div()
                    .absolute()
                    .top(px(sidebar::BAND))
                    .left_0()
                    .bottom_0()
                    .w(px(left))
                    .child(sidebar::render(l, update, width, &t, cx)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(left))
                    .right_0()
                    .border_l_1()
                    .border_color(t.separator)
                    .flex()
                    .flex_col()
                    .child(self.toolbar(l, left, page_width + reserved, width - left - crumb::BORDER, search_open, window, &t))
                    .child(div().id("page-area").flex_1().min_h_0().mr(px(reserved))
                        .when(floating && column, |d| d.on_mouse_down(MouseButton::Left, |_, _, cx| super::ai::close_panel(cx)))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, cx| {
                            this.press_stamp = (!window.default_prevented()).then(|| (state::read(cx, Library::selection_stamp), e.position));
                        }))
                        .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, _, cx| {
                            let Some((stamp, at)) = this.press_stamp.take() else { return };
                            let still = f32::from((e.position.x - at.x).abs() + (e.position.y - at.y).abs()) < 6.;
                            if still && state::read(cx, |l| l.selection_stamp() == stamp && l.selection().is_some()) {
                                act(cx, |l| l.select(None));
                            }
                        }))
                        .child(page).with_animation(("page", page_key), Animation::new(page_motion).with_easing(ease_out_quint()), |d, p| d.opacity(0.4 + 0.6 * p))),
            )
            .when(slot_w > 0.5, |d| {
                let content: AnyElement = match self.ai_panel.clone().filter(|_| column) {
                    Some(panel) => panel.into_any_element(),
                    None if super::ai::active() => {
                        let head = div().h(px(control::SUBBAR)).flex_none().flex().items_center().gap(px(space::XS)).pl(px(space::S)).pr(px(space::S)).border_b_1().border_color(t.separator)
                            .child(super::ai::panel_tabs(false, &t, cx))
                            .child(div().flex_1())
                            .child(Button::new("details-close").ghost().size(px(control::S)).rounded(px(radius::INNER)).icon(menu_icon("x")).tooltip(format!("{}{}", tr("Details ausblenden"), ShortcutAction::ToggleInspector.hint())).on_click(|_, _, cx| super::ai::close_panel(cx)));
                        div().size_full().flex().flex_col().child(head).child(div().flex_1().min_h_0().child(self.inspector.clone())).into_any_element()
                    }
                    None => self.inspector.clone().into_any_element(),
                };
                let shown = inspector_target.max(slot_w);
                let edge = div()
                    .id("right-panel-edge")
                    .occlude()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(px(EDGE_GRIP))
                    .cursor_col_resize()
                    .soft(t.accent.opacity(hover::RAISED), 0.)
                    .when(self.resizing.is_some(), |d| d.bg(t.accent.opacity(0.35)))
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        if e.click_count() == 2 {
                            this.resizing = None;
                            crate::core::prefs::update(|p| {
                                p.inspector_width = None;
                                p.ai.width = None;
                            });
                            let _ = window;
                            cx.notify();
                        }
                    }))
                    .on_drag(PanelEdge, |d, _, _, cx| cx.new(|_| d.clone()));
                d.child(
                    div()
                        .id("right-panel")
                        .absolute()
                        .top(px(control::BAR))
                        .bottom_0()
                        .right_0()
                        .w(px(slot_w))
                        .overflow_hidden()
                        .bg(t.content)
                        .when(floating, |d| d.occlude().shadow_lg().capture_key_down(|e: &KeyDownEvent, _, cx| {
                            if e.keystroke.key == "escape" {
                                super::ai::close_panel(cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(div().absolute().top_0().bottom_0().right_0().w(px(shown)).child(content))
                        .child(div().absolute().top_0().bottom_0().left_0().w(px(1.)).bg(t.separator))
                        .child(edge),
                )
                .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<PanelEdge>, window, cx| {
                    let total = f32::from(window.viewport_size().width);
                    let max = panel_max(total);
                    let w = (total - f32::from(e.event.position.x)).round().clamp(PANEL_MIN, max);
                    if this.resizing != Some(w) {
                        this.resizing = Some(w);
                        cx.notify();
                    }
                }))
            })
            .children(super::link_card::overlay(cx))
            .children(super::ai::ask_card(cx, &t))
            .children(slides)
            .children(toast.or(working).map(|v| {
                let toast_motion = motion::timed(motion::SLOW, cx);
                div()
                    .absolute()
                    .left(px(left))
                    .right(px(reserved))
                    .flex()
                    .justify_center()
                    .child(v)
                    .with_animation(("toast", toast_id), Animation::new(toast_motion).with_easing(ease_out_quint()), move |d, p| d.bottom(px(10. + notice_room + bar_room + 12. * p)).opacity(p))
            }));
        if error.is_none() && !dialog_open && !floating {
            root = root.children(super::tips::card(reserved + space::L, space::L + notice_room + bar_room, &t, cx));
        }
        root = root.children(super::coach::overlay(window, cx, &t));
        if let Some(message) = error {
            root = root.child(error_alert(message, broken, config_path, &t));
        }
        root = super::perf::frame_end(root);
        if super::chrome::CUSTOM {
            let bar = super::chrome::main_bar(cx);
            return div().size_full().flex().flex_col().child(bar).child(div().flex_1().min_h_0().relative().child(root)).into_any_element();
        }
        root.into_any_element()
    }
}

fn toast_view(message: String, icon_name: &'static str, action: Option<String>, t: &Theme) -> Stateful<Div> {
    div()
        .id("toast")
        .occlude()
        .on_hover(|hovered, _, cx| state::lib(cx).update(cx, |l, _| l.toast_hovered = *hovered))
        .max_w(px(560.))
        .flex()
        .items_center()
        .gap(px(space::S))
        .px(px(space::L))
        .py(px(space::S))
        .rounded_full()
        .bg(t.material)
        .border_1()
        .border_color(t.separator)
        .shadow_lg()
        .child(icon(icon_name, iconsize::S, t.secondary))
        .child(div().min_w_0().truncate().child(message))
        .when_some(action, |d, label| {
            d.child(div().w(px(1.)).h(px(16.)).bg(t.separator)).child(link_button("toast-undo", label, t).font_weight(FontWeight::SEMIBOLD).on_click(|_, _, cx| act(cx, |l| l.perform_toast_action())))
        })
}

fn error_alert(message: String, broken: bool, config: Option<PathBuf>, t: &Theme) -> Stateful<Div> {
    let mut buttons = div().flex().justify_end().gap(px(space::S));
    if broken {
        buttons = buttons.child(Button::new("restore").label(tr("Letzte Sicherung wiederherstellen")).on_click(|_, _, cx| act(cx, |l| l.restore_config_backup())));
        if let Some(p) = config {
            buttons = buttons.child(Button::new("show-config").label(tr("schrumbi.json zeigen")).on_click(move |_, _, _| crate::core::library::reveal(&p)));
        }
    }
    buttons = buttons.child(Button::new("ok").primary().label("OK").on_click(|_, _, cx| act(cx, |l| l.last_error = None)));
    div().id("error-alert").absolute().inset_0().bg(black().opacity(0.25)).occlude().flex().items_center().justify_center().child(
        div()
            .w(px(420.))
            .p(px(space::XL))
            .rounded(px(radius::CARD))
            .bg(t.material)
            .shadow_2xl()
            .flex()
            .flex_col()
            .gap(px(space::M))
            .child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::BOLD).child(tr("Fehler")))
            .child(div().text_color(t.secondary).child(message))
            .child(buttons),
    )
}

pub fn open(cx: &mut App) {
    let saved = crate::core::prefs::get().main_window.map(|[x, y, w, h]| Bounds::new(point(px(x), px(y)), size(px(w), px(h)))).filter(|b| cx.displays().iter().any(|d| d.bounds().intersects(b)));
    let bounds = saved.unwrap_or_else(|| Bounds::centered(None, size(px(1360.), px(860.)), cx));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(640.), px(500.))),
        titlebar: Some(TitlebarOptions { title: Some("Schrumbi".into()), appears_transparent: true, traffic_light_position: Some(point(px(19.), px(19.))) }),
        app_id: Some("io.github.schrumbo.schrumbi".into()),
        ..Default::default()
    };
    let options = super::chrome::window_options(options, "Schrumbi");
    if let Ok((handle, _)) = super::chrome::open_window(options, cx, |window, cx| {
        super::platform::set_window_appearance(theme::native_appearance());
        theme::refresh(window, cx);
        cx.new(|cx| MainWindow::new(window, cx))
    }) {
        cx.set_global(MainHandle(handle));
    }
}

struct MainHandle(AnyWindowHandle);

impl Global for MainHandle {}

/// Brings the main window to the front, reopening it if it was closed.
pub fn show(cx: &mut App) {
    let handle = cx.try_global::<MainHandle>().map(|m| m.0).filter(|h| cx.windows().contains(h));
    match handle {
        Some(h) => {
            let _ = h.update(cx, |_, window, _| super::drive::activate_window(window));
        }
        None => open(cx),
    }
    super::drive::activate(cx);
}

/// Whether `id` is the main window, open or just closed.
pub fn is_main(id: WindowId, cx: &App) -> bool {
    cx.try_global::<MainHandle>().is_some_and(|m| m.0.window_id() == id)
}

/// Whether the main window is the frontmost one.
pub fn is_active(cx: &App) -> bool {
    cx.active_window().is_some_and(|w| cx.try_global::<MainHandle>().is_some_and(|m| m.0 == w))
}

pub fn dialog_open(window: &mut Window, cx: &mut App) -> bool {
    window.has_active_dialog(cx)
}
