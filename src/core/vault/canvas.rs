use serde_json::{Map, Value};

use super::index::NoteFacts;
use super::links::{FoundLink, LinkTarget};

/// A JSON Canvas document. Unknown fields stay in the raw values, so saving loses nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Canvas {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Top-level fields other than `nodes` and `edges`.
    extra: Map<String, Value>,
}

/// What a node shows.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Text(String),
    File { path: String, subpath: Option<String> },
    Link(String),
    Group { label: Option<String>, background: Option<String>, style: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub color: Option<String>,
    raw: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub from_side: Option<String>,
    pub to_side: Option<String>,
    pub from_end: Option<String>,
    pub to_end: Option<String>,
    pub color: Option<String>,
    pub label: Option<String>,
    raw: Map<String, Value>,
}

fn text(m: &Map<String, Value>, key: &str) -> Option<String> {
    m.get(key).and_then(Value::as_str).map(String::from)
}

fn number(m: &Map<String, Value>, key: &str) -> f64 {
    m.get(key).and_then(Value::as_f64).unwrap_or(0.)
}

impl Node {
    /// A new text card.
    pub fn text(id: String, x: f64, y: f64, content: String) -> Self {
        Self { id, kind: NodeKind::Text(content), x, y, width: 260., height: 120., color: None, raw: Map::new() }
    }

    /// A new card showing a file.
    pub fn file(id: String, x: f64, y: f64, path: String) -> Self {
        Self { id, kind: NodeKind::File { path, subpath: None }, x, y, width: 400., height: 400., color: None, raw: Map::new() }
    }

    fn from_value(v: &Value) -> Option<Self> {
        let m = v.as_object()?;
        let kind = match m.get("type")?.as_str()? {
            "text" => NodeKind::Text(text(m, "text").unwrap_or_default()),
            "file" => NodeKind::File { path: text(m, "file")?, subpath: text(m, "subpath") },
            "link" => NodeKind::Link(text(m, "url")?),
            "group" => NodeKind::Group { label: text(m, "label"), background: text(m, "background"), style: text(m, "backgroundStyle") },
            _ => return None,
        };
        Some(Self { id: text(m, "id")?, kind, x: number(m, "x"), y: number(m, "y"), width: number(m, "width"), height: number(m, "height"), color: text(m, "color"), raw: m.clone() })
    }

    fn to_value(&self) -> Value {
        let mut m = self.raw.clone();
        let int = |n: f64| Value::from(n.round() as i64);
        m.insert("id".into(), self.id.clone().into());
        m.insert("x".into(), int(self.x));
        m.insert("y".into(), int(self.y));
        m.insert("width".into(), int(self.width));
        m.insert("height".into(), int(self.height));
        let mut set = |key: &str, value: Option<&String>| match value {
            Some(v) => {
                m.insert(key.into(), v.clone().into());
            }
            None => {
                m.remove(key);
            }
        };
        set("color", self.color.as_ref());
        match &self.kind {
            NodeKind::Text(t) => {
                set("type", Some(&"text".to_string()));
                set("text", Some(t));
            }
            NodeKind::File { path, subpath } => {
                set("type", Some(&"file".to_string()));
                set("file", Some(path));
                set("subpath", subpath.as_ref());
            }
            NodeKind::Link(url) => {
                set("type", Some(&"link".to_string()));
                set("url", Some(url));
            }
            NodeKind::Group { label, background, style } => {
                set("type", Some(&"group".to_string()));
                set("label", label.as_ref());
                set("background", background.as_ref());
                set("backgroundStyle", style.as_ref());
            }
        }
        Value::Object(m)
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

impl Edge {
    pub fn new(id: String, from: String, to: String) -> Self {
        Self { id, from, to, from_side: None, to_side: None, from_end: None, to_end: None, color: None, label: None, raw: Map::new() }
    }

    fn from_value(v: &Value) -> Option<Self> {
        let m = v.as_object()?;
        Some(Self { id: text(m, "id")?, from: text(m, "fromNode")?, to: text(m, "toNode")?, from_side: text(m, "fromSide"), to_side: text(m, "toSide"), from_end: text(m, "fromEnd"), to_end: text(m, "toEnd"), color: text(m, "color"), label: text(m, "label"), raw: m.clone() })
    }

    fn to_value(&self) -> Value {
        let mut m = self.raw.clone();
        let mut set = |key: &str, value: Option<&String>| match value {
            Some(v) => {
                m.insert(key.into(), v.clone().into());
            }
            None => {
                m.remove(key);
            }
        };
        set("id", Some(&self.id));
        set("fromNode", Some(&self.from));
        set("toNode", Some(&self.to));
        set("fromSide", self.from_side.as_ref());
        set("toSide", self.to_side.as_ref());
        set("fromEnd", self.from_end.as_ref());
        set("toEnd", self.to_end.as_ref());
        set("color", self.color.as_ref());
        set("label", self.label.as_ref());
        Value::Object(m)
    }
}

impl Canvas {
    pub fn parse(source: &str) -> Option<Self> {
        if source.trim().is_empty() {
            return Some(Self::default());
        }
        let Value::Object(mut root) = serde_json::from_str::<Value>(source).ok()? else { return None };
        let nodes = root.remove("nodes").and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let edges = root.remove("edges").and_then(|v| v.as_array().cloned()).unwrap_or_default();
        Some(Self { nodes: nodes.iter().filter_map(Node::from_value).collect(), edges: edges.iter().filter_map(Edge::from_value).collect(), extra: root })
    }

    /// The document as Obsidian writes it: tabs and one line per node.
    pub fn to_json(&self) -> String {
        let line = |v: Value| v.to_string();
        let nodes: Vec<String> = self.nodes.iter().map(|n| format!("\t\t{}", line(n.to_value()))).collect();
        let edges: Vec<String> = self.edges.iter().map(|e| format!("\t\t{}", line(e.to_value()))).collect();
        let mut out = String::from("{\n");
        out.push_str(&format!("\t\"nodes\":[\n{}\n\t],\n", nodes.join(",\n")));
        out.push_str(&format!("\t\"edges\":[\n{}\n\t]", edges.join(",\n")));
        for (k, v) in &self.extra {
            out.push_str(&format!(",\n\t{}:{}", Value::from(k.clone()), v));
        }
        out.push_str("\n}");
        out
    }

    /// A node id no other node has, in Obsidian's style.
    pub fn new_id(&self) -> String {
        loop {
            let id: String = uuid::Uuid::new_v4().simple().to_string().chars().take(16).collect();
            if !self.nodes.iter().any(|n| n.id == id) && !self.edges.iter().any(|e| e.id == id) {
                return id;
            }
        }
    }

    pub fn remove_node(&mut self, id: &str) {
        self.nodes.retain(|n| n.id != id);
        self.edges.retain(|e| e.from != id && e.to != id);
    }

    /// The side of a node facing another point.
    pub fn facing_side(node: &Node, x: f64, y: f64) -> &'static str {
        let (cx, cy) = (node.x + node.width / 2., node.y + node.height / 2.);
        let (dx, dy) = ((x - cx) / node.width.max(1.), (y - cy) / node.height.max(1.));
        if dx.abs() > dy.abs() {
            if dx > 0. { "right" } else { "left" }
        } else if dy > 0. {
            "bottom"
        } else {
            "top"
        }
    }

    pub fn side_point(node: &Node, side: &str) -> (f64, f64) {
        match side {
            "top" => (node.x + node.width / 2., node.y),
            "bottom" => (node.x + node.width / 2., node.y + node.height),
            "left" => (node.x, node.y + node.height / 2.),
            _ => (node.x + node.width, node.y + node.height / 2.),
        }
    }

    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let first = self.nodes.first()?;
        Some(self.nodes.iter().fold((first.x, first.y, first.x + first.width, first.y + first.height), |(a, b, c, d), n| (a.min(n.x), b.min(n.y), c.max(n.x + n.width), d.max(n.y + n.height))))
    }

    /// Nodes inside a group, by position.
    pub fn children_of(&self, group: &Node) -> Vec<String> {
        self.nodes.iter().filter(|n| n.id != group.id && n.x >= group.x && n.y >= group.y && n.x + n.width <= group.x + group.width && n.y + n.height <= group.y + group.height).map(|n| n.id.clone()).collect()
    }
}

/// Links a canvas holds, so renaming a file reaches its cards too.
pub fn facts_of(source: &str, modified: f64) -> NoteFacts {
    let Some(canvas) = Canvas::parse(source) else { return NoteFacts { modified, ..Default::default() } };
    let mut facts = NoteFacts { modified, ..Default::default() };
    for node in &canvas.nodes {
        match &node.kind {
            NodeKind::File { path, subpath } => {
                let target = LinkTarget { path: path.clone(), ..LinkTarget::parse(subpath.as_deref().unwrap_or_default()) };
                facts.links.push((FoundLink { range: 0..0, target, alias: None, embed: true, markdown: false }, 0, path.clone()));
            }
            NodeKind::Text(text) => {
                for link in super::links::find_links(text) {
                    let context = link.target.path.clone();
                    facts.links.push((FoundLink { range: 0..0, ..link }, 0, context));
                }
            }
            _ => {}
        }
    }
    facts
}

/// Points file cards at a moved file; `None` when no card changes.
pub fn rewrite_file(source: &str, old: &str, new: &str) -> Option<String> {
    let mut canvas = Canvas::parse(source)?;
    let mut changed = false;
    for node in &mut canvas.nodes {
        if let NodeKind::File { path, .. } = &mut node.kind
            && path == old
        {
            *path = new.to_string();
            changed = true;
        }
    }
    changed.then(|| canvas.to_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r##"{"nodes":[{"id":"a","type":"text","text":"Hallo [[Z]]","x":-10,"y":20,"width":100,"height":50,"color":"2","eigenes":{"x":1}},{"id":"b","type":"file","file":"o/n.md","subpath":"#H","x":200,"y":0,"width":300,"height":200},{"id":"c","type":"link","url":"https://x.y","x":0,"y":300,"width":100,"height":100},{"id":"g","type":"group","label":"G","background":"b.png","backgroundStyle":"cover","x":-20,"y":-20,"width":600,"height":300,"color":"#ff0000"}],"edges":[{"id":"e","fromNode":"a","fromSide":"right","toNode":"b","toSide":"left","toEnd":"none","color":"1","label":"L","mehr":true}],"meta":{"k":1}}"##;

    #[test]
    fn reads_all_kinds_and_keeps_unknown_fields() {
        let c = Canvas::parse(DOC).unwrap();
        assert_eq!(c.nodes.len(), 4);
        assert!(matches!(&c.nodes[1].kind, NodeKind::File { path, subpath } if path == "o/n.md" && subpath.as_deref() == Some("#H")));
        assert_eq!(c.edges[0].to_end.as_deref(), Some("none"));
        let again: Value = serde_json::from_str(&c.to_json()).unwrap();
        let original: Value = serde_json::from_str(DOC).unwrap();
        assert_eq!(again, original);
    }

    #[test]
    fn broken_or_empty_documents_are_handled() {
        assert!(Canvas::parse("kaputt").is_none());
        assert_eq!(Canvas::parse("").unwrap().nodes.len(), 0);
        assert_eq!(Canvas::parse("{}").unwrap().edges.len(), 0);
        assert!(Canvas::parse("{}").unwrap().to_json().contains("\"nodes\":["));
    }

    #[test]
    fn removing_a_node_removes_its_edges() {
        let mut c = Canvas::parse(DOC).unwrap();
        c.remove_node("b");
        assert!(c.edges.is_empty());
        assert_eq!(c.nodes.len(), 3);
    }

    #[test]
    fn geometry() {
        let c = Canvas::parse(DOC).unwrap();
        let group = c.nodes.iter().find(|n| n.id == "g").unwrap();
        assert_eq!(c.children_of(group), ["a", "b"]);
        assert_eq!(c.bounds(), Some((-20., -20., 580., 400.)));
        assert_eq!(Canvas::facing_side(&c.nodes[0], 500., 40.), "right");
        assert_eq!(Canvas::facing_side(&c.nodes[0], 40., -500.), "top");
    }

    #[test]
    fn links_and_moves_reach_the_cards() {
        let facts = facts_of(DOC, 0.);
        let targets: Vec<&str> = facts.links.iter().map(|(l, _, _)| l.target.path.as_str()).collect();
        assert_eq!(targets, ["Z", "o/n.md"]);
        let moved = rewrite_file(DOC, "o/n.md", "x/neu.md").unwrap();
        assert!(moved.contains("x/neu.md"));
        assert_eq!(rewrite_file(DOC, "nein", "x"), None);
    }
}
