use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::browser::DraggedFile;
use super::motion::{Soft, chevron};
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
#[derive(Default)]
struct Tree {
    folders: BTreeMap<String, BTreeSet<String>>,
    files: BTreeMap<String, Vec<PathBuf>>,
}

struct Cache {
    key: (u64, String, View, Order, Option<String>, Vec<String>, Option<PathBuf>),
    rows: Rc<Vec<Row>>,
    open_tasks: usize,
}

/// The left side of the notes room: everything in the vault, with the document beside it.
pub struct NotesPage {
    view: View,
    order: Order,
    open_folders: BTreeSet<String>,
    query: Entity<InputState>,
    selected: Option<Row>,
    /// The folder new notes land in.
    target_folder: Option<String>,
    renaming: Option<(PathBuf, Entity<InputState>, Subscription)>,
    width: f32,
    tree: Option<(u64, Rc<Tree>)>,
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

/// The file tree, from the vault's own list; the workspace's settings file does not belong in it.
fn build_tree(vault: &index::VaultIndex) -> Tree {
    let mut tree = Tree::default();
    for path in &vault.files {
        let Ok(rel) = path.strip_prefix(&vault.root) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel == crate::core::workspace::CONFIG_NAME || rel == crate::core::workspace::STATE_NAME || vault.config.is_ignored(&rel) {
            continue;
        }
        let mut parent = String::new();
        let parts: Vec<&str> = rel.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            let full = if parent.is_empty() { part.to_string() } else { format!("{parent}/{part}") };
            tree.folders.entry(parent.clone()).or_default().insert(full.clone());
            parent = full;
        }
        tree.files.entry(parent).or_default().push(path.clone());
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
            cx.observe_in(&lib, window, |this, lib, window, cx| {
                let focus_tree = lib.update(cx, |l, _| std::mem::take(&mut l.focus_explorer));
                if focus_tree {
                    window.focus(&this.focus, cx);
                }
                if let Some(folder) = lib.update(cx, |l, _| l.reveal_folder.take()) {
                    this.view = View::Files;
                    let mut up = Some(folder.as_str());
                    while let Some(f) = up {
                        this.open_folders.insert(f.to_string());
                        up = f.rsplit_once('/').map(|(a, _)| a);
                    }
                    this.select_folder(Some(folder.clone()));
                    this.target_folder = Some(folder);
                    window.focus(&this.focus, cx);
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
        Self {
            view: View::Files,
            order: Order::Name,
            open_folders: BTreeSet::new(),
            query,
            selected: None,
            target_folder: None,
            renaming: None,
            width: crate::core::prefs::get().notes_column.unwrap_or(260.),
            tree: None,
            cache: None,
            focus: cx.focus_handle(),
            list_scroll: UniformListScrollHandle::new(),
            revealed: None,
            shown_tag: None,
            search_rows: Rc::new(Vec::new()),
            searched: String::new(),
            searching: None,
            _subs: subs,
        }
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

    fn file_rows(&self, tree: &Tree, folder: &str, depth: usize, vault: &index::VaultIndex, out: &mut Vec<Row>) {
        if let Some(subfolders) = tree.folders.get(folder) {
            let mut sorted: Vec<&String> = subfolders.iter().collect();
            sorted.sort_by(|a, b| natural(a, b));
            for sub in sorted {
                let open = self.open_folders.contains(sub);
                out.push(Row::Folder { rel: sub.clone(), name: sub.rsplit('/').next().unwrap_or(sub).to_string(), depth, open });
                if open {
                    self.file_rows(tree, sub, depth + 1, vault, out);
                }
            }
        }
        let in_rename = self.renaming.as_ref().map(|(p, ..)| p).filter(|p| {
            !tree.files.get(folder).is_some_and(|f| f.contains(p)) && p.parent().and_then(|d| d.strip_prefix(&vault.root).ok()).is_some_and(|d| d.to_string_lossy().replace('\\', "/") == folder)
        });
        if let Some(path) = in_rename {
            out.push(Self::file_row(path, depth, false, None, vault));
        }
        if let Some(files) = tree.files.get(folder) {
            let mut sorted: Vec<&PathBuf> = files.iter().collect();
            match self.order {
                Order::Name => sorted.sort_by(|a, b| natural(&shown_name(a), &shown_name(b))),
                Order::Changed => sorted.sort_by(|a, b| {
                    let time = |p: &PathBuf| vault.notes.get(p.as_path()).map_or(0., |f| f.modified);
                    time(b).total_cmp(&time(a)).then_with(|| natural(&shown_name(a), &shown_name(b)))
                }),
            }
            out.extend(sorted.into_iter().map(|p| Self::file_row(p, depth, false, None, vault)));
        }
    }

    fn visible_tasks(vault: &index::VaultIndex, filter: &str) -> Vec<Row> {
        let mut notes: Vec<(&PathBuf, &index::NoteFacts)> = vault
            .notes
            .iter()
            .filter(|(p, facts)| {
                let Ok(rel) = p.strip_prefix(&vault.root) else { return false };
                let rel = rel.to_string_lossy().replace('\\', "/");
                !facts.tasks.is_empty() && !vault.config.is_ignored(&rel) && !vault.is_template(p)
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
    fn refresh(&mut self, tag: Option<String>, cx: &App) {
        let vault = index::current();
        let filter = self.query.read(cx).value().to_lowercase();
        let key = (vault.generation, filter.clone(), self.view, self.order, tag.clone(), self.open_folders.iter().cloned().collect::<Vec<_>>(), self.renaming.as_ref().map(|(p, ..)| p.clone()));
        if self.cache.as_ref().is_some_and(|c| c.key == key) {
            return;
        }
        if self.tree.as_ref().is_none_or(|(g, _)| *g != vault.generation) {
            self.tree = Some((vault.generation, Rc::new(build_tree(&vault))));
        }
        let tree = self.tree.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
        let open_tasks = Self::visible_tasks(&vault, "").len();
        let now = chrono::Local::now().timestamp() as f64;
        let rows = match self.view {
            View::Files if filter.is_empty() => {
                let mut rows = Vec::new();
                self.file_rows(&tree, "", 0, &vault, &mut rows);
                rows
            }
            View::Files => {
                let mut hits: Vec<(i64, Row)> = Vec::new();
                for path in tree.files.values().flatten() {
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
                    rows.extend(notes.iter().filter(|p| filter.is_empty() || shown_name(p).to_lowercase().contains(&filter)).map(|p| {
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
        self.cache = Some(Cache { key, rows: Rc::new(rows), open_tasks });
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
        let made = act(cx, |l| l.new_untitled_note(folder.as_deref()));
        if let Some(path) = made {
            state::reindex(cx);
            self.view = View::Files;
            self.reveal(&path);
            self.start_rename(path, window, cx);
        }
    }

    fn new_folder(&mut self, within: Option<String>, cx: &mut Context<Self>) {
        let parent = within.or_else(|| self.target_folder.clone()).map_or_else(|| index::current().root.clone(), |f| Self::folder_path(&f));
        if let Some(made) = act(cx, |l| l.create_folder(&tr("Neuer Ordner"), &parent)) {
            state::reindex(cx);
            if let Ok(rel) = made.strip_prefix(&index::current().root) {
                self.target_folder = Some(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    /// Renames in place: the row becomes a field with the name selected.
    pub fn start_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = shown_name(&path);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name.clone()));
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.set_selected_range(0..name.len(), cx);
        });
        let target = path.clone();
        let sub = cx.subscribe_in(&input, window, move |this, field, e: &InputEvent, window, cx| match e {
            InputEvent::PressEnter { .. } => {
                let text = field.read(cx).value().trim().to_string();
                this.finish_rename(&target, &text, window, cx);
            }
            InputEvent::Blur => {
                let text = field.read(cx).value().trim().to_string();
                this.finish_rename(&target, &text, window, cx);
            }
            _ => {}
        });
        self.view = View::Files;
        self.query.update(cx, |q, cx| q.set_value("", window, cx));
        self.reveal(&path);
        self.renaming = Some((path, input, sub));
        cx.notify();
    }

    fn finish_rename(&mut self, path: &Path, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.take().is_none() {
            return;
        }
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
        act(cx, |l| l.focus_document = true);
        let _ = window;
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
                if let Some(Row::File { path, .. }) = selected {
                    self.start_rename(path, window, cx);
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
                .pr(px(space::S))
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
                let (drop_rel, menu_rel, menu_me, drop_me) = (rel.clone(), rel.clone(), me.clone(), me.clone());
                let accent = t.accent;
                base(("tree-folder", i))
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
                    .context_menu(move |m, _, _| {
                        let (a, b, c, r) = (menu_me.clone(), menu_me.clone(), menu_rel.clone(), NotesPage::folder_path(&menu_rel));
                        let d = menu_rel.clone();
                        m.item(PopupMenuItem::new(tr("Neue Notiz")).icon(menu_icon("square-pen")).on_click(move |_, window, cx| {
                            let folder = c.clone();
                            a.update(cx, |s, cx| {
                                s.target_folder = Some(folder);
                                s.new_note(window, cx);
                            });
                        }))
                        .item(PopupMenuItem::new(tr("Neuer Ordner")).icon(menu_icon("folder-plus")).on_click(move |_, _, cx| {
                            let folder = d.clone();
                            b.update(cx, |s, cx| s.new_folder(Some(folder), cx));
                        }))
                        .separator()
                        .item(PopupMenuItem::new(super::platform::reveal_label()).icon(menu_icon("folder")).on_click(move |_, _, _| crate::core::library::reveal(&r)))
                    })
                    .into_any_element()
            }
            Row::File { path, name, depth, folder, ago } => {
                let is_active = active == Some(path);
                if let Some((renamed, input, _)) = &self.renaming
                    && renamed == path
                {
                    return div().w_full().h(px(control::M)).pl(px(space::S + *depth as f32 * space::M + iconsize::M + space::XS)).pr(px(space::S)).flex().items_center().child(Input::new(input).small()).into_any_element();
                }
                let (menu, rename, trash, drag) = (path.clone(), path.clone(), path.clone(), path.clone());
                let rename_me = me.clone();
                base(("tree-file", i))
                    .pl(px(space::S + *depth as f32 * space::M + iconsize::XS + space::XS))
                    .when(is_active, |d| d.bg(t.accent.opacity(0.18)))
                    .child(file_icon(path, iconsize::S))
                    .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().child(name.clone())))
                    .children(folder.as_ref().filter(|f| !f.is_empty()).map(|f| caption(f.clone(), t).flex_none().max_w(px(90.)).truncate()))
                    .children(ago.map(|a| caption(changed_ago(a), t).flex_none()))
                    .on_drag(DraggedFile(drag), |d, _, _, cx| cx.new(|_| d.clone()))
                    .on_click(click(row, me))
                    .context_menu(move |m, _, _| {
                        let (r, m2, x) = (rename.clone(), rename_me.clone(), trash.clone());
                        file_menu(m, menu.clone())
                            .separator()
                            .item(PopupMenuItem::new(tr("Umbenennen")).icon(menu_icon("pencil")).on_click(move |_, window, cx| {
                                let p = r.clone();
                                m2.update(cx, |s, cx| s.start_rename(p, window, cx));
                            }))
                            .item(PopupMenuItem::new(tr("In den Papierkorb")).icon(menu_icon("trash-2")).on_click(move |_, _, cx| {
                                let p = x.clone();
                                act(cx, |l| {
                                    l.close_document(&p);
                                    l.trash(&p);
                                });
                                state::reindex(cx);
                            }))
                    })
                    .into_any_element()
            }
            Row::Tag { label, depth, count, .. } => base(("tree-tag", i))
                .pl(px(space::M + *depth as f32 * space::M))
                .child(div().flex_1().min_w_0().truncate().child(format!("#{label}")))
                .child(caption(count.to_string(), t).flex_none().font_features(tabular()))
                .on_click(click(row, me))
                .into_any_element(),
            Row::BackToTags => base(("tree-back", i)).pl(px(space::S)).text_color(t.secondary).child(icon("arrow-left", iconsize::S, t.secondary)).child(tr("Alle Tags")).on_click(click(row, me)).into_any_element(),
            Row::Mark { title, query, .. } => base(("tree-mark", i))
                .pl(px(space::M))
                .child(icon(if query.is_some() { "search" } else { "bookmark" }, iconsize::S, t.secondary))
                .child(div().flex_1().min_w_0().truncate().child(title.clone()))
                .tooltip(tip(tr("Lesezeichen kommen aus Obsidian und lassen sich dort ändern.")))
                .on_click(click(row, me))
                .into_any_element(),
            Row::Task { path, note, line, text } => {
                let (tick, menu, line, label) = (path.clone(), path.clone(), *line, text.clone());
                base(("tree-task", i))
                    .pl(px(space::M))
                    .gap(px(space::S))
                    .child(div().id(("task-box", i)).flex_none().size(px(iconsize::S)).rounded(px(radius::SMALL)).border_1().border_color(t.secondary).cursor_pointer().aria_label(tr("Abhaken")).tooltip(tip(tr("Abhaken"))).on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        finish(&tick, line, &label, cx);
                    }))
                    .child(div().flex_1().min_w_0().truncate().child(text.clone()))
                    .child(caption(note.clone(), t).flex_none().max_w(px(90.)).truncate())
                    .on_click(click(row, me))
                    .context_menu(move |m, _, _| file_menu(m, menu.clone()))
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
        let (tag, active) = state::read(cx, |l| (l.notes_tag.clone(), l.active_document.clone()));
        if self.view == View::Search {
            self.search_vault(cx);
        }
        self.refresh(tag.clone(), cx);
        let rows = self.rows();
        let open_tasks = self.cache.as_ref().map_or(0, |c| c.open_tasks);
        let view = self.view;
        let order = self.order;
        let me = cx.entity();
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
            .child(self.view_button(View::Marks, "bookmark", tr("Lesezeichen"), 0, &t, cx))
            .child(self.view_button(View::Tasks, "circle-check", tr("Aufgaben"), open_tasks, &t, cx))
            .child(div().flex_1())
            .child(Button::new("explorer-hide").ghost().small().icon(menu_icon("panel-left-close")).tooltip(tr("Dateien ausblenden")).on_click(|_, _, cx| set_hidden(true, cx)))
            .child(Button::new("explorer-new").ghost().small().icon(menu_icon("square-pen")).tooltip(tr("Neue Notiz")).on_click(cx.listener(|s, _, window, cx| s.new_note(window, cx))))
            .child(Button::new("explorer-more").ghost().small().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu({
                let me = me.clone();
                move |menu, _, _| {
                    let (a, b, c, d) = (me.clone(), me.clone(), me.clone(), me.clone());
                    menu.item(PopupMenuItem::new(tr("Neuer Ordner")).icon(menu_icon("folder-plus")).on_click(move |_, _, cx| a.update(cx, |s, cx| s.new_folder(None, cx))))
                        .item(PopupMenuItem::new(tr("Tagesnotiz öffnen")).icon(menu_icon("calendar")).on_click(|_, _, cx| {
                            act(cx, |l| l.open_daily_note());
                            state::reindex(cx);
                        }))
                        .item(PopupMenuItem::new(tr("Graph zeigen")).icon(menu_icon("waypoints")).on_click(|_, _, cx| act(cx, |l| l.show_graph(None))))
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
        let body: AnyElement = if rows.is_empty() {
            let (name, title, detail) = match view {
                _ if filtered => ("search", tr("Nichts gefunden"), tr("Ändere den Filter.")),
                View::Search => ("search", tr("Inhalt durchsuchen"), tr("Tippe oben ein Wort oder Suchbefehle wie tag:#x.")),
                View::Files => ("book-open", tr("Noch keine Notizen"), tr("Mit ⌘N legst du die erste an.")),
                View::Tags => ("tag", tr("Noch keine Tags"), tr("Schreibe #tag in eine Notiz.")),
                View::Marks => ("bookmark", tr("Keine Lesezeichen"), tr("Lesezeichen kommen aus Obsidian und lassen sich dort ändern.")),
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
        let width = self.width;
        let handle = div().id("explorer-split").w(px(space::XS)).h_full().flex_none().cursor_col_resize().border_r_1().border_color(t.separator).on_drag(ExplorerSplit, |d, _, _, cx| cx.new(|_| d.clone())).on_drag_move(cx.listener(|s, e: &DragMoveEvent<ExplorerSplit>, _, cx| {
            s.width = f32::from(e.event.position.x - e.bounds.left()).clamp(180., 420.);
            cx.notify();
        }));
        div()
            .id("explorer")
            .track_focus(&self.focus)
            .key_context("Explorer")
            .h_full()
            .flex_none()
            .flex()
            .on_key_down(cx.listener(Self::on_key))
            .on_action(|_: &super::app::TogglePreview, _, cx| act(cx, |l| l.toggle_mode = true))
            .on_mouse_up(MouseButton::Left, cx.listener(|s, _, _, _| crate::core::prefs::update(|p| p.notes_column = Some(s.width))))
            .child(div().w(px(width)).h_full().flex().flex_col().bg(t.column).child(header).child(filter).child(div().flex_1().min_h_0().child(body)))
            .child(handle)
    }
}
