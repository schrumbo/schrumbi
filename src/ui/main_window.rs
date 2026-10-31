use gpui_kit::component::Disableable;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::WindowExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::inspector::Inspector;
use super::motion;
use super::pages::Pages;
use super::state::{self, act};
use super::theme::{self, Theme, font, space, iconsize, radius, control};
use super::widgets::*;
use super::{sheets, sidebar, today};
use crate::core::i18n::tr;
use crate::core::library::{Library, NewNoteRequest, Route};
use crate::core::shortcuts::ShortcutAction;

actions!(schrumbi_main, [Dismiss, FocusSearch, ToggleSidebar, SearchUp, SearchDown]);

/// The main window: floating sidebar, toolbar, the routed page and the inspector.
pub struct MainWindow {
    search: Entity<InputState>,
    inspector: Entity<Inspector>,
    pages: Entity<Pages>,
    /// Whether the sidebar shows labels; its icons always stay.
    sidebar_open: bool,
    save_bounds: Option<Task<()>>,
    sidebar_auto_closed: bool,
    syncing_search: bool,
    focus: FocusHandle,
    focused_once: bool,
    shown_route: Option<Route>,
    error_shown: bool,
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
            cx.observe_window_bounds(window, |this, window, cx| {
                if let WindowBounds::Windowed(b) = window.window_bounds() {
                    let saved = [f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)];
                    this.save_bounds = Some(cx.spawn(async move |_, cx| {
                        cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
                        crate::core::prefs::update(|p| p.main_window = Some(saved));
                    }));
                }
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
            save_bounds: None,
            sidebar_auto_closed: false,
            error_shown: false,
            syncing_search: false,
            focus: cx.focus_handle(),
            focused_once: false,
            shown_route: None,
            _subs: subs,
        }
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.sidebar_open = open;
        self.sidebar_auto_closed = false;
        crate::core::prefs::update(|p| p.sidebar_open = open);
        cx.notify();
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
            Route::Checks => tr("Aufräumen"),
            Route::Module(id) => lib.module_by_id(id).map_or(id.clone(), |m| m.display_name()),
            Route::Doc(p) => lib.note_title(p),
        }
    }

    fn toolbar(&self, lib: &Library, left: f32, page_width: f32, t: &Theme) -> Div {
        let show = lib.show_inspector;
        let module = if let Route::Module(id) = lib.route() { Some(id.clone()) } else { None };
        let module2 = module.clone();
        div()
            .h(px(control::BAR))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(space::S))
            .pl(px(if super::chrome::CUSTOM { space::M } else { (92. - left).max(space::M) }))
            .pr(px(space::M))
            .child(Button::new("go-back").ghost().size(px(control::M)).rounded_full().icon(menu_icon("arrow-left")).disabled(!lib.can_go_back()).tooltip(format!("{}{}", tr("Zurück"), ShortcutAction::GoBack.hint())).on_click(|_, _, cx| act(cx, |l| l.go_back())))
            .child(Button::new("go-forward").ghost().size(px(control::M)).rounded_full().icon(menu_icon("arrow-right")).disabled(!lib.can_go_forward()).tooltip(tr("Vorwärts")).on_click(|_, _, cx| act(cx, |l| l.go_forward())))
            .child(div().flex_1().min_w(px(80.)).truncate().text_size(px(font::TITLE3)).font_weight(FontWeight::BOLD).child(Self::title(lib)))
            .child(Button::new("new-menu").ghost().size(px(control::M)).rounded_full().bg(t.ink.opacity(0.06)).border_1().border_color(t.ink.opacity(0.1)).icon(menu_icon("plus")).tooltip(tr("Neu")).dropdown_menu(move |menu, _, _| {
                let m = module2.clone();
                menu.item(PopupMenuItem::new(format!("{}{}", tr("Neuer Eintrag…"), ShortcutAction::NewEntry.hint())).icon(menu_icon("calendar-plus")).on_click(|_, _, cx| act(cx, |l| l.show_quick_add = true)))
                    .item(PopupMenuItem::new(tr("Neue Notiz, Datei oder Ordner…")).icon(menu_icon("file-plus")).on_click(move |_, _, cx| {
                        let m = m.clone();
                        act(cx, |l| l.new_note = Some(NewNoteRequest { module_id: m, folder: None }))
                    }))
            }))
            .child(
                Button::new("toggle-inspector")
                    .ghost()
                    .size(px(control::M))
                    .rounded_full()
                    .bg(t.ink.opacity(if show { 0.14 } else { 0.06 }))
                    .border_1()
                    .border_color(t.ink.opacity(0.1))
                    .icon(menu_icon("panel-right"))
                    .tooltip(format!("{}{}", if show { tr("Details ausblenden") } else { tr("Details einblenden") }, ShortcutAction::ToggleInspector.hint()))
                    .on_click(|_, _, cx| act(cx, |l| l.toggle_inspector())),
            )
            .child(div().w(px(if page_width < 760. { 200. } else { 325. })).flex_none().key_context("SearchField").child(Input::new(&self.search).h(px(control::M)).prefix(icon("search", iconsize::M, t.secondary)).suffix(kbd("/", t)).cleanable(true)))
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !std::mem::replace(&mut self.focused_once, true) && window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        let t = Theme::get(cx);
        let lib = state::lib(cx);
        let has_error = lib.read(cx).last_error.is_some();
        if has_error && !self.error_shown {
            window.focus(&self.focus, cx);
        }
        self.error_shown = has_error;
        let route = lib.read(cx).route().clone();
        if self.shown_route.as_ref() != Some(&route) {
            let typing = self.search.read(cx).focus_handle(cx).is_focused(window);
            if self.shown_route.is_some() && !typing && !window.has_active_dialog(cx) {
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
        let crowded = l.show_inspector && width < if self.sidebar_auto_closed { 1280. } else { 1240. };
        if crowded && self.sidebar_open {
            self.sidebar_open = false;
            self.sidebar_auto_closed = true;
        } else if !crowded && self.sidebar_auto_closed {
            self.sidebar_open = true;
            self.sidebar_auto_closed = false;
        }

        let base = div()
            .child(gpui_kit::base::TextSelectionLayer)
            .id("main")
            .capture_any_mouse_down(|e, _, cx| {
                if e.button == MouseButton::Right {
                    pause_tooltips(cx);
                }
            })
            .track_focus(&self.focus)
            .key_context("MainWindow")
            .size_full()
            .relative()
            .bg(t.content)
            .font_family(Theme::font_family())
            .text_size(px(font::BODY))
            .line_height(relative(1.22))
            .text_color(t.primary)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                let typing = window.context_stack().iter().any(|c| c.contains("Input"));
                if !typing {
                    act(cx, |l| {
                        l.show_inspector = false;
                        l.select(None);
                    });
                }
                window.focus(&this.focus, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &SearchUp, _, cx| this.step_results(-1, cx)))
            .on_action(cx.listener(|this, _: &SearchDown, _, cx| this.step_results(1, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.set_open(!this.sidebar_open, cx)))
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
                if slash && !k.modifiers.platform && !k.modifiers.control && !k.modifiers.alt && !typing && !window.has_active_dialog(cx) {
                    this.focus_search(window, cx);
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
        let target = if self.sidebar_open { sidebar::WIDTH } else { sidebar::RAIL };
        let inspector_w = if l.show_inspector { 340. } else { 0. };
        let page_width = width - target - inspector_w;
        self.pages.update(cx, |p, _| p.width = page_width);
        let left = motion::follow("sidebar-width", target, motion::MEDIUM, window, cx);
        let l = lib.read(cx);
        let page = match l.route() {
            Route::Today => div().id("today-scroll").size_full().overflow_y_scrollbar().child(today::render(l, page_width, &t)).into_any_element(),
            _ => self.pages.clone().into_any_element(),
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
                    .child(sidebar::render(l, update, self.sidebar_open, &t)),
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
                    .child(self.toolbar(l, left, page_width, &t))
                    .child(div().flex_1().min_h_0().child(page).with_animation(("page", page_key), Animation::new(page_motion).with_easing(ease_out_quint()), |d, p| d.opacity(0.4 + 0.6 * p))),
            )
            .when(l.show_inspector, |d| {
                d.child(div().absolute().top_0().bottom_0().right_0().w(px(inspector_w)).border_l_1().border_color(t.separator).bg(t.content).pt(px(control::BAR)).child(self.inspector.clone()))
            })
            .children(toast.or(working).map(|v| {
                let toast_motion = motion::timed(motion::SLOW, cx);
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(v)
                    .with_animation(("toast", toast_id), Animation::new(toast_motion).with_easing(ease_out_quint()), |d, p| d.bottom(px(10. + 12. * p)).opacity(p))
            }));
        if let Some(message) = error {
            root = root.child(error_alert(message, broken, config_path, &t));
        }
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
        window_min_size: Some(size(px(1000.), px(620.))),
        titlebar: Some(TitlebarOptions { title: Some("Schrumbi".into()), appears_transparent: true, traffic_light_position: Some(point(px(19.), px(19.))) }),
        app_id: Some("io.github.schrumbo.schrumbi".into()),
        ..Default::default()
    };
    let options = super::chrome::window_options(options, "Schrumbi");
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
