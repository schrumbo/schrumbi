use std::collections::HashMap;
use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use super::links::LinkTarget;
use crate::core::markdown;

/// Case and Unicode form never decide whether a link resolves.
fn key(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Finds the file a link means, by Obsidian's rules.
#[derive(Debug, Default, Clone)]
pub struct Resolver {
    root: PathBuf,
    /// Lowercased names, with and without the note extension, to files relative to the root.
    by_name: HashMap<String, Vec<String>>,
    by_path: HashMap<String, String>,
    aliases: HashMap<String, Vec<String>>,
}

impl Resolver {
    /// `files` are absolute; `aliases` maps an alias to the note declaring it.
    pub fn new(root: &Path, files: impl IntoIterator<Item = PathBuf>, aliases: impl IntoIterator<Item = (String, PathBuf)>) -> Self {
        let mut r = Self { root: root.to_path_buf(), ..Default::default() };
        for file in files {
            r.add(&file);
        }
        for (alias, file) in aliases {
            if let Ok(rel) = file.strip_prefix(root) {
                r.aliases.entry(key(&alias)).or_default().push(slashes(rel));
            }
        }
        for list in r.by_name.values_mut().chain(r.aliases.values_mut()) {
            list.sort_by_key(|p| (p.matches('/').count(), p.len(), p.clone()));
        }
        r
    }

    fn add(&mut self, file: &Path) {
        let Ok(rel) = file.strip_prefix(&self.root) else { return };
        let rel = slashes(rel);
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        self.by_name.entry(key(&name)).or_default().push(rel.clone());
        self.by_path.insert(key(&rel), rel.clone());
        if markdown::is_markdown(file) {
            let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
            self.by_name.entry(key(stem)).or_default().push(rel.clone());
            let bare = rel.rsplit_once('.').map_or(rel.as_str(), |(s, _)| s);
            self.by_path.insert(key(bare), rel.clone());
        }
    }

    /// The file for a link written in the note `from`; `None` when it does not exist yet.
    /// A bare name prefers the same folder, then the shortest path. A path matches from the root,
    /// then relative to the note, then as the end of a longer path.
    pub fn resolve(&self, target: &LinkTarget, from: Option<&Path>) -> Option<PathBuf> {
        if target.path.is_empty() {
            return from.map(Path::to_path_buf);
        }
        let wanted = target.path.replace('\\', "/");
        let wanted = wanted.trim_start_matches("./");
        let folder = from.and_then(|f| f.strip_prefix(&self.root).ok()).and_then(Path::parent).map(slashes).unwrap_or_default();
        let found = if let Some(rooted) = wanted.strip_prefix('/') {
            self.by_path.get(&key(rooted)).cloned()
        } else if wanted.contains('/') {
            self.by_path.get(&key(wanted)).cloned().or_else(|| self.relative(wanted, &folder)).or_else(|| self.by_suffix(wanted))
        } else {
            self.by_name_in(wanted, &folder)
        };
        found.or_else(|| self.aliases.get(&key(wanted)).and_then(|l| l.first().cloned())).map(|rel| self.root.join(rel))
    }

    fn by_name_in(&self, name: &str, folder: &str) -> Option<String> {
        let list = self.by_name.get(&key(name))?;
        let prefix = if folder.is_empty() { String::new() } else { format!("{}/", key(folder)) };
        list.iter().find(|p| key(p).strip_prefix(&prefix).is_some_and(|rest| !rest.contains('/'))).or_else(|| list.first()).cloned()
    }

    fn relative(&self, wanted: &str, folder: &str) -> Option<String> {
        let mut parts: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
        for piece in wanted.split('/') {
            match piece {
                ".." => {
                    parts.pop()?;
                }
                "." | "" => {}
                other => parts.push(other),
            }
        }
        self.by_path.get(&key(&parts.join("/"))).cloned()
    }

    fn by_suffix(&self, wanted: &str) -> Option<String> {
        let end = format!("/{}", key(wanted));
        self.by_path.iter().filter(|(k, _)| k.ends_with(&end)).map(|(_, v)| v.clone()).min_by_key(|p| (p.matches('/').count(), p.len()))
    }

    /// The shortest way to write a link to `file` that still resolves to it from anywhere.
    pub fn shortest(&self, file: &Path) -> String {
        let Ok(rel) = file.strip_prefix(&self.root) else { return slashes(file) };
        let rel = slashes(rel);
        let is_note = markdown::is_markdown(file);
        let written = if is_note { rel.rsplit_once('.').map_or(rel.as_str(), |(s, _)| s).to_string() } else { rel.clone() };
        let name = written.rsplit('/').next().unwrap_or(&written);
        let unique = self.by_name.get(&key(name)).is_some_and(|l| l.iter().filter(|p| **p != rel).count() == 0);
        if unique { name.to_string() } else { written }
    }

    /// Every note name, for suggestions.
    pub fn notes(&self) -> impl Iterator<Item = &String> {
        self.by_path.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver() -> Resolver {
        let root = PathBuf::from("/v");
        let files = ["A.md", "Ordner/A.md", "Ordner/Tief/C.md", "Ordner/Bild.png", "Übung.md", "x/Lang.md", "y/z/Lang.md"].map(|f| root.join(f));
        Resolver::new(&root, files, [("Spitzname".to_string(), root.join("Ordner/Tief/C.md"))])
    }

    fn get(r: &Resolver, link: &str, from: Option<&str>) -> Option<String> {
        let from = from.map(|f| PathBuf::from("/v").join(f));
        r.resolve(&LinkTarget::parse(link), from.as_deref()).map(|p| p.strip_prefix("/v").unwrap().to_string_lossy().to_string())
    }

    #[test]
    fn a_bare_name_prefers_the_same_folder_then_the_shortest_path() {
        let r = resolver();
        assert_eq!(get(&r, "A", Some("Ordner/n.md")).as_deref(), Some("Ordner/A.md"));
        assert_eq!(get(&r, "A", Some("x/n.md")).as_deref(), Some("A.md"));
        assert_eq!(get(&r, "Lang", None).as_deref(), Some("x/Lang.md"));
    }

    #[test]
    fn case_extension_and_unicode_form_do_not_matter() {
        let r = resolver();
        assert_eq!(get(&r, "a.MD", None).as_deref(), Some("A.md"));
        assert_eq!(get(&r, "c", None).as_deref(), Some("Ordner/Tief/C.md"));
        let decomposed: String = "Übung".nfd().collect();
        assert_eq!(get(&r, &decomposed, None).as_deref(), Some("Übung.md"));
    }

    #[test]
    fn paths_match_from_root_relative_and_by_suffix() {
        let r = resolver();
        assert_eq!(get(&r, "Ordner/A", None).as_deref(), Some("Ordner/A.md"));
        assert_eq!(get(&r, "/A", Some("Ordner/n.md")).as_deref(), Some("A.md"));
        assert_eq!(get(&r, "../A", Some("Ordner/Tief/n.md")).as_deref(), Some("Ordner/A.md"));
        assert_eq!(get(&r, "Tief/C", Some("x/n.md")).as_deref(), Some("Ordner/Tief/C.md"));
        assert_eq!(get(&r, "z/Lang", None).as_deref(), Some("y/z/Lang.md"));
    }

    #[test]
    fn attachments_need_their_extension_and_aliases_work() {
        let r = resolver();
        assert_eq!(get(&r, "Bild.png", None).as_deref(), Some("Ordner/Bild.png"));
        assert_eq!(get(&r, "Bild", None), None);
        assert_eq!(get(&r, "spitzname", None).as_deref(), Some("Ordner/Tief/C.md"));
        assert_eq!(get(&r, "Fehlt", None), None);
    }

    #[test]
    fn same_note_links_point_at_the_note_itself() {
        let r = resolver();
        assert_eq!(get(&r, "#Abschnitt", Some("A.md")).as_deref(), Some("A.md"));
    }

    #[test]
    fn the_shortest_link_stays_unique() {
        let r = resolver();
        assert_eq!(r.shortest(Path::new("/v/Ordner/Tief/C.md")), "C");
        assert_eq!(r.shortest(Path::new("/v/Ordner/A.md")), "Ordner/A");
        assert_eq!(r.shortest(Path::new("/v/Ordner/Bild.png")), "Bild.png");
    }
}
