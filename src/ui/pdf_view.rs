use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::{self, Soft};
use super::pdf_worker::{self, Hit, Message, PageKey, RenderQueue, TextJob};
use super::theme::{Theme, control, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::highlights::{Highlight, HighlightPart};
use crate::core::i18n::{tr, trf};
use crate::core::pdf_render::{LinkTarget, PageRect, PageText, PdfMeta};
use crate::core::prefs::{self, PdfPosition};

actions!(schrumbi_pdf, [Escape]);

/// Key bindings of the viewer; Escape must reach it before text fields and the main window take it.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("escape", Escape, Some("PdfView")), KeyBinding::new("escape", Escape, Some("PdfView > Input"))]
}

const GAP: f32 = space::M;
const PADDING: f32 = space::L;
const PANEL_WIDTH: f32 = 240.;
const MARK_COLORS: [&str; 4] = ["yellow", "green", "blue", "pink"];
const MARK_BAR_WIDTH: f32 = 148.;
const PREVIEW: u16 = 2;
const MAX_SHARP_WIDTH: f32 = 4096.;
const SHARP_BUDGET: usize = 128 * 1024 * 1024;
const PREVIEW_BUDGET: usize = 48 * 1024 * 1024;
const TEXT_PAGES_KEPT: usize = 32;
const LINE_STEP: f32 = 48.;
const MIN_SCALE: f32 = 0.25;
const MAX_SCALE: f32 = 6.;
const THUMB_ROW: f32 = 176.;
const OUTLINE_ROW: f32 = 28.;
const HIT_ROW: f32 = 56.;

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
    div().id(id).p(px(space::XXS)).flex().items_center().justify_center().rounded(px(radius::SMALL)).cursor_pointer().soft(t.ink.opacity(0.08), radius::SMALL).tooltip(tip(help)).child(icon(icon_name, iconsize::S, t.secondary))
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
    bytes: usize,
    used: u64,
}

/// A selection position: page and caret between glyphs.
type Caret = (usize, usize);

/// A PDF viewer that stays fast with thousands of pages: page positions are computed, not measured,
/// and only pages in view are rendered.
pub struct PdfView {
    path: PathBuf,
    sizes: Vec<(f32, f32)>,
    max_size: (f32, f32),
    failed: bool,
    cache: HashMap<PageKey, Cached>,
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
    thumbs: UniformListScrollHandle,
    thumb_range: Range<usize>,
    followed_page: usize,
    hits: Vec<Hit>,
    current_hit: Option<usize>,
    /// The match to show once the search has found it, as when arriving from the file search.
    wanted_hit: Option<(String, usize)>,
    hit_to_reveal: Option<usize>,
    marks_key: Option<String>,
    marks: Vec<Highlight>,
    active_mark: Option<u64>,
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
    save_task: Option<Task<()>>,
    _timer: Option<Task<()>>,
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
        let (result_tx, result_rx) = async_channel::unbounded();
        let (queue, jobs, search_generation) = pdf_worker::start(path.to_path_buf(), result_tx);
        let load = cx.spawn(async move |this, cx| {
            while let Ok(message) = result_rx.recv().await {
                if this.update(cx, |v, cx| v.receive(message, cx)).is_err() {
                    return;
                }
            }
        });
        let closing = queue.clone();
        let release = cx.on_release(move |this: &mut Self, cx| {
            closing.close();
            this.search_generation.fetch_add(1, Ordering::Relaxed);
            this.save_position();
            for (_, c) in this.cache.drain() {
                let _ = cx.drop_image(c.image, None);
            }
        });
        let library = cx.observe(&super::state::lib(cx), |_, _, cx| cx.notify());
        Self {
            path: path.to_path_buf(),
            sizes: Vec::new(),
            max_size: (1., 1.),
            failed: false,
            cache: HashMap::new(),
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
            thumbs: UniformListScrollHandle::new(),
            thumb_range: 0..0,
            followed_page: usize::MAX,
            hits: Vec::new(),
            current_hit: None,
            wanted_hit: None,
            hit_to_reveal: None,
            marks_key: None,
            marks: Vec::new(),
            active_mark: None,
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
            save_task: None,
            _timer: None,
            _load: Some(load),
            _subs: vec![release, library],
        }
    }

    fn receive(&mut self, message: Message, cx: &mut Context<Self>) {
        match message {
            Message::Opened(sizes) => {
                self.max_size = sizes.iter().fold((1., 1.), |m, s| (f32::max(m.0, s.0), f32::max(m.1, s.1)));
                self.sizes = sizes;
            }
            Message::Key(key) => self.marks_key = Some(key),
            Message::Failed => self.failed = true,
            Message::Page(key, image) => {
                let size = image.size(0);
                let bytes = size.width.0 as usize * size.height.0 as usize * 4;
                self.tick += 1;
                self.cache.insert(key, Cached { image, bytes, used: self.tick });
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
            Message::Clipboard(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
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

    fn request_pages(&mut self, visible: Range<usize>, window: &Window) {
        let mut wanted: Vec<PageKey> = Vec::new();
        let count = self.page_count();
        let sharp_ready = Instant::now() >= self.sharp_after;
        let mut add = |key: PageKey, cache: &HashMap<PageKey, Cached>| {
            if !cache.contains_key(&key) && !wanted.contains(&key) {
                wanted.push(key);
            }
        };
        for page in visible.clone() {
            add((page, PREVIEW), &self.cache);
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
        for (is_preview, budget) in [(true, PREVIEW_BUDGET), (false, SHARP_BUDGET)] {
            let mut used: usize = self.cache.iter().filter(|(k, _)| (k.1 == PREVIEW) == is_preview).map(|(_, c)| c.bytes).sum();
            while used > budget {
                let oldest = self.cache.iter().filter(|(k, _)| (k.1 == PREVIEW) == is_preview && !needed.contains(k)).min_by_key(|(_, c)| c.used).map(|(k, _)| *k);
                let Some(key) = oldest else { break };
                if let Some(c) = self.cache.remove(&key) {
                    used -= c.bytes;
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
        self.sharp_after = Instant::now() + Duration::from_millis(140);
        self.refresh_after(Duration::from_millis(150), cx);
        cx.notify();
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
                let _ = opener::open(uri);
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
        let mut list = self.marks.clone();
        let id = list.iter().map(|h| h.id).max().map_or(1, |m| m + 1);
        list.push(Highlight { id, color: color.to_string(), text, parts });
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
        let x = (left + r[0] * scale).min(self.geo.content_width - MARK_BAR_WIDTH).max(0.);
        let y = top + (r[1] + r[3]) * scale + space::XS;
        let me = cx.entity();
        let current = editing.map(|h| h.color.clone());
        let editing_id = editing.map(|h| h.id);
        let ring = t.secondary;
        let action: Stateful<Div> = match editing_id {
            Some(id) => {
                let e = me.clone();
                bar_action("pdf-mark-remove", "trash-2", tr("Markierung entfernen"), t).on_click(move |_, _, cx| e.update(cx, |v, cx| v.remove_mark(id, cx)))
            }
            None => {
                let e = me.clone();
                bar_action("pdf-mark-copy", "copy", tr("Kopieren"), t).on_click(move |_, _, cx| e.read(cx).copy_selection())
            }
        };
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
                .children(MARK_COLORS.into_iter().map(|color| {
                    let e = me.clone();
                    let selected = current.as_deref() == Some(color);
                    div()
                        .id(SharedString::from(format!("pdf-mark-{color}")))
                        .size(px(iconsize::M))
                        .rounded_full()
                        .bg(t.palette(color))
                        .cursor_pointer()
                        .when(selected, |d| d.border_2().border_color(t.primary))
                        .when(!selected, |d| d.hover(move |s| s.border_2().border_color(ring)))
                        .tooltip(tip(tr("Markieren (H)")))
                        .on_click(move |_, _, cx| {
                            e.update(cx, |v, cx| match editing_id {
                                Some(id) => v.recolor_mark(id, color, cx),
                                None => v.add_mark(color, cx),
                            })
                        })
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
                        .child(div().text_size(px(font::CAPTION)).text_color(t.secondary).child(trf("Seite %lld", &[&(page + 1)])))
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

    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_page || self.editing_zoom {
            self.editing_page = false;
            self.editing_zoom = false;
            window.focus(&self.focus, cx);
        } else if self.search_active {
            self.close_search(window, cx);
        } else if self.active_mark.take().is_none() && self.selection.take().is_none() {
            cx.propagate();
        }
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let m = &k.modifiers;
        let command = if cfg!(target_os = "macos") { m.platform } else { m.control };
        let typing = window.context_stack().iter().any(|c| c.contains("Input"));
        let key = k.key.as_str();
        let page_step = (self.geo.view.1 - LINE_STEP).max(LINE_STEP);
        match key {
            "f" if command && !m.shift => self.open_search(window, cx),
            "g" if command => self.step_hit(if m.shift { -1 } else { 1 }, cx),
            _ if typing => return,
            "h" if !command && self.ordered_selection().is_some() => self.add_mark(self.last_color, cx),
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

    fn page_element(&self, page: usize, window: &Window, t: &Theme) -> AnyElement {
        let (left, top, w, h) = self.page_rect(page);
        let scale = self.geo.scale;
        let sharp = self.cache.get(&(page, self.sharp_quarters(page, window))).or_else(|| self.cache.iter().filter(|(k, _)| k.0 == page && k.1 != PREVIEW).max_by_key(|(k, _)| k.1).map(|(_, c)| c));
        let image = sharp.or_else(|| self.cache.get(&(page, PREVIEW))).map(|c| c.image.clone());
        let mark = |r: PageRect, color: Hsla| div().absolute().left(px(r.x * scale)).top(px(r.y * scale)).w(px(r.w * scale)).h(px(r.h * scale)).bg(color);
        let mut el = div().absolute().left(px(left)).top(px(top)).w(px(w)).h(px(h)).bg(white()).shadow_md().children(image.map(|i| img(i).size_full()));
        let (yellow, orange) = (t.palette("yellow").opacity(0.45), t.orange.opacity(0.6));
        for h in &self.marks {
            let opacity = if self.active_mark == Some(h.id) { 0.6 } else { 0.38 };
            for r in h.parts.iter().filter(|p| p.page == page).flat_map(|p| &p.rects) {
                el = el.child(mark(PageRect { x: r[0], y: r[1], w: r[2], h: r[3] }, t.palette(&h.color).opacity(opacity)));
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

    fn toolbar(&self, cx: &mut Context<Self>, t: &Theme) -> Div {
        let me = cx.entity();
        let count = self.page_count();
        let current = self.current_page() + 1;
        let (m1, m2, m3, m4, m5, m6, m7) = (me.clone(), me.clone(), me.clone(), me.clone(), me.clone(), me.clone(), me);
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
                .tooltip(tip(tr("Zu Seite springen")))
                .child(trf("Seite %lld von %lld", &[&current, &count]))
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
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::S))
            .border_b_1()
            .border_color(t.separator)
            .child(Button::new("pdf-panel").ghost().small().icon(menu_icon("panel-left")).tooltip(tr("Seitenleiste")).on_click(move |_, _, cx| m3.update(cx, |v, cx| {
                v.panel_open = !v.panel_open;
                cx.notify();
            })))
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
                menu.item(entry(tr("Breite einpassen"), Fit::Width, v.clone())).item(entry(tr("Seite einpassen"), Fit::Page, v.clone())).separator().item(entry("50 %".into(), Fit::Scale(0.5), v.clone())).item(entry("100 %".into(), Fit::Scale(1.), v.clone())).item(entry("150 %".into(), Fit::Scale(1.5), v.clone())).item(entry("200 %".into(), Fit::Scale(2.), v))
            }))
    }

    fn panel(&mut self, width: f32, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> Div {
        self.ensure_inputs(window, cx);
        let me = cx.entity();
        let tab = |id: &'static str, label: String, value: PanelTab, active: bool, view: Entity<PdfView>| {
            div()
                .id(id)
                .flex_1()
                .flex()
                .justify_center()
                .py(px(space::XS))
                .rounded(px(radius::INNER))
                .cursor_pointer()
                .key_nav()
                .text_size(px(font::CALLOUT))
                .when(active, |d| d.bg(t.ink.opacity(0.08)).font_weight(FontWeight::MEDIUM))
                .soft(t.ink.opacity(0.05), radius::INNER)
                .child(label)
                .on_click(move |_, window, cx| view.update(cx, |v, cx| {
                    v.panel_tab = value;
                    if value == PanelTab::Search {
                        v.open_search(window, cx);
                    }
                    cx.notify();
                }))
        };
        let body: AnyElement = match self.panel_tab {
            PanelTab::Thumbnails => self.thumbnails(cx).into_any_element(),
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
                    .child(
                        div()
                            .flex()
                            .gap(px(space::XS))
                            .p(px(space::S))
                            .child(tab("pdf-tab-thumbs", tr("Seiten"), PanelTab::Thumbnails, self.panel_tab == PanelTab::Thumbnails, me.clone()))
                            .child(tab("pdf-tab-outline", tr("Inhalt"), PanelTab::Outline, self.panel_tab == PanelTab::Outline, me.clone()))
                            .child(tab("pdf-tab-search", tr("Suche"), PanelTab::Search, self.panel_tab == PanelTab::Search, me.clone()))
                            .child(tab("pdf-tab-marks", tr("Marker"), PanelTab::Marks, self.panel_tab == PanelTab::Marks, me)),
                    )
                    .child(div().flex_1().min_h_0().child(body)),
            )
    }

    fn thumbnails(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.current_page();
        if self.followed_page != current {
            self.followed_page = current;
            self.thumbs.scroll_to_item(current, ScrollStrategy::Center);
        }
        let handle = self.thumbs.clone();
        div().relative().size_full().child(
            uniform_list(
                "pdf-thumbs",
                self.page_count(),
                cx.processor(move |this, range: Range<usize>, _, cx| {
                    this.thumb_range = range.clone();
                    let t = Theme::get(cx);
                    let current = this.current_page();
                    let me = cx.entity();
                    range
                        .map(|page| {
                            let (w, h) = this.sizes[page];
                            let fit = (130. / h.max(1.)).min(180. / w.max(1.));
                            let image = this.cache.get(&(page, PREVIEW)).map(|c| c.image.clone());
                            let view = me.clone();
                            div()
                                .id(("pdf-thumb", page))
                                .w_full()
                                .h(px(THUMB_ROW))
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(px(space::XS))
                                .cursor_pointer()
                                .key_nav()
                                .soft(t.ink.opacity(0.05), 0.)
                                .child(div().w(px(w * fit)).h(px(h * fit)).bg(white()).shadow_sm().when(page == current, |d| d.border_2().border_color(t.accent)).children(image.map(|i| img(i).size_full())))
                                .child(div().text_size(px(font::CAPTION)).font_features(tabular()).text_color(if page == current { t.accent } else { t.secondary }).child((page + 1).to_string()))
                                .on_click(move |_, _, cx| view.update(cx, |v, cx| v.jump_to(page, cx)))
                                .into_any_element()
                        })
                        .collect()
                }),
            )
            .track_scroll(&handle)
            .size_full(),
        ).vertical_scrollbar(&handle)
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
        let total = self.page_count();
        let running = self.search_progress < total && self.search_input.as_ref().is_some_and(|i| !i.read(cx).value().trim().is_empty());
        let summary = if self.hits.is_empty() {
            if running { trf("Durchsuche Seite %lld von %lld", &[&self.search_progress, &total]) } else if self.search_input.as_ref().is_some_and(|i| !i.read(cx).value().trim().is_empty()) { tr("Keine Treffer") } else { String::new() }
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
            .child(div().px(px(space::S)).children(self.search_input.as_ref().map(|i| Input::new(i).small())))
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
                        cx.processor(|this, range: Range<usize>, _, cx| {
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
                                        .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).text_color(t.secondary).child(trf("Seite %lld", &[&(hit.page + 1)])))
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

impl Render for PdfView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        if self.failed {
            return unavailable("file-question", tr("PDF konnte nicht geöffnet werden"), tr("Die Datei ist beschädigt oder mit einem Passwort geschützt."), &t).into_any_element();
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
        self.update_geometry(view);
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
        let panel = (panel_width > 0.).then(|| self.panel(panel_width, window, cx, &t));
        if let Some(key) = self.marks_key.clone() {
            self.marks = super::state::read(cx, |l| l.highlights_of(&key));
        }
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
                } else {
                    v.select_page(cx);
                }
            }))
            .child(self.toolbar(cx, &t))
            .child(
                div().flex_1().min_h_0().flex().children(panel).child(
                    div()
                        .relative()
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
                        .children(badge),
                ),
            )
            .into_any_element()
    }
}
