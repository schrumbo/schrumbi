use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::{Soft, chevron};
use super::state::{self, act};
use super::theme::{Theme, control, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::library::Change;
use crate::core::note_buffer::write_atomic;
use crate::core::vault::bases::{self, Base, ResultRow, Value};
use crate::core::vault::index;

#[derive(Clone)]
struct Rendered {
    columns: Vec<(String, String)>,
    rows: Arc<Vec<ResultRow>>,
    groups: Option<Vec<(String, Vec<usize>)>>,
    kind: String,
    summaries: Vec<(String, String, String)>,
    error: Option<String>,
}

/// A `.base` file as its own document.
pub struct BaseView {
    path: PathBuf,
    fit: super::bar_fit::BarFit,
    source: String,
    parsed: Result<Base, String>,
    view: usize,
    table: Option<Rendered>,
    built: Option<(u64, u64, usize, Option<(String, bool)>)>,
    sort: Option<(String, bool)>,
    filters_open: bool,
    filter_rows: Vec<Entity<InputState>>,
    filter_error: Option<String>,
    cursor: (usize, usize),
    editing: Option<(usize, usize, Entity<InputState>)>,
    closed_groups: std::collections::BTreeSet<String>,
    focus: FocusHandle,
    list: UniformListScrollHandle,
    version: u64,
    seen_generation: u64,
    task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

/// A column dragged in the table's head.
#[derive(Clone)]
struct DraggedColumn(String, String);

impl Render for DraggedColumn {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        div().px(px(space::S)).py(px(space::XS)).rounded(px(radius::INNER)).bg(t.secondary_bg).border_1().border_color(t.separator).text_size(px(font::CALLOUT)).child(self.1.clone())
    }
}

const COLUMN: f32 = 160.;
const ROW: f32 = 30.;

fn is_number(v: &Value) -> bool {
    matches!(v, Value::Number(_))
}

impl BaseView {
    pub fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        let parsed = Base::parse(&source);
        let sort = bases::view_sort(&source, 0);
        Self { path, source, parsed, view: 0, table: None, built: None, sort, filters_open: false, filter_rows: Vec::new(), filter_error: None, cursor: (0, 0), editing: None, closed_groups: Default::default(), focus: cx.focus_handle(), list: UniformListScrollHandle::new(), version: 0, seen_generation: index::current().generation, task: None, fit: Default::default(), _subs: Vec::new() }
    }

    /// Gives the keyboard to the table.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    /// Writes the file and keeps the previous bytes for undo.
    fn write(&mut self, updated: String, label: String, cx: &mut Context<Self>) {
        if updated == self.source {
            return;
        }
        let before = std::mem::replace(&mut self.source, updated.clone());
        if let Err(e) = write_atomic(&self.path, updated.as_bytes()) {
            self.source = before;
            act(cx, |l| l.last_error = Some(e.to_string()));
            return;
        }
        self.parsed = Base::parse(&self.source);
        self.version += 1;
        let path = self.path.clone();
        act(cx, |l| l.record_edit("base", label, Change::Bytes { path, bytes: before.into_bytes() }));
        cx.notify();
    }

    fn reload_if_changed(&mut self) {
        let generation = index::current().generation;
        if std::mem::replace(&mut self.seen_generation, generation) == generation {
            return;
        }
        let Ok(text) = std::fs::read_to_string(&self.path) else { return };
        if text != self.source {
            self.source = text;
            self.parsed = Base::parse(&self.source);
            self.view = self.view.min(self.parsed.as_ref().map_or(0, |b| b.views.len().saturating_sub(1)));
            self.sort = bases::view_sort(&self.source, self.view);
            self.version += 1;
        }
    }

    fn compute(&mut self, cx: &mut Context<Self>) {
        let vault = index::current();
        let key = (vault.generation, self.version, self.view, self.sort.clone());
        if self.built.as_ref() == Some(&key) {
            return;
        }
        self.built = Some(key);
        let Ok(base) = self.parsed.clone() else {
            self.table = None;
            return;
        };
        let view_index = self.view;
        let this_path = self.path.clone();
        let sort = self.sort.clone();
        let task = cx.background_spawn(async move {
            let view = base.views.get(view_index).cloned().unwrap_or_default();
            let now = chrono::Local::now();
            let columns: Vec<(String, String)> = base.columns(&view).into_iter().map(|c| (base.column_label(&c), c)).collect();
            match base.rows(&view, &vault, Some(&this_path), now) {
                Err(e) => Rendered { columns, rows: Arc::default(), groups: None, kind: view.kind.clone(), summaries: Vec::new(), error: Some(e) },
                Ok(mut rows) => {
                    if let Some((column, ascending)) = &sort {
                        let at = |r: &ResultRow| r.cells.iter().find(|(c, _)| c == column).map(|(_, v)| v.clone()).unwrap_or(Value::Null);
                        rows.sort_by(|a, b| {
                            let order = bases::compare(&at(a), &at(b));
                            if *ascending { order } else { order.reverse() }
                        });
                    }
                    let summaries = view.summaries.iter().map(|(column, name)| {
                        let values: Vec<Value> = rows.iter().filter_map(|r| r.cells.iter().find(|(c, _)| c == column).map(|(_, v)| v.clone())).collect();
                        (column.clone(), name.clone(), base.summary(name, &values, &vault, now).display())
                    }).collect();
                    let groups = view.group_by.as_ref().map(|_| {
                        let mut order: Vec<(String, Vec<usize>)> = Vec::new();
                        for (i, r) in rows.iter().enumerate() {
                            let key = r.group.as_ref().map_or_else(String::new, Value::display);
                            match order.iter_mut().find(|(k, _)| *k == key) {
                                Some((_, list)) => list.push(i),
                                None => order.push((key, vec![i])),
                            }
                        }
                        order
                    });
                    Rendered { columns, rows: Arc::new(rows), groups, kind: view.kind.clone(), summaries, error: None }
                }
            }
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let rendered = task.await;
            let _ = this.update(cx, |v, cx| {
                v.table = Some(rendered);
                cx.notify();
            });
        }));
    }

    fn set_sort(&mut self, column: &str, cx: &mut Context<Self>) {
        self.sort = match &self.sort {
            Some((c, true)) if c == column => Some((column.to_string(), false)),
            Some((c, false)) if c == column => None,
            _ => Some((column.to_string(), true)),
        };
        if let Some(updated) = bases::set_view_sort(&self.source, self.view, self.sort.as_ref()) {
            self.write(updated, tr("Sortierung geändert"), cx);
        }
        cx.notify();
    }

    fn move_column(&mut self, column: &str, before: &str, cx: &mut Context<Self>) {
        let Some(table) = &self.table else { return };
        let mut order: Vec<String> = table.columns.iter().map(|(_, c)| c.clone()).collect();
        let Some(from) = order.iter().position(|c| c == column) else { return };
        let moved = order.remove(from);
        let to = order.iter().position(|c| c == before).unwrap_or(order.len());
        order.insert(to, moved);
        if let Some(updated) = bases::set_view_order(&self.source, self.view, &order) {
            self.write(updated, tr("Spalten geändert"), cx);
        }
    }

    fn toggle_column(&mut self, column: &str, cx: &mut Context<Self>) {
        let Some(table) = &self.table else { return };
        let mut order: Vec<String> = table.columns.iter().map(|(_, c)| c.clone()).collect();
        match order.iter().position(|c| c == column) {
            Some(i) => {
                order.remove(i);
            }
            None => order.push(column.to_string()),
        }
        if let Some(updated) = bases::set_view_order(&self.source, self.view, &order) {
            self.write(updated, tr("Spalten geändert"), cx);
        }
    }

    fn open_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filters_open = !self.filters_open;
        self.filter_error = None;
        if self.filters_open {
            let lines = bases::view_filter_lines(&self.source, self.view).unwrap_or_default();
            self.filter_rows = lines.iter().map(|l| cx.new(|cx| InputState::new(window, cx).default_value(l.clone()))).collect();
        }
        cx.notify();
    }

    fn apply_filters(&mut self, cx: &mut Context<Self>) {
        let lines: Vec<String> = self.filter_rows.iter().map(|r| r.read(cx).value().trim().to_string()).filter(|l| !l.is_empty()).collect();
        for line in &lines {
            if let Err(e) = bases::parse(line) {
                self.filter_error = Some(format!("{line}: {e}"));
                cx.notify();
                return;
            }
        }
        self.filter_error = None;
        if let Some(updated) = bases::set_view_filter(&self.source, self.view, &lines) {
            self.write(updated, tr("Filter geändert"), cx);
        }
    }

    fn start_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(table) = &self.table else { return };
        let (row, col) = self.cursor;
        let Some((_, column)) = table.columns.get(col) else { return };
        if column.contains('.') && !column.starts_with("note.") {
            return;
        }
        let Some(cell) = table.rows.get(row).and_then(|r| r.cells.get(col)) else { return };
        let value = cell.1.display();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        input.update(cx, |s, cx| {
            let end = s.value().len();
            s.focus(window, cx);
            s.set_selected_range(0..end, cx);
        });
        self._subs.retain(|_| false);
        self._subs.push(cx.subscribe(&input, |this, input, e: &InputEvent, cx| match e {
            InputEvent::PressEnter { .. } => {
                let value = input.read(cx).value().to_string();
                this.commit_edit(value, cx);
            }
            InputEvent::Blur => {
                this.editing = None;
                cx.notify();
            }
            _ => {}
        }));
        self.editing = Some((row, col, input));
        cx.notify();
    }

    fn commit_edit(&mut self, value: String, cx: &mut Context<Self>) {
        let Some((row, col, _)) = self.editing.take() else { return };
        let Some(table) = &self.table else { return };
        let (Some(row), Some((_, column))) = (table.rows.get(row), table.columns.get(col)) else { return };
        let (path, column) = (row.path.clone(), column.clone());
        act(cx, |l| {
            l.edit_note(&path, "cell", tr("Eigenschaft geändert"), |text| bases::write_cell(text, &column, &value).unwrap_or_else(|| text.to_string()));
        });
        state::reindex(cx);
        window_focus_after(self, cx);
        cx.notify();
    }

    fn new_row(&mut self, cx: &mut Context<Self>) {
        let folder = self.path.parent().map(PathBuf::from);
        act(cx, |l| l.new_note = Some(crate::core::library::NewNoteRequest { module_id: None, folder }));
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            if e.keystroke.key == "escape" {
                self.editing = None;
                window.focus(&self.focus, cx);
                cx.notify();
            }
            return;
        }
        if self.filters_open || window.context_stack().iter().any(|c| c.contains("Input")) {
            return;
        }
        let Some(table) = &self.table else { return };
        let (rows, cols) = (table.rows.len(), table.columns.len());
        if rows == 0 {
            return;
        }
        let (mut r, mut c) = self.cursor;
        match e.keystroke.key.as_str() {
            "down" => r = (r + 1).min(rows - 1),
            "up" => r = r.saturating_sub(1),
            "right" => c = (c + 1).min(cols.saturating_sub(1)),
            "left" => c = c.saturating_sub(1),
            "enter" => {
                self.start_edit(window, cx);
                cx.stop_propagation();
                return;
            }
            "o" if e.keystroke.modifiers.platform => {
                if let Some(p) = table.rows.get(r).map(|x| x.path.clone()) {
                    act(cx, |l| l.open_file(&p));
                }
                return;
            }
            _ => return,
        }
        self.cursor = (r, c);
        if table.groups.is_none() {
            self.list.scroll_to_item(r, ScrollStrategy::Center);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn cell(&self, row: usize, col: usize, value: &Value, path: &PathBuf, me: &Entity<Self>, t: &Theme) -> AnyElement {
        if let Some((r, c, input)) = &self.editing
            && *r == row
            && *c == col
        {
            return div().w(px(COLUMN)).flex_none().px(px(space::XS)).child(Input::new(input).small()).into_any_element();
        }
        let selected = self.cursor == (row, col);
        let (click, open) = (me.clone(), path.clone());
        let editable = self.table.as_ref().and_then(|t| t.columns.get(col)).is_some_and(|(_, column)| !column.contains('.') || column.starts_with("note."));
        div()
            .id(("base-cell", row * 1000 + col))
            .on_click(move |e, window, cx| {
                click.update(cx, |v, cx| {
                    v.cursor = (row, col);
                    window.focus(&v.focus, cx);
                    if e.click_count() >= 2 && editable {
                        v.start_edit(window, cx);
                    }
                    cx.notify();
                });
                if e.click_count() >= 2 && !editable {
                    let p = open.clone();
                    act(cx, |l| l.open_file(&p));
                }
            })
            .w(px(COLUMN))
            .flex_none()
            .h(px(ROW))
            .px(px(space::S))
            .flex()
            .items_center()
            .when(is_number(value), |d| d.justify_end().font_features(tabular()))
            .when(selected, |d| d.border_1().border_color(t.accent).rounded(px(radius::SMALL)))
            .text_size(px(font::CALLOUT))
            .child(div().truncate().child(value.display().strip_suffix(".md").map_or_else(|| value.display(), str::to_string)))
            .into_any_element()
    }

    fn table_rows(&self, table: &Rendered, range: std::ops::Range<usize>, me: Entity<Self>, t: &Theme) -> Vec<AnyElement> {
        range
            .filter_map(|i| {
                let row = table.rows.get(i)?;
                let hover = t.ink.opacity(0.05);
                let cells: Vec<AnyElement> = row.cells.iter().enumerate().map(|(c, (_, v))| self.cell(i, c, v, &row.path, &me, t)).collect();
                Some(
                    div()
                        .id(("base-row", i))
                        .h(px(ROW))
                        .flex()
                        .border_b_1()
                        .border_color(t.separator)
                        .soft(hover, 0.)
                        .children(cells)
                        .into_any_element(),
                )
            })
            .collect()
    }
}

fn window_focus_after(view: &mut BaseView, cx: &mut Context<BaseView>) {
    let focus = view.focus.clone();
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, cx| window.focus(&focus, cx));
        }
    });
}

impl Render for BaseView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        if self.editing.is_none() {
            self.reload_if_changed();
        }
        self.compute(cx);
        if let Err(e) = &self.parsed {
            return div().size_full().p(px(space::L)).flex().flex_col().gap(px(space::M)).child(notice("triangle-alert", t.orange, tr("Diese Datei lässt sich nicht lesen."), Some(e.clone()), div(), &t)).child(div().id("base-source").flex_1().overflow_y_scrollbar().rounded(px(radius::CARD)).bg(t.secondary_bg).p(px(space::M)).font_family("Menlo").text_size(px(font::CALLOUT)).child(self.source.clone())).into_any_element();
        }
        let views: Vec<(usize, String)> = self.parsed.as_ref().map_or(Vec::new(), |b| b.views.iter().enumerate().map(|(i, v)| (i, if v.name.is_empty() { tr("Ansicht") } else { v.name.clone() })).collect());
        let current_name = views.get(self.view).map_or_else(|| tr("Ansicht"), |v| v.1.clone());
        let me = cx.entity();
        let filter_count = bases::view_filter_lines(&self.source, self.view);
        let filter_label = match &filter_count {
            Some(l) if !l.is_empty() => format!("{} ({})", tr("Filter"), l.len()),
            _ => tr("Filter"),
        };
        let view_menu = {
            let me = me.clone();
            let views = views.clone();
            let current = self.view;
            Button::new("base-view").small().icon(menu_icon("table")).label(current_name).dropdown_menu(move |mut menu, _, _| {
                for (i, name) in &views {
                    let (i, me) = (*i, me.clone());
                    menu = menu.item(PopupMenuItem::new(name.clone()).checked(i == current).on_click(move |_, _, cx| {
                        me.update(cx, |v, cx| {
                            v.view = i;
                            v.sort = bases::view_sort(&v.source, i);
                            v.cursor = (0, 0);
                            v.table = None;
                            cx.notify();
                        });
                    }));
                }
                menu
            })
        };
        let property_menu = {
            let me = me.clone();
            let active: Vec<String> = self.table.as_ref().map_or(Vec::new(), |tb| tb.columns.iter().map(|(_, c)| c.clone()).collect());
            let vault = index::current();
            let mut all: Vec<String> = ["file.name", "file.basename", "file.path", "file.folder", "file.ext", "file.size", "file.ctime", "file.mtime", "file.tags", "file.links"].iter().map(|s| s.to_string()).collect();
            let mut own: std::collections::BTreeSet<String> = Default::default();
            for facts in vault.notes.values() {
                own.extend(facts.properties.iter().map(|p| p.name.clone()));
            }
            all.extend(own);
            if let Ok(base) = &self.parsed {
                let mut formulas: Vec<String> = base.formulas.keys().map(|k| format!("formula.{k}")).collect();
                formulas.sort();
                all.extend(formulas);
            }
            Button::new("base-props").small().ghost().label(tr("Eigenschaften")).dropdown_menu(move |mut menu, _, _| {
                for name in &all {
                    let (m, n) = (me.clone(), name.clone());
                    menu = menu.item(PopupMenuItem::new(name.clone()).checked(active.contains(name)).on_click(move |_, _, cx| {
                        let n = n.clone();
                        m.update(cx, |v, cx| v.toggle_column(&n, cx));
                    }));
                }
                menu
            })
        };
        let bar = div()
            .flex_none()
            .px(px(space::M))
            .h(px(control::SUBBAR))
            .flex()
            .items_center()
            .gap(px(space::S))
            .border_b_1()
            .border_color(t.separator)
            .child(view_menu)
            .child(Button::new("base-filter").small().ghost().icon(menu_icon("list-filter")).label(filter_label).on_click(cx.listener(|v, _, window, cx| v.open_filters(window, cx))))
            .child(property_menu)
            .child(div().flex_1())
            .child(Button::new("base-new-row").small().icon(menu_icon("plus")).label(tr("Neue Notiz")).on_click(cx.listener(|v, _, _, cx| v.new_row(cx))));
        let filters = self.filters_open.then(|| {
            let readable = filter_count.is_some();
            div()
                .flex_none()
                .p(px(space::M))
                .flex()
                .flex_col()
                .gap(px(space::S))
                .border_b_1()
                .border_color(t.separator)
                .when(!readable, |d| d.child(caption(tr("Dieser Filter ist verschachtelt und lässt sich nur im Quelltext ändern."), &t)))
                .when(readable, |d| {
                    d.children(self.filter_rows.iter().enumerate().map(|(i, row)| {
                        div().flex().items_center().gap(px(space::S)).child(div().flex_1().child(Input::new(row).small())).child(Button::new(("filter-remove", i)).ghost().small().icon(menu_icon("x")).tooltip(tr("Filter entfernen")).on_click(cx.listener(move |v, _, _, cx| {
                            v.filter_rows.remove(i);
                            v.apply_filters(cx);
                        })))
                    }))
                    .when_some(self.filter_error.clone(), |d, e| d.child(text(e, font::CAPTION, t.red)))
                    .child(div().flex().gap(px(space::S)).child(Button::new("filter-add").small().ghost().icon(menu_icon("plus")).label(tr("Bedingung")).on_click(cx.listener(|v, _, window, cx| {
                        v.filter_rows.push(cx.new(|cx| InputState::new(window, cx).placeholder("status == \"offen\"")));
                        cx.notify();
                    }))).child(Button::new("filter-apply").small().primary().label(tr("Anwenden")).on_click(cx.listener(|v, _, _, cx| v.apply_filters(cx)))))
                })
        });
        let body: AnyElement = match self.table.clone() {
            None => div().flex_1().flex().items_center().justify_center().child(caption(tr("Lädt…"), &t)).into_any_element(),
            Some(table) if table.error.is_some() => div().p(px(space::L)).child(notice("triangle-alert", t.orange, tr("Diese Ansicht lässt sich nicht lesen."), table.error.clone(), div(), &t)).into_any_element(),
            Some(table) if table.rows.is_empty() => unavailable("list-filter", tr("Keine Notizen passen zu diesem Filter"), tr("Ändere den Filter, um Zeilen zu sehen."), &t).into_any_element(),
            Some(table) => self.body(table, window, cx),
        };
        let footer = self.table.as_ref().filter(|tb| !tb.rows.is_empty()).map(|tb| {
            let n = tb.rows.len();
            div().flex_none().px(px(space::L)).py(px(space::XS)).flex().gap(px(space::L)).border_t_1().border_color(t.separator).child(caption(if n == 1 { tr("1 Ergebnis") } else { trf("%@ Ergebnisse", &[&n.to_string()]) }, &t)).child(div().flex_1()).children(tb.summaries.iter().map(|(c, name, v)| caption(format!("{c} · {name}: {v}"), &t).font_features(tabular())))
        });
        let hand_over = super::vault_tools::in_vault(&self.path, cx).then(|| super::vault_tools::obsidian_button("base-obsidian", self.path.clone()));
        let tabs = super::documents::document_bar(&self.path, &self.fit, div().children(hand_over).into_any_element(), &t, window, cx);
        div()
            .id("base-page")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(Self::on_key))
            .child(tabs)
            .child(bar)
            .children(filters)
            .child(div().flex_1().min_h_0().child(body))
            .children(footer)
            .into_any_element()
    }
}

impl BaseView {
    fn head(&self, table: &Rendered, t: &Theme, cx: &mut Context<Self>) -> Div {
        let hover = t.ink.opacity(0.05);
        div().flex().flex_none().h(px(ROW)).bg(t.secondary_bg).border_b_1().border_color(t.separator).children(table.columns.iter().enumerate().map(|(i, (label, column))| {
            let sorted = self.sort.as_ref().filter(|(c, _)| c == column).map(|(_, up)| *up);
            let (a, b, label2) = (column.clone(), column.clone(), label.clone());
            div()
                .id(("base-col", i))
                .w(px(COLUMN))
                .flex_none()
                .px(px(space::S))
                .flex()
                .items_center()
                .gap(px(space::XS))
                .cursor_pointer()
                .key_nav()
                .soft(hover, 0.)
                .text_size(px(font::CALLOUT))
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().truncate().child(label.clone()))
                .children(sorted.map(|up| icon(if up { "arrow-up" } else { "arrow-down" }, iconsize::XS, t.secondary)))
                .on_click(cx.listener(move |v, _, _, cx| v.set_sort(&a, cx)))
                .on_drag(DraggedColumn(column.clone(), label2), |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<DraggedColumn>(|s, _, _, cx| s.bg(Theme::get(cx).accent.opacity(0.2)))
                .on_drop(cx.listener(move |v, d: &DraggedColumn, _, cx| v.move_column(&d.0, &b, cx)))
        }))
    }

    fn body(&mut self, table: Rendered, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::get(cx);
        let me = cx.entity();
        match table.kind.as_str() {
            "cards" => {
                let hover = t.ink.opacity(0.05);
                div()
                    .id("base-cards")
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(div().p(px(space::L)).flex().flex_wrap().gap(px(space::M)).children(table.rows.iter().enumerate().map(|(i, row)| {
                        let open = row.path.clone();
                        let title = row.path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                        div()
                            .id(("base-card", i))
                            .w(px(220.))
                            .p(px(space::M))
                            .flex()
                            .flex_col()
                            .gap(px(space::XS))
                            .rounded(px(radius::CARD))
                            .border_1()
                            .border_color(t.separator)
                            .cursor_pointer()
                            .key_nav()
                            .soft(hover, radius::CARD)
                            .child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(title))
                            .children(row.cells.iter().filter(|(_, v)| !matches!(v, Value::Null)).skip(1).map(|(c, v)| caption(format!("{c}: {}", v.display()), &t).truncate()))
                            .on_click(move |_, _, cx| act(cx, |l| l.open_file(&open)))
                    })))
                    .into_any_element()
            }
            "list" => {
                let hover = t.ink.opacity(0.05);
                div()
                    .id("base-list")
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(div().py(px(space::S)).flex().flex_col().children(table.rows.iter().enumerate().map(|(i, row)| {
                        let open = row.path.clone();
                        let title = row.path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                        let rest: Vec<String> = row.cells.iter().filter(|(_, v)| !matches!(v, Value::Null)).skip(1).map(|(_, v)| v.display()).collect();
                        div().id(("base-line", i)).h(px(control::M)).px(px(space::L)).flex().items_center().gap(px(space::M)).cursor_pointer().key_nav().soft(hover, 0.).child(file_icon(&row.path, iconsize::S)).child(div().truncate().child(title)).child(caption(rest.join(" · "), &t).truncate()).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open)))
                    })))
                    .into_any_element()
            }
            _ => {
                let width = table.columns.len() as f32 * COLUMN;
                let head = self.head(&table, &t, cx);
                let rows: AnyElement = if let Some(groups) = table.groups.clone() {
                    let mut list = div().flex().flex_col();
                    for (g, (name, members)) in groups.into_iter().enumerate() {
                        let open = !self.closed_groups.contains(&name);
                        let toggle = name.clone();
                        let label = if name.is_empty() { tr("Ohne Wert") } else { name };
                        list = list.child(
                            div()
                                .id(("base-group", g))
                                .h(px(ROW))
                                .px(px(space::S))
                                .flex()
                                .items_center()
                                .gap(px(space::XS))
                                .cursor_pointer()
                                .key_nav()
                                .soft(t.ink.opacity(0.045), 0.)
                                .bg(t.secondary_bg)
                                .text_size(px(font::CALLOUT))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(chevron(("base-group-chevron", g), open, t.secondary))
                                .child(label)
                                .child(caption(members.len().to_string(), &t).font_features(tabular()))
                                .on_click(cx.listener(move |v, _, _, cx| {
                                    if !v.closed_groups.remove(&toggle) {
                                        v.closed_groups.insert(toggle.clone());
                                    }
                                    cx.notify();
                                })),
                        );
                        if open {
                            for i in members {
                                list = list.children(self.table_rows(&table, i..i + 1, me.clone(), &t));
                            }
                        }
                    }
                    div().id("base-groups").flex_1().min_h_0().overflow_y_scrollbar().child(list).into_any_element()
                } else {
                    let count = table.rows.len();
                    let shared = Rc::new(table.clone());
                    let entity = me.clone();
                    uniform_list("base-rows", count, move |range, _, cx| {
                        let t = Theme::get(cx);
                        let shared = shared.clone();
                        entity.read(cx).table_rows(&shared, range, entity.clone(), &t)
                    })
                    .track_scroll(&self.list)
                    .flex_1()
                    .min_h_0()
                    .into_any_element()
                };
                div().id("base-hscroll").size_full().overflow_x_scrollbar().child(div().h_full().min_w(px(width)).flex().flex_col().child(head).child(rows)).into_any_element()
            }
        }
    }
}
