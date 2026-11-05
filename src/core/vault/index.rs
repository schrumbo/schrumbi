use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use rayon::prelude::*;
use walkdir::WalkDir;

use super::config::{LinkFormat, VaultConfig};
use super::frontmatter;
use super::links::{self, FoundLink, LinkTarget};
use super::resolve::Resolver;
use super::syntax;
use crate::core::markdown;

/// What one note says about itself and others.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NoteFacts {
    pub modified: f64,
    pub links: Vec<(FoundLink, usize, String)>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub headings: Vec<(u8, String, usize)>,
    pub blocks: Vec<(String, usize)>,
    pub properties: Vec<frontmatter::Property>,
    pub words: usize,
    /// Open tasks with their line, outside code and comments.
    pub tasks: Vec<(usize, String)>,
    /// Footnote definitions: label, text and line.
    pub footnotes: Vec<(String, String, usize)>,
    /// The flashcards written in the note.
    pub cards: Vec<crate::core::cards::RawCard>,
    /// The file's length when it was read; with `modified` it tells whether the file changed since.
    pub size: u64,
}

/// A place where a note is linked or mentioned.
#[derive(Debug, Clone, PartialEq)]
pub struct Mention {
    pub source: PathBuf,
    pub line: usize,
    pub text: String,
}

/// Everything known about the links of a vault; replaced as a whole after each scan.
#[derive(Debug, Clone, Default)]
pub struct VaultIndex {
    pub root: PathBuf,
    pub config: VaultConfig,
    pub resolver: Resolver,
    pub notes: HashMap<PathBuf, Arc<NoteFacts>>,
    pub files: Vec<PathBuf>,
    /// Every folder, also the empty ones; hidden and excluded folders stay out.
    pub folders: Vec<PathBuf>,
    pub backlinks: HashMap<PathBuf, Vec<Mention>>,
    /// Link targets that do not exist yet, with where they are used.
    pub missing: BTreeMap<String, Vec<Mention>>,
    pub tags: BTreeMap<String, Vec<PathBuf>>,
    /// Folder of `Mod/Datei` style note ids, for the file tree.
    pub generation: u64,
    /// Whether Obsidian can open files of this vault on this device.
    pub reach: Option<super::config::Reach>,
    pub bookmarks: Vec<super::config::Bookmark>,
    /// Property types Obsidian remembers, by name.
    pub types: HashMap<String, frontmatter::PropertyKind>,
}

static CURRENT: LazyLock<RwLock<Arc<VaultIndex>>> = LazyLock::new(Default::default);
static CACHE: LazyLock<Mutex<HashMap<PathBuf, Arc<NoteFacts>>>> = LazyLock::new(Default::default);

/// The index of the open vault; empty before the first scan.
pub fn current() -> Arc<VaultIndex> {
    CURRENT.read().unwrap().clone()
}

/// Forgets everything, e.g. when the workspace closes.
pub fn reset() {
    let mut current = CURRENT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    *current = Arc::new(VaultIndex { generation: current.generation + 1, ..Default::default() });
    drop(current);
    CACHE.lock().unwrap().clear();
    crate::core::cards::close();
}

/// Reads what a note's text says.
pub fn facts_of(text: &str, modified: f64) -> NoteFacts {
    let lines: Vec<&str> = text.split('\n').collect();
    let line_of = |offset: usize| text[..offset.min(text.len())].bytes().filter(|b| *b == b'\n').count();
    let links = links::find_links(text)
        .into_iter()
        .map(|l| {
            let line = line_of(l.range.start);
            let context = lines.get(line).map_or(String::new(), |t| t.trim().chars().take(200).collect());
            (l, line, context)
        })
        .collect();
    let mut tags = syntax::tags_in(text);
    tags.extend(frontmatter::tags(text));
    tags.sort();
    tags.dedup();
    static FOOTNOTE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^\[\^([^\]]+)\]:\s*(.*)$").unwrap());
    let footnotes = lines.iter().enumerate().filter_map(|(i, l)| FOOTNOTE.captures(l).map(|c| (c[1].to_string(), c[2].to_string(), i))).collect();
    NoteFacts { cards: crate::core::cards::parse::cards_in(text), tasks: open_tasks(text), footnotes, modified, links, tags, aliases: frontmatter::aliases(text), headings: links::headings(text), blocks: links::block_ids(text), properties: frontmatter::properties(text).unwrap_or_default(), words: text.split_whitespace().count(), size: 0 }
}

/// The tasks of a note still to be done. Any other mark counts as done, as in Obsidian.
pub fn open_tasks(text: &str) -> Vec<(usize, String)> {
    let visible = syntax::prepare(text);
    let code = links::code_ranges(&visible);
    let mut offset = 0;
    let mut tasks = Vec::new();
    for (i, line) in visible.split('\n').enumerate() {
        if !code.iter().any(|r| r.contains(&offset))
            && let Some((at, ' ')) = syntax::task_marker(line)
        {
            let rest = line[at + 1..].trim_start_matches(']').trim();
            if !rest.is_empty() {
                tasks.push((i, rest.to_string()));
            }
        }
        offset += line.len() + 1;
    }
    tasks
}

fn unchanged(facts: &NoteFacts, modified: f64, size: u64) -> bool {
    (facts.modified - modified).abs() < 0.001 && facts.size == size
}

fn stamp_of(path: &Path) -> (f64, u64) {
    std::fs::metadata(path).ok().map_or((0., 0), |m| stamp_from(&m))
}

fn stamp_from(m: &std::fs::Metadata) -> (f64, u64) {
    (m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0., |d| d.as_secs_f64()), m.len())
}

fn list_files(root: &Path, exclude: &[String]) -> Vec<PathBuf> {
    list_stamped(root, exclude).into_iter().map(|(p, _, _)| p).collect()
}

/// Every file with its modification time and length, taken from the directory listing itself.
fn list_stamped(root: &Path, exclude: &[String]) -> Vec<(PathBuf, f64, u64)> {
    WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') && !(e.file_type().is_dir() && exclude.iter().any(|x| *x == name))
        })
        .filter_map(Result::ok)
        .filter(|e| !e.file_type().is_dir())
        .map(|e| {
            let stamp = if e.path_is_symlink() { None } else { e.metadata().ok().map(|m| stamp_from(&m)) };
            let (modified, size) = stamp.unwrap_or_else(|| stamp_of(e.path()));
            (e.into_path(), modified, size)
        })
        .collect()
}

fn list_folders(root: &Path, exclude: &[String]) -> Vec<PathBuf> {
    WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            e.file_type().is_dir() && !name.starts_with('.') && !exclude.iter().any(|x| *x == name)
        })
        .filter_map(Result::ok)
        .map(|e| e.into_path())
        .collect()
}

fn is_canvas(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("canvas"))
}

/// Scans the vault off the main thread; only changed notes are read again.
pub fn rebuild(root: &Path, exclude: &[String]) {
    let scan = SCANS.fetch_add(1, Ordering::SeqCst) + 1;
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let stamped = list_stamped(&root, exclude);
    let known = std::mem::take(&mut *CACHE.lock().unwrap());
    let mut notes: HashMap<PathBuf, Arc<NoteFacts>> = HashMap::with_capacity(known.len());
    let mut stale: Vec<(&PathBuf, f64)> = Vec::new();
    for (p, modified, size) in stamped.iter().filter(|(p, _, _)| markdown::is_markdown(p) || is_canvas(p)) {
        match known.get(p).filter(|o| unchanged(o, *modified, *size)) {
            Some(old) => {
                notes.insert(p.clone(), old.clone());
            }
            None => stale.push((p, *modified)),
        }
    }
    let read: Vec<(PathBuf, Arc<NoteFacts>)> = stale
        .par_iter()
        .filter_map(|(p, modified)| {
            let bytes = std::fs::read(p).ok()?;
            let text = String::from_utf8_lossy(&bytes);
            let mut facts = if is_canvas(p) { super::canvas::facts_of(&text, *modified) } else { facts_of(&text, *modified) };
            facts.size = bytes.len() as u64;
            Some(((*p).clone(), Arc::new(facts)))
        })
        .collect();
    notes.extend(read);
    let files: Vec<PathBuf> = stamped.into_iter().map(|(p, _, _)| p).collect();
    let folders = list_folders(&root, exclude);
    let mut index = assemble(root, files, notes);
    index.folders = folders;
    *CACHE.lock().unwrap() = index.notes.clone();
    publish(index);
    if hidden().paths.values().any(|ended| ended.is_some_and(|e| e < scan)) {
        change_hidden(|hidden| hidden.retain(|_, ended| ended.is_none_or(|e| e >= scan)));
    }
}

fn publish(mut index: VaultIndex) {
    let mut current = CURRENT.write().unwrap();
    index.generation = current.generation + 1;
    *current = Arc::new(index);
}

fn assemble(root: PathBuf, files: Vec<PathBuf>, notes: HashMap<PathBuf, Arc<NoteFacts>>) -> VaultIndex {
    let aliases: Vec<(String, PathBuf)> = notes.iter().flat_map(|(p, f)| f.aliases.iter().map(move |a| (a.clone(), p.clone()))).collect();
    let resolver = Resolver::new(&root, files.iter().cloned(), aliases);
    let mut backlinks: HashMap<PathBuf, Vec<Mention>> = HashMap::new();
    let mut missing: BTreeMap<String, Vec<Mention>> = BTreeMap::new();
    let mut tags: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for (source, facts) in &notes {
        for (link, line, text) in &facts.links {
            let mention = Mention { source: source.clone(), line: *line, text: text.clone() };
            match resolver.resolve(&link.target, Some(source)) {
                Some(target) if target != *source => backlinks.entry(target).or_default().push(mention),
                Some(_) => {}
                None => missing.entry(link.target.path.clone()).or_default().push(mention),
            }
        }
        for tag in &facts.tags {
            tags.entry(tag.to_lowercase()).or_default().push(source.clone());
        }
    }
    for list in backlinks.values_mut().chain(missing.values_mut()) {
        list.sort_by(|a, b| a.source.cmp(&b.source).then(a.line.cmp(&b.line)));
    }
    for list in tags.values_mut() {
        list.sort();
    }
    VaultIndex { reach: Some(super::config::reach(&root, &super::config::known_vaults(), super::config::obsidian_installed())), bookmarks: super::config::bookmarks(&root), types: super::config::property_types(&root), config: VaultConfig::read(&root), root, resolver, notes, files, folders: Vec::new(), backlinks, missing, tags, generation: 0 }
}

impl VaultIndex {
    /// The file a link means when written in `from`.
    pub fn resolve(&self, destination: &str, from: Option<&Path>) -> Option<PathBuf> {
        self.resolver.resolve(&LinkTarget::parse(destination), from)
    }

    /// How a new link from `source` to `target` is written, by the vault's settings. Notes lose their extension.
    pub fn link_path(&self, source: Option<&Path>, target: &Path) -> String {
        let Ok(rel) = target.strip_prefix(&self.root) else { return target.to_string_lossy().to_string() };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let bare = if markdown::is_markdown(target) { rel.rsplit_once('.').map_or(rel.clone(), |(s, _)| s.to_string()) } else { rel.clone() };
        match self.config.new_link_format {
            LinkFormat::Shortest => self.resolver.shortest(target),
            LinkFormat::Absolute => bare,
            LinkFormat::Relative => {
                let folder = source.and_then(|s| s.parent()).and_then(|f| f.strip_prefix(&self.root).ok()).map(|f| f.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                relative_path(&folder, &bare)
            }
        }
    }

    /// A complete new link to `target`, in the vault's format.
    pub fn new_link(&self, source: Option<&Path>, target: &Path, subpath: &str) -> String {
        let path = self.link_path(source, target);
        if self.config.use_markdown_links {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            let file = if markdown::is_markdown(target) && !path.ends_with(".md") { format!("{path}.md") } else { path };
            format!("[{name}]({}{})", file.replace(' ', "%20"), subpath.replace(' ', "%20"))
        } else {
            format!("[[{path}{subpath}]]")
        }
    }

    /// Notes mentioning a note's name without linking to it.
    pub fn unlinked_mentions(&self, target: &Path, limit: usize) -> Vec<Mention> {
        let name = target.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        if name.chars().count() < 3 {
            return Vec::new();
        }
        let linked: Vec<&PathBuf> = self.backlinks.get(target).map(|l| l.iter().map(|m| &m.source).collect()).unwrap_or_default();
        let mut found = Vec::new();
        let mut sources: Vec<&PathBuf> = self.notes.keys().filter(|p| *p != target && !linked.contains(p) && markdown::is_markdown(p)).collect();
        sources.sort();
        for source in sources {
            let Ok(text) = std::fs::read_to_string(source) else { continue };
            let code = links::code_ranges(&text);
            let mut offset = 0;
            for (line_no, line) in text.split('\n').enumerate() {
                if !code.iter().any(|r| r.contains(&offset)) && line.to_lowercase().contains(&name) {
                    found.push(Mention { source: source.clone(), line: line_no, text: line.trim().chars().take(200).collect() });
                    break;
                }
                offset += line.len() + 1;
            }
            if found.len() >= limit {
                break;
            }
        }
        found
    }

    /// Every tag with its notes; `a/b` also counts for `a`.
    /// Whether a note sits in the vault's template folder and is no real note.
    pub fn is_template(&self, path: &Path) -> bool {
        let folder = self.config.templates.folder.trim_matches('/');
        !folder.is_empty() && path.starts_with(self.root.join(folder))
    }

    pub fn tag_tree(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, std::collections::BTreeSet<&PathBuf>> = BTreeMap::new();
        for (tag, notes) in &self.tags {
            let mut prefix = String::new();
            for part in tag.split('/') {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                counts.entry(prefix.clone()).or_default().extend(notes.iter());
            }
        }
        counts.into_iter().map(|(t, n)| (t, n.len())).collect()
    }

    /// Notes carrying a tag or one nested below it.
    pub fn notes_with_tag(&self, tag: &str) -> Vec<PathBuf> {
        let tag = tag.to_lowercase();
        let nested = format!("{tag}/");
        let mut out: Vec<PathBuf> = self.tags.iter().filter(|(t, _)| **t == tag || t.starts_with(&nested)).flat_map(|(_, n)| n.iter().cloned()).collect();
        out.sort();
        out.dedup();
        out
    }
}

/// Builds an index directly, for tests of what sits on top of it.
#[cfg(test)]
pub fn rebuild_for_test(root: &Path, facts: impl Fn(&str) -> NoteFacts) -> VaultIndex {
    let files = list_files(root, &[]);
    let notes = files.iter().filter(|p| markdown::is_markdown(p)).map(|p| (p.clone(), Arc::new(facts(&std::fs::read_to_string(p).unwrap_or_default())))).collect();
    assemble(root.to_path_buf(), files, notes)
}

/// `folder` to `target`, both relative to the same root.
pub fn relative_path(folder: &str, target: &str) -> String {
    let from: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
    let to: Vec<&str> = target.split('/').collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(&to[common..]);
    parts.join("/")
}

/// New text of `text` once the file `old` lives at `new`; `None` when no link changes.
/// A link without display text keeps showing the old name.
pub fn rewrite_links(text: &str, source: &Path, old: &Path, new: &Path, before: &Resolver, after: &VaultIndex) -> Option<(String, usize)> {
    let mut changes = Vec::new();
    for link in links::find_links(text) {
        if link.target.path.is_empty() || before.resolve(&link.target, Some(source)).as_deref() != Some(old) {
            continue;
        }
        let changed = link.clone();
        let path = after.link_path(Some(source), new);
        let path = if link.markdown && markdown::is_markdown(new) && !path.ends_with(".md") { format!("{path}.md") } else { path };
        changes.push((link.range.clone(), links::render_link(&changed, &path)));
    }
    let count = changes.len();
    (count > 0).then(|| (links::apply_replacements(text, changes), count))
}

/// The index as it is after a file moved, so several moves in a row see each other.
pub fn moved(old: &Path, new: &Path) -> Arc<VaultIndex> {
    let next = moved_in(&current(), old, new, new.is_dir());
    publish(next);
    current()
}

/// `index` after `old` moved to `new`; a folder takes every file and folder below it along.
fn moved_in(index: &VaultIndex, old: &Path, new: &Path, folder: bool) -> VaultIndex {
    let follow = |p: &Path| p.strip_prefix(old).ok().map(|rest| if rest.as_os_str().is_empty() { new.to_path_buf() } else { new.join(rest) });
    let mut files = index.files.clone();
    let mut notes = index.notes.clone();
    let mut folders = index.folders.clone();
    if folder {
        files.iter_mut().for_each(|f| *f = follow(f).unwrap_or_else(|| f.clone()));
        folders.iter_mut().for_each(|f| *f = follow(f).unwrap_or_else(|| f.clone()));
        let below: Vec<PathBuf> = notes.keys().filter(|p| follow(p).is_some()).cloned().collect();
        for was in below {
            if let (Some(facts), Some(now)) = (notes.remove(&was), follow(&was)) {
                notes.insert(now, facts);
            }
        }
    } else {
        files.retain(|f| f != old);
        files.push(new.to_path_buf());
        if let Some(facts) = notes.remove(old) {
            notes.insert(new.to_path_buf(), facts);
        }
    }
    let mut next = assemble(index.root.clone(), files, notes);
    next.folders = folders;
    next
}

/// Files on their way to the trash, kept out of the tree until a scan that started after they were gone.
#[derive(Debug, Clone, Default)]
pub struct Hidden {
    pub generation: u64,
    /// Path, and the scan count once the trash ended.
    paths: HashMap<PathBuf, Option<u64>>,
    /// A folder is among them, so files below it need their parents checked too.
    folders: bool,
}

impl Hidden {
    /// The path or a folder above it is hidden.
    pub fn hides(&self, path: &Path) -> bool {
        if self.paths.is_empty() {
            false
        } else if self.folders {
            path.ancestors().any(|a| self.paths.contains_key(a))
        } else {
            self.paths.contains_key(path)
        }
    }
}

static HIDDEN: LazyLock<RwLock<Arc<Hidden>>> = LazyLock::new(Default::default);
static SCANS: AtomicU64 = AtomicU64::new(0);

pub fn hidden() -> Arc<Hidden> {
    HIDDEN.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

fn change_hidden(f: impl FnOnce(&mut HashMap<PathBuf, Option<u64>>)) {
    let mut guard = HIDDEN.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut next = (**guard).clone();
    let before: Vec<PathBuf> = next.paths.keys().cloned().collect();
    f(&mut next.paths);
    if next.paths.len() != before.len() || before.iter().any(|p| !next.paths.contains_key(p)) {
        next.generation += 1;
        next.folders = next.folders && !next.paths.is_empty();
    }
    *guard = Arc::new(next);
}

/// Hides files at once; the index itself stays untouched until the next scan.
pub fn hide(paths: &[PathBuf]) {
    let folders = paths.iter().any(|p| p.is_dir());
    change_hidden(|hidden| hidden.extend(paths.iter().map(|p| (p.clone(), None))));
    if folders {
        let mut guard = HIDDEN.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = (**guard).clone();
        next.folders = true;
        next.generation += 1;
        *guard = Arc::new(next);
    }
}

/// The trash ended: files that are gone stay hidden until a later scan, the rest show again.
pub fn trash_ended(gone: &[PathBuf], kept: &[PathBuf]) {
    let scans = SCANS.load(Ordering::SeqCst);
    change_hidden(|hidden| {
        for p in kept {
            hidden.remove(p);
        }
        for p in gone {
            hidden.insert(p.clone(), Some(scans));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_open_tasks_only() {
        let text = "---\na: 1\n---\n- [ ] Eins\n- [x] Fertig\n- [/] Halb\n  * [ ] Zwei\n> - [ ] Im Zitat\n```\n- [ ] Code\n```\n%%\n- [ ] Kommentar\n%%\n- [ ] \n1. [ ] Drei\n";
        let tasks = open_tasks(text);
        let found: Vec<(usize, &str)> = tasks.iter().map(|(l, t)| (*l, t.as_str())).collect();
        assert_eq!(found, [(3, "Eins"), (6, "Zwei"), (7, "Im Zitat"), (15, "Drei")]);
    }

    fn vault(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("schrumbi-index-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dunce::canonicalize(&dir).unwrap()
    }

    fn built(root: PathBuf, files: Vec<PathBuf>) -> VaultIndex {
        let notes = files.iter().filter(|p| markdown::is_markdown(p)).map(|p| (p.clone(), Arc::new(facts_of(&std::fs::read_to_string(p).unwrap_or_default(), 0.)))).collect();
        assemble(root, files, notes)
    }

    #[test]
    fn a_moved_folder_takes_its_files_notes_and_folders_along() {
        let root = vault("moved-folder", &[("kurs/A.md", "x\n"), ("kurs/tief/B.md", "y\n"), ("C.md", "z\n")]);
        let mut files = vec![root.join("kurs/A.md"), root.join("kurs/tief/B.md"), root.join("C.md")];
        files.sort();
        let mut before = built(root.clone(), files);
        before.folders = vec![root.join("kurs"), root.join("kurs/tief")];
        let (old, new) = (root.join("kurs"), root.join("neu"));
        let after = moved_in(&before, &old, &new, true);
        assert!(after.files.contains(&new.join("A.md")) && after.files.contains(&new.join("tief/B.md")) && after.files.contains(&root.join("C.md")));
        assert!(!after.files.iter().any(|f| f.starts_with(&old)), "nothing stays below the old name");
        assert!(!after.files.contains(&new), "the folder itself is no file");
        assert!(after.notes.contains_key(&new.join("A.md")) && !after.notes.contains_key(&old.join("A.md")));
        assert_eq!(after.folders, vec![new.clone(), new.join("tief")]);
        let single = moved_in(&before, &root.join("C.md"), &root.join("D.md"), false);
        assert!(single.files.contains(&root.join("D.md")) && !single.files.contains(&root.join("C.md")));
    }

    #[test]
    fn a_rewrite_within_the_same_clock_tick_is_read_again() {
        let read = NoteFacts { modified: 100., size: 5, ..Default::default() };
        assert!(unchanged(&read, 100., 5));
        assert!(!unchanged(&read, 100., 22), "coarse file clocks keep the time, the length still tells");
        assert!(!unchanged(&read, 101., 5));
    }

    #[test]
    fn collects_backlinks_missing_targets_and_tags() {
        let root = vault("facts", &[("A.md", "[[B]] [[Fehlt]] #eins #eins/zwei\n"), ("sub/B.md", "---\naliases: [Bee]\ntags: [drei]\n---\n![[A]]\n"), ("C.md", "[[Bee]] [[A#H|x]]\n")]);
        let files = list_files(&root, &[]);
        let i = built(root.clone(), files);
        let b = root.join("sub/B.md");
        assert_eq!(i.backlinks[&b].len(), 2);
        assert_eq!(i.backlinks[&root.join("A.md")].len(), 2);
        assert_eq!(i.missing["Fehlt"][0].source, root.join("A.md"));
        assert_eq!(i.tag_tree(), [("drei".to_string(), 1), ("eins".to_string(), 1), ("eins/zwei".to_string(), 1)]);
        assert_eq!(i.notes_with_tag("EINS"), [root.join("A.md")]);
    }

    #[test]
    fn renaming_rewrites_every_kind_of_link() {
        let root = vault("rename", &[("Alt.md", "Ziel\n"), ("Q.md", "[[Alt]] [[Alt#H]] [[Alt|Mein Text]] ![[Alt]] [t](Alt.md#x)\n`[[Alt]]`\n---\n"), ("N/Neu.md", "x")]);
        let files = list_files(&root, &[]);
        let before = built(root.clone(), files.clone());
        let old = root.join("Alt.md");
        let new = root.join("P1_26_Neu_Muster-M.md");
        let mut after_files = files;
        after_files.retain(|f| *f != old);
        after_files.push(new.clone());
        let after = built(root.clone(), after_files);
        let source = root.join("Q.md");
        let (text, count) = rewrite_links(&std::fs::read_to_string(&source).unwrap(), &source, &old, &new, &before.resolver, &after).unwrap();
        assert_eq!(count, 5);
        assert_eq!(text, "[[P1_26_Neu_Muster-M]] [[P1_26_Neu_Muster-M#H]] [[P1_26_Neu_Muster-M|Mein Text]] ![[P1_26_Neu_Muster-M]] [t](P1_26_Neu_Muster-M.md#x)\n`[[Alt]]`\n---\n");
        assert_eq!(rewrite_links("[[Anders]]", &source, &old, &new, &before.resolver, &after), None);
    }

    #[test]
    fn new_links_follow_the_settings() {
        let root = vault("format", &[("a/Ziel Name.md", "x"), ("b/Q.md", "x"), ("c/Ziel Name.md", "x")]);
        let mut i = built(root.clone(), list_files(&root, &[]));
        let target = root.join("a/Ziel Name.md");
        let source = root.join("b/Q.md");
        assert_eq!(i.new_link(Some(&source), &target, ""), "[[a/Ziel Name]]");
        assert_eq!(i.new_link(Some(&source), &root.join("b/Q.md"), "#H"), "[[Q#H]]");
        i.config.new_link_format = LinkFormat::Relative;
        assert_eq!(i.new_link(Some(&source), &target, ""), "[[../a/Ziel Name]]");
        i.config.new_link_format = LinkFormat::Absolute;
        i.config.use_markdown_links = true;
        assert_eq!(i.new_link(Some(&source), &target, ""), "[Ziel Name](a/Ziel%20Name.md)");
    }

    #[test]
    fn unlinked_mentions_skip_real_links_and_code() {
        let root = vault("mentions", &[("Radar.md", "x"), ("a.md", "Das Radar hier\n"), ("b.md", "[[Radar]] Radar\n"), ("c.md", "`Radar`\n```\nRadar\n```\n")]);
        let i = built(root.clone(), list_files(&root, &[]));
        let found = i.unlinked_mentions(&root.join("Radar.md"), 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, root.join("a.md"));
    }

    #[test]
    fn relative_paths() {
        assert_eq!(relative_path("", "a/b"), "a/b");
        assert_eq!(relative_path("a", "a/b"), "b");
        assert_eq!(relative_path("x/y", "a/b"), "../../a/b");
    }
}
