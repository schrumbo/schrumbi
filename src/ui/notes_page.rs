use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

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
use crate::core::vault::{config, index};

/// Every note of the vault: tree on the left, list on the right.
pub struct NotesPage {
    folder: Option<String>,
    open_folders: BTreeSet<String>,
    query: Entity<InputState>,
    selected: usize,
    tree_width: f32,
    show_all_tags: bool,
    focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    _subs: Vec<Subscription>,
}

struct Line {
    path: PathBuf,
    name: String,
    folder: String,
    tags: Vec<String>,
    modified: f64,
}

impl NotesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Notizen filtern")));
        let subs = vec![cx.subscribe(&query, |this, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                this.selected = 0;
                cx.notify();
            }
        })];
        Self { folder: None, open_folders: BTreeSet::new(), query, selected: 0, tree_width: 240., show_all_tags: false, focus: cx.focus_handle(), list_scroll: UniformListScrollHandle::new(), _subs: subs }
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

    fn tree(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let vault = index::current();
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
        let marks = config::bookmarks(&vault.root);
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
        let lines = self.lines(tag.as_deref(), cx);
        self.selected = self.selected.min(lines.len().saturating_sub(1));
        let narrow = window.bounds().size.width < px(1100.);
        let tree = self.tree(&t, cx);
        let tree_width = self.tree_width;
        let tags = vault.tag_tree();
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
            .child(div().flex().items_center().gap(px(space::S)).child(div().flex_1().child(Input::new(&self.query).small())).child(Button::new("notes-daily").small().icon(menu_icon("calendar")).label(tr("Heute")).tooltip(tr("Tagesnotiz öffnen")).on_click(|_, _, cx| act(cx, |l| l.open_daily_note()))).child(Button::new("notes-new").small().primary().icon(menu_icon("plus")).label(tr("Neue Notiz")).on_click(cx.listener(|s, _, _, cx| {
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
        let rows: Vec<(PathBuf, String, String, Vec<String>)> = lines.into_iter().map(|l| (l.path, l.name, l.folder, l.tags)).collect();
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
                        let (path, name, folder, tags) = keyed[i].clone();
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
                            .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().child(name)).child(caption(folder, &t).truncate()))
                            .children(tags.first().map(|tag| div().flex_none().px(px(space::S)).py(px(space::XXS)).rounded(px(radius::FULL)).bg(t.ink.opacity(0.08)).text_size(px(font::CAPTION)).text_color(t.secondary).child(format!("#{tag}"))))
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
        let keys = rows.iter().map(|r| r.0.clone()).collect::<Vec<_>>();
        div()
            .id("notes-page")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .on_key_down(cx.listener(move |s, e: &KeyDownEvent, window, cx| {
                if window.context_stack().iter().any(|c| c.contains("Input")) {
                    return;
                }
                match e.keystroke.key.as_str() {
                    "down" => s.selected = (s.selected + 1).min(keys.len().saturating_sub(1)),
                    "up" => s.selected = s.selected.saturating_sub(1),
                    "enter" => {
                        if let Some(p) = keys.get(s.selected) {
                            let p = p.clone();
                            act(cx, |l| l.open_file(&p));
                        }
                        return;
                    }
                    _ => return,
                }
                s.list_scroll.scroll_to_item(s.selected, ScrollStrategy::Center);
                cx.notify();
            }))
            .when(!narrow, |d| d.child(div().w(px(tree_width)).flex_none().h_full().border_r_1().border_color(t.separator).child(tree)))
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().child(filter_row).child(div().flex_1().min_h_0().child(body)).child(div().flex_none().px(px(space::L)).py(px(space::XS)).border_t_1().border_color(t.separator).child(caption(if count == 1 { tr("1 Notiz") } else { trf("%@ Notizen", &[&count.to_string()]) }, &t))))
    }
}
