use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::browser::DraggedFile;
use super::drive::Mark;
use super::motion::{self, Soft, chevron};
use super::state::{self, act};
use super::theme::{Theme, control, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::library::Route;
use crate::core::markdown;
use crate::core::vault::index;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Files,
    Search,
    Tags,
    Marks,
    Tasks,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Order {
    Name,
    Changed,
}

/// One line of the explorer. All views share one virtual list.
#[derive(Clone, PartialEq)]
enum Row {
    Folder { rel: String, name: String, depth: usize, open: bool },
    File { path: PathBuf, name: String, depth: usize, folder: Option<String>, ago: Option<f64> },
    Tag { tag: String, label: String, depth: usize, count: usize },
    BackToTags,
    Mark { title: String, destination: Option<String>, query: Option<String> },
    Task { path: PathBuf, note: String, line: usize, text: String },
}

/// Folders and their contents, worked out once per vault state.
#[derive(Default, Clone, PartialEq, Debug)]
struct Tree {
    folders: BTreeMap<String, BTreeSet<String>>,
    files: BTreeMap<String, Vec<PathBuf>>,
}

/// Index and hidden-file generations.
type Generation = (u64, u64);

/// Everything the rows depend on.
#[derive(PartialEq)]
struct Key {
    generation: Generation,
    tree: Option<u64>,
    filter: String,
    view: View,
    order: Order,
    tag: Option<String>,
    open: Vec<String>,
    renaming: Option<PathBuf>,
}

struct Cache {
    key: Key,
    rows: Rc<Vec<Row>>,
    open_tasks: usize,
    loading: bool,
}

/// A worked-out file tree and the vault list it came from.
#[derive(Clone)]
struct Ready {
    generation: Generation,
    source: Arc<index::VaultIndex>,
    tree: Arc<Tree>,
    serial: u64,
}

/// The file tree last worked out, and the one being worked out off the UI thread.
#[derive(Default)]
struct Trees {
    ready: Option<Ready>,
    building: Option<Generation>,
    job: Option<Task<()>>,
    serial: u64,
}

impl Global for Trees {}

/// Most changes of the vault list are a few moved files: those are applied to the last tree at once.
const PATCHED_AT_MOST: usize = 64;

/// The files that left and the files that came between two vault lists, when there are only a few.
fn changed_files(before: &[PathBuf], after: &[PathBuf]) -> Option<(Vec<PathBuf>, Vec<PathBuf>)> {
    let (mut gone, mut came) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < before.len() && j < after.len() {
        if before[i] == after[j] {
            i += 1;
            j += 1;
        } else if before.get(i + 1) == Some(&after[j]) {
            gone.push(before[i].clone());
            i += 1;
        } else if after.get(j + 1) == Some(&before[i]) {
            came.push(after[j].clone());
            j += 1;
        } else {
            gone.push(before[i].clone());
            came.push(after[j].clone());
            i += 1;
            j += 1;
        }
        if gone.len() + came.len() > PATCHED_AT_MOST {
            return None;
        }
    }
    gone.extend(before[i..].iter().cloned());
    came.extend(after[j..].iter().cloned());
    let both: Vec<PathBuf> = gone.iter().filter(|p| came.contains(p)).cloned().collect();
    gone.retain(|p| !both.contains(p));
    came.retain(|p| !both.contains(p));
    (gone.len() + came.len() <= PATCHED_AT_MOST).then_some((gone, came))
}

/// Works out the file tree for the current vault state in the background, so the notes room never waits for it.
/// A few moved files are applied to the last tree right away, so a rename never shows the old name.
pub fn prepare_tree(cx: &mut App) {
    let vault = index::current();
    let hidden = index::hidden();
    let generation = (vault.generation, hidden.generation);
    if cx.try_global::<Trees>().is_some_and(|t| t.building == Some(generation) || t.building.is_none() && t.ready.as_ref().is_some_and(|r| r.generation == generation)) {
        return;
    }
    let trees = cx.default_global::<Trees>();
    if let Some(ready) = trees.ready.as_mut()
        && ready.generation.0 != vault.generation
        && ready.source.root == vault.root
        && let Some((gone, came)) = changed_files(&ready.source.files, &vault.files)
    {
        let tree = Arc::make_mut(&mut ready.tree);
        for path in &gone {
            tree.remove(&vault, path);
        }
        for path in &came {
            if !hidden.hides(path) {
                tree.add(&vault, path);
            }
        }
        ready.generation.0 = vault.generation;
        ready.source = vault.clone();
        trees.serial += 1;
        ready.serial = trees.serial;
    }
    let source = vault.clone();
    let work = cx.background_spawn(async move { build_tree(&vault, &hidden) });
    let job = cx.spawn(async move |cx| {
        let tree = Arc::new(work.await);
        cx.update(|cx| {
            cx.update_global::<Trees, _>(|t, _| {
                if t.building == Some(generation) {
                    t.serial += 1;
                    t.ready = Some(Ready { generation, source, tree, serial: t.serial });
                    t.building = None;
                }
            });
        });
    });
    let trees = cx.default_global::<Trees>();
    trees.building = Some(generation);
    trees.job = Some(job);
}

/// The left side of the notes room: everything in the workspace or vault, with the document beside it.
pub struct NotesPage {
    view: View,
    order: Order,
    open_folders: BTreeSet<String>,
    query: Entity<InputState>,
    selected: Option<Row>,
    /// The folder new notes land in.
    target_folder: Option<String>,
    renaming: Option<(PathBuf, Entity<InputState>, Subscription)>,
    /// A folder just made, shown before the next scan has seen it.
    pending_folder: Option<PathBuf>,
    width: f32,
    /// A width drag is under way; it is saved when the drag ends.
    resizing: bool,
    cache: Option<Cache>,
    focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    revealed: Option<PathBuf>,
    shown_tag: Option<String>,
    search_rows: Rc<Vec<Row>>,
    searched: String,
    searching: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

fn unlisted_rename(path: &Path, folder: &str, listed: Option<&Vec<PathBuf>>, root: &Path) -> bool {
    path.exists() && !listed.is_some_and(|f| f.iter().any(|p| p == path)) && path.parent().and_then(|d| d.strip_prefix(root).ok()).is_some_and(|d| d.to_string_lossy().replace('\\', "/") == folder)
}

fn cancel_on_escape(me: &Entity<NotesPage>) -> impl Fn(&gpui_kit::component::input::Escape, &mut Window, &mut App) + 'static {
    let me = me.clone();
    move |_, window, cx| {
        cx.stop_propagation();
        me.update(cx, |s, cx| s.cancel_rename(window, cx));
    }
}

/// The path inside the vault of a file the tree lists; the workspace's settings files do not belong in it.
fn listed(vault: &index::VaultIndex, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(&vault.root).ok()?.to_string_lossy().replace('\\', "/");
    (rel != crate::core::workspace::CONFIG_NAME && rel != crate::core::workspace::STATE_NAME && !vault.config.is_ignored(&rel)).then_some(rel)
}

fn folder_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(up, _)| up)
}

impl Tree {
    fn add_folders(&mut self, dir: &str) {
        let mut parent = String::new();
        for part in dir.split('/').filter(|p| !p.is_empty()) {
            let full = if parent.is_empty() { part.to_string() } else { format!("{parent}/{part}") };
            self.folders.entry(parent.clone()).or_default().insert(full.clone());
            parent = full;
        }
    }

    fn add(&mut self, vault: &index::VaultIndex, path: &Path) {
        let Some(rel) = listed(vault, path) else { return };
        let dir = folder_of(&rel);
        self.add_folders(dir);
        if is_folder(vault, path) {
            self.folders.entry(dir.to_string()).or_default().insert(rel);
            self.drop_gone_folders(&vault.root);
            return;
        }
        let files = self.files.entry(dir.to_string()).or_default();
        if files.iter().any(|p| p == path) {
            return;
        }
        let key = shown_name(path).to_lowercase();
        let at = files.partition_point(|p| shown_name(p).to_lowercase() <= key);
        files.insert(at, path.to_path_buf());
    }

    fn remove_folder(&mut self, rel: &str) {
        if let Some(up) = self.folders.get_mut(folder_of(rel)) {
            up.remove(rel);
        }
        let below = format!("{rel}/");
        self.folders.retain(|k, _| k != rel && !k.starts_with(&below));
        self.files.retain(|k, _| k != rel && !k.starts_with(&below));
    }

    /// Drops the folders gone from disk: a moved folder leaves its old place before the next scan.
    fn drop_gone_folders(&mut self, root: &Path) {
        let gone: Vec<String> = self.folders.values().flatten().filter(|f| !root.join(f).is_dir()).cloned().collect();
        for f in gone {
            self.remove_folder(&f);
        }
    }

    fn remove(&mut self, vault: &index::VaultIndex, path: &Path) {
        let Ok(rel) = path.strip_prefix(&vault.root) else { return };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if let Some(files) = self.files.get_mut(folder_of(&rel)) {
            files.retain(|p| p != path);
        }
        if !path.is_dir()
            && let Some(folders) = self.folders.get_mut(folder_of(&rel))
        {
            folders.remove(&rel);
        }
    }
}

/// A moved folder enters the vault's file list until the next scan.
fn is_folder(vault: &index::VaultIndex, path: &Path) -> bool {
    !vault.notes.contains_key(path) && path.is_dir()
}

/// The file tree, from the vault's own list.
fn build_tree(vault: &index::VaultIndex, hidden: &index::Hidden) -> Tree {
    let mut tree = Tree::default();
    let moved_folders = vault.files.iter().any(|p| is_folder(vault, p));
    let mut exists: HashMap<String, bool> = HashMap::new();
    let mut there = |dir: &str| !moved_folders || dir.is_empty() || *exists.entry(dir.to_string()).or_insert_with(|| vault.root.join(dir).is_dir());
    let mut last_dir: Option<String> = None;
    for path in &vault.files {
        if hidden.hides(path) {
            continue;
        }
        let Some(rel) = listed(vault, path) else { continue };
        let dir = folder_of(&rel);
        if !there(dir) {
            continue;
        }
        if last_dir.as_deref() != Some(dir) {
            tree.add_folders(dir);
            last_dir = Some(dir.to_string());
        }
        if is_folder(vault, path) {
            tree.folders.entry(dir.to_string()).or_default().insert(rel);
            continue;
        }
        match tree.files.get_mut(dir) {
            Some(files) => files.push(path.clone()),
            None => {
                tree.files.insert(dir.to_string(), vec![path.clone()]);
            }
        }
    }
    for path in &vault.folders {
        if hidden.hides(path) {
            continue;
        }
        let Ok(rel) = path.strip_prefix(&vault.root) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if vault.config.is_ignored(&rel) || !there(&rel) {
            continue;
        }
        let parent = rel.rsplit_once('/').map_or(String::new(), |(up, _)| up.to_string());
        tree.folders.entry(parent).or_default().insert(rel);
    }
    for files in tree.files.values_mut() {
        files.sort_by_cached_key(|p| shown_name(p).to_lowercase());
    }
    tree
}

fn shown_name(path: &Path) -> String {
    if markdown::is_markdown(path) { path.file_stem() } else { path.file_name() }.map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

impl NotesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Filtern")));
        let lib = state::lib(cx);
        let subs = vec![
            cx.subscribe(&query, |_, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.observe_global::<Trees>(|_, cx| cx.notify()),
            cx.observe_in(&lib, window, |this, lib, window, cx| {
                let focus_tree = lib.update(cx, |l, _| std::mem::take(&mut l.focus_explorer));
                if focus_tree {
                    window.focus(&this.focus, cx);
                }
                if let Some(folder) = lib.update(cx, |l, _| l.reveal_folder.take()) {
                    this.show_folder(folder, window, cx);
                }
                if lib.read(cx).notes_tag.is_some() && lib.read(cx).notes_tag != this.shown_tag {
                    this.view = View::Tags;
                }
                this.shown_tag = lib.read(cx).notes_tag.clone();
                let active = lib.read(cx).active_document.clone();
                if active != this.revealed {
                    this.revealed = active.clone();
                    if let Some(path) = active {
                        this.reveal(&path);
                    }
                }
                cx.notify();
            }),
        ];
        let waiting = lib.update(cx, |l, _| l.reveal_folder.take());
        let mut page = Self {
            view: View::Files,
            order: Order::Name,
            open_folders: BTreeSet::new(),
            query,
            selected: None,
            target_folder: None,
            renaming: None,
            pending_folder: None,
            width: crate::core::prefs::get().notes_column.unwrap_or(260.),
            resizing: false,
            cache: None,
            focus: cx.focus_handle(),
            list_scroll: UniformListScrollHandle::new(),
            revealed: None,
            shown_tag: None,
            search_rows: Rc::new(Vec::new()),
            searched: String::new(),
            searching: None,
            _subs: subs,
        };
        if let Some(folder) = waiting {
            page.show_folder(folder, window, cx);
        }
        page
    }

    fn show_folder(&mut self, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        self.view = View::Files;
        let mut up = Some(folder.as_str());
        while let Some(f) = up {
            self.open_folders.insert(f.to_string());
            up = f.rsplit_once('/').map(|(a, _)| a);
        }
        self.select_folder(Some(folder.clone()));
        self.target_folder = Some(folder);
        window.focus(&self.focus, cx);
    }

    /// The explorer's full width, its resize handle included.
    pub fn outer_width(&self) -> f32 {
        self.width + space::XS
    }

    /// Takes the keyboard, so arrows walk the list.
    pub fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    /// Opens the folders down to a file, so the open note is always visible in the tree.
    fn reveal(&mut self, path: &Path) {
        let vault = index::current();
        let Ok(rel) = path.strip_prefix(&vault.root) else { return };
        let mut folder = rel.parent().map(Path::to_path_buf);
        while let Some(f) = folder.filter(|f| !f.as_os_str().is_empty()) {
            self.open_folders.insert(f.to_string_lossy().replace('\\', "/"));
            folder = f.parent().map(Path::to_path_buf);
        }
    }

    fn file_row(path: &Path, depth: usize, with_folder: bool, ago: Option<f64>, vault: &index::VaultIndex) -> Row {
        let folder = with_folder.then(|| path.parent().and_then(|p| p.strip_prefix(&vault.root).ok()).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default());
        Row::File { path: path.to_path_buf(), name: shown_name(path), depth, folder, ago }
    }

    fn file_rows(&self, tree: &Tree, folder: &str, depth: usize, vault: &index::VaultIndex, hide: Option<&index::Hidden>, out: &mut Vec<Row>) {
        let pending = self.pending_folder.as_ref().and_then(|p| p.strip_prefix(&vault.root).ok()).map(|p| p.to_string_lossy().replace('\\', "/")).filter(|p| p.rsplit_once('/').map_or("", |(up, _)| up) == folder);
        if tree.folders.contains_key(folder) || pending.is_some() {
            let mut sorted: Vec<String> = tree.folders.get(folder).map(|s| s.iter().cloned().collect()).unwrap_or_default();
            if let Some(h) = hide {
                sorted.retain(|s| !h.hides(&vault.root.join(s)));
            }
            if let Some(p) = pending.filter(|p| !sorted.contains(p)) {
                sorted.push(p);
            }
            sorted.sort_by_cached_key(|s| s.to_lowercase());
            for sub in &sorted {
                let open = self.open_folders.contains(sub);
                out.push(Row::Folder { rel: sub.clone(), name: sub.rsplit('/').next().unwrap_or(sub).to_string(), depth, open });
                if open {
                    self.file_rows(tree, sub, depth + 1, vault, hide, out);
                }
            }
        }
        let in_rename = self.renaming.as_ref().map(|(p, ..)| p).filter(|p| Some(*p) != self.pending_folder.as_ref()).filter(|p| unlisted_rename(p, folder, tree.files.get(folder), &vault.root));
        if let Some(path) = in_rename {
            out.push(Self::file_row(path, depth, false, None, vault));
        }
        if let Some(files) = tree.files.get(folder) {
            let mut sorted: Vec<&PathBuf> = files.iter().filter(|p| hide.is_none_or(|h| !h.hides(p))).collect();
            if self.order == Order::Changed {
                let time = |p: &PathBuf| vault.notes.get(p.as_path()).map_or(0., |f| f.modified);
                sorted.sort_by(|a, b| time(b).total_cmp(&time(a)));
            }
            out.extend(sorted.into_iter().map(|p| Self::file_row(p, depth, false, None, vault)));
        }
    }

    fn visible_tasks(vault: &index::VaultIndex, filter: &str) -> Vec<Row> {
        let mut notes: Vec<(&PathBuf, &index::NoteFacts)> = vault
            .notes
            .iter()
            .filter(|(p, facts)| {
                if facts.tasks.is_empty() {
                    return false;
                }
                let Ok(rel) = p.strip_prefix(&vault.root) else { return false };
                let rel = rel.to_string_lossy().replace('\\', "/");
                !vault.config.is_ignored(&rel) && !vault.is_template(p)
            })
            .collect();
        notes.sort_by(|a, b| b.1.modified.total_cmp(&a.1.modified).then_with(|| a.0.cmp(b.0)));
        let mut rows = Vec::new();
        for (path, facts) in notes {
            let note = shown_name(path);
            for (line, text) in &facts.tasks {
                if filter.is_empty() || text.to_lowercase().contains(filter) || note.to_lowercase().contains(filter) {
                    rows.push(Row::Task { path: path.clone(), note: note.clone(), line: *line, text: text.clone() });
                }
            }
        }
        rows
    }

    /// Searches the whole vault in the background; the list fills in when it is done.
    fn search_vault(&mut self, cx: &mut Context<Self>) {
        let source = self.query.read(cx).value().to_string();
        if source == self.searched {
            return;
        }
        self.searched = source.clone();
        if source.trim().is_empty() {
            self.search_rows = Rc::new(Vec::new());
            self.searching = None;
            self.cache = None;
            return;
        }
        let Ok(query) = crate::core::vault::query::parse(&source) else { return };
        let vault = index::current();
        self.searching = Some(cx.spawn(async move |this, cx| {
            let v = vault.clone();
            let paths = cx.background_spawn(async move { crate::core::vault::query::search(&query, &v, Default::default()) }).await;
            let rows: Vec<Row> = paths.iter().take(2000).map(|p| Self::file_row(p, 0, true, None, &vault)).collect();
            let _ = this.update(cx, |s, cx| {
                s.search_rows = Rc::new(rows);
                s.cache = None;
                cx.notify();
            });
        }));
    }

    /// Everything the list shows; drawing happens often and only asks this cache.
    fn refresh(&mut self, tag: Option<String>, cx: &mut Context<Self>) {
        prepare_tree(cx);
        let vault = index::current();
        let hidden = index::hidden();
        let generation = (vault.generation, hidden.generation);
        let ready = cx.try_global::<Trees>().and_then(|t| t.ready.clone());
        let hide = ready.as_ref().is_some_and(|r| r.generation.1 != hidden.generation).then_some(hidden.as_ref());
        let filter = self.query.read(cx).value().to_lowercase();
        let key = Key { generation, tree: ready.as_ref().map(|r| r.serial), filter: filter.clone(), view: self.view, order: self.order, tag: tag.clone(), open: self.open_folders.iter().cloned().collect(), renaming: self.renaming.as_ref().map(|(p, ..)| p.clone()) };
        if self.cache.as_ref().is_some_and(|c| c.key == key) {
            return;
        }
        let loading = ready.is_none() && self.view == View::Files;
        let tree = ready.map(|r| r.tree).unwrap_or_default();
        let open_tasks = Self::visible_tasks(&vault, "").len();
        let now = chrono::Local::now().timestamp() as f64;
        let rows = match self.view {
            View::Files if filter.is_empty() => {
                let mut rows = Vec::new();
                self.file_rows(&tree, "", 0, &vault, hide, &mut rows);
                rows
            }
            View::Files => {
                let mut hits: Vec<(i64, Row)> = Vec::new();
                for path in tree.files.values().flatten().filter(|p| hide.is_none_or(|h| !h.hides(p))) {
                    let name = shown_name(path);
                    let lower = name.to_lowercase();
                    let alias = vault.notes.get(path.as_path()).is_some_and(|f| f.aliases.iter().any(|a| a.to_lowercase().contains(&filter)));
                    let score = if lower.starts_with(&filter) {
                        0
                    } else if lower.contains(&filter) {
                        1
                    } else if alias {
                        2
                    } else {
                        continue;
                    };
                    hits.push((score, Self::file_row(path, 0, true, None, &vault)));
                }
                hits.sort_by(|a, b| {
                    let name = |r: &Row| if let Row::File { name, .. } = r { name.clone() } else { String::new() };
                    a.0.cmp(&b.0).then_with(|| natural(&name(&a.1), &name(&b.1)))
                });
                hits.truncate(2000);
                hits.into_iter().map(|(_, r)| r).collect()
            }
            View::Tags => match &tag {
                Some(selected) => {
                    let mut rows = vec![Row::BackToTags];
                    let mut notes = vault.notes_with_tag(selected);
                    notes.sort_by(|a, b| natural(&shown_name(a), &shown_name(b)));
                    rows.extend(notes.iter().filter(|p| !hidden.hides(p) && (filter.is_empty() || shown_name(p).to_lowercase().contains(&filter))).map(|p| {
                        let modified = vault.notes.get(p.as_path()).map(|f| now - f.modified);
                        Self::file_row(p, 0, true, modified, &vault)
                    }));
                    rows
                }
                None => vault
                    .tag_tree()
                    .into_iter()
                    .filter(|(t, _)| filter.is_empty() || t.contains(&filter))
                    .map(|(t, count)| {
                        let depth = t.matches('/').count();
                        let label = t.rsplit('/').next().unwrap_or(&t).to_string();
                        Row::Tag { tag: t, label, depth, count }
                    })
                    .collect(),
            },
            View::Marks => vault
                .bookmarks
                .iter()
                .filter(|m| filter.is_empty() || m.title.to_lowercase().contains(&filter))
                .map(|m| Row::Mark { title: m.title.clone(), destination: m.path.as_ref().map(|p| format!("{p}{}", m.subpath.clone().unwrap_or_default())), query: m.query.clone() })
                .collect(),
            View::Tasks => Self::visible_tasks(&vault, &filter),
            View::Search => self.search_rows.as_ref().clone(),
        };
        self.cache = Some(Cache { key, rows: Rc::new(rows), open_tasks, loading });
    }

    fn rows(&self) -> Rc<Vec<Row>> {
        self.cache.as_ref().map(|c| c.rows.clone()).unwrap_or_default()
    }

    fn folder_path(rel: &str) -> PathBuf {
        index::current().root.join(rel)
    }

    /// Where a new note goes: the chosen folder, or next to the open note.
    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let folder = self.target_folder.as_deref().map(Self::folder_path);
        let made = act(cx, |l| {
            let made = l.new_untitled_note(folder.as_deref());
            l.quiet_open = made.is_some();
            made
        });
        if let Some(path) = made {
            state::reindex(cx);
            self.view = View::Files;
            self.reveal(&path);
            self.start_rename(path, window, cx);
        }
    }

    /// Makes the folder and lets its name be typed right in the row.
    fn new_folder(&mut self, within: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let parent = within.or_else(|| self.target_folder.clone()).map_or_else(|| index::current().root.clone(), |f| Self::folder_path(&f));
        if let Some(made) = act(cx, |l| l.create_folder(&tr("Neuer Ordner"), &parent)) {
            state::reindex(cx);
            if let Ok(rel) = made.strip_prefix(&index::current().root) {
                self.target_folder = Some(rel.to_string_lossy().replace('\\', "/"));
            }
            self.pending_folder = Some(made.clone());
            self.view = View::Files;
            self.reveal(&made.join("x"));
            self.start_rename(made, window, cx);
        }
    }

    /// Renames in place: the row becomes a field with the name selected.
    pub fn start_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = shown_name(&path);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name.clone()));
        let selected = if path.is_dir() || markdown::is_markdown(&path) { name.len() } else { crate::core::naming::stem(&name).len() };
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.set_selected_range(0..selected, cx);
        });
        let target = path.clone();
        let sub = cx.subscribe_in(&input, window, move |this, field, e: &InputEvent, window, cx| match e {
            InputEvent::PressEnter { .. } => {
                let text = field.read(cx).value().trim().to_string();
                this.finish_rename(&target, &text, true, window, cx);
            }
            InputEvent::Blur => {
                let text = field.read(cx).value().trim().to_string();
                this.finish_rename(&target, &text, false, window, cx);
            }
            _ => {}
        });
        self.view = View::Files;
        self.query.update(cx, |q, cx| q.set_value("", window, cx));
        self.reveal(&path);
        self.renaming = Some((path, input, sub));
        cx.notify();
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, ..)) = self.renaming.take() else { return };
        if self.pending_folder.as_ref() == Some(&path) || path.is_dir() {
            self.pending_folder = None;
            window.focus(&self.focus, cx);
        } else {
            act(cx, |l| l.focus_document = true);
        }
        cx.notify();
    }

    fn finish_rename(&mut self, path: &Path, text: &str, hand: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.take().is_none() {
            return;
        }
        let is_folder = path.is_dir();
        let was_note = markdown::is_markdown(path);
        let old = shown_name(path);
        if !text.is_empty() && text != old && !text.contains(['/', '\\', ':']) {
            let name = if was_note {
                format!("{text}.{}", path.extension().unwrap_or_default().to_string_lossy())
            } else {
                text.to_string()
            };
            act(cx, |l| l.rename(path, &name));
            state::reindex(cx);
        }
        if is_folder {
            self.pending_folder = None;
        }
        if hand && is_folder {
            window.focus(&self.focus, cx);
        } else if hand {
            act(cx, |l| l.focus_document = true);
        }
        cx.notify();
    }

    fn activate_row(&mut self, row: &Row, double: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(row.clone());
        window.focus(&self.focus, cx);
        match row {
            Row::Folder { rel, .. } => {
                self.target_folder = Some(rel.clone());
                if !self.open_folders.remove(rel) {
                    self.open_folders.insert(rel.clone());
                }
            }
            Row::File { path, .. } => {
                self.target_folder = None;
                let p = path.clone();
                act(cx, |l| {
                    l.quiet_open = true;
                    if double {
                        l.open_file(&p);
                        l.pin_document(&p);
                    } else {
                        l.open_preview(&p);
                    }
                });
            }
            Row::Tag { tag, .. } => {
                let t = tag.clone();
                act(cx, |l| l.notes_tag = Some(t));
            }
            Row::BackToTags => act(cx, |l| l.notes_tag = None),
            Row::Mark { destination, query, .. } => match (query, destination) {
                (Some(q), _) => {
                    let q = q.clone();
                    act(cx, |l| {
                        l.search_query = q;
                        l.set_route(Route::Search);
                    });
                }
                (_, Some(d)) => {
                    let d = d.clone();
                    act(cx, |l| {
                        l.quiet_open = true;
                        l.open_link(&d, None);
                    });
                }
                _ => {}
            },
            Row::Task { path, line, .. } => super::doc_side::open_at(path, *line, cx),
        }
        cx.notify();
    }

    fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|s| rows.iter().position(|r| same_row(r, s)));
        let next = match at {
            Some(i) => i.saturating_add_signed(by).min(rows.len() - 1),
            None => 0,
        };
        let row = rows[next].clone();
        self.selected = Some(row.clone());
        self.list_scroll.scroll_to_item(next, ScrollStrategy::Center);
        if matches!(row, Row::File { .. }) {
            self.activate_row(&row, false, window, cx);
        }
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let selected = self.selected.clone();
        match e.keystroke.key.as_str() {
            "down" => self.step(1, window, cx),
            "up" => self.step(-1, window, cx),
            "right" => {
                if let Some(Row::Folder { rel, open: false, .. }) = &selected {
                    self.open_folders.insert(rel.clone());
                }
            }
            "left" => match &selected {
                Some(Row::Folder { rel, open: true, .. }) => {
                    self.open_folders.remove(rel);
                }
                Some(Row::Folder { rel, .. }) => self.select_folder(rel.rsplit_once('/').map(|(up, _)| up.to_string())),
                Some(Row::File { path, .. }) => {
                    let parent = path.parent().and_then(|p| p.strip_prefix(&index::current().root).ok()).map(|p| p.to_string_lossy().replace('\\', "/")).filter(|p| !p.is_empty());
                    self.select_folder(parent);
                }
                _ => {}
            },
            "enter" => match selected {
                Some(Row::File { .. }) => act(cx, |l| l.focus_document = true),
                Some(row) => self.activate_row(&row, false, window, cx),
                None => {}
            },
            "f2" => {
                match selected {
                    Some(Row::File { path, .. }) => self.start_rename(path, window, cx),
                    Some(Row::Folder { rel, .. }) => self.start_rename(Self::folder_path(&rel), window, cx),
                    _ => {}
                }
            }
            "space" => {
                if let Some(Row::Task { path, line, text, .. }) = selected {
                    finish(&path, line, &text, cx);
                }
            }
            "/" => self.query.update(cx, |q, cx| q.focus(window, cx)),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn select_folder(&mut self, rel: Option<String>) {
        if let Some(rel) = rel {
            let depth = rel.matches('/').count();
            let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
            self.selected = Some(Row::Folder { open: self.open_folders.contains(&rel), rel, name, depth });
        }
    }

    fn view_button(&self, view: View, icon_name: &'static str, label: String, badge: usize, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let on = self.view == view;
        let hover = t.ink.opacity(0.06);
        let accent = t.accent;
        div()
            .id(icon_name)
            .relative()
            .size(px(control::M))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(radius::INNER))
            .cursor_pointer()
            .key_nav()
            .aria_label(label.clone())
            .when(on, |d| d.bg(t.ink.opacity(0.1)))
            .when(!on, |d| d.soft(hover, radius::INNER))
            .child(icon(icon_name, iconsize::M, if on { t.primary } else { t.secondary }))
            .when(badge > 0, |d| d.child(div().absolute().top(px(space::XXS)).right(px(space::XXS)).size(px(space::XS + space::XXS)).rounded_full().bg(accent)))
            .tooltip(tip(if badge > 0 { format!("{label} ({badge})") } else { label }))
            .on_click(cx.listener(move |s, _, _, cx| {
                s.view = view;
                s.selected = None;
                cx.notify();
            }))
    }

    fn line(&self, row: &Row, i: usize, active: Option<&PathBuf>, me: &Entity<Self>, t: &Theme) -> AnyElement {
        let hover = t.ink.opacity(0.05);
        let selected = self.selected.as_ref().is_some_and(|s| same_row(s, row));
        let base = |id: (&'static str, usize)| {
            div()
                .id(id)
                .w_full()
                .h(px(control::M))
                .pr(px(space::S + space::XS))
                .flex()
                .items_center()
                .gap(px(space::XS))
                .cursor_pointer()
                .text_size(px(font::CALLOUT))
                .when(selected, |d| d.bg(t.ink.opacity(0.08)))
                .when(!selected, |d| d.soft(hover, 0.))
        };
        let click = |row: &Row, me: &Entity<Self>| {
            let (row, me) = (row.clone(), me.clone());
            move |e: &ClickEvent, window: &mut Window, cx: &mut App| {
                let double = e.click_count() >= 2;
                me.update(cx, |s, cx| s.activate_row(&row, double, window, cx));
            }
        };
        match row {
            Row::Folder { rel, name, depth, open } => {
                if let Some((renamed, input, _)) = &self.renaming
                    && *renamed == Self::folder_path(rel)
                {
                    return div().w_full().h(px(control::M)).pl(px(space::S + *depth as f32 * space::M + iconsize::M + space::XS)).pr(px(space::S)).flex().items_center().capture_action(cancel_on_escape(me)).child(Input::new(input).small()).into_any_element();
                }
                let (drop_rel, menu_rel, menu_me, drop_me) = (rel.clone(), rel.clone(), me.clone(), me.clone());
                let accent = t.accent;
                base(("tree-folder", i))
                    .mark("folder", name.clone())
                    .pl(px(space::S + *depth as f32 * space::M))
                    .font_weight(FontWeight::MEDIUM)
                    .child(chevron(("tree-chevron", i), *open, t.secondary))
                    .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                    .drag_over::<DraggedFile>(move |s, _, _, _| s.bg(accent.opacity(0.2)))
                    .on_drop(move |d: &DraggedFile, _, cx| {
                        let (path, folder) = (d.0.clone(), NotesPage::folder_path(&drop_rel));
                        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                        act(cx, |l| {
                            l.file(&path, &folder, &name, true);
                        });
                        state::reindex(cx);
                        drop_me.update(cx, |_, cx| cx.notify());
                    })
                    .on_click(click(row, me))
                    .context_menu(move |m, _, cx| {
                        let (a, b, c, r) = (menu_me.clone(), menu_me.clone(), menu_rel.clone(), NotesPage::folder_path(&menu_rel));
                        let d = menu_rel.clone();
                        let (rename_path, trash_path, rename_me) = (NotesPage::folder_path(&menu_rel), NotesPage::folder_path(&menu_rel), menu_me.clone());
                        let module_rel = menu_rel.clone();
                        let is_module = state::read(cx, |l| l.config.as_ref().is_some_and(|c| c.modules.iter().any(|m| m.folder == menu_rel)));
                        m.item(PopupMenuItem::new(tr("Neue Notiz")).icon(menu_icon("square-pen")).on_click(move |_, window, cx| {
                            let folder = c.clone();
                            a.update(cx, |s, cx| {
                                s.target_folder = Some(folder);
                                s.new_note(window, cx);
                            });
                        }))
                        .item(PopupMenuItem::new(tr("Neuer Ordner")).icon(menu_icon("folder-plus")).on_click(move |_, window, cx| {
                            let folder = d.clone();
                            b.update(cx, |s, cx| s.new_folder(Some(folder), window, cx));
                        }))
                        .separator()
                        .item(PopupMenuItem::new(if is_module { tr("Kein Modul mehr") } else { tr("Als Modul verwenden") }).icon(menu_icon("library")).on_click(move |_, _, cx| {
                            let folder = module_rel.clone();
                            act(cx, |l| l.toggle_module_folder(&folder));
                        }))
                        .item(PopupMenuItem::new(super::platform::reveal_label()).icon(menu_icon("folder")).on_click(move |_, _, _| crate::core::library::reveal(&r)))
                        .separator()
                        .item(PopupMenuItem::new(format!("{} (F2)", tr("Umbenennen…"))).icon(menu_icon("pencil")).on_click(move |_, window, cx| {
                            let p = rename_path.clone();
                            rename_me.update(cx, |s, cx| s.start_rename(p, window, cx));
                        }))
                        .item(PopupMenuItem::new(tr("In den Papierkorb")).icon(menu_icon("trash-2")).on_click(move |_, _, cx| {
                            state::trash(vec![trash_path.clone()], cx);
                        }))
                    })
                    .into_any_element()
            }
            Row::File { path, name, depth, folder, ago } => {
                let is_active = active == Some(path);
                if let Some((renamed, input, _)) = &self.renaming
                    && renamed == path
                {
                    return div().w_full().h(px(control::M)).pl(px(space::S + *depth as f32 * space::M + iconsize::M + space::XS)).pr(px(space::S)).flex().items_center().capture_action(cancel_on_escape(me)).child(Input::new(input).small()).into_any_element();
                }
                let (menu, rename, trash, drag) = (path.clone(), path.clone(), path.clone(), path.clone());
                let rename_me = me.clone();
                base(("tree-file", i))
                    .mark("file", name.clone())
                    .pl(px(space::S + *depth as f32 * space::M + iconsize::XS + space::XS))
                    .when(is_active, |d| d.bg(t.accent.opacity(0.18)))
                    .child(file_icon(path, iconsize::S))
                    .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().child(name.clone())))
                    .children(folder.as_ref().filter(|f| !f.is_empty()).map(|f| caption(f.clone(), t).flex_none().max_w(px(90.)).truncate()))
                    .children(ago.map(|a| caption(changed_ago(a), t).flex_none()))
                    .on_drag(DraggedFile(drag), |d, _, _, cx| cx.new(|_| d.clone()))
                    .on_click(click(row, me))
                    .context_menu(move |m, window, cx| {
                        let (r, m2, x) = (rename.clone(), rename_me.clone(), trash.clone());
                        deadline_submenu(file_menu(m, menu.clone()).separator(), &menu, window, cx)
                            .separator()
                            .item(PopupMenuItem::new(format!("{} (F2)", tr("Umbenennen…"))).icon(menu_icon("pencil")).on_click(move |_, window, cx| {
                                let p = r.clone();
                                m2.update(cx, |s, cx| s.start_rename(p, window, cx));
                            }))
                            .when_some(super::convention_rename::menu_item_or_hint(&menu, cx), |m, item| m.item(item))
                            .item(PopupMenuItem::new(tr("In den Papierkorb")).icon(menu_icon("trash-2")).on_click(move |_, _, cx| {
                                state::trash(vec![x.clone()], cx);
                            }))
                    })
                    .into_any_element()
            }
            Row::Tag { label, depth, count, .. } => base(("tree-tag", i))
                .mark("tag", format!("#{label}"))
                .pl(px(space::M + *depth as f32 * space::M))
                .child(div().flex_1().min_w_0().truncate().child(format!("#{label}")))
                .child(caption(count.to_string(), t).flex_none().font_features(tabular()))
                .on_click(click(row, me))
                .into_any_element(),
            Row::BackToTags => base(("tree-back", i)).pl(px(space::S)).text_color(t.secondary).child(icon("arrow-left", iconsize::S, t.secondary)).child(tr("Alle Tags")).on_click(click(row, me)).into_any_element(),
            Row::Mark { title, query, .. } => base(("tree-mark", i))
                .mark("bookmark", title.clone())
                .pl(px(space::M))
                .child(icon(if query.is_some() { "search" } else { "bookmark" }, iconsize::S, t.secondary))
                .child(div().flex_1().min_w_0().truncate().child(title.clone()))
                .tooltip(tip(tr("Lesezeichen setzt du über die Befehlspalette, sie stehen auch in Obsidian.")))
                .on_click(click(row, me))
                .into_any_element(),
            Row::Task { path, note, line, text } => {
                let (tick, menu, line, label) = (path.clone(), path.clone(), *line, text.clone());
                base(("tree-task", i))
                    .mark("task", text.clone())
                    .pl(px(space::M))
                    .gap(px(space::S))
                    .child(div().id(("task-box", i)).flex_none().size(px(iconsize::S)).rounded(px(radius::SMALL)).soft(t.ink.opacity(0.08), radius::SMALL).border_1().border_color(t.secondary).cursor_pointer().aria_label(tr("Abhaken")).tooltip(tip(tr("Abhaken"))).on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        finish(&tick, line, &label, cx);
                    }))
                    .child(div().flex_1().min_w_0().truncate().child(text.clone()))
                    .child(caption(note.clone(), t).flex_none().max_w(px(90.)).truncate())
                    .on_click(click(row, me))
                    .context_menu(move |m, _, cx| file_menu(m, menu.clone()).when_some(super::convention_rename::menu_item(&menu, cx), |m, item| m.item(item)))
                    .into_any_element()
            }
        }
    }
}

/// Shows or hides the explorer beside the document.
pub fn set_hidden(hidden: bool, cx: &mut App) {
    crate::core::prefs::update(|p| p.notes_explorer_hidden = hidden);
    state::lib(cx).update(cx, |_, cx| cx.notify());
}

/// Two rows mean the same thing; folders change their `open` without becoming another row.
fn same_row(a: &Row, b: &Row) -> bool {
    match (a, b) {
        (Row::Folder { rel: x, .. }, Row::Folder { rel: y, .. }) => x == y,
        (Row::File { path: x, .. }, Row::File { path: y, .. }) => x == y,
        (Row::Task { path: x, line: l, .. }, Row::Task { path: y, line: m, .. }) => x == y && l == m,
        _ => a == b,
    }
}

/// Ticks off a task and brings the list up to date right away.
fn finish(path: &Path, line: usize, text: &str, cx: &mut App) {
    if act(cx, |l| l.finish_task(path, line, text)) {
        state::reindex(cx);
    }
}

/// When a note last changed, short enough for a row.
pub fn changed_ago(seconds: f64) -> String {
    let minutes = (seconds.max(0.) / 60.) as i64;
    match minutes {
        0 => tr("Gerade eben"),
        1..=59 => trf("Vor %@ Min.", &[&minutes.to_string()]),
        60..=1439 => trf("Vor %@ Std.", &[&(minutes / 60).to_string()]),
        1440..=2879 => tr("Gestern"),
        _ => trf("Vor %@ Tagen", &[&(minutes / 1440).to_string()]),
    }
}

/// A divider being dragged to change the explorer's width.
#[derive(Clone)]
struct ExplorerSplit;

impl Render for ExplorerSplit {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Render for NotesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(path) = state::lib(cx).update(cx, |l, _| l.rename_request.take()) {
            self.start_rename(path, window, cx);
        }
        let t = Theme::get(cx);
        let (tag, active, obsidian) = state::read(cx, |l| (l.notes_tag.clone(), l.active_document.clone(), l.is_vault));
        if self.view == View::Marks && !obsidian {
            self.view = View::Files;
        }
        if self.view == View::Search {
            self.search_vault(cx);
        }
        if let Some((gone, ..)) = self.renaming.take_if(|(p, ..)| !p.exists()) {
            if self.pending_folder.take_if(|f| *f == gone).is_some() {
                let focus = self.focus.clone();
                window.on_next_frame(move |window, cx| window.focus(&focus, cx));
            } else {
                window.on_next_frame(|_, cx| act(cx, |l| l.focus_document = true));
            }
        }
        self.refresh(tag.clone(), cx);
        let rows = self.rows();
        let open_tasks = self.cache.as_ref().map_or(0, |c| c.open_tasks);
        let view = self.view;
        let order = self.order;
        let me = cx.entity();
        let marks = obsidian.then(|| self.view_button(View::Marks, "bookmark", tr("Lesezeichen"), 0, &t, cx));
        let header = div()
            .flex_none()
            .h(px(control::SUBBAR))
            .px(px(space::S))
            .flex()
            .items_center()
            .gap(px(space::XXS))
            .border_b_1()
            .border_color(t.separator)
            .child(self.view_button(View::Files, "folder-tree", tr("Dateien"), 0, &t, cx))
            .child(self.view_button(View::Search, "search", tr("Suche"), 0, &t, cx))
            .child(self.view_button(View::Tags, "tag", tr("Tags"), 0, &t, cx))
            .children(marks)
            .child(self.view_button(View::Tasks, "circle-check", tr("Aufgaben"), open_tasks, &t, cx))
            .child(div().flex_1())
            .child(Button::new("explorer-hide").ghost().small().icon(menu_icon("panel-left-close")).tooltip(tr("Dateien ausblenden")).on_click(|_, _, cx| set_hidden(true, cx)))
            .child(Button::new("explorer-new").ghost().small().icon(menu_icon("square-pen")).tooltip(format!("{}{}", tr("Neue Notiz"), crate::core::shortcuts::ShortcutAction::NewFile.hint())).on_click(cx.listener(|s, _, window, cx| s.new_note(window, cx))))
            .child(Button::new("explorer-more").ghost().small().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu({
                let me = me.clone();
                move |menu, _, _| {
                    let (a, b, c, d) = (me.clone(), me.clone(), me.clone(), me.clone());
                    menu.item(PopupMenuItem::new(tr("Neuer Ordner")).icon(menu_icon("folder-plus")).on_click(move |_, window, cx| a.update(cx, |s, cx| s.new_folder(None, window, cx))))
                        .item(PopupMenuItem::new(format!("{}{}", tr("Tagesnotiz öffnen"), crate::core::shortcuts::ShortcutAction::DailyNote.hint())).icon(menu_icon("calendar")).on_click(|_, _, cx| {
                            act(cx, |l| l.open_daily_note());
                            state::reindex(cx);
                        }))
                        .item(PopupMenuItem::new(format!("{}{}", tr("Graph zeigen"), crate::core::shortcuts::ShortcutAction::ShowGraph.hint())).icon(menu_icon("waypoints")).on_click(|_, _, cx| act(cx, |l| l.show_graph(None))))
                        .separator()
                        .item(PopupMenuItem::new(tr("Nach Name sortieren")).checked(order == Order::Name).on_click(move |_, _, cx| {
                            b.update(cx, |s, cx| {
                                s.order = Order::Name;
                                cx.notify();
                            })
                        }))
                        .item(PopupMenuItem::new(tr("Nach Änderung sortieren")).checked(order == Order::Changed).on_click(move |_, _, cx| {
                            c.update(cx, |s, cx| {
                                s.order = Order::Changed;
                                cx.notify();
                            })
                        }))
                        .item(PopupMenuItem::new(tr("Alle einklappen")).on_click(move |_, _, cx| {
                            d.update(cx, |s, cx| {
                                s.open_folders.clear();
                                cx.notify();
                            })
                        }))
                }
            }));
        let filter = div().flex_none().p(px(space::S)).child(Input::new(&self.query).small());
        let filtered = !self.query.read(cx).value().is_empty() && view != View::Search;
        let loading = self.cache.as_ref().is_some_and(|c| c.loading);
        let body: AnyElement = if rows.is_empty() && loading {
            div().size_full().flex().items_center().justify_center().child(motion::loading(iconsize::L, t.secondary)).into_any_element()
        } else if rows.is_empty() {
            let (name, title, detail) = match view {
                _ if filtered => ("search", tr("Nichts gefunden"), tr("Ändere den Filter.")),
                View::Search => ("search", tr("Inhalt durchsuchen"), tr("Tippe oben ein Wort oder Suchbefehle wie tag:#x.")),
                View::Files => ("book-open", tr("Noch keine Notizen"), tr("Mit ⌘N legst du die erste an.")),
                View::Tags => ("tag", tr("Noch keine Tags"), tr("Schreibe #tag in eine Notiz.")),
                View::Marks => ("bookmark", tr("Keine Lesezeichen"), tr("Lesezeichen setzt du über die Befehlspalette, sie stehen auch in Obsidian.")),
                View::Tasks => ("circle-check", tr("Alles erledigt"), tr("In deinen Notizen ist keine Aufgabe mehr offen.")),
            };
            unavailable(name, title, detail, &t).into_any_element()
        } else {
            let count = rows.len();
            let (list, list_me) = (rows.clone(), me.clone());
            uniform_list("explorer-list", count, move |range, _, cx| {
                let t = Theme::get(cx);
                let active = state::read(cx, |l| l.active_document.clone());
                let page = list_me.read(cx);
                range.map(|i| page.line(&list[i], i, active.as_ref(), &list_me, &t)).collect::<Vec<_>>()
            })
            .track_scroll(&self.list_scroll)
            .size_full()
            .into_any_element()
        };
        let _ = active;
        let accent = t.accent;
        let handle = div().id("explorer-split").occlude().absolute().top_0().bottom_0().right_0().w(px(space::XS)).flex_none().cursor_col_resize().border_r_1().border_color(t.separator).soft(accent.opacity(0.3), 0.).on_drag(ExplorerSplit, |d, _, _, cx| cx.new(|_| d.clone()));
        if self.resizing && !cx.has_active_drag() {
            self.resizing = false;
            let width = self.width;
            crate::core::prefs::update(|p| p.notes_column = Some(width));
        }
        div()
            .id("explorer")
            .relative()
            .track_focus(&self.focus)
            .key_context("Explorer")
            .h_full()
            .flex_none()
            .flex()
            .on_key_down(cx.listener(Self::on_key))
            .on_action(|_: &super::app::TogglePreview, _, cx| act(cx, |l| l.toggle_mode = l.active_document.as_deref().is_some_and(markdown::is_markdown)))
            .on_drag_move(cx.listener(|s, e: &DragMoveEvent<ExplorerSplit>, _, cx| {
                let width = f32::from(e.event.position.x - e.bounds.left()).round().clamp(180., 420.);
                s.resizing = true;
                if width != s.width {
                    s.width = width;
                    cx.notify();
                }
            }))
            .child(div().w(px(self.outer_width())).h_full().flex().flex_col().bg(t.column).child(header).child(filter).child(div().flex_1().min_h_0().child(body)))
            .child(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::{build_tree, changed_files, unlisted_rename};
    use crate::core::vault::index::{Hidden, VaultIndex};
    use std::path::PathBuf;

    fn vault(files: &[&str]) -> VaultIndex {
        let root = PathBuf::from("/vault");
        VaultIndex { files: files.iter().map(|f| root.join(f)).collect(), root, ..Default::default() }
    }

    #[test]
    fn a_moved_file_is_patched_into_the_tree() {
        let before = vault(&["A/a.md", "A/b.md", "B/c.md", "d.md"]);
        let after = vault(&["A/a.md", "B/c.md", "d.md", "C/D/Beispiel.md"]);
        let (gone, came) = changed_files(&before.files, &after.files).unwrap();
        assert_eq!(gone, vec![PathBuf::from("/vault/A/b.md")]);
        assert_eq!(came, vec![PathBuf::from("/vault/C/D/Beispiel.md")]);
        let mut tree = build_tree(&before, &Hidden::default());
        for p in &gone {
            tree.remove(&after, p);
        }
        for p in &came {
            tree.add(&after, p);
        }
        let mut fresh = build_tree(&after, &Hidden::default());
        fresh.files.entry("A".into()).or_default();
        assert_eq!(tree.files, fresh.files);
        assert!(tree.folders.get("C").is_some_and(|s| s.contains("C/D")));
        assert!(tree.folders.get("").is_some_and(|s| s.contains("C")));
    }

    #[test]
    fn a_folder_moved_to_another_parent_leaves_its_old_place() {
        let root = std::env::temp_dir().join(format!("schrumbi-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("B/Woche 1.2")).unwrap();
        std::fs::create_dir_all(root.join("A")).unwrap();
        let at = |f: &str| root.join(f);
        let before = VaultIndex { root: root.clone(), files: vec![at("A/Woche 1.2/n.md"), at("B/m.md")], ..Default::default() };
        let after = VaultIndex { root: root.clone(), files: vec![at("A/Woche 1.2/n.md"), at("B/m.md"), at("B/Woche 1.2")], ..Default::default() };
        let mut tree = build_tree(&before, &Hidden::default());
        let (gone, came) = changed_files(&before.files, &after.files).unwrap();
        for p in &gone {
            tree.remove(&after, p);
        }
        for p in &came {
            tree.add(&after, p);
        }
        let fresh = build_tree(&after, &Hidden::default());
        let _ = std::fs::remove_dir_all(&root);
        for t in [&tree, &fresh] {
            assert!(t.folders.get("B").is_some_and(|s| s.contains("B/Woche 1.2")));
            assert!(!t.folders.get("A").is_some_and(|s| s.contains("A/Woche 1.2")));
            assert!(!t.files.contains_key("A/Woche 1.2"));
            assert!(!t.files.get("B").is_some_and(|f| f.iter().any(|p| p.ends_with("Woche 1.2"))));
        }
    }

    #[test]
    fn a_renamed_file_keeps_its_sorted_place() {
        let before = vault(&["n/a.md", "n/c.md", "n/e.md"]);
        let after = vault(&["n/a.md", "n/e.md", "n/d.md"]);
        let (gone, came) = changed_files(&before.files, &after.files).unwrap();
        let mut tree = build_tree(&before, &Hidden::default());
        for p in &gone {
            tree.remove(&after, p);
        }
        for p in &came {
            tree.add(&after, p);
        }
        assert_eq!(tree, build_tree(&after, &Hidden::default()));
    }

    #[test]
    fn many_changes_or_a_new_order_are_left_to_a_full_build() {
        let a: Vec<PathBuf> = (0..200).map(|i| PathBuf::from(format!("/v/{i}.md"))).collect();
        let b: Vec<PathBuf> = (0..200).map(|i| PathBuf::from(format!("/w/{i}.md"))).collect();
        assert!(changed_files(&a, &b).is_none());
        let swapped = vec![a[1].clone(), a[0].clone()];
        assert_eq!(changed_files(&a[..2], &swapped), Some((vec![], vec![])));
    }

    #[test]
    fn rename_row_goes_when_its_file_moves() {
        let root = std::env::temp_dir().join(format!("schrumbi-rename-row-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let made = root.join("Unbenannt.md");
        std::fs::write(&made, "").unwrap();
        assert!(unlisted_rename(&made, "", None, &root));
        assert!(!unlisted_rename(&made, "", Some(&vec![made.clone()]), &root));
        assert!(!unlisted_rename(&made, "Sub", None, &root));
        std::fs::rename(&made, root.join("Hallo.md")).unwrap();
        assert!(!unlisted_rename(&made, "", None, &root));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
