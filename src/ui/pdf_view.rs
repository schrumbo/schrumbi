use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::{self, Soft};
use super::pdf_worker::{self, Hit, Message, PageKey, RenderQueue, TextJob};
use super::theme::{Kind, Theme, control, font, iconsize, pages, radius, space, tint};
use super::widgets::*;
use crate::core::highlights::{self, Highlight, HighlightPart};
use crate::core::library::{PdfWriteError, WrittenPdf};
use crate::core::i18n::{tr, trf};
use crate::core::office::{self, OpenError};
use crate::core::pdf_edit::{self, EditError, PageOp};
use crate::core::pdf_render::{LinkTarget, MARK_OPACITY, MarkPaint, PageRect, PageText, PdfDoc, PdfMeta, marks_hash};
use crate::core::prefs::{self, PdfPosition};
use crate::core::shortcuts::ShortcutAction;

actions!(schrumbi_pdf, [Escape, MarkSelection, RotatePagesLeft, RotatePagesRight, MovePagesUp, MovePagesDown, DeletePages, InsertPdf]);

/// Key bindings of the viewer; Escape must reach it before text fields and the main window take it. The page commands read their keys in `on_key`, so plain letters never reach a text field.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("escape", Escape, Some("PdfView")), KeyBinding::new("escape", Escape, Some("PdfView > Input"))]
}

const GAP: f32 = space::M;
const PADDING: f32 = space::L;
const PANEL_WIDTH: f32 = 240.;
const FLOAT_BELOW: f32 = 2. * PANEL_WIDTH;
const MARK_COLORS: [&str; 4] = ["yellow", "green", "blue", "pink"];
const MARK_BAR_WIDTH: f32 = 148.;
const MARK_UNDERLINE: f32 = 2.;
const PREVIEW: u16 = 2;
const MAX_SHARP_WIDTH: f32 = 4096.;
const SHARP_BUDGET: usize = 128 * 1024 * 1024;
const PREVIEW_BUDGET: usize = 48 * 1024 * 1024;
const TEXT_PAGES_KEPT: usize = 32;
const SHARP_SETTLE: Duration = Duration::from_millis(140);
const LINE_STEP: f32 = 48.;
const MIN_SCALE: f32 = 0.25;
const MAX_SCALE: f32 = 6.;
const THUMB_ROW: f32 = 176.;
const OUTLINE_ROW: f32 = 28.;
const HIT_ROW: f32 = 56.;
const SPINNER_AFTER: Duration = Duration::from_millis(150);
const NOTICE_FOR: Duration = Duration::from_secs(8);
const DRAG_EDGE: f32 = 28.;
const DRAG_SCROLL: f32 = 16.;
const NOTICE_WIDTH: f32 = 360.;
const MENU_AFTER_MOVE: Duration = Duration::from_millis(40);
const DRAG_TICK: Duration = Duration::from_millis(16);
const KEPT_STATES: usize = 16;

#[derive(Clone, Copy, PartialEq)]
enum Fit {
    Width,
    Page,
    Scale(f32),
}

#[derive(Clone, Copy, PartialEq)]
enum PanelTab {
    Thumbnails,
    Outline,
    Search,
    Marks,
}

/// A place in the document: a point on a page held at a position in the view, in fractions and pixels.
#[derive(Clone, Copy)]
struct Anchor {
    page: usize,
    x: f32,
    y: f32,
    view_x: f32,
    view_y: f32,
}

fn bar_action(id: &'static str, icon_name: &'static str, help: String, t: &Theme) -> Stateful<Div> {
    div().id(id).p(px(space::XXS)).flex().items_center().justify_center().rounded(px(radius::SMALL)).cursor_pointer().key_nav().soft(t.ink.opacity(0.08), radius::SMALL).tooltip(tip(help)).child(icon(icon_name, iconsize::S, t.secondary))
}

/// A page change to run off the UI thread.
enum Edit {
    Op(PageOp),
    Insert { at: usize, file: PathBuf },
}

enum Failure {
    Edit(EditError),
    Write(PdfWriteError),
}

struct Edited {
    op: PageOp,
    written: WrittenPdf,
    inserted: usize,
}

/// What the view held when an edit started.
struct Started {
    sizes: Vec<(f32, f32)>,
    pages: Vec<usize>,
    label: String,
    old_key: String,
}

/// The selection and the place a document had, kept per content so undo and redo bring them back.
#[derive(Clone)]
struct ViewState {
    selected: BTreeSet<usize>,
    anchor: Option<Anchor>,
}

/// Where the view goes once the edited file is loaded: the pages to select, and either the place it kept or the page to show.
struct AfterEdit {
    select: Vec<usize>,
    page: usize,
    anchor: Option<Anchor>,
}

/// Where the page lists of open PDFs are on screen, so a dragged file dropped there is not taken for "open beside".
#[derive(Default)]
pub struct PagePanels(HashMap<EntityId, Bounds<Pixels>>);

impl PagePanels {
    pub fn contains(&self, at: Point<Pixels>) -> bool {
        self.0.values().any(|b| b.contains(&at))
    }

    /// Forgets every list; the ones on screen report again while they are drawn.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

impl Global for PagePanels {}

/// A page of the old file that is dragged, or the pages selected with it, and the view they belong to.
#[derive(Clone)]
struct DraggedPages {
    view: EntityId,
    pages: Vec<usize>,
}

impl Render for DraggedPages {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let label = match self.pages.as_slice() {
            [page] => page_word(false, page + 1),
            many => trf("%lld Seiten", &[&many.len()]),
        };
        div().flex().items_center().gap(px(space::S)).px(px(space::S)).py(px(space::XS)).rounded(px(radius::INNER)).bg(t.material).text_color(t.primary).shadow_md().child(icon("arrow-up-down", iconsize::M, t.secondary)).child(label)
    }
}

/// The page of the new file that a page of the old one became; `None` when it is gone.
fn new_index(op: &PageOp, count: usize, inserted: usize, page: usize) -> Option<usize> {
    match op {
        PageOp::Rotate { .. } => Some(page),
        PageOp::Delete(gone) => (!gone.contains(&page)).then(|| page - gone.iter().copied().filter(|g| *g < page).collect::<BTreeSet<_>>().len()),
        PageOp::Move { pages, to } => pdf_edit::move_order(count, pages, *to).iter().position(|old| *old == page),
        PageOp::Insert { at, .. } => Some(if page >= (*at).min(count) { page + inserted } else { page }),
    }
}

/// The pages to select and the page to show after a change, given the page that was shown and the pages it acted on. After a delete the page that took the first gone one's place is shown.
fn follow_edit(op: &PageOp, count: usize, inserted: usize, current: usize, pages: &[usize]) -> (Vec<usize>, usize, bool) {
    match op {
        PageOp::Rotate { .. } => (pages.to_vec(), current, false),
        PageOp::Delete(gone) => {
            let left = count - gone.iter().copied().filter(|g| *g < count).collect::<BTreeSet<_>>().len();
            (Vec::new(), gone.iter().min().map_or(0, |first| (*first).min(left.saturating_sub(1))), true)
        }
        PageOp::Move { pages: moved, .. } => {
            let mut now: Vec<usize> = moved.iter().filter_map(|p| new_index(op, count, inserted, *p)).collect();
            now.sort_unstable();
            let first = now.first().copied().unwrap_or(0);
            (now, first, true)
        }
        PageOp::Insert { at, .. } => {
            let at = (*at).min(count);
            ((at..at + inserted).collect(), at, true)
        }
    }
}

#[derive(Default)]
struct Geometry {
    scale: f32,
    tops: Vec<f32>,
    total: f32,
    content_width: f32,
    view: (f32, f32),
}

struct Cached {
    image: Arc<RenderImage>,
    baked: u64,
    baked_sigs: Vec<u64>,
    bytes: usize,
    used: u64,
}

/// A selection position: page and caret between glyphs.
type Caret = (usize, usize);

/// A PDF viewer that stays fast with thousands of pages: page positions are computed, not measured,
/// and only pages in view are rendered.
pub struct PdfView {
    path: PathBuf,
    source: pdf_worker::Source,
    slides: bool,
    stamp: Option<(std::time::SystemTime, u64)>,
    sizes: Vec<(f32, f32)>,
    max_size: (f32, f32),
    failed: Option<Option<OpenError>>,
    cache: HashMap<PageKey, Cached>,
    page_marks: Arc<HashMap<usize, Vec<MarkPaint>>>,
    marks_dirty: bool,
    marks_theme: Option<Kind>,
    underlines: HashMap<u64, Hsla>,
    removed_mark: Option<u64>,
    failed_pages: HashMap<PageKey, u64>,
    page_arrived: bool,
    tick: u64,
    queue: Arc<RenderQueue>,
    jobs: async_channel::Sender<TextJob>,
    search_generation: Arc<AtomicU64>,
    last_wanted: Vec<PageKey>,
    scroll: ScrollHandle,
    fit: Fit,
    geo: Geometry,
    view_bounds: Bounds<Pixels>,
    pending: Option<Anchor>,
    zoom_point: Option<(f32, f32)>,
    last_mouse: Option<(f32, f32)>,
    sharp_after: Instant,
    glide: Option<(Instant, Point<Pixels>, Point<Pixels>)>,
    last_offset: Point<Pixels>,
    badge_until: Instant,
    meta: PdfMeta,
    texts: HashMap<usize, (Arc<PageText>, u64)>,
    text_requested: Vec<usize>,
    selection: Option<(Caret, Caret)>,
    selecting: bool,
    hover_cursor: CursorStyle,
    history: Vec<Anchor>,
    focus: FocusHandle,
    focused_once: bool,
    panel_open: bool,
    panel_tab: PanelTab,
    bar: super::bar_fit::BarFit,
    thumbs: UniformListScrollHandle,
    thumb_range: Range<usize>,
    selected: BTreeSet<usize>,
    select_anchor: Option<usize>,
    list_cursor: Option<usize>,
    pages_focus: FocusHandle,
    drop_at: Option<usize>,
    dragging: BTreeSet<usize>,
    drag_step: f32,
    drag_ticking: bool,
    drag_pointer: Option<(Point<Pixels>, Bounds<Pixels>)>,
    states: Vec<(String, ViewState)>,
    restore_on_key: bool,
    queued: Vec<PathBuf>,
    queue_at: usize,
    bar_listed: bool,
    working: Option<Instant>,
    after_edit: Option<AfterEdit>,
    page_notice: Option<(String, String)>,
    notice_task: Option<Task<()>>,
    working_task: Option<Task<()>>,
    followed_page: usize,
    hits: Vec<Hit>,
    current_hit: Option<usize>,
    /// The match to show once the search has found it, as when arriving from the file search.
    wanted_hit: Option<(String, usize)>,
    hit_to_reveal: Option<usize>,
    marks_key: Option<String>,
    marks: Vec<Highlight>,
    active_mark: Option<u64>,
    active_print: Option<(u64, String)>,
    last_color: &'static str,
    hit_list: UniformListScrollHandle,
    search_progress: usize,
    search_active: bool,
    search_input: Option<Entity<InputState>>,
    page_input: Option<Entity<InputState>>,
    zoom_input: Option<Entity<InputState>>,
    editing_page: bool,
    editing_zoom: bool,
    restored: bool,
    reloading: bool,
    save_task: Option<Task<()>>,
    _timer: Option<Task<()>>,
    settle: Option<Task<()>>,
    _load: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl PdfView {
    /// Takes the keyboard once the document is loaded, for views that open it on purpose rather than as a side pane.
    pub fn take_focus_on_load(mut self) -> Self {
        self.focused_once = false;
        self
    }

    pub fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focused_once = true;
        window.focus(&self.focus, cx);
    }

    pub fn new(path: &Path, cx: &mut Context<Self>) -> Self {
        Self::with_source(pdf_worker::Source::Pdf(path.to_path_buf()), false, cx)
    }

    /// A Word or PowerPoint file, laid out as pages off the main thread.
    pub fn office(path: &Path, cx: &mut Context<Self>) -> Self {
        let slides = office::kind(path) == Some(office::Kind::Slides);
        Self::with_source(pdf_worker::Source::Office(path.to_path_buf()), slides, cx)
    }

    fn connect(source: pdf_worker::Source, cx: &mut Context<Self>) -> (Arc<RenderQueue>, async_channel::Sender<TextJob>, Arc<AtomicU64>, Task<()>) {
        let (result_tx, result_rx) = async_channel::unbounded();
        let (queue, jobs, search_generation) = pdf_worker::start(source, result_tx);
        let load = cx.spawn(async move |this, cx| {
            while let Ok(message) = result_rx.recv().await {
                if this.update(cx, |v, cx| v.receive(message, cx)).is_err() {
                    return;
                }
            }
        });
        (queue, jobs, search_generation, load)
    }

    fn with_source(source: pdf_worker::Source, slides: bool, cx: &mut Context<Self>) -> Self {
        let path = match &source {
            pdf_worker::Source::Pdf(p) | pdf_worker::Source::Office(p) => p.clone(),
        };
        let (queue, jobs, search_generation, load) = Self::connect(source.clone(), cx);
        let id = cx.entity_id();
        let release = cx.on_release(move |this: &mut Self, cx| {
            cx.default_global::<PagePanels>().0.remove(&id);
            let lib = super::state::lib(cx);
            lib.update(cx, |l, cx| {
                if l.page_bars.remove(&id.as_u64()).is_some() {
                    cx.notify();
                }
            });
            this.queue.close();
            this.search_generation.fetch_add(1, Ordering::Relaxed);
            this.save_position();
            for (_, c) in this.cache.drain() {
                let _ = cx.drop_image(c.image, None);
            }
        });
        let library = cx.observe(&super::state::lib(cx), |v: &mut Self, _, cx| {
            v.marks_dirty = true;
            v.reload_if_changed(cx);
            cx.notify();
        });
        Self {
            stamp: stamp(&path),
            path,
            source,
            slides,
            sizes: Vec::new(),
            max_size: (1., 1.),
            failed: None,
            cache: HashMap::new(),
            page_marks: Arc::default(),
            marks_dirty: true,
            marks_theme: None,
            underlines: HashMap::new(),
            removed_mark: None,
            failed_pages: HashMap::new(),
            page_arrived: false,
            tick: 0,
            queue,
            jobs,
            search_generation,
            last_wanted: Vec::new(),
            scroll: ScrollHandle::new(),
            fit: Fit::Width,
            geo: Geometry::default(),
            view_bounds: Bounds::default(),
            pending: None,
            zoom_point: None,
            last_mouse: None,
            sharp_after: Instant::now(),
            glide: None,
            last_offset: point(px(0.), px(0.)),
            badge_until: Instant::now(),
            meta: PdfMeta::default(),
            texts: HashMap::new(),
            text_requested: Vec::new(),
            selection: None,
            selecting: false,
            hover_cursor: CursorStyle::Arrow,
            history: Vec::new(),
            focus: cx.focus_handle(),
            focused_once: true,
            panel_open: false,
            panel_tab: PanelTab::Thumbnails,
            bar: Default::default(),
            thumbs: UniformListScrollHandle::new(),
            thumb_range: 0..0,
            selected: BTreeSet::new(),
            select_anchor: None,
            list_cursor: None,
            pages_focus: cx.focus_handle().tab_stop(true),
            drop_at: None,
            dragging: BTreeSet::new(),
            drag_step: 0.,
            drag_ticking: false,
            drag_pointer: None,
            states: Vec::new(),
            restore_on_key: false,
            queued: Vec::new(),
            queue_at: 0,
            bar_listed: false,
            working: None,
            after_edit: None,
            page_notice: None,
            notice_task: None,
            working_task: None,
            followed_page: usize::MAX,
            hits: Vec::new(),
            current_hit: None,
            wanted_hit: None,
            hit_to_reveal: None,
            marks_key: None,
            marks: Vec::new(),
            active_mark: None,
            active_print: None,
            last_color: MARK_COLORS[0],
            hit_list: UniformListScrollHandle::new(),
            search_progress: 0,
            search_active: false,
            search_input: None,
            page_input: None,
            zoom_input: None,
            editing_page: false,
            editing_zoom: false,
            restored: false,
            reloading: false,
            save_task: None,
            _timer: None,
            settle: None,
            _load: Some(load),
            _subs: vec![release, library],
        }
    }

    /// Starts over when the file changed on disk, keeping the place in it.
    fn reload_if_changed(&mut self, cx: &mut Context<Self>) {
        let now = stamp(&self.path);
        if now.is_none() || now == self.stamp {
            return;
        }
        self.stamp = now;
        self.marks_key = None;
        self.selection = None;
        self.active_mark = None;
        self.selecting = false;
        self.queue.close();
        self.search_generation.fetch_add(1, Ordering::Relaxed);
        let (queue, jobs, search_generation, load) = Self::connect(self.source.clone(), cx);
        self.page_marks = Arc::default();
        queue.set_marks(self.page_marks.clone());
        self.queue = queue;
        self.jobs = jobs;
        self.search_generation = search_generation;
        self._load = Some(load);
        self.reloading = true;
    }

    fn receive(&mut self, message: Message, cx: &mut Context<Self>) {
        match message {
            Message::Opened(sizes) if std::mem::take(&mut self.reloading) => {
                for (_, c) in self.cache.drain() {
                    let _ = cx.drop_image(c.image, None);
                }
                self.failed_pages.clear();
                self.texts.clear();
                self.text_requested.clear();
                self.last_wanted.clear();
                self.hits.clear();
                self.current_hit = None;
                self.selection = None;
                self.failed = None;
                self.max_size = sizes.iter().fold((1., 1.), |m, s| (f32::max(m.0, s.0), f32::max(m.1, s.1)));
                self.marks_key = None;
                self.marks.clear();
                self.marks_dirty = true;
                let last = sizes.len().saturating_sub(1);
                self.selected.clear();
                self.list_cursor = None;
                self.restore_on_key = self.after_edit.is_none();
                if let Some(after) = self.after_edit.take() {
                    self.selected = after.select.into_iter().filter(|p| *p <= last).collect();
                    self.select_anchor = self.selected.first().copied();
                    self.pending = Some(match after.anchor {
                        Some(anchor) => Anchor { page: anchor.page.min(last), ..anchor },
                        None => Anchor { page: after.page.min(last), x: 0., y: 0., view_x: 0., view_y: PADDING },
                    });
                }
                self.sizes = sizes;
                self.geo.tops.clear();
            }
            Message::Opened(sizes) => {
                self.max_size = sizes.iter().fold((1., 1.), |m, s| (f32::max(m.0, s.0), f32::max(m.1, s.1)));
                self.sizes = sizes;
            }
            Message::Key(key) => {
                if std::mem::take(&mut self.restore_on_key)
                    && let Some(state) = self.states.iter().find(|(k, _)| *k == key).map(|(_, state)| state.clone())
                {
                    let last = self.page_count().saturating_sub(1);
                    self.selected = state.selected.iter().copied().filter(|p| *p <= last).collect();
                    self.select_anchor = self.selected.first().copied();
                    if let Some(anchor) = state.anchor {
                        self.pending = Some(Anchor { page: anchor.page.min(last), ..anchor });
                    }
                }
                self.marks_key = Some(key);
                self.marks_dirty = true;
                self.run_queue(cx);
            }
            Message::Failed(why) => {
                self.reloading = false;
                self.queued.clear();
                self.failed = Some(why);
            }
            Message::Page(key, image, baked, baked_sigs) => {
                let size = image.size(0);
                let bytes = size.width.0 as usize * size.height.0 as usize * 4;
                self.tick += 1;
                self.page_arrived = true;
                if self.page_hash(key.0) != baked {
                    self.last_wanted.clear();
                }
                self.failed_pages.remove(&key);
                if let Some(old) = self.cache.insert(key, Cached { image, baked, baked_sigs, bytes, used: self.tick }) {
                    let _ = cx.drop_image(old.image, None);
                }
            }
            Message::PageFailed(key, hash) => {
                self.failed_pages.insert(key, hash);
            }
            Message::Meta(meta) => self.meta = meta,
            Message::Text(page, text) => {
                self.tick += 1;
                self.texts.insert(page, (text, self.tick));
                self.text_requested.retain(|p| *p != page);
                if self.texts.len() > TEXT_PAGES_KEPT
                    && let Some(oldest) = self.texts.iter().min_by_key(|(_, (_, used))| *used).map(|(p, _)| *p)
                {
                    self.texts.remove(&oldest);
                }
            }
            Message::Hits(generation, hits) => {
                if generation == self.search_generation.load(Ordering::Relaxed) {
                    let first = self.hits.is_empty();
                    self.hits.extend(hits);
                    match self.wanted_hit.as_ref().map(|(_, nth)| *nth) {
                        Some(nth) if nth < self.hits.len() => {
                            self.wanted_hit = None;
                            self.select_hit(nth, cx);
                        }
                        None if first => self.select_hit(self.first_hit_from_current(), cx),
                        _ => {}
                    }
                }
            }
            Message::SearchProgress(generation, page) => {
                if generation == self.search_generation.load(Ordering::Relaxed) {
                    self.search_progress = page;
                    if page >= self.sizes.len() && self.wanted_hit.take().is_some() && !self.hits.is_empty() {
                        self.select_hit(self.hits.len() - 1, cx);
                    }
                }
            }
            Message::Clipboard(text) => super::drive::copy(text.to_string(), cx),
        }
        cx.notify();
    }

    fn page_count(&self) -> usize {
        self.sizes.len()
    }

    fn scale_for(&self, view: (f32, f32)) -> f32 {
        let width = ((view.0 - PADDING * 2.) / self.max_size.0).max(0.05);
        let height = ((view.1 - PADDING * 2.) / self.max_size.1).max(0.05);
        match self.fit {
            Fit::Width => width,
            Fit::Page => width.min(height),
            Fit::Scale(s) => s,
        }
    }

    fn page_rect(&self, page: usize) -> (f32, f32, f32, f32) {
        let (w, h) = self.sizes[page];
        let (w, h) = (w * self.geo.scale, h * self.geo.scale);
        ((self.geo.content_width - w) / 2., self.geo.tops[page], w, h)
    }

    /// The page whose area contains a vertical position in the content.
    fn page_at(&self, y: f32) -> usize {
        self.geo.tops.partition_point(|top| *top <= y).saturating_sub(1)
    }

    fn offset(&self) -> (f32, f32) {
        let o = self.scroll.offset();
        (f32::from(o.x), f32::from(o.y))
    }

    fn clamped(&self, x: f32, y: f32) -> Point<Pixels> {
        let (vw, vh) = self.geo.view;
        point(px(x.clamp(-(self.geo.content_width - vw).max(0.), 0.)), px(y.clamp(-(self.geo.total - vh).max(0.), 0.)))
    }

    fn anchor_at(&self, view_x: f32, view_y: f32) -> Option<Anchor> {
        if self.geo.tops.is_empty() {
            return None;
        }
        let (ox, oy) = self.offset();
        let page = self.page_at(view_y - oy);
        let (left, top, w, h) = self.page_rect(page);
        Some(Anchor { page, x: (view_x - ox - left) / w.max(1.), y: (view_y - oy - top) / h.max(1.), view_x, view_y })
    }

    fn apply_anchor(&mut self, a: Anchor) {
        let page = a.page.min(self.page_count().saturating_sub(1));
        let (left, top, w, h) = self.page_rect(page);
        self.scroll.set_offset(self.clamped(a.view_x - (left + a.x * w), a.view_y - (top + a.y * h)));
    }

    fn update_geometry(&mut self, view: (f32, f32)) {
        let scale = self.scale_for(view).clamp(0.05, MAX_SCALE);
        if !self.geo.tops.is_empty() && (self.geo.scale - scale).abs() < 0.0005 && self.geo.view == view {
            return;
        }
        let anchor = self.pending.take().or_else(|| {
            let (x, y) = self.zoom_point.take().unwrap_or((view.0 / 2., view.1 / 3.));
            self.anchor_at(x, y)
        });
        let mut top = PADDING;
        let mut tops = Vec::with_capacity(self.sizes.len());
        for (_, h) in &self.sizes {
            tops.push(top);
            top += h * scale + GAP;
        }
        self.geo = Geometry { scale, tops, total: top - GAP + PADDING, content_width: (self.max_size.0 * scale + PADDING * 2.).max(view.0), view };
        if let Some(a) = anchor {
            self.apply_anchor(a);
        }
    }

    /// The page in the middle of attention: the one crossing the upper part of the view.
    pub fn current_page(&self) -> usize {
        if self.geo.tops.is_empty() {
            return 0;
        }
        self.page_at(self.geo.view.1 * 0.4 - self.offset().1)
    }

    fn visible_pages(&self) -> Range<usize> {
        if self.geo.tops.is_empty() {
            return 0..0;
        }
        let top = -self.offset().1;
        self.page_at(top)..(self.page_at(top + self.geo.view.1) + 1).min(self.page_count())
    }

    fn sharp_quarters(&self, page: usize, window: &Window) -> u16 {
        let (w, _) = self.sizes[page];
        let pixels_per_point = (self.geo.scale * window.scale_factor()).min(MAX_SHARP_WIDTH / w.max(1.));
        ((pixels_per_point * 4.).ceil() as u16).max(1)
    }

    fn page_hash(&self, page: usize) -> u64 {
        marks_hash(self.page_marks.get(&page).map(Vec::as_slice).unwrap_or_default())
    }

    /// The cached page that `page_element` shows: the sharp one, else the largest older one, else the preview.
    fn displayed_key(&self, page: usize, window: &Window) -> Option<PageKey> {
        let sharp = (page, self.sharp_quarters(page, window));
        if self.cache.contains_key(&sharp) {
            return Some(sharp);
        }
        let largest = self.cache.keys().filter(|k| k.0 == page && k.1 != PREVIEW).max_by_key(|k| k.1).copied();
        largest.or_else(|| self.cache.contains_key(&(page, PREVIEW)).then_some((page, PREVIEW)))
    }

    fn is_stale(&self, key: PageKey) -> bool {
        let hash = self.page_hash(key.0);
        self.cache.get(&key).is_some_and(|c| c.baked != hash) && self.failed_pages.get(&key) != Some(&hash)
    }

    fn request_pages(&mut self, visible: Range<usize>, window: &Window) {
        let mut wanted: Vec<PageKey> = Vec::new();
        let count = self.page_count();
        let sharp_ready = Instant::now() >= self.sharp_after;
        let mut stale: Vec<PageKey> = Vec::new();
        for page in visible.clone() {
            for key in [self.displayed_key(page, window), Some((page, PREVIEW))].into_iter().flatten() {
                if self.is_stale(key) && !stale.contains(&key) {
                    stale.push(key);
                }
            }
        }
        if self.panel_open && self.panel_tab == PanelTab::Thumbnails {
            let thumbs: Vec<PageKey> = self.thumb_range.clone().map(|page| (page, PREVIEW)).filter(|k| self.is_stale(*k)).collect();
            stale.extend(thumbs);
        }
        let mut add = |key: PageKey, cache: &HashMap<PageKey, Cached>| {
            if (!cache.contains_key(&key) || stale.contains(&key)) && !wanted.contains(&key) {
                wanted.push(key);
            }
        };
        for page in visible.clone() {
            add((page, PREVIEW), &self.cache);
        }
        for key in stale.iter() {
            add(*key, &self.cache);
        }
        if sharp_ready {
            for page in visible.clone() {
                add((page, self.sharp_quarters(page, window)), &self.cache);
            }
        }
        for page in visible.start.saturating_sub(3)..(visible.end + 3).min(count) {
            add((page, PREVIEW), &self.cache);
        }
        if self.panel_open && self.panel_tab == PanelTab::Thumbnails {
            for page in self.thumb_range.clone() {
                add((page, PREVIEW), &self.cache);
            }
        }
        if sharp_ready {
            for page in [visible.start.checked_sub(1), Some(visible.end).filter(|p| *p < count)].into_iter().flatten() {
                add((page, self.sharp_quarters(page, window)), &self.cache);
            }
        }
        if wanted != self.last_wanted {
            self.queue.replace(wanted.clone());
            self.last_wanted = wanted;
        }
    }

    /// Re-reads the highlights when the library or the document changed and hands the render threads their paint.
    fn sync_marks(&mut self, cx: &mut Context<Self>, t: &Theme) {
        if !self.marks_dirty && self.marks_theme == Some(t.kind) {
            return;
        }
        if self.marks_dirty
            && let Some(key) = self.marks_key.clone()
        {
            self.marks = super::state::read(cx, |l| l.highlights_of(&key));
            self.marks_dirty = false;
            if let Some(id) = self.removed_mark
                && self.marks.iter().any(|h| h.id == id)
            {
                self.removed_mark = None;
                self.active_mark = Some(id);
            }
            let now = print_of(&self.marks, self.active_mark);
            if self.active_mark.is_some() {
                match (&self.active_print, &now) {
                    (_, None) => self.active_mark = None,
                    (Some((id, before)), Some((now_id, print))) if id == now_id && before != print => self.active_mark = None,
                    _ => {}
                }
            }
            self.active_print = print_of(&self.marks, self.active_mark);
        }
        self.marks_theme = Some(t.kind);
        self.underlines = self.marks.iter().map(|h| (h.id, t.mark_underline(&h.color))).collect();
        let mut layer: HashMap<usize, Vec<MarkPaint>> = HashMap::new();
        for h in &self.marks {
            let color = Rgba::from(t.palette(&h.color));
            let color = [color.r, color.g, color.b].map(|c| (c * 255.).round().clamp(0., 255.) as u8);
            for part in &h.parts {
                layer.entry(part.page).or_default().push(MarkPaint::new(h.id, &part.rects, color));
            }
        }
        if layer != *self.page_marks {
            self.page_marks = Arc::new(layer);
            self.queue.set_marks(self.page_marks.clone());
            self.last_wanted.clear();
        }
    }

    fn request_text(&mut self, visible: Range<usize>) {
        for page in visible {
            if !self.texts.contains_key(&page) && !self.text_requested.contains(&page) {
                self.text_requested.push(page);
                let _ = self.jobs.try_send(TextJob::Page(page));
            }
        }
    }

    /// Keeps memory bounded: the least recently shown pages leave first. Pages the view currently asks for
    /// stay even over budget, otherwise they would be rendered and dropped again in a loop.
    fn evict(&mut self, visible: &Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let previews = visible.start.saturating_sub(3)..visible.end + 3;
        let sharp = visible.start.saturating_sub(1)..visible.end + 1;
        let needed: Vec<PageKey> = self.cache.keys().filter(|k| if k.1 == PREVIEW { previews.contains(&k.0) || self.thumb_range.contains(&k.0) } else { sharp.contains(&k.0) && k.1 == self.sharp_quarters(k.0, window) }).copied().collect();
        let mut spared = std::mem::take(&mut self.page_arrived);
        for (is_preview, budget) in [(true, PREVIEW_BUDGET), (false, SHARP_BUDGET)] {
            let mut used: usize = self.cache.iter().filter(|(k, _)| (k.1 == PREVIEW) == is_preview).map(|(_, c)| c.bytes).sum();
            while used > budget {
                let oldest = self.cache.iter().filter(|(k, _)| (k.1 == PREVIEW) == is_preview && !needed.contains(k)).min_by_key(|(_, c)| c.used).map(|(k, _)| *k);
                let Some(key) = oldest else { break };
                if spared {
                    window.request_animation_frame();
                    return;
                }
                if let Some(c) = self.cache.remove(&key) {
                    used -= c.bytes;
                    spared = true;
                    let _ = cx.drop_image(c.image, Some(window));
                }
            }
        }
    }

    fn save_position(&self) {
        if self.geo.tops.is_empty() {
            return;
        }
        let page = self.page_at(-self.offset().1 + 1.);
        let (_, top, _, h) = self.page_rect(page);
        let position = PdfPosition { page, fraction: (-self.offset().1 - top) / h.max(1.), scale: if let Fit::Scale(s) = self.fit { Some(s) } else { None }, fit_page: self.fit == Fit::Page, used: chrono::Utc::now().timestamp() };
        let key = self.path.to_string_lossy().to_string();
        prefs::update(|p| {
            p.pdf_positions.insert(key, position);
            while p.pdf_positions.len() > 200 {
                let Some(oldest) = p.pdf_positions.iter().min_by_key(|(_, v)| v.used).map(|(k, _)| k.clone()) else { break };
                p.pdf_positions.remove(&oldest);
            }
        });
    }

    fn restore_position(&mut self) {
        self.restored = true;
        let Some(position) = prefs::get().pdf_positions.get(self.path.to_string_lossy().as_ref()).cloned() else { return };
        self.fit = match (position.scale, position.fit_page) {
            (Some(s), _) => Fit::Scale(s.clamp(MIN_SCALE, MAX_SCALE)),
            (None, true) => Fit::Page,
            _ => Fit::Width,
        };
        self.pending = Some(Anchor { page: position.page, x: 0., y: position.fraction, view_x: 0., view_y: 0. });
    }

    fn schedule_after_scroll(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let _ = this.update(cx, |v, cx| {
                v.save_position();
                cx.notify();
            });
        }));
    }

    fn refresh_after(&mut self, delay: Duration, cx: &mut Context<Self>) {
        self._timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
    }

    fn set_fit(&mut self, fit: Fit, point: Option<(f32, f32)>, cx: &mut Context<Self>) {
        self.fit = fit;
        self.zoom_point = point;
        self.hold_sharp(cx);
        cx.notify();
    }

    /// Waits with sharp renders until the scale has stopped changing, so zooming, resizing and the panel animation
    /// show the pages already rendered instead of rendering every step.
    fn hold_sharp(&mut self, cx: &mut Context<Self>) {
        self.sharp_after = Instant::now() + SHARP_SETTLE;
        self.settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHARP_SETTLE + Duration::from_millis(10)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
    }

    fn zoom_by(&mut self, factor: f32, point: Option<(f32, f32)>, cx: &mut Context<Self>) {
        let next = (self.geo.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        self.set_fit(Fit::Scale(next), point, cx);
    }

    fn to_view(&self, position: Point<Pixels>) -> (f32, f32) {
        (f32::from(position.x - self.view_bounds.origin.x), f32::from(position.y - self.view_bounds.origin.y))
    }

    fn jump_to(&mut self, page: usize, cx: &mut Context<Self>) {
        self.pending = Some(Anchor { page, x: 0., y: 0., view_x: 0., view_y: PADDING });
        self.apply_pending();
        cx.notify();
    }

    fn apply_pending(&mut self) {
        if !self.geo.tops.is_empty()
            && let Some(a) = self.pending.take()
        {
            self.apply_anchor(a);
        }
    }

    fn follow_link(&mut self, target: LinkTarget, cx: &mut Context<Self>) {
        match target {
            LinkTarget::Uri(uri) => {
                crate::core::library::open_path(uri);
            }
            LinkTarget::Page(page) => {
                if let Some(here) = self.anchor_at(0., 0.) {
                    self.history.push(here);
                }
                self.jump_to(page, cx);
            }
        }
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(a) = self.history.pop() {
            self.apply_anchor(a);
            cx.notify();
        }
    }

    fn glide_by(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) {
        let (x, y) = self.offset();
        let target = self.clamped(x + dx, y + dy);
        let start = self.glide.map_or(point(px(x), px(y)), |(_, _, to)| to);
        if cx.reduce_motion() {
            self.scroll.set_offset(target);
        } else {
            let target = self.clamped(f32::from(start.x) + dx, f32::from(start.y) + dy);
            self.glide = Some((Instant::now(), self.scroll.offset(), target));
        }
        cx.notify();
    }

    fn jump_to_edge(&mut self, end: bool, cx: &mut Context<Self>) {
        let (x, _) = self.offset();
        self.glide = None;
        self.scroll.set_offset(self.clamped(x, if end { f32::MIN / 2. } else { 0. }));
        cx.notify();
    }

    fn locate(&self, position: Point<Pixels>) -> Option<(usize, f32, f32)> {
        if self.geo.tops.is_empty() {
            return None;
        }
        let (vx, vy) = self.to_view(position);
        let (ox, oy) = self.offset();
        let page = self.page_at(vy - oy);
        let (left, top, ..) = self.page_rect(page);
        Some((page, (vx - ox - left) / self.geo.scale, (vy - oy - top) / self.geo.scale))
    }

    fn link_at(&self, page: usize, x: f32, y: f32) -> Option<&LinkTarget> {
        self.meta.links.get(&page)?.iter().find(|l| l.rect.contains(x, y)).map(|l| &l.target)
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.active_mark = None;
        self.selected.clear();
        let Some((page, x, y)) = self.locate(e.position) else {
            cx.notify();
            return;
        };
        if let Some(target) = self.link_at(page, x, y).cloned() {
            self.follow_link(target, cx);
            return;
        }
        self.selection = None;
        self.active_mark = self.marks.iter().rev().find(|h| h.covers(page, x, y)).map(|h| h.id);
        if let Some((text, _)) = self.texts.get(&page) {
            let caret = text.caret_at(x, y);
            if e.click_count == 2 {
                let (start, end) = text.word_at(caret.min(text.glyph_count().saturating_sub(1)));
                self.selection = Some(((page, start), (page, end)));
            } else if text.on_text(x, y) {
                self.selection = Some(((page, caret), (page, caret)));
                self.selecting = true;
            }
        }
        cx.notify();
    }

    fn on_mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.last_mouse = Some(self.to_view(e.position));
        let Some((page, x, y)) = self.locate(e.position) else { return };
        if self.selecting {
            if let (Some((start, _)), Some((text, _))) = (self.selection, self.texts.get(&page)) {
                self.selection = Some((start, (page, text.caret_at(x, y))));
                cx.notify();
            }
            let (_, vy) = self.to_view(e.position);
            let edge = if vy < 24. { 24. } else if vy > self.geo.view.1 - 24. { -24. } else { 0. };
            if edge != 0. {
                let (ox, oy) = self.offset();
                self.scroll.set_offset(self.clamped(ox, oy + edge));
            }
            return;
        }
        let cursor = if self.link_at(page, x, y).is_some() {
            CursorStyle::PointingHand
        } else if self.texts.get(&page).is_some_and(|(t, _)| t.on_text(x, y)) {
            CursorStyle::IBeam
        } else {
            CursorStyle::Arrow
        };
        if cursor != self.hover_cursor {
            self.hover_cursor = cursor;
            cx.notify();
        }
    }

    /// The current selection as highlight parts per page, with its text.
    fn selected_parts(&self) -> Option<(Vec<HighlightPart>, String)> {
        let (from, to) = self.ordered_selection()?;
        let (mut parts, mut text) = (Vec::new(), String::new());
        for page in from.0..=to.0 {
            let Some((page_text, _)) = self.texts.get(&page) else { continue };
            let start = if page == from.0 { from.1 } else { 0 };
            let end = if page == to.0 { to.1 } else { page_text.glyph_count() };
            let rects: Vec<[f32; 4]> = page_text.rects(start, end).into_iter().map(|r| [r.x, r.y, r.w, r.h]).collect();
            if rects.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(page_text.slice(start, end).trim());
            parts.push(HighlightPart { page, rects });
        }
        (!parts.is_empty()).then_some((parts, text))
    }

    fn add_mark(&mut self, color: &'static str, cx: &mut Context<Self>) {
        let (Some(key), Some((parts, text))) = (self.marks_key.clone(), self.selected_parts()) else { return };
        self.selection = None;
        self.last_color = color;
        if let Some(same) = self.marks.iter().find(|h| h.pin.is_none() && h.ink.is_empty() && h.text == text && h.parts == parts) {
            let (id, recolor) = (same.id, same.color != color);
            self.active_mark = Some(id);
            if recolor {
                self.recolor_mark(id, color, cx);
            }
            cx.notify();
            return;
        }
        let mut list = self.marks.clone();
        let id = super::state::read(cx, |l| l.next_mark_id(&key, true));
        self.active_mark = Some(id);
        self.removed_mark = None;
        list.push(Highlight { id, color: color.to_string(), text, parts, ..Default::default() });
        super::state::act(cx, |l| l.set_highlights(&key, list, tr("Markiert")));
    }

    fn recolor_mark(&mut self, id: u64, color: &'static str, cx: &mut Context<Self>) {
        let Some(key) = self.marks_key.clone() else { return };
        self.last_color = color;
        let mut list = self.marks.clone();
        if let Some(h) = list.iter_mut().find(|h| h.id == id) {
            h.color = color.to_string();
        }
        super::state::act(cx, |l| l.set_highlights(&key, list, tr("Farbe geändert")));
    }

    fn remove_mark(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(key) = self.marks_key.clone() else { return };
        self.active_mark = None;
        self.removed_mark = Some(id);
        let list = self.marks.iter().filter(|h| h.id != id).cloned().collect();
        super::state::act(cx, |l| l.set_highlights(&key, list, tr("Markierung entfernt")));
    }

    fn reveal_mark(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some((page, r)) = self.marks.iter().find(|h| h.id == id).and_then(|h| h.start()) else { return };
        self.active_mark = Some(id);
        if !self.geo.tops.is_empty() && page < self.sizes.len() {
            let (_, _, _, h) = self.page_rect(page);
            let (ox, _) = self.offset();
            self.pending = Some(Anchor { page, x: 0., y: r[1] * self.geo.scale / h.max(1.), view_x: ox + (self.geo.content_width - self.sizes[page].0 * self.geo.scale) / 2., view_y: self.geo.view.1 * 0.3 });
            self.apply_pending();
        }
        cx.notify();
    }

    /// The small bar under a selection or a clicked highlight: colors, plus copying or removing.
    fn mark_bar(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.selecting || self.geo.tops.is_empty() {
            return None;
        }
        let selection = self.selected_parts();
        let editing = self.active_mark.and_then(|id| self.marks.iter().find(|h| h.id == id)).filter(|_| selection.is_none());
        let part = selection.as_ref().and_then(|(parts, _)| parts.last()).or_else(|| editing.and_then(|h| h.parts.last()))?;
        let r = *part.rects.last()?;
        let (left, top, _, _) = self.page_rect(part.page);
        let scale = self.geo.scale;
        let (ox, oy) = self.offset();
        let (x_min, x_max) = (-ox + space::XS, -ox + self.geo.view.0 - space::L);
        let covered = self.floating() && self.panel_open;
        let beside_panel = x_min + PANEL_WIDTH;
        if covered && x_max - beside_panel < MARK_BAR_WIDTH {
            return None;
        }
        let right = (x_max - MARK_BAR_WIDTH).max(x_min);
        let (y_min, y_max) = (-oy + space::XS, -oy + self.geo.view.1 - space::XS);
        let x = (left + r[0] * scale).min(right).max(if covered { beside_panel } else { x_min });
        let below = top + (r[1] + r[3]) * scale + space::XS;
        let above = top + r[1] * scale - control::M - space::XS;
        let y = if below + control::M <= y_max { below } else { above }.min(y_max - control::M).max(y_min);
        let me = cx.entity();
        let current = editing.map(|h| h.color.clone());
        let editing_id = editing.map(|h| h.id);
        let action: Stateful<Div> = match editing_id {
            Some(id) => {
                let e = me.clone();
                bar_action("pdf-mark-remove", "trash-2", tr("Markierung entfernen"), t).on_click(move |_, window, cx| {
                    e.update(cx, |v, cx| {
                        v.remove_mark(id, cx);
                        window.focus(&v.focus, cx);
                    })
                })
            }
            None => {
                let e = me.clone();
                bar_action("pdf-mark-copy", "copy", tr("Kopieren"), t).on_click(move |_, _, cx| e.read(cx).copy_selection())
            }
        }
        .on_key_down(|ev, window, cx| match ev.keystroke.key.as_str() {
            "left" => {
                window.focus_prev(cx);
                cx.stop_propagation();
            }
            "space" => cx.stop_propagation(),
            _ => {}
        });
        Some(
            div()
                .id("pdf-mark-bar")
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(MARK_BAR_WIDTH))
                .h(px(control::M))
                .flex()
                .items_center()
                .justify_between()
                .px(px(space::S))
                .rounded(px(radius::CONTROL))
                .bg(t.material)
                .border_1()
                .border_color(t.separator)
                .shadow_md()
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .children(MARK_COLORS.into_iter().enumerate().map(|(i, color)| {
                    let e = me.clone();
                    let selected = current.as_deref() == Some(color);
                    let label = match editing_id {
                        Some(_) => color_name(color),
                        None if color == self.last_color => format!("{}{}", trf("%@ markieren", &[&color_name(color)]), ShortcutAction::MarkText.hint()),
                        None => trf("%@ markieren", &[&color_name(color)]),
                    };
                    div()
                        .id(SharedString::from(format!("pdf-mark-{color}")))
                        .size(px(control::S))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .cursor_pointer()
                        .key_nav()
                        .soft(t.ink.opacity(0.08), control::S / 2.)
                        .when(selected, |d| d.border_2().border_color(t.primary))
                        .tooltip(tip(label))
                        .on_key_down(move |ev, window, cx| match ev.keystroke.key.as_str() {
                            "left" => {
                                if i > 0 {
                                    window.focus_prev(cx);
                                }
                                cx.stop_propagation();
                            }
                            "right" => {
                                window.focus_next(cx);
                                cx.stop_propagation();
                            }
                            "space" => cx.stop_propagation(),
                            _ => {}
                        })
                        .on_click(move |_, _, cx| {
                            pause_tooltips(cx);
                            e.update(cx, |v, cx| match editing_id {
                                Some(id) => v.recolor_mark(id, color, cx),
                                None => v.add_mark(color, cx),
                            })
                        })
                        .child(div().size(px(iconsize::M)).rounded_full().bg(t.palette(color)))
                }))
                .child(action)
                .into_any_element(),
        )
    }

    fn marks_panel(&self, cx: &mut Context<Self>, t: &Theme) -> AnyElement {
        if self.marks.is_empty() {
            return div().p(px(space::L)).text_size(px(font::CALLOUT)).text_color(t.secondary).child(tr("Text auswählen und H drücken, um ihn zu markieren.")).into_any_element();
        }
        let mut marks: Vec<&Highlight> = self.marks.iter().collect();
        marks.sort_by(|a, b| {
            let (x, y) = (a.start(), b.start());
            x.map(|s| s.0).cmp(&y.map(|s| s.0)).then(x.map_or(0., |s| s.1[1]).total_cmp(&y.map_or(0., |s| s.1[1])))
        });
        let me = cx.entity();
        let rows = marks.into_iter().map(|h| {
            let (id, jump, remove) = (h.id, me.clone(), me.clone());
            let page = h.start().map_or(0, |s| s.0);
            let active = self.active_mark == Some(id);
            div()
                .id(("pdf-mark-row", id as usize))
                .group("pdf-mark-row")
                .relative()
                .flex()
                .gap(px(space::S))
                .p(px(space::S))
                .rounded(px(radius::INNER))
                .cursor_pointer()
                .when(active, |d| d.bg(t.ink.opacity(0.08)))
                .soft(t.ink.opacity(0.05), radius::INNER)
                .on_click(move |_, _, cx| jump.update(cx, |v, cx| v.reveal_mark(id, cx)))
                .child(div().flex_none().w(px(space::XS)).rounded_full().bg(t.palette(&h.color)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(space::XXS))
                        .child(div().text_size(px(font::CAPTION)).text_color(t.secondary).child(page_word(self.slides, page + 1)))
                        .child(div().text_size(px(font::CALLOUT)).line_clamp(3).child(h.text.clone())),
                )
                .child(
                    div()
                        .id(("pdf-mark-delete", id as usize))
                        .absolute()
                        .top(px(space::XS))
                        .right(px(space::XS))
                        .rounded(px(radius::SMALL))
                        .cursor_pointer()
                        .soft(t.ink.opacity(0.08), radius::SMALL)
                        .invisible()
                        .group_hover("pdf-mark-row", |s| s.visible())
                        .tooltip(tip(tr("Markierung entfernen")))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            remove.update(cx, |v, cx| v.remove_mark(id, cx));
                        })
                        .child(icon("x", iconsize::S, t.secondary)),
                )
        });
        div().id("pdf-marks").size_full().overflow_y_scrollbar().child(div().flex().flex_col().gap(px(space::XS)).p(px(space::S)).children(rows.collect::<Vec<_>>())).into_any_element()
    }

    fn ordered_selection(&self) -> Option<(Caret, Caret)> {
        let (a, b) = self.selection?;
        if a == b { None } else { Some(if a <= b { (a, b) } else { (b, a) }) }
    }

    fn copy_selection(&self) {
        if let Some((from, to)) = self.ordered_selection() {
            let _ = self.jobs.try_send(TextJob::Copy { from, to });
        }
    }

    fn select_page(&mut self, cx: &mut Context<Self>) {
        let page = self.current_page();
        if let Some((text, _)) = self.texts.get(&page) {
            self.selection = Some(((page, 0), (page, text.glyph_count())));
            cx.notify();
        }
    }

    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.panel_open = true;
        self.panel_tab = PanelTab::Search;
        self.search_active = true;
        self.ensure_inputs(window, cx);
        if let Some(input) = self.search_input.clone() {
            input.update(cx, |s, cx| s.focus(window, cx));
            let query = input.read(cx).value().trim().to_string();
            if !query.is_empty() && self.hits.is_empty() {
                self.start_search(query, cx);
            }
        }
        cx.notify();
    }

    /// Opens the search panel with a query, as when arriving from the file search.
    pub fn search_for(&mut self, query: &str, nth: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search(window, cx);
        self.wanted_hit = Some((query.to_string(), nth));
        if let Some(input) = self.search_input.clone() {
            input.update(cx, |s, cx| s.set_value(query.to_string(), window, cx));
        }
        self.start_search(query.to_string(), cx);
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_generation.fetch_add(1, Ordering::Relaxed);
        self.hits.clear();
        self.current_hit = None;
        self.search_active = false;
        self.panel_open = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn start_search(&mut self, query: String, cx: &mut Context<Self>) {
        let generation = self.search_generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.hits.clear();
        self.current_hit = None;
        self.search_progress = 0;
        self.wanted_hit.take_if(|(wanted, _)| *wanted != query);
        if !query.trim().is_empty() {
            let _ = self.jobs.try_send(TextJob::Search { generation, query });
        }
        cx.notify();
    }

    fn first_hit_from_current(&self) -> usize {
        let page = self.current_page();
        self.hits.iter().position(|h| h.page >= page).unwrap_or(0)
    }

    fn select_hit(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.hits.len() {
            return;
        }
        self.current_hit = Some(index);
        self.hit_list.scroll_to_item(index, ScrollStrategy::Center);
        self.hit_to_reveal = Some(index);
        cx.notify();
    }

    /// Scrolls a match into view; runs from render, because a freshly opened PDF has no final layout before that.
    fn reveal_hit(&mut self, index: usize) {
        let Some(hit) = self.hits.get(index) else { return };
        let (page, rect) = (hit.page, hit.rects.first().copied());
        if let Some(r) = rect
            && !self.geo.tops.is_empty()
        {
            let (_, top, _, h) = self.page_rect(page);
            let y = top + r.y * self.geo.scale;
            let (ox, oy) = self.offset();
            if y + oy < 0. || y + oy + r.h * self.geo.scale > self.geo.view.1 {
                self.pending = Some(Anchor { page, x: 0., y: (y - top) / h.max(1.), view_x: ox + (self.geo.content_width - self.sizes[page].0 * self.geo.scale) / 2., view_y: self.geo.view.1 * 0.3 });
                self.apply_pending();
            }
        }
    }

    fn step_hit(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.hits.is_empty() {
            return;
        }
        let count = self.hits.len() as isize;
        let next = self.current_hit.map_or(self.first_hit_from_current() as isize, |c| (c as isize + by).rem_euclid(count));
        self.select_hit(next as usize, cx);
    }

    fn ensure_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_input.is_some() {
            return;
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("In diesem PDF suchen")));
        let page = cx.new(|cx| InputState::new(window, cx));
        let zoom = cx.new(|cx| InputState::new(window, cx));
        self._subs.push(cx.subscribe_in(&search, window, |this, s, e: &InputEvent, window, cx| match e {
            InputEvent::Change => {
                let q = s.read(cx).value().to_string();
                this.start_search(q, cx);
            }
            InputEvent::PressEnter { shift, .. } => {
                let _ = window;
                this.step_hit(if *shift { -1 } else { 1 }, cx);
            }
            _ => {}
        }));
        self._subs.push(cx.subscribe_in(&page, window, |this, s, e: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = e {
                if let Ok(n) = s.read(cx).value().trim().parse::<usize>() {
                    let last = this.page_count().saturating_sub(1);
                    if let Some(here) = this.anchor_at(0., 0.) {
                        this.history.push(here);
                    }
                    this.jump_to(n.saturating_sub(1).min(last), cx);
                }
                this.editing_page = false;
                window.focus(&this.focus, cx);
                cx.notify();
            }
        }));
        self._subs.push(cx.subscribe_in(&zoom, window, |this, s, e: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = e {
                if let Ok(percent) = s.read(cx).value().trim().trim_end_matches('%').trim().parse::<f32>() {
                    this.set_fit(Fit::Scale((percent / 100.).clamp(MIN_SCALE, MAX_SCALE)), None, cx);
                }
                this.editing_zoom = false;
                window.focus(&this.focus, cx);
                cx.notify();
            }
        }));
        self.search_input = Some(search);
        self.page_input = Some(page);
        self.zoom_input = Some(zoom);
    }

    fn start_editing(&mut self, zoom: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_inputs(window, cx);
        let (input, value) = if zoom {
            self.editing_zoom = true;
            (self.zoom_input.clone(), format!("{}", (self.geo.scale * 100.).round()))
        } else {
            self.editing_page = true;
            (self.page_input.clone(), (self.current_page() + 1).to_string())
        };
        if let Some(input) = input {
            input.update(cx, |s, cx| {
                s.set_value("", window, cx);
                s.set_placeholder(value, window, cx);
                s.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn floating(&self) -> bool {
        self.bar.bar().is_some_and(|w| w < FLOAT_BELOW)
    }

    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.page_notice.take().is_some() {
            self.notice_task = None;
        } else if cx.has_active_drag() {
            cx.stop_active_drag(window);
            self.drop_at = None;
            self.drag_step = 0.;
            self.dragging.clear();
        } else if self.editing_page || self.editing_zoom {
            self.editing_page = false;
            self.editing_zoom = false;
            window.focus(&self.focus, cx);
        } else if self.search_active {
            self.close_search(window, cx);
        } else if self.panel_open && self.floating() {
            self.panel_open = false;
            window.focus(&self.focus, cx);
        } else if self.active_mark.take().is_none() && self.selection.take().is_none() && std::mem::take(&mut self.selected).is_empty() {
            cx.propagate();
        }
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let m = &k.modifiers;
        let command = if cfg!(target_os = "macos") { m.platform } else { m.control };
        let typing = in_field(window);
        let key = k.key.as_str();
        let panel = self.pages_focus.contains_focused(window, cx) && self.editable();
        let page_step = (self.geo.view.1 - LINE_STEP).max(LINE_STEP);
        let page_command = PAGE_COMMANDS.into_iter().find(|a| is_shortcut(*a, k)).filter(|a| !(is_plain(*a) && self.edit_block().is_some()));
        match key {
            "f" if command && !m.shift => self.open_search(window, cx),
            "g" if command => self.step_hit(if m.shift { -1 } else { 1 }, cx),
            _ if typing => return,
            _ if is_mark_key(k) && self.ordered_selection().is_some() => self.add_mark(self.last_color, cx),
            _ if page_command.is_some() => {
                if let Some(action) = page_command.filter(|_| !e.is_held) {
                    self.run_page_command(action, cx);
                }
            }
            "f10" if panel && m.shift => self.open_row_menu(window, cx),
            "menu" if panel => self.open_row_menu(window, cx),
            "up" | "down" if panel && !command && !m.alt => self.step_page(if key == "up" { -1 } else { 1 }, m.shift, cx),
            "backspace" | "delete" if panel && !self.selected.is_empty() => self.delete_pages(None, cx),
            "backspace" | "delete" if self.active_mark.is_some() => {
                if let Some(id) = self.active_mark {
                    self.remove_mark(id, cx);
                }
            }
            "=" | "+" if command => self.zoom_by(1.25, self.last_mouse, cx),
            "-" if command => self.zoom_by(0.8, self.last_mouse, cx),
            "0" if command => self.set_fit(Fit::Width, None, cx),
            "pagedown" => self.glide_by(0., -page_step, cx),
            "pageup" => self.glide_by(0., page_step, cx),
            "space" => self.glide_by(0., if m.shift { page_step } else { -page_step }, cx),
            "down" if command => self.jump_to_edge(true, cx),
            "up" if command => self.jump_to_edge(false, cx),
            "down" => self.glide_by(0., -LINE_STEP, cx),
            "up" => self.glide_by(0., LINE_STEP, cx),
            "left" if m.alt || command => self.go_back(cx),
            "left" => self.glide_by(LINE_STEP, 0., cx),
            "right" => self.glide_by(-LINE_STEP, 0., cx),
            "home" => self.jump_to_edge(false, cx),
            "end" => self.jump_to_edge(true, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn run_page_command(&mut self, action: ShortcutAction, cx: &mut Context<Self>) {
        match action {
            ShortcutAction::RotatePagesLeft => self.rotate_pages(None, false, cx),
            ShortcutAction::RotatePagesRight => self.rotate_pages(None, true, cx),
            ShortcutAction::MovePagesUp => self.move_pages(None, false, cx),
            ShortcutAction::MovePagesDown => self.move_pages(None, true, cx),
            ShortcutAction::DeletePages => self.delete_pages(None, cx),
            _ => self.choose_pdf(None, cx),
        }
    }

    fn page_element(&self, page: usize, window: &Window, t: &Theme) -> AnyElement {
        let (left, top, w, h) = self.page_rect(page);
        let scale = self.geo.scale;
        let shown = self.displayed_key(page, window).and_then(|k| self.cache.get(&k));
        let image = shown.map(|c| c.image.clone());
        let mark = |r: PageRect, color: Hsla| div().absolute().left(px(r.x * scale)).top(px(r.y * scale)).w(px(r.w * scale)).h(px(r.h * scale)).bg(color);
        let mut el = div().absolute().left(px(left)).top(px(top)).w(px(w)).h(px(h)).bg(white()).shadow_md().children(image.map(|i| img(i).size_full()));
        let (yellow, orange) = (t.palette("yellow").opacity(0.45), t.orange.opacity(0.6));
        for m in self.page_marks.get(&page).into_iter().flatten().filter(|m| !shown.is_some_and(|c| c.baked_sigs.contains(&m.sig))) {
            let color = Theme::document(u32::from_be_bytes([0, m.color[0], m.color[1], m.color[2]])).opacity(MARK_OPACITY);
            for r in &m.rects {
                el = el.child(mark(PageRect { x: r[0], y: r[1], w: r[2], h: r[3] }, color));
            }
        }
        if let Some(h) = self.active_mark.and_then(|id| self.marks.iter().find(|h| h.id == id))
            && let Some(line) = self.underlines.get(&h.id).copied()
        {
            for r in h.parts.iter().filter(|p| p.page == page).flat_map(|p| &p.rects).filter(|r| r.iter().all(|v| v.is_finite())) {
                el = el.child(div().absolute().left(px(r[0] * scale)).top(px((r[1] + r[3]) * scale - MARK_UNDERLINE)).w(px(r[2] * scale)).h(px(MARK_UNDERLINE)).bg(line));
            }
        }
        for (i, hit) in self.hits.iter().enumerate().filter(|(_, h)| h.page == page) {
            let color = if self.current_hit == Some(i) { orange } else { yellow };
            for r in &hit.rects {
                el = el.child(mark(*r, color));
            }
        }
        if let (Some((from, to)), Some((text, _))) = (self.ordered_selection(), self.texts.get(&page))
            && from.0 <= page
            && page <= to.0
        {
            let start = if page == from.0 { from.1 } else { 0 };
            let end = if page == to.0 { to.1 } else { text.glyph_count() };
            for r in text.rects(start, end) {
                el = el.child(mark(r, t.accent.opacity(0.3)));
            }
        }
        el.into_any_element()
    }

    fn zoom_label(&self) -> String {
        format!("{} %", (self.geo.scale * 100.).round())
    }

    fn toolbar(&self, window: &Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        let me = cx.entity();
        let count = self.page_count();
        let slides = self.slides;
        let current = self.current_page() + 1;
        let (m1, m2, m3, m4, m5, m6, m7) = (me.clone(), me.clone(), me.clone(), me.clone(), me.clone(), me.clone(), me);
        let page_label = if self.slides { trf("Folie %lld von %lld", &[&current, &count]) } else { trf("Seite %lld von %lld", &[&current, &count]) };
        let text = |s: &str| super::bar_fit::text_width(s, font::CALLOUT, FontWeight::NORMAL, window);
        let back_w = if self.history.is_empty() { 0. } else { control::S };
        let fields = [text(&page_label) + 2. * space::S, control::S, (space::XXL + space::L).max(text(&self.zoom_label())), control::S, control::S];
        let spacers = 2. * space::S;
        let needed = super::bar_fit::row(&[&[self.navigator_width(), back_w][..], &fields[..]].concat(), space::S) + spacers + 2. * space::S;
        let compact = self.bar.bar().is_some_and(|w| w < needed);
        let pager: AnyElement = match (&self.page_input, self.editing_page) {
            (Some(input), true) => div().w(px(space::XXL * 2.)).child(Input::new(input).small()).into_any_element(),
            _ => div()
                .id("pdf-page-label")
                .px(px(space::S))
                .py(px(space::XXS))
                .rounded(px(radius::INNER))
                .cursor_pointer()
                .key_nav()
                .soft(t.ink.opacity(0.06), radius::INNER)
                .font_features(tabular())
                .text_size(px(font::CALLOUT))
                .tooltip(tip(if self.slides { tr("Zu Folie springen") } else { tr("Zu Seite springen") }))
                .child(page_label)
                .on_click(move |_, window, cx| m1.update(cx, |v, cx| v.start_editing(false, window, cx)))
                .into_any_element(),
        };
        let zoom_field: AnyElement = match (&self.zoom_input, self.editing_zoom) {
            (Some(input), true) => div().w(px(space::XXL * 2.)).child(Input::new(input).small()).into_any_element(),
            _ => div()
                .id("pdf-zoom-label")
                .min_w(px(space::XXL + space::L))
                .flex()
                .justify_center()
                .py(px(space::XXS))
                .rounded(px(radius::INNER))
                .cursor_pointer()
                .key_nav()
                .soft(t.ink.opacity(0.06), radius::INNER)
                .font_features(tabular())
                .text_size(px(font::CALLOUT))
                .tooltip(tip(tr("Zoom eingeben")))
                .child(self.zoom_label())
                .on_click(move |_, window, cx| m2.update(cx, |v, cx| v.start_editing(true, window, cx)))
                .into_any_element(),
        };
        div()
            .h(px(control::SUBBAR))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::S))
            .border_b_1()
            .border_color(t.separator)
            .child(self.bar.measure_bar())
            .child(if compact { self.navigator_menu(m3).into_any_element() } else { self.navigator(m3, t).into_any_element() })
            .when(!self.history.is_empty(), |d| {
                let back = m4.clone();
                d.child(Button::new("pdf-back").ghost().small().icon(menu_icon("arrow-left")).tooltip(tr("Zurück zur vorherigen Stelle")).on_click(move |_, _, cx| back.update(cx, |v, cx| v.go_back(cx))))
            })
            .child(div().flex_1())
            .child(pager)
            .child(div().flex_1())
            .child(Button::new("pdf-out").ghost().small().icon(menu_icon("zoom-out")).tooltip(tr("Verkleinern")).on_click(move |_, _, cx| m5.update(cx, |v, cx| v.zoom_by(0.8, None, cx))))
            .child(zoom_field)
            .child(Button::new("pdf-in").ghost().small().icon(menu_icon("zoom-in")).tooltip(tr("Vergrößern")).on_click(move |_, _, cx| m6.update(cx, |v, cx| v.zoom_by(1.25, None, cx))))
            .child(Button::new("pdf-fit").ghost().small().icon(menu_icon("chevron-down")).tooltip(tr("Einpassen")).dropdown_menu(move |menu, _, _| {
                let entry = |label: String, fit: Fit, view: Entity<PdfView>| PopupMenuItem::new(label).on_click(move |_, _, cx| view.update(cx, |v, cx| v.set_fit(fit, None, cx)));
                let v = m7.clone();
                menu.item(entry(tr("Breite einpassen"), Fit::Width, v.clone())).item(entry(if slides { tr("Folie einpassen") } else { tr("Seite einpassen") }, Fit::Page, v.clone())).separator().item(entry("50 %".into(), Fit::Scale(0.5), v.clone())).item(entry("100 %".into(), Fit::Scale(1.), v.clone())).item(entry("150 %".into(), Fit::Scale(1.5), v.clone())).item(entry("200 %".into(), Fit::Scale(2.), v))
            }))
    }

    fn navigator_picks(&self) -> Vec<(PanelTab, String, &'static str)> {
        [(PanelTab::Thumbnails, if self.slides { tr("Folien") } else { tr("Seiten") }, "gallery-vertical-end"), (PanelTab::Outline, tr("Inhalt"), "list-tree"), (PanelTab::Search, tr("Suche"), "search"), (PanelTab::Marks, tr("Marker"), "highlighter")]
            .into_iter()
            .filter(|(value, ..)| *value != PanelTab::Outline || !self.meta.outline.is_empty())
            .collect()
    }

    fn navigator_width(&self) -> f32 {
        2. * space::XXS + self.navigator_picks().len() as f32 * control::S
    }

    fn navigator_menu(&self, view: Entity<PdfView>) -> impl IntoElement + use<> {
        let open = self.panel_open;
        let shown = self.panel_tab;
        let picks = self.navigator_picks();
        let icon_name = picks.iter().find(|(v, ..)| open && *v == shown).map_or("gallery-vertical-end", |(_, _, i)| *i);
        Button::new("pdf-navigator").ghost().small().icon(menu_icon(icon_name)).selected(open).tooltip(tr("Seitenleiste")).dropdown_menu(move |mut menu, _, _| {
            for (value, label, icon_name) in picks.iter().cloned() {
                let v = view.clone();
                menu = menu.item(PopupMenuItem::new(label).icon(menu_icon(icon_name)).checked(open && value == shown).on_click(move |_, window, cx| v.update(cx, |v, cx| v.navigate(value, window, cx))));
            }
            menu
        })
    }

    fn navigator(&self, view: Entity<PdfView>, t: &Theme) -> Stateful<Div> {
        let mut row = div().id("pdf-navigator").flex().flex_none().rounded(px(radius::INNER)).bg(t.ink.opacity(0.06)).p(px(space::XXS));
        for (value, label, icon_name) in self.navigator_picks() {
            let shown = self.panel_open && self.panel_tab == value;
            let v = view.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("pdf-nav-{icon_name}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(control::S))
                    .h(px(control::S - 2. * space::XXS))
                    .rounded(px(radius::SMALL))
                    .cursor_pointer()
                    .key_nav()
                    .when(!shown, |d| d.soft(t.ink.opacity(0.06), radius::SMALL))
                    .when(shown, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm())
                    .child(icon(icon_name, iconsize::XS, if shown { t.primary } else { t.secondary }))
                    .tooltip(tip(if shown { trf("%@ ausblenden", &[&label]) } else { label }))
                    .on_click(move |_, window, cx| v.update(cx, |v, cx| v.navigate(value, window, cx))),
            );
        }
        row
    }

    fn navigate(&mut self, value: PanelTab, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.panel_open && self.panel_tab == value;
        match (value, shown) {
            (PanelTab::Search, true) => self.close_search(window, cx),
            (PanelTab::Search, false) => self.open_search(window, cx),
            (_, true) => self.panel_open = false,
            (_, false) => {
                if self.panel_tab == PanelTab::Search {
                    self.search_generation.fetch_add(1, Ordering::Relaxed);
                    self.hits.clear();
                    self.current_hit = None;
                    self.search_active = false;
                }
                self.panel_open = true;
                self.panel_tab = value;
            }
        }
        if self.panel_open && self.panel_tab == PanelTab::Thumbnails {
            self.selection = None;
            self.active_mark = None;
        }
        cx.notify();
    }

    fn panel(&mut self, width: f32, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        self.ensure_inputs(window, cx);
        let body: AnyElement = match self.panel_tab {
            PanelTab::Thumbnails => self.thumbnails(cx, t),
            PanelTab::Outline => self.outline(cx, t).into_any_element(),
            PanelTab::Search => self.search_panel(cx, t).into_any_element(),
            PanelTab::Marks => self.marks_panel(cx, t),
        };
        div()
            .w(px(width))
            .flex_none()
            .h_full()
            .overflow_hidden()
            .border_r_1()
            .border_color(t.separator)
            .child(
                div()
                    .w(px(PANEL_WIDTH))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(div().flex_1().min_h_0().child(body)),
            )
    }

    fn thumbnails(&mut self, cx: &mut Context<Self>, t: &Theme) -> AnyElement {
        let current = self.current_page();
        if self.followed_page != current {
            self.followed_page = current;
            self.thumbs.scroll_to_item(current, ScrollStrategy::Center);
        }
        let handle = self.thumbs.clone();
        let editable = self.editable();
        let dragging = cx.has_active_drag();
        if !dragging {
            self.drop_at = None;
            self.dragging.clear();
            self.drag_step = 0.;
        }
        let list = uniform_list(
            "pdf-thumbs",
            self.page_count(),
            cx.processor(move |this, range: Range<usize>, window, cx| {
                this.thumb_range = range.clone();
                let t = Theme::get(cx);
                let current = this.current_page();
                let me = cx.entity();
                let count = this.page_count();
                let editable = this.editable();
                let cursor = this.list_cursor.unwrap_or(current);
                let ring = window.last_input_was_keyboard() && this.pages_focus.contains_focused(window, cx);
                range
                    .map(|page| {
                        let (w, h) = this.sizes[page];
                        let fit = (130. / h.max(1.)).min(180. / w.max(1.));
                        let image = this.cache.get(&(page, PREVIEW)).map(|c| c.image.clone());
                        let selected = this.selected.contains(&page);
                        let (view, menu_view, press_view, drag_view) = (me.clone(), me.clone(), me.clone(), me.clone());
                        let moving = this.targets(Some(page));
                        let (up, down) = (moving.first().is_some_and(|p| *p > 0), moving.last().is_some_and(|p| p + 1 < count));
                        let labels = this.page_labels(Some(page));
                        let before = this.drop_at == Some(page);
                        let after = page + 1 == count && this.drop_at == Some(count);
                        let line = |d: Div| d.absolute().left(px(space::S)).right(px(space::S)).h(px(pages::DROP_LINE)).rounded(px(radius::FULL)).bg(t.accent);
                        let dragged: Vec<usize> = if selected { this.selected.iter().copied().collect() } else { vec![page] };
                        let edge = if ring && page == cursor {
                            t.primary
                        } else if selected {
                            t.accent_text.opacity(tint::SELECTED_EDGE)
                        } else {
                            transparent_black()
                        };
                        let row = div()
                            .id(("pdf-thumb", page))
                            .size_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(space::XS))
                            .rounded(px(radius::INNER))
                            .border_2()
                            .border_color(edge)
                            .cursor_pointer()
                            .when(selected, |d| d.bg(t.accent.opacity(tint::SELECTED)))
                            .when(this.dragging.contains(&page), |d| d.opacity(tint::DRAGGED))
                            .soft(if selected { t.accent.opacity(tint::SELECTED_HOVER) } else { t.ink.opacity(0.05) }, radius::INNER)
                            .child(div().w(px(w * fit)).h(px(h * fit)).bg(white()).shadow_sm().when(page == current, |d| d.border_2().border_color(t.accent)).children(image.map(|i| img(i).size_full())))
                            .child(
                                div()
                                    .text_size(px(font::CAPTION))
                                    .font_features(tabular())
                                    .font_weight(if selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                    .text_color(if selected || page == current { t.accent_text } else { t.secondary })
                                    .child((page + 1).to_string()),
                            )
                            .on_click(move |e, window, cx| {
                                let m = e.modifiers();
                                view.update(cx, |v, cx| v.click_page(page, m.platform || m.control, m.shift, window, cx))
                            })
                            .when(editable, |d| {
                                d.on_mouse_down(MouseButton::Right, move |_, _, cx| {
                                    press_view.update(cx, |v, cx| {
                                        v.list_cursor = Some(page);
                                        if !v.selected.contains(&page) {
                                            v.selected = BTreeSet::from([page]);
                                            v.select_anchor = Some(page);
                                        }
                                        cx.notify();
                                    })
                                })
                                .on_drag(DraggedPages { view: me.entity_id(), pages: dragged }, move |d, _, window, cx| {
                                    drag_view.update(cx, |v, cx| {
                                        v.dragging = d.pages.iter().copied().collect();
                                        window.focus(&v.pages_focus, cx);
                                        cx.notify();
                                    });
                                    cx.new(|_| d.clone())
                                })
                            });
                        let row = if editable { row.context_menu(move |menu, _, _| Self::thumb_menu(menu, page, menu_view.clone(), labels.clone(), up, down)).into_any_element() } else { row.into_any_element() };
                        div()
                            .id(("pdf-thumb-slot", page))
                            .relative()
                            .w_full()
                            .h(px(THUMB_ROW))
                            .px(px(space::XS))
                            .py(px(space::XXS))
                            .child(row)
                            .when(before, |d| d.child(line(div()).top_0()))
                            .when(after, |d| d.child(line(div()).bottom_0()))
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .track_scroll(&handle)
        .size_full();
        let id = cx.entity_id();
        let carrying = dragging && !self.dragging.is_empty();
        let body = div()
            .relative()
            .flex_1()
            .min_h_0()
            .when(carrying, |d| d.cursor(CursorStyle::ClosedHand))
            .child(list)
            .vertical_scrollbar(&handle)
            .child(
                canvas(
                    move |bounds, _, cx| {
                        cx.default_global::<PagePanels>().0.insert(id, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .when(editable, |d| {
                d.on_drag_move::<DraggedPages>(cx.listener(|v, e: &DragMoveEvent<DraggedPages>, _, cx| {
                    if e.drag(cx).view == cx.entity_id() {
                        v.hover_slot(e.event.position, e.bounds, cx);
                    }
                }))
                .on_drag_move::<ExternalPaths>(cx.listener(|v, e: &DragMoveEvent<ExternalPaths>, _, cx| v.hover_slot(e.event.position, e.bounds, cx)))
                .on_drag_move::<super::browser::DraggedFile>(cx.listener(|v, e: &DragMoveEvent<super::browser::DraggedFile>, _, cx| v.hover_slot(e.event.position, e.bounds, cx)))
                .on_drop(cx.listener(|v, d: &DraggedPages, _, cx| v.drop_pages(d, cx)))
                .on_drop(cx.listener(|v, paths: &ExternalPaths, _, cx| {
                    let at = v.drop_at.take().unwrap_or(v.page_count());
                    v.insert_pdfs(paths.paths().to_vec(), at, cx);
                }))
                .on_drop(cx.listener(|v, file: &super::browser::DraggedFile, _, cx| {
                    let at = v.drop_at.take().unwrap_or(v.page_count());
                    v.insert_pdfs(vec![file.0.clone()], at, cx);
                }))
            });
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.pages_focus)
            .key_context("PdfPages")
            .child(body)
            .children((editable || self.edit_block().is_some()).then(|| self.page_bar(cx, t)))
            .into_any_element()
    }

    fn outline(&mut self, cx: &mut Context<Self>, t: &Theme) -> AnyElement {
        if self.meta.outline.is_empty() {
            return unavailable("list", tr("Kein Inhaltsverzeichnis"), tr("Diese Datei enthält keine Lesezeichen."), t).into_any_element();
        }
        let count = self.meta.outline.len();
        uniform_list(
            "pdf-outline",
            count,
            cx.processor(|this, range: Range<usize>, _, cx| {
                let t = Theme::get(cx);
                let me = cx.entity();
                let current = this.current_page();
                let active = this.meta.outline.iter().rposition(|e| e.page <= current);
                range
                    .map(|i| {
                        let entry = &this.meta.outline[i];
                        let (page, view) = (entry.page, me.clone());
                        div()
                            .id(("pdf-outline-row", i))
                            .w_full()
                            .h(px(OUTLINE_ROW))
                            .flex()
                            .items_center()
                            .gap(px(space::S))
                            .pl(px(space::M + space::M * entry.level.saturating_sub(1) as f32))
                            .pr(px(space::M))
                            .cursor_pointer()
                            .key_nav()
                            .soft(t.ink.opacity(0.05), 0.)
                            .when(active == Some(i), |d| d.text_color(t.accent_text).font_weight(FontWeight::MEDIUM))
                            .child(div().flex_1().min_w_0().truncate().text_size(px(font::CALLOUT)).child(entry.title.clone()))
                            .child(div().text_size(px(font::CAPTION)).font_features(tabular()).text_color(t.secondary).child((page + 1).to_string()))
                            .on_click(move |_, _, cx| view.update(cx, |v, cx| {
                                if let Some(here) = v.anchor_at(0., 0.) {
                                    v.history.push(here);
                                }
                                v.jump_to(page, cx);
                            }))
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .size_full()
        .into_any_element()
    }

    fn search_panel(&mut self, cx: &mut Context<Self>, t: &Theme) -> Div {
        let slides = self.slides;
        let total = self.page_count();
        let running = self.search_progress < total && self.search_input.as_ref().is_some_and(|i| !i.read(cx).value().trim().is_empty());
        let summary = if self.hits.is_empty() {
            if running && slides { trf("Durchsuche Folie %lld von %lld", &[&self.search_progress, &total]) } else if running { trf("Durchsuche Seite %lld von %lld", &[&self.search_progress, &total]) } else if self.search_input.as_ref().is_some_and(|i| !i.read(cx).value().trim().is_empty()) { tr("Keine Treffer") } else { String::new() }
        } else {
            let at = self.current_hit.map_or(0, |c| c + 1);
            trf("%lld von %lld", &[&at, &self.hits.len()])
        };
        let (prev, next) = (cx.entity(), cx.entity());
        let handle = self.hit_list.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().px(px(space::S)).pt(px(space::S)).children(self.search_input.as_ref().map(|i| Input::new(i).small())))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .px(px(space::M))
                    .py(px(space::XS))
                    .child(div().flex_1().truncate().font_features(tabular()).text_size(px(font::CAPTION)).text_color(t.secondary).child(summary))
                    .when(running, |d| d.child(motion::spinner(iconsize::XS, t.secondary)))
                    .child(Button::new("pdf-hit-prev").ghost().small().icon(menu_icon("chevron-up")).tooltip(tr("Vorheriger Treffer")).on_click(move |_, _, cx| prev.update(cx, |v, cx| v.step_hit(-1, cx))))
                    .child(Button::new("pdf-hit-next").ghost().small().icon(menu_icon("chevron-down")).tooltip(tr("Nächster Treffer")).on_click(move |_, _, cx| next.update(cx, |v, cx| v.step_hit(1, cx)))),
            )
            .child(
                div().relative().flex_1().min_h_0().child(
                    uniform_list(
                        "pdf-hits",
                        self.hits.len(),
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            let t = Theme::get(cx);
                            let me = cx.entity();
                            range
                                .map(|i| {
                                    let hit = &this.hits[i];
                                    let view = me.clone();
                                    let selected = this.current_hit == Some(i);
                                    div()
                                        .id(("pdf-hit", i))
                                        .w_full()
                                        .h(px(HIT_ROW))
                                        .flex()
                                        .flex_col()
                                        .justify_center()
                                        .gap(px(space::XXS))
                                        .px(px(space::M))
                                        .cursor_pointer()
                                        .key_nav()
                                        .when(selected, |d| d.bg(t.accent.opacity(0.15)))
                                        .soft(t.ink.opacity(0.05), 0.)
                                        .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).text_color(t.secondary).child(page_word(slides, hit.page + 1)))
                                        .child(div().text_size(px(font::CALLOUT)).line_clamp(2).child(hit.excerpt.clone()))
                                        .on_click(move |_, _, cx| view.update(cx, |v, cx| v.select_hit(i, cx)))
                                        .into_any_element()
                                })
                                .collect()
                        }),
                    )
                    .track_scroll(&handle)
                    .size_full(),
                ).vertical_scrollbar(&handle),
            )
    }
}

/// The words of the commands for the pages they would act on right now.
#[derive(Clone)]
struct PageLabels {
    left: String,
    right: String,
    up: String,
    down: String,
    delete: String,
    insert: String,
}

impl PdfView {
    fn editable(&self) -> bool {
        matches!(self.source, pdf_worker::Source::Pdf(_)) && self.failed.is_none() && !self.sizes.is_empty()
    }

    fn busy(&self) -> bool {
        self.working.is_some() || self.reloading || self.marks_key.is_none()
    }

    /// Why page changes are not offered for this file; `None` when they are.
    fn edit_block(&self) -> Option<String> {
        matches!(self.source, pdf_worker::Source::Office(_)).then(|| tr("Seiten ordnen geht nur bei PDF-Dateien."))
    }

    /// The pages a command acts on: the selection, or the page it was asked on, or else the page shown.
    fn targets(&self, page: Option<usize>) -> Vec<usize> {
        let last = self.page_count().saturating_sub(1);
        match page {
            Some(p) if !self.selected.contains(&p) => vec![p.min(last)],
            _ if !self.selected.is_empty() => self.selected.iter().copied().filter(|p| *p <= last).collect(),
            _ => vec![self.current_page().min(last)],
        }
    }

    fn page_labels(&self, page: Option<usize>) -> PageLabels {
        let pages = self.targets(page);
        let n = pages.len();
        let after = pages.last().map_or(0, |p| p + 1);
        PageLabels {
            left: if n == 1 { tr("Seite links drehen") } else { trf("%lld Seiten links drehen", &[&n]) },
            right: if n == 1 { tr("Seite rechts drehen") } else { trf("%lld Seiten rechts drehen", &[&n]) },
            up: if n == 1 { tr("Seite nach oben") } else { trf("%lld Seiten nach oben", &[&n]) },
            down: if n == 1 { tr("Seite nach unten") } else { trf("%lld Seiten nach unten", &[&n]) },
            delete: if n == 1 { tr("Seite löschen") } else { trf("%lld Seiten löschen", &[&n]) },
            insert: trf("PDF nach Seite %lld einfügen …", &[&after]),
        }
    }

    fn show_notice(&mut self, title: String, body: String, cx: &mut Context<Self>) {
        self.page_notice = Some((title, body));
        self.notice_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTICE_FOR).await;
            let _ = this.update(cx, |v, cx| {
                v.page_notice = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn show_error(&mut self, error: EditError, cx: &mut Context<Self>) {
        let (title, body) = error.describe();
        self.show_notice(title, body, cx);
    }

    /// Whether a page command may start now; tells why when this file cannot be changed at all.
    fn ready(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(why) = self.edit_block() {
            self.show_notice(tr("Das geht hier nicht"), why, cx);
            return false;
        }
        self.editable() && !self.busy()
    }

    fn rotate_pages(&mut self, page: Option<usize>, clockwise: bool, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }
        let pages = self.targets(page);
        self.start_edit(Edit::Op(PageOp::Rotate { pages, clockwise }), cx);
    }

    fn delete_pages(&mut self, page: Option<usize>, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }
        let pages = self.targets(page);
        if pages.len() >= self.page_count() {
            self.show_error(EditError::Empty, cx);
            return;
        }
        self.start_edit(Edit::Op(PageOp::Delete(pages)), cx);
    }

    fn move_pages(&mut self, page: Option<usize>, down: bool, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }
        let pages = self.targets(page);
        let (Some(first), Some(last)) = (pages.first().copied(), pages.last().copied()) else { return };
        if (!down && first == 0) || (down && last + 1 >= self.page_count()) {
            return;
        }
        let to = if down { last + 2 - pages.len() } else { first - 1 };
        self.start_edit(Edit::Op(PageOp::Move { pages, to }), cx);
    }

    fn drop_pages(&mut self, dragged: &DraggedPages, cx: &mut Context<Self>) {
        self.drag_step = 0.;
        self.dragging.clear();
        let slot = self.drop_at.take();
        let (Some(slot), true) = (slot, dragged.view == cx.entity_id()) else {
            cx.notify();
            return;
        };
        let count = self.page_count();
        let mut pages = dragged.pages.clone();
        pages.retain(|p| *p < count);
        pages.sort_unstable();
        pages.dedup();
        if pages.is_empty() || !self.ready(cx) {
            return;
        }
        let to = slot - pages.iter().filter(|p| **p < slot).count();
        if pdf_edit::move_order(count, &pages, to).iter().enumerate().all(|(i, old)| i == *old) {
            cx.notify();
            return;
        }
        self.start_edit(Edit::Op(PageOp::Move { pages, to }), cx);
    }

    fn choose_pdf(&mut self, page: Option<usize>, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }
        let at = self.targets(page).last().map_or(0, |p| p + 1);
        let me = cx.entity().downgrade();
        super::platform::pick(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None }, cx, move |paths, cx| {
            let _ = me.update(cx, |v, cx| v.insert_pdfs(paths, at, cx));
        });
    }

    /// Inserts the PDFs one after another, in the order given, each as its own undo step.
    fn insert_pdfs(&mut self, paths: Vec<PathBuf>, at: usize, cx: &mut Context<Self>) {
        let pdfs: Vec<PathBuf> = paths.into_iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))).collect();
        if pdfs.is_empty() {
            self.show_notice(tr("Das ist keine PDF"), tr("Es lassen sich nur PDFs einfügen."), cx);
            return;
        }
        if !self.ready(cx) {
            return;
        }
        self.queued = pdfs;
        self.queue_at = at.min(self.page_count());
        self.run_queue(cx);
    }

    fn run_queue(&mut self, cx: &mut Context<Self>) {
        if self.queued.is_empty() || self.busy() {
            return;
        }
        let file = self.queued.remove(0);
        self.start_edit(Edit::Insert { at: self.queue_at, file }, cx);
        if self.working.is_none() {
            self.queued.clear();
        }
    }

    fn start_edit(&mut self, edit: Edit, cx: &mut Context<Self>) {
        let Some(old_key) = self.marks_key.clone() else { return };
        if !self.editable() || self.working.is_some() || self.reloading || !super::state::act(cx, |l| l.can_edit_pdf()) {
            return;
        }
        let (pages, label) = match &edit {
            Edit::Op(op) => (
                match op {
                    PageOp::Rotate { pages, .. } | PageOp::Move { pages, .. } | PageOp::Delete(pages) => pages.clone(),
                    PageOp::Insert { .. } => Vec::new(),
                },
                edit_label(op),
            ),
            Edit::Insert { .. } => (Vec::new(), tr("PDF eingefügt")),
        };
        let started = Started { sizes: self.sizes.clone(), pages, label, old_key: old_key.clone() };
        let path = self.path.clone();
        let root = super::state::read(cx, |l| l.root.clone());
        let task = cx.background_spawn(async move {
            let data = std::fs::read(&path).map_err(|_| Failure::Edit(EditError::Broken))?;
            let (op, inserted) = match edit {
                Edit::Op(op) => (op, 0),
                Edit::Insert { at, file } => {
                    let pdf = std::fs::read(&file).map_err(|_| Failure::Edit(EditError::Broken))?;
                    let inserted = PdfDoc::from_data(Arc::new(pdf.clone())).map_or(0, |d| d.page_count());
                    if inserted == 0 {
                        return Err(Failure::Edit(EditError::Broken));
                    }
                    (PageOp::Insert { at, pdf }, inserted)
                }
            };
            let bytes = pdf_edit::apply(&data, &op).map_err(Failure::Edit)?;
            let written = crate::core::library::write_edited_pdf(root, &path, &bytes, &old_key).map_err(Failure::Write)?;
            Ok(Edited { op, written, inserted })
        });
        self.working = Some(Instant::now());
        self.page_notice = None;
        self.working_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SPINNER_AFTER).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let slot = Rc::new(RefCell::new(Some((result, started))));
            let mine = slot.clone();
            let reached = this.update(cx, move |v, cx| {
                if let Some((result, started)) = mine.take() {
                    v.finish_edit(result, started, cx);
                }
            });
            if reached.is_err()
                && let Some((result, started)) = slot.take()
            {
                cx.update(|cx| settle_without_view(result, started, cx));
            }
        })
        .detach();
        cx.notify();
    }

    fn remember(&mut self, key: String, state: ViewState) {
        self.states.retain(|(k, _)| *k != key);
        self.states.push((key, state));
        if self.states.len() > KEPT_STATES {
            self.states.remove(0);
        }
    }

    fn finish_edit(&mut self, result: Result<Edited, Failure>, started: Started, cx: &mut Context<Self>) {
        self.working = None;
        self.working_task = None;
        let done = match result {
            Ok(done) => done,
            Err(Failure::Edit(e)) => {
                self.queued.clear();
                self.show_error(e, cx);
                return;
            }
            Err(Failure::Write(e)) => {
                self.queued.clear();
                super::state::act(cx, |l| l.pdf_edit_failed(e));
                cx.notify();
                return;
            }
        };
        let count = started.sizes.len();
        let before = self.anchor_at(0., 0.);
        let (select, page, jump) = follow_edit(&done.op, count, done.inserted, self.current_page(), &started.pages);
        let anchor = before.filter(|_| !jump).and_then(|a| new_index(&done.op, count, done.inserted, a.page).map(|page| Anchor { page, ..a }));
        let landing = anchor.unwrap_or(Anchor { page, x: 0., y: 0., view_x: 0., view_y: PADDING });
        self.remember(started.old_key.clone(), ViewState { selected: self.selected.clone(), anchor: before });
        self.remember(done.written.new_key().to_string(), ViewState { selected: select.iter().copied().collect(), anchor: Some(landing) });
        self.queue_at = if matches!(done.op, PageOp::Insert { .. }) { select.last().map_or(0, |p| p + 1) } else { 0 };
        self.after_edit = Some(AfterEdit { select, page, anchor });
        super::state::act(cx, |l| l.finish_pdf_edit(done.written, &done.op, &started.sizes, done.inserted, started.label));
        if stamp(&self.path) == self.stamp {
            self.after_edit = None;
        } else {
            self.reload_if_changed(cx);
        }
        cx.notify();
    }

    fn click_page(&mut self, page: usize, toggle: bool, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable() {
            self.jump_to(page, cx);
            return;
        }
        window.focus(&self.pages_focus, cx);
        self.active_mark = None;
        self.selection = None;
        self.list_cursor = Some(page);
        if extend {
            let from = self.select_anchor.unwrap_or_else(|| self.current_page());
            self.selected = (from.min(page)..=from.max(page)).collect();
        } else if toggle {
            if !self.selected.remove(&page) {
                self.selected.insert(page);
            }
            self.select_anchor = Some(page);
        } else {
            self.selected = BTreeSet::from([page]);
            self.select_anchor = Some(page);
            self.jump_to(page, cx);
        }
        cx.notify();
    }

    /// Moves the keyboard's place in the page list by `by` pages and shows that page; `extend` grows the selection from where it began.
    fn step_page(&mut self, by: isize, extend: bool, cx: &mut Context<Self>) {
        let count = self.page_count();
        if count == 0 {
            return;
        }
        let from = self.list_cursor.unwrap_or_else(|| self.current_page()).min(count - 1);
        let to = (from as isize + by).clamp(0, count as isize - 1) as usize;
        self.active_mark = None;
        self.selection = None;
        if extend {
            let anchor = self.select_anchor.unwrap_or(from);
            self.selected = (anchor.min(to)..=anchor.max(to)).collect();
            self.select_anchor = Some(anchor);
        } else {
            self.selected = BTreeSet::from([to]);
            self.select_anchor = Some(to);
        }
        self.list_cursor = Some(to);
        self.followed_page = to;
        self.thumbs.scroll_to_item(to, ScrollStrategy::Nearest);
        self.jump_to(to, cx);
    }

    fn select_all_pages(&mut self, cx: &mut Context<Self>) {
        let count = self.page_count();
        self.active_mark = None;
        self.selection = None;
        self.selected = (0..count).collect();
        self.select_anchor = Some(0);
        self.list_cursor = Some(count.saturating_sub(1));
        cx.notify();
    }

    fn clear_page_selection(&mut self, cx: &mut Context<Self>) {
        self.selected.clear();
        self.select_anchor = None;
        self.list_cursor = None;
        cx.notify();
    }

    /// Opens the menu of the row the keyboard is on, as a right click on it would.
    fn open_row_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = cx.try_global::<PagePanels>().and_then(|p| p.0.get(&cx.entity_id()).copied()) else { return };
        let page = self.list_cursor.unwrap_or_else(|| self.current_page());
        let offset = self.thumbs.0.borrow().base_handle.offset();
        let top = f32::from(bounds.top());
        let y = (top + f32::from(offset.y) + (page as f32 + 0.5) * THUMB_ROW).clamp(top + 1., f32::from(bounds.bottom()) - 1.);
        let position = point(bounds.center().x, px(y));
        window.defer(cx, move |window, cx| {
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::none() }), cx);
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor().timer(MENU_AFTER_MOVE).await;
                    let _ = cx.update(|window, cx| {
                        window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Right, position, modifiers: Modifiers::none(), click_count: 1, first_mouse: false }), cx);
                    });
                })
                .detach();
        });
    }

    fn scroll_thumbs(&mut self, step: f32) {
        let handle = self.thumbs.0.borrow().base_handle.clone();
        let offset = handle.offset();
        let most = f32::from(handle.max_offset().y);
        handle.set_offset(point(offset.x, px((f32::from(offset.y) + step).clamp(-most, 0.))));
    }

    /// The gap of the page list nearest to the pointer while something is dragged over it.
    fn place_slot(&mut self, cx: &mut Context<Self>) {
        let offset = self.thumbs.0.borrow().base_handle.offset();
        let slot = self.drag_pointer.filter(|(at, bounds)| bounds.contains(at)).map(|(at, bounds)| {
            let y = f32::from(at.y - bounds.top() - offset.y);
            ((y / THUMB_ROW).round().max(0.) as usize).min(self.page_count())
        });
        if slot != self.drop_at {
            self.drop_at = slot;
            cx.notify();
        }
    }

    /// Follows a dragged item over the page list: the insertion line goes to the nearest gap, and the list keeps scrolling while the pointer rests near an edge.
    fn hover_slot(&mut self, at: Point<Pixels>, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.drag_pointer = Some((at, bounds));
        self.drag_step = if !bounds.contains(&at) {
            0.
        } else if f32::from(at.y - bounds.top()) < DRAG_EDGE {
            DRAG_SCROLL
        } else if f32::from(bounds.bottom() - at.y) < DRAG_EDGE {
            -DRAG_SCROLL
        } else {
            0.
        };
        if self.drag_step != 0. && !self.drag_ticking {
            self.drag_ticking = true;
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(DRAG_TICK).await;
                    let go = this.update(cx, |v, cx| {
                        if v.drag_step == 0. || !cx.has_active_drag() {
                            v.drag_step = 0.;
                            v.drag_ticking = false;
                            return false;
                        }
                        v.scroll_thumbs(v.drag_step);
                        v.place_slot(cx);
                        cx.notify();
                        true
                    });
                    if !go.unwrap_or(false) {
                        break;
                    }
                }
            })
            .detach();
        }
        self.place_slot(cx);
    }

    fn page_button(&self, id: &'static str, icon_name: &'static str, help: String, busy: bool, t: &Theme) -> Stateful<Div> {
        let base = div().id(id).size(px(control::S)).flex().items_center().justify_center().rounded(px(radius::SMALL)).tooltip(tip(help));
        if busy {
            base.opacity(tint::DISABLED).key_nav().child(icon(icon_name, iconsize::S, t.secondary))
        } else {
            base.cursor_pointer().key_nav().soft(t.ink.opacity(0.08), radius::SMALL).child(icon(icon_name, iconsize::S, t.secondary))
        }
    }

    fn page_bar(&self, cx: &mut Context<Self>, t: &Theme) -> Div {
        let bar = div().flex_none().h(px(control::SUBBAR)).flex().items_center().gap(px(space::XS)).px(px(space::S)).border_t_1().border_color(t.separator);
        if let Some(why) = self.edit_block() {
            return bar.child(icon("info", iconsize::S, t.secondary)).child(div().flex_1().min_w_0().child(caption(why, t).line_clamp(2)));
        }
        let busy = self.busy();
        let labels = self.page_labels(None);
        let help = |a: ShortcutAction, label: String| if busy { tr("Wird gespeichert …") } else { format!("{label}{}", a.hint()) };
        let me = cx.entity();
        let on = |button: Stateful<Div>, run: fn(&mut PdfView, &mut Context<PdfView>)| {
            let view = me.clone();
            if busy { button } else { button.on_click(move |_, _, cx| view.update(cx, |v, cx| run(v, cx))) }
        };
        let chosen = self.selected.len();
        let clear = {
            let view = me.clone();
            self.page_button("pdf-pages-clear", "x", tr("Auswahl aufheben"), false, t).on_click(move |_, _, cx| view.update(cx, |v, cx| v.clear_page_selection(cx)))
        };
        bar.child(on(self.page_button("pdf-pages-left", "rotate-ccw", help(ShortcutAction::RotatePagesLeft, labels.left), busy, t), |v, cx| v.rotate_pages(None, false, cx)))
            .child(on(self.page_button("pdf-pages-right", "rotate-cw", help(ShortcutAction::RotatePagesRight, labels.right), busy, t), |v, cx| v.rotate_pages(None, true, cx)))
            .child(on(self.page_button("pdf-pages-delete", "trash-2", help(ShortcutAction::DeletePages, labels.delete), busy, t), |v, cx| v.delete_pages(None, cx)))
            .child(on(self.page_button("pdf-pages-insert", "file-plus", help(ShortcutAction::InsertPdf, labels.insert), busy, t), |v, cx| v.choose_pdf(None, cx)))
            .child(div().flex_1())
            .when(chosen > 0, |d| {
                d.child(div().min_w_0().truncate().font_features(tabular()).text_size(px(font::CAPTION)).text_color(t.secondary).child(trf("%lld ausgewählt", &[&chosen]))).child(clear)
            })
    }

    /// The notice about a page change, or the sign that one is being written, over the pages.
    fn page_status(&self, cx: &mut Context<Self>, t: &Theme) -> Option<AnyElement> {
        let card: AnyElement = if let Some((title, body)) = self.page_notice.clone() {
            let me = cx.entity();
            div()
                .id("pdf-pages-notice")
                .occlude()
                .max_w(px(NOTICE_WIDTH))
                .flex()
                .items_start()
                .gap(px(space::S))
                .p(px(space::S))
                .rounded(px(radius::INNER))
                .bg(t.material)
                .border_1()
                .border_color(t.separator)
                .shadow_md()
                .child(icon("info", iconsize::M, t.secondary))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).child(title)).child(caption(body, t)))
                .child(
                    div()
                        .id("pdf-pages-notice-close")
                        .flex_none()
                        .size(px(control::S))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(radius::SMALL))
                        .cursor_pointer()
                        .key_nav()
                        .soft(t.ink.opacity(0.08), radius::SMALL)
                        .tooltip(tip(tr("Schließen")))
                        .on_click(move |_, _, cx| {
                            me.update(cx, |v, cx| {
                                v.page_notice = None;
                                cx.notify();
                            })
                        })
                        .child(icon("x", iconsize::S, t.secondary)),
                )
                .into_any_element()
        } else if self.working.is_some_and(|since| since.elapsed() >= SPINNER_AFTER) {
            div()
                .flex()
                .items_center()
                .gap(px(space::S))
                .px(px(space::M))
                .py(px(space::XS))
                .rounded_full()
                .bg(t.material)
                .border_1()
                .border_color(t.separator)
                .shadow_md()
                .text_size(px(font::CALLOUT))
                .child(motion::spinner(iconsize::S, t.secondary))
                .child(tr("Wird gespeichert …"))
                .into_any_element()
        } else {
            return None;
        };
        Some(div().absolute().top(px(space::S)).left_0().right_0().px(px(space::L)).flex().justify_center().child(card).into_any_element())
    }

    fn thumb_menu(menu: PopupMenu, page: usize, view: Entity<PdfView>, labels: PageLabels, up: bool, down: bool) -> PopupMenu {
        let hint = |a: ShortcutAction, label: String| format!("{label}{}", a.hint());
        let (left, right, above, below, delete, insert) = (view.clone(), view.clone(), view.clone(), view.clone(), view.clone(), view);
        menu.item(PopupMenuItem::new(hint(ShortcutAction::RotatePagesLeft, labels.left)).icon(menu_icon("rotate-ccw")).on_click(move |_, _, cx| left.update(cx, |v, cx| v.rotate_pages(Some(page), false, cx))))
            .item(PopupMenuItem::new(hint(ShortcutAction::RotatePagesRight, labels.right)).icon(menu_icon("rotate-cw")).on_click(move |_, _, cx| right.update(cx, |v, cx| v.rotate_pages(Some(page), true, cx))))
            .separator()
            .item(PopupMenuItem::new(hint(ShortcutAction::MovePagesUp, labels.up)).icon(menu_icon("arrow-up")).disabled(!up).on_click(move |_, _, cx| above.update(cx, |v, cx| v.move_pages(Some(page), false, cx))))
            .item(PopupMenuItem::new(hint(ShortcutAction::MovePagesDown, labels.down)).icon(menu_icon("arrow-down")).disabled(!down).on_click(move |_, _, cx| below.update(cx, |v, cx| v.move_pages(Some(page), true, cx))))
            .separator()
            .item(PopupMenuItem::new(hint(ShortcutAction::DeletePages, labels.delete)).icon(menu_icon("trash-2")).on_click(move |_, _, cx| delete.update(cx, |v, cx| v.delete_pages(Some(page), cx))))
            .item(PopupMenuItem::new(hint(ShortcutAction::InsertPdf, labels.insert)).icon(menu_icon("file-plus")).on_click(move |_, _, cx| insert.update(cx, |v, cx| v.choose_pdf(Some(page), cx))))
    }
}

/// Finishes an edit whose viewer is gone, so the written file is still recorded or put back.
fn settle_without_view(result: Result<Edited, Failure>, started: Started, cx: &mut App) {
    match result {
        Ok(done) => super::state::act(cx, |l| l.finish_pdf_edit(done.written, &done.op, &started.sizes, done.inserted, started.label)),
        Err(Failure::Edit(e)) => {
            let (title, body) = e.describe();
            super::state::act(cx, |l| l.last_error = Some(format!("{title}: {body}")));
        }
        Err(Failure::Write(e)) => super::state::act(cx, |l| l.pdf_edit_failed(e)),
    }
}

fn edit_label(op: &PageOp) -> String {
    let one = |pages: &[usize]| pages.len() == 1;
    match op {
        PageOp::Rotate { pages, .. } => if one(pages) { tr("Seite gedreht") } else { tr("Seiten gedreht") },
        PageOp::Delete(pages) => if one(pages) { tr("Seite gelöscht") } else { tr("Seiten gelöscht") },
        PageOp::Move { pages, .. } => if one(pages) { tr("Seite verschoben") } else { tr("Seiten verschoben") },
        PageOp::Insert { .. } => tr("PDF eingefügt"),
    }
}

impl Render for PdfView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let bar_shown = self.failed.is_none() && !self.sizes.is_empty() && self.panel_open && self.panel_tab == PanelTab::Thumbnails;
        if bar_shown != self.bar_listed {
            self.bar_listed = bar_shown;
            let (id, path) = (cx.entity_id().as_u64(), self.path.clone());
            cx.defer(move |cx| {
                super::state::lib(cx).update(cx, |l, cx| {
                    if bar_shown {
                        l.page_bars.insert(id, path);
                    } else {
                        l.page_bars.remove(&id);
                    }
                    cx.notify();
                })
            });
        }
        if let Some(why) = self.failed {
            let (title, body) = match why {
                Some(e) => e.describe(),
                None => (tr("PDF konnte nicht geöffnet werden"), tr("Die Datei ist beschädigt oder mit einem Passwort geschützt.")),
            };
            return unopenable(&self.path, why == Some(OpenError::Locked), title, body, &t).into_any_element();
        }
        if self.sizes.is_empty() {
            return div().size_full().flex().items_center().justify_center().child(motion::loading(iconsize::L, t.secondary)).into_any_element();
        }
        if !std::mem::replace(&mut self.focused_once, true) {
            window.focus(&self.focus, cx);
        }
        let view = (f32::from(self.view_bounds.size.width), f32::from(self.view_bounds.size.height));
        let view = if view.0 > 0. { view } else { (820., 600.) };
        if !self.restored && self.view_bounds.size.width > px(0.) {
            self.restore_position();
            self.geo.tops.clear();
        }
        let scale = if self.geo.tops.is_empty() { 0. } else { self.geo.scale };
        self.update_geometry(view);
        if scale > 0. && (self.geo.scale - scale).abs() >= 0.0005 {
            self.hold_sharp(cx);
        }
        self.apply_pending();
        if self.restored
            && let Some(index) = self.hit_to_reveal.take()
        {
            self.reveal_hit(index);
        }
        if let Some((start, from, to)) = self.glide {
            let progress = start.elapsed().as_secs_f32() / motion::FAST.as_secs_f32();
            if progress >= 1. {
                self.scroll.set_offset(to);
                self.glide = None;
            } else {
                let eased = 1. - (1. - progress).powi(3);
                self.scroll.set_offset(point(from.x + (to.x - from.x) * eased, from.y + (to.y - from.y) * eased));
                window.request_animation_frame();
            }
        }
        let offset = self.scroll.offset();
        if offset != self.last_offset {
            self.last_offset = offset;
            self.badge_until = Instant::now() + Duration::from_millis(900);
            self.refresh_after(Duration::from_millis(920), cx);
            self.schedule_after_scroll(cx);
        }
        self.sync_marks(cx, &t);
        let visible = self.visible_pages();
        self.request_pages(visible.clone(), window);
        self.request_text(visible.clone());
        self.tick += 1;
        for page in visible.clone() {
            let tick = self.tick;
            for (_, c) in self.cache.iter_mut().filter(|(k, _)| k.0 == page) {
                c.used = tick;
            }
            if let Some((_, used)) = self.texts.get_mut(&page) {
                *used = tick;
            }
        }
        self.evict(&visible, window, cx);
        if self.panel_open {
            self.ensure_inputs(window, cx);
        }
        let panel_width = motion::follow("pdf-panel-width", if self.panel_open { PANEL_WIDTH } else { 0. }, motion::MEDIUM, window, cx);
        let floating = self.floating();
        let panel = (panel_width > 0.).then(|| self.panel(panel_width, window, cx, &t));
        let (docked, floated) = if floating { (None, panel.map(|p| p.absolute().top_0().left_0().bottom_0().bg(t.material).shadow_lg().occlude())) } else { (panel, None) };
        let pages: Vec<AnyElement> = visible.clone().map(|page| self.page_element(page, window, &t)).collect();
        let mark_bar = self.mark_bar(&t, cx);
        let (total, content_width) = (self.geo.total, self.geo.content_width);
        let badge = (Instant::now() < self.badge_until && self.page_count() > 1).then(|| {
            div()
                .absolute()
                .top(px(space::S))
                .right(px(space::L))
                .px(px(space::S))
                .py(px(space::XS))
                .rounded(px(radius::INNER))
                .bg(t.material)
                .border_1()
                .border_color(t.separator)
                .shadow_md()
                .font_features(tabular())
                .text_size(px(font::CAPTION))
                .child(format!("{} / {}", self.current_page() + 1, self.page_count()))
        });
        let me = cx.entity();
        div()
            .id("pdf")
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .key_context("PdfView")
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(Self::on_escape))
            .on_action(cx.listener(|v, _: &super::find_bar::FindInNote, window, cx| v.open_search(window, cx)))
            .on_action(cx.listener(|v, _: &MarkSelection, _, cx| {
                if v.ordered_selection().is_some() {
                    v.add_mark(v.last_color, cx);
                }
            }))
            .on_action(cx.listener(|v, _: &RotatePagesLeft, window, cx| if in_field(window) { cx.propagate() } else { v.rotate_pages(None, false, cx) }))
            .on_action(cx.listener(|v, _: &RotatePagesRight, window, cx| if in_field(window) { cx.propagate() } else { v.rotate_pages(None, true, cx) }))
            .on_action(cx.listener(|v, _: &MovePagesUp, window, cx| if in_field(window) { cx.propagate() } else { v.move_pages(None, false, cx) }))
            .on_action(cx.listener(|v, _: &MovePagesDown, window, cx| if in_field(window) { cx.propagate() } else { v.move_pages(None, true, cx) }))
            .on_action(cx.listener(|v, _: &DeletePages, window, cx| if in_field(window) { cx.propagate() } else { v.delete_pages(None, cx) }))
            .on_action(cx.listener(|v, _: &InsertPdf, window, cx| if in_field(window) { cx.propagate() } else { v.choose_pdf(None, cx) }))
            .on_action(cx.listener(|v, _: &gpui_kit::component::input::Copy, window, cx| {
                if window.context_stack().iter().any(|c| c.contains("Input")) {
                    cx.propagate();
                } else {
                    v.copy_selection();
                }
            }))
            .on_action(cx.listener(|v, _: &gpui_kit::component::input::SelectAll, window, cx| {
                if window.context_stack().iter().any(|c| c.contains("Input")) {
                    cx.propagate();
                } else if v.pages_focus.contains_focused(window, cx) && v.editable() {
                    v.select_all_pages(cx);
                } else {
                    v.select_page(cx);
                }
            }))
            .child(self.toolbar(window, cx, &t))
            .child(
                div().flex_1().min_h_0().flex().relative().children(docked).child(
                    div()
                        .relative()
                        .when(floating && self.panel_open, |d| d.capture_any_mouse_down(cx.listener(|v, _, window, cx| {
                            cx.stop_propagation();
                            if v.panel_tab == PanelTab::Search {
                                v.close_search(window, cx);
                            } else {
                                v.panel_open = false;
                                cx.notify();
                            }
                        })))
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .bg(t.ink.opacity(0.04))
                        .child(canvas(
                            move |bounds, _, cx| {
                                me.update(cx, |v, cx| {
                                    if v.view_bounds != bounds {
                                        v.view_bounds = bounds;
                                        cx.notify();
                                    }
                                })
                            },
                            |_, _, _, _| {},
                        ).absolute().size_full())
                        .child(
                            div()
                                .id("pdf-pages")
                                .size_full()
                                .overflow_scroll()
                                .track_scroll(&self.scroll)
                                .cursor(self.hover_cursor)
                                .on_scroll_wheel(cx.listener(|v, _, _, cx| {
                                    v.glide = None;
                                    cx.notify();
                                }))
                                .on_pinch(cx.listener(|v, e: &PinchEvent, _, cx| {
                                    let at = v.to_view(e.position);
                                    v.zoom_by(1. + e.delta, Some(at), cx);
                                }))
                                .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                                .on_mouse_move(cx.listener(Self::on_mouse_move))
                                .on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, _| v.selecting = false))
                                .on_mouse_up_out(MouseButton::Left, cx.listener(|v, _, _, _| v.selecting = false))
                                .child(div().relative().w(px(content_width)).h(px(total)).children(pages).children(mark_bar)),
                        )
                        .scrollbar(&self.scroll, gpui_kit::component::scroll::ScrollbarAxis::Both)
                        .child(super::image_view::wheel_zoom({
                            let me = cx.entity();
                            move |factor, at, _, cx| me.update(cx, |v, cx| {
                                let at = v.to_view(at);
                                v.zoom_by(factor, Some(at), cx);
                            })
                        }))
                        .children(self.page_status(cx, &t))
                        .children(badge),
                ).children(floated),
            )
            .into_any_element()
    }
}

fn in_field(window: &Window) -> bool {
    window.context_stack().iter().any(|c| c.contains("Input"))
}

const PAGE_COMMANDS: [ShortcutAction; 6] = [ShortcutAction::RotatePagesLeft, ShortcutAction::RotatePagesRight, ShortcutAction::MovePagesUp, ShortcutAction::MovePagesDown, ShortcutAction::DeletePages, ShortcutAction::InsertPdf];

fn is_shortcut(action: ShortcutAction, k: &Keystroke) -> bool {
    let Some(keys) = action.active_shortcut() else { return false };
    let key = match keys.key.as_str() {
        "return" => "enter",
        "delete" => "backspace",
        other => other,
    };
    let (command, control) = if cfg!(target_os = "macos") { (k.modifiers.platform, k.modifiers.control) } else { (k.modifiers.control, false) };
    k.key == key && k.modifiers.shift == keys.shift && k.modifiers.alt == keys.option && command == keys.command && control == keys.control
}

fn is_plain(action: ShortcutAction) -> bool {
    action.active_shortcut().is_some_and(|k| !k.command && !k.option && !k.control)
}

fn is_mark_key(k: &Keystroke) -> bool {
    is_shortcut(ShortcutAction::MarkText, k)
}

fn print_of(marks: &[Highlight], id: Option<u64>) -> Option<(u64, String)> {
    let h = marks.iter().find(|h| Some(h.id) == id)?;
    Some((h.id, highlights::fingerprint(h)))
}

fn color_name(color: &str) -> String {
    match color {
        "yellow" => tr("Gelb"),
        "green" => tr("Grün"),
        "blue" => tr("Blau"),
        _ => tr("Rosa"),
    }
}

fn page_word(slides: bool, page: usize) -> String {
    if slides { trf("Folie %lld", &[&page]) } else { trf("Seite %lld", &[&page]) }
}

fn stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

#[cfg(test)]
mod tests {
    use super::{ShortcutAction, follow_edit, is_mark_key, is_plain, is_shortcut, new_index};
    use crate::core::pdf_edit::PageOp;
    use gpui_kit::Keystroke;

    #[test]
    fn pages_keep_their_place_after_a_delete() {
        let op = PageOp::Delete(vec![1, 2]);
        assert_eq!(new_index(&op, 6, 0, 0), Some(0));
        assert_eq!(new_index(&op, 6, 0, 1), None);
        assert_eq!(new_index(&op, 6, 0, 3), Some(1));
        assert_eq!(new_index(&op, 6, 0, 5), Some(3));
        assert_eq!(follow_edit(&op, 6, 0, 4, &[1, 2]), (Vec::new(), 1, true));
        assert_eq!(follow_edit(&op, 6, 0, 1, &[1, 2]), (Vec::new(), 1, true));
        assert_eq!(follow_edit(&PageOp::Delete(vec![4, 5]), 6, 0, 5, &[4, 5]), (Vec::new(), 3, true));
    }

    #[test]
    fn the_selection_follows_moved_pages() {
        let op = PageOp::Move { pages: vec![1, 2], to: 3 };
        assert_eq!(follow_edit(&op, 6, 0, 1, &[1, 2]), (vec![3, 4], 3, true));
        assert_eq!(new_index(&op, 6, 0, 3), Some(1));
        let up = PageOp::Move { pages: vec![3], to: 2 };
        assert_eq!(follow_edit(&up, 6, 0, 3, &[3]), (vec![2], 2, true));
    }

    #[test]
    fn inserted_pages_are_selected_and_later_pages_shift() {
        let op = PageOp::Insert { at: 2, pdf: Vec::new() };
        assert_eq!(new_index(&op, 4, 3, 1), Some(1));
        assert_eq!(new_index(&op, 4, 3, 2), Some(5));
        assert_eq!(follow_edit(&op, 4, 3, 0, &[]), (vec![2, 3, 4], 2, true));
        let end = PageOp::Insert { at: 99, pdf: Vec::new() };
        assert_eq!(follow_edit(&end, 4, 2, 0, &[]), (vec![4, 5], 4, true));
    }

    #[test]
    fn a_rotation_keeps_the_place_and_the_selection() {
        let op = PageOp::Rotate { pages: vec![0, 2], clockwise: true };
        assert_eq!(follow_edit(&op, 4, 0, 3, &[0, 2]), (vec![0, 2], 3, false));
    }

    #[test]
    fn rotating_is_a_plain_key_and_shift_turns_it_left() {
        let right = Keystroke::parse("r").expect("key");
        let left = Keystroke::parse("shift-r").expect("key");
        assert!(is_shortcut(ShortcutAction::RotatePagesRight, &right));
        assert!(!is_shortcut(ShortcutAction::RotatePagesRight, &left));
        assert!(is_shortcut(ShortcutAction::RotatePagesLeft, &left));
        assert!(is_plain(ShortcutAction::RotatePagesRight));
        assert!(!is_plain(ShortcutAction::DeletePages));
    }

    #[test]
    fn the_mark_key_is_recognised_by_its_shortcut() {
        let h = Keystroke::parse("h").expect("key");
        assert!(is_mark_key(&h));
        assert!(!is_mark_key(&Keystroke::parse("g").expect("key")));
        assert!(!is_mark_key(&Keystroke::parse("shift-h").expect("key")));
    }
}
