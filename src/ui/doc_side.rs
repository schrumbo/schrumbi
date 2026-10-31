use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::history::VersionList;
use super::motion::{Soft, chevron, reveal};
use super::state::{self, act};
use super::theme::{Theme, control, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::tr;
use crate::core::markdown;
use crate::core::vault::frontmatter::{self, Property, PropertyKind};
use crate::core::vault::index::{self, Mention};
use crate::core::vault::links;

#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Links,
    Properties,
    Versions,
}

/// The details column beside a document.
pub struct DocSide {
    side: Side,
    versions: Entity<VersionList>,
    properties: Entity<PropertyEditor>,
    closed: std::collections::BTreeSet<&'static str>,
    unlinked: Option<(PathBuf, u64, Vec<Mention>)>,
    focus_versions: bool,
}

impl DocSide {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self { side: Side::Links, versions: cx.new(VersionList::new), properties: cx.new(|cx| PropertyEditor::new(window, cx)), closed: Default::default(), unlinked: None, focus_versions: false }
    }

    pub fn show(&mut self, side: Side, cx: &mut Context<Self>) {
        self.side = side;
        self.focus_versions = side == Side::Versions;
        cx.notify();
    }

    fn tab(&self, id: &'static str, label: String, side: Side, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let hover = t.ink.opacity(0.05);
        let selected = self.side == side;
        div()
            .id(id)
            .px(px(space::S))
            .py(px(space::XS))
            .rounded(px(radius::INNER))
            .text_size(px(font::CALLOUT))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .key_nav()
            .when(selected, |d| d.bg(t.ink.opacity(0.08)))
            .when(!selected, |d| d.soft(hover, radius::INNER).text_color(t.secondary))
            .child(label)
            .on_click(cx.listener(move |s, _, _, cx| s.show(side, cx)))
    }

    fn section(&self, key: &'static str, title: String, count: usize, body: AnyElement, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let open = !self.closed.contains(key);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(key)
                    .h(px(control::M))
                    .px(px(space::M))
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .cursor_pointer()
                    .key_nav()
                    .text_size(px(font::CAPTION))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.secondary)
                    .child(chevron((key, 0usize), open, t.secondary))
                    .child(title)
                    .child(div().font_features(tabular()).child(count.to_string()))
                    .on_click(cx.listener(move |s, _, _, cx| {
                        if !s.closed.remove(key) {
                            s.closed.insert(key);
                        }
                        cx.notify();
                    })),
            )
            .child(reveal((key, 1usize), open, body))
    }

    fn mention_rows(key: &'static str, list: &[Mention], name: &str, t: &Theme) -> AnyElement {
        let hover = t.ink.opacity(0.05);
        div()
            .flex()
            .flex_col()
            .children(list.iter().take(200).enumerate().map(|(i, m)| {
                let (open, line) = (m.source.clone(), m.line);
                let file = m.source.file_stem().unwrap_or_default().to_string_lossy().to_string();
                let text = m.text.clone();
                let start = text.to_lowercase().find(&name.to_lowercase());
                let highlighted: AnyElement = match start {
                    Some(at) if !name.is_empty() && text.is_char_boundary(at) && text.is_char_boundary(at + name.len()) => StyledText::new(text.clone()).with_highlights(vec![(at..at + name.len(), HighlightStyle { color: Some(t.accent_text), ..Default::default() })]).into_any_element(),
                    _ => text.clone().into_any_element(),
                };
                div()
                    .id((key, i))
                    .px(px(space::L))
                    .py(px(space::XS))
                    .flex()
                    .flex_col()
                    .cursor_pointer()
                    .key_nav()
                    .soft(hover, 0.)
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).text_size(px(font::CALLOUT)).child(file))
                    .child(div().text_size(px(font::CAPTION)).text_color(t.secondary).line_clamp(2).child(highlighted))
                    .on_click(move |_, _, cx| open_at(&open, line, cx))
            }))
            .into_any_element()
    }

    fn links_view(&mut self, path: &Path, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let vault = index::current();
        let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let backlinks = vault.backlinks.get(path).cloned().unwrap_or_default();
        let facts = vault.notes.get(path).cloned().unwrap_or_default();
        let hover = t.ink.opacity(0.05);
        let from = path.to_path_buf();
        let mut existing = Vec::new();
        let mut missing = Vec::new();
        for (link, line, _) in &facts.links {
            let shown = if link.target.path.is_empty() { link.target.subpath() } else { format!("{}{}", link.target.path, link.target.subpath()) };
            match vault.resolver.resolve(&link.target, Some(path)) {
                Some(target) => {
                    let label = format!("{}{}", target.file_stem().unwrap_or_default().to_string_lossy(), link.target.subpath());
                    existing.push((label, target, link.target.subpath()));
                }
                None => missing.push((shown, *line)),
            }
        }
        existing.sort_by(|a, b| a.0.cmp(&b.0));
        existing.dedup_by(|a, b| a.0 == b.0);
        missing.sort();
        missing.dedup_by(|a, b| a.0 == b.0);
        let unlinked = match &self.unlinked {
            Some((p, generation, list)) if p == path && *generation == vault.generation => list.clone(),
            _ => {
                let (worker, target, generation) = (vault.clone(), path.to_path_buf(), vault.generation);
                self.unlinked = Some((path.to_path_buf(), generation, Vec::new()));
                let task = cx.background_spawn(async move { worker.unlinked_mentions(&target, 50) });
                let key = path.to_path_buf();
                cx.spawn(async move |this, cx| {
                    let found = task.await;
                    let _ = this.update(cx, |s, cx| {
                        if s.unlinked.as_ref().is_some_and(|(p, g, _)| *p == key && *g == generation) {
                            s.unlinked = Some((key, generation, found));
                            cx.notify();
                        }
                    });
                })
                .detach();
                Vec::new()
            }
        };
        let mut list = div().flex().flex_col().py(px(space::S));
        let around = path.to_path_buf();
        list = list.child(div().px(px(space::M)).pb(px(space::S)).child(Button::new("side-graph").ghost().small().icon(menu_icon("waypoints")).label(tr("Graph um diese Notiz")).on_click(move |_, _, cx| {
            let p = around.clone();
            act(cx, |l| l.show_graph(Some(p)));
        })));
        let nothing = backlinks.is_empty() && existing.is_empty() && missing.is_empty() && unlinked.is_empty() && facts.headings.is_empty();
        if nothing {
            return unavailable("link", tr("Noch keine Verknüpfungen"), tr("Tippe [[ im Editor, um eine Notiz zu verlinken."), t).into_any_element();
        }
        if !backlinks.is_empty() {
            let body = Self::mention_rows("backlink", &backlinks, &name, t);
            list = list.child(self.section("side-backlinks", tr("Rückverweise"), backlinks.len(), body, t, cx));
        }
        if !unlinked.is_empty() {
            let body = Self::mention_rows("unlinked", &unlinked, &name, t);
            list = list.child(self.section("side-unlinked", tr("Erwähnungen ohne Link"), unlinked.len(), body, t, cx));
        }
        if !existing.is_empty() {
            let count = existing.len();
            let body = div()
                .flex()
                .flex_col()
                .children(existing.into_iter().enumerate().map(|(i, (shown, target, sub))| {
                    let destination = format!("{}{sub}", vault.link_path(Some(&from), &target));
                    let source = from.clone();
                    let icon_target = target.clone();
                    div().id(("outgoing", i)).h(px(control::M)).px(px(space::L)).flex().items_center().gap(px(space::S)).cursor_pointer().key_nav().soft(hover, 0.).child(file_icon(&icon_target, iconsize::S)).child(div().flex_1().min_w_0().truncate().text_size(px(font::CALLOUT)).child(shown)).on_click(move |_, _, cx| {
                        let (d, s) = (destination.clone(), source.clone());
                        act(cx, |l| {
                            l.open_link(&d, Some(&s));
                        });
                    })
                }))
                .into_any_element();
            list = list.child(self.section("side-outgoing", tr("Ausgehend"), count, body, t, cx));
        }
        if !missing.is_empty() {
            let count = missing.len();
            let body = div()
                .flex()
                .flex_col()
                .children(missing.into_iter().enumerate().map(|(i, (shown, _))| {
                    let (destination, source) = (shown.clone(), from.clone());
                    div().px(px(space::L)).h(px(control::M)).flex().items_center().gap(px(space::S)).child(div().flex_1().min_w_0().truncate().text_size(px(font::CALLOUT)).text_color(t.tertiary).child(shown)).child(Button::new(("create-missing", i)).ghost().small().label(tr("Anlegen")).on_click(move |_, _, cx| {
                        let (d, s) = (destination.clone(), source.clone());
                        act(cx, |l| l.create_link_target(&d, Some(&s)));
                    }))
                }))
                .into_any_element();
            list = list.child(self.section("side-missing", tr("Fehlende Ziele"), count, body, t, cx));
        }
        if !facts.headings.is_empty() {
            let count = facts.headings.len();
            let source = from.clone();
            let body = div()
                .flex()
                .flex_col()
                .children(facts.headings.iter().enumerate().map(|(i, (level, h, line))| {
                    let (open, line) = (source.clone(), *line);
                    div().id(("outline", i)).h(px(control::S)).pl(px(space::L + (*level as f32 - 1.) * space::M)).pr(px(space::M)).flex().items_center().cursor_pointer().key_nav().soft(hover, 0.).child(div().truncate().text_size(px(font::CALLOUT)).child(h.clone())).on_click(move |_, _, cx| open_at(&open, line, cx))
                }))
                .into_any_element();
            list = list.child(self.section("side-outline", tr("Gliederung"), count, body, t, cx));
        }
        let footnotes = facts.footnotes.clone();
        if !footnotes.is_empty() {
            let count = footnotes.len();
            let source = from.clone();
            let body = div()
                .flex()
                .flex_col()
                .children(footnotes.into_iter().enumerate().map(|(i, (label, text, line))| {
                    let open = source.clone();
                    div().id(("footnote", i)).px(px(space::L)).py(px(space::XS)).flex().gap(px(space::S)).cursor_pointer().key_nav().soft(hover, 0.).child(caption(label, t).flex_none()).child(div().flex_1().min_w_0().text_size(px(font::CALLOUT)).line_clamp(2).child(text)).on_click(move |_, _, cx| open_at(&open, line, cx))
                }))
                .into_any_element();
            list = list.child(self.section("side-footnotes", tr("Fußnoten"), count, body, t, cx));
        }
        div().id("side-links").size_full().overflow_y_scrollbar().child(list).into_any_element()
    }
}

/// Opens a note at a line.
pub fn open_at(path: &Path, line: usize, cx: &mut App) {
    let (path, text) = (path.to_path_buf(), std::fs::read_to_string(path).unwrap_or_default());
    let wanted = text.split('\n').nth(line).map(|l| l.trim().chars().take(60).collect::<String>()).unwrap_or_default();
    act(cx, |l| {
        l.open_file(&path);
        if !wanted.is_empty() {
            let nth = links::occurrences_before(&text, &wanted, line);
            l.find_in_file = Some((path.clone(), wanted.clone(), nth));
        }
    });
}

impl Render for DocSide {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let path = state::read(cx, |l| l.active_document.clone());
        let is_note = path.as_ref().is_some_and(|p| markdown::is_markdown(p));
        if state::lib(cx).update(cx, |l, _| std::mem::take(&mut l.show_versions)) {
            self.side = Side::Versions;
            self.focus_versions = true;
        }
        if std::mem::take(&mut self.focus_versions) {
            self.versions.update(cx, |v, cx| v.take_focus(window, cx));
        }
        if !is_note && self.side != Side::Versions {
            self.side = Side::Versions;
        }
        let tabs = div()
            .flex_none()
            .p(px(space::S))
            .flex()
            .gap(px(space::XS))
            .border_b_1()
            .border_color(t.separator)
            .when(is_note, |d| d.child(self.tab("side-tab-links", tr("Verknüpfungen"), Side::Links, &t, cx)).child(self.tab("side-tab-props", tr("Eigenschaften"), Side::Properties, &t, cx)))
            .child(self.tab("side-tab-versions", tr("Verlauf"), Side::Versions, &t, cx));
        let body: AnyElement = match (self.side, path) {
            (Side::Links, Some(p)) => self.links_view(&p, &t, cx),
            (Side::Properties, Some(_)) => self.properties.clone().into_any_element(),
            _ => self.versions.clone().into_any_element(),
        };
        div().size_full().flex().flex_col().child(tabs).child(div().flex_1().min_h_0().child(body))
    }
}

// MARK: Properties

/// The properties of the open note as fields.
pub struct PropertyEditor {
    path: Option<PathBuf>,
    new_name: Entity<InputState>,
    fields: super::fields::Fields,
    chip_inputs: std::collections::HashMap<(PathBuf, String), (Entity<InputState>, Subscription)>,
    pending_clear: Option<(PathBuf, String)>,
    _subs: Vec<Subscription>,
}

fn kind_icon(kind: PropertyKind) -> &'static str {
    match kind {
        PropertyKind::Text => "type",
        PropertyKind::List => "list",
        PropertyKind::Number => "hash",
        PropertyKind::Checkbox => "circle-check",
        PropertyKind::Date | PropertyKind::DateTime => "calendar",
        PropertyKind::Tags => "tag",
    }
}

/// The type Obsidian remembers for a name; one type per name in the whole vault.
fn known_kind(name: &str) -> Option<PropertyKind> {
    index::current().types.get(name).copied()
}

impl PropertyEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let new_name = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Neue Eigenschaft")));
        let subs = vec![cx.subscribe(&new_name, |this, input, e: &InputEvent, cx| {
            if matches!(e, InputEvent::PressEnter { .. }) {
                let name = input.read(cx).value().trim().to_string();
                if !name.is_empty() {
                    let kind = known_kind(&name).unwrap_or(PropertyKind::Text);
                    this.change(&name, kind, vec![String::new()], cx);
                }
            }
        })];
        Self { path: None, new_name, fields: Default::default(), chip_inputs: Default::default(), pending_clear: None, _subs: subs }
    }

    fn write(&mut self, label: String, edit: impl FnOnce(&str) -> String, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return };
        act(cx, |l| l.edit_note(&path, "property", label, edit));
        state::reindex(cx);
        cx.notify();
    }

    fn change(&mut self, name: &str, kind: PropertyKind, values: Vec<String>, cx: &mut Context<Self>) {
        let name = name.to_string();
        self.write(tr("Eigenschaft geändert"), move |text| frontmatter::set(text, &name, kind, &values), cx);
    }

    fn remove(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = name.to_string();
        self.write(tr("Eigenschaft entfernt"), move |text| frontmatter::remove(text, &name), cx);
    }

    /// Kapseln: a click removes a value, Enter in the field adds one and Backspace in the empty field takes the last back.
    fn chips(&mut self, row: usize, name: &str, kind: PropertyKind, values: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::get(cx);
        let key = (self.path.clone().unwrap_or_default(), name.to_string());
        let input = match self.chip_inputs.get(&key) {
            Some((input, _)) => input.clone(),
            None => {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Hinzufügen")));
                let (n, list_key) = (name.to_string(), key.clone());
                let sub = cx.subscribe(&input, move |this, field, e: &InputEvent, cx| {
                    if matches!(e, InputEvent::PressEnter { .. }) {
                        let value = field.read(cx).value().trim().to_string();
                        if value.is_empty() {
                            return;
                        }
                        let mut list = this.current_values(&n, cx);
                        if !list.contains(&value) {
                            list.push(value);
                        }
                        this.pending_clear = Some(list_key.clone());
                        this.change(&n, kind, list, cx);
                    }
                });
                self.chip_inputs.insert(key.clone(), (input.clone(), sub));
                input
            }
        };
        if self.pending_clear.as_ref() == Some(&key) {
            self.pending_clear = None;
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
        let me = cx.entity();
        let (back_name, back_values) = (name.to_string(), values.clone());
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::XS))
            .children(values.iter().enumerate().map(|(i, value)| {
                let (e, n, list) = (me.clone(), name.to_string(), values.clone());
                div()
                    .id(("prop-chip", row * 1000 + i))
                    .flex()
                    .items_center()
                    .gap(px(space::XXS))
                    .px(px(space::S))
                    .py(px(space::XXS))
                    .rounded_full()
                    .bg(t.ink.opacity(0.08))
                    .text_size(px(font::CAPTION))
                    .cursor_pointer()
                    .key_nav()
                    .child(value.clone())
                    .child(icon("x", iconsize::XS, t.secondary))
                    .tooltip(tip(tr("Entfernen")))
                    .on_click(move |_, _, cx| {
                        let mut rest = list.clone();
                        rest.remove(i);
                        let n = n.clone();
                        e.update(cx, |ed, cx| ed.change(&n, kind, rest, cx));
                    })
            }))
            .child(
                div()
                    .w(px(90.))
                    .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                        if ev.keystroke.key == "backspace" && !back_values.is_empty() && this.chip_field_is_empty(&back_name, cx) {
                            let mut rest = back_values.clone();
                            rest.pop();
                            this.change(&back_name, kind, rest, cx);
                        }
                    }))
                    .child(Input::new(&input).small().appearance(false)),
            )
            .into_any_element()
    }

    fn chip_field_is_empty(&self, name: &str, cx: &App) -> bool {
        let key = (self.path.clone().unwrap_or_default(), name.to_string());
        self.chip_inputs.get(&key).is_none_or(|(i, _)| i.read(cx).value().is_empty())
    }

    fn current_values(&self, name: &str, cx: &mut Context<Self>) -> Vec<String> {
        let Some(path) = self.path.clone() else { return Vec::new() };
        let text = state::lib(cx).update(cx, |l, _| l.buffer(&path).text.clone());
        frontmatter::properties(&text).and_then(|p| p.into_iter().find(|x| x.name == name)).map_or(Vec::new(), |p| p.values)
    }
}

impl Render for PropertyEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        self.path = state::read(cx, |l| l.active_document.clone());
        let Some(path) = self.path.clone() else { return div().into_any_element() };
        let text = state::lib(cx).update(cx, |l, _| l.buffer(&path).text.clone());
        let has_block = frontmatter::range(&text).is_some();
        let props = if has_block { frontmatter::properties(&text) } else { Some(Vec::new()) };
        let Some(props) = props else {
            return div().p(px(space::S)).child(notice("info", t.blue, tr("Nur im Quelltext bearbeitbar"), Some(tr("Diese Eigenschaften sind verschachtelt oder nicht lesbar. Schrumbi lässt sie unverändert.")), div(), &t)).into_any_element();
        };
        let me = cx.entity();
        let mut fields = std::mem::take(&mut self.fields);
        let mut rows = div().flex().flex_col();
        for (i, p) in props.iter().enumerate() {
            let Property { name, values, .. } = p.clone();
            let kind = known_kind(&name).unwrap_or(p.kind);
            let key = format!("{}:{name}", path.display());
            let control: AnyElement = match kind {
                PropertyKind::Checkbox => {
                    let (name, editor) = (name.clone(), me.clone());
                    Switch::new(("prop-check", i)).checked(values.first().is_some_and(|v| v == "true")).on_click(move |v, _, cx| {
                        let value = v.to_string();
                        editor.update(cx, |e, cx| e.change(&name, PropertyKind::Checkbox, vec![value], cx));
                    }).into_any_element()
                }
                PropertyKind::List | PropertyKind::Tags => self.chips(i, &name, kind, values.clone(), window, cx),
                PropertyKind::Date => {
                    let (name2, editor) = (name.clone(), me.clone());
                    let shown = values.first().and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
                    let picker = fields.date(&key, shown, window, cx, move |d, cx| {
                        let (n, e) = (name2.clone(), editor.clone());
                        e.update(cx, |e, cx| e.change(&n, kind, vec![d.format("%Y-%m-%d").to_string()], cx));
                    });
                    let vault = index::current();
                    let daily_name = shown.map(|d| crate::core::vault::moment::format(if vault.config.daily.format.is_empty() { "YYYY-MM-DD" } else { vault.config.daily.format.as_str() }, d.and_hms_opt(12, 0, 0).and_then(|x| x.and_local_timezone(chrono::Local).single()).unwrap_or_else(chrono::Local::now), crate::core::i18n::is_english()));
                    let daily = daily_name.map(|n| vault.root.join(vault.config.daily.folder.trim_matches('/')).join(format!("{n}.md"))).filter(|p| vault.notes.contains_key(p));
                    div().flex().items_center().gap(px(space::XS)).child(div().flex_1().min_w_0().child(gpui_kit::component::date_picker::DatePicker::new(&picker).small())).children(daily.map(|p| Button::new(("prop-daily", i)).ghost().small().icon(menu_icon("calendar")).tooltip(tr("Tagesnotiz öffnen")).on_click(move |_, _, cx| {
                        let p = p.clone();
                        act(cx, |l| l.open_file(&p));
                    }))).into_any_element()
                }
                _ => {
                    let (name2, editor) = (name.clone(), me.clone());
                    let state = fields.input(&key, values.first().map_or("", String::as_str), "", window, cx, move |v, cx| {
                        let (n, e) = (name2.clone(), editor.clone());
                        let valid = match kind {
                            PropertyKind::Number => v.is_empty() || v.parse::<f64>().is_ok(),
                            _ => true,
                        };
                        if valid {
                            e.update(cx, |e, cx| e.change(&n, kind, vec![v], cx));
                        }
                    });
                    let link = values.first().and_then(|v| v.strip_prefix("[[")).and_then(|v| v.strip_suffix("]]")).map(|d| d.split('|').next().unwrap_or(d).to_string());
                    let from = path.clone();
                    div().flex().items_center().gap(px(space::XS)).child(div().flex_1().min_w_0().child(Input::new(&state).small())).children(link.map(|d| Button::new(("prop-link", i)).ghost().small().icon(menu_icon("arrow-up-right")).tooltip(tr("Öffnen")).on_click(move |_, _, cx| {
                        let (d, f) = (d.clone(), from.clone());
                        act(cx, |l| {
                            l.open_link(&d, Some(&f));
                        });
                    }))).into_any_element()
                }
            };
            let (remove, editor) = (name.clone(), me.clone());
            rows = rows.child(
                div()
                    .px(px(space::M))
                    .py(px(space::XS))
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .child(icon(kind_icon(kind), iconsize::S, t.tertiary))
                    .child(div().w(px(84.)).flex_none().truncate().text_size(px(font::CALLOUT)).text_color(t.secondary).child(name))
                    .child(div().flex_1().min_w_0().child(control))
                    .child(Button::new(("prop-remove", i)).ghost().small().icon(menu_icon("x")).tooltip(tr("Eigenschaft entfernen")).on_click(move |_, _, cx| editor.update(cx, |e, cx| e.remove(&remove, cx)))),
            );
        }
        self.fields = fields;
        let empty = props.is_empty();
        let me2 = cx.entity();
        let suggestions: Vec<String> = {
            let mut names: std::collections::BTreeSet<String> = Default::default();
            for facts in index::current().notes.values() {
                names.extend(facts.properties.iter().map(|p| p.name.clone()));
            }
            for p in &props {
                names.remove(&p.name);
            }
            names.into_iter().take(40).collect()
        };
        div()
            .id("side-props")
            .size_full()
            .overflow_y_scrollbar()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .py(px(space::S))
                    .when(empty, |d| d.child(div().px(px(space::M)).py(px(space::S)).child(caption(tr("Diese Notiz hat noch keine Eigenschaften."), &t))))
                    .child(rows)
                    .child(div().p(px(space::M)).flex().items_center().gap(px(space::XS)).child(div().flex_1().child(Input::new(&self.new_name).small())).child(Button::new("prop-names").ghost().small().icon(menu_icon("chevron-down")).tooltip(tr("Namen aus dem Vault")).dropdown_menu(move |mut menu, _, _| {
                        for known in &suggestions {
                            let (k, e) = (known.clone(), me2.clone());
                            menu = menu.item(PopupMenuItem::new(known.clone()).on_click(move |_, _, cx| {
                                let k = k.clone();
                                let kind = known_kind(&k).unwrap_or(PropertyKind::Text);
                                e.update(cx, |ed, cx| ed.change(&k, kind, vec![String::new()], cx));
                            }));
                        }
                        menu
                    })))
                    .child(div().px(px(space::M)).child(caption(tr("Enter legt sie an."), &t))),
            )
            .into_any_element()
    }
}
