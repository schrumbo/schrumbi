use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use chrono::{Days, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::Sizable;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::core::vault::query as vault_query;
use super::state::{self, act};
use super::motion::{self, Soft};
use super::theme::{Theme, control, font, radius, space, iconsize};
use super::widgets::*;
use crate::core::catalog::{self, CatalogFile, FileKind, Query};
use crate::core::day;
use crate::core::model::{LectureWeek, StudyConfig};
use crate::core::schedule;
use crate::core::i18n::{tr, trf};
use crate::core::library::{self, Library, NewNoteRequest};
use crate::core::markdown;
use crate::core::naming::extension;
use crate::core::prefs::{self, FileGrouping};
use crate::core::search_index::Spot;
use crate::core::text::fold;

#[derive(Clone, PartialEq)]
pub struct Item {
    pub file: CatalogFile,
    pub spots: Vec<Spot>,
    pub score: f64,
}

#[derive(Clone, PartialEq)]
enum GroupKind {
    Week(Option<u32>, Option<NaiveDate>),
    Folder(Option<String>, Vec<String>),
    Date(String),
}

#[derive(Clone)]
struct Group {
    kind: GroupKind,
    items: Vec<Item>,
}

impl Group {
    fn id(&self) -> String {
        match &self.kind {
            GroupKind::Week(w, m) => w.map_or("none".into(), |w| format!("w{w}{}", m.map(|m| format!("-{m}")).unwrap_or_default())),
            GroupKind::Folder(m, p) => format!("f:{}/{}", m.clone().unwrap_or_default(), p.join("/")),
            GroupKind::Date(d) => format!("d:{d}"),
        }
    }
}

/// Where each lecture week starts, in the semester of the file's own module.
#[derive(Clone, Default, PartialEq)]
struct WeekStarts {
    by_semester: HashMap<String, Vec<LectureWeek>>,
    semester_of: HashMap<String, String>,
    shown: Vec<LectureWeek>,
    today: NaiveDate,
}

impl WeekStarts {
    fn new(l: &Library, config: &StudyConfig) -> Self {
        Self {
            by_semester: config.semesters.iter().map(|s| (s.id.clone(), schedule::lecture_weeks(s))).collect(),
            semester_of: config.modules.iter().map(|m| (m.id.clone(), m.semester.clone())).collect(),
            shown: l.weeks(),
            today: l.today(),
        }
    }

    fn shown_monday(&self, w: u32) -> Option<NaiveDate> {
        self.shown.iter().find(|x| x.index == w).map(|x| x.monday)
    }

    fn monday(&self, f: &CatalogFile) -> Option<NaiveDate> {
        let w = f.week?;
        let list = f.module_id.as_ref().and_then(|m| self.semester_of.get(m)).and_then(|s| self.by_semester.get(s)).unwrap_or(&self.shown);
        list.iter().find(|x| x.index == w).map(|x| x.monday)
    }

    fn in_shown(&self, f: &CatalogFile) -> bool {
        f.week.is_some_and(|w| self.monday(f).is_none_or(|m| Some(m) == self.shown_monday(w)))
    }
}

#[derive(Default)]
struct Results {
    best: Vec<Item>,
    groups: Vec<Group>,
    total: usize,
    other_kinds: usize,
    searching: bool,
    query: Query,
    loaded: bool,
}

/// One row of the virtualized list: indices into `Results`.
#[derive(Clone, Copy, PartialEq)]
enum Line {
    BestHeader,
    Best(usize),
    Group(usize),
    Row(usize, usize),
    OtherKinds,
}

/// Files grouped by lecture week, newest first; typing narrows by name, folder and content.
/// "w3", "kw44", "woche 3" and module codes in the query act as filters.
pub struct FileBrowser {
    scope_module: Option<String>,
    query: String,
    kind: FileKind,
    module_filter: Option<String>,
    week_filter: Option<u32>,
    week_starts: WeekStarts,
    pub selected: Option<PathBuf>,
    picked: HashSet<PathBuf>,
    range_start: Option<PathBuf>,
    renaming: Option<(PathBuf, Entity<InputState>, Subscription)>,
    /// Enter that confirms a rename must not also open the file.
    renamed_at: Option<std::time::Instant>,
    toggled: HashSet<String>,
    results: Results,
    key: String,
    load: Option<Task<()>>,
    /// Picks the best hit automatically, as the search page's preview needs.
    pub auto_select: bool,
    focus: FocusHandle,
    list_focused: bool,
    lines: Vec<Line>,
    list: ListState,
    results_changed: bool,
    shown_view: String,
    /// Why a vault search cannot be understood; the last hits stay meanwhile.
    query_error: Option<String>,
    vault_sort: vault_query::Sort,
    _sub: Subscription,
}

impl FileBrowser {
    pub fn new(scope_module: Option<String>, auto_select: bool, cx: &mut Context<Self>) -> Self {
        let lib = state::lib(cx);
        let sub = cx.observe(&lib, |this, _, cx| this.reload(cx));
        let mut b = Self {
            scope_module,
            query: String::new(),
            kind: FileKind::from_key(&prefs::get().file_kind),
            module_filter: None,
            week_filter: None,
            week_starts: WeekStarts::default(),
            selected: None,
            picked: HashSet::new(),
            range_start: None,
            renaming: None, renamed_at: None,
            toggled: HashSet::new(),
            results: Results::default(),
            key: String::new(),
            load: None,
            auto_select,
            focus: cx.focus_handle(),
            list_focused: false,
            lines: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            results_changed: false,
            shown_view: String::new(),
            query_error: None,
            vault_sort: vault_query::Sort::default(),
            _sub: sub,
        };
        b.reload(cx);
        b
    }

    /// Gives the keyboard to the list.
    pub fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    pub fn set_scope(&mut self, module: Option<String>, cx: &mut Context<Self>) {
        if self.scope_module != module {
            self.scope_module = module;
            self.toggled.clear();
            self.selected = None;
            self.reload(cx);
        }
    }

    pub fn set_query(&mut self, q: &str, cx: &mut Context<Self>) {
        if self.query != q {
            self.query = q.to_string();
            self.reload(cx);
        }
    }

    fn key(&self, lib: &Library) -> String {
        format!("{}|{:?}|{:?}|{:?}|{}|{:?}|{}", self.query, self.scope_module, self.module_filter, self.week_filter, self.kind.key(), prefs::get().file_grouping, lib.index_version)
    }

    /// Recomputes the list when its inputs changed; name matches are instant, content matches arrive from a background query.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let lib = state::lib(cx);
        let l = lib.read(cx);
        let key = self.key(l);
        let starts = l.config.as_ref().map(|c| WeekStarts::new(l, c)).unwrap_or_default();
        if key == self.key && starts == self.week_starts {
            return;
        }
        let was_searching = self.results.searching;
        self.key = key;
        let Some(config) = l.config.clone() else { return };
        let weeks = l.weeks();
        let current = current_week(l);
        let q = Query::parse(&self.query, &config.modules, &weeks);
        let module = self.scope_module.clone().or(q.module_id.clone()).or(self.module_filter.clone());
        let week = q.week.or(self.week_filter);
        let picked_week = q.week.is_none() && week.is_some();
        let searching = !q.terms.is_empty();
        let scoped: Vec<CatalogFile> = l.catalog.iter().filter(|f| module.as_ref().is_none_or(|m| f.module_id.as_ref() == Some(m)) && week.is_none_or(|w| f.week == Some(w) && (!picked_week || starts.in_shown(f)))).cloned().collect();
        self.week_starts = starts.clone();
        let mut items: Vec<Item> = if searching {
            scoped.iter().filter_map(|f| catalog::score(f, &q.terms, current).map(|s| Item { file: f.clone(), spots: Vec::new(), score: s })).collect()
        } else {
            scoped.iter().filter(|f| !f.in_project).map(|f| Item { file: f.clone(), spots: Vec::new(), score: 0. }).collect()
        };
        let kind = self.kind;
        let grouping = prefs::get().file_grouping;
        let today = l.today();
        let module_order: HashMap<String, usize> = config.modules.iter().enumerate().map(|(i, m)| (m.id.clone(), i)).collect();
        let finish = move |items: Vec<Item>, scoped: &[CatalogFile]| -> Results {
            let mut items = items;
            if searching && items.is_empty() {
                items = scoped.iter().filter_map(|f| catalog::fuzzy_score(f, &q.terms, current).map(|s| Item { file: f.clone(), spots: Vec::new(), score: s })).collect();
            }
            let total_all = items.len();
            let mut matching: Vec<Item> = items.into_iter().filter(|i| kind.matches(i.file.name())).collect();
            let other_kinds = total_all - matching.len();
            matching.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| newest_first(a, b)));
            let best: Vec<Item> = if searching && week.is_none() && matching.len() > 8 { matching[..5].to_vec() } else { Vec::new() };
            let rest = matching[best.len()..].to_vec();
            Results { best, groups: group(rest, searching, grouping, today, &module_order, &starts), total: matching.len(), other_kinds: if searching { other_kinds } else { 0 }, searching, query: q.clone(), loaded: true }
        };
        self.query_error = None;
        if searching && vault_query::is_structured(&self.query) {
            match vault_query::parse(&self.query) {
                Err(e) => {
                    self.query_error = Some(e);
                    cx.notify();
                }
                Ok(parsed) => {
                    let (vault, sort) = (crate::core::vault::index::current(), self.vault_sort);
                    self.load = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(Duration::from_millis(120)).await;
                        let paths = cx.background_spawn(async move { vault_query::search(&parsed, &vault, sort) }).await;
                        let by_path: HashMap<PathBuf, &CatalogFile> = scoped.iter().map(|f| (f.file.path.clone(), f)).collect();
                        let count = paths.len() as f64;
                        let items: Vec<Item> = paths.iter().enumerate().filter_map(|(i, p)| by_path.get(p).map(|f| Item { file: (*f).clone(), spots: Vec::new(), score: count - i as f64 })).collect();
                        let results = finish(items, &scoped);
                        let _ = this.update(cx, |b, cx| {
                            b.results = results;
                            b.after_load(true);
                            cx.notify();
                        });
                    }));
                }
            }
            return;
        }
        if !searching {
            self.results = finish(items, &scoped);
            self.after_load(was_searching);
            cx.notify();
            return;
        }
        let index = l.search_index.clone();
        let prefix = module.as_ref().and_then(|m| config.module(m)).zip(l.root.clone()).map(|(m, r)| r.join(&m.folder));
        self.results = finish(items.clone(), &scoped);
        self.after_load(was_searching);
        cx.notify();
        let terms = self.results.query.terms.clone();
        let finish = std::sync::Arc::new(finish);
        self.load = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            let hits = cx.background_spawn(async move { index.content_search(&terms, prefix.as_deref(), 150) }).await;
            let mut by_name: HashMap<PathBuf, usize> = items.iter().enumerate().map(|(i, item)| (item.file.file.path.clone(), i)).collect();
            let by_path: HashMap<PathBuf, &CatalogFile> = scoped.iter().map(|f| (f.file.path.clone(), f)).collect();
            for h in hits {
                if let Some(i) = by_name.get(&h.file.path) {
                    items[*i].spots = h.spots;
                } else if let Some(f) = by_path.get(&h.file.path) {
                    by_name.insert(h.file.path.clone(), items.len());
                    items.push(Item { file: (*f).clone(), spots: h.spots, score: 0.5 + catalog::recency(f.week, current) });
                }
            }
            let results = finish(items, &scoped);
            let _ = this.update(cx, |b, cx| {
                b.results = results;
                b.after_load(true);
                cx.notify();
            });
        }));
    }

    fn after_load(&mut self, was_searching: bool) {
        if was_searching != self.results.searching {
            self.toggled.clear();
        }
        let view = format!("{}|{:?}|{:?}|{:?}", self.query, self.kind.key(), self.module_filter, self.week_filter);
        if view != self.shown_view {
            self.shown_view = view;
            self.results_changed = true;
        }
        let first = self.results.best.first().or_else(|| self.results.groups.first().and_then(|g| g.items.first())).map(|i| i.file.file.path.clone());
        if self.auto_select && (self.results.searching || self.selected.as_ref().is_none_or(|s| !self.contains(s))) {
            self.selected = first;
        } else if self.selected.as_ref().is_some_and(|s| !self.contains(s)) {
            self.selected = None;
        }
        let picked = std::mem::take(&mut self.picked);
        self.picked = picked.into_iter().filter(|p| self.contains(p)).collect();
    }

    fn contains(&self, p: &PathBuf) -> bool {
        self.results.best.iter().chain(self.results.groups.iter().flat_map(|g| g.items.iter())).any(|i| &i.file.file.path == p)
    }

    fn is_expanded(&self, g: &Group, current: Option<u32>) -> bool {
        let base = if self.results.searching || self.week_filter.is_some() || self.results.groups.len() <= 3 {
            true
        } else if let GroupKind::Week(Some(_), Some(m)) = &g.kind {
            *m + Days::new(7) >= day::monday(self.week_starts.today)
        } else if let (GroupKind::Week(Some(w), None), Some(c)) = (&g.kind, current) {
            *w + 1 >= c
        } else {
            self.results.groups.iter().position(|x| x.id() == g.id()).unwrap_or(99) < 3
        };
        base != self.toggled.contains(&g.id())
    }

    /// Paths in display order, for previews and arrow keys.
    pub fn visible(&self, current: Option<u32>) -> Vec<PathBuf> {
        self.results.best.iter().chain(self.results.groups.iter().filter(|g| self.is_expanded(g, current)).flat_map(|g| g.items.iter())).map(|i| i.file.file.path.clone()).collect()
    }

    /// The files an action applies to: the multiple selection, otherwise the selected file.
    fn targets(&self, current: Option<u32>) -> Vec<PathBuf> {
        if self.picked.len() > 1 { self.visible(current).into_iter().filter(|p| self.picked.contains(p)).collect() } else { self.selected.iter().cloned().collect() }
    }

    fn pick_only(&mut self, path: &PathBuf) {
        self.selected = Some(path.clone());
        self.picked = HashSet::from([path.clone()]);
        self.range_start = Some(path.clone());
    }

    fn pick_range(&mut self, to: &PathBuf, current: Option<u32>) {
        let v = self.visible(current);
        let start = self.range_start.as_ref().and_then(|s| v.iter().position(|p| p == s));
        match (start, v.iter().position(|p| p == to)) {
            (Some(a), Some(b)) => {
                self.picked = v[a.min(b)..=a.max(b)].iter().cloned().collect();
                self.selected = Some(to.clone());
            }
            _ => self.pick_only(to),
        }
    }

    fn pick_toggle(&mut self, path: &PathBuf) {
        if let Some(s) = self.selected.clone().filter(|_| self.picked.is_empty()) {
            self.picked.insert(s);
        }
        if !self.picked.remove(path) {
            self.picked.insert(path.clone());
            self.selected = Some(path.clone());
        }
        self.range_start = Some(path.clone());
    }

    fn trash_targets(&mut self, cx: &mut Context<Self>) {
        let targets = self.targets(state::read(cx, current_week));
        if !targets.is_empty() {
            self.picked.clear();
            state::trash(targets, cx);
        }
    }

    /// Turns the file's name into a field in its row; Enter renames, leaving the field cancels.
    fn start_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let ext = extension(&name).to_string();
        let stem = if ext.is_empty() { name.clone() } else { name[..name.len() - ext.len() - 1].to_string() };
        let field = cx.new(|cx| InputState::new(window, cx).default_value(stem));
        let target = path.clone();
        let sub = cx.subscribe_in(&field, window, move |this, s, e: &InputEvent, window, cx| match e {
            InputEvent::PressEnter { .. } => {
                let stem = s.read(cx).value().trim().to_string();
                if !stem.is_empty() && !stem.contains(['/', '\\']) {
                    let name = if ext.is_empty() { stem } else { format!("{stem}.{ext}") };
                    let file = target.clone();
                    act(cx, |l| l.rename(&file, &name));
                    if let Some(new) = state::lib(cx).update(cx, |l, _| l.last_renamed.take()) {
                        this.pick_only(&new);
                    }
                }
                this.renaming = None;
                this.renamed_at = Some(std::time::Instant::now());
                window.focus(&this.focus, cx);
                cx.notify();
            }
            InputEvent::Blur => {
                this.renaming = None;
                cx.notify();
            }
            _ => {}
        });
        field.update(cx, |s, cx| s.focus(window, cx));
        self.renaming = Some((path, field, sub));
        cx.notify();
    }

    pub fn move_selection(&mut self, step: i64, cx: &mut Context<Self>) {
        let current = state::read(cx, current_week);
        let v = self.visible(current);
        if v.is_empty() {
            return;
        }
        let i = self.selected.as_ref().and_then(|s| v.iter().position(|p| p == s)).map_or(0, |i| (i as i64 + step).clamp(0, v.len() as i64 - 1) as usize);
        self.selected = Some(v[i].clone());
        self.reveal_selection();
        cx.notify();
    }

    /// Opens the selected file; a content match opens at its first occurrence.
    pub fn open_selected(&self, cx: &mut App) {
        let Some(path) = self.selected.clone() else { return };
        let r = &self.results;
        let spot = r.best.iter().chain(r.groups.iter().flat_map(|g| g.items.iter())).find(|i| i.file.file.path == path).and_then(|i| i.spots.first().cloned());
        open_at(&path, spot, cx);
    }

    fn line_list(&self, current: Option<u32>) -> Vec<Line> {
        let r = &self.results;
        let mut lines = Vec::new();
        if !r.best.is_empty() {
            lines.push(Line::BestHeader);
            lines.extend((0..r.best.len()).map(Line::Best));
        }
        for (g, group) in r.groups.iter().enumerate() {
            lines.push(Line::Group(g));
            if self.is_expanded(group, current) {
                lines.extend((0..group.items.len()).map(|i| Line::Row(g, i)));
            }
        }
        if r.other_kinds > 0 {
            lines.push(Line::OtherKinds);
        }
        lines
    }

    fn reveal_selection(&self) {
        let Some(selected) = &self.selected else { return };
        let r = &self.results;
        let found = self.lines.iter().position(|l| match *l {
            Line::Best(i) => &r.best[i].file.file.path == selected,
            Line::Row(g, i) => &r.groups[g].items[i].file.file.path == selected,
            _ => false,
        });
        if let Some(ix) = found {
            self.list.scroll_to_reveal_item(ix);
        }
    }

    fn render_line(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let unsaved = super::sync::unsaved_files(cx);
        let t = Theme::get(cx);
        let me = cx.entity();
        let lib = state::lib(cx);
        let l = lib.read(cx);
        let show_module = self.scope_module.is_none();
        let content = match self.lines.get(ix).copied() {
            Some(Line::BestHeader) => div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).py(px(space::XS)).child(tr("Beste Treffer")).into_any_element(),
            Some(Line::Best(i)) => self.row(&self.results.best[i], show_module, true, l, &unsaved, &t, &me),
            Some(Line::Group(g)) => {
                let group = &self.results.groups[g];
                self.group_header(group, self.is_expanded(group, current_week(l)), l, &t, &me)
            }
            Some(Line::Row(g, i)) => self.row(&self.results.groups[g].items[i], show_module, false, l, &unsaved, &t, &me),
            Some(Line::OtherKinds) => {
                let e = me.clone();
                link_button("other-kinds", trf("%lld weitere Treffer in anderen Dateitypen", &[&self.results.other_kinds]), &t)
                    .py(px(space::XS))
                    .on_click(move |_, _, cx| {
                        e.update(cx, |b, cx| {
                            b.kind = FileKind::All;
                            b.reload(cx);
                        })
                    })
                    .into_any_element()
            }
            None => return div().into_any_element(),
        };
        div().w_full().px(px(space::L)).child(content).into_any_element()
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let command = k.modifiers.platform || k.modifiers.control;
        if command && !k.modifiers.alt {
            match k.key.as_str() {
                "a" => {
                    self.picked = self.visible(state::read(cx, current_week)).into_iter().collect();
                    cx.notify();
                }
                "backspace" | "delete" => self.trash_targets(cx),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if command || k.modifiers.alt {
            return;
        }
        match k.key.as_str() {
            "up" | "down" => {
                let start = self.range_start.clone().or_else(|| self.selected.clone());
                self.move_selection(if k.key == "up" { -1 } else { 1 }, cx);
                if let Some(to) = self.selected.clone() {
                    if k.modifiers.shift {
                        self.range_start = start;
                        self.pick_range(&to, state::read(cx, current_week));
                    } else {
                        self.pick_only(&to);
                    }
                }
            }
            "f2" => {
                if let Some(path) = self.selected.clone() {
                    self.start_rename(path, window, cx);
                }
            }
            "space" => {
                if let Some(path) = self.selected.clone() {
                    let files = self.visible(state::read(cx, current_week));
                    let index = files.iter().position(|f| *f == path).unwrap_or(0);
                    super::preview::show_in(files, index, cx);
                }
            }
            "enter" if self.renaming.is_some() || self.renamed_at.is_some_and(|at| at.elapsed() < Duration::from_millis(200)) => {}
            "enter" => self.open_selected(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

pub fn current_week(l: &Library) -> Option<u32> {
    l.current_week().map(|w| w.index).or_else(|| l.weeks().iter().rev().find(|w| w.monday <= l.today()).map(|w| w.index))
}

/// Newer files first; equally old ones by name, so equal results keep their order.
fn newest_first(a: &Item, b: &Item) -> std::cmp::Ordering {
    b.file.date.cmp(&a.file.date).then_with(|| crate::core::text::natural_cmp(a.file.name(), b.file.name()))
}

fn group(items: Vec<Item>, searching: bool, grouping: FileGrouping, today: NaiveDate, order: &HashMap<String, usize>, starts: &WeekStarts) -> Vec<Group> {
    match grouping {
        FileGrouping::Week => {
            let mut map: BTreeMap<(Option<NaiveDate>, i64), Vec<Item>> = BTreeMap::new();
            for i in items {
                map.entry((starts.monday(&i.file), i.file.week.map_or(-1, |w| w as i64))).or_default().push(i);
            }
            map.into_iter().rev().map(|((m, w), items)| Group { kind: GroupKind::Week((w >= 0).then_some(w as u32), m), items }).collect()
        }
        FileGrouping::Folder => {
            let mut map: HashMap<(Option<String>, Vec<String>), Vec<Item>> = HashMap::new();
            for i in items {
                map.entry((i.file.module_id.clone(), i.file.place.clone())).or_default().push(i);
            }
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort_by(|a, b| {
                let (ma, mb) = (a.0.as_ref().and_then(|m| order.get(m)).copied().unwrap_or(usize::MAX), b.0.as_ref().and_then(|m| order.get(m)).copied().unwrap_or(usize::MAX));
                ma.cmp(&mb).then_with(|| crate::core::text::natural_cmp(&a.1.join("/"), &b.1.join("/")))
            });
            keys.into_iter()
                .map(|k| {
                    let mut items = map.remove(&k).unwrap_or_default();
                    if !searching {
                        items.sort_by(newest_first);
                    }
                    Group { kind: GroupKind::Folder(k.0, k.1), items }
                })
                .collect()
        }
        FileGrouping::Date => {
            let monday = day::monday(today);
            let bucket = |d: NaiveDate| -> (u8, String) {
                if d >= today {
                    (0, tr("Heute"))
                } else if d >= today - Days::new(1) {
                    (1, tr("Gestern"))
                } else if d >= monday {
                    (2, tr("Diese Woche"))
                } else if d >= monday - Days::new(7) {
                    (3, tr("Letzte Woche"))
                } else if d >= today - Days::new(30) {
                    (4, tr("Letzte 30 Tage"))
                } else {
                    (5, tr("Älter"))
                }
            };
            let mut map: BTreeMap<u8, (String, Vec<Item>)> = BTreeMap::new();
            for i in items {
                let (k, label) = bucket(i.file.date);
                map.entry(k).or_insert_with(|| (label, Vec::new())).1.push(i);
            }
            map.into_values()
                .map(|(label, mut items)| {
                    if !searching {
                        items.sort_by(newest_first);
                    }
                    Group { kind: GroupKind::Date(label), items }
                })
                .collect()
        }
    }
}

/// A week as people say it: "Letzte Woche", then its days and its codes.
pub fn week_title(d: NaiveDate, lecture: Option<u32>, today: NaiveDate, t: &Theme) -> Div {
    let current = day::monday(d) == day::monday(today);
    div()
        .flex()
        .items_baseline()
        .gap(px(space::S))
        .min_w_0()
        .overflow_hidden()
        .child(div().flex_shrink_0().whitespace_nowrap().font_weight(FontWeight::SEMIBOLD).when(current, |x| x.text_color(t.accent_text)).child(day::week_relative(d, today)))
        .child(div().flex_shrink_0().whitespace_nowrap().text_color(t.secondary).child(day::week_range(d, today)))
        .child(div().truncate().text_size(px(font::CAPTION)).font_features(tabular()).text_color(t.secondary).child(day::week_codes(lecture, d)))
}

/// Text with every search term bold and tinted.
fn highlighted(s: &str, terms: &[String], t: &Theme) -> StyledText {
    let mut fchars: Vec<char> = Vec::with_capacity(s.len());
    let mut map: Vec<usize> = Vec::with_capacity(s.len());
    for (at, c) in s.char_indices() {
        for f in fold(c.encode_utf8(&mut [0; 4])).chars() {
            fchars.push(f);
            map.push(at);
        }
    }
    let end_of = |i: usize| map.get(i).copied().unwrap_or(s.len());
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    for term in terms.iter().filter(|t| t.chars().count() >= 2) {
        let tc: Vec<char> = fold(term).chars().collect();
        let mut i = 0;
        while i + tc.len() <= fchars.len() {
            if fchars[i..i + tc.len()] == tc[..] {
                let last = map[i + tc.len() - 1];
                let after = (i + tc.len()..=fchars.len()).map(end_of).find(|e| *e > last).unwrap_or(s.len());
                ranges.push(map[i]..after);
                i += tc.len();
            } else {
                i += 1;
            }
        }
    }
    ranges.sort_by_key(|r| r.start);
    ranges.dedup_by(|b, a| b.start < a.end);
    let style = HighlightStyle { color: Some(t.accent), font_weight: Some(FontWeight::BOLD), ..Default::default() };
    StyledText::new(s.to_string()).with_highlights(ranges.into_iter().map(|r| (r, style)))
}

impl FileBrowser {
    fn row(&self, item: &Item, show_module: bool, show_week: bool, lib: &Library, unsaved: &HashSet<PathBuf>, t: &Theme, entity: &Entity<Self>) -> AnyElement {
        let f = &item.file;
        let path = f.file.path.clone();
        let selected = self.selected.as_ref() == Some(&path) || self.picked.contains(&path);
        let renaming = self.renaming.as_ref().filter(|(p, _, _)| *p == path).map(|(_, field, _)| field.clone());
        let terms = self.results.query.terms.clone();
        let (sel, open, reveal, preview, menu_path) = (path.clone(), path.clone(), path.clone(), path.clone(), path.clone());
        let week_filter = self.week_filter;
        let same_semester = self.week_starts.in_shown(f);
        let module_filter = self.module_filter.clone();
        let scoped = self.scope_module.is_some();
        let f2 = f.clone();
        let (entity, click) = (entity.clone(), entity.clone());
        let mut meta = div().flex().items_center().gap(px(space::S)).text_size(px(font::CAPTION)).text_color(t.secondary);
        if show_module && let Some(m) = f.module_id.as_deref().and_then(|id| lib.module_by_id(id)) {
            meta = meta.child(module_tag(&m.badge(), t.palette(&m.color)));
        }
        if let (true, Some(w)) = (show_week, f.week) {
            meta = meta.child(self.week_starts.monday(f).map_or_else(|| format!("W{w:02}"), |m| day::week_relative(m, lib.today())));
        }
        if !f.place.is_empty() && (prefs::get().file_grouping != prefs::FileGrouping::Folder || !self.query.is_empty() || show_week) {
            meta = meta.child(div().truncate().child(f.place.join(" › ")));
        }
        if f.title.is_some() && !self.query.is_empty() {
            meta = meta.child(div().truncate().text_color(t.tertiary).child(f.name().to_string()));
        }
        if f.is_brief {
            meta = meta.child(div().flex().items_center().gap(px(space::XXS)).child(icon("clipboard-list", iconsize::XS, t.secondary)).child(tr("Aufgabe")));
        }
        let hover = t.ink.opacity(0.045);
        let first_spot = item.spots.first().cloned();
        div()
            .id(SharedString::from(format!("file-{}", path.display())))
            .group("file-row")
            .flex()
            .items_start()
            .gap(px(space::S))
            .py(px(space::XS))
            .px(px(space::S))
            .rounded(px(radius::INNER))
            .when(selected, |d| d.bg(if self.list_focused { t.accent.opacity(0.32) } else { t.ink.opacity(0.09) }))
            .when(!selected, |d| d.soft(hover, radius::INNER))
            .tooltip(tip(lib.relative_path(&path)))
            .on_click(move |e: &ClickEvent, window, cx| {
                let (toggle, extend) = (e.modifiers().platform || e.modifiers().control, e.modifiers().shift);
                let current = state::read(cx, current_week);
                click.update(cx, |this, cx| {
                    if toggle {
                        this.pick_toggle(&sel);
                    } else if extend {
                        this.pick_range(&sel, current);
                    } else {
                        this.pick_only(&sel);
                    }
                    window.focus(&this.focus, cx);
                    cx.notify();
                });
                if toggle || extend {
                    return;
                }
                if e.click_count() >= 2 {
                    open_at(&open, first_spot.clone(), cx);
                }
            })
            .on_drag(DraggedFile(path.clone()), |d, _, _, cx| cx.new(|_| d.clone()))
            .child(file_icon(&path, iconsize::L))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .child(match renaming {
                        Some(field) => Input::new(&field).small().into_any_element(),
                        None => {
                            let is_copy = f.display_name().contains(&format!("({} ", tr("Konflikt")));
                            let dot = unsaved.contains(&path);
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::XS))
                                .child(div().min_w_0().truncate().child(highlighted(&f.display_name(), &terms, t)))
                                .when(dot, |d| d.child(div().id("unsaved-dot").flex_none().size(px(6.)).rounded_full().bg(t.accent).tooltip(tip(tr("Noch nicht gesichert")))))
                                .when(is_copy, |d| d.child(div().flex_none().px(px(space::S)).rounded_full().bg(t.orange.opacity(0.18)).text_size(px(font::CAPTION)).text_color(t.orange).child(tr("Konflikt"))))
                                .into_any_element()
                        }
                    })
                    .child(meta)
                    .children(item.spots.iter().enumerate().map(|(i, spot)| {
                        let (file, spot_at) = (path.clone(), spot.clone());
                        div()
                            .id(("spot", i))
                            .px(px(space::XS))
                            .rounded(px(radius::SMALL))
                            .soft(hover, radius::SMALL)
                            .cursor_pointer()
                            .truncate()
                            .text_size(px(font::CAPTION))
                            .text_color(t.secondary)
                            .tooltip(tip(tr("Zur Fundstelle springen")))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                open_at(&file, Some(spot_at.clone()), cx);
                            })
                            .child(highlighted(&spot.excerpt, &terms, t))
                    })),
            )
            .child(
                div()
                    .relative()
                    .child(div().group_hover("file-row", Styled::invisible).text_size(px(font::CAPTION)).text_color(t.secondary).child(day::day_label(day::at(f.date, chrono::NaiveTime::MIN).unwrap_or(lib.now), lib.now)))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .flex()
                            .invisible()
                            .group_hover("file-row", |s| s.visible())
                            .child(Button::new(SharedString::from(format!("pv-{}", path.display()))).ghost().small().icon(menu_icon("eye")).tooltip(tr("Vorschau (Leertaste)")).on_click(move |_, _, cx| super::preview::show(&preview, cx)))
                            .child(Button::new(SharedString::from(format!("rv-{}", path.display()))).ghost().small().icon(menu_icon("folder")).tooltip(super::platform::reveal_label()).on_click(move |_, _, _| library::reveal(&reveal))),
                    ),
            )
            .context_menu(move |menu, window, cx| {
                let p = menu_path.clone();
                let (p1, p2, p3, p4) = (p.clone(), p.clone(), p.clone(), p.clone());
                let current = state::read(cx, current_week);
                let group: Vec<PathBuf> = entity.read(cx).targets(current).into_iter().filter(|_| entity.read(cx).picked.contains(&p)).collect();
                if group.len() > 1 {
                    let count = group.len();
                    return menu.item(PopupMenuItem::new(trf("%lld Dateien in den Papierkorb", &[&count])).on_click(move |_, _, cx| {
                        state::trash(group.clone(), cx);
                    }));
                }
                let module = f2.module_id.clone();
                let hand_over = p.clone();
                let menu = if super::vault_tools::in_vault(&p, cx) { menu.item(PopupMenuItem::new(tr("In Obsidian öffnen")).icon(menu_icon("external-link")).on_click(move |_, _, cx| super::vault_tools::open_in_obsidian(&hand_over, cx))) } else { menu };
                let mut menu = menu
                    .item(PopupMenuItem::new(tr("Öffnen")).on_click(move |_, _, cx| act(cx, |l| l.open_file(&p1))))
                    .item(PopupMenuItem::new(tr("Vorschau")).on_click(move |_, _, cx| super::preview::show(&p2, cx)))
                    .item(PopupMenuItem::new(super::platform::reveal_label()).on_click(move |_, _, _| library::reveal(&p3)))
                    .item(PopupMenuItem::new(tr("Neu in diesem Ordner…")).on_click(move |_, _, cx| {
                        let (m, folder) = (module.clone(), p4.parent().map(PathBuf::from));
                        act(cx, |l| l.new_note = Some(NewNoteRequest { module_id: m, folder }))
                    }));
                if markdown::is_markdown(&p) {
                    let (n, e) = (p.clone(), p.clone());
                    menu = menu
                        .item(PopupMenuItem::new(tr("Als Notiz öffnen")).on_click(move |_, _, cx| super::note_window::open(&n, cx)))
                        .item(PopupMenuItem::new(tr("Als PDF exportieren")).on_click(move |_, window, cx| super::pdf::export(&e, None, window, cx)));
                }
                if let Some(w) = f2.week.filter(|w| week_filter != Some(*w) && same_semester) {
                    let e = entity.clone();
                    menu = menu.separator().item(PopupMenuItem::new(tr("Alles aus derselben Woche zeigen")).on_click(move |_, _, cx| e.update(cx, |b, cx| {
                        b.week_filter = Some(w);
                        b.reload(cx);
                    })));
                }
                if let Some(id) = f2.module_id.clone().filter(|id| !scoped && module_filter.as_ref() != Some(id)) {
                    let e = entity.clone();
                    let code = id.clone();
                    menu = menu.item(PopupMenuItem::new(trf("Nur %@ zeigen", &[&code])).on_click(move |_, _, cx| e.update(cx, |b, cx| {
                        b.module_filter = Some(id.clone());
                        b.reload(cx);
                    })));
                }
                let (rename, trash_path, e) = (p.clone(), p.clone(), entity.clone());
                deadline_submenu(menu.separator(), &p, window, cx)
                    .separator()
                    .item(PopupMenuItem::new(tr("Umbenennen…")).on_click(move |_, window, cx| e.update(cx, |b, cx| b.start_rename(rename.clone(), window, cx))))
                    .when(super::convention_rename::available(&p, cx), |m| {
                        let n = p.clone();
                        m.item(PopupMenuItem::new(tr("Nach Konvention benennen…")).icon(menu_icon("wand-sparkles")).on_click(move |_, window, cx| super::convention_rename::open(n.clone(), window, cx)))
                    })
                    .item(PopupMenuItem::new(tr("In den Papierkorb")).on_click(move |_, _, cx| state::trash(vec![trash_path.clone()], cx)))
            })
            .into_any_element()
    }

    fn group_header(&self, g: &Group, expanded: bool, lib: &Library, t: &Theme, entity: &Entity<Self>) -> AnyElement {
        let entity = entity.clone();
        let id = g.id();
        let mut title = div().flex().items_center().gap(px(space::S)).min_w_0();
        match &g.kind {
            GroupKind::Week(Some(w), Some(m)) => title = title.child(week_title(*m, Some(*w), lib.today(), t)),
            GroupKind::Week(Some(w), None) => title = title.child(div().font_weight(FontWeight::SEMIBOLD).child(format!("W{w:02}"))),
            GroupKind::Week(None, _) => title = title.child(div().font_weight(FontWeight::SEMIBOLD).child(tr("Ohne Woche"))),
            GroupKind::Folder(m, place) => {
                if let Some(m) = m.as_deref().and_then(|id| lib.module_by_id(id)).filter(|_| self.scope_module.is_none()) {
                    title = title.child(module_chip(&m.badge(), t.palette(&m.color)));
                }
                title = title.child(div().truncate().font_weight(FontWeight::SEMIBOLD).child(if place.is_empty() { tr("Hauptordner") } else { place.join(" › ") }));
            }
            GroupKind::Date(label) => title = title.child(div().font_weight(FontWeight::SEMIBOLD).child(label.clone())),
        }
        div()
            .id(SharedString::from(format!("group-{id}")))
            .flex()
            .items_center()
            .gap(px(space::S))
            .py(px(space::XS))
            .px(px(space::S))
            .mt(px(space::S))
            .rounded(px(radius::INNER))
            .cursor_pointer().key_nav()
            .soft(t.ink.opacity(0.05), radius::INNER)
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    if !this.toggled.remove(&id) {
                        this.toggled.insert(id.clone());
                    }
                    cx.notify();
                })
            })
            .child(motion::chevron("chevron", expanded, t.tertiary))
            .child(title)
            .child(div().flex_1())
            .child(div().text_color(t.tertiary).child(g.items.len().to_string()))
            .into_any_element()
    }

    fn filter_bar(&self, lib: &Library, current: Option<u32>, t: &Theme, entity: &Entity<Self>) -> Div {
        let e = entity.clone();
        let kinds = div().flex().rounded(px(radius::INNER)).bg(t.ink.opacity(0.06)).p(px(space::XXS)).children(FileKind::ALL.into_iter().map(|k| {
            let active = k == self.kind;
            let e = e.clone();
            div()
                .id(SharedString::from(format!("kind-{}", k.key())))
                .px(px(space::S))
                .py(px(space::XXS))
                .rounded(px(radius::SMALL))
                .text_size(px(font::CALLOUT))
                .cursor_pointer().key_nav()
                .when(!active, |d| d.soft(t.ink.opacity(0.06), radius::SMALL))
                .when(active, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm().font_weight(FontWeight::MEDIUM))
                .child(match k {
                    FileKind::Documents => tr("Dokumente"),
                    FileKind::Notes => tr("Notizen"),
                    FileKind::Pdf => "PDF".into(),
                    FileKind::Office => "Office".into(),
                    FileKind::Image => tr("Bilder"),
                    FileKind::All => tr("Alle"),
                })
                .on_click(move |_, _, cx| {
                    prefs::update(|p| p.file_kind = k.key().into());
                    e.update(cx, |b, cx| {
                        b.kind = k;
                        b.reload(cx);
                    })
                })
        }));
        let mut bar = div().flex().flex_wrap().items_center().gap(px(space::S)).px(px(space::L)).py(px(space::S)).min_h(px(control::PANE_HEAD)).border_b_1().border_color(t.separator).child(kinds);
        if self.scope_module.is_none() {
            let typed = self.results.query.module_id.clone();
            let active = typed.clone().or(self.module_filter.clone());
            let title = active.as_deref().and_then(|id| lib.module_by_id(id)).map_or_else(|| tr("Alle Module"), |m| m.code.clone());
            let modules: Vec<(String, String)> = lib.semester_modules().iter().map(|m| (m.id.clone(), m.code.clone())).collect();
            bar = bar.child(filter_menu("module-filter", title, active.is_some(), typed.is_some(), e.clone(), move |b| b.module_filter = None, move |menu, e| {
                let active = active.clone();
                let mut menu = menu.item(PopupMenuItem::new(tr("Alle Module")).checked(active.is_none()).on_click({
                    let e = e.clone();
                    move |_, _, cx| e.update(cx, |b, cx| {
                        b.module_filter = None;
                        b.reload(cx);
                    })
                }));
                menu = menu.separator();
                for (id, code) in &modules {
                    let (e, id) = (e.clone(), id.clone());
                    let checked = active.as_ref() == Some(&id);
                    menu = menu.item(PopupMenuItem::new(code.clone()).checked(checked).on_click(move |_, _, cx| e.update(cx, |b, cx| {
                        b.module_filter = Some(id.clone());
                        b.reload(cx);
                    })));
                }
                menu
            }, t));
        }
        if let Some(m) = self.scope_module.as_deref().and_then(|id| lib.module_by_id(id)) {
            bar = bar.child(
                div()
                    .id("search-scope")
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .h(px(control::S))
                    .pl(px(space::S))
                    .pr(px(space::XS))
                    .rounded_full()
                    .bg(t.accent.opacity(0.14))
                    .text_size(px(font::CALLOUT))
                    .text_color(t.accent_text)
                    .cursor_pointer()
                    .key_nav()
                    .soft(t.accent.opacity(0.12), radius::FULL)
                    .tooltip(tip(tr("Überall suchen")))
                    .child(trf("In %@", &[&m.code]))
                    .child(icon("x", iconsize::XS, t.accent_text))
                    .on_click(|_, _, cx| act(cx, Library::widen_module_search)),
            );
        }
        if vault_query::is_structured(&self.query) {
            let sorts = [(vault_query::Sort::NameAsc, tr("Name A–Z")), (vault_query::Sort::NameDesc, tr("Name Z–A")), (vault_query::Sort::ModifiedNew, tr("Zuletzt geändert")), (vault_query::Sort::ModifiedOld, tr("Am längsten unverändert")), (vault_query::Sort::CreatedNew, tr("Zuletzt erstellt")), (vault_query::Sort::CreatedOld, tr("Zuerst erstellt"))];
            let label = sorts.iter().find(|(s, _)| *s == self.vault_sort).map_or_else(String::new, |(_, l)| l.clone());
            let e2 = e.clone();
            bar = bar.child(Button::new("vault-sort").ghost().small().icon(menu_icon("arrow-up-down")).label(label).dropdown_menu(move |mut menu, _, _| {
                for (sort, name) in sorts.clone() {
                    let e = e2.clone();
                    menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| e.update(cx, |b, cx| {
                        b.vault_sort = sort;
                        b.key.clear();
                        b.reload(cx);
                    })));
                }
                menu
            }));
        }
        bar = bar.child(Button::new("vault-operators").ghost().small().icon(menu_icon("circle-help")).tooltip(tr("Suchbefehle")).dropdown_menu(|mut menu, _, _| {
            for op in ["file:", "path:", "content:", "tag:", "line:()", "block:()", "section:()", "task:", "task-todo:", "task-done:", "match-case:", "ignore-case:", "[property]", "[property:wert]", "/regex/", "-ausschließen", "OR"] {
                menu = menu.item(PopupMenuItem::new(op.to_string()).on_click(move |_, _, cx| {
                    act(cx, |l| {
                        let spacer = if l.search_query.is_empty() || l.search_query.ends_with(' ') { "" } else { " " };
                        l.search_query = format!("{}{spacer}{op}", l.search_query);
                    });
                }));
            }
            menu
        }));
        let typed_week = self.results.query.week;
        let active_week = typed_week.or(self.week_filter);
        let today = lib.today();
        let all_weeks = lib.weeks();
        let weeks: Vec<(u32, NaiveDate)> = all_weeks.iter().filter(|w| current.is_none_or(|c| w.index <= c)).rev().map(|w| (w.index, w.monday)).collect();
        let week_name = |w: u32| all_weeks.iter().find(|x| x.index == w).map_or_else(|| format!("W{w:02}"), |x| day::week_relative(x.monday, today));
        let (secondary, accent) = (t.secondary, t.accent_text);
        bar = bar.child(filter_menu("week-filter", active_week.map_or_else(|| tr("Alle Wochen"), week_name), active_week.is_some(), typed_week.is_some(), e.clone(), |b| b.week_filter = None, move |menu, e| {
            let set = |w: Option<u32>| {
                let e = e.clone();
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| e.update(cx, |b, cx| {
                    b.week_filter = w;
                    b.reload(cx);
                })
            };
            let mut menu = menu.item(PopupMenuItem::new(tr("Alle Wochen")).checked(active_week.is_none()).on_click(set(None))).separator();
            for (w, monday) in &weeks {
                let (w, monday) = (*w, *monday);
                let current = day::monday(monday) == day::monday(today);
                let (name, range, codes) = (day::week_relative(monday, today), day::week_range(monday, today), day::week_codes(Some(w), monday));
                menu = menu.item(PopupMenuItem::element(move |_, _| {
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(space::L))
                        .w_full()
                        .child(div().flex_1().whitespace_nowrap().when(current, |x| x.text_color(accent)).child(name.clone()))
                        .child(div().whitespace_nowrap().text_color(secondary).child(range.clone()))
                        .child(div().whitespace_nowrap().text_size(px(font::CAPTION)).font_features(tabular()).text_color(secondary).child(codes.clone()))
                }).checked(active_week == Some(w)).on_click(set(Some(w))));
            }
            menu
        }, t));
        let grouping = prefs::get().file_grouping;
        let label = |g: FileGrouping| match g {
            FileGrouping::Week => tr("Nach Woche"),
            FileGrouping::Folder => tr("Nach Ordner"),
            FileGrouping::Date => tr("Nach Datum"),
        };
        let e2 = e.clone();
        bar.child(div().flex_1())
            .child(Button::new("grouping").ghost().small().icon(menu_icon("layers")).label(label(grouping)).tooltip(tr("Gruppierung")).dropdown_menu(move |mut menu, _, _| {
                for g in [FileGrouping::Week, FileGrouping::Folder, FileGrouping::Date] {
                    let e = e2.clone();
                    menu = menu.item(PopupMenuItem::new(label(g)).checked(g == grouping).on_click(move |_, _, cx| {
                        prefs::update(|p| p.file_grouping = g);
                        e.update(cx, |b, cx| {
                            b.toggled.clear();
                            b.reload(cx);
                        })
                    }));
                }
                menu
            }))
            .when(lib.is_indexing, |d| d.child(div().id("indexing").child(motion::spinner(iconsize::S, t.secondary)).tooltip(tip(tr("Index wird aktualisiert")))))
    }
}

/// Opens a file, at a search match where the viewer can show one.
fn open_at(path: &std::path::Path, spot: Option<Spot>, cx: &mut App) {
    let jumps = markdown::is_markdown(path) || extension(&path.to_string_lossy()).eq_ignore_ascii_case("pdf");
    act(cx, |l| {
        if let Some(spot) = spot.filter(|_| jumps) {
            l.find_in_file = Some((path.to_path_buf(), spot.term, spot.nth));
        }
        l.open_file(path)
    });
}

/// A file dragged out of a list, e.g. onto an assignment.
#[derive(Clone)]
pub struct DraggedFile(pub PathBuf);

impl Render for DraggedFile {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        div().flex().items_center().gap(px(space::S)).px(px(space::S)).py(px(space::XS)).rounded(px(radius::INNER)).bg(t.material).text_color(t.primary).shadow_md().child(file_icon(&self.0, iconsize::M)).child(self.0.file_name().unwrap_or_default().to_string_lossy().to_string())
    }
}

#[allow(clippy::too_many_arguments)]
fn filter_menu(
    id: &'static str,
    title: String,
    active: bool,
    locked: bool,
    entity: Entity<FileBrowser>,
    clear: impl Fn(&mut FileBrowser) + 'static,
    items: impl Fn(gpui_kit::component::menu::PopupMenu, Entity<FileBrowser>) -> gpui_kit::component::menu::PopupMenu + 'static,
    t: &Theme,
) -> AnyElement {
    if locked {
        return div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(space::XS))
            .px(px(space::S))
            .py(px(space::XS))
            .rounded(px(radius::INNER))
            .bg(t.accent.opacity(0.18))
            .text_color(t.accent_text)
            .text_size(px(font::CALLOUT))
            .font_weight(FontWeight::MEDIUM)
            .child(icon("search", iconsize::XS, t.accent))
            .child(title)
            .tooltip(tip(tr("Aus der Suche übernommen")))
            .into_any_element();
    }
    let e = entity.clone();
    let mut b = Button::new(id).small().label(title).dropdown_caret(true);
    if active {
        b = b.primary();
    }
    div()
        .flex()
        .items_center()
        .gap(px(space::XXS))
        .child(b.dropdown_menu(move |menu, _, _| items(menu, e.clone())))
        .when(active, |d| {
            d.child(Button::new(SharedString::from(format!("{id}-clear"))).ghost().small().icon(menu_icon("circle-x")).tooltip(tr("Filter entfernen")).on_click(move |_, _, cx| entity.update(cx, |b, cx| {
                clear(b);
                b.reload(cx);
            })))
        })
        .into_any_element()
}

impl Render for FileBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.list_focused = self.focus.is_focused(window);
        let t = Theme::get(cx);
        let me = cx.entity();
        let lib = state::lib(cx);
        let l = lib.read(cx);
        let current = current_week(l);
        let bar = self.filter_bar(l, current, &t, &me);
        let query_error = self.query_error.clone().map(|e| div().px(px(space::L)).py(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.red).child(e));
        let lines = self.line_list(current);
        if lines != self.lines {
            self.list.reset(lines.len());
            self.lines = lines;
        }
        if std::mem::take(&mut self.results_changed) {
            self.list.scroll_to(ListOffset::default());
        }
        let empty = self.results.loaded && self.results.total == 0;
        let body: AnyElement = if empty {
            if self.results.searching || self.module_filter.is_some() || self.week_filter.is_some() || self.results.query.week.is_some() {
                let e = me.clone();
                let filtered = self.module_filter.is_some() || self.week_filter.is_some() || self.kind != FileKind::All;
                unavailable(
                    "search",
                    if self.query.is_empty() { tr("Keine Dateien") } else { trf("Keine Treffer für „%@“", &[&self.query]) },
                    tr("Gesucht wird in Namen, Ordnern, PDFs und Notizen. Tipp: „w3“ oder „kw44“ filtert nach Woche."),
                    &t,
                )
                .when(self.scope_module.is_some() && self.results.searching, |d| d.child(Button::new("search-everywhere").label(tr("Überall suchen")).on_click(|_, _, cx| act(cx, Library::widen_module_search))))
                .when(filtered, |d| {
                    d.child(Button::new("reset-filters").label(tr("Filter zurücksetzen")).on_click(move |_, _, cx| e.update(cx, |b, cx| {
                        b.module_filter = None;
                        b.week_filter = None;
                        b.kind = FileKind::All;
                        b.reload(cx);
                    })))
                })
                .into_any_element()
            } else if l.index_count == 0 {
                unavailable("file-search", if l.is_indexing { tr("Indexiere Dateien …") } else { tr("Keine Dateien") }, "", &t).into_any_element()
            } else {
                unavailable("folder", tr("Noch keine Dokumente"), tr("Abgelegte Dateien erscheinen hier, nach Woche sortiert."), &t).into_any_element()
            }
        } else {
            div().id("browser-list").flex_1().min_h_0().pt(px(space::S)).track_focus(&self.focus).on_key_down(cx.listener(Self::on_key)).child(list(self.list.clone(), cx.processor(|this, ix, _, cx| this.render_line(ix, cx))).size_full()).vertical_scrollbar(&self.list).into_any_element()
        };
        let count = self.picked.len();
        let e = me.clone();
        let footer = (count > 1).then(|| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(space::S))
                .px(px(space::M))
                .py(px(space::S))
                .border_t_1()
                .border_color(t.separator)
                .child(caption(trf("%lld ausgewählt", &[&count]), &t))
                .child(div().flex_1())
                .child(Button::new("browser-trash").small().ghost().icon(menu_icon("trash-2")).label(tr("In den Papierkorb")).on_click(move |_, _, cx| e.update(cx, |b, cx| b.trash_targets(cx))))
        });
        div().size_full().flex().flex_col().child(bar).children(query_error).child(div().flex_1().min_h_0().flex().flex_col().child(body)).children(footer)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use chrono::NaiveDate;

    use super::{CatalogFile, FileGrouping, GroupKind, Item, LectureWeek, WeekStarts, group};

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn file(module: Option<&str>, week: Option<u32>) -> CatalogFile {
        CatalogFile {
            file: crate::core::search_index::IndexedFile { path: PathBuf::from("x.md"), name: "x.md".into(), modified: chrono::Local::now() },
            module_id: module.map(String::from),
            week,
            place: Vec::new(),
            date: ymd(2026, 11, 3),
            is_brief: false,
            in_project: false,
            haystack: String::new(),
            folded_name: String::new(),
            title: None,
        }
    }

    fn starts() -> WeekStarts {
        let ws26 = vec![LectureWeek { index: 3, monday: ymd(2026, 10, 19), iso_week: 43 }];
        let ws25 = vec![LectureWeek { index: 3, monday: ymd(2025, 10, 20), iso_week: 43 }];
        WeekStarts {
            by_semester: HashMap::from([("ws26".to_string(), ws26.clone()), ("ws25".to_string(), ws25)]),
            semester_of: HashMap::from([("now".to_string(), "ws26".to_string()), ("old".to_string(), "ws25".to_string())]),
            shown: ws26,
            today: ymd(2026, 11, 3),
        }
    }

    #[test]
    fn dates_a_week_in_the_semester_of_its_own_module() {
        let s = starts();
        assert_eq!(s.monday(&file(Some("now"), Some(3))), Some(ymd(2026, 10, 19)));
        assert_eq!(s.monday(&file(Some("old"), Some(3))), Some(ymd(2025, 10, 20)));
        assert_eq!(s.monday(&file(None, Some(3))), Some(ymd(2026, 10, 19)));
        assert_eq!(s.monday(&file(Some("now"), Some(9))), None);
        assert!(s.in_shown(&file(Some("now"), Some(3))));
        assert!(!s.in_shown(&file(Some("old"), Some(3))));
        assert!(!s.in_shown(&file(Some("now"), None)));
    }

    #[test]
    fn groups_the_same_week_number_of_two_semesters_apart() {
        let s = starts();
        let item = |m: &str| Item { file: file(Some(m), Some(3)), spots: Vec::new(), score: 0. };
        let groups = group(vec![item("old"), item("now")], false, FileGrouping::Week, s.today, &HashMap::new(), &s);
        let kinds: Vec<GroupKind> = groups.into_iter().map(|g| g.kind).collect();
        assert!(kinds == vec![GroupKind::Week(Some(3), Some(ymd(2026, 10, 19))), GroupKind::Week(Some(3), Some(ymd(2025, 10, 20)))]);
    }
}
