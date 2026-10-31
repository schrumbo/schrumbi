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
            graph.nodes.push(GraphNode { kind: NodeKind::Note((*path).clone()), name, links: 0 });
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
                            graph.nodes.push(GraphNode { kind: NodeKind::Attachment(t), name, links: 0 });
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
                            graph.nodes.push(GraphNode { kind: NodeKind::Missing(label.clone()), name: label, links: 0 });
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
                        graph.nodes.push(GraphNode { kind: NodeKind::Tag(key.clone()), name: format!("#{tag}"), links: 0 });
                        graph.nodes.len() as u32 - 1
                    });
                    graph.connect(from, to, &mut seen);
                }
            }
        }
        if !options.orphans {
            graph.drop_orphans();
        }
        graph
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
        Self { nodes, edges }
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
const CELL: f32 = 180.;

/// Force-directed positions. Repulsion only looks at neighboring grid cells, which keeps a step linear in the number of points.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub positions: Vec<(f32, f32)>,
    velocity: Vec<(f32, f32)>,
    /// How much the layout still moves, falling towards zero.
    pub heat: f32,
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
        Self { positions, velocity: vec![(0., 0.); count], heat: 1. }
    }

    /// One step of the simulation.
    pub fn step(&mut self, edges: &[(u32, u32)]) {
        let count = self.positions.len();
        if count == 0 {
            self.heat = 0.;
            return;
        }
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::with_capacity(count);
        let cell_of = |p: (f32, f32)| ((p.0 / CELL).floor() as i32, (p.1 / CELL).floor() as i32);
        for (i, p) in self.positions.iter().enumerate() {
            grid.entry(cell_of(*p)).or_default().push(i as u32);
        }
        let mut force = vec![(0_f32, 0_f32); count];
        for (i, p) in self.positions.iter().enumerate() {
            let (cx, cy) = cell_of(*p);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(list) = grid.get(&(cx + dx, cy + dy)) else { continue };
                    for &j in list {
                        if j as usize == i {
                            continue;
                        }
                        let q = self.positions[j as usize];
                        let (mut ax, mut ay) = (p.0 - q.0, p.1 - q.1);
                        let mut d2 = ax * ax + ay * ay;
                        if d2 < 0.01 {
                            ax = (i as f32 * 0.37).cos();
                            ay = (i as f32 * 0.37).sin();
                            d2 = 1.;
                        }
                        let f = REPULSION / d2;
                        let d = d2.sqrt();
                        force[i].0 += ax / d * f;
                        force[i].1 += ay / d * f;
                    }
                }
            }
            force[i].0 -= p.0 * 0.012;
            force[i].1 -= p.1 * 0.012;
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
        let mut moved = 0.;
        for i in 0..count {
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
