use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::{Soft, chevron};
use super::theme::{Theme, control, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::folders;
use crate::core::i18n::tr;
use crate::core::text::fold;

#[derive(Clone, PartialEq)]
struct Line {
    rel: String,
    name: String,
    depth: usize,
    open: bool,
    has_children: bool,
    /// The folder lies inside one that is already chosen.
    inside: bool,
}

/// Picks any folders at any depth from a tree; the choice is told with `Changed`.
pub struct FolderPicker {
    children: BTreeMap<String, Vec<String>>,
    all: Vec<String>,
    pub selected: BTreeSet<String>,
    open: BTreeSet<String>,
    filter: Entity<InputState>,
    cursor: Option<usize>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    _sub: Subscription,
}

/// The choice changed.
pub struct Changed;

impl EventEmitter<Changed> for FolderPicker {}

impl FolderPicker {
    pub fn new(root: &Path, selected: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let all = folders::all_folders(root);
        let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for folder in &all {
            let parent = folder.rsplit_once('/').map_or(String::new(), |(up, _)| up.to_string());
            children.entry(parent).or_default().push(folder.clone());
        }
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Ordner filtern")));
        let sub = cx.subscribe(&filter, |this, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                this.cursor = None;
                cx.notify();
            }
        });
        let open = folders::parents_of(&selected).into_iter().collect();
        Self { children, all, selected: selected.into_iter().collect(), open, filter, cursor: None, focus: cx.focus_handle(), scroll: UniformListScrollHandle::new(), _sub: sub }
    }

    fn walk(&self, parent: &str, depth: usize, out: &mut Vec<Line>) {
        for rel in self.children.get(parent).into_iter().flatten() {
            let open = self.open.contains(rel);
            out.push(self.line(rel, depth, open));
            if open {
                self.walk(rel, depth + 1, out);
            }
        }
    }

    fn line(&self, rel: &str, depth: usize, open: bool) -> Line {
        Line { rel: rel.to_string(), name: rel.rsplit('/').next().unwrap_or(rel).to_string(), depth, open, has_children: self.children.contains_key(rel), inside: self.selected.iter().any(|s| rel.starts_with(&format!("{s}/"))) }
    }

    fn lines(&self, cx: &App) -> Vec<Line> {
        let query = fold(self.filter.read(cx).value().trim());
        if query.is_empty() {
            let mut out = Vec::new();
            self.walk("", 0, &mut out);
            return out;
        }
        self.all.iter().filter(|f| fold(f).contains(&query)).take(500).map(|f| {
            let mut line = self.line(f, 0, false);
            line.name = f.replace('/', " › ");
            line.has_children = false;
            line
        }).collect()
    }

    fn toggle(&mut self, rel: &str, cx: &mut Context<Self>) {
        if !self.selected.remove(rel) {
            self.selected.insert(rel.to_string());
        }
        cx.emit(Changed);
        cx.notify();
    }

    fn fold(&mut self, rel: &str, cx: &mut Context<Self>) {
        if !self.open.remove(rel) {
            self.open.insert(rel.to_string());
        }
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let lines = self.lines(cx);
        let current = self.cursor.and_then(|i| lines.get(i)).cloned();
        match e.keystroke.key.as_str() {
            "down" => self.cursor = Some(self.cursor.map_or(0, |i| (i + 1).min(lines.len().saturating_sub(1)))),
            "up" => match self.cursor {
                Some(0) | None => {
                    self.cursor = None;
                    self.filter.update(cx, |f, cx| f.focus(window, cx));
                }
                Some(i) => self.cursor = Some(i - 1),
            },
            "right" => {
                if let Some(l) = current.filter(|l| l.has_children && !l.open) {
                    self.open.insert(l.rel);
                }
            }
            "left" => {
                if let Some(l) = current {
                    if l.open {
                        self.open.remove(&l.rel);
                    } else if let Some((up, _)) = l.rel.rsplit_once('/') {
                        self.cursor = lines.iter().position(|x| x.rel == up);
                    }
                }
            }
            "space" | "enter" => {
                if let Some(l) = current {
                    self.toggle(&l.rel, cx);
                }
            }
            _ => return,
        }
        if let Some(i) = self.cursor {
            self.scroll.scroll_to_item(i, ScrollStrategy::Center);
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for FolderPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let lines = Rc::new(self.lines(cx));
        let me = cx.entity();
        let count = lines.len();
        let cursor = self.cursor;
        let body: AnyElement = if count == 0 {
            unavailable("folder", tr("Keine Ordner gefunden"), tr("Lege den Ordner an, er taucht dann hier auf."), &t).into_any_element()
        } else {
            let list = lines.clone();
            uniform_list("folder-tree", count, move |range, _, cx| {
                let t = Theme::get(cx);
                range
                    .map(|i| {
                        let line = &list[i];
                        let selected = me.read(cx).selected.contains(&line.rel);
                        let (a, b, rel_a, rel_b) = (me.clone(), me.clone(), line.rel.clone(), line.rel.clone());
                        let inside = line.inside;
                        div()
                            .id(("folder-row", i))
                            .h(px(control::M))
                            .w_full()
                            .pl(px(space::S + line.depth as f32 * space::L))
                            .pr(px(space::S))
                            .flex()
                            .items_center()
                            .gap(px(space::XS))
                            .cursor_pointer()
                            .text_size(px(font::CALLOUT))
                            .when(cursor == Some(i), |d| d.bg(t.ink.opacity(0.08)))
                            .when(cursor != Some(i), |d| d.soft(t.ink.opacity(0.05), 0.))
                            .child(if line.has_children {
                                div()
                                    .id(("folder-fold", i))
                                    .flex_none()
                                    .size(px(iconsize::M))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(chevron(("folder-chevron", i), line.open, t.secondary))
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        a.update(cx, |p, cx| p.fold(&rel_a, cx));
                                    })
                                    .into_any_element()
                            } else {
                                div().flex_none().size(px(iconsize::M)).into_any_element()
                            })
                            .child(div().flex_none().size(px(iconsize::M)).rounded(px(radius::SMALL)).border_1().border_color(if selected { t.accent } else { t.secondary.opacity(0.5) }).when(selected, |d| d.bg(t.accent)).flex().items_center().justify_center().when(selected, |d| d.child(icon("check", iconsize::XS, t.on_accent))))
                            .child(icon("folder", iconsize::S, t.secondary))
                            .child(div().flex_1().min_w_0().truncate().when(inside, |d| d.text_color(t.secondary)).child(line.name.clone()))
                            .on_click(move |_, _, cx| b.update(cx, |p, cx| p.toggle(&rel_b, cx)))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            })
            .track_scroll(&self.scroll)
            .size_full()
            .into_any_element()
        };
        div()
            .id("folder-picker")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(Self::on_key))
            .capture_key_down(cx.listener(|p, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "down" && !p.focus.is_focused(window) {
                    window.focus(&p.focus, cx);
                    p.cursor = Some(0);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(div().flex_none().p(px(space::S)).child(Input::new(&self.filter).small().prefix(icon("search", iconsize::XS, t.secondary))))
            .child(div().flex_1().min_h_0().child(body))
    }
}
