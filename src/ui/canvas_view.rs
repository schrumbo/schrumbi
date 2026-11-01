use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::browser::DraggedFile;
use super::state::act;
use super::theme::{Theme, font, radius, space};
use super::widgets::*;
use crate::core::i18n::tr;
use crate::core::note_buffer::write_atomic;
use crate::core::vault::canvas::{Canvas, Edge, Node, NodeKind};
use crate::core::vault::index;

const HANDLE: f32 = 8.;
const SNAP: f64 = 6.;
const MIN_SIZE: f64 = 40.;
const SAVE_DELAY: Duration = Duration::from_millis(400);
const COALESCE: Duration = Duration::from_millis(800);
const UNDO_LIMIT: usize = 200;

/// The field a card is edited in; only text cards have several lines.
#[derive(Clone)]
enum Field {
    Line(Entity<InputState>),
    Text(Entity<TextareaState>),
}

/// What the pointer is doing at the moment.
enum Gesture {
    Pan { from: Point<Pixels>, pan: (f32, f32) },
    Move { from: (f64, f64), starts: Vec<(String, f64, f64)>, moved: bool },
    Resize { id: String, handle: (i8, i8), from: (f64, f64), start: (f64, f64, f64, f64) },
    Edge { from: String, side: &'static str, to: (f64, f64) },
    Marquee { from: (f64, f64), to: (f64, f64), added: bool },
}

actions!(schrumbi_walk, [NextItem, PreviousItem]);

/// Obsidian's six preset card colors.
const PRESETS: [(&str, u32); 6] = [("1", 0xe93147), ("2", 0xec7500), ("3", 0xe0ac00), ("4", 0x08b94e), ("5", 0x00bfbc), ("6", 0x7852ee)];

fn color_of(node_color: Option<&String>) -> Option<Hsla> {
    let c = node_color?;
    if let Some((_, rgb_value)) = PRESETS.iter().find(|(k, _)| k == c) {
        return Some(rgb(*rgb_value).into());
    }
    let hex = c.trim_start_matches('#');
    u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 6).map(|v| rgb(v).into())
}

/// A `.canvas` file as its own document. Coordinates are stored as in the file, the view only pans and zooms.
pub struct CanvasView {
    path: PathBuf,
    doc: Option<Canvas>,
    source: String,
    pan: (f32, f32),
    zoom: f32,
    fitted: bool,
    selection: BTreeSet<String>,
    selected_edge: Option<String>,
    gesture: Option<Gesture>,
    guides: Vec<(bool, f64)>,
    edge_mode: bool,
    space_down: bool,
    editing: Option<(String, Field)>,
    bounds: Rc<std::cell::Cell<Bounds<Pixels>>>,
    undo: Vec<String>,
    redo: Vec<String>,
    last_edit: Option<std::time::Instant>,
    save_task: Option<Task<()>>,
    seen_generation: u64,
    /// Texts of note cards, read again when the vault changed.
    card_texts: std::cell::RefCell<std::collections::HashMap<PathBuf, (u64, String)>>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl CanvasView {
    pub fn new(path: PathBuf, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        let doc = Canvas::parse(&source);
        Self { path, doc, source, pan: (0., 0.), zoom: 1., fitted: false, selection: BTreeSet::new(), selected_edge: None, gesture: None, guides: Vec::new(), edge_mode: false, space_down: false, editing: None, bounds: Default::default(), undo: Vec::new(), redo: Vec::new(), last_edit: None, save_task: None, seen_generation: index::current().generation, card_texts: Default::default(), focus: cx.focus_handle(), _subs: Vec::new() }
    }

    /// Gives the keyboard to the canvas.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn reload_if_changed(&mut self) {
        let generation = index::current().generation;
        if self.save_task.is_some() || self.gesture.is_some() || std::mem::replace(&mut self.seen_generation, generation) == generation {
            return;
        }
        let Ok(text) = std::fs::read_to_string(&self.path) else { return };
        if text != self.source {
            self.doc = Canvas::parse(&text);
            self.source = text;
            self.selection.clear();
        }
    }

    /// Remembers the state before a change; quick repeats of the same gesture stay one undo step.
    fn before_edit(&mut self, merge: bool) {
        let Some(doc) = &self.doc else { return };
        let recent = self.last_edit.is_some_and(|at| at.elapsed() < COALESCE);
        if !(merge && recent) {
            self.undo.push(doc.to_json());
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = Some(std::time::Instant::now());
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = &self.doc else { return };
        self.source = doc.to_json();
        let (path, bytes) = (self.path.clone(), self.source.clone().into_bytes());
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let result = cx.background_spawn(async move { write_atomic(&path, &bytes) }).await;
            let _ = this.update(cx, |v, cx| {
                v.save_task = None;
                match result {
                    Ok(()) => super::state::reindex(cx),
                    Err(e) => act(cx, |l| l.last_error = Some(e.to_string())),
                }
            });
        }));
        cx.notify();
    }

    fn step_history(&mut self, back: bool, cx: &mut Context<Self>) {
        let (from, to) = if back { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some(state) = from.pop() else { return };
        if let Some(doc) = &self.doc {
            to.push(doc.to_json());
        }
        self.doc = Canvas::parse(&state);
        self.last_edit = None;
        let known: BTreeSet<String> = self.doc.as_ref().map_or_else(Default::default, |d| d.nodes.iter().map(|n| n.id.clone()).collect());
        self.selection.retain(|id| known.contains(id));
        self.changed(cx);
    }

    fn origin(&self) -> Point<Pixels> {
        self.bounds.get().origin
    }

    fn to_doc(&self, at: Point<Pixels>) -> (f64, f64) {
        let o = self.origin();
        (((f32::from(at.x - o.x) - self.pan.0) / self.zoom) as f64, ((f32::from(at.y - o.y) - self.pan.1) / self.zoom) as f64)
    }

    fn to_view(&self, x: f64, y: f64) -> (f32, f32) {
        (x as f32 * self.zoom + self.pan.0, y as f32 * self.zoom + self.pan.1)
    }

    fn fit_to(&mut self, area: Option<(f64, f64, f64, f64)>) {
        let Some((a, b, c, d)) = area else { return };
        let size = self.bounds.get().size;
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        if w < 10. {
            return;
        }
        self.zoom = ((w - 96.) / (c - a).max(1.) as f32).min((h - 96.) / (d - b).max(1.) as f32).clamp(0.05, 1.5);
        self.pan = (w / 2. - (a + c) as f32 / 2. * self.zoom, h / 2. - (b + d) as f32 / 2. * self.zoom);
        self.fitted = true;
    }

    fn fit_all(&mut self) {
        let area = self.doc.as_ref().and_then(Canvas::bounds);
        self.fit_to(area);
    }

    fn fit_selection(&mut self) {
        let Some(doc) = &self.doc else { return };
        let area = doc.nodes.iter().filter(|n| self.selection.contains(&n.id)).fold(None::<(f64, f64, f64, f64)>, |acc, n| {
            let (x1, y1) = (n.x + n.width, n.y + n.height);
            Some(acc.map_or((n.x, n.y, x1, y1), |(a, b, c, d)| (a.min(n.x), b.min(n.y), c.max(x1), d.max(y1))))
        });
        self.fit_to(area);
    }

    fn zoom_at(&mut self, factor: f32, x: f32, y: f32) {
        let next = (self.zoom * factor).clamp(0.05, 4.);
        let k = next / self.zoom;
        self.pan = (x - (x - self.pan.0) * k, y - (y - self.pan.1) * k);
        self.zoom = next;
    }

    fn zoom_center(&mut self, factor: f32) {
        let size = self.bounds.get().size;
        self.zoom_at(factor, f32::from(size.width) / 2., f32::from(size.height) / 2.);
    }

    /// The topmost node under a point. Groups lie below their content, so they lose against every card.
    fn node_at(&self, x: f64, y: f64) -> Option<String> {
        let doc = self.doc.as_ref()?;
        let is_group = |n: &&Node| matches!(n.kind, NodeKind::Group { .. });
        doc.nodes.iter().rev().filter(|n| !is_group(n)).find(|n| n.contains(x, y)).or_else(|| doc.nodes.iter().rev().filter(is_group).find(|n| n.contains(x, y))).map(|n| n.id.clone())
    }

    fn handle_at(&self, x: f64, y: f64) -> Option<(String, (i8, i8))> {
        let doc = self.doc.as_ref()?;
        let reach = (HANDLE / self.zoom) as f64;
        for n in doc.nodes.iter().rev().filter(|n| self.selection.contains(&n.id)) {
            for (hx, hy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                let px_ = n.x + n.width * (hx as f64 + 1.) / 2.;
                let py_ = n.y + n.height * (hy as f64 + 1.) / 2.;
                if (x - px_).abs() < reach && (y - py_).abs() < reach {
                    return Some((n.id.clone(), (hx, hy)));
                }
            }
        }
        None
    }

    fn edge_handle_at(&self, x: f64, y: f64) -> Option<(String, &'static str)> {
        let doc = self.doc.as_ref()?;
        let reach = (HANDLE / self.zoom) as f64;
        let offset = (14. / self.zoom) as f64;
        for n in doc.nodes.iter().rev().filter(|n| self.selection.contains(&n.id)) {
            for side in ["top", "right", "bottom", "left"] {
                let (sx, sy) = Canvas::side_point(n, side);
                let (dx, dy) = match side {
                    "top" => (0., -offset),
                    "bottom" => (0., offset),
                    "left" => (-offset, 0.),
                    _ => (offset, 0.),
                };
                if (x - sx - dx).abs() < reach && (y - sy - dy).abs() < reach {
                    return Some((n.id.clone(), side));
                }
            }
        }
        None
    }

    fn edge_at(&self, x: f64, y: f64) -> Option<String> {
        let doc = self.doc.as_ref()?;
        let reach = (6. / self.zoom) as f64;
        doc.edges.iter().find_map(|e| {
            let (a, b) = edge_points(doc, e)?;
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy).max(1.)).clamp(0., 1.);
            let (nx, ny) = (a.0 + dx * t, a.1 + dy * t);
            ((x - nx).powi(2) + (y - ny).powi(2) < reach * reach).then(|| e.id.clone())
        })
    }

    fn add_text(&mut self, x: f64, y: f64, window: &mut Window, cx: &mut Context<Self>) {
        self.before_edit(false);
        let Some(doc) = &mut self.doc else { return };
        let id = doc.new_id();
        doc.nodes.push(Node::text(id.clone(), x - 130., y - 30., String::new()));
        self.selection = BTreeSet::from([id.clone()]);
        self.changed(cx);
        self.start_edit(&id, window, cx);
    }

    fn add_group(&mut self, cx: &mut Context<Self>) {
        let size = self.bounds.get().size;
        let (x, y) = self.to_doc(point(self.origin().x + size.width / 2., self.origin().y + size.height / 2.));
        self.before_edit(false);
        let Some(doc) = &mut self.doc else { return };
        let id = doc.new_id();
        let mut group = Node::text(id.clone(), x - 240., y - 160., String::new());
        group.kind = NodeKind::Group { label: Some(tr("Gruppe")), background: None, style: None };
        group.width = 480.;
        group.height = 320.;
        doc.nodes.insert(0, group);
        self.selection = BTreeSet::from([id]);
        self.changed(cx);
    }

    fn add_files(&mut self, paths: &[PathBuf], at: Point<Pixels>, cx: &mut Context<Self>) {
        let vault = index::current();
        let (mut x, y) = self.to_doc(at);
        self.before_edit(false);
        let Some(doc) = &mut self.doc else { return };
        let mut added = BTreeSet::new();
        for path in paths {
            let Ok(relative) = path.strip_prefix(&vault.root) else {
                act(cx, |l| l.last_error = Some(tr("Nur Dateien aus diesem Ordner können auf das Canvas.")));
                continue;
            };
            let id = doc.new_id();
            doc.nodes.push(Node::file(id.clone(), x, y, relative.to_string_lossy().replace('\\', "/")));
            added.insert(id);
            x += 420.;
        }
        if !added.is_empty() {
            self.selection = added;
            self.changed(cx);
        }
    }

    fn start_edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.doc.as_ref().and_then(|d| d.nodes.iter().find(|n| n.id == id)) else { return };
        let value = match &node.kind {
            NodeKind::Text(t) => t.clone(),
            NodeKind::Link(u) => u.clone(),
            NodeKind::Group { label, .. } => label.clone().unwrap_or_default(),
            NodeKind::File { path, .. } => {
                let path = index::current().root.join(path);
                act(cx, |l| l.open_file(&path));
                return;
            }
        };
        let field = if matches!(node.kind, NodeKind::Text(_)) {
            let area = cx.new(|cx| TextareaState::new(window, cx).default_value(value));
            area.update(cx, |s, cx| {
                s.focus(window, cx);
                let end = s.value().len();
                s.set_selected_range(end..end, cx);
            });
            self._subs = vec![cx.subscribe_in(&area, window, |this, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::Blur) {
                    this.finish_edit(window, cx);
                }
            })];
            Field::Text(area)
        } else {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
            input.update(cx, |s, cx| s.focus(window, cx));
            self._subs = vec![cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.finish_edit(window, cx);
                }
            })];
            Field::Line(input)
        };
        self.editing = Some((id.to_string(), field));
        cx.notify();
    }

    fn finish_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, field)) = self.editing.take() else { return };
        let value = match &field {
            Field::Line(input) => input.read(cx).value().to_string(),
            Field::Text(area) => area.read(cx).value().to_string(),
        };
        window.focus(&self.focus, cx);
        let Some(position) = self.doc.as_ref().and_then(|d| d.nodes.iter().position(|n| n.id == id)) else { return };
        let same = matches!(&self.doc.as_ref().unwrap().nodes[position].kind, NodeKind::Text(t) | NodeKind::Link(t) if *t == value);
        if same {
            cx.notify();
            return;
        }
        self.before_edit(false);
        let doc = self.doc.as_mut().unwrap();
        match &mut doc.nodes[position].kind {
            NodeKind::Text(t) | NodeKind::Link(t) => *t = value,
            NodeKind::Group { label, .. } => *label = (!value.is_empty()).then_some(value),
            NodeKind::File { .. } => {}
        }
        self.changed(cx);
    }

    fn delete_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_empty() && self.selected_edge.is_none() {
            return;
        }
        self.before_edit(false);
        let Some(doc) = &mut self.doc else { return };
        for id in std::mem::take(&mut self.selection) {
            doc.remove_node(&id);
        }
        if let Some(edge) = self.selected_edge.take() {
            doc.edges.retain(|e| e.id != edge);
        }
        self.changed(cx);
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) {
        self.before_edit(false);
        let Some(doc) = &mut self.doc else { return };
        let mut fresh = BTreeSet::new();
        let copies: Vec<Node> = doc.nodes.iter().filter(|n| self.selection.contains(&n.id)).cloned().collect();
        for mut node in copies {
            node.id = doc.new_id();
            node.x += 30.;
            node.y += 30.;
            fresh.insert(node.id.clone());
            doc.nodes.push(node);
        }
        self.selection = fresh;
        self.changed(cx);
    }

    fn nudge(&mut self, dx: f64, dy: f64, grow: bool, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        self.before_edit(true);
        let Some(doc) = &mut self.doc else { return };
        let moved = moved_with_groups(doc, &self.selection);
        for n in doc.nodes.iter_mut() {
            if grow {
                if self.selection.contains(&n.id) {
                    n.width = (n.width + dx).max(MIN_SIZE);
                    n.height = (n.height + dy).max(MIN_SIZE);
                }
            } else if moved.contains(&n.id) {
                n.x += dx;
                n.y += dy;
            }
        }
        self.changed(cx);
    }

    fn cycle(&mut self, back: bool, cx: &mut Context<Self>) {
        let Some(doc) = &self.doc else { return };
        if doc.nodes.is_empty() {
            return;
        }
        let at = self.selection.iter().next().and_then(|id| doc.nodes.iter().position(|n| n.id == *id));
        let count = doc.nodes.len();
        let next = match (at, back) {
            (None, _) => 0,
            (Some(i), false) => (i + 1) % count,
            (Some(i), true) => (i + count - 1) % count,
        };
        let edge_from = self.edge_mode.then(|| self.selection.iter().next().cloned()).flatten();
        let target = doc.nodes[next].id.clone();
        if let Some(from) = edge_from
            && from != target
        {
            self.connect(&from, None, &target, cx);
            self.edge_mode = false;
        }
        self.selection = BTreeSet::from([target]);
        cx.notify();
    }

    fn connect(&mut self, from: &str, side: Option<&str>, to: &str, cx: &mut Context<Self>) {
        self.before_edit(false);
        let Some(doc) = &mut self.doc else { return };
        let (Some(a), Some(b)) = (doc.nodes.iter().find(|n| n.id == from).cloned(), doc.nodes.iter().find(|n| n.id == to).cloned()) else { return };
        let (ax, ay) = (a.x + a.width / 2., a.y + a.height / 2.);
        let (bx, by) = (b.x + b.width / 2., b.y + b.height / 2.);
        let mut edge = Edge::new(doc.new_id(), from.to_string(), to.to_string());
        edge.from_side = Some(side.map_or_else(|| Canvas::facing_side(&a, bx, by).to_string(), String::from));
        edge.to_side = Some(Canvas::facing_side(&b, ax, ay).to_string());
        edge.to_end = Some("arrow".to_string());
        doc.edges.push(edge);
        self.changed(cx);
    }

    fn set_color(&mut self, color: Option<String>, cx: &mut Context<Self>) {
        self.before_edit(false);
        let Some(doc) = &mut self.doc else { return };
        for n in doc.nodes.iter_mut().filter(|n| self.selection.contains(&n.id)) {
            n.color = color.clone();
        }
        if let Some(id) = &self.selected_edge
            && let Some(e) = doc.edges.iter_mut().find(|e| e.id == *id)
        {
            e.color = color;
        }
        self.changed(cx);
    }

    fn on_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.finish_edit(window, cx);
        }
        window.focus(&self.focus, cx);
        let (x, y) = self.to_doc(e.position);
        if self.space_down || e.button == MouseButton::Middle {
            self.gesture = Some(Gesture::Pan { from: e.position, pan: self.pan });
            return;
        }
        if e.click_count >= 2 {
            match self.node_at(x, y) {
                Some(id) => cx.defer_in(window, move |v, window, cx| v.start_edit(&id, window, cx)),
                None => cx.defer_in(window, move |v, window, cx| v.add_text(x, y, window, cx)),
            }
            return;
        }
        if let Some((id, side)) = self.edge_handle_at(x, y) {
            self.gesture = Some(Gesture::Edge { from: id, side, to: (x, y) });
            return;
        }
        if let Some((id, handle)) = self.handle_at(x, y)
            && let Some(n) = self.doc.as_ref().and_then(|d| d.nodes.iter().find(|n| n.id == id))
        {
            self.gesture = Some(Gesture::Resize { start: (n.x, n.y, n.width, n.height), id, handle, from: (x, y) });
            return;
        }
        self.selected_edge = None;
        match self.node_at(x, y) {
            Some(id) => {
                if e.modifiers.shift {
                    if !self.selection.remove(&id) {
                        self.selection.insert(id);
                    }
                } else if !self.selection.contains(&id) {
                    self.selection = BTreeSet::from([id]);
                }
                let Some(doc) = &self.doc else { return };
                let moved = moved_with_groups(doc, &self.selection);
                let starts = doc.nodes.iter().filter(|n| moved.contains(&n.id)).map(|n| (n.id.clone(), n.x, n.y)).collect();
                self.gesture = Some(Gesture::Move { from: (x, y), starts, moved: false });
            }
            None => {
                if let Some(edge) = self.edge_at(x, y) {
                    self.selected_edge = Some(edge);
                    self.selection.clear();
                } else {
                    if !e.modifiers.shift {
                        self.selection.clear();
                    }
                    self.gesture = Some(Gesture::Marquee { from: (x, y), to: (x, y), added: e.modifiers.shift });
                }
            }
        }
        cx.notify();
    }

    fn on_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (x, y) = self.to_doc(e.position);
        let snap = !e.modifiers.alt;
        let mut gesture = self.gesture.take();
        match &mut gesture {
            Some(Gesture::Pan { from, pan }) => {
                self.pan = (pan.0 + f32::from(e.position.x - from.x), pan.1 + f32::from(e.position.y - from.y));
            }
            Some(Gesture::Move { from, starts, moved }) => {
                if !*moved {
                    self.before_edit(false);
                    *moved = true;
                }
                let (mut dx, mut dy) = (x - from.0, y - from.1);
                self.guides.clear();
                if snap && let Some(doc) = &self.doc {
                    let ids: BTreeSet<&String> = starts.iter().map(|s| &s.0).collect();
                    let others: Vec<&Node> = doc.nodes.iter().filter(|n| !ids.contains(&n.id)).collect();
                    let tolerance = SNAP / self.zoom as f64;
                    if let Some(first) = doc.nodes.iter().find(|n| Some(&n.id) == starts.first().map(|s| &s.0)) {
                        let start = &starts[0];
                        let (nx, ny) = (start.1 + dx, start.2 + dy);
                        let xs = [nx, nx + first.width / 2., nx + first.width];
                        let ys = [ny, ny + first.height / 2., ny + first.height];
                        let mut found_x = None;
                        let mut found_y = None;
                        for o in &others {
                            for ox in [o.x, o.x + o.width / 2., o.x + o.width] {
                                for v in xs {
                                    if (v - ox).abs() < tolerance && found_x.is_none() {
                                        found_x = Some((ox - v, ox));
                                    }
                                }
                            }
                            for oy in [o.y, o.y + o.height / 2., o.y + o.height] {
                                for v in ys {
                                    if (v - oy).abs() < tolerance && found_y.is_none() {
                                        found_y = Some((oy - v, oy));
                                    }
                                }
                            }
                        }
                        if let Some((shift, at)) = found_x {
                            dx += shift;
                            self.guides.push((true, at));
                        }
                        if let Some((shift, at)) = found_y {
                            dy += shift;
                            self.guides.push((false, at));
                        }
                    }
                }
                if let Some(doc) = &mut self.doc {
                    for (id, sx, sy) in starts.iter() {
                        if let Some(n) = doc.nodes.iter_mut().find(|n| n.id == *id) {
                            n.x = (sx + dx).round();
                            n.y = (sy + dy).round();
                        }
                    }
                }
            }
            Some(Gesture::Resize { id, handle, from, start }) => {
                let (dx, dy) = (x - from.0, y - from.1);
                if let Some(doc) = &mut self.doc
                    && let Some(n) = doc.nodes.iter_mut().find(|n| n.id == *id)
                {
                    let (sx, sy, sw, sh) = *start;
                    if handle.0 > 0 {
                        n.width = (sw + dx).max(MIN_SIZE).round();
                    } else {
                        n.width = (sw - dx).max(MIN_SIZE).round();
                        n.x = (sx + sw - n.width).round();
                    }
                    if handle.1 > 0 {
                        n.height = (sh + dy).max(MIN_SIZE).round();
                    } else {
                        n.height = (sh - dy).max(MIN_SIZE).round();
                        n.y = (sy + sh - n.height).round();
                    }
                }
            }
            Some(Gesture::Edge { to, .. }) => *to = (x, y),
            Some(Gesture::Marquee { to, .. }) => *to = (x, y),
            None => {
                self.gesture = None;
                return;
            }
        }
        self.gesture = gesture;
        cx.notify();
    }

    fn on_up(&mut self, cx: &mut Context<Self>) {
        let gesture = self.gesture.take();
        self.guides.clear();
        match gesture {
            Some(Gesture::Move { moved: true, .. }) => self.changed(cx),
            Some(Gesture::Resize { .. }) => {
                self.undo.push(self.source.clone());
                self.changed(cx);
            }
            Some(Gesture::Edge { from, side, to }) => {
                if let Some(target) = self.node_at(to.0, to.1).filter(|t| *t != from) {
                    self.connect(&from, Some(side), &target, cx);
                }
            }
            Some(Gesture::Marquee { from, to, .. }) => {
                let (a, b) = ((from.0.min(to.0), from.1.min(to.1)), (from.0.max(to.0), from.1.max(to.1)));
                if let Some(doc) = &self.doc {
                    for n in &doc.nodes {
                        if n.x >= a.0 && n.y >= a.1 && n.x + n.width <= b.0 && n.y + n.height <= b.1 {
                            self.selection.insert(n.id.clone());
                        }
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if self.editing.is_some() {
            if k.key == "escape" {
                self.finish_edit(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        let command = k.modifiers.platform || k.modifiers.control;
        let step = if k.modifiers.alt { 1. } else { 10. };
        match (k.key.as_str(), command, k.modifiers.shift) {
            ("space", false, _) => self.space_down = true,
            ("left", false, grow) => self.nudge(-step, 0., grow, cx),
            ("right", false, grow) => self.nudge(step, 0., grow, cx),
            ("up", false, grow) => self.nudge(0., -step, grow, cx),
            ("down", false, grow) => self.nudge(0., step, grow, cx),
            ("enter", false, _) => {
                if let Some(id) = self.selection.iter().next().cloned() {
                    self.start_edit(&id, window, cx);
                }
            }
            ("e", false, _) => self.edge_mode = !self.selection.is_empty(),
            ("backspace" | "delete", _, _) => self.delete_selection(cx),
            ("d", true, _) => self.duplicate(cx),
            ("a", true, _) => {
                if let Some(doc) = &self.doc {
                    self.selection = doc.nodes.iter().map(|n| n.id.clone()).collect();
                }
            }
            ("1" | "!", false, true) => self.fit_all(),
            ("2" | "\"", false, true) => self.fit_selection(),
            ("=" | "+", true, _) => self.zoom_center(1.2),
            ("-", true, _) => self.zoom_center(1. / 1.2),
            ("0", true, _) => {
                let size = self.bounds.get().size;
                self.zoom_at(1. / self.zoom, f32::from(size.width) / 2., f32::from(size.height) / 2.);
            }
            ("escape", _, _) => {
                self.edge_mode = false;
                self.selection.clear();
                self.selected_edge = None;
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn node_content(&self, node: &Node, zoom: f32, t: &Theme, cx: &mut App) -> AnyElement {
        let size = font::BODY * zoom;
        let title = |s: String| div().p(px(space::S * zoom)).text_size(px(size)).font_weight(FontWeight::SEMIBOLD).truncate().child(s);
        if zoom < 0.3 {
            let name = match &node.kind {
                NodeKind::Text(text) => text.lines().next().unwrap_or_default().trim_start_matches('#').trim().to_string(),
                NodeKind::File { path, .. } => Path::new(path).file_stem().unwrap_or_default().to_string_lossy().to_string(),
                NodeKind::Link(url) => url.clone(),
                NodeKind::Group { .. } => String::new(),
            };
            return title(name).into_any_element();
        }
        match &node.kind {
            NodeKind::Text(text) if (zoom - 1.).abs() < 0.3 => card_note(text, t),
            NodeKind::Text(text) => mini_markdown(text, zoom, t),
            NodeKind::Link(url) => div().p(px(space::S * zoom)).flex().items_center().gap(px(space::XS * zoom)).child(icon("link", 14. * zoom, t.secondary)).child(div().truncate().text_size(px(size)).text_color(t.accent_text).child(url.clone())).into_any_element(),
            NodeKind::Group { background: Some(picture), style, .. } => {
                let full = index::current().root.join(picture);
                let fit = match style.as_deref() {
                    Some("ratio") => ObjectFit::Contain,
                    Some("repeat") => ObjectFit::None,
                    _ => ObjectFit::Cover,
                };
                img(full).size_full().object_fit(fit).opacity(0.5).into_any_element()
            }
            NodeKind::Group { .. } => div().into_any_element(),
            NodeKind::File { path, subpath } => {
                let full = index::current().root.join(path);
                let name = Path::new(path).file_name().unwrap_or_default().to_string_lossy().to_string();
                let image = ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"].contains(&Path::new(path).extension().unwrap_or_default().to_string_lossy().to_lowercase().as_str());
                if image {
                    return img(full).size_full().object_fit(ObjectFit::Contain).into_any_element();
                }
                let is_pdf = Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
                let page = subpath.as_deref().and_then(|s| s.strip_prefix("#page=")).and_then(|p| p.parse().ok()).unwrap_or(1);
                let body = if is_pdf {
                    match super::embeds::pdf_page(&full, page, cx) {
                        Some(image) => img(image).size_full().object_fit(ObjectFit::Contain).into_any_element(),
                        None => div().p(px(space::S * zoom)).child(file_icon(&full, 28. * zoom)).into_any_element(),
                    }
                } else if crate::core::markdown::is_markdown(&full) {
                    let generation = index::current().generation;
                    let text = {
                        let mut cards = self.card_texts.borrow_mut();
                        let entry = cards.entry(full.clone()).or_insert_with(|| (generation, std::fs::read_to_string(&full).unwrap_or_default()));
                        if entry.0 != generation {
                            *entry = (generation, std::fs::read_to_string(&full).unwrap_or_default());
                        }
                        entry.1.clone()
                    };
                    let shown = match subpath.as_deref() {
                        Some(sub) => crate::core::vault::links::section(&text, &crate::core::vault::links::LinkTarget::parse(sub)),
                        None => text,
                    };
                    let start = crate::core::vault::frontmatter::body_offset(&shown);
                    mini_markdown(&shown[start..], zoom, t)
                } else {
                    div().p(px(space::S * zoom)).child(file_icon(&full, 28. * zoom)).into_any_element()
                };
                div().size_full().flex().flex_col().child(div().flex_none().px(px(space::S * zoom)).py(px(space::XS * zoom)).bg(t.secondary_bg).text_size(px(font::CAPTION * zoom)).text_color(t.secondary).truncate().child(name)).child(div().flex_1().min_h_0().overflow_hidden().child(body)).into_any_element()
            }
        }
    }
}

fn moved_with_groups(doc: &Canvas, selection: &BTreeSet<String>) -> BTreeSet<String> {
    let mut moved = selection.clone();
    for n in doc.nodes.iter().filter(|n| selection.contains(&n.id)) {
        if matches!(n.kind, NodeKind::Group { .. }) {
            moved.extend(doc.children_of(n));
        }
    }
    moved
}

fn edge_points(doc: &Canvas, e: &Edge) -> Option<((f64, f64), (f64, f64))> {
    let a = doc.nodes.iter().find(|n| n.id == e.from)?;
    let b = doc.nodes.iter().find(|n| n.id == e.to)?;
    let from_side = e.from_side.clone().unwrap_or_else(|| Canvas::facing_side(a, b.x + b.width / 2., b.y + b.height / 2.).to_string());
    let to_side = e.to_side.clone().unwrap_or_else(|| Canvas::facing_side(b, a.x + a.width / 2., a.y + a.height / 2.).to_string());
    Some((Canvas::side_point(a, &from_side), Canvas::side_point(b, &to_side)))
}

/// Cards scale with the zoom, which the regular reader cannot; this draws the common blocks at any size.
fn mini_markdown(source: &str, zoom: f32, t: &Theme) -> AnyElement {
    static BLOCK_ID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\s\^[A-Za-z0-9-]+\s*$").unwrap());
    let size = font::BODY * zoom;
    let lines = source.lines().take(60).map(|line| {
        let line = BLOCK_ID.replace(line, "");
        let trimmed = line.trim_start();
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        let (text_part, scale, bold) = match level {
            1..=6 if trimmed.chars().nth(level) == Some(' ') => (trimmed[level + 1..].to_string(), 1.5 - level as f32 * 0.08, true),
            _ => (trimmed.strip_prefix("- ").map_or_else(|| trimmed.to_string(), |r| format!("• {r}")), 1., false),
        };
        let plain = strip_inline(&text_part);
        div().min_h(px(size * 1.4)).text_size(px(size * scale)).line_height(px(size * scale * 1.35)).when(bold, |d| d.font_weight(FontWeight::SEMIBOLD)).child(plain)
    });
    div().p(px(space::M * zoom)).flex().flex_col().gap(px(space::XXS * zoom)).text_color(t.ink).children(lines).into_any_element()
}

fn strip_inline(line: &str) -> String {
    static LINK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"!?\[\[(?:[^\]|]*\|)?([^\]]*)\]\]").unwrap());
    static MD_LINK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"!?\[([^\]]*)\]\([^)]*\)").unwrap());
    let out = LINK.replace_all(line, "$1");
    let out = MD_LINK.replace_all(&out, "$1");
    out.replace("**", "").replace("==", "").replace('`', "")
}

impl Render for CanvasView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        self.reload_if_changed();
        let Some(doc) = self.doc.clone() else {
            return div().p(px(space::L)).child(notice("triangle-alert", t.orange, tr("Diese Datei lässt sich nicht lesen."), Some(tr("Sie bleibt unverändert. Prüfe sie in Obsidian oder einem Texteditor.")), div(), &t)).into_any_element();
        };
        if !self.fitted && f32::from(self.bounds.get().size.width) > 10. {
            if doc.nodes.is_empty() {
                let size = self.bounds.get().size;
                self.pan = (f32::from(size.width) / 2., f32::from(size.height) / 2.);
                self.fitted = true;
            } else {
                self.fit_all();
            }
            window.request_animation_frame();
        }
        let (zoom, pan) = (self.zoom, self.pan);
        let size = self.bounds.get().size;
        let (vw, vh) = (f32::from(size.width), f32::from(size.height));
        let visible = |n: &Node| {
            let (x, y) = (n.x as f32 * zoom + pan.0, n.y as f32 * zoom + pan.1);
            x + n.width as f32 * zoom > -40. && y + n.height as f32 * zoom > -40. && x < vw + 40. && y < vh + 40.
        };
        let accent = t.accent;
        let mut nodes: Vec<AnyElement> = Vec::new();
        let mut order: Vec<&Node> = doc.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Group { .. })).collect();
        order.extend(doc.nodes.iter().filter(|n| !matches!(n.kind, NodeKind::Group { .. })));
        let far = zoom < 0.15;
        let mut far_cards: Vec<(f32, f32, f32, f32, Hsla, Hsla)> = Vec::new();
        for node in order.into_iter().filter(|n| visible(n)) {
            let selected = self.selection.contains(&node.id);
            if far && self.editing.is_none() {
                let (x, y) = self.to_view(node.x, node.y);
                let tint = color_of(node.color.as_ref());
                let fill = if matches!(node.kind, NodeKind::Group { .. }) { tint.map_or(t.ink.opacity(0.04), |c| c.opacity(0.08)) } else { tint.map_or(t.column, |c| c.opacity(0.3)) };
                far_cards.push((x, y, node.width as f32 * zoom, node.height as f32 * zoom, fill, if selected { accent } else { tint.unwrap_or(t.separator) }));
                continue;
            }
            let (x, y) = self.to_view(node.x, node.y);
            let (w, h) = (node.width as f32 * zoom, node.height as f32 * zoom);
            let tint = color_of(node.color.as_ref());
            let is_group = matches!(node.kind, NodeKind::Group { .. });
            let border = tint.unwrap_or(t.separator);
            let label = if let NodeKind::Group { label: Some(l), .. } = &node.kind { Some(l.clone()) } else { None };
            let editing = self.editing.as_ref().filter(|(id, _)| *id == node.id).map(|(_, field)| field.clone());
            let content = if editing.is_none() { Some(self.node_content(node, zoom, &t, cx)) } else { None };
            nodes.push(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(radius::CARD * zoom))
                    .border_1()
                    .border_color(if selected { accent } else { border })
                    .when(selected, |d| d.border_2())
                    .when(!is_group, |d| d.bg(tint.map_or(t.column, |c| c.opacity(0.12))).overflow_hidden())
                    .when(is_group, |d| d.bg(tint.map_or(t.ink.opacity(0.04), |c| c.opacity(0.08))))
                    .when_some(label, |d, l| d.child(div().absolute().left(px(0.)).top(px(-(font::CALLOUT * zoom + space::S))).text_size(px(font::CALLOUT * zoom.max(0.5))).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(l)))
                    .children(content)
                    .when_some(editing, |d, input| d.child(div().size_full().p(px(space::XS)).child(match input {
                        Field::Line(i) => Input::new(&i).small().into_any_element(),
                        Field::Text(a) => Textarea::new(&a).h_full().into_any_element(),
                    })))
                    .into_any_element(),
            );
        }
        let handles: Vec<AnyElement> = doc
            .nodes
            .iter()
            .filter(|n| self.selection.contains(&n.id) && self.editing.is_none())
            .flat_map(|n| {
                let mut out: Vec<AnyElement> = Vec::new();
                for (hx, hy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                    let (x, y) = self.to_view(n.x + n.width * (hx as f64 + 1.) / 2., n.y + n.height * (hy as f64 + 1.) / 2.);
                    out.push(div().absolute().left(px(x - 4.)).top(px(y - 4.)).size(px(8.)).rounded(px(2.)).bg(t.content).border_1().border_color(accent).into_any_element());
                }
                for side in ["top", "right", "bottom", "left"] {
                    let (sx, sy) = Canvas::side_point(n, side);
                    let (mut x, mut y) = self.to_view(sx, sy);
                    match side {
                        "top" => y -= 14.,
                        "bottom" => y += 14.,
                        "left" => x -= 14.,
                        _ => x += 14.,
                    }
                    out.push(div().absolute().left(px(x - 4.)).top(px(y - 4.)).size(px(8.)).rounded_full().bg(accent).into_any_element());
                }
                out
            })
            .collect();
        let edge_lines: Vec<((f64, f64), (f64, f64), Hsla, bool, bool)> = doc.edges.iter().filter_map(|e| {
            let (a, b) = edge_points(&doc, e)?;
            let selected = self.selected_edge.as_ref() == Some(&e.id);
            Some((a, b, if selected { accent } else { color_of(e.color.as_ref()).unwrap_or(t.secondary) }, e.to_end.as_deref().unwrap_or("arrow") == "arrow", e.from_end.as_deref() == Some("arrow")))
        }).collect();
        let labels: Vec<AnyElement> = doc.edges.iter().filter(|_| !far).filter_map(|e| {
            let label = e.label.clone()?;
            let (a, b) = edge_points(&doc, e)?;
            let (x, y) = self.to_view((a.0 + b.0) / 2., (a.1 + b.1) / 2.);
            let half = label.chars().count() as f32 * font::CAPTION * 0.28 + space::XS;
            Some(div().absolute().left(px(x - half)).top(px(y - font::CAPTION)).px(px(space::XS)).rounded(px(radius::SMALL)).bg(t.content).text_size(px(font::CAPTION)).text_color(t.secondary).child(label).into_any_element())
        }).collect();
        let preview = match &self.gesture {
            Some(Gesture::Edge { from, side, to }) => doc.nodes.iter().find(|n| n.id == *from).map(|n| (Canvas::side_point(n, side), *to)),
            _ => None,
        };
        let marquee = if let Some(Gesture::Marquee { from, to, .. }) = &self.gesture {
            let (x, y) = self.to_view(from.0.min(to.0), from.1.min(to.1));
            let (w, h) = (((from.0 - to.0).abs()) as f32 * zoom, ((from.1 - to.1).abs()) as f32 * zoom);
            Some(div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h)).bg(accent.opacity(0.1)).border_1().border_color(accent).into_any_element())
        } else {
            None
        };
        let guides = self.guides.clone();
        let measured = self.bounds.clone();
        let line_color = accent;
        let lines = canvas(
            move |b, _, _| measured.set(b),
            move |bounds, _, window, _| {
                let at = |p: (f64, f64)| point(bounds.origin.x + px(p.0 as f32 * zoom + pan.0), bounds.origin.y + px(p.1 as f32 * zoom + pan.1));
                for (x, y, w, h, fill, border) in &far_cards {
                    let area = Bounds::new(point(bounds.origin.x + px(*x), bounds.origin.y + px(*y)), gpui_kit::size(px(*w), px(*h)));
                    window.paint_quad(PaintQuad { bounds: area, corner_radii: Corners::all(px(1.)), background: (*fill).into(), border_widths: Edges::all(px(1.)), border_color: *border, border_style: BorderStyle::Solid });
                }
                if far {
                    let mut all = PathBuilder::stroke(px(1.));
                    let mut any = false;
                    for (a, b, _, _, _) in &edge_lines {
                        all.move_to(at(*a));
                        all.line_to(at(*b));
                        any = true;
                    }
                    if any && let Ok(p) = all.build() {
                        window.paint_path(p, line_color.opacity(0.4));
                    }
                }
                let curve = |a: Point<Pixels>, b: Point<Pixels>, color: Hsla, end: bool, start: bool, window: &mut Window| {
                    let mut path = PathBuilder::stroke(px(1.5_f32.max(1.5 * zoom)));
                    path.move_to(a);
                    let bend = (b.x - a.x) * 0.4;
                    path.cubic_bezier_to(b, point(a.x + bend, a.y), point(b.x - bend, b.y));
                    if let Ok(p) = path.build() {
                        window.paint_path(p, color);
                    }
                    let tip = |tip: Point<Pixels>, from: Point<Pixels>, window: &mut Window| {
                        let (dx, dy) = (f32::from(tip.x - from.x), f32::from(tip.y - from.y));
                        let d = (dx * dx + dy * dy).sqrt().max(1.);
                        let (ux, uy) = (dx / d, dy / d);
                        let s = 8. * zoom.max(0.5);
                        let mut arrow = PathBuilder::fill();
                        arrow.move_to(tip);
                        arrow.line_to(point(tip.x - px(ux * s + uy * s * 0.5), tip.y - px(uy * s - ux * s * 0.5)));
                        arrow.line_to(point(tip.x - px(ux * s - uy * s * 0.5), tip.y - px(uy * s + ux * s * 0.5)));
                        arrow.close();
                        if let Ok(p) = arrow.build() {
                            window.paint_path(p, color);
                        }
                    };
                    if end {
                        tip(b, a, window);
                    }
                    if start {
                        tip(a, b, window);
                    }
                };
                if !far {
                    for (a, b, color, end, start) in &edge_lines {
                        curve(at(*a), at(*b), *color, *end, *start, window);
                    }
                }
                if let Some((a, b)) = preview {
                    curve(at(a), at(b), line_color, true, false, window);
                }
                for (vertical, value) in &guides {
                    let mut path = PathBuilder::stroke(px(1.));
                    if *vertical {
                        let x = at((*value, 0.)).x;
                        path.move_to(point(x, bounds.origin.y));
                        path.line_to(point(x, bounds.origin.y + bounds.size.height));
                    } else {
                        let y = at((0., *value)).y;
                        path.move_to(point(bounds.origin.x, y));
                        path.line_to(point(bounds.origin.x + bounds.size.width, y));
                    }
                    if let Ok(p) = path.build() {
                        window.paint_path(p, line_color.opacity(0.7));
                    }
                }
            },
        )
        .absolute()
        .inset_0();
        let has_selection = !self.selection.is_empty() || self.selected_edge.is_some();
        let colors = has_selection.then(|| {
            let t2 = t.clone();
            div()
                .absolute()
                .top(px(space::M))
                .left(px(space::M))
                .p(px(space::XS))
                .flex()
                .gap(px(space::XS))
                .rounded(px(radius::CONTROL))
                .bg(t.column)
                .border_1()
                .border_color(t.separator)
                .child(div().id("canvas-color-none").size(px(18.)).rounded_full().border_1().border_color(t2.secondary).cursor_pointer().key_nav().tooltip(tip(tr("Standardfarbe"))).on_click(cx.listener(|v, _, _, cx| v.set_color(None, cx))))
                .children(PRESETS.iter().enumerate().map(|(i, (key, value))| {
                    let key = key.to_string();
                    div().id(("canvas-color", i)).size(px(18.)).rounded_full().bg(rgb(*value)).cursor_pointer().key_nav().tooltip(tip(tr("Farbe"))).on_click(cx.listener(move |v, _, _, cx| v.set_color(Some(key.clone()), cx)))
                }))
        });
        let zoom_label = format!("{}%", (zoom * 100.).round());
        let tools = div()
            .absolute()
            .bottom(px(space::M))
            .right(px(space::M))
            .p(px(space::XS))
            .flex()
            .items_center()
            .gap(px(space::XS))
            .rounded(px(radius::CONTROL))
            .bg(t.column)
            .border_1()
            .border_color(t.separator)
            .child(Button::new("canvas-group").ghost().small().icon(menu_icon("square-dashed")).tooltip(tr("Gruppe anlegen")).on_click(cx.listener(|v, _, _, cx| v.add_group(cx))))
            .child(Button::new("canvas-out").ghost().small().icon(menu_icon("minus")).tooltip(tr("Verkleinern")).on_click(cx.listener(|v, _, _, cx| {
                v.zoom_center(1. / 1.2);
                cx.notify();
            })))
            .child(div().min_w(px(40.)).flex().justify_center().font_features(tabular()).text_size(px(font::CAPTION)).child(zoom_label))
            .child(Button::new("canvas-in").ghost().small().icon(menu_icon("plus")).tooltip(tr("Vergrößern")).on_click(cx.listener(|v, _, _, cx| {
                v.zoom_center(1.2);
                cx.notify();
            })))
            .child(Button::new("canvas-fit").ghost().small().icon(menu_icon("maximize")).tooltip(tr("Alles zeigen (⇧1)")).on_click(cx.listener(|v, _, _, cx| {
                v.fit_all();
                cx.notify();
            })));
        let edge_hint = self.edge_mode.then(|| div().absolute().top(px(space::M)).right(px(space::M)).px(px(space::S)).py(px(space::XS)).rounded(px(radius::CONTROL)).bg(t.column).border_1().border_color(accent).text_size(px(font::CALLOUT)).child(tr("Kante: Tab wählt das Ziel")));
        let empty = doc.nodes.is_empty();
        let hand_over = super::vault_tools::in_vault(&self.path, cx).then(|| super::vault_tools::obsidian_button("canvas-obsidian", self.path.clone()));
        let bar = super::documents::document_bar(&self.path, div().children(hand_over).into_any_element(), &t, cx);
        let surface = div()
            .id("canvas-page")
            .track_focus(&self.focus)
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .bg(t.content)
            .cursor(if self.space_down { CursorStyle::OpenHand } else { CursorStyle::Arrow })
            .key_context("WalkItems")
            .on_action(cx.listener(|v, _: &super::app::Undo, _, cx| v.step_history(true, cx)))
            .on_action(cx.listener(|v, _: &super::app::Redo, _, cx| v.step_history(false, cx)))
            .on_action(cx.listener(|v, _: &NextItem, _, cx| v.cycle(false, cx)))
            .on_action(cx.listener(|v, _: &PreviousItem, _, cx| v.cycle(true, cx)))
            .on_key_down(cx.listener(Self::on_key))
            .on_key_up(cx.listener(|v, e: &KeyUpEvent, _, cx| {
                if e.keystroke.key == "space" {
                    v.space_down = false;
                    cx.notify();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_down))
            .on_mouse_move(cx.listener(|v, e: &MouseMoveEvent, _, cx| v.on_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, cx| v.on_up(cx)))
            .on_mouse_up(MouseButton::Middle, cx.listener(|v, _, _, cx| v.on_up(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|v, _, _, cx| v.on_up(cx)))
            .on_scroll(cx.listener(|v, e: &ScrollEvent, _, cx| {
                let delta = e.delta.pixel_delta(px(20.));
                if e.modifiers.platform || e.modifiers.control {
                    let o = v.origin();
                    v.zoom_at(1. + f32::from(delta.y) * 0.004, f32::from(e.position.x - o.x), f32::from(e.position.y - o.y));
                } else {
                    v.pan.0 += f32::from(delta.x);
                    v.pan.1 += f32::from(delta.y);
                }
                cx.notify();
            }))
            .on_pinch(cx.listener(|v, e: &PinchEvent, _, cx| {
                let o = v.origin();
                v.zoom_at(1. + e.delta, f32::from(e.position.x - o.x), f32::from(e.position.y - o.y));
                cx.notify();
            }))
            .on_drop(cx.listener(|v, paths: &ExternalPaths, window, cx| {
                let at = window.mouse_position();
                v.add_files(paths.paths(), at, cx);
            }))
            .on_drop(cx.listener(|v, file: &DraggedFile, window, cx| {
                let at = window.mouse_position();
                v.add_files(std::slice::from_ref(&file.0), at, cx);
            }))
            .children(nodes)
            .child(lines)
            .children(labels)
            .children(handles)
            .children(marquee)
            .when(empty, |d| d.child(div().absolute().inset_0().flex().items_center().justify_center().child(caption(tr("Doppelklick legt eine Karte an"), &t))))
            .children(colors)
            .children(edge_hint)
            .child(tools);
        div().size_full().flex().flex_col().child(bar).child(surface).into_any_element()
    }
}

thread_local! {
    static CARD_NOTES: std::cell::RefCell<std::collections::HashMap<String, std::rc::Rc<super::markdown_view::ParsedNote>>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// A text card drawn as the note it is, not a sketch of it.
fn card_note(text: &str, t: &Theme) -> AnyElement {
    let note = CARD_NOTES.with(|cards| {
        let mut cards = cards.borrow_mut();
        if cards.len() > 512 {
            cards.clear();
        }
        cards.entry(text.to_string()).or_insert_with(|| std::rc::Rc::new(super::markdown_view::parse(text))).clone()
    });
    super::markdown_view::render(&note, None, None, t).into_any_element()
}
