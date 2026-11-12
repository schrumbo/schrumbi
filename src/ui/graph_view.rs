use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::{self, act};
use super::theme::{Theme, font, radius, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::shortcuts::ShortcutAction;
use crate::core::vault::graph::{Graph, Layout, NodeKind, Options};
use crate::core::vault::index;

const HIT_MIN: f32 = 12.;

/// Positions stay for the whole session, so returning to the graph looks the same.
#[derive(Default)]
struct KnownPositions(HashMap<String, (f32, f32)>);

impl Global for KnownPositions {}

/// The graph of the vault or around one note.
pub struct GraphView {
    options: Options,
    focus_note: Option<(PathBuf, usize)>,
    graph: Arc<Graph>,
    tints: Vec<Option<Hsla>>,
    layout: Arc<Mutex<Layout>>,
    positions: Vec<(f32, f32)>,
    generation: u64,
    built_for: Option<(Options, Option<(PathBuf, usize)>)>,
    pan: (f32, f32),
    zoom: f32,
    fitted: bool,
    /// Set once the user panned or zoomed; from then on the view stays where it is.
    hand_moved: bool,
    /// The point pressed, to tell a click from dragging.
    pressed: Option<(u32, Point<Pixels>)>,
    bounds: Rc2,
    hover: Option<u32>,
    selected: Option<u32>,
    drag: Option<(Point<Pixels>, (f32, f32))>,
    /// A point being carried to another place.
    carried: Option<u32>,
    double_press: bool,
    /// Point indices by how many links they have, the order labels get room in.
    order: Arc<Vec<u32>>,
    /// Each group's colour, by the module it is or by its turn.
    cluster_tints: Vec<Hsla>,
    filter: Entity<InputState>,
    focus: FocusHandle,
    running: Option<Task<()>>,
    building: Option<Task<()>>,
    missing: (u64, Arc<Vec<(String, usize)>>),
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
            tints: Vec::new(),
            layout: Arc::default(),
            positions: Vec::new(),
            generation: 0,
            built_for: None,
            pan: (0., 0.),
            zoom: 1.,
            fitted: false,
            hand_moved: false,
            pressed: None,
            bounds: Default::default(),
            hover: None,
            selected: None,
            drag: None,
            carried: None,
            double_press: false,
            order: Arc::default(),
            cluster_tints: Vec::new(),
            filter,
            focus: cx.focus_handle(),
            running: None,
            building: None,
            missing: (u64::MAX, Arc::default()),
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
        let wanted_options = if wanted.is_some() { Options { orphans: true, ..self.options.clone() } } else { self.options.clone() };
        let known = cx.try_global::<KnownPositions>().map(|k| k.0.clone()).unwrap_or_default();
        let focus = wanted.clone();
        let task = cx.background_spawn(async move {
            let full = Graph::build(&vault, &wanted_options);
            let graph = Arc::new(match &focus {
                Some((path, depth)) => full.local(path, *depth),
                None => full,
            });
            let order = Arc::new(graph.by_importance());
            let nodes = &graph.nodes;
            let mut layout = Layout::for_graph(&graph, |i| known.get(&nodes[i].name).copied());
            if nodes.iter().all(|n| known.contains_key(&n.name)) && !nodes.is_empty() {
                layout.heat = 0.2;
            }
            (graph, order, layout)
        });
        self.building = Some(cx.spawn(async move |this, cx| {
            let (graph, order, layout) = task.await;
            let _ = this.update(cx, |v, cx| {
                v.building = None;
                v.adopt(graph, order, layout, wanted, cx);
                cx.notify();
            });
        }));
    }

    fn adopt(&mut self, graph: Arc<Graph>, order: Arc<Vec<u32>>, layout: Layout, wanted: Option<(PathBuf, usize)>, cx: &mut Context<Self>) {
        let vault = index::current();
        self.focus_note = wanted;
        let t = Theme::get(cx);
        let modules: Vec<(String, Hsla)> = state::read(cx, |l| l.config.as_ref().map_or(Vec::new(), |c| c.modules.iter().map(|m| (m.folder.rsplit('/').next().unwrap_or(&m.folder).to_string(), t.palette(&m.color))).collect()));
        const TURNS: [&str; 8] = ["blue", "teal", "orange", "pink", "purple", "green", "brown", "indigo"];
        self.cluster_tints = graph.clusters.iter().enumerate().map(|(i, name)| modules.iter().find(|(f, _)| f == name).map_or_else(|| t.palette(TURNS[i % TURNS.len()]), |(_, c)| *c)).collect();
        self.tints = graph.nodes.iter().map(|n| n.cluster.and_then(|c| self.cluster_tints.get(c as usize).copied())).collect();
        self.order = order;
        self.positions = layout.positions.clone();
        self.graph = graph.clone();
        self.layout = Arc::new(Mutex::new(layout));
        let picked = state::read(cx, |l| l.graph_selection.clone());
        self.selected = picked.as_ref().and_then(|p| self.graph.nodes.iter().position(|n| matches!(&n.kind, NodeKind::Note(q) if q == p))).map(|i| i as u32);
        if self.selected.is_none() && picked.as_ref().is_some_and(|p| !vault.notes.contains_key(p)) {
            cx.defer(|cx| act(cx, |l| l.graph_selection = None));
        }
        self.hover = None;
        self.fitted = false;
        self.hand_moved = false;
        self.fit();
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
                        if !v.hand_moved {
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
            let r = (self.radius(i as u32) * self.zoom.clamp(0.25, 1.5) + 2.).max(HIT_MIN);
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

    /// Zooms around the middle of the view.
    fn zoom_by(&mut self, factor: f32, cx: &mut Context<Self>) {
        let size = self.bounds.get().size;
        self.hand_moved = true;
        self.zoom_at(factor, f32::from(size.width) / 2., f32::from(size.height) / 2.);
        cx.notify();
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

    /// A filter in the menu.
    fn filter_item(menu: gpui_kit::component::menu::PopupMenu, me: &Entity<Self>, label: String, on: bool, set: fn(&mut Options, bool)) -> gpui_kit::component::menu::PopupMenu {
        let me = me.clone();
        let item = PopupMenuItem::new(label).checked(on);
        let item = if on { item } else { item.icon(Icon::empty()) };
        menu.item(item.on_click(move |_, _, cx| {
            me.update(cx, |g, cx| {
                set(&mut g.options, !on);
                cx.notify();
            })
        }))
    }

    fn deselect(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        act(cx, |l| l.graph_selection = None);
    }

    /// Selecting shows the note beside the graph; the graph stays where it is.
    fn select(&mut self, index: u32, cx: &mut Context<Self>) {
        self.selected = Some(index);
        let picked = self.graph.nodes.get(index as usize).map(|n| n.kind.clone());
        act(cx, |l| {
            l.graph_selection = match picked {
                Some(NodeKind::Note(path)) => Some(path),
                _ => None,
            };
        });
    }
}

impl Render for GraphView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.rebuild_if_needed(cx);
        if let Some(i) = self.selected
            && matches!(self.graph.nodes.get(i as usize).map(|n| &n.kind), Some(NodeKind::Note(_)))
            && state::read(cx, |l| l.graph_selection.is_none())
        {
            self.selected = None;
        }
        let t = Theme::get(cx);
        let count = self.graph.nodes.len();
        let local = self.focus_note.clone();
        let vault = index::current();
        let missing_total = vault.missing.len();
        if self.missing.0 != vault.generation {
            let mut names: Vec<(String, usize)> = vault.missing.iter().map(|(name, from)| (name.clone(), from.len())).collect();
            names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            names.truncate(30);
            self.missing = (vault.generation, Arc::new(names));
        }
        let missing_names = self.missing.1.clone();
        let bar = div()
            .flex_none()
            .px(px(space::M))
            .py(px(space::XS))
            .min_h(px(super::theme::control::SUBBAR))
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(space::M))
            .border_b_1()
            .border_color(t.separator)
            .child(div().w(px(220.)).child(Input::new(&self.filter).small()))
            .child(Button::new("graph-filters").ghost().small().icon(menu_icon("sliders-horizontal")).label(tr("Anzeigen")).dropdown_menu({
                let (me, options) = (cx.entity(), self.options.clone());
                move |menu, _, _| {
                    let menu = Self::filter_item(menu, &me, tr("Tags"), options.tags, |o, v| o.tags = v);
                    let menu = Self::filter_item(menu, &me, tr("Anhänge"), options.attachments, |o, v| o.attachments = v);
                    let menu = Self::filter_item(menu, &me, tr("Nicht aufgelöst"), options.missing, |o, v| o.missing = v);
                    Self::filter_item(menu, &me, tr("Ohne Links"), options.orphans, |o, v| o.orphans = v)
                }
            }))
            .children((missing_total > 0).then(|| {
                let names = missing_names.clone();
                let (me, shown) = (cx.entity(), self.options.missing);
                Button::new("graph-missing-list").ghost().small().label(trf("Nicht aufgelöst: %@", &[&missing_total.to_string()])).tooltip(tr("Links auf Notizen, die es nicht gibt")).dropdown_menu(move |menu, _, _| {
                    let mut menu = Self::filter_item(menu, &me, tr("Im Graph zeigen"), shown, |o, v| o.missing = v).separator();
                    for (name, count) in names.iter() {
                        let target = name.clone();
                        menu = menu.item(PopupMenuItem::new(format!("{name} · {count}")).on_click(move |_, _, cx| act(cx, |l| l.create_link_target(&target, None))));
                    }
                    menu
                })
            }))
            .child(Button::new("graph-zoom-out").ghost().small().icon(menu_icon("minus")).accessibility_label(tr("Verkleinern")).tooltip(tr("Verkleinern")).on_click(cx.listener(|v, _, _, cx| v.zoom_by(0.75, cx))))
            .child(Button::new("graph-zoom-in").ghost().small().icon(menu_icon("plus")).accessibility_label(tr("Vergrößern")).tooltip(tr("Vergrößern")).on_click(cx.listener(|v, _, _, cx| v.zoom_by(1.33, cx))))
            .child(Button::new("graph-fit").ghost().small().icon(menu_icon("maximize")).accessibility_label(tr("Einpassen")).tooltip(format!("{}{}", tr("Einpassen"), ShortcutAction::ZoomReset.hint())).on_click(cx.listener(|v, _, _, cx| {
                v.hand_moved = false;
                v.fit();
                cx.notify();
            })))
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
        let tints = self.tints.clone();
        let order = self.order.clone();
        let colors = (t.secondary, t.accent, t.ink, t.separator, t.content, t.dark);
        let view = canvas(
            move |b, _, _| measured.set(b),
            move |bounds, _, window, cx| {
                let (secondary, accent, ink, line, bg, dark) = colors;
                let own = |i: usize, kind: &NodeKind| match kind {
                    NodeKind::Note(_) => tints.get(i).copied().flatten().unwrap_or(secondary),
                    NodeKind::Attachment(_) => ink.opacity(0.35),
                    NodeKind::Tag(_) => accent,
                    NodeKind::Missing(_) => secondary,
                };
                let lift = |c: Hsla| if dark { Hsla { l: c.l + (1. - c.l) * 0.35, ..c } } else { Hsla { l: c.l * 0.72, ..c } };
                let edge_color = highlight.and_then(|h| graph.nodes.get(h as usize).map(|n| own(h as usize, &n.kind))).unwrap_or(accent);
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
                        let quiet = graph.edges.len() > 6000 && zoom < 0.3 && graph.nodes[a as usize].links + graph.nodes[b as usize].links < 12;
                        if quiet && highlight != Some(a) && highlight != Some(b) {
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
                        let density = (1500. / graph.edges.len().max(1) as f32).clamp(0.14, 1.);
                        let fade = density * (zoom * 2.5).clamp(0.5, 1.);
                        window.paint_path(path, ink.opacity(if highlight.is_some() { fade * 0.12 } else { fade * 0.3 }));
                    }
                    if any_strong && let Ok(path) = strong.build() {
                        window.paint_path(path, edge_color);
                    }
                });
                window.paint_layer(bounds, |window| {
                    for (i, node) in graph.nodes.iter().enumerate() {
                        let Some(p) = positions.get(i) else { continue };
                        let c = screen(*p);
                        if !bounds.contains(&c) {
                            continue;
                        }
                        let i = i as u32;
                        let r = ((3. + (node.links as f32).sqrt() * 1.6).min(16.) * zoom.clamp(0.25, 1.5)).max(1.5);
                        let on = highlight == Some(i);
                        let near = neighbors.contains(&i);
                        let dim = highlight.is_some() && !on && !near;
                        let hollow = matches!(node.kind, NodeKind::Missing(_));
                        let base = if hollow { bg } else { own(i as usize, &node.kind) };
                        let color = if on { lift(base) } else { base };
                        let color = if dim { color.opacity(0.3) } else { color };
                        let r = if on { r * 1.3 } else { r };
                        let area = Bounds::new(point(c.x - px(r), c.y - px(r)), size(px(r * 2.), px(r * 2.)));
                        let border = if hollow { secondary } else { transparent_black() };
                        if on {
                            let halo = r + 3.;
                            window.paint_quad(PaintQuad { bounds: Bounds::new(point(c.x - px(halo), c.y - px(halo)), size(px(halo * 2.), px(halo * 2.))), corner_radii: Corners::all(px(halo)), background: transparent_black().into(), border_widths: Edges::all(px(1.5)), border_color: own(i as usize, &node.kind).opacity(0.6), border_style: BorderStyle::Solid });
                        }
                        window.paint_quad(PaintQuad { bounds: area, corner_radii: Corners::all(px(r)), background: color.into(), border_widths: Edges::all(px(if hollow { 1. } else { 0. })), border_color: border, border_style: BorderStyle::Solid });
                    }
                    let mut taken: Vec<(f32, f32, f32, f32)> = Vec::new();
                    let mut label = |window: &mut Window, cx: &mut App, text: String, at: Point<Pixels>, color: Hsla, bold: bool, force: bool, pill: bool| {
                        let text_size = if bold { font::CALLOUT } else { font::CAPTION };
                        let (x, y) = (f32::from(at.x), f32::from(at.y));
                        let (pad_x, pad_y) = if pill { (space::S, space::XXS) } else { (space::XS, 0.) };
                        let area = |w: f32| (x - w / 2. - pad_x, y - pad_y, x + w / 2. + pad_x, y + 16. + pad_y);
                        let clear = |r: (f32, f32, f32, f32), taken: &[(f32, f32, f32, f32)]| !taken.iter().any(|t| r.0 < t.2 && r.2 > t.0 && r.1 < t.3 && r.3 > t.1);
                        if !force && !clear(area(text.chars().count() as f32 * text_size * 0.4), &taken) {
                            return false;
                        }
                        let mut font = window.text_style().font();
                        if bold {
                            font.weight = FontWeight::SEMIBOLD;
                        }
                        let text: SharedString = text.into();
                        let run = TextRun { len: text.len(), font, color, background_color: None, underline: None, strikethrough: None };
                        let shaped = window.text_system().shape_line(text, px(text_size), &[run], None);
                        let rect = area(f32::from(shaped.width));
                        if !force && !clear(rect, &taken) {
                            return false;
                        }
                        taken.push(rect);
                        if pill {
                            window.paint_quad(fill(Bounds::new(point(px(rect.0), px(rect.1)), size(px(rect.2 - rect.0), px(rect.3 - rect.1))), bg).corner_radii(px(radius::SMALL)));
                        }
                        let _ = shaped.paint(point(at.x - shaped.width / 2., at.y), px(16.), TextAlign::Left, None, window, cx);
                        true
                    };
                    if zoom < 0.8 && highlight.is_none() {
                        let mut sums = vec![(0_f32, 0_f32, 0_u32); graph.clusters.len()];
                        for (i, node) in graph.nodes.iter().enumerate() {
                            if let (Some(c), Some(p)) = (node.cluster, positions.get(i)) {
                                let s = &mut sums[c as usize];
                                s.0 += p.0;
                                s.1 += p.1;
                                s.2 += 1;
                            }
                        }
                        let mut titled: Vec<usize> = (0..sums.len()).filter(|&c| sums[c].2 >= 2 && !graph.clusters[c].is_empty()).collect();
                        titled.sort_by_key(|&c| std::cmp::Reverse(sums[c].2));
                        for c in titled {
                            let s = &sums[c];
                            let center = screen((s.0 / s.2 as f32, s.1 / s.2 as f32));
                            label(window, cx, format!("{} · {}", graph.clusters[c], s.2), center, ink, true, false, true);
                        }
                    }
                    if let Some(h) = highlight {
                        let near_first = order.iter().copied().filter(|i| neighbors.contains(i)).take(40);
                        for i in std::iter::once(h).chain(near_first) {
                            let (Some(p), Some(node)) = (positions.get(i as usize), graph.nodes.get(i as usize)) else { continue };
                            let c = screen(*p);
                            label(window, cx, node.name.clone(), point(c.x, c.y + px(space::S + space::XXS)), ink, i == h, i == h, false);
                        }
                    } else if zoom >= 0.3 {
                        let budget = if zoom >= 0.8 { 140 } else { 40 };
                        let (mut shown, mut tried) = (0, 0);
                        for &i in order.iter() {
                            if shown >= budget || tried >= 600 {
                                break;
                            }
                            let (Some(p), Some(node)) = (positions.get(i as usize), graph.nodes.get(i as usize)) else { continue };
                            let c = screen(*p);
                            if !bounds.contains(&c) {
                                continue;
                            }
                            tried += 1;
                            if label(window, cx, node.name.clone(), point(c.x, c.y + px(space::S)), secondary, false, false, false) {
                                shown += 1;
                            }
                        }
                    }
                });
            },
        )
        .size_full();
        let building = self.building.is_some() && count == 0;
        let empty = count == 0 && !building;
        let unlinked_hidden = !building && !self.options.orphans && self.focus_note.is_none() && count < 3 && vault.notes.keys().filter(|p| crate::core::markdown::is_markdown(p)).nth(count).is_some();
        let surface = div()
            .id("graph-surface")
            .track_focus(&self.focus)
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .cursor(if self.hover.is_some() { CursorStyle::PointingHand } else { CursorStyle::Arrow })
            .child(view)
            .when(building, |d| d.child(div().absolute().inset_0().flex().items_center().justify_center().child(super::motion::loading(super::theme::iconsize::L, t.secondary))))
            .when(empty, |d| d.child(div().absolute().inset_0().flex().items_center().justify_center().child(unavailable("waypoints", tr("Noch keine Verknüpfungen"), tr("Sobald Notizen sich verlinken, erscheinen sie hier."), &t))))
            .when(unlinked_hidden, |d| d.child(div().absolute().left_0().right_0().bottom(px(space::L)).flex().justify_center().child(caption(tr("Notizen ohne Verknüpfung: Anzeigen → Ohne Links"), &t))))
            .on_scroll(cx.listener(|v, e: &ScrollEvent, window, cx| {
                let delta = e.delta.pixel_delta(px(20.));
                v.hand_moved = true;
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
                v.hand_moved = true;
                v.zoom_at(1. + e.delta, f32::from(e.position.x - o.x), f32::from(e.position.y - o.y));
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|v, e: &MouseDownEvent, window, cx| {
                window.focus(&v.focus, cx);
                if let Some(hit) = v.hit(e.position) {
                    v.pressed = Some((hit, e.position));
                    v.double_press = e.click_count >= 2;
                } else {
                    v.drag = Some((e.position, v.pan));
                }
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|v, e: &MouseMoveEvent, _, cx| {
                if let Some((start, pan)) = v.drag {
                    v.hand_moved = true;
                    v.pan = (pan.0 + f32::from(e.position.x - start.x), pan.1 + f32::from(e.position.y - start.y));
                    cx.notify();
                } else if let Some((hit, at)) = v.pressed {
                    if v.carried.is_some() || (f32::from(e.position.x - at.x).abs() + f32::from(e.position.y - at.y).abs()) >= 6. {
                        v.carried = Some(hit);
                        let o = v.bounds.get().origin;
                        let spot = ((f32::from(e.position.x - o.x) - v.pan.0) / v.zoom, (f32::from(e.position.y - o.y) - v.pan.1) / v.zoom);
                        v.hand_moved = true;
                        if let Some(p) = v.positions.get_mut(hit as usize) {
                            *p = spot;
                        }
                        if let Ok(mut layout) = v.layout.lock()
                            && let Some(p) = layout.positions.get_mut(hit as usize)
                        {
                            *p = spot;
                        }
                        cx.notify();
                    }
                } else {
                    let hit = v.hit(e.position);
                    if hit != v.hover {
                        v.hover = hit;
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|v, e: &MouseUpEvent, _, cx| {
                if let Some((start, _)) = v.drag.take()
                    && (f32::from(e.position.x - start.x).abs() + f32::from(e.position.y - start.y).abs()) < 6.
                    && (v.selected.is_some() || state::read(cx, |l| l.graph_selection.is_some()))
                {
                    v.deselect(cx);
                }
                let carried = v.carried.take().is_some();
                if carried {
                    v.remember(cx);
                }
                if let Some((hit, at)) = v.pressed.take()
                    && !carried
                    && (f32::from(e.position.x - at.x).abs() + f32::from(e.position.y - at.y).abs()) < 6.
                {
                    if std::mem::take(&mut v.double_press) {
                        v.open(hit, cx);
                    } else {
                        v.select(hit, cx);
                    }
                }
            }))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|v, _, _, _| {
                v.drag = None;
                v.pressed = None;
                v.carried = None;
            }))
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, _, cx| {
                let before = v.selected;
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
                    "0" if e.keystroke.modifiers.platform || e.keystroke.modifiers.control => {
                        v.hand_moved = false;
                        v.fit();
                    }
                    "escape" if v.selected.is_some() || state::read(cx, |l| l.graph_selection.is_some()) => v.deselect(cx),
                    _ => return,
                }
                if let Some(i) = v.selected.filter(|i| before != Some(*i)) {
                    v.select(i, cx);
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let mut sizes: Vec<(usize, usize)> = (0..self.graph.clusters.len()).map(|c| (self.graph.nodes.iter().filter(|n| n.cluster == Some(c as u32)).count(), c)).collect();
        sizes.sort_by_key(|s| std::cmp::Reverse(s.0));
        let legend = sizes.iter().take(8).filter(|(n, c)| *n > 0 && !self.graph.clusters[*c].is_empty()).map(|(n, c)| {
            div()
                .flex()
                .items_center()
                .gap(px(space::XS))
                .child(div().size(px(space::S)).rounded_full().bg(self.cluster_tints.get(*c).copied().unwrap_or(t.secondary)))
                .child(div().max_w(px(140.)).truncate().child(format!("{} · {n}", self.graph.clusters[*c])))
        });
        let legend = div().absolute().left(px(space::M)).bottom(px(space::M)).flex().flex_col().gap(px(space::XXS)).p(px(space::S)).rounded(px(super::theme::radius::CONTROL)).bg(t.content.opacity(0.85)).text_size(px(font::CAPTION)).text_color(t.secondary).children(legend.collect::<Vec<_>>());
        div().size_full().flex().flex_col().child(bar).child(surface.when(self.graph.clusters.len() > 1, |d| d.child(legend)))
    }
}
