use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::WindowExt;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::inspector::Inspector;
use super::pages::Pages;
use super::state::{self, act};
use super::theme::{self, Theme, font};
use super::widgets::*;
use super::{sheets, sidebar, today};
use crate::core::i18n::tr;
use crate::core::library::{Library, NewNoteRequest, Route};

actions!(schrumbi_main, [Dismiss, FocusSearch, ToggleSidebar]);

/// The main window: floating sidebar, toolbar, the routed page and the inspector.
pub struct MainWindow {
    search: Entity<InputState>,
    inspector: Entity<Inspector>,
    pages: Entity<Pages>,
    /// Whether the sidebar shows labels; its icons always stay.
    sidebar_open: bool,
    sidebar_auto_closed: bool,
    /// The width being drawn and the running transition: its start and starting width.
    sidebar_shown: f32,
    sidebar_motion: Option<(std::time::Instant, f32)>,
    syncing_search: bool,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

fn main_view(window: &Window, cx: &App) -> Option<Entity<MainWindow>> {
    let root = window.root::<gpui_kit::base::Root>()??;
    root.read(cx).view().clone().downcast::<MainWindow>().ok()
}

/// Shows or hides the sidebar labels (⌘B or a click on the selected tab).
pub fn toggle_sidebar(window: &mut Window, cx: &mut App) {
    if let Some(view) = main_view(window, cx) {
        view.update(cx, |w, cx| w.set_open(!w.sidebar_open, cx));
    }
}

impl MainWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Suchen")).clean_on_escape());
        let inspector = cx.new(|cx| Inspector::new(window, cx));
        let pages = cx.new(|cx| Pages::new(window, cx));
        let lib = state::lib(cx);
        let subs = vec![
            cx.observe_in(&lib, window, |this, lib, window, cx| {
                this.sync_search(window, cx);
                sheets::present(&lib, window, cx);
                cx.notify();
            }),
            cx.subscribe(&search, |this, s, e: &InputEvent, cx| match e {
                InputEvent::Change if !this.syncing_search => {
                    let text = s.read(cx).value().to_string();
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
            cx.observe_window_appearance(window, |_, window, cx| {
                theme::apply(theme::is_dark(window.appearance()), Some(window), cx);
                cx.notify();
            }),
        ];
        let p = crate::core::prefs::get();
        Self {
            search,
            inspector,
            pages,
            sidebar_open: p.sidebar_open,
            sidebar_auto_closed: false,
            sidebar_shown: if p.sidebar_open { sidebar::WIDTH } else { sidebar::RAIL },
            sidebar_motion: None,
            syncing_search: false,
            focus: cx.focus_handle(),
            _subs: subs,
        }
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.sidebar_open = open;
        self.sidebar_auto_closed = false;
        self.sidebar_motion = Some((std::time::Instant::now(), self.sidebar_shown));
        crate::core::prefs::update(|p| p.sidebar_open = open);
        cx.notify();
    }

    /// The width to draw this frame, easing out over 180 ms towards the target.
    fn sidebar_frame(&mut self, target: f32, window: &mut Window) -> f32 {
        self.sidebar_shown = match self.sidebar_motion {
            Some((start, from)) => {
                let progress = start.elapsed().as_secs_f32() / 0.18;
                if progress >= 1. {
                    self.sidebar_motion = None;
                    target
                } else {
                    window.request_animation_frame();
                    from + (target - from) * (1. - (1. - progress).powi(3))
                }
            }
            None => target,
        };
        self.sidebar_shown
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
        if state::read(cx, |l| *l.route() == Route::Search) && !self.search.read(cx).focus_handle(cx).is_focused(window) && state::read(cx, |l| l.search_query.is_empty()) {
            self.search.update(cx, |s, cx| s.focus(window, cx));
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
            Route::Checks => tr("Prüfen"),
            Route::Module(id) => lib.module_by_id(id).map_or(id.clone(), |m| m.display_name()),
            Route::Doc(p) => lib.note_title(p),
        }
    }

    fn toolbar(&self, lib: &Library, left: f32, t: &Theme) -> Div {
        let show = lib.show_inspector;
        let module = if let Route::Module(id) = lib.route() { Some(id.clone()) } else { None };
        let module2 = module.clone();
        div()
            .h(px(52.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px((92. - left).max(10.)))
            .pr(px(10.))
            .child(div().flex_1().min_w_0().truncate().text_size(px(font::TITLE3)).font_weight(FontWeight::BOLD).child(Self::title(lib)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(t.ink.opacity(0.06))
                    .border_1()
                    .border_color(t.ink.opacity(0.1))
                    .child(
                        Button::new("new-entry")
                            .ghost()
                            .rounded(px(18.))
                            .icon(menu_icon("plus"))
                            .tooltip(tr("Klick: Abgabe, Aufgabe oder Prüfung (⌘N) · Gedrückt halten: Notiz, Datei oder Ordner (⇧⌘N)"))
                            .on_click(|_, _, cx| act(cx, |l| l.show_quick_add = true)),
                    )
                    .child(div().w(px(1.)).h(px(18.)).bg(t.ink.opacity(0.12)))
                    .child(Button::new("new-menu").ghost().rounded(px(18.)).icon(menu_icon("chevron-down")).dropdown_menu(move |menu, _, _| {
                        let m = module2.clone();
                        menu.item(PopupMenuItem::new(tr("Neuer Termin…")).icon(menu_icon("calendar-plus")).on_click(|_, _, cx| act(cx, |l| l.show_quick_add = true)))
                            .item(PopupMenuItem::new(tr("Neue Notiz, Datei oder Ordner…")).icon(menu_icon("file-plus")).on_click(move |_, _, cx| {
                                let m = m.clone();
                                act(cx, |l| l.new_note = Some(NewNoteRequest { module_id: m, folder: None }))
                            }))
                    })),
            )
            .child(
                Button::new("toggle-inspector")
                    .ghost()
                    .rounded(px(18.))
                    .icon(menu_icon("panel-right"))
                    .tooltip(if show { tr("Details ausblenden (⌥⌘I)") } else { tr("Details einblenden (⌥⌘I)") })
                    .on_click(|_, _, cx| act(cx, |l| l.toggle_inspector())),
            )
            .child(div().w(px(325.)).child(Input::new(&self.search).prefix(icon("search", 15., t.secondary)).cleanable(true)))
            .when(module.is_some(), |d| d)
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let lib = state::lib(cx);
        let l = lib.read(cx);
        let width = f32::from(window.viewport_size().width);
        let crowded = l.show_inspector && width < if self.sidebar_auto_closed { 1280. } else { 1240. };
        if crowded && self.sidebar_open {
            self.sidebar_open = false;
            self.sidebar_auto_closed = true;
            self.sidebar_motion = Some((std::time::Instant::now(), self.sidebar_shown));
        } else if !crowded && self.sidebar_auto_closed {
            self.sidebar_open = true;
            self.sidebar_auto_closed = false;
            self.sidebar_motion = Some((std::time::Instant::now(), self.sidebar_shown));
        }

        let base = div()
            .id("main")
            .track_focus(&self.focus)
            .key_context("MainWindow")
            .size_full()
            .relative()
            .bg(t.content)
            .font_family(Theme::font_family())
            .text_size(px(font::BODY))
            .line_height(relative(1.22))
            .text_color(t.primary)
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| {
                act(cx, |l| {
                    l.show_inspector = false;
                    l.select(None);
                    l.show_inspector = false;
                })
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.set_open(!this.sidebar_open, cx)));

        if l.root.is_none() {
            let error = l.last_error.clone();
            return base.child(super::onboarding::render(error, &t)).into_any_element();
        }

        let target = if self.sidebar_open { sidebar::WIDTH } else { sidebar::RAIL };
        let left = self.sidebar_frame(target, window);
        let inspector_w = if l.show_inspector { 340. } else { 0. };
        let page_width = width - target - inspector_w;
        self.pages.update(cx, |p, _| p.width = page_width);
        let l = lib.read(cx);
        let page = match l.route() {
            Route::Today => div().id("today-scroll").size_full().overflow_y_scroll().child(today::render(l, page_width, &t)).into_any_element(),
            _ => self.pages.clone().into_any_element(),
        };
        let toast = l.toast.as_ref().map(|toast| toast_view(toast.message.clone(), toast.icon, toast.undo.is_some().then(|| toast.action_title.clone()), &t));
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
                    .child(sidebar::render(l, &t)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(left))
                    .right(px(inspector_w))
                    .border_l_1()
                    .border_color(t.separator)
                    .flex()
                    .flex_col()
                    .child(self.toolbar(l, left, &t))
                    .child(div().flex_1().min_h_0().child(page)),
            )
            .when(l.show_inspector, |d| {
                d.child(div().absolute().top_0().bottom_0().right_0().w(px(inspector_w)).border_l_1().border_color(t.separator).bg(t.content).pt(px(52.)).child(self.inspector.clone()))
            })
            .children(toast.or(working).map(|v| div().absolute().bottom(px(22.)).left_0().right_0().flex().justify_center().child(v)));
        if let Some(message) = error {
            root = root.child(error_alert(message, broken, config_path, &t));
        }
        root.into_any_element()
    }
}

fn toast_view(message: String, icon_name: &'static str, action: Option<String>, t: &Theme) -> Div {
    div()
        .max_w(px(560.))
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(16.))
        .py(px(10.))
        .rounded_full()
        .bg(t.material)
        .border_1()
        .border_color(t.separator)
        .shadow_lg()
        .child(icon(icon_name, 14., t.secondary))
        .child(div().min_w_0().truncate().child(message))
        .when_some(action, |d, label| {
            d.child(div().w(px(1.)).h(px(16.)).bg(t.separator)).child(link_button("toast-undo", label, t).font_weight(FontWeight::SEMIBOLD).tooltip(tip("⌘Z")).on_click(|_, _, cx| act(cx, |l| l.perform_undo())))
        })
}

fn error_alert(message: String, broken: bool, config: Option<PathBuf>, t: &Theme) -> Div {
    let mut buttons = div().flex().justify_end().gap(px(8.));
    if broken {
        buttons = buttons.child(Button::new("restore").label(tr("Letzte Sicherung wiederherstellen")).on_click(|_, _, cx| act(cx, |l| l.restore_config_backup())));
        if let Some(p) = config {
            buttons = buttons.child(Button::new("show-config").label(tr("schrumbi.json zeigen")).on_click(move |_, _, _| crate::core::library::reveal(&p)));
        }
    }
    buttons = buttons.child(Button::new("ok").primary().label("OK").on_click(|_, _, cx| act(cx, |l| l.last_error = None)));
    div().absolute().inset_0().bg(black().opacity(0.25)).flex().items_center().justify_center().child(
        div()
            .w(px(420.))
            .p(px(20.))
            .rounded(px(14.))
            .bg(t.material)
            .shadow_2xl()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::BOLD).child(tr("Fehler")))
            .child(div().text_color(t.secondary).child(message))
            .child(buttons),
    )
}

pub fn open(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(1000.), px(620.))),
        titlebar: Some(TitlebarOptions { title: Some("Schrumbi".into()), appears_transparent: true, traffic_light_position: Some(point(px(19.), px(19.))) }),
        app_id: Some("io.github.schrumbo.schrumbi".into()),
        ..Default::default()
    };
    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |window, cx| {
        theme::apply(theme::is_dark(window.appearance()), Some(window), cx);
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
            let _ = h.update(cx, |_, window, _| window.activate_window());
        }
        None => open(cx),
    }
    cx.activate(true);
}

/// Whether the main window is the frontmost one.
pub fn is_active(cx: &App) -> bool {
    cx.active_window().is_some_and(|w| cx.try_global::<MainHandle>().is_some_and(|m| m.0 == w))
}

pub fn dialog_open(window: &mut Window, cx: &mut App) -> bool {
    window.has_active_dialog(cx)
}
