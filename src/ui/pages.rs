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
use super::theme::{Theme, font};
use super::widgets::*;
use crate::core::day;
use crate::core::i18n::tr;
use crate::core::library::{Library, Route};
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
    module_drop: bool,
    docs: std::collections::HashMap<std::path::PathBuf, AnyView>,
    shown_doc: Option<std::path::PathBuf>,
    /// Width of the page area, set by the main window each frame.
    pub width: f32,
    fields: Fields,
    _subs: Vec<Subscription>,
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
                        this.module.files_pane = true;
                    }
                }
                cx.notify();
            }),
            cx.observe(&search, |this, b, cx| {
                let selected = b.read(cx).selected.clone();
                this.preview.update(cx, |p, cx| p.set_path(selected, cx));
            }),
        ];
        Self {
            grid: GridState::default(),
            record: RecordState::default(),
            checks: ChecksState::default(),
            module: ModuleState::default(),
            search,
            preview,
            module_files,
            inbox,
            module_drop: false,
            docs: Default::default(),
            shown_doc: None,
            width: 1000.,
            fields: Fields::default(),
            _subs: subs,
        }
    }

    /// Return in the search field opens the selected search result.
    pub fn open_search_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.search.read(cx).selected.clone() {
            act(cx, |l| l.open_file(&p));
        }
    }

    pub fn move_search_selection(&mut self, step: i64, cx: &mut Context<Self>) {
        self.search.update(cx, |b, cx| b.move_selection(step, cx));
    }

    fn search_page(&self, lib: &Library, width: f32, t: &Theme) -> AnyElement {
        let split = width >= 940.;
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
                .gap(px(8.))
                .px(px(16.))
                .py(px(6.))
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
                        .gap(px(5.))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .text_size(px(font::CAPTION))
                        .tooltip(tip(tr("Details öffnen")))
                        .child(div().size(px(7.)).rounded_full().bg(color))
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

    fn module_page(&self, lib: &Library, id: &str, width: f32, view: Entity<Self>, t: &Theme) -> AnyElement {
        let Some(m) = lib.module_by_id(id).cloned() else {
            return unavailable("folder-search", tr("Modul nicht gefunden"), "", t).into_any_element();
        };
        let split = width >= 900.;
        let deadlines = div().id("module-deadlines").size_full().overflow_y_scroll().child(module_page::deadlines(lib, &m, &self.module, view.clone(), t));
        let body = if split {
            div().flex_1().min_h_0().flex().child(div().h_full().w(px((width * 0.42).round().clamp(320., 500.))).border_r_1().border_color(t.separator).child(deadlines)).child(div().h_full().flex_1().min_w_0().child(self.module_files.clone()))
        } else if self.module.files_pane {
            div().flex_1().min_h_0().child(self.module_files.clone())
        } else {
            div().flex_1().min_h_0().child(deadlines)
        };
        let accent = t.accent;
        let code = m.code.clone();
        let (v1, v2) = (view.clone(), view.clone());
        div()
            .id("module-page")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(module_page::header(lib, &m, split, &self.module, view, t))
            .child(body)
            .on_drag_move::<ExternalPaths>(move |_, _, cx| v1.update(cx, |p, cx| {
                if !p.module_drop {
                    p.module_drop = true;
                    cx.notify();
                }
            }))
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                let paths = paths.paths().to_vec();
                v2.update(cx, |p, cx| {
                    p.module_drop = false;
                    cx.notify();
                });
                act(cx, |l| l.request_filing(paths, None));
            })
            .when(self.module_drop, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset(px(10.))
                        .rounded(px(12.))
                        .border(px(3.))
                        .border_dashed()
                        .border_color(accent)
                        .bg(accent.opacity(0.06))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(8.))
                        .text_size(px(font::TITLE3))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(accent)
                        .child(icon("inbox", 18., accent))
                        .child(crate::core::i18n::trf("In %@ ablegen", &[&code])),
                )
            })
            .into_any_element()
    }
}

impl Pages {
    fn document(&mut self, path: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let view = self
            .docs
            .entry(path.clone())
            .or_insert_with(|| {
                let path = path.clone();
                if crate::core::markdown::is_markdown(&path) {
                    cx.new(|cx| super::documents::NoteView::new(path, true, window, cx)).into()
                } else {
                    cx.new(|cx| super::documents::FileViewer::new(path, cx)).into()
                }
            })
            .clone();
        if self.shown_doc.as_ref() != Some(&path) {
            self.shown_doc = Some(path);
            if let Ok(note) = view.clone().downcast::<super::documents::NoteView>() {
                window.defer(cx, move |window, cx| note.update(cx, |n, cx| n.activate(window, cx)));
            }
        }
        view.into_any_element()
    }
}

impl Render for Pages {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = state::read(cx, |l| l.open_documents.clone());
        if !matches!(state::read(cx, |l| l.route().clone()), Route::Doc(_)) {
            self.shown_doc = None;
        }
        self.docs.retain(|p, _| open.contains(p));
        let t = Theme::get(cx);
        let lib = state::lib(cx);
        let view = cx.entity();
        let width = self.width;
        let route = lib.read(cx).route().clone();
        if !matches!(route, Route::Module(_)) {
            self.module_drop = false;
        }
        let mut fields = std::mem::take(&mut self.fields);
        let body: AnyElement = match route {
            Route::Grid => div().id("grid-scroll").size_full().overflow_y_scroll().child(grid::render(lib.read(cx), &self.grid, view, &t)).into_any_element(),
            Route::Record => match lib.read(cx).config.clone() {
                Some(c) => div().id("record-scroll").size_full().overflow_y_scroll().child(record::render(&c, &self.record, &mut fields, window, cx, &t)).into_any_element(),
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
            Route::Module(id) => self.module_page(lib.read(cx), &id, width, view, &t),
            Route::Doc(path) => self.document(path, window, cx),
            Route::Today => div().into_any_element(),
        };
        fields.prune();
        self.fields = fields;
        body
    }
}
