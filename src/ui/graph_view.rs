use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::{self, act};
use super::theme::{Theme, font, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::vault::graph::{Graph, Layout, NodeKind, Options};
use crate::core::vault::index;

/// Positions stay for the whole session, so returning to the graph looks the same.
#[derive(Default)]
struct KnownPositions(HashMap<String, (f32, f32)>);

impl Global for KnownPositions {}

/// The graph of the vault or around one note.
pub struct GraphView {
    options: Options,
    focus_note: Option<(PathBuf, usize)>,
    graph: Arc<Graph>,
    layout: Arc<Mutex<Layout>>,
    positions: Vec<(f32, f32)>,
    generation: u64,
    built_for: Option<(Options, Option<(PathBuf, usize)>)>,
    pan: (f32, f32),
    zoom: f32,
    fitted: bool,
    bounds: Rc2,
    hover: Option<u32>,
    selected: Option<u32>,
    drag: Option<(Point<Pixels>, (f32, f32))>,
    filter: Entity<InputState>,
    focus: FocusHandle,
    running: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

type Rc2 = std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>;

impl GraphView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Dateien filtern")));
        let subs = vec![cx.subscribe(&filter, |this, input, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                this.options.filter = input.read(cx).value().to_string();
                cx.notify();
            }
        })];
        Self {
            options: Options::default(),
            focus_note: None,
            graph: Arc::default(),
            layout: Arc::default(),
            positions: Vec::new(),
            generation: 0,
            built_for: None,
            pan: (0., 0.),
            zoom: 1.,
            fitted: false,
            bounds: Default::default(),
            hover: None,
            selected: None,
            drag: None,
            filter,
            focus: cx.focus_handle(),
            running: None,
            _subs: subs,
        }
    }

    fn rebuild_if_needed(&mut self, cx: &mut Context<Self>) {
        let vault = index::current();
        let wanted = state::read(cx, |l| l.graph_focus.clone());
        if self.generation == vault.generation && self.built_for.as_ref() == Some(&(self.options.clone(), wanted.clone())) {
            return;
        }
        self.generation = vault.generation;
        self.built_for = Some((self.options.clone(), wanted.clone()));
        self.focus_note = wanted.clone();
        let full = Graph::build(&vault, &self.options);
        let graph = Arc::new(match &wanted {
            Some((path, depth)) => full.local(path, *depth),
            None => full,
        });
        let known = cx.try_global::<KnownPositions>().map(|k| k.0.clone()).unwrap_or_default();
        let nodes = graph.nodes.clone();
        let mut layout = Layout::new(nodes.len(), |i| known.get(&nodes[i].name).copied());
        if nodes.iter().all(|n| known.contains_key(&n.name)) && !nodes.is_empty() {
            layout.heat = 0.2;
        }
        self.positions = layout.positions.clone();
        self.graph = graph.clone();
        self.layout = Arc::new(Mutex::new(layout));
        self.selected = None;
        self.hover = None;
        self.fitted = false;
        let shared = self.layout.clone();
        let reduce = cx.reduce_motion();
        self.running = Some(cx.spawn(async move |this, cx| {
            loop {
                let (edges, shared2) = (graph.clone(), shared.clone());
                let (positions, heat) = cx
                    .background_spawn(async move {
                        let mut l = shared2.lock().unwrap();
                        if reduce {
                            l.settle(&edges.edges, 400);
                        } else {
                            for _ in 0..3 {
                                l.step(&edges.edges);
                            }
                        }
                        (l.positions.clone(), l.heat)
                    })
                    .await;
                let done = heat < 0.01 || reduce;
                let alive = this
                    .update(cx, |v, cx| {
                        v.positions = positions;
                        if !v.fitted || (done && !v.moved_by_hand()) {
                            v.fit();
                        }
                        if done {
                            v.remember(cx);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if done || !alive {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(16)).await;
            }
        }));
    }

    fn moved_by_hand(&self) -> bool {
        self.fitted
    }

    fn remember(&self, cx: &mut Context<Self>) {
        let names = self.graph.nodes.iter().map(|n| n.name.clone()).zip(self.positions.iter().copied());
        let known = cx.default_global::<KnownPositions>();
        known.0.extend(names);
    }

    fn fit(&mut self) {
        let size = self.bounds.get().size;
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        if w < 10. || self.positions.is_empty() {
            return;
        }
        let first = self.positions[0];
        let (a, b, c, d) = self.positions.iter().fold((first.0, first.1, first.0, first.1), |(a, b, c, d), p| (a.min(p.0), b.min(p.1), c.max(p.0), d.max(p.1)));
        self.zoom = ((w - 80.) / (c - a).max(1.)).min((h - 80.) / (d - b).max(1.)).clamp(0.05, 2.);
        self.pan = (w / 2. - (a + c) / 2. * self.zoom, h / 2. - (b + d) / 2. * self.zoom);
        self.fitted = true;
    }

    fn to_screen(&self, p: (f32, f32)) -> (f32, f32) {
        (p.0 * self.zoom + self.pan.0, p.1 * self.zoom + self.pan.1)
    }

    fn radius(&self, index: u32) -> f32 {
        let links = self.graph.nodes.get(index as usize).map_or(0, |n| n.links) as f32;
        (3. + links.sqrt() * 1.6).min(16.)
    }

    fn hit(&self, at: Point<Pixels>) -> Option<u32> {
        let origin = self.bounds.get().origin;
        let (x, y) = (f32::from(at.x - origin.x), f32::from(at.y - origin.y));
        let mut best = None;
        let mut distance = f32::MAX;
        for (i, p) in self.positions.iter().enumerate() {
            let s = self.to_screen(*p);
            let d = (s.0 - x).powi(2) + (s.1 - y).powi(2);
            let r = (self.radius(i as u32) * self.zoom.max(0.5)).max(6.) + 2.;
            if d < r * r && d < distance {
                distance = d;
                best = Some(i as u32);
            }
        }
        best
    }

    fn open(&self, index: u32, cx: &mut App) {
        match self.graph.nodes.get(index as usize).map(|n| n.kind.clone()) {
            Some(NodeKind::Note(path) | NodeKind::Attachment(path)) => act(cx, |l| l.open_file(&path)),
            Some(NodeKind::Tag(tag)) => act(cx, |l| l.show_tag(&tag)),
            Some(NodeKind::Missing(name)) => act(cx, |l| l.create_link_target(&name, None)),
            None => {}
        }
    }

    fn zoom_at(&mut self, factor: f32, x: f32, y: f32) {
        let next = (self.zoom * factor).clamp(0.03, 6.);
        let k = next / self.zoom;
        self.pan = (x - (x - self.pan.0) * k, y - (y - self.pan.1) * k);
        self.zoom = next;
    }

    /// Arrow keys walk along the links towards the point lying most in that direction.
    fn step_to_neighbor(&mut self, dx: f32, dy: f32) {
        let Some(current) = self.selected.or_else(|| (!self.positions.is_empty()).then_some(0)) else { return };
        if self.selected.is_none() {
            self.selected = Some(current);
            return;
        }
        let here = self.positions[current as usize];
        let best = self.graph.neighbors(current).into_iter().max_by(|a, b| {
            let score = |i: &u32| {
                let p = self.positions[*i as usize];
                let (vx, vy) = (p.0 - here.0, p.1 - here.1);
                (vx * dx + vy * dy) / (vx * vx + vy * vy).sqrt().max(0.1)
            };
            score(a).total_cmp(&score(b))
        });
        if best.is_some() {
            self.selected = best;
        }
    }

    fn toggle(&self, id: &'static str, label: String, on: bool, set: fn(&mut Options, bool), cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let me = cx.entity();
        div().flex().items_center().gap(px(space::XS)).text_size(px(font::CALLOUT)).child(Switch::new(id).small().checked(on).on_click(move |v, _, cx| {
            let value = *v;
            me.update(cx, |g, cx| {
                set(&mut g.options, value);
                cx.notify();
            });
        })).child(label)
    }
}

impl Render for GraphView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.rebuild_if_needed(cx);
        let t = Theme::get(cx);
        let count = self.graph.nodes.len();
        let local = self.focus_note.clone();
        let bar = div()
            .flex_none()
            .p(px(space::M))
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(space::M))
            .border_b_1()
            .border_color(t.separator)
            .child(div().w(px(220.)).child(Input::new(&self.filter).small()))
            .child(self.toggle("graph-tags", tr("Tags"), self.options.tags, |o, v| o.tags = v, cx))
            .child(self.toggle("graph-attachments", tr("Anhänge"), self.options.attachments, |o, v| o.attachments = v, cx))
            .child(self.toggle("graph-missing", tr("Nicht angelegt"), self.options.missing, |o, v| o.missing = v, cx))
            .child(self.toggle("graph-orphans", tr("Ohne Links"), self.options.orphans, |o, v| o.orphans = v, cx))
            .when_some(local, |d, (path, depth)| {
                let (p1, p2) = (path.clone(), path.clone());
                d.child(caption(trf("Um %@", &[&path.file_stem().unwrap_or_default().to_string_lossy()]), &t))
                    .child(Button::new("graph-depth-less").ghost().small().icon(menu_icon("minus")).tooltip(tr("Weniger Tiefe")).disabled(depth <= 1).on_click(move |_, _, cx| {
                        let p = p1.clone();
                        act(cx, |l| l.graph_focus = Some((p, depth - 1)));
                    }))
                    .child(div().font_features(tabular()).text_size(px(font::CALLOUT)).child(depth.to_string()))
                    .child(Button::new("graph-depth-more").ghost().small().icon(menu_icon("plus")).tooltip(tr("Mehr Tiefe")).disabled(depth >= 3).on_click(move |_, _, cx| {
                        let p = p2.clone();
                        act(cx, |l| l.graph_focus = Some((p, depth + 1)));
                    }))
                    .child(Button::new("graph-all").ghost().small().label(tr("Ganzer Graph")).on_click(|_, _, cx| act(cx, |l| l.graph_focus = None)))
            })
            .child(div().flex_1())
            .child(caption(if count == 1 { tr("1 Datei") } else { trf("%@ Dateien", &[&count.to_string()]) }, &t));
        let measured = self.bounds.clone();
        let (graph, positions, pan, zoom, hover, selected) = (self.graph.clone(), self.positions.clone(), self.pan, self.zoom, self.hover, self.selected);
        let highlight = hover.or(selected);
        let neighbors: std::collections::HashSet<u32> = highlight.map_or_else(Default::default, |h| graph.neighbors(h).into_iter().collect());
        let colors = (t.secondary, t.accent, t.ink, t.separator, t.content);
        let view = canvas(
            move |b, _, _| measured.set(b),
            move |bounds, _, window, cx| {
                let (secondary, accent, ink, line, bg) = colors;
                let origin = bounds.origin;
                let screen = |p: (f32, f32)| point(origin.x + px(p.0 * zoom + pan.0), origin.y + px(p.1 * zoom + pan.1));
                window.paint_layer(bounds, |window| {
                    let mut normal = PathBuilder::stroke(px(1.));
                    let mut strong = PathBuilder::stroke(px(1.5));
                    let (mut any_normal, mut any_strong) = (false, false);
                    for &(a, b) in &graph.edges {
                        let (Some(p), Some(q)) = (positions.get(a as usize), positions.get(b as usize)) else { continue };
                        let (s, e) = (screen(*p), screen(*q));
                        if !bounds.contains(&s) && !bounds.contains(&e) && graph.edges.len() > 2000 {
                            continue;
                        }
                        if highlight == Some(a) || highlight == Some(b) {
                            strong.move_to(s);
                            strong.line_to(e);
                            any_strong = true;
                        } else {
                            normal.move_to(s);
                            normal.line_to(e);
                            any_normal = true;
                        }
                    }
                    if any_normal && let Ok(path) = normal.build() {
                        window.paint_path(path, if highlight.is_some() { line.opacity(0.4) } else { line });
                    }
                    if any_strong && let Ok(path) = strong.build() {
                        window.paint_path(path, accent);
                    }
                    for (i, node) in graph.nodes.iter().enumerate() {
                        let Some(p) = positions.get(i) else { continue };
                        let c = screen(*p);
                        if !bounds.contains(&c) {
                            continue;
                        }
                        let i = i as u32;
                        let r = ((3. + (node.links as f32).sqrt() * 1.6).min(16.) * zoom.max(0.5)).max(2.5);
                        let on = highlight == Some(i);
                        let near = neighbors.contains(&i);
                        let dim = highlight.is_some() && !on && !near;
                        let color = match node.kind {
                            _ if on => accent,
                            NodeKind::Note(_) => secondary,
                            NodeKind::Attachment(_) => ink.opacity(0.35),
                            NodeKind::Tag(_) => accent.opacity(0.7),
                            NodeKind::Missing(_) => bg,
                        };
                        let color = if dim { color.opacity(0.3) } else { color };
                        let hollow = matches!(node.kind, NodeKind::Missing(_));
                        let area = Bounds::new(point(c.x - px(r), c.y - px(r)), size(px(r * 2.), px(r * 2.)));
                        let border = if hollow { secondary } else { transparent_black() };
                        window.paint_quad(PaintQuad { bounds: area, corner_radii: Corners::all(px(r)), background: color.into(), border_widths: Edges::all(px(if hollow { 1. } else { 0. })), border_color: border, border_style: BorderStyle::Solid });
                        if on || near || zoom >= 0.75 {
                            let font_size = px(font::CAPTION);
                            let text: SharedString = node.name.clone().into();
                            let run = TextRun { len: text.len(), font: window.text_style().font(), color: if dim { secondary.opacity(0.4) } else { ink }, background_color: None, underline: None, strikethrough: None };
                            let shaped = window.text_system().shape_line(text, font_size, &[run], None);
                            let at = point(c.x - shaped.width / 2., c.y + px(r + 2.));
                            let _ = shaped.paint(at, px(14.), TextAlign::Left, None, window, cx);
                        }
                    }
                });
            },
        )
        .size_full();
        let empty = count == 0;
        let surface = div()
            .id("graph-surface")
            .track_focus(&self.focus)
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .cursor(if self.hover.is_some() { CursorStyle::PointingHand } else { CursorStyle::Arrow })
            .child(view)
            .when(empty, |d| d.child(div().absolute().inset_0().flex().items_center().justify_center().child(unavailable("waypoints", tr("Noch keine Verknüpfungen"), tr("Sobald Notizen sich verlinken, erscheinen sie hier."), &t))))
            .on_scroll_wheel(cx.listener(|v, e: &ScrollWheelEvent, window, cx| {
                let delta = e.delta.pixel_delta(px(20.));
                if e.modifiers.platform || e.modifiers.control {
                    let o = v.bounds.get().origin;
                    v.zoom_at(1. + f32::from(delta.y) * 0.004, f32::from(e.position.x - o.x), f32::from(e.position.y - o.y));
                } else {
                    v.pan.0 += f32::from(delta.x);
                    v.pan.1 += f32::from(delta.y);
                }
                let _ = window;
                cx.notify();
            }))
            .on_pinch(cx.listener(|v, e: &PinchEvent, _, cx| {
                let o = v.bounds.get().origin;
                v.zoom_at(1. + e.delta, f32::from(e.position.x - o.x), f32::from(e.position.y - o.y));
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|v, e: &MouseDownEvent, window, cx| {
                window.focus(&v.focus, cx);
                if let Some(hit) = v.hit(e.position) {
                    v.selected = Some(hit);
                    v.open(hit, cx);
                } else {
                    v.drag = Some((e.position, v.pan));
                }
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|v, e: &MouseMoveEvent, _, cx| {
                if let Some((start, pan)) = v.drag {
                    v.pan = (pan.0 + f32::from(e.position.x - start.x), pan.1 + f32::from(e.position.y - start.y));
                    cx.notify();
                } else {
                    let hit = v.hit(e.position);
                    if hit != v.hover {
                        v.hover = hit;
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, _| v.drag = None))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|v, _, _, _| v.drag = None))
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, _, cx| {
                match e.keystroke.key.as_str() {
                    "left" => v.step_to_neighbor(-1., 0.),
                    "right" => v.step_to_neighbor(1., 0.),
                    "up" => v.step_to_neighbor(0., -1.),
                    "down" => v.step_to_neighbor(0., 1.),
                    "enter" => {
                        if let Some(s) = v.selected {
                            v.open(s, cx);
                        }
                    }
                    "0" if e.keystroke.modifiers.platform => v.fit(),
                    "escape" => v.selected = None,
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }));
        div().size_full().flex().flex_col().child(bar).child(surface)
    }
}
