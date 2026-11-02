use gpui_kit::component::WindowExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::browser::FileBrowser;
use super::checks::{self, ChecksState};
use super::fields::Fields;
use super::grid::{self, GridState};
use super::module_page::{self, ModuleState};
use super::preview::FilePreview;
use super::record::{self, RecordState};
use super::state::{self, act};
use super::motion::Soft;
use super::theme::{Theme, control, font, space, iconsize, radius};
use super::widgets::*;
use crate::core::day;
use crate::core::i18n::tr;
use crate::core::library::{DocDrop, Library, Route};
use crate::core::text::fold;

/// Hosts every page except the overview, keeping each page's view state alive across navigation.
pub struct Pages {
    pub grid: GridState,
    pub record: RecordState,
    pub checks: ChecksState,
    pub module: ModuleState,
    search: Entity<FileBrowser>,
    preview: Entity<FilePreview>,
    module_files: Entity<FileBrowser>,
    inbox: Entity<super::inbox::InboxPage>,
    docs: std::collections::HashMap<std::path::PathBuf, AnyView>,
    version_pane: Option<Entity<super::history::VersionPane>>,
    notes_page: Option<Entity<super::notes_page::NotesPage>>,
    graph_page: Option<Entity<super::graph_view::GraphView>>,
    cards_page: Option<Entity<super::cards_page::CardsPage>>,
    cards_shown: bool,
    split_share: f32,
    shown_doc: Option<std::path::PathBuf>,
    /// The shown document gets the keyboard once no dialog holds it any more.
    doc_needs_focus: bool,
    inbox_shown: bool,
    /// Tabs by last use; only the latest keep their view, a PDF view holds pages and threads.
    doc_recent: Vec<std::path::PathBuf>,
    search_split: std::cell::Cell<bool>,
    /// The module shown last, to take the keyboard when another one opens.
    module_shown: Option<String>,
    module_needs_focus: bool,
    /// Width of the page area, set by the main window each frame.
    pub width: f32,
    fields: Fields,
    /// A tab or file dragged over the documents, and where it would land.
    doc_drop: Option<DropHover>,
    /// A drag of a tab or file is under way.
    doc_dragging: bool,
    /// Counts drags, so each one's overlay starts where it first appears.
    drop_epoch: usize,
    drop_rect: (f32, f32),
    _subs: Vec<Subscription>,
}

/// Where a dragged document would land, kept while the pointer stays in that zone.
#[derive(Clone, PartialEq)]
struct DropHover {
    path: std::path::PathBuf,
    to: DocDrop,
    /// Only the right edge takes it: the canvas below takes files itself.
    band: bool,
    fallback: Option<std::path::PathBuf>,
}

/// The share of the width at the right edge that opens a file beside a canvas.
const CANVAS_BAND: f32 = 0.2;

/// Switches to two columns at `on` and back at `off`, so resizing near the limit doesn't flicker.
fn two_columns(state: &std::cell::Cell<bool>, width: f32, on: f32, off: f32) -> bool {
    state.set(if state.get() { width >= off } else { width >= on });
    state.get()
}

impl Pages {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let lib = state::lib(cx);
        let search = cx.new(|cx| FileBrowser::new(None, true, cx));
        let module_files = cx.new(|cx| FileBrowser::new(None, false, cx));
        let preview = cx.new(|_| FilePreview::new(true));
        let inbox = cx.new(super::inbox::InboxPage::new);
        let subs = vec![
            cx.observe(&lib, |this, lib, cx| {
                let (query, module_query, route) = {
                    let l = lib.read(cx);
                    (l.search_query.clone(), l.module_query.clone(), l.route().clone())
                };
                this.search.update(cx, |b, cx| b.set_query(&query, cx));
                if let Route::Module(id) = route {
                    this.module_files.update(cx, |b, cx| {
                        b.set_scope(Some(id), cx);
                        b.set_query(&module_query, cx);
                    });
                    if !module_query.is_empty() {
                        this.module.tab = module_page::ModuleTab::Files;
                    }
                }
                cx.notify();
            }),
            {
                let me = cx.weak_entity();
                cx.intercept_keystrokes(move |e, window, cx| {
                    let dragging = me.upgrade().is_some_and(|p| p.read(cx).doc_dragging);
                    if dragging && e.keystroke.key == "escape" && cx.stop_active_drag(window) {
                        cx.stop_propagation();
                        if let Some(p) = me.upgrade() {
                            p.update(cx, |p, cx| {
                                p.doc_dragging = false;
                                p.doc_drop = None;
                                cx.notify();
                            });
                        }
                    }
                })
            },
            cx.observe(&search, |this, b, cx| {
                let selected = b.read(cx).selected.clone();
                this.preview.update(cx, |p, cx| p.set_path(selected, cx));
            }),
        ];
        Self {
            grid: GridState::default(),
            record: RecordState::default(),
            checks: ChecksState::default(),
            module: ModuleState { tab: module_page::ModuleTab::saved(), focus: Some(cx.focus_handle()), ..Default::default() },
            search,
            preview,
            module_files,
            inbox,
            docs: Default::default(),
            version_pane: None,
            notes_page: None,
            graph_page: None,
            cards_page: None,
            cards_shown: false,
            split_share: crate::core::prefs::get().split_share,
            shown_doc: None,
            doc_needs_focus: false,
            inbox_shown: false,
            doc_recent: Vec::new(),
            search_split: Default::default(),
            module_shown: None,
            module_needs_focus: false,
            width: 1000.,
            fields: Fields::default(),
            doc_drop: None,
            doc_dragging: false,
            drop_epoch: 0,
            drop_rect: (0., 1.),
            _subs: subs,
        }
    }

    /// Return in the search field opens the selected search result.
    pub fn open_search_selection(&mut self, cx: &mut Context<Self>) {
        let search = self.search.clone();
        search.update(cx, |b, cx| b.open_selected(cx));
    }

    pub fn move_search_selection(&mut self, step: i64, cx: &mut Context<Self>) {
        self.search.update(cx, |b, cx| b.move_selection(step, cx));
    }

    fn search_page(&self, lib: &Library, width: f32, t: &Theme) -> AnyElement {
        let split = two_columns(&self.search_split, width, 940., 860.);
        let q = lib.search_query.trim().to_string();
        let entries: Vec<_> = if q.chars().count() >= 2 {
            let needle = fold(&q);
            let mut v: Vec<_> = lib.assignments.iter().filter(|a| fold(&a.title).contains(&needle) || fold(&lib.note(&a.id)).contains(&needle)).collect();
            v.sort_by_key(|a| (a.due - lib.now).num_seconds().abs());
            v.truncate(3);
            v
        } else {
            Vec::new()
        };
        let bar = (!entries.is_empty()).then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(space::S))
                .px(px(space::L))
                .py(px(space::S))
                .border_t_1()
                .border_color(t.separator)
                .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(tr("Einträge")))
                .children(entries.into_iter().map(|a| {
                    let id = a.id.clone();
                    let color = t.palette(lib.module(a).map_or("blue", |m| m.color.as_str()));
                    let hover = t.ink.opacity(0.06);
                    div()
                        .id(SharedString::from(format!("entry-{}", a.id)))
                        .flex()
                        .items_center()
                        .gap(px(space::XS))
                        .px(px(space::S))
                        .py(px(space::XS))
                        .rounded(px(radius::INNER))
                        .cursor_pointer().key_nav()
                        .soft(hover, radius::INNER)
                        .text_size(px(font::CAPTION))
                        .tooltip(tip(tr("Details öffnen")))
                        .child(div().size(px(space::S)).rounded_full().bg(color))
                        .child(div().truncate().max_w(px(220.)).child(a.title.clone()))
                        .child(div().text_color(t.secondary).child(day::day_label(a.due, lib.now)))
                        .on_click(move |_, _, cx| act(cx, |l| l.select(Some(id.clone()))))
                }))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(div().h_full().when(split, |d| d.w(px((width * 0.45).round().min(560.)))).when(!split, |d| d.flex_1()).child(self.search.clone()))
                    .when(split, |d| d.child(div().h_full().flex_1().min_w_0().border_l_1().border_color(t.separator).child(self.preview.clone()))),
            )
            .children(bar)
            .into_any_element()
    }

    /// Switches the module page's view and hands the keyboard to it.
    pub fn show_module_tab(&mut self, tab: module_page::ModuleTab, cx: &mut Context<Self>) {
        self.module.tab = tab;
        tab.remember();
        self.module.scroll.set_offset(point(px(0.), px(0.)));
        self.module_needs_focus = true;
        cx.notify();
    }

    fn module_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        use module_page::ModuleTab as Tab;
        let m = &e.keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        let Route::Module(id) = state::read(cx, |l| l.route().clone()) else { return };
        match e.keystroke.key.as_str() {
            "1" => self.show_module_tab(Tab::Overview, cx),
            "2" => self.show_module_tab(Tab::Deadlines, cx),
            "3" => {
                self.show_module_tab(Tab::Files, cx);
                self.module_needs_focus = false;
                self.module_files.update(cx, |b, cx| b.take_focus(window, cx));
            }
            "n" => act(cx, |l| l.new_note = Some(crate::core::library::NewNoteRequest { module_id: Some(id), folder: None })),
            "f" => {
                let next = state::read(cx, |l| l.next_in_module(&id));
                super::platform::pick_files_for_filing(next, cx);
            }
            "up" | "down" => {
                let rows: Vec<String> = state::read(cx, |l| match (l.module_by_id(&id), self.module.tab) {
                    (Some(m), Tab::Overview) => module_page::next_up(l, m).iter().map(|a| a.id.clone()).collect(),
                    (Some(m), Tab::Deadlines) => module_page::deadline_order(l, m, &self.module),
                    _ => Vec::new(),
                });
                if rows.is_empty() {
                    return;
                }
                let at = state::read(cx, |l| l.selected().and_then(|s| rows.iter().position(|r| *r == s.id)));
                let next = match (at, e.keystroke.key.as_str()) {
                    (None, _) => 0,
                    (Some(i), "down") => (i + 1).min(rows.len() - 1),
                    (Some(i), _) => i.saturating_sub(1),
                };
                let picked = rows[next].clone();
                act(cx, |l| l.select(Some(picked)));
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn module_page(&self, lib: &Library, id: &str, width: f32, view: Entity<Self>, t: &Theme) -> AnyElement {
        let Some(m) = lib.module_by_id(id).cloned() else {
            return unavailable("folder-search", tr("Modul nicht gefunden"), "", t).into_any_element();
        };
        use module_page::ModuleTab as Tab;
        let scrolled = |inner: Div| div().flex_1().min_h_0().relative().child(div().id("module-body").size_full().overflow_y_scroll().track_scroll(&self.module.scroll).child(inner)).vertical_scrollbar(&self.module.scroll);
        let body = match self.module.tab {
            Tab::Overview => scrolled(module_page::overview(lib, &m, width, &self.module, view.clone(), t)),
            Tab::Deadlines => scrolled(div().w_full().max_w(px(820.)).mx_auto().child(module_page::deadlines(lib, &m, &self.module, view.clone(), t))),
            Tab::Files => div().flex_1().min_h_0().child(self.module_files.clone()),
        };
        let accent = t.accent;
        let code = m.code.clone();
        let focus = self.module.focus.clone();
        let (key_view, dismiss_view) = (view_for_keys(&view), view_for_keys(&view));
        div()
            .id("module-page")
            .group("module-page")
            .when_some(focus, |d, f| d.track_focus(&f))
            .key_context("ModulePage")
            .on_key_down(move |e: &KeyDownEvent, window, cx| key_view.update(cx, |p, cx| p.module_key(e, window, cx)))
            .capture_action(move |_: &super::main_window::Dismiss, window, cx| {
                let focused = dismiss_view.read(cx).module.focus.as_ref().is_some_and(|f| f.is_focused(window));
                if focused {
                    cx.stop_propagation();
                    act(cx, |l| {
                        l.show_inspector = false;
                        l.select(None);
                    });
                }
            })
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(module_page::header(lib, &m, width, &self.module, view, t))
            .child(body)
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                let paths = paths.paths().to_vec();
                act(cx, |l| l.request_filing(paths, None));
            })
            .child(
                    div()
                        .absolute()
                        .inset(px(10.))
                        .invisible()
                        .hover(|s| s)
                        .group_drag_over::<ExternalPaths>("module-page", |s| s.visible())
                        .rounded(px(radius::CARD))
                        .border(px(3.))
                        .border_dashed()
                        .border_color(accent)
                        .bg(t.content.blend(accent.opacity(0.12)).opacity(0.94))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(space::S))
                        .text_size(px(font::TITLE3))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(accent)
                        .child(icon("inbox", iconsize::M, accent))
                        .child(crate::core::i18n::trf("In %@ ablegen", &[&code])),
            )
            .into_any_element()
    }
}

const KEPT_VIEWS: usize = 8;

fn view_for_keys(view: &Entity<Pages>) -> Entity<Pages> {
    view.clone()
}

impl Pages {
    fn doc_view(&mut self, path: &std::path::Path, window: &mut Window, cx: &mut Context<Self>) -> AnyView {
        if self.doc_recent.first().map(|p| p.as_path()) != Some(path) {
            self.doc_recent.retain(|p| p != path);
            self.doc_recent.insert(0, path.to_path_buf());
            self.doc_recent.truncate(KEPT_VIEWS);
        }
        self.docs
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let path = path.to_path_buf();
                let kind = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                if crate::core::markdown::is_markdown(&path) {
                    cx.new(|cx| super::documents::NoteView::new(path, true, window, cx)).into()
                } else if kind == "base" {
                    cx.new(|cx| super::base_view::BaseView::new(path, cx)).into()
                } else if kind == "canvas" {
                    cx.new(|cx| super::canvas_view::CanvasView::new(path, window, cx)).into()
                } else {
                    cx.new(|cx| super::documents::FileViewer::new(path, cx)).into()
                }
            })
            .clone()
    }

    /// The notes room: the workspace's files beside whatever note is shown.
    fn with_explorer(&mut self, body: AnyElement, width: f32, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let route = state::read(cx, |l| l.route().clone());
        if !matches!(route, Route::Doc(_) | Route::Notes) {
            return body;
        }
        let page = self.notes_page.get_or_insert_with(|| cx.new(|cx| super::notes_page::NotesPage::new(window, cx))).clone();
        let shown = !crate::core::prefs::get().notes_explorer_hidden && width >= 620.;
        div().size_full().flex().when(shown, |d| d.child(page)).child(div().flex_1().min_w_0().h_full().child(body)).into_any_element()
    }

    fn document(&mut self, path: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let view = self.doc_view(&path, window, cx);
        let fallback = self.doc_recent.iter().find(|p| **p != path).cloned();
        let beside = state::read(cx, |l| l.split_document.clone()).filter(|p| *p != path);
        let (quiet, asked) = state::lib(cx).update(cx, |l, _| (std::mem::take(&mut l.quiet_open), std::mem::take(&mut l.focus_document)));
        if self.shown_doc.as_ref() != Some(&path) {
            self.shown_doc = Some(path);
            self.doc_needs_focus = !quiet;
        }
        self.doc_needs_focus |= asked;
        if self.doc_needs_focus && !window.has_active_dialog(cx) {
            self.doc_needs_focus = false;
            let target = view.clone();
            window.on_next_frame(move |window, cx| {
                if let Ok(note) = target.clone().downcast::<super::documents::NoteView>() {
                    note.update(cx, |n, cx| n.activate(window, cx));
                    if let Some(action) = cx.default_global::<super::palette::PendingNoteAction>().0.take() {
                        window.dispatch_action(action, cx);
                    }
                } else if let Ok(canvas) = target.clone().downcast::<super::canvas_view::CanvasView>() {
                    canvas.update(cx, |c, cx| c.activate(window, cx));
                } else if let Ok(base) = target.clone().downcast::<super::base_view::BaseView>() {
                    base.update(cx, |b, cx| b.activate(window, cx));
                } else if let Ok(file) = target.downcast::<super::documents::FileViewer>() {
                    file.update(cx, |f, cx| f.activate(window, cx));
                }
            });
        }
        let canvas = |v: &AnyView| v.clone().downcast::<super::canvas_view::CanvasView>().is_ok();
        let left_canvas = canvas(&view);
        let split = beside.is_some();
        let area = match beside {
            Some(other) => {
                let right = self.doc_view(&other, window, cx);
                let right_canvas = canvas(&right);
                let t = Theme::get(cx);
                let (accent, me) = (t.accent, cx.entity());
                let area = div()
                    .size_full()
                    .flex()
                    .on_drag_move::<module_page::SplitDrag>(move |e, _, cx| {
                        let share = f32::from(e.event.position.x - e.bounds.left()) / f32::from(e.bounds.size.width);
                        me.update(cx, |p, cx| {
                            p.split_share = share.clamp(0.25, 0.75);
                            cx.notify();
                        })
                    })
                    .on_drop(cx.listener(|p, _: &module_page::SplitDrag, _, _| {
                        let share = p.split_share;
                        crate::core::prefs::update(|prefs| prefs.split_share = share);
                    }))
                    .child(div().flex_none().min_w_0().h_full().w(relative(self.split_share)).child(view))
                    .child(div().id("doc-split").h_full().w(px(space::XS)).flex_none().border_r_1().border_color(t.separator).soft(accent.opacity(0.3), 0.).cursor_col_resize().on_drag(module_page::SplitDrag, |d, _, _, cx| cx.new(|_| d.clone())))
                    .child(div().flex_1().min_w_0().h_full().child(right))
                    .into_any_element();
                (area, right_canvas)
            }
            None => (view.into_any_element(), false),
        };
        let (area, right_canvas) = area;
        let panes = DropPanes { split, share: self.split_share, left_canvas, right_canvas, fallback };
        self.drop_area(area, panes, window, cx)
    }

    fn drop_area(&mut self, area: AnyElement, panes: DropPanes, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::get(cx);
        let rect = self.doc_drop.as_ref().map(|h| panes.rect(h));
        let split = panes.split;
        if let Some(r) = rect {
            self.drop_rect = r;
        }
        let alpha = super::motion::follow("doc-drop-alpha", if rect.is_some() { 1. } else { 0. }, super::motion::FAST, window, cx);
        let (me, tab_panes, file_panes) = (cx.entity(), panes.clone(), panes);
        let file_me = me.clone();
        let mut container = div()
            .relative()
            .size_full()
            .on_drag_move::<super::documents::DraggedTab>(move |e, _, cx| {
                let path = e.drag(cx).path.clone();
                me.update(cx, |p, cx| p.hover_drop(path, false, e.event.position, e.bounds, &tab_panes, cx));
            })
            .on_drag_move::<super::browser::DraggedFile>(move |e, _, cx| {
                let path = e.drag(cx).0.clone();
                file_me.update(cx, |p, cx| p.hover_drop(path, true, e.event.position, e.bounds, &file_panes, cx));
            })
            .when(rect.is_some(), |d| {
                d.on_drop(cx.listener(|p, d: &super::documents::DraggedTab, _, cx| p.drop_here(&d.path, cx)))
                    .on_drop(cx.listener(|p, d: &super::browser::DraggedFile, _, cx| p.drop_here(&d.0, cx)))
            })
            .child(area);
        if alpha > 0.01 {
            let target = rect.unwrap_or(self.drop_rect);
            let epoch = self.drop_epoch;
            let left = super::motion::follow(("doc-drop-left", epoch), target.0, super::motion::FAST, window, cx);
            let right = super::motion::follow(("doc-drop-right", epoch), target.1, super::motion::FAST, window, cx);
            let beside = self.doc_drop.as_ref().is_none_or(|h| h.to == DocDrop::Beside);
            let path = self.doc_drop.as_ref().map(|h| h.path.clone());
            let (label, mark) = if beside { (tr(if split { "Rechts öffnen" } else { "Daneben öffnen" }), icon("columns-2", iconsize::S, t.accent).into_any_element()) } else { (tr(if split { "Links öffnen" } else { "Hier öffnen" }), path.as_deref().map_or_else(|| div().into_any_element(), |p| file_icon(p, iconsize::S).into_any_element())) };
            let pill = div().flex().items_center().gap(px(space::S)).px(px(space::M)).py(px(space::XS)).rounded_full().bg(t.material).shadow_md().text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).text_color(t.accent).child(mark).child(label);
            let overlay = div()
                .absolute()
                .top(px(control::SUBBAR))
                .bottom_0()
                .left(relative(left))
                .w(relative((right - left).max(0.)))
                .p(px(space::S))
                .opacity(alpha)
                .when(rect.is_some(), |d| {
                    d.on_drop(cx.listener(|p, d: &super::documents::DraggedTab, _, cx| p.drop_here(&d.path, cx)))
                        .on_drop(cx.listener(|p, d: &super::browser::DraggedFile, _, cx| p.drop_here(&d.0, cx)))
                })
                .child(div().size_full().rounded(px(radius::CARD)).bg(t.accent.opacity(0.12)).border_1().border_color(t.accent.opacity(0.5)).flex().justify_center().pt(px(space::XL)).child(div().child(pill)));
            container = container.child(overlay);
        }
        container.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn hover_drop(&mut self, path: std::path::PathBuf, file: bool, at: Point<Pixels>, bounds: Bounds<Pixels>, panes: &DropPanes, cx: &mut Context<Self>) {
        if !self.doc_dragging {
            self.doc_dragging = true;
            self.drop_epoch += 1;
        }
        let top = bounds.top() + px(control::SUBBAR);
        let x = if bounds.size.width > px(0.) { f32::from(at.x - bounds.left()) / f32::from(bounds.size.width) } else { 0. };
        let zone = (bounds.contains(&at) && at.y >= top).then(|| panes.zone(x, file)).flatten();
        let hover = zone.and_then(|(to, band)| {
            let fallback = panes.fallback.clone();
            let ok = match &self.doc_drop {
                Some(h) if h.path == path && h.to == to && h.band == band => true,
                _ => state::read(cx, |l| l.can_drop_document(&path, to, fallback.as_deref())),
            };
            ok.then_some(DropHover { path, to, band, fallback })
        });
        if hover != self.doc_drop {
            self.doc_drop = hover;
            cx.notify();
        }
    }

    fn drop_here(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        self.doc_dragging = false;
        let Some(h) = self.doc_drop.take().filter(|h| h.path == path) else { return };
        act(cx, |l| l.drop_document(&h.path, h.to, h.fallback.as_deref()));
        cx.notify();
    }
}

/// How the document area is split, for the drop zones of a dragged tab or file.
#[derive(Clone)]
struct DropPanes {
    split: bool,
    share: f32,
    left_canvas: bool,
    right_canvas: bool,
    fallback: Option<std::path::PathBuf>,
}

impl DropPanes {
    fn zone(&self, x: f32, file: bool) -> Option<(DocDrop, bool)> {
        let divide = if self.split { self.share } else { 0.5 };
        let on_right = self.split && x >= divide;
        let canvas = file && if on_right { self.right_canvas } else { self.left_canvas };
        if canvas {
            return (x >= 1. - CANVAS_BAND).then_some((DocDrop::Beside, true));
        }
        Some((if x >= divide { DocDrop::Beside } else { DocDrop::Here }, false))
    }

    fn rect(&self, h: &DropHover) -> (f32, f32) {
        match (h.to, h.band, self.split) {
            (_, true, _) => (1. - CANVAS_BAND, 1.),
            (DocDrop::Beside, _, true) => (self.share, 1.),
            (DocDrop::Beside, _, false) => (0.5, 1.),
            (DocDrop::Here, _, true) => (0., self.share),
            (DocDrop::Here, _, false) => (0., 1.),
        }
    }
}

impl Render for Pages {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.doc_dragging = false;
            self.doc_drop = None;
        }
        let open = state::read(cx, |l| l.open_documents.clone());
        if !matches!(state::read(cx, |l| l.route().clone()), Route::Doc(_)) {
            self.shown_doc = None;
        }
        self.doc_recent.retain(|p| open.contains(p));
        let recent = &self.doc_recent;
        self.docs.retain(|p, _| recent.contains(p));
        let t = Theme::get(cx);
        let lib = state::lib(cx);
        let view = cx.entity();
        let width = self.width;
        let route = lib.read(cx).route().clone();
        let on_inbox = route == Route::Inbox;
        let focus_back = state::take_page_focus_request(cx);
        self.doc_needs_focus |= focus_back;
        if on_inbox && (!self.inbox_shown || focus_back) && !window.has_active_dialog(cx) {
            let inbox = self.inbox.clone();
            window.on_next_frame(move |window, cx| inbox.update(cx, |i, cx| i.activate(window, cx)));
        }
        self.inbox_shown = on_inbox;
        if !matches!(route, Route::Cards) {
            self.cards_shown = false;
        }
        let mut fields = std::mem::take(&mut self.fields);
        let body: AnyElement = match route {
            Route::Grid => div().relative().size_full().child(div().id("grid-scroll").size_full().overflow_y_scroll().track_scroll(&self.grid.scroll).child(grid::render(lib.read(cx), &self.grid, view, &t))).vertical_scrollbar(&self.grid.scroll).into_any_element(),
            Route::Record => match lib.read(cx).config.clone() {
                Some(c) => div().id("record-scroll").size_full().overflow_y_scrollbar().child(record::render(&c, &self.record, &mut fields, window, cx, &t)).into_any_element(),
                None => div().into_any_element(),
            },
            Route::Checks => {
                let st = std::mem::take(&mut self.checks);
                let el = checks::render(&lib, &st, &mut fields, window, cx, &t);
                let names = std::mem::take(&mut self.checks.names);
                self.checks = st;
                self.checks.names.extend(names);
                el
            }
            Route::Inbox => self.inbox.clone().into_any_element(),
            Route::Search => self.search_page(lib.read(cx), width, &t),
            Route::Module(id) => {
                if self.module_shown.as_deref() != Some(id.as_str()) {
                    self.module_shown = Some(id.clone());
                    self.module_needs_focus = true;
                }
                if self.module_needs_focus && !window.has_active_dialog(cx) {
                    self.module_needs_focus = false;
                    if self.module.tab != module_page::ModuleTab::Files
                        && let Some(focus) = self.module.focus.clone()
                    {
                        window.on_next_frame(move |window, cx| window.focus(&focus, cx));
                    }
                }
                self.module_page(lib.read(cx), &id, width, view, &t)
            }
            Route::Doc(path) if super::history::viewed_path(cx).as_ref() == Some(&path) => {
                let pane = self.version_pane.get_or_insert_with(|| cx.new(super::history::VersionPane::new)).clone();
                pane.into_any_element()
            }
            Route::Doc(path) => self.document(path, window, cx),
            Route::Graph => {
                let page = self.graph_page.get_or_insert_with(|| cx.new(|cx| super::graph_view::GraphView::new(window, cx))).clone();
                page.into_any_element()
            }
            Route::Cards => {
                let page = self.cards_page.get_or_insert_with(|| cx.new(|cx| super::cards_page::CardsPage::new(window, cx))).clone();
                if !std::mem::replace(&mut self.cards_shown, true) {
                    let focus = page.clone();
                    window.on_next_frame(move |window, cx| focus.update(cx, |p, cx| p.take_focus(window, cx)));
                }
                page.into_any_element()
            }
            Route::Notes => unavailable("book-open", tr("Keine Notiz offen"), tr("Wähle links eine Notiz oder lege mit ⌘N eine neue an."), &t).into_any_element(),
            Route::Today => div().into_any_element(),
        };
        fields.prune();
        self.fields = fields;
        self.with_explorer(body, width, window, cx)
    }
}
