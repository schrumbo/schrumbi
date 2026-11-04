use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use super::OpenError;
use super::xml::{self, El};

const PART_LIMIT: u64 = 256 * 1024 * 1024;

/// An Office Open XML package: the zip with its parts and relationships.
pub struct Package {
    zip: zip::ZipArchive<Cursor<Vec<u8>>>,
}

impl Package {
    pub fn open(path: &Path) -> Result<Self, OpenError> {
        Self::from_bytes(std::fs::read(path).map_err(|_| OpenError::Broken)?)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, OpenError> {
        if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
            return Err(OpenError::Locked);
        }
        zip::ZipArchive::new(Cursor::new(bytes)).map(|zip| Self { zip }).map_err(|_| OpenError::Broken)
    }

    pub fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        let name = name.trim_start_matches('/');
        let name = if self.zip.index_for_name(name).is_some() { name.to_string() } else { self.zip.file_names().filter_map(Result::ok).find(|n| n.eq_ignore_ascii_case(name))?.to_string() };
        let mut file = self.zip.by_name(&name).ok()?;
        if file.size() > PART_LIMIT {
            return None;
        }
        let mut out = Vec::with_capacity(file.size() as usize);
        file.by_ref().take(PART_LIMIT).read_to_end(&mut out).ok()?;
        Some(out)
    }

    pub fn xml(&mut self, name: &str) -> Option<El> {
        xml::parse(&self.read(name)?)
    }

    /// The relationships of a part, id to resolved part name; external targets keep their URL.
    pub fn rels(&mut self, part: &str) -> HashMap<String, Rel> {
        let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
        let name = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
        let Some(root) = self.xml(&name) else { return HashMap::new() };
        root.all("Relationship")
            .filter_map(|r| {
                let id = r.attr("Id")?.to_string();
                let target = r.attr("Target")?;
                let kind = r.attr("Type").unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
                let external = r.attr("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
                let target = if external { target.to_string() } else { resolve(dir, target) };
                Some((id, Rel { target, kind, external }))
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct Rel {
    pub target: String,
    pub kind: String,
    pub external: bool,
}

/// Resolves a relative target against the folder of the part that names it.
pub fn resolve(dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The image type typst can draw, judged by the bytes.
pub fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
        head.contains("<svg").then_some("svg")
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn resolves_targets() {
        assert_eq!(resolve("word", "media/image1.png"), "word/media/image1.png");
        assert_eq!(resolve("ppt/slides", "../media/a.png"), "ppt/media/a.png");
        assert_eq!(resolve("ppt/slides", "/ppt/x.xml"), "ppt/x.xml");
    }
}
