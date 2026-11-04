use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::XmlVersion;

#[derive(Debug, Default, Clone)]
pub struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone)]
pub enum Node {
    El(El),
    Text(String),
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

impl El {
    /// An attribute by qualified name (`r:id`) or, without a prefix, by local name.
    pub fn attr(&self, name: &str) -> Option<&str> {
        if name.contains(':') {
            return self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        }
        self.attrs.iter().find(|(k, _)| local(k) == name).map(|(_, v)| v.as_str())
    }

    pub fn els(&self) -> impl Iterator<Item = &El> {
        self.children.iter().filter_map(|n| if let Node::El(e) = n { Some(e) } else { None })
    }

    pub fn child(&self, name: &str) -> Option<&El> {
        self.els().find(|e| e.name == name)
    }

    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.els().filter(move |e| e.name == name)
    }

    pub fn path(&self, names: &[&str]) -> Option<&El> {
        names.iter().try_fold(self, |e, n| e.child(n))
    }

    /// The `val` attribute of a child, the usual shape of a property.
    pub fn val(&self, name: &str) -> Option<&str> {
        self.child(name).and_then(|e| e.attr("val"))
    }

    /// Whether an on/off property is on: present without `val`, or with a true value.
    pub fn flag(&self, name: &str) -> Option<bool> {
        self.child(name).map(|e| !matches!(e.attr("val"), Some("0" | "false" | "off" | "none")))
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        for n in &self.children {
            match n {
                Node::Text(t) => out.push_str(t),
                Node::El(e) => e.collect_text(out),
            }
        }
    }

    /// Every element below, depth first.
    pub fn descendants(&self) -> Vec<&El> {
        let mut out = Vec::new();
        let mut stack: Vec<&El> = self.els().collect::<Vec<_>>().into_iter().rev().collect();
        while let Some(e) = stack.pop() {
            out.push(e);
            stack.extend(e.els().collect::<Vec<_>>().into_iter().rev());
        }
        out
    }

    pub fn find(&self, name: &str) -> Option<&El> {
        self.descendants().into_iter().find(|e| e.name == name)
    }
}

fn start(e: &BytesStart) -> El {
    let name = local(e.name().0).to_string();
    let attrs = e.attributes().flatten().map(|a| (a.key.0.to_string(), a.normalized_value(XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_default())).collect();
    El { name, attrs, children: Vec::new() }
}

fn entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => return None,
    })
}

/// The character an entity or character reference stands for.
pub fn reference(r: &quick_xml::events::BytesRef) -> Option<char> {
    r.resolve_char_ref().ok().flatten().or_else(|| entity(r))
}

fn push_text(stack: &mut [El], text: &str) {
    let Some(top) = stack.last_mut() else { return };
    if let Some(Node::Text(t)) = top.children.last_mut() {
        t.push_str(text);
    } else {
        top.children.push(Node::Text(text.to_string()));
    }
}

/// Markup Compatibility: keeps what the first choice offers, which is the newer drawing form readers understand.
fn resolve_alternates(el: &mut El) {
    let mut out = Vec::with_capacity(el.children.len());
    for node in std::mem::take(&mut el.children) {
        match node {
            Node::El(mut e) if e.name == "AlternateContent" => {
                let pick = e.children.iter().position(|n| matches!(n, Node::El(c) if c.name == "Choice")).or_else(|| e.children.iter().position(|n| matches!(n, Node::El(c) if c.name == "Fallback")));
                if let Some(i) = pick
                    && let Node::El(mut chosen) = e.children.swap_remove(i)
                {
                    resolve_alternates(&mut chosen);
                    out.extend(chosen.children);
                }
                e.children.clear();
            }
            Node::El(mut e) => {
                resolve_alternates(&mut e);
                out.push(Node::El(e));
            }
            text => out.push(text),
        }
    }
    el.children = out;
}

const MAX_DEPTH: usize = 256;

/// Parses a whole part into a tree; text inside elements is kept as written, spaces included.

pub fn parse(bytes: &[u8]) -> Option<El> {
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes));
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<El> = vec![El::default()];
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => {
                if stack.len() > MAX_DEPTH {
                    return None;
                }
                stack.push(start(&e));
            }
            Event::Empty(e) => {
                let el = start(&e);
                stack.last_mut()?.children.push(Node::El(el));
            }
            Event::End(_) => {
                let el = stack.pop()?;
                stack.last_mut()?.children.push(Node::El(el));
            }
            Event::Text(t) => push_text(&mut stack, &t.xml10_content()),
            Event::CData(t) => push_text(&mut stack, &t.xml10_content()),
            Event::GeneralRef(r) => {
                if let Some(c) = reference(&r) {
                    push_text(&mut stack, c.encode_utf8(&mut [0; 4]));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut root = stack.into_iter().next()?;
    resolve_alternates(&mut root);
    root.children.into_iter().find_map(|n| if let Node::El(e) = n { Some(e) } else { None })
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn keeps_spaces_entities_and_the_first_choice() {
        let root = parse(br#"<?xml version="1.0"?><w:p xmlns:w="x"><w:t xml:space="preserve"> a &amp; b&#33; </w:t><mc:AlternateContent><mc:Choice><w:new/></mc:Choice><mc:Fallback><w:old/></mc:Fallback></mc:AlternateContent></w:p>"#).unwrap();
        assert_eq!(root.name, "p");
        assert_eq!(root.child("t").unwrap().text(), " a & b! ");
        assert!(root.child("new").is_some() && root.child("old").is_none());
        assert_eq!(root.child("t").unwrap().attr("space"), Some("preserve"));
    }
}
