use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::index::VaultIndex;
use crate::core::markdown;

/// What a point stands for.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Note(PathBuf),
    Attachment(PathBuf),
    Missing(String),
    Tag(String),
}

#[derive(Debug, Clone)]
pub struct GraphNode {
    pub kind: NodeKind,
    pub name: String,
    pub links: usize,
    /// The group of notes this point belongs to, an index into `Graph::clusters`.
    pub cluster: Option<u32>,
}

/// Which points the graph shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Options {
    pub tags: bool,
    pub attachments: bool,
    pub missing: bool,
    pub orphans: bool,
    pub filter: String,
}

/// Points and lines between them; line ends are indices into `nodes`.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<(u32, u32)>,
    /// Names of the groups the notes fall into, by folder.
    pub clusters: Vec<String>,
}

impl Graph {
    /// Builds the graph of a whole vault.
    pub fn build(vault: &VaultIndex, options: &Options) -> Self {
        let mut graph = Self::default();
        let mut ids: HashMap<NodeKind2, u32> = HashMap::new();
        let mut seen: HashSet<(u32, u32)> = HashSet::new();
        let filter = options.filter.to_lowercase();
        let mut paths: Vec<&PathBuf> = vault.notes.keys().filter(|p| markdown::is_markdown(p)).collect();
        paths.sort();
        for path in &paths {
            let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            if !filter.is_empty() && !path.to_string_lossy().to_lowercase().contains(&filter) {
                continue;
            }
            let id = graph.nodes.len() as u32;
            ids.insert(NodeKind2::Path((*path).clone()), id);
            graph.nodes.push(GraphNode { kind: NodeKind::Note((*path).clone()), name, links: 0, cluster: None });
        }
        for path in paths {
            let Some(&from) = ids.get(&NodeKind2::Path(path.clone())) else { continue };
            let facts = &vault.notes[path];
            for (link, _, _) in &facts.links {
                let target = vault.resolver.resolve(&link.target, Some(path));
                let to = match target {
                    Some(t) if ids.contains_key(&NodeKind2::Path(t.clone())) => ids[&NodeKind2::Path(t)],
                    Some(t) if !markdown::is_markdown(&t) => {
                        if !options.attachments {
                            continue;
                        }
                        let name = t.file_name().unwrap_or_default().to_string_lossy().to_string();
                        *ids.entry(NodeKind2::Path(t.clone())).or_insert_with(|| {
                            graph.nodes.push(GraphNode { kind: NodeKind::Attachment(t), name, links: 0, cluster: None });
                            graph.nodes.len() as u32 - 1
                        })
                    }
                    Some(_) => continue,
                    None => {
                        let label = link.target.path.clone();
                        if !options.missing || label.is_empty() {
                            continue;
                        }
                        *ids.entry(NodeKind2::Name(label.clone())).or_insert_with(|| {
                            graph.nodes.push(GraphNode { kind: NodeKind::Missing(label.clone()), name: label, links: 0, cluster: None });
                            graph.nodes.len() as u32 - 1
                        })
                    }
                };
                graph.connect(from, to, &mut seen);
            }
            if options.tags {
                for tag in &facts.tags {
                    let key = tag.to_lowercase();
                    let to = *ids.entry(NodeKind2::Tag(key.clone())).or_insert_with(|| {
                        graph.nodes.push(GraphNode { kind: NodeKind::Tag(key.clone()), name: format!("#{tag}"), links: 0, cluster: None });
                        graph.nodes.len() as u32 - 1
                    });
                    graph.connect(from, to, &mut seen);
                }
            }
        }
        if !options.orphans {
            graph.drop_orphans();
        }
        graph.find_clusters(&vault.root);
        graph
    }

    /// Groups notes by their folder. A vault with one folder on top is grouped one level further in.
    fn find_clusters(&mut self, root: &Path) {
        let parts = |path: &Path| -> Vec<String> {
            path.strip_prefix(root).ok().and_then(Path::parent).map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect()).unwrap_or_default()
        };
        let tops: HashSet<String> = self.nodes.iter().filter_map(|n| if let NodeKind::Note(p) = &n.kind { Some(parts(p).first().cloned().unwrap_or_default()) } else { None }).collect();
        let depth = if tops.len() == 1 && tops.iter().next().is_some_and(|t| !t.is_empty()) { 2 } else { 1 };
        let mut names: Vec<String> = Vec::new();
        let mut ids: HashMap<String, u32> = HashMap::new();
        for node in &mut self.nodes {
            let NodeKind::Note(path) = &node.kind else { continue };
            let folder = parts(path);
            let name = folder.get(depth - 1).or_else(|| folder.last()).cloned().unwrap_or_default();
            let id = *ids.entry(name.clone()).or_insert_with(|| {
                names.push(name);
                names.len() as u32 - 1
            });
            node.cluster = Some(id);
        }
        self.clusters = names;
    }

    /// Point indices with the most links first, the order labels get their room in.
    pub fn by_importance(&self) -> Vec<u32> {
        let mut order: Vec<u32> = (0..self.nodes.len() as u32).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.nodes[i as usize].links));
        order
    }

    /// The part of the graph up to `depth` links away from a note.
    pub fn local(&self, center: &Path, depth: usize) -> Self {
        let Some(start) = self.nodes.iter().position(|n| matches!(&n.kind, NodeKind::Note(p) if p == center)) else { return Self::default() };
        let mut keep = HashSet::from([start as u32]);
        let mut frontier = vec![start as u32];
        for _ in 0..depth {
            let mut next = Vec::new();
            for &(a, b) in &self.edges {
                if frontier.contains(&a) && keep.insert(b) {
                    next.push(b);
                }
                if frontier.contains(&b) && keep.insert(a) {
                    next.push(a);
                }
            }
            frontier = next;
        }
        self.subset(|i| keep.contains(&i))
    }

    fn connect(&mut self, a: u32, b: u32, seen: &mut HashSet<(u32, u32)>) {
        if a == b || !seen.insert((a.min(b), a.max(b))) {
            return;
        }
        self.edges.push((a, b));
        self.nodes[a as usize].links += 1;
        self.nodes[b as usize].links += 1;
    }

    fn drop_orphans(&mut self) {
        let keep: Vec<bool> = self.nodes.iter().map(|n| n.links > 0).collect();
        *self = self.subset(|i| keep[i as usize]);
    }

    fn subset(&self, keep: impl Fn(u32) -> bool) -> Self {
        let mut map = vec![u32::MAX; self.nodes.len()];
        let mut nodes = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if keep(i as u32) {
                map[i] = nodes.len() as u32;
                nodes.push(n.clone());
            }
        }
        let edges = self.edges.iter().filter(|(a, b)| keep(*a) && keep(*b)).map(|(a, b)| (map[*a as usize], map[*b as usize])).collect();
        Self { nodes, edges, clusters: self.clusters.clone() }
    }

    /// Indices of the points directly connected to `index`.
    pub fn neighbors(&self, index: u32) -> Vec<u32> {
        self.edges.iter().filter_map(|&(a, b)| if a == index { Some(b) } else if b == index { Some(a) } else { None }).collect()
    }
}

#[derive(Hash, PartialEq, Eq)]
enum NodeKind2 {
    Path(PathBuf),
    Name(String),
    Tag(String),
}

const SPRING: f32 = 80.;
const REPULSION: f32 = 6000.;

/// How far away a group of points may count as one when pushing.
const THETA: f32 = 0.9;
/// How strongly notes keep to their folder's group.
const CLUSTER_PULL: f32 = 0.035;

/// One cell of the tree that sums up far away points.
#[derive(Clone, Copy)]
struct Cell {
    center: (f32, f32),
    half: f32,
    mass: f32,
    sum: (f32, f32),
    point: Option<u32>,
    first_child: u32,
}

impl Cell {
    fn empty(center: (f32, f32), half: f32) -> Self {
        Self { center, half, mass: 0., sum: (0., 0.), point: None, first_child: 0 }
    }
}

/// A Barnes-Hut tree over all points, so one step stays near-linear however large the vault is.
struct PointTree {
    cells: Vec<Cell>,
}

impl PointTree {
    fn build(points: &[(f32, f32)]) -> Self {
        let first = points.first().copied().unwrap_or((0., 0.));
        let (a, b, c, d) = points.iter().fold((first.0, first.1, first.0, first.1), |(a, b, c, d), p| (a.min(p.0), b.min(p.1), c.max(p.0), d.max(p.1)));
        let half = ((c - a).max(d - b) / 2.).max(1.) + 1.;
        let mut tree = Self { cells: Vec::with_capacity(points.len() * 2) };
        tree.cells.push(Cell::empty(((a + c) / 2., (b + d) / 2.), half));
        for (i, p) in points.iter().enumerate() {
            tree.insert(i as u32, *p, points);
        }
        tree
    }

    fn insert(&mut self, index: u32, p: (f32, f32), points: &[(f32, f32)]) {
        let mut at = 0_usize;
        for _ in 0..28 {
            self.cells[at].mass += 1.;
            self.cells[at].sum.0 += p.0;
            self.cells[at].sum.1 += p.1;
            if self.cells[at].mass == 1. {
                self.cells[at].point = Some(index);
                return;
            }
            if self.cells[at].first_child == 0 {
                let Cell { center, half, .. } = self.cells[at];
                let quarter = half / 2.;
                self.cells[at].first_child = self.cells.len() as u32;
                for (dx, dy) in [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)] {
                    self.cells.push(Cell::empty((center.0 + dx * quarter, center.1 + dy * quarter), quarter));
                }
                if let Some(old) = self.cells[at].point.take() {
                    let q = points[old as usize];
                    let child = self.child_of(at, q);
                    let cell = &mut self.cells[child];
                    cell.mass = 1.;
                    cell.sum = q;
                    cell.point = Some(old);
                }
            }
            at = self.child_of(at, p);
        }
    }

    fn child_of(&self, at: usize, p: (f32, f32)) -> usize {
        let cell = &self.cells[at];
        cell.first_child as usize + usize::from(p.0 >= cell.center.0) + usize::from(p.1 >= cell.center.1) * 2
    }

    /// The push all other points give the point `index` at `p`.
    fn push_on(&self, index: u32, p: (f32, f32), stack: &mut Vec<u32>) -> (f32, f32) {
        let mut force = (0., 0.);
        stack.clear();
        stack.push(0);
        while let Some(at) = stack.pop() {
            let cell = &self.cells[at as usize];
            if cell.mass == 0. || cell.point == Some(index) {
                continue;
            }
            let center = (cell.sum.0 / cell.mass, cell.sum.1 / cell.mass);
            let (mut dx, mut dy) = (p.0 - center.0, p.1 - center.1);
            let mut d2 = dx * dx + dy * dy;
            let far = (cell.half * 2.).powi(2) < THETA * THETA * d2;
            if far || cell.first_child == 0 {
                if d2 < 0.01 {
                    dx = (index as f32 * 0.37).cos();
                    dy = (index as f32 * 0.37).sin();
                    d2 = 1.;
                }
                let d = d2.sqrt();
                let f = REPULSION * cell.mass / d2;
                force.0 += dx / d * f;
                force.1 += dy / d * f;
            } else {
                for child in 0..4 {
                    stack.push(cell.first_child + child);
                }
            }
        }
        force
    }
}

/// Force-directed positions.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub positions: Vec<(f32, f32)>,
    velocity: Vec<(f32, f32)>,
    /// How much the layout still moves, falling towards zero.
    pub heat: f32,
    /// Which group each point belongs to; groups keep together.
    clusters: Vec<Option<u32>>,
    /// A point being held by the user does not move.
    pub held: Option<u32>,
}

impl Layout {
    /// Starts on a sunflower spiral, so the same graph always opens the same way; known positions stay.
    pub fn new(count: usize, known: impl Fn(usize) -> Option<(f32, f32)>) -> Self {
        let positions = (0..count)
            .map(|i| {
                known(i).unwrap_or_else(|| {
                    let angle = i as f32 * 2.399_963;
                    let radius = 24. * (i as f32 + 1.).sqrt();
                    (radius * angle.cos(), radius * angle.sin())
                })
            })
            .collect();
        Self { positions, velocity: vec![(0., 0.); count], heat: 1., clusters: Vec::new(), held: None }
    }

    /// Starts each folder's notes on their own island, so the first picture already reads.
    pub fn for_graph(graph: &Graph, known: impl Fn(usize) -> Option<(f32, f32)>) -> Self {
        let groups = graph.clusters.len().max(1) as f32;
        let ring = 60. * (graph.nodes.len() as f32).sqrt().max(4.);
        let mut seen = vec![0_u32; graph.clusters.len() + 1];
        let mut layout = Self::new(graph.nodes.len(), |_| None);
        for (i, node) in graph.nodes.iter().enumerate() {
            layout.positions[i] = known(i).unwrap_or_else(|| {
                let group = node.cluster.map_or(graph.clusters.len(), |c| c as usize);
                let angle = group as f32 / groups * std::f32::consts::TAU;
                let nth = seen[group];
                seen[group] += 1;
                let spin = nth as f32 * 2.399_963;
                let radius = 18. * (nth as f32 + 1.).sqrt();
                (ring * angle.cos() + radius * spin.cos(), ring * angle.sin() + radius * spin.sin())
            });
        }
        layout.clusters = graph.nodes.iter().map(|n| n.cluster).collect();
        layout
    }

    /// One step of the simulation.
    pub fn step(&mut self, edges: &[(u32, u32)]) {
        let count = self.positions.len();
        if count == 0 {
            self.heat = 0.;
            return;
        }
        let tree = PointTree::build(&self.positions);
        let mut stack = Vec::with_capacity(128);
        let mut force = vec![(0_f32, 0_f32); count];
        for (i, p) in self.positions.iter().enumerate() {
            let push = tree.push_on(i as u32, *p, &mut stack);
            force[i].0 += push.0 - p.0 * 0.012;
            force[i].1 += push.1 - p.1 * 0.012;
        }
        for &(a, b) in edges {
            let (p, q) = (self.positions[a as usize], self.positions[b as usize]);
            let (dx, dy) = (q.0 - p.0, q.1 - p.1);
            let d = (dx * dx + dy * dy).sqrt().max(0.1);
            let f = (d - SPRING) * 0.04;
            let (fx, fy) = (dx / d * f, dy / d * f);
            force[a as usize].0 += fx;
            force[a as usize].1 += fy;
            force[b as usize].0 -= fx;
            force[b as usize].1 -= fy;
        }
        if !self.clusters.is_empty() {
            let groups = self.clusters.iter().flatten().max().map_or(0, |m| *m as usize + 1);
            let mut sum = vec![(0_f32, 0_f32, 0_f32); groups];
            for (i, group) in self.clusters.iter().enumerate() {
                if let Some(g) = group {
                    let s = &mut sum[*g as usize];
                    s.0 += self.positions[i].0;
                    s.1 += self.positions[i].1;
                    s.2 += 1.;
                }
            }
            for (i, group) in self.clusters.iter().enumerate() {
                if let Some(g) = group {
                    let s = sum[*g as usize];
                    force[i].0 += (s.0 / s.2 - self.positions[i].0) * CLUSTER_PULL;
                    force[i].1 += (s.1 / s.2 - self.positions[i].1) * CLUSTER_PULL;
                }
            }
        }
        let mut moved = 0.;
        for i in 0..count {
            if self.held == Some(i as u32) {
                self.velocity[i] = (0., 0.);
                continue;
            }
            let v = &mut self.velocity[i];
            v.0 = (v.0 + force[i].0) * 0.6;
            v.1 = (v.1 + force[i].1) * 0.6;
            let speed = (v.0 * v.0 + v.1 * v.1).sqrt();
            let limit = 40. * self.heat.max(0.05);
            let scale = if speed > limit { limit / speed } else { 1. };
            self.positions[i].0 += v.0 * scale;
            self.positions[i].1 += v.1 * scale;
            moved += speed.min(limit);
        }
        self.heat = (self.heat * 0.985).min((moved / count as f32 / 4.).max(0.)).max(0.);
    }

    /// Runs until the layout rests or the step limit is reached.
    pub fn settle(&mut self, edges: &[(u32, u32)], limit: usize) {
        for _ in 0..limit {
            if self.heat < 0.01 {
                break;
            }
            self.step(edges);
        }
    }

    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let first = *self.positions.first()?;
        Some(self.positions.iter().fold((first.0, first.1, first.0, first.1), |(a, b, c, d), p| (a.min(p.0), b.min(p.1), c.max(p.0), d.max(p.1))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::vault::index::{facts_of, rebuild_for_test};

    fn fixture() -> (PathBuf, VaultIndex) {
        let root = std::env::temp_dir().join(format!("schrumbi-graph-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("A.md"), "[[B]] [[Fehlt]] ![[bild.png]] #thema").unwrap();
        std::fs::write(root.join("B.md"), "[[C]] [[A]]").unwrap();
        std::fs::write(root.join("C.md"), "x").unwrap();
        std::fs::write(root.join("Allein.md"), "x").unwrap();
        std::fs::write(root.join("bild.png"), "x").unwrap();
        let index = rebuild_for_test(&root, |t| facts_of(t, 0.));
        (root, index)
    }

    fn names(g: &Graph) -> Vec<String> {
        let mut v: Vec<String> = g.nodes.iter().map(|n| n.name.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn options_decide_what_shows() {
        let (_, index) = fixture();
        let plain = Graph::build(&index, &Options::default());
        assert_eq!(names(&plain), ["A", "B", "C"]);
        assert_eq!(plain.edges.len(), 2, "A→B and B→A count once");
        let full = Graph::build(&index, &Options { tags: true, attachments: true, missing: true, orphans: true, filter: String::new() });
        assert_eq!(names(&full), ["#thema", "A", "Allein", "B", "C", "Fehlt", "bild.png"]);
        let filtered = Graph::build(&index, &Options { orphans: true, filter: "alle".into(), ..Default::default() });
        assert_eq!(names(&filtered), ["Allein"]);
    }

    #[test]
    fn local_graph_respects_depth() {
        let (root, index) = fixture();
        let graph = Graph::build(&index, &Options::default());
        let center = root.join("A.md");
        assert_eq!(names(&graph.local(&center, 1)), ["A", "B"]);
        assert_eq!(names(&graph.local(&center, 2)), ["A", "B", "C"]);
    }

    #[test]
    fn layout_spreads_and_comes_to_rest() {
        let edges: Vec<(u32, u32)> = (1..50).map(|i| (0, i)).collect();
        let mut layout = Layout::new(50, |_| None);
        layout.settle(&edges, 600);
        assert!(layout.heat < 0.5, "heat {}", layout.heat);
        let (a, b, c, d) = layout.bounds().unwrap();
        assert!(c - a > 80. && d - b > 80.);
        assert!(layout.positions.iter().all(|p| p.0.is_finite() && p.1.is_finite()));
    }

    #[test]
    fn a_big_vault_steps_quickly() {
        let edges: Vec<(u32, u32)> = (1..5000).map(|i| (i / 3, i)).collect();
        let mut layout = Layout::new(5000, |_| None);
        let start = std::time::Instant::now();
        for _ in 0..10 {
            layout.step(&edges);
        }
        assert!(start.elapsed().as_millis() < 5000, "{:?}", start.elapsed());
    }
}
