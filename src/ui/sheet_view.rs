use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::{self, Soft};
use super::theme::{Theme, control, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::crash;
use crate::core::i18n::{tr, trf};
use crate::core::office::OpenError;
use crate::core::office::sheet::{Book, Cell, CellKind, HAlign, Merge, Sheet, Style, VAlign, column_name};
use crate::core::pdf_render::{fold, fold_query};

actions!(schrumbi_sheet, [Escape]);

/// Escape must reach the grid before text fields and the main window take it.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("escape", Escape, Some("SheetView")), KeyBinding::new("escape", Escape, Some("SheetView > Input"))]
}

const HEADER_HEIGHT: f32 = 22.;
const CELL_PADDING: f32 = 2.;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.;
const EXTRA_ROWS: u32 = 60;
const EXTRA_COLS: u32 = 16;
const HIT_LIMIT: usize = 10_000;
const SCROLLBAR: f32 = 12.;
const TAB_WIDTH: f32 = 180.;
const TINT: f32 = 0.12;
const MUTED: u32 = 0x6b6b6b;
const FROZEN_SHARE: f32 = 0.75;
const COPY_IN_BACKGROUND: u64 = 20_000;
const PAPER: u32 = 0xffffff;
const GRID: u32 = 0xe1e1e1;
const FROZEN_LINE: u32 = 0xb4b4b4;
const INK: u32 = 0x000000;
const ZOOMS: [f32; 6] = [0.5, 0.75, 1., 1.25, 1.5, 2.];

enum Message {
    Opened(Vec<String>, usize),
    Sheet(usize, Arc<Sheet>),
    SheetFailed(usize),
    Failed(OpenError),
}

fn start(path: PathBuf, results: async_channel::Sender<Message>) {
    std::thread::spawn(move || {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let opened = crash::caught(|| std::fs::read(&path).map_err(|_| OpenError::Broken).and_then(|bytes| Book::open(bytes, &ext))).unwrap_or(Err(OpenError::Broken)).and_then(|book| if book.names.is_empty() { Err(OpenError::Broken) } else { Ok(book) });
        let mut book = match opened {
            Ok(book) => book,
            Err(why) => {
                let _ = results.send_blocking(Message::Failed(why));
                return;
            }
        };
        if results.send_blocking(Message::Opened(book.names.clone(), book.active)).is_err() {
            return;
        }
        let active = book.active;
        for i in std::iter::once(active).chain((0..book.names.len()).filter(|i| *i != active)) {
            let message = match crash::caught(|| book.sheet(i)) {
                Some(sheet) => Message::Sheet(i, Arc::new(sheet)),
                None => Message::SheetFailed(i),
            };
            if results.send_blocking(message).is_err() {
                return;
            }
        }
    });
}

/// A selected range: where it started and where it ends now, as (row, column).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Selection {
    anchor: (u32, u32),
    head: (u32, u32),
}

impl Selection {
    fn at(cell: (u32, u32)) -> Self {
        Self { anchor: cell, head: cell }
    }

    fn range(&self) -> (u32, u32, u32, u32) {
        (self.anchor.0.min(self.head.0), self.anchor.1.min(self.head.1), self.anchor.0.max(self.head.0), self.anchor.1.max(self.head.1))
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Cells,
    Rows,
    Columns,
}

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Cell(u32, u32),
    Row(u32),
    Column(u32),
    Corner,
}

struct Place {
    offset: Point<Pixels>,
    selection: Selection,
}

/// Positions of the grid in the view at one moment: header sizes, frozen panes and scroll.
#[derive(Clone, Copy)]
struct Geo {
    zoom: f32,
    header_w: f32,
    header_h: f32,
    frozen_w: f32,
    frozen_h: f32,
    frozen_rows: u32,
    frozen_cols: u32,
    sx: f32,
    sy: f32,
    w: f32,
    h: f32,
}

impl Geo {
    fn new(s: &Sheet, zoom: f32, offset: Point<Pixels>, size: (f32, f32)) -> Self {
        let rows = s.used_rows.max(s.frozen_rows).saturating_add(EXTRA_ROWS);
        let digits = (rows.max(1000) as f32).log10().floor() + 1.;
        let header_w = (digits * 7. + 2. * space::S).round();
        let fits = |frozen: f32, room: f32| room <= 0. || frozen <= room * FROZEN_SHARE;
        let frozen_cols = if fits(s.cols.start(s.frozen_cols) * zoom, size.0 - header_w) { s.frozen_cols } else { 0 };
        let frozen_rows = if fits(s.heights.start(s.frozen_rows) * zoom, size.1 - HEADER_HEIGHT) { s.frozen_rows } else { 0 };
        Self {
            zoom,
            header_w,
            header_h: HEADER_HEIGHT,
            frozen_w: s.cols.start(frozen_cols) * zoom,
            frozen_h: s.heights.start(frozen_rows) * zoom,
            frozen_rows,
            frozen_cols,
            sx: (-f32::from(offset.x)).max(0.),
            sy: (-f32::from(offset.y)).max(0.),
            w: size.0,
            h: size.1,
        }
    }

    fn col_x(&self, s: &Sheet, c: u32) -> f32 {
        let x = self.header_w + s.cols.start(c) * self.zoom;
        if c < self.frozen_cols { x } else { x - self.sx }
    }

    fn row_y(&self, s: &Sheet, r: u32) -> f32 {
        let y = self.header_h + s.heights.start(r) * self.zoom;
        if r < self.frozen_rows { y } else { y - self.sy }
    }

    fn col_end(&self, s: &Sheet, c: u32) -> f32 {
        let x = self.header_w + s.cols.start(c + 1) * self.zoom;
        if c < self.frozen_cols { x } else { x - self.sx }
    }

    fn row_end(&self, s: &Sheet, r: u32) -> f32 {
        let y = self.header_h + s.heights.start(r + 1) * self.zoom;
        if r < self.frozen_rows { y } else { y - self.sy }
    }

    fn col_at(&self, s: &Sheet, x: f32) -> u32 {
        let local = x - self.header_w;
        if local < self.frozen_w { s.cols.at(local.max(0.) / self.zoom) } else { s.cols.at((local + self.sx) / self.zoom).max(self.frozen_cols) }
    }

    fn row_at(&self, s: &Sheet, y: f32) -> u32 {
        let local = y - self.header_h;
        if local < self.frozen_h { s.heights.at(local.max(0.) / self.zoom) } else { s.heights.at((local + self.sy) / self.zoom).max(self.frozen_rows) }
    }

    fn scroll_cols(&self, s: &Sheet) -> Range<u32> {
        let first = s.cols.at((self.frozen_w + self.sx) / self.zoom).max(self.frozen_cols);
        let last = s.cols.at((self.sx + self.w - self.header_w).max(0.) / self.zoom) + 1;
        first..last.max(first)
    }

    fn scroll_rows(&self, s: &Sheet) -> Range<u32> {
        let first = s.heights.at((self.frozen_h + self.sy) / self.zoom).max(self.frozen_rows);
        let last = s.heights.at((self.sy + self.h - self.header_h).max(0.) / self.zoom) + 1;
        first..last.max(first)
    }

    fn extent(&self, s: &Sheet) -> (u32, u32) {
        let rows = s.used_rows.max(s.frozen_rows).saturating_add(EXTRA_ROWS).max(s.heights.at((self.h - self.header_h).max(0.) / self.zoom) + 2);
        let cols = s.used_cols.max(s.frozen_cols).saturating_add(EXTRA_COLS).max(s.cols.at((self.w - self.header_w).max(0.) / self.zoom) + 2);
        (rows, cols)
    }

    fn content(&self, s: &Sheet) -> (f32, f32) {
        let (rows, cols) = self.extent(s);
        (self.header_w + s.cols.start(cols) * self.zoom, self.header_h + s.heights.start(rows) * self.zoom)
    }
}

/// A read-only spreadsheet: sheet tabs, a virtualised grid with frozen panes, selection, copy and search.
pub struct SheetView {
    path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
    names: Vec<String>,
    sheets: Vec<Option<Arc<Sheet>>>,
    broken: Vec<bool>,
    fonts: Vec<Arc<Vec<Font>>>,
    active: usize,
    failed: Option<OpenError>,
    opened: bool,
    places: HashMap<usize, Place>,
    scroll: ScrollHandle,
    tabs: ScrollHandle,
    view: Bounds<Pixels>,
    zoom: f32,
    selection: Selection,
    drag: Option<Drag>,
    whole: Option<(Drag, Selection)>,
    before_drag: Selection,
    focus: FocusHandle,
    focused_once: bool,
    search_open: bool,
    search_input: Option<Entity<InputState>>,
    hits: Arc<Vec<(u32, u32)>>,
    current_hit: Option<usize>,
    searching: bool,
    search_generation: Arc<AtomicU64>,
    wanted_hit: Option<usize>,
    _load: Option<Task<()>>,
    _search: Option<Task<()>>,
    _copy: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl SheetView {
    pub fn new(path: &Path, cx: &mut Context<Self>) -> Self {
        let library = cx.observe(&super::state::lib(cx), |v: &mut Self, _, cx| {
            v.reload_if_changed(cx);
            cx.notify();
        });
        let mut view = Self {
            path: path.to_path_buf(),
            stamp: stamp(path),
            names: Vec::new(),
            sheets: Vec::new(),
            broken: Vec::new(),
            fonts: Vec::new(),
            active: 0,
            failed: None,
            opened: false,
            places: HashMap::new(),
            scroll: ScrollHandle::new(),
            tabs: ScrollHandle::new(),
            view: Bounds::default(),
            zoom: 1.,
            selection: Selection::at((0, 0)),
            drag: None,
            whole: None,
            before_drag: Selection::at((0, 0)),
            focus: cx.focus_handle(),
            focused_once: true,
            search_open: false,
            search_input: None,
            hits: Arc::new(Vec::new()),
            current_hit: None,
            searching: false,
            search_generation: Arc::new(AtomicU64::new(0)),
            wanted_hit: None,
            _load: None,
            _search: None,
            _copy: None,
            _subs: vec![library],
        };
        view.connect(cx);
        view
    }

    /// Takes the keyboard once the workbook is loaded.
    pub fn take_focus_on_load(mut self) -> Self {
        self.focused_once = false;
        self
    }

    pub fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focused_once = true;
        window.focus(&self.focus, cx);
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = async_channel::unbounded();
        start(self.path.clone(), tx);
        self._load = Some(cx.spawn(async move |this, cx| {
            while let Ok(message) = rx.recv().await {
                if this.update(cx, |v, cx| v.receive(message, cx)).is_err() {
                    return;
                }
            }
        }));
    }

    fn reload_if_changed(&mut self, cx: &mut Context<Self>) {
        let now = stamp(&self.path);
        if now.is_none() || now == self.stamp {
            return;
        }
        self.stamp = now;
        self.connect(cx);
    }

    fn receive(&mut self, message: Message, cx: &mut Context<Self>) {
        match message {
            Message::Opened(names, active) => {
                let first = !self.opened;
                self.failed = None;
                self.sheets.resize(names.len(), None);
                self.broken = vec![false; names.len()];
                self.fonts.resize(names.len(), Arc::default());
                self.names = names;
                if first {
                    self.active = active.min(self.names.len().saturating_sub(1));
                } else {
                    self.active = self.active.min(self.names.len().saturating_sub(1));
                }
                self.opened = true;
            }
            Message::Sheet(i, sheet) => {
                if i < self.sheets.len() {
                    self.fonts[i] = Arc::new(sheet.styles.iter().map(|s| cell_font(s, cx)).collect());
                    self.broken[i] = false;
                    self.sheets[i] = Some(sheet);
                    if i == self.active && self.search_open {
                        self.rerun_search(cx);
                    }
                }
            }
            Message::SheetFailed(i) => {
                if i < self.sheets.len() {
                    self.broken[i] = true;
                    self.sheets[i] = None;
                    self.fonts[i] = Arc::default();
                }
            }
            Message::Failed(why) => self.failed = Some(why),
        }
        cx.notify();
    }

    fn sheet(&self) -> Option<Arc<Sheet>> {
        self.sheets.get(self.active).cloned().flatten()
    }

    fn view_size(&self) -> (f32, f32) {
        let (w, h) = (f32::from(self.view.size.width), f32::from(self.view.size.height));
        if w > 0. { (w, h) } else { (800., 600.) }
    }

    fn geo(&self, s: &Sheet) -> Geo {
        Geo::new(s, self.zoom, self.scroll.offset(), self.view_size())
    }

    fn switch_to(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index == self.active || index >= self.names.len() {
            return;
        }
        self.places.insert(self.active, Place { offset: self.scroll.offset(), selection: self.selection });
        self.active = index;
        self.tabs.scroll_to_item(index);
        let place = self.places.remove(&index);
        self.selection = place.as_ref().map_or(Selection::at((0, 0)), |p| p.selection);
        self.scroll.set_offset(place.map_or(point(px(0.), px(0.)), |p| p.offset));
        if self.search_open {
            self.rerun_search(cx);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn target(&self, position: Point<Pixels>) -> Option<Target> {
        let s = self.sheet()?;
        let g = self.geo(&s);
        let x = f32::from(position.x - self.view.origin.x);
        let y = f32::from(position.y - self.view.origin.y);
        Some(match (x < g.header_w, y < g.header_h) {
            (true, true) => Target::Corner,
            (true, false) => Target::Row(g.row_at(&s, y)),
            (false, true) => Target::Column(g.col_at(&s, x)),
            (false, false) => Target::Cell(g.row_at(&s, y), g.col_at(&s, x)),
        })
    }

    fn last_cell(&self) -> (u32, u32) {
        self.sheet().map_or((0, 0), |s| (s.used_rows.saturating_sub(1), s.used_cols.saturating_sub(1)))
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(target) = self.target(e.position) else { return };
        let (last_row, last_col) = self.extent_last();
        let shift = e.modifiers.shift;
        self.before_drag = self.selection;
        match target {
            Target::Cell(r, c) => {
                if shift {
                    self.selection.head = (r, c);
                } else {
                    self.selection = self.merged((r, c));
                }
                self.drag = Some(Drag::Cells);
            }
            Target::Row(r) => {
                self.selection = if shift { Selection { anchor: (self.selection.anchor.0, 0), head: (r, last_col) } } else { Selection { anchor: (r, 0), head: (r, last_col) } };
                self.drag = Some(Drag::Rows);
                self.whole = Some((Drag::Rows, self.selection));
            }
            Target::Column(c) => {
                self.selection = if shift { Selection { anchor: (0, self.selection.anchor.1), head: (last_row, c) } } else { Selection { anchor: (0, c), head: (last_row, c) } };
                self.drag = Some(Drag::Columns);
                self.whole = Some((Drag::Columns, self.selection));
            }
            Target::Corner => self.select_all(cx),
        }
        cx.notify();
    }

    fn on_mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let Some(s) = self.sheet() else { return };
        let g = self.geo(&s);
        let x = f32::from(e.position.x - self.view.origin.x).clamp(g.header_w, g.w - 1.);
        let y = f32::from(e.position.y - self.view.origin.y).clamp(g.header_h, g.h - 1.);
        let edge = |p: f32, lo: f32, hi: f32| if p <= lo + 1. { -24. } else if p >= hi - 2. { 24. } else { 0. };
        let (dx, dy) = (edge(f32::from(e.position.x - self.view.origin.x), g.header_w, g.w), edge(f32::from(e.position.y - self.view.origin.y), g.header_h, g.h));
        if dx != 0. || dy != 0. {
            let o = self.scroll.offset();
            self.scroll.set_offset(point(px((f32::from(o.x) - dx).min(0.)), px((f32::from(o.y) - dy).min(0.))));
        }
        let (r, c) = (g.row_at(&s, y), g.col_at(&s, x));
        let (last_row, last_col) = self.extent_last();
        let head = match drag {
            Drag::Cells => (r, c),
            Drag::Rows => (r, last_col),
            Drag::Columns => (last_row, c),
        };
        if head != self.selection.head {
            self.selection.head = head;
            if let Some((d, _)) = self.whole {
                self.whole = Some((d, self.selection));
            }
            cx.notify();
        }
    }

    fn extent_last(&self) -> (u32, u32) {
        self.sheet().map_or((0, 0), |s| {
            let (rows, cols) = self.geo(&s).extent(&s);
            (rows - 1, cols - 1)
        })
    }

    fn merged(&self, cell: (u32, u32)) -> Selection {
        match self.sheet().and_then(|s| s.merge_at(cell.0, cell.1)) {
            Some(m) => Selection { anchor: (m.r0, m.c0), head: (m.r1, m.c1) },
            None => Selection::at(cell),
        }
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let (r, c) = self.last_cell();
        self.selection = Selection { anchor: (0, 0), head: (r, c) };
        cx.notify();
    }

    fn move_by(&mut self, dr: i64, dc: i64, extend: bool, cx: &mut Context<Self>) {
        let (last_row, last_col) = self.extent_last();
        let from = if extend { self.selection.head } else { self.selection.anchor };
        let to = ((from.0 as i64 + dr).clamp(0, last_row as i64) as u32, (from.1 as i64 + dc).clamp(0, last_col as i64) as u32);
        self.move_to(to, extend, cx);
    }

    fn move_to(&mut self, to: (u32, u32), extend: bool, cx: &mut Context<Self>) {
        if extend {
            self.selection.head = to;
        } else {
            self.selection = Selection::at(to);
        }
        self.reveal(to);
        cx.notify();
    }

    /// Scrolls so a cell is in view, leaving frozen panes alone.
    fn reveal(&mut self, (r, c): (u32, u32)) {
        let Some(s) = self.sheet() else { return };
        let g = self.geo(&s);
        let mut o = self.scroll.offset();
        if c >= g.frozen_cols {
            let left = s.cols.start(c) * g.zoom - g.frozen_w;
            let right = s.cols.start(c + 1) * g.zoom - g.frozen_w;
            let room = g.w - g.header_w - g.frozen_w - SCROLLBAR;
            if left < g.sx {
                o.x = px(-left);
            } else if right > g.sx + room {
                o.x = px(-(right - room).max(0.));
            }
        }
        if r >= g.frozen_rows {
            let top = s.heights.start(r) * g.zoom - g.frozen_h;
            let bottom = s.heights.start(r + 1) * g.zoom - g.frozen_h;
            let room = g.h - g.header_h - g.frozen_h - SCROLLBAR;
            if top < g.sy {
                o.y = px(-top);
            } else if bottom > g.sy + room {
                o.y = px(-(bottom - room).max(0.));
            }
        }
        self.scroll.set_offset(o);
    }

    fn page_rows(&self) -> i64 {
        self.sheet().map_or(20, |s| {
            let g = self.geo(&s);
            let r = g.scroll_rows(&s);
            (r.end - r.start).saturating_sub(2).max(1) as i64
        })
    }

    fn zoom_to(&mut self, zoom: f32, at: Option<(f32, f32)>, cx: &mut Context<Self>) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let Some(s) = self.sheet() else {
            self.zoom = zoom;
            cx.notify();
            return;
        };
        let g = self.geo(&s);
        let (ax, ay) = at.unwrap_or((g.header_w, g.header_h));
        let ratio = zoom / self.zoom;
        let x = ((g.sx + (ax - g.header_w).max(0.)) * ratio - (ax - g.header_w).max(0.)).max(0.);
        let y = ((g.sy + (ay - g.header_h).max(0.)) * ratio - (ay - g.header_h).max(0.)).max(0.);
        self.zoom = zoom;
        self.scroll.set_offset(point(px(-x), px(-y)));
        cx.notify();
    }

    fn zoom_by(&mut self, factor: f32, at: Option<(f32, f32)>, cx: &mut Context<Self>) {
        let target = (self.zoom * factor * 100.).round() / 100.;
        self.zoom_to(target, at, cx);
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.sheet() else { return };
        let (r0, c0, r1, c1) = self.selection.range();
        let (r1, c1) = (r1.min(s.used_rows.max(r0 + 1) - 1).max(r0), c1.min(s.used_cols.max(c0 + 1) - 1).max(c0));
        let cells = (r1 - r0 + 1) as u64 * (c1 - c0 + 1) as u64;
        if cells < COPY_IN_BACKGROUND {
            cx.write_to_clipboard(ClipboardItem::new_string(s.tsv(r0, c0, r1, c1)));
            return;
        }
        let task = cx.background_spawn(async move { s.tsv(r0, c0, r1, c1) });
        self._copy = Some(cx.spawn(async move |_, cx| {
            let text = task.await;
            let _ = cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text)));
        }));
    }

    fn ensure_search_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_input.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr("In diesem Blatt suchen")));
        self._subs.push(cx.subscribe_in(&input, window, |this, s, e: &InputEvent, _, cx| match e {
            InputEvent::Change => {
                let q = s.read(cx).value().to_string();
                this.start_search(q, cx);
            }
            InputEvent::PressEnter { shift, .. } => this.step_hit(if *shift { -1 } else { 1 }, cx),
            _ => {}
        }));
        self.search_input = Some(input);
    }

    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        self.ensure_search_input(window, cx);
        if let Some(input) = self.search_input.clone() {
            input.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            });
        }
        cx.notify();
    }

    /// Opens the search with a query, as when arriving from the file search.
    pub fn search_for(&mut self, query: &str, nth: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search(window, cx);
        self.wanted_hit = Some(nth);
        if let Some(input) = self.search_input.clone() {
            input.update(cx, |s, cx| s.set_value(query.to_string(), window, cx));
        }
        self.start_search(query.to_string(), cx);
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_generation.fetch_add(1, Ordering::Relaxed);
        self.search_open = false;
        self.hits = Arc::new(Vec::new());
        self.current_hit = None;
        self.searching = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn rerun_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.as_ref().map(|i| i.read(cx).value().to_string()).unwrap_or_default();
        self.start_search(query, cx);
    }

    fn start_search(&mut self, query: String, cx: &mut Context<Self>) {
        let generation = self.search_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let folded = fold_query(query.trim());
        self.current_hit = None;
        if folded.is_empty() {
            self.hits = Arc::new(Vec::new());
            self.searching = false;
            cx.notify();
            return;
        }
        let Some(s) = self.sheet() else { return };
        self.searching = true;
        let latest = self.search_generation.clone();
        let task = cx.background_spawn(async move {
            let mut hits = Vec::new();
            let mut buf = String::new();
            'rows: for row in &s.rows {
                if latest.load(Ordering::Relaxed) != generation {
                    return None;
                }
                for cell in &row.cells {
                    buf.clear();
                    for ch in cell.text.chars() {
                        fold(ch, &mut buf);
                    }
                    if buf.contains(&folded) {
                        hits.push((row.index, cell.col));
                        if hits.len() >= HIT_LIMIT {
                            break 'rows;
                        }
                    }
                }
            }
            Some(hits)
        });
        self._search = Some(cx.spawn(async move |this, cx| {
            let Some(hits) = task.await else { return };
            let _ = this.update(cx, |v, cx| {
                if v.search_generation.load(Ordering::Relaxed) != generation {
                    return;
                }
                v.searching = false;
                v.hits = Arc::new(hits);
                if !v.hits.is_empty() {
                    let nth = v.wanted_hit.take().unwrap_or_else(|| v.first_hit_from_selection());
                    v.select_hit(nth.min(v.hits.len() - 1), cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn first_hit_from_selection(&self) -> usize {
        let here = self.selection.anchor;
        self.hits.iter().position(|h| *h >= here).unwrap_or(0)
    }

    fn select_hit(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(cell) = self.hits.get(index).copied() else { return };
        self.current_hit = Some(index);
        self.selection = Selection::at(cell);
        self.reveal(cell);
        cx.notify();
    }

    fn step_hit(&mut self, delta: i64, cx: &mut Context<Self>) {
        let n = self.hits.len() as i64;
        if n == 0 {
            return;
        }
        let next = self.current_hit.map_or(if delta > 0 { 0 } else { n - 1 }, |i| (i as i64 + delta).rem_euclid(n));
        self.select_hit(next as usize, cx);
    }

    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.selection = self.before_drag;
            cx.notify();
        } else if self.search_open {
            self.close_search(window, cx);
        } else if self.selection.anchor != self.selection.head {
            self.selection = Selection::at(self.selection.anchor);
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let m = &k.modifiers;
        let command = if cfg!(target_os = "macos") { m.platform } else { m.control };
        let typing = window.context_stack().iter().any(|c| c.contains("Input"));
        let shift = m.shift;
        let (last_row, last_col) = self.last_cell();
        match k.key.as_str() {
            "f" if command && !shift => self.open_search(window, cx),
            "g" if command => self.step_hit(if shift { -1 } else { 1 }, cx),
            _ if typing => return,
            "=" | "+" if command => self.zoom_by(1.25, None, cx),
            "-" if command => self.zoom_by(0.8, None, cx),
            "0" if command => self.zoom_to(1., None, cx),
            "up" if command => self.move_to((0, self.selection.head.1), shift, cx),
            "down" if command => self.move_to((last_row, self.selection.head.1), shift, cx),
            "left" if command => self.move_to((self.selection.head.0, 0), shift, cx),
            "right" if command => self.move_to((self.selection.head.0, last_col), shift, cx),
            "home" if command => {
                self.move_to((0, 0), shift, cx);
                self.scroll.set_offset(point(px(0.), px(0.)));
            }
            "pageup" if command => self.switch_to(self.active.saturating_sub(1), window, cx),
            "pagedown" if command => self.switch_to((self.active + 1).min(self.names.len().saturating_sub(1)), window, cx),
            "end" if command => self.move_to((last_row, last_col), shift, cx),
            "home" => self.move_to((self.selection.head.0, 0), shift, cx),
            "up" => self.move_by(-1, 0, shift, cx),
            "down" => self.move_by(1, 0, shift, cx),
            "left" => self.move_by(0, -1, shift, cx),
            "right" => self.move_by(0, 1, shift, cx),
            "pageup" => self.move_by(-self.page_rows(), 0, shift, cx),
            "pagedown" => self.move_by(self.page_rows(), 0, shift, cx),
            "tab" => self.move_by(0, if shift { -1 } else { 1 }, false, cx),
            "enter" => self.move_by(if shift { -1 } else { 1 }, 0, false, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn address(&self) -> String {
        let (r0, c0, r1, c1) = self.selection.range();
        match self.whole {
            Some((Drag::Rows, s)) if s == self.selection => return format!("{}:{}", r0 + 1, r1 + 1),
            Some((Drag::Columns, s)) if s == self.selection => return format!("{}:{}", column_name(c0), column_name(c1)),
            _ => {}
        }
        let a = format!("{}{}", column_name(c0), r0 + 1);
        if (r0, c0) == (r1, c1) || self.sheet().and_then(|s| s.merge_at(r0, c0)).is_some_and(|m| (m.r0, m.c0, m.r1, m.c1) == (r0, c0, r1, c1)) { a } else { format!("{a}:{}{}", column_name(c1), r1 + 1) }
    }

    fn toolbar(&self, cx: &mut Context<Self>, t: &Theme) -> Div {
        let me = cx.entity();
        let (m1, m2, m3, m4, m5, m6) = (me.clone(), me.clone(), me.clone(), me.clone(), me.clone(), me);
        let cell = self.sheet().and_then(|s| s.cell(self.selection.anchor.0, self.selection.anchor.1).cloned());
        let content = cell.map(|c| c.formula.map(|f| f.to_string()).unwrap_or(c.text)).unwrap_or_default();
        let content = content.lines().next().unwrap_or("").chars().take(400).collect::<String>();
        let address = div()
            .flex_none()
            .min_w(px(space::XXL * 2.))
            .px(px(space::S))
            .py(px(space::XXS))
            .rounded(px(radius::INNER))
            .bg(t.ink.opacity(0.06))
            .font_features(tabular())
            .text_size(px(font::CALLOUT))
            .font_weight(FontWeight::MEDIUM)
            .child(self.address());
        let search: AnyElement = match (&self.search_input, self.search_open) {
            (Some(input), true) => {
                let count = if self.searching {
                    tr("Suche …")
                } else if self.hits.is_empty() {
                    if input.read(cx).value().trim().is_empty() { String::new() } else { tr("Keine Treffer") }
                } else {
                    let shown = self.current_hit.map_or(0, |i| i + 1);
                    trf("%lld von %lld", &[&shown, &self.hits.len()])
                };
                let many = if self.hits.len() >= HIT_LIMIT { "+" } else { "" };
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(space::XS))
                    .child(div().w(px(200.)).child(Input::new(input).small().cleanable(false)))
                    .child(div().min_w(px(space::XXL * 2.)).text_size(px(font::CAPTION)).text_color(t.secondary).font_features(tabular()).child(format!("{count}{many}")))
                    .child(Button::new("sheet-prev").ghost().small().icon(menu_icon("chevron-up")).tooltip(tr("Vorheriger Treffer")).disabled(self.hits.is_empty()).on_click(move |_, _, cx| m1.update(cx, |v, cx| v.step_hit(-1, cx))))
                    .child(Button::new("sheet-next").ghost().small().icon(menu_icon("chevron-down")).tooltip(tr("Nächster Treffer")).disabled(self.hits.is_empty()).on_click(move |_, _, cx| m2.update(cx, |v, cx| v.step_hit(1, cx))))
                    .child(Button::new("sheet-search-close").ghost().small().icon(menu_icon("x")).tooltip(tr("Suche schließen")).on_click(move |_, window, cx| m3.update(cx, |v, cx| v.close_search(window, cx))))
                    .into_any_element()
            }
            _ => Button::new("sheet-search").ghost().small().icon(menu_icon("search")).tooltip(tr("Suche")).on_click(move |_, window, cx| m3.update(cx, |v, cx| v.open_search(window, cx))).into_any_element(),
        };
        let zoom = format!("{} %", (self.zoom * 100.).round());
        div()
            .h(px(control::SUBBAR))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::S))
            .border_b_1()
            .border_color(t.separator)
            .child(address)
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(font::CALLOUT)).text_color(if content.starts_with('=') { t.secondary } else { t.primary }).child(content))
            .child(div().flex_none().text_size(px(font::CAPTION)).text_color(t.secondary).child(tr("Nur Lesen")))
            .child(search)
            .child(Button::new("sheet-out").ghost().small().icon(menu_icon("zoom-out")).tooltip(tr("Verkleinern")).disabled(self.zoom <= MIN_ZOOM).on_click(move |_, _, cx| m4.update(cx, |v, cx| v.zoom_by(0.8, None, cx))))
            .child(Button::new("sheet-zoom").ghost().small().label(zoom).tooltip(tr("Zoom")).dropdown_menu(move |mut menu, _, _| {
                for z in ZOOMS {
                    let v = m5.clone();
                    menu = menu.item(PopupMenuItem::new(format!("{} %", (z * 100.).round())).on_click(move |_, _, cx| v.update(cx, |v, cx| v.zoom_to(z, None, cx))));
                }
                menu
            }))
            .child(Button::new("sheet-in").ghost().small().icon(menu_icon("zoom-in")).tooltip(tr("Vergrößern")).disabled(self.zoom >= MAX_ZOOM).on_click(move |_, _, cx| m6.update(cx, |v, cx| v.zoom_by(1.25, None, cx))))
    }

    fn tab_bar(&self, cx: &mut Context<Self>, t: &Theme) -> Option<Div> {
        if self.names.len() < 2 {
            return None;
        }
        let me = cx.entity();
        let mut row = div().id("sheet-tabs").flex().items_center().gap(px(space::XXS)).px(px(space::S)).overflow_x_scroll().track_scroll(&self.tabs);
        for (i, name) in self.names.iter().enumerate() {
            let on = i == self.active;
            let v = me.clone();
            row = row.child(
                div()
                    .id(("sheet-tab", i))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(space::XS))
                    .h(px(control::S))
                    .px(px(space::M))
                    .rounded(px(radius::INNER))
                    .cursor_pointer()
                    .key_nav()
                    .max_w(px(TAB_WIDTH))
                    .when(!on, |d| d.soft(t.ink.opacity(0.06), radius::INNER))
                    .tooltip(tip(name.clone()))
                    .when(on, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm())
                    .text_size(px(font::CALLOUT))
                    .text_color(if on { t.primary } else { t.secondary })
                    .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name.clone()))
                    .on_click(move |_, window, cx| v.update(cx, |v, cx| v.switch_to(i, window, cx))),
            );
        }
        Some(div().flex_none().h(px(control::S + 2. * space::XS)).flex().items_center().border_t_1().border_color(t.separator).bg(t.ink.opacity(0.03)).child(row))
    }

    fn grid(&mut self, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> AnyElement {
        let me = cx.entity();
        let Some(s) = self.sheet() else {
            if self.broken.get(self.active).copied().unwrap_or(false) {
                return div().flex_1().min_h_0().child(unopenable(&self.path, false, tr("Dieses Blatt konnte nicht gelesen werden"), tr("Die anderen Blätter bleiben verfügbar."), t)).into_any_element();
            }
            return div().flex_1().min_h_0().flex().items_center().justify_center().child(motion::loading(iconsize::L, t.secondary)).into_any_element();
        };
        let g = self.geo(&s);
        let (content_w, content_h) = g.content(&s);
        let empty = s.rows.is_empty() && s.merges.is_empty();
        let fonts = self.fonts.get(self.active).cloned().unwrap_or_default();

        let paint = Paint {
            sheet: s,
            fonts,
            zoom: self.zoom,
            scroll: self.scroll.clone(),
            selection: self.selection,
            hits: self.hits.clone(),
            current_hit: self.current_hit.and_then(|i| self.hits.get(i).copied()),
            base: window.text_style().font(),
            paper: if t.dark { t.content } else { hex(PAPER) },
            grid: if t.dark { t.ink.opacity(0.12) } else { hex(GRID) },
            frozen: if t.dark { t.ink.opacity(0.3) } else { hex(FROZEN_LINE) },
            ink: if t.dark { t.primary } else { hex(INK) },
            header_bg: if t.dark { t.content } else { t.secondary_bg },
            header_line: t.separator,
            header_text: t.secondary,
            accent: t.accent,
            hit: t.palette("yellow").opacity(0.5),
            current: t.orange.opacity(0.65),
        };
        let measure = cx.entity();
        div()
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .bg(if t.dark { t.content } else { hex(PAPER) })
            .child(
                canvas(
                    move |bounds, _, cx| {
                        measure.update(cx, |v, cx| {
                            if v.view != bounds {
                                v.view = bounds;
                                cx.notify();
                            }
                        })
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                div()
                    .id("sheet-grid")
                    .size_full()
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .cursor(CursorStyle::Arrow)
                    .on_scroll(cx.listener(|_, _, _, cx| cx.notify()))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, _| v.drag = None))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(|v, _, _, _| v.drag = None))
                    .child(div().w(px(content_w)).h(px(content_h))),
            )
            .child(canvas(|_, _, _| {}, move |bounds, _, window, cx| window.with_content_mask(Some(ContentMask { bounds }), |window| paint.paint(bounds, window, cx))).absolute().top_0().left_0().size_full())
            .when(empty, |d| d.child(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().text_size(px(font::BODY)).text_color(if t.dark { t.secondary } else { hex(MUTED) }).child(tr("Dieses Blatt ist leer"))))
            .scrollbar(&self.scroll, gpui_kit::component::scroll::ScrollbarAxis::Both)
            .child(super::image_view::wheel_zoom(move |factor, at, _, cx| {
                me.update(cx, |v, cx| {
                    let local = ((f32::from(at.x - v.view.origin.x)), f32::from(at.y - v.view.origin.y));
                    v.zoom_by(factor, Some(local), cx);
                })
            }))
            .into_any_element()
    }
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

fn installed(name: &str, cx: &App) -> bool {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| cx.text_system().all_font_names().into_iter().map(|n| n.to_lowercase()).collect()).contains(&name.to_lowercase())
}

fn cell_font(style: &Style, cx: &App) -> Font {
    let mut f = match style.font.as_deref().filter(|f| installed(f, cx)) {
        Some(name) => font(SharedString::from(name.to_string())),
        None => font(Theme::font_family()),
    };
    if style.bold {
        f.weight = FontWeight::BOLD;
    }
    if style.italic {
        f.style = FontStyle::Italic;
    }
    f
}

/// Everything the grid paints in one frame, read at paint time so scrolling needs no new layout.
struct Paint {
    sheet: Arc<Sheet>,
    fonts: Arc<Vec<Font>>,
    zoom: f32,
    scroll: ScrollHandle,
    selection: Selection,
    hits: Arc<Vec<(u32, u32)>>,
    current_hit: Option<(u32, u32)>,
    base: Font,
    paper: Hsla,
    grid: Hsla,
    frozen: Hsla,
    ink: Hsla,
    header_bg: Hsla,
    header_line: Hsla,
    header_text: Hsla,
    accent: Hsla,
    hit: Hsla,
    current: Hsla,
}

fn quad(window: &mut Window, origin: Point<Pixels>, x: f32, y: f32, w: f32, h: f32, color: impl Into<Background>) {
    if w <= 0. || h <= 0. {
        return;
    }
    window.paint_quad(fill(Bounds::new(point(origin.x + px(x), origin.y + px(y)), size(px(w), px(h))), color));
}

fn hex(c: u32) -> Hsla {
    Theme::document(c)
}

impl Paint {
    fn font_of(&self, style: u32) -> Font {
        self.fonts.get(style as usize).cloned().unwrap_or_else(|| self.base.clone())
    }

    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let s = &*self.sheet;
        let g = Geo::new(s, self.zoom, self.scroll.offset(), (f32::from(bounds.size.width), f32::from(bounds.size.height)));
        let o = bounds.origin;
        let rows = g.scroll_rows(s);
        let cols = g.scroll_cols(s);
        let (fr, fc) = (g.frozen_rows, g.frozen_cols);
        let areas = [
            (rows.clone(), cols.clone(), (g.header_w + g.frozen_w, g.header_h + g.frozen_h, g.w, g.h)),
            (rows.clone(), 0..fc, (g.header_w, g.header_h + g.frozen_h, g.header_w + g.frozen_w, g.h)),
            (0..fr, cols.clone(), (g.header_w + g.frozen_w, g.header_h, g.w, g.header_h + g.frozen_h)),
            (0..fr, 0..fc, (g.header_w, g.header_h, g.header_w + g.frozen_w, g.header_h + g.frozen_h)),
        ];
        for (rows, cols, (x0, y0, x1, y1)) in areas {
            if rows.is_empty() || cols.is_empty() || x1 <= x0 || y1 <= y0 {
                continue;
            }
            let clip = Bounds::new(point(o.x + px(x0), o.y + px(y0)), size(px(x1 - x0), px(y1 - y0)));
            window.with_content_mask(Some(ContentMask { bounds: clip }), |window| self.paint_area(&g, o, rows, cols, clip, window, cx));
        }
        if fc > 0 {
            quad(window, o, g.header_w + g.frozen_w - 1., g.header_h, 1., g.h - g.header_h, self.frozen);
        }
        if fr > 0 {
            quad(window, o, g.header_w, g.header_h + g.frozen_h - 1., g.w - g.header_w, 1., self.frozen);
        }
        self.paint_headers(&g, o, rows, cols, window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_area(&self, g: &Geo, o: Point<Pixels>, rows: Range<u32>, cols: Range<u32>, clip: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let s = &*self.sheet;
        let z = g.zoom;
        let x_of = |c: u32| g.col_x(s, c);
        let y_of = |r: u32| g.row_y(s, r);
        let (left, top) = (x_of(cols.start), y_of(rows.start));
        let (right, bottom) = (g.col_end(s, cols.end - 1), g.row_end(s, rows.end - 1));
        if s.grid_lines {
            for c in cols.start..cols.end {
                if c == cols.start || s.cols.size(c - 1) > 0. {
                    quad(window, o, x_of(c), top, 1., bottom - top, self.grid);
                }
            }
            quad(window, o, right, top, 1., bottom - top, self.grid);
            for r in rows.start..rows.end {
                quad(window, o, left, y_of(r), right - left + 1., 1., self.grid);
            }
            quad(window, o, left, bottom, right - left + 1., 1., self.grid);
        }
        let merges: Vec<Merge> = s.merges.iter().copied().filter(|m| m.r1 >= rows.start && m.r0 < rows.end && m.c1 >= cols.start && m.c0 < cols.end).collect();
        let in_merge = |r: u32, c: u32| merges.iter().any(|m| (m.r0..=m.r1).contains(&r) && (m.c0..=m.c1).contains(&c));
        let rect = |r0: u32, c0: u32, r1: u32, c1: u32| (x_of(c0), y_of(r0), g.col_end(s, c1) - x_of(c0), g.row_end(s, r1) - y_of(r0));
        let visible_rows = s.rows_from(rows.start);
        let visible_rows = &visible_rows[..visible_rows.partition_point(|r| r.index < rows.end)];
        for m in &merges {
            let (x, y, w, h) = rect(m.r0, m.c0, m.r1, m.c1);
            let fill = s.cell(m.r0, m.c0).and_then(|c| s.style(c).fill);
            match fill {
                Some(f) => quad(window, o, x, y, w + 1., h + 1., hex(f)),
                None => quad(window, o, x + 1., y + 1., w - 1., h - 1., self.paper),
            }
        }
        let cells_of = |row: &'_ crate::core::office::sheet::Row| -> Range<usize> {
            let a = row.cells.partition_point(|c| c.col < cols.start);
            let b = row.cells.partition_point(|c| c.col < cols.end);
            a..b
        };
        for row in visible_rows {
            let (y, h) = (y_of(row.index), s.heights.size(row.index) * z);
            if h <= 0. {
                continue;
            }
            for cell in &row.cells[cells_of(row)] {
                if let Some(f) = s.style(cell).fill
                    && !in_merge(row.index, cell.col)
                {
                    let w = s.cols.size(cell.col) * z;
                    quad(window, o, x_of(cell.col), y, w + 1., h + 1., hex(f));
                }
            }
        }
        for &(r, c) in self.hits.iter().filter(|(r, c)| rows.contains(r) && cols.contains(c)) {
            let (x, y, w, h) = rect(r, c, r, c);
            quad(window, o, x + 1., y + 1., w - 1., h - 1., if self.current_hit == Some((r, c)) { self.current } else { self.hit });
        }
        for row in visible_rows {
            let y = y_of(row.index);
            for cell in &row.cells[cells_of(row)] {
                let st = s.style(cell);
                if st.borders.iter().all(Option::is_none) {
                    continue;
                }
                let (x, y, w, h) = match merges.iter().find(|m| m.r0 == row.index && m.c0 == cell.col) {
                    Some(m) => rect(m.r0, m.c0, m.r1, m.c1),
                    None => (x_of(cell.col), y, s.cols.size(cell.col) * z, s.heights.size(row.index) * z),
                };
                for (i, b) in st.borders.iter().enumerate() {
                    let Some(b) = b else { continue };
                    let bw = (b.width * z.min(1.5)).max(1.);
                    let half = ((bw - 1.) / 2.).floor();
                    match i {
                        0 => quad(window, o, x - half, y - half, bw, h + bw, hex(b.color)),
                        1 => quad(window, o, x - half, y - half, w + bw, bw, hex(b.color)),
                        2 => quad(window, o, x + w - half, y - half, bw, h + bw, hex(b.color)),
                        _ => quad(window, o, x - half, y + h - half, w + bw, bw, hex(b.color)),
                    }
                }
            }
        }
        for row in visible_rows {
            let range = cells_of(row);
            let start = range.start;
            let lead = start.checked_sub(1).and_then(|i| row.cells[..=i].iter().rposition(|c| c.kind != CellKind::Empty)).filter(|&i| row.cells[i].kind == CellKind::Text);
            for i in lead.into_iter().chain(range) {
                let cell = &row.cells[i];
                if cell.text.is_empty() {
                    continue;
                }
                let merge = merges.iter().find(|m| m.r0 == row.index && m.c0 == cell.col).copied();
                if merge.is_none() && in_merge(row.index, cell.col) {
                    continue;
                }
                self.paint_text(g, o, s, row, i, merge, clip, window, cx);
            }
        }
        for m in s.merges.iter().filter(|m| (m.r0 < rows.start || m.c0 < cols.start) && m.r1 >= rows.start && m.c1 >= cols.start && m.r0 < rows.end && m.c0 < cols.end) {
            if let Some(row) = s.row(m.r0)
                && let Ok(i) = row.cells.binary_search_by_key(&m.c0, |c| c.col)
                && !row.cells[i].text.is_empty()
            {
                self.paint_text(g, o, s, row, i, Some(*m), clip, window, cx);
            }
        }
        self.paint_selection(g, o, &rows, &cols, window);
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_text(&self, g: &Geo, o: Point<Pixels>, s: &Sheet, row: &crate::core::office::sheet::Row, i: usize, merge: Option<Merge>, clip: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let z = g.zoom;
        let cell: &Cell = &row.cells[i];
        let st = s.style(cell);
        let (r1, c1) = merge.map_or((row.index, cell.col), |m| (m.r1, m.c1));
        let (x, y) = (g.col_x(s, cell.col), g.row_y(s, row.index));
        let (w, h) = (g.col_end(s, c1) - x, g.row_end(s, r1) - y);
        if w <= 0. || h <= 0. {
            return;
        }
        let size = st.size * 4. / 3. * z;
        if size < 3. {
            return;
        }
        let line_h = (size * 1.25).round().max(1.);
        let color = cell.color.or(st.color).map_or(if st.fill.is_some() { hex(INK) } else { self.ink }, hex);
        let align = match st.h {
            HAlign::General => match cell.kind {
                CellKind::Number => HAlign::Right,
                CellKind::Bool | CellKind::Error => HAlign::Center,
                _ => HAlign::Left,
            },
            a => a,
        };
        let pad = CELL_PADDING * z + st.indent as f32 * 9. * z;
        let font = self.font_of(cell.style);
        let run = |len: usize| TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: st.underline.then(|| UnderlineStyle { thickness: px(1.), color: Some(color), wavy: false }),
            strikethrough: st.strike.then(|| StrikethroughStyle { thickness: px(1.), color: Some(color) }),
        };
        let cell_clip = Bounds::new(point(o.x + px(x + 1.), o.y + px(y + 1.)), size_of(w - 1., h - 1.));
        let wraps = st.wrap || cell.text.contains('\n') || st.h == HAlign::Justify;
        if wraps {
            let text: SharedString = cell.text.replace("\r\n", "\n").into();
            let Ok(lines) = window.text_system().shape_text(text.clone(), px(size), &[run(text.len())], Some(px((w - 2. * pad).max(1.))), None) else { return };
            let total: f32 = lines.iter().map(|l| (l.wrap_boundaries().len() + 1) as f32 * line_h).sum();
            let top = match st.v {
                VAlign::Top => y + 1.,
                VAlign::Center => y + (h - total) / 2.,
                VAlign::Bottom => y + h - total - 1.,
            };
            let text_align = match align {
                HAlign::Center => TextAlign::Center,
                HAlign::Right => TextAlign::Right,
                _ => TextAlign::Left,
            };
            window.with_content_mask(Some(ContentMask { bounds: cell_clip.intersect(&clip) }), |window| {
                let mut at = top;
                for line in lines {
                    let n = (line.wrap_boundaries().len() + 1) as f32;
                    let origin = point(o.x + px(x + pad), o.y + px(at));
                    let _ = line.paint(origin, px(line_h), text_align, Some(Bounds::new(origin, size_of(w - 2. * pad, n * line_h))), window, cx);
                    at += n * line_h;
                }
            });
            return;
        }
        let room = w - 2. * pad;
        let shape = |text: SharedString, window: &mut Window| {
            let len = text.len();
            window.text_system().shape_line(text, px(size), &[run(len)], None)
        };
        let shaped = if cell.kind == CellKind::Number && merge.is_none() {
            let digit = f32::from(shape("0".into(), window).width).max(0.1);
            let guess = |t: &str| t.chars().count() as f32 * digit;
            let full = guess(&cell.text);
            let mut shown = None;
            if full <= room * 0.85 || full <= room * 1.15 && f32::from(shape(cell.text.clone().into(), window).width) <= room {
                shown = Some(shape(cell.text.clone().into(), window));
            } else if cell.general {
                for shorter in rounded(&cell.text).into_iter().filter(|t| guess(t) <= room * 1.15) {
                    let line = shape(shorter.into(), window);
                    if f32::from(line.width) <= room {
                        shown = Some(line);
                        break;
                    }
                }
            }
            shown.unwrap_or_else(|| {
                let hash = f32::from(shape("#".into(), window).width).max(1.);
                shape("#".repeat((room / hash).floor().max(1.) as usize).into(), window)
            })
        } else {
            shape(cell.text.clone().into(), window)
        };
        let tw = f32::from(shaped.width);
        let tx = match align {
            HAlign::Center => x + (w - tw) / 2.,
            HAlign::Right => x + w - pad - tw,
            _ => x + pad,
        };
        let ty = match st.v {
            VAlign::Top => y + 1.,
            VAlign::Center => y + (h - line_h) / 2.,
            VAlign::Bottom => y + h - line_h,
        };
        let (mut cl, mut cr) = (x + 1., x + w);
        if merge.is_none() && cell.kind == CellKind::Text && tw > room {
            let free = |c: &Cell| c.kind == CellKind::Empty || c.text.is_empty();
            if matches!(align, HAlign::Left | HAlign::Center) {
                let next = row.cells[i + 1..].iter().find(|c| !free(c)).map_or(u32::MAX, |c| c.col);
                let mut c = cell.col + 1;
                while c < next && g.col_x(s, c) < tx + tw && c < cell.col + 256 {
                    c += 1;
                }
                cr = g.col_x(s, c);
            }
            if matches!(align, HAlign::Right | HAlign::Center) {
                let prev = row.cells[..i].iter().rev().find(|c| !free(c)).map_or(0, |c| c.col + 1);
                let mut c = cell.col;
                while c > prev && g.col_x(s, c) > tx && cell.col - c < 256 {
                    c -= 1;
                }
                cl = g.col_x(s, c) + 1.;
            }
            if cr - cl > w {
                quad(window, o, cl, ty.max(y + 1.), cr - cl - 1., line_h.min(h - 1.), self.paper);
                let fill = st.fill;
                if let Some(f) = fill {
                    quad(window, o, x, y, w + 1., h + 1., hex(f));
                }
            }
        }
        let mask = Bounds::new(point(o.x + px(cl), o.y + px(y + 1.)), size_of(cr - cl, h - 1.)).intersect(&clip);
        window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
            let _ = shaped.paint(point(o.x + px(tx), o.y + px(ty)), px(line_h), TextAlign::Left, None, window, cx);
        });
    }

    fn paint_selection(&self, g: &Geo, o: Point<Pixels>, rows: &Range<u32>, cols: &Range<u32>, window: &mut Window) {
        let s = &*self.sheet;
        let (r0, c0, r1, c1) = self.selection.range();
        if r1 < rows.start || r0 >= rows.end || c1 < cols.start || c0 >= cols.end {
            return;
        }
        let (x, y) = (g.col_x(s, c0), g.row_y(s, r0));
        let (w, h) = (g.col_end(s, c1) - x, g.row_end(s, r1) - y);
        let anchor = self.selection.anchor;
        let single = (r0, c0, r1, c1) == (anchor.0, anchor.1, anchor.0, anchor.1) || s.merge_at(anchor.0, anchor.1).is_some_and(|m| (m.r0, m.c0, m.r1, m.c1) == (r0, c0, r1, c1));
        if !single {
            quad(window, o, x, y, w, h, self.accent.opacity(TINT));
        }
        let b = 2.;
        let edge = self.accent;
        quad(window, o, x - 1., y - 1., w + b, b, edge);
        quad(window, o, x - 1., y + h - 1., w + b, b, edge);
        quad(window, o, x - 1., y - 1., b, h + b, edge);
        quad(window, o, x + w - 1., y - 1., b, h + b, edge);
    }

    fn paint_headers(&self, g: &Geo, o: Point<Pixels>, rows: Range<u32>, cols: Range<u32>, window: &mut Window, cx: &mut App) {
        let s = &*self.sheet;
        let (r0, c0, r1, c1) = self.selection.range();
        quad(window, o, 0., 0., g.w, g.header_h, self.header_bg);
        quad(window, o, 0., 0., g.header_w, g.h, self.header_bg);
        let mut font = self.base.clone();
        let label = |window: &mut Window, cx: &mut App, text: String, cx0: f32, cy0: f32, w: f32, h: f32, on: bool, font: &Font| {
            let text: SharedString = text.into();
            let mut f = font.clone();
            if on {
                f.weight = FontWeight::SEMIBOLD;
            }
            let color = if on { self.accent } else { self.header_text };
            let run = TextRun { len: text.len(), font: f, color, background_color: None, underline: None, strikethrough: None };
            let shaped = window.text_system().shape_line(text, px(font::CAPTION), &[run], None);
            let tw = f32::from(shaped.width);
            let line = font::CAPTION * 1.3;
            let at = point(o.x + px(cx0 + (w - tw) / 2.), o.y + px(cy0 + (h - line) / 2.));
            let mask = Bounds::new(point(o.x + px(cx0), o.y + px(cy0)), size_of(w, h));
            window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
                let _ = shaped.paint(at, px(line), TextAlign::Left, None, window, cx);
            });
        };
        font.features = tabular();
        let col_strips = [(0..g.frozen_cols, (g.header_w, g.header_w + g.frozen_w)), (cols, (g.header_w + g.frozen_w, g.w))];
        for (range, (x0, x1)) in col_strips {
            if range.is_empty() || x1 <= x0 {
                continue;
            }
            let clip = Bounds::new(point(o.x + px(x0), o.y), size_of(x1 - x0, g.header_h));
            window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                for c in range {
                    let (x, w) = (g.col_x(s, c), s.cols.size(c) * g.zoom);
                    if w <= 0. {
                        continue;
                    }
                    let on = c >= c0 && c <= c1;
                    if on {
                        quad(window, o, x, 0., w, g.header_h, self.accent.opacity(TINT));
                        quad(window, o, x, g.header_h - 2., w, 2., self.accent);
                    }
                    quad(window, o, x, 0., 1., g.header_h, self.header_line);
                    if w > 8. {
                        label(window, cx, column_name(c), x, 0., w, g.header_h, on, &font);
                    }
                }
            });
        }
        let row_strips = [(0..g.frozen_rows, (g.header_h, g.header_h + g.frozen_h)), (rows, (g.header_h + g.frozen_h, g.h))];
        for (range, (y0, y1)) in row_strips {
            if range.is_empty() || y1 <= y0 {
                continue;
            }
            let clip = Bounds::new(point(o.x, o.y + px(y0)), size_of(g.header_w, y1 - y0));
            window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                for r in range {
                    let (y, h) = (g.row_y(s, r), s.heights.size(r) * g.zoom);
                    if h <= 0. {
                        continue;
                    }
                    let on = r >= r0 && r <= r1;
                    if on {
                        quad(window, o, 0., y, g.header_w, h, self.accent.opacity(TINT));
                        quad(window, o, g.header_w - 2., y, 2., h, self.accent);
                    }
                    quad(window, o, 0., y, g.header_w, 1., self.header_line);
                    if h > 6. {
                        label(window, cx, (r + 1).to_string(), 0., y, g.header_w, h, on, &font);
                    }
                }
            });
        }
        quad(window, o, 0., 0., g.header_w, g.header_h, self.header_bg);
        quad(window, o, 0., g.header_h - 1., g.w, 1., self.header_line);
        quad(window, o, g.header_w - 1., 0., 1., g.h, self.header_line);
    }
}

/// Shorter forms of a plain decimal, one digit less each, as Excel shows General numbers in narrow columns.
fn rounded(text: &str) -> Vec<String> {
    let Some(sep) = text.find(['.', ',']) else { return Vec::new() };
    let digits = text.len() - sep - 1;
    let plain = text.bytes().enumerate().all(|(i, b)| b.is_ascii_digit() || i == sep || (i == 0 && b == b'-'));
    let Ok(value) = text.replace(',', ".").parse::<f64>() else { return Vec::new() };
    if !plain || digits == 0 {
        return Vec::new();
    }
    let mark = &text[sep..=sep];
    let mut forms: Vec<String> = (0..digits)
        .rev()
        .map(|d| {
            let scale = 10f64.powi(d as i32);
            let s = format!("{:.d$}", (value * scale).round() / scale);
            let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
            s.replace('.', mark)
        })
        .collect();
    forms.dedup();
    forms
}

fn size_of(w: f32, h: f32) -> Size<Pixels> {
    size(px(w.max(0.)), px(h.max(0.)))
}

impl Render for SheetView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        if let Some(why) = self.failed {
            let (title, body) = why.describe();
            return unopenable(&self.path, why == OpenError::Locked, title, body, &t).into_any_element();
        }
        if !self.opened {
            return div().size_full().flex().items_center().justify_center().child(motion::loading(iconsize::L, t.secondary)).into_any_element();
        }
        if self.sheet().is_some() && !std::mem::replace(&mut self.focused_once, true) {
            window.focus(&self.focus, cx);
        }
        let grid = self.grid(window, cx, &t);
        div()
            .id("sheet")
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .key_context("SheetView")
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(Self::on_escape))
            .on_action(cx.listener(|v, _: &super::find_bar::FindInNote, window, cx| v.open_search(window, cx)))
            .on_action(cx.listener(|v, _: &gpui_kit::component::input::Copy, window, cx| {
                if window.context_stack().iter().any(|c| c.contains("Input")) {
                    cx.propagate();
                } else {
                    v.copy(cx);
                }
            }))
            .on_action(cx.listener(|v, _: &gpui_kit::component::input::SelectAll, window, cx| {
                if window.context_stack().iter().any(|c| c.contains("Input")) {
                    cx.propagate();
                } else {
                    v.select_all(cx);
                }
            }))
            .child(self.toolbar(cx, &t))
            .child(grid)
            .children(self.tab_bar(cx, &t))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Selection, rounded};

    #[test]
    fn narrow_general_numbers_lose_digits_not_meaning() {
        assert_eq!(rounded("95.7500761"), ["95.750076", "95.75008", "95.7501", "95.75", "95.8", "96"]);
        assert_eq!(rounded("-1,25"), ["-1,3", "-1"]);
        assert!(rounded("1.234,50 €").is_empty());
        assert!(rounded("42").is_empty());
    }

    #[test]
    fn selections_span_both_directions() {
        let s = Selection { anchor: (5, 3), head: (2, 7) };
        assert_eq!(s.range(), (2, 3, 5, 7));
    }
}
