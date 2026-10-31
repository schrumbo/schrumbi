use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::{Soft, chevron};
use super::state::{self, act};
use super::theme::{Theme, control, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::library::NewNoteRequest;
use crate::core::vault::index;

/// Every note of the vault: tree on the left, list on the right.
pub struct NotesPage {
    folder: Option<String>,
    open_folders: BTreeSet<String>,
    query: Entity<InputState>,
    selected: usize,
    /// The same note stays selected when new ones arrive from Obsidian.
    selected_path: Option<PathBuf>,
    tree_width: f32,
    show_all_tags: bool,
    tree_open: bool,
    cache: Option<Cache>,
    focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    _subs: Vec<Subscription>,
}

struct Cache {
    key: (u64, String, Option<String>, Option<String>),
    lines: Rc<Vec<Line>>,
    children: Rc<BTreeMap<String, BTreeSet<String>>>,
    tags: Rc<Vec<(String, usize)>>,
}

struct Line {
    path: PathBuf,
    name: String,
    folder: String,
    tags: Vec<String>,
    modified: f64,
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

#[derive(Clone)]
struct NotesSplit;

impl Render for NotesSplit {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl NotesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Notizen filtern")));
        let lib = state::lib(cx);
        let subs = vec![
            cx.subscribe(&query, |this, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.selected = 0;
                    this.selected_path = None;
                    cx.notify();
                }
            }),
            cx.observe(&lib, |_, _, cx| cx.notify()),
        ];
        Self { folder: None, open_folders: BTreeSet::new(), query, selected: 0, selected_path: None, tree_width: crate::core::prefs::get().notes_column.unwrap_or(240.), show_all_tags: false, tree_open: false, cache: None, focus: cx.focus_handle(), list_scroll: UniformListScrollHandle::new(), _subs: subs }
    }

    fn lines(&self, tag: Option<&str>, cx: &App) -> Vec<Line> {
        let vault = index::current();
        let filter = self.query.read(cx).value().to_lowercase();
        let tagged = tag.map(|t| vault.notes_with_tag(t));
        let mut out: Vec<Line> = vault
            .notes
            .iter()
            .filter(|(p, _)| crate::core::markdown::is_markdown(p))
            .filter_map(|(path, facts)| {
                let rel = path.strip_prefix(&vault.root).ok()?.to_string_lossy().replace('\\', "/");
                if vault.config.is_ignored(&rel) {
                    return None;
                }
                let folder = rel.rsplit_once('/').map_or(String::new(), |(f, _)| f.to_string());
                if self.folder.as_ref().is_some_and(|f| folder != *f && !folder.starts_with(&format!("{f}/"))) {
                    return None;
                }
                if tagged.as_ref().is_some_and(|t| !t.contains(path)) {
                    return None;
                }
                let name = path.file_stem()?.to_string_lossy().to_string();
                if !filter.is_empty() && !name.to_lowercase().contains(&filter) && !facts.aliases.iter().any(|a| a.to_lowercase().contains(&filter)) {
                    return None;
                }
                Some(Line { path: path.clone(), name, folder, tags: facts.tags.clone(), modified: facts.modified })
            })
            .collect();
        out.sort_by(|a, b| b.modified.total_cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// Folders as the tree shows them, top to bottom.
    fn shown_folders(&self) -> Vec<String> {
        let vault = index::current();
        let mut all: BTreeSet<String> = BTreeSet::new();
        for path in vault.notes.keys() {
            let Ok(rel) = path.strip_prefix(&vault.root) else { continue };
            let mut parent = rel.parent();
            while let Some(p) = parent.filter(|p| !p.as_os_str().is_empty()) {
                all.insert(p.to_string_lossy().replace('\\', "/"));
                parent = p.parent();
            }
        }
        all.into_iter().filter(|f| f.rsplit_once('/').is_none_or(|(up, _)| up.split('/').scan(String::new(), |acc, part| {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            Some(acc.clone())
        }).all(|a| self.open_folders.contains(&a)))).collect()
    }

    /// ⌥↑↓ walk the folders, → opens one and ← closes it or goes to its parent.
    fn tree_key(&mut self, key: &str) {
        let shown = self.shown_folders();
        let at = self.folder.as_ref().and_then(|f| shown.iter().position(|s| s == f));
        match key {
            "down" => self.folder = shown.get(at.map_or(0, |i| i + 1)).cloned().or(self.folder.clone()),
            "up" => self.folder = at.filter(|i| *i > 0).and_then(|i| shown.get(i - 1).cloned()),
            "right" => {
                if let Some(f) = self.folder.clone() {
                    self.open_folders.insert(f);
                }
            }
            "left" => {
                if let Some(f) = self.folder.clone()
                    && !self.open_folders.remove(&f)
                {
                    self.folder = f.rsplit_once('/').map(|(up, _)| up.to_string());
                }
            }
            _ => {}
        }
        self.selected = 0;
    }

    /// What the page shows, worked out once per vault state and filter: scrolling draws often.
    fn refresh_cache(&mut self, tag: Option<String>, cx: &App) {
        let vault = index::current();
        let key = (vault.generation, self.query.read(cx).value().to_string(), self.folder.clone(), tag.clone());
        if self.cache.as_ref().is_some_and(|c| c.key == key) {
            return;
        }
        let same_vault = self.cache.as_ref().is_some_and(|c| c.key.0 == vault.generation);
        let lines = Rc::new(self.lines(tag.as_deref(), cx));
        let (children, tags) = match self.cache.take().filter(|_| same_vault) {
            Some(old) => (old.children, old.tags),
            None => {
                let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
                for path in vault.notes.keys() {
                    let Ok(rel) = path.strip_prefix(&vault.root) else { continue };
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    let mut parent = String::new();
                    let parts: Vec<&str> = rel.split('/').collect();
                    for part in &parts[..parts.len().saturating_sub(1)] {
                        let full = if parent.is_empty() { part.to_string() } else { format!("{parent}/{part}") };
                        children.entry(parent.clone()).or_default().insert(full.clone());
                        parent = full;
                    }
                }
                (Rc::new(children), Rc::new(vault.tag_tree()))
            }
        };
        self.cache = Some(Cache { key, lines, children, tags });
    }

    fn tree(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let vault = index::current();
        let children = self.cache.as_ref().map(|c| c.children.clone()).unwrap_or_default();
        let hover = t.ink.opacity(0.05);
        let mut list = div().py(px(space::S)).flex().flex_col();
        let all_selected = self.folder.is_none();
        list = list.child(
            div()
                .id("notes-all")
                .h(px(control::M))
                .flex_none()
                .px(px(space::M))
                .flex()
                .items_center()
                .gap(px(space::S))
                .cursor_pointer()
                .key_nav()
                .when(all_selected, |d| d.bg(t.ink.opacity(0.08)))
                .when(!all_selected, |d| d.soft(hover, 0.))
                .child(icon("book-open", iconsize::S, t.secondary))
                .child(tr("Alle Notizen"))
                .on_click(cx.listener(|s, _, _, cx| {
                    s.folder = None;
                    s.selected = 0;
                    cx.notify();
                })),
        );
        let mut stack: Vec<(String, usize)> = children.get("").map(|c| c.iter().rev().map(|f| (f.clone(), 0)).collect()).unwrap_or_default();
        let mut row = 0usize;
        while let Some((folder, depth)) = stack.pop() {
            let open = self.open_folders.contains(&folder);
            let has_children = children.contains_key(&folder);
            if open
                && let Some(c) = children.get(&folder)
            {
                stack.extend(c.iter().rev().map(|f| (f.clone(), depth + 1)));
            }
            let selected = self.folder.as_ref() == Some(&folder);
            let name = folder.rsplit('/').next().unwrap_or(&folder).to_string();
            let (click, toggle) = (folder.clone(), folder.clone());
            row += 1;
            list = list.child(
                div()
                    .id(("notes-folder", row))
                    .h(px(control::M))
                    .flex_none()
                    .pl(px(space::M + depth as f32 * space::M))
                    .pr(px(space::S))
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .cursor_pointer()
                    .key_nav()
                    .when(selected, |d| d.bg(t.ink.opacity(0.08)))
                    .when(!selected, |d| d.soft(hover, 0.))
                    .child(div().id(("notes-chevron", row)).size(px(iconsize::S)).flex_none().flex().items_center().justify_center().when(has_children, |d| d.child(chevron(("notes-chevron-icon", row), open, t.secondary))).on_click(cx.listener(move |s, _, _, cx| {
                        cx.stop_propagation();
                        if !s.open_folders.remove(&toggle) {
                            s.open_folders.insert(toggle.clone());
                        }
                        cx.notify();
                    })))
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    .tooltip(tip(folder.clone()))
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.folder = Some(click.clone());
                        s.selected = 0;
                        if !s.open_folders.contains(&click) {
                            s.open_folders.insert(click.clone());
                        }
                        cx.notify();
                    })),
            );
        }
        let marks = vault.bookmarks.clone();
        if !marks.is_empty() {
            list = list.child(div().px(px(space::M)).pt(px(space::L)).pb(px(space::XS)).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(tr("Lesezeichen")));
            for (i, mark) in marks.into_iter().enumerate() {
                let title = mark.title.clone();
                list = list.child(
                    div()
                        .id(("notes-bookmark", i))
                        .h(px(control::M))
                        .flex_none()
                        .px(px(space::M))
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .cursor_pointer()
                        .key_nav()
                        .soft(hover, 0.)
                        .child(icon(if mark.query.is_some() { "search" } else { "bookmark" }, iconsize::S, t.secondary))
                        .child(div().flex_1().min_w_0().truncate().child(title))
                        .tooltip(tip(tr("Lesezeichen kommen aus Obsidian und lassen sich dort ändern.")))
                        .on_click(move |_, _, cx| match (&mark.query, &mark.path) {
                            (Some(q), _) => {
                                let q = q.clone();
                                act(cx, |l| {
                                    l.search_query = q;
                                    l.set_route(crate::core::library::Route::Search);
                                });
                            }
                            (_, Some(p)) => {
                                let destination = format!("{p}{}", mark.subpath.clone().unwrap_or_default());
                                act(cx, |l| {
                                    l.open_link(&destination, None);
                                });
                            }
                            _ => {}
                        }),
                );
            }
        }
        div().id("notes-tree").size_full().overflow_y_scrollbar().child(list).into_any_element()
    }
}

impl Render for NotesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let vault = index::current();
        let tag = state::read(cx, |l| l.notes_tag.clone());
        self.refresh_cache(tag.clone(), cx);
        let lines = self.cache.as_ref().map(|c| c.lines.clone()).unwrap_or_default();
        if let Some(at) = self.selected_path.as_ref().and_then(|p| lines.iter().position(|l| l.path == *p)) {
            self.selected = at;
        }
        self.selected = self.selected.min(lines.len().saturating_sub(1));
        let narrow = window.bounds().size.width < px(1100.);
        let toggle_tree = narrow.then(|| {
            Button::new("notes-tree-toggle").small().ghost().icon(menu_icon("folder")).tooltip(tr("Ordner und Lesezeichen")).on_click(cx.listener(|s, _, _, cx| {
                s.tree_open = !s.tree_open;
                cx.notify();
            }))
        });
        let show_tree = !narrow || self.tree_open;
        let tree = self.tree(&t, cx);
        let tree_width = self.tree_width;
        let handle = div().id("notes-split").w(px(space::XS)).h_full().flex_none().cursor_col_resize().on_drag(NotesSplit, |d, _, _, cx| cx.new(|_| d.clone())).on_drag_move(cx.listener(|s, e: &DragMoveEvent<NotesSplit>, _, cx| {
            let left = e.bounds.left();
            s.tree_width = f32::from(e.event.position.x - left).clamp(160., 420.);
            cx.notify();
        }));
        let tags = self.cache.as_ref().map(|c| c.tags.clone()).unwrap_or_default();
        let shown_tags = if self.show_all_tags { tags.len() } else { tags.len().min(12) };
        let more = tags.len() - shown_tags;
        let current_tag = tag.clone();
        let filter_row = div()
            .flex_none()
            .p(px(space::M))
            .flex()
            .flex_col()
            .gap(px(space::S))
            .border_b_1()
            .border_color(t.separator)
            .child(div().flex().items_center().gap(px(space::S)).children(toggle_tree).child(div().flex_1().child(Input::new(&self.query).small())).child(Button::new("notes-graph").small().ghost().icon(menu_icon("waypoints")).tooltip(tr("Graph zeigen")).on_click(|_, _, cx| act(cx, |l| l.show_graph(None)))).child(Button::new("notes-daily").small().icon(menu_icon("calendar")).label(tr("Heute")).tooltip(tr("Tagesnotiz öffnen")).on_click(|_, _, cx| act(cx, |l| l.open_daily_note()))).child(Button::new("notes-new").small().primary().icon(menu_icon("plus")).label(tr("Neue Notiz")).on_click(cx.listener(|s, _, _, cx| {
                let folder = s.folder.clone().map(|f| index::current().root.join(f));
                act(cx, |l| l.new_note = Some(NewNoteRequest { module_id: None, folder }));
            }))))
            .when(!tags.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(space::XS))
                        .children(tags.iter().take(shown_tags).enumerate().map(|(i, (name, count))| {
                            let selected = current_tag.as_deref() == Some(name.as_str());
                            let set = name.clone();
                            div()
                                .id(("notes-tag", i))
                                .px(px(space::S))
                                .py(px(space::XXS))
                                .rounded_full()
                                .flex()
                                .gap(px(space::XS))
                                .text_size(px(font::CAPTION))
                                .cursor_pointer()
                                .key_nav()
                                .bg(if selected { t.accent } else { t.ink.opacity(0.08) })
                                .text_color(if selected { white() } else { t.secondary })
                                .child(format!("#{name}"))
                                .child(div().font_features(tabular()).child(count.to_string()))
                                .on_click(move |_, _, cx| {
                                    let name = set.clone();
                                    act(cx, |l| l.notes_tag = if l.notes_tag.as_deref() == Some(name.as_str()) { None } else { Some(name.clone()) });
                                })
                        }))
                        .when(more > 0, |d| {
                            d.child(link_button("notes-tags-more", format!("+{more}"), &t).text_size(px(font::CAPTION)).on_click(cx.listener(|s, _, _, cx| {
                                s.show_all_tags = true;
                                cx.notify();
                            })))
                        }),
                )
            });
        let count = lines.len();
        let selected = self.selected;
        let now = chrono::Local::now().timestamp() as f64;
        let rows = lines;
        let body: AnyElement = if rows.is_empty() {
            if tag.is_some() || !self.query.read(cx).value().is_empty() {
                unavailable("search", tr("Keine Notiz passt"), tr("Ändere den Filter oder wähle ein anderes Tag."), &t).into_any_element()
            } else {
                unavailable("book-open", tr("Noch keine Notizen"), tr("Mit „Neue Notiz“ legst du die erste an."), &t).into_any_element()
            }
        } else {
            let keyed = rows.clone();
            uniform_list("notes-list", count, move |range, _, cx| {
                let t = Theme::get(cx);
                let hover = t.ink.opacity(0.05);
                range
                    .map(|i| {
                        let line = &keyed[i];
                        let (path, name, folder, tags, ago) = (line.path.clone(), line.name.clone(), line.folder.clone(), line.tags.clone(), changed_ago(now - line.modified));
                        let (open, menu) = (path.clone(), path.clone());
                        div()
                            .id(("notes-row", i))
                            .w_full()
                            .h(px(control::BAR - space::S))
                            .px(px(space::L))
                            .flex()
                            .items_center()
                            .gap(px(space::M))
                            .cursor_pointer()
                            .key_nav()
                            .when(i == selected, |d| d.bg(t.ink.opacity(0.08)))
                            .when(i != selected, |d| d.soft(hover, 0.))
                            .child(file_icon(&path, iconsize::M))
                            .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().child(name)).when(!folder.is_empty(), |d| d.child(caption(folder, &t).truncate())))
                            .children(tags.first().map(|tag| div().flex_none().px(px(space::S)).py(px(space::XXS)).rounded(px(radius::FULL)).bg(t.ink.opacity(0.08)).text_size(px(font::CAPTION)).text_color(t.secondary).child(format!("#{tag}"))))
                            .child(caption(ago, &t).flex_none().font_features(tabular()))
                            .on_click(move |_, _, cx| act(cx, |l| l.open_file(&open)))
                            .context_menu(move |m, _, _| file_menu(m, menu.clone()))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            })
            .track_scroll(&self.list_scroll)
            .size_full()
            .into_any_element()
        };
        let keys = rows.clone();
        div()
            .id("notes-page")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .on_key_down(cx.listener(move |s, e: &KeyDownEvent, window, cx| {
                if window.context_stack().iter().any(|c| c.contains("Input")) {
                    return;
                }
                let k = &e.keystroke;
                if k.key == "/" {
                    s.query.update(cx, |q, cx| q.focus(window, cx));
                    cx.stop_propagation();
                    return;
                }
                if k.key == "n" && k.modifiers.platform {
                    let folder = s.folder.clone().map(|f| index::current().root.join(f));
                    act(cx, |l| l.new_note = Some(NewNoteRequest { module_id: None, folder }));
                    cx.stop_propagation();
                    return;
                }
                if matches!(k.key.as_str(), "left" | "right") || (k.modifiers.alt && matches!(k.key.as_str(), "up" | "down")) {
                    s.tree_key(k.key.as_str());
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                match e.keystroke.key.as_str() {
                    "down" => {
                        s.selected = (s.selected + 1).min(keys.len().saturating_sub(1));
                        s.selected_path = keys.get(s.selected).map(|l| l.path.clone());
                    }
                    "up" => {
                        s.selected = s.selected.saturating_sub(1);
                        s.selected_path = keys.get(s.selected).map(|l| l.path.clone());
                    }
                    "enter" => {
                        if let Some(line) = keys.get(s.selected) {
                            let p = line.path.clone();
                            act(cx, |l| l.open_file(&p));
                        }
                        return;
                    }
                    _ => return,
                }
                s.list_scroll.scroll_to_item(s.selected, ScrollStrategy::Center);
                cx.notify();
            }))
            .when(show_tree, |d| d.child(div().w(px(tree_width)).flex_none().h_full().border_r_1().border_color(t.separator).child(tree)).child(handle).on_mouse_up(MouseButton::Left, cx.listener(|s, _, _, _| crate::core::prefs::update(|p| p.notes_column = Some(s.tree_width)))))
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().child(filter_row).child(div().flex_1().min_h_0().child(body)).child(div().flex_none().px(px(space::L)).py(px(space::XS)).border_t_1().border_color(t.separator).child(caption(if count == 1 { tr("1 Notiz") } else { trf("%@ Notizen", &[&count.to_string()]) }, &t))))
    }
}
