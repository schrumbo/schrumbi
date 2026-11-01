use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::markdown_view::{self, ParsedNote};
use super::motion::{Soft, chevron};
use super::state::{self, act};
use super::theme::{Theme, control, font, iconsize, note_size, radius, space};
use super::widgets::*;
use crate::core::cards::fsrs::{self, Rating};
use crate::core::cards::{self, Card, CardKind, Counts, Phase};
use crate::core::i18n::{tr, trf};
use crate::core::text::fold;
use crate::core::vault::index;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Filter {
    All,
    New,
    Learning,
    Due,
    Suspended,
    Hard,
}

impl Filter {
    const ALL: [Filter; 6] = [Filter::All, Filter::New, Filter::Learning, Filter::Due, Filter::Hard, Filter::Suspended];

    fn label(self) -> String {
        match self {
            Filter::All => tr("Alle"),
            Filter::New => tr("Neu"),
            Filter::Learning => tr("Lernen"),
            Filter::Due => tr("Fällig"),
            Filter::Hard => tr("Schwierig"),
            Filter::Suspended => tr("Pausiert"),
        }
    }

    fn keeps(self, card: &Card, now: i64) -> bool {
        match self {
            Filter::All => true,
            Filter::New => card.phase() == Phase::New,
            Filter::Learning => matches!(card.phase(), Phase::Learning | Phase::Relearning),
            Filter::Due => !card.suspended() && card.phase() != Phase::New && card.state.as_ref().is_some_and(|s| s.due < cards::next_day_start(now)),
            Filter::Hard => card.is_leech(),
            Filter::Suspended => card.suspended(),
        }
    }
}

/// One line of the deck tree.
#[derive(Clone, PartialEq)]
struct DeckRow {
    deck: String,
    name: String,
    depth: usize,
    open: bool,
    has_children: bool,
    total: usize,
    module: Option<(String, String)>,
}

/// The card shown in a learning session and how the session went so far.
struct Session {
    deck: String,
    current: Option<Card>,
    front: Arc<ParsedNote>,
    back: Arc<ParsedNote>,
    shown: bool,
    answered: Vec<cards::Answered>,
    ratings: [usize; 4],
    started: i64,
    left: Counts,
}

/// The flashcards room: decks on the left, a deck's state and cards on the right, or a learning session.
pub struct CardsPage {
    deck: String,
    open: BTreeSet<String>,
    session: Option<Session>,
    query: Entity<InputState>,
    filter: Filter,
    selected: Option<String>,
    preview: Option<(String, Arc<ParsedNote>, Arc<ParsedNote>)>,
    focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    cached: Option<((u64, i64, String, Filter, String), Rc<Vec<usize>>)>,
    all: Arc<Vec<Card>>,
    _subs: Vec<Subscription>,
}

fn parsed(text: &str) -> Arc<ParsedNote> {
    Arc::new(markdown_view::parse(text))
}

/// Which module a deck's folder is.
fn module_of(folder: &str, cx: &App) -> Option<(String, String)> {
    state::read(cx, |l| l.config.as_ref().and_then(|c| c.modules.iter().find(|m| m.folder == folder).map(|m| (m.badge(), m.color.clone()))))
}

fn deck_title(deck: &str, cx: &App) -> String {
    if deck.is_empty() {
        return tr("Alle Karten");
    }
    if let Some(name) = state::read(cx, |l| l.config.as_ref().and_then(|c| c.modules.iter().find(|m| m.folder == deck).map(|m| m.name.clone()))) {
        return name;
    }
    let name = deck.rsplit('/').next().unwrap_or(deck);
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

fn deck_rows(all: &[Card], open: &BTreeSet<String>, cx: &App) -> Vec<DeckRow> {
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for card in all {
        let mut parent = String::new();
        for part in card.note_rel.split('/') {
            let here = if parent.is_empty() { part.to_string() } else { format!("{parent}/{part}") };
            *totals.entry(here.clone()).or_default() += 1;
            children.entry(parent.clone()).or_default().insert(here.clone());
            parent = here;
        }
    }
    fn walk(parent: &str, depth: usize, children: &BTreeMap<String, BTreeSet<String>>, totals: &BTreeMap<String, usize>, open: &BTreeSet<String>, cx: &App, out: &mut Vec<DeckRow>) {
        let Some(list) = children.get(parent) else { return };
        let mut sorted: Vec<&String> = list.iter().collect();
        sorted.sort_by_key(|d| (d.ends_with(".md"), d.to_lowercase()));
        for deck in sorted {
            let is_open = open.contains(deck);
            let name = deck.rsplit('/').next().unwrap_or(deck);
            let module = module_of(deck, cx);
            let shown = if module.is_some() { deck_title(deck, cx) } else { name.strip_suffix(".md").unwrap_or(name).to_string() };
            out.push(DeckRow { deck: deck.clone(), name: shown, depth, open: is_open, has_children: children.contains_key(deck), total: totals.get(deck).copied().unwrap_or(0), module });
            if is_open {
                walk(deck, depth + 1, children, totals, open, cx, out);
            }
        }
    }
    let mut out = Vec::new();
    walk("", 0, &children, &totals, open, cx, &mut out);
    out
}

fn counts_pills(counts: Counts, t: &Theme) -> Div {
    let piece = |n: usize, color: Hsla| (n > 0).then(|| div().text_color(color).font_features(tabular()).child(n.to_string()));
    div().flex_none().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).children(piece(counts.new, t.blue)).children(piece(counts.learning, t.orange)).children(piece(counts.review, t.green))
}

fn rating_name(rating: Rating) -> String {
    match rating {
        Rating::Again => tr("Nochmal"),
        Rating::Hard => tr("Schwer"),
        Rating::Good => tr("Gut"),
        Rating::Easy => tr("Einfach"),
    }
}

impl CardsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Karten durchsuchen")));
        let subs = vec![
            cx.subscribe(&query, |_, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.observe(&state::lib(cx), |_, _, cx| cx.notify()),
        ];
        Self { deck: String::new(), open: BTreeSet::new(), session: None, query, filter: Filter::All, selected: None, preview: None, focus: cx.focus_handle(), list_scroll: UniformListScrollHandle::new(), cached: None, all: Arc::new(Vec::new()), _subs: subs }
    }

    /// Takes the keyboard, so the keys of the page work right away.
    pub fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn select_deck(&mut self, deck: String, cx: &mut Context<Self>) {
        self.deck = deck;
        self.selected = None;
        self.preview = None;
        cx.notify();
    }

    fn start(&mut self, deck: String, window: &mut Window, cx: &mut Context<Self>) {
        let mut session = Session { deck, current: None, front: parsed(""), back: parsed(""), shown: false, answered: Vec::new(), ratings: [0; 4], started: cards::now(), left: Counts::default() };
        Self::advance(&mut session, None);
        self.session = Some(session);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Moves on to whatever is due now; the card just answered waits unless nothing else is left.
    fn advance(session: &mut Session, last: Option<&str>) {
        let now = cards::now();
        let queue = cards::queue(&session.deck, now);
        session.left = cards::counts(&session.deck, now);
        let due_now = queue.iter().find(|c| Some(c.id()) != last && c.state.as_ref().is_none_or(|s| s.due <= now || c.phase() == Phase::Review));
        let next = due_now.or_else(|| queue.iter().find(|c| Some(c.id()) != last)).or_else(|| queue.first()).cloned();
        session.shown = false;
        if let Some(card) = &next {
            session.front = parsed(&card.raw.front);
            session.back = parsed(&card.raw.back);
        }
        session.current = next;
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = &mut self.session {
            s.shown = true;
            cx.notify();
        }
    }

    fn rate(&mut self, rating: Rating, cx: &mut Context<Self>) {
        let Some(s) = &mut self.session else { return };
        let Some(card) = s.current.clone().filter(|_| s.shown) else { return };
        let done = cards::answer(&card, rating, cards::now());
        s.answered.push(done);
        s.ratings[rating as usize - 1] += 1;
        Self::advance(s, Some(card.id()));
        state::lib(cx).update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &mut self.session else { return };
        let Some(done) = s.answered.pop() else { return };
        let id = done.id.clone();
        cards::undo(done);
        let card = cards::all_cards().iter().find(|c| c.id() == id).cloned();
        if let Some(card) = card {
            s.front = parsed(&card.raw.front);
            s.back = parsed(&card.raw.back);
            s.current = Some(card);
            s.shown = true;
        }
        s.left = cards::counts(&s.deck, cards::now());
        state::lib(cx).update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn set_aside(&mut self, suspend: bool, cx: &mut Context<Self>) {
        let Some(s) = &mut self.session else { return };
        let Some(card) = s.current.clone() else { return };
        if suspend {
            cards::set_suspended(&card, true);
        } else {
            cards::bury(&card, cards::now());
        }
        Self::advance(s, Some(card.id()));
        state::lib(cx).update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn open_source(card: &Card, cx: &mut App) {
        super::doc_side::open_at(&card.note, card.raw.lines.0, cx);
    }

    fn compose(&mut self, editing: Option<Card>, window: &mut Window, cx: &mut Context<Self>) {
        let deck = self.deck.clone();
        super::sheets::show_sheet(760., window, cx, move |window, cx| CardComposer::new(deck, editing, None, window, cx));
    }

    fn select_card(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(card) = self.all.iter().find(|c| c.id() == id) {
            self.preview = Some((id.clone(), parsed(&card.raw.front), parsed(&card.raw.back)));
        }
        self.selected = Some(id);
        cx.notify();
    }

    fn visible(&mut self, now: i64, cx: &App) -> Rc<Vec<usize>> {
        let query = fold(self.query.read(cx).value().trim());
        let key = (index::current().generation, now / 60, self.deck.clone(), self.filter, query.clone());
        if let Some((k, rows)) = &self.cached
            && *k == key
            && !self.all.is_empty()
        {
            return rows.clone();
        }
        let rows: Vec<usize> = self.all.iter().enumerate().filter(|(_, c)| c.in_deck(&self.deck) && self.filter.keeps(c, now) && (query.is_empty() || fold(&c.raw.front).contains(&query) || fold(&c.raw.back).contains(&query))).map(|(i, _)| i).collect();
        let rows = Rc::new(rows);
        self.cached = Some((key, rows.clone()));
        rows
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let key = e.keystroke.key.as_str();
        let m = &e.keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        if self.session.is_some() {
            let shown = self.session.as_ref().is_some_and(|s| s.shown);
            let has_card = self.session.as_ref().is_some_and(|s| s.current.is_some());
            match key {
                "space" | "enter" if has_card && !shown => self.reveal(cx),
                "space" | "enter" if has_card => self.rate(Rating::Good, cx),
                "1" if shown => self.rate(Rating::Again, cx),
                "2" if shown => self.rate(Rating::Hard, cx),
                "3" if shown => self.rate(Rating::Good, cx),
                "4" if shown => self.rate(Rating::Easy, cx),
                "escape" | "enter" | "space" => {
                    self.session = None;
                    self.cached = None;
                    cx.notify();
                }
                "e" => {
                    if let Some(card) = self.session.as_ref().and_then(|s| s.current.clone()) {
                        self.compose(Some(card), window, cx);
                    }
                }
                "s" if has_card => self.set_aside(true, cx),
                "b" if has_card => self.set_aside(false, cx),
                "o" => {
                    if let Some(card) = self.session.as_ref().and_then(|s| s.current.clone()) {
                        Self::open_source(&card, cx);
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        let now = cards::now();
        match key {
            "enter" | "space" if self.selected.is_none() => {
                if cards::counts(&self.deck, now).total() > 0 {
                    let deck = self.deck.clone();
                    self.start(deck, window, cx);
                }
            }
            "enter" => {
                let card = self.selected.as_ref().and_then(|id| self.all.iter().find(|c| c.id() == id)).cloned();
                self.compose(card, window, cx);
            }
            "n" => self.compose(None, window, cx),
            "/" => self.query.update(cx, |q, cx| q.focus(window, cx)),
            "escape" if self.selected.is_some() => {
                self.selected = None;
                self.preview = None;
                cx.notify();
            }
            "down" | "up" => {
                let rows = self.visible(now, cx);
                if rows.is_empty() {
                    return;
                }
                let at = self.selected.as_ref().and_then(|id| rows.iter().position(|i| self.all[*i].id() == id));
                let next = match (at, key) {
                    (Some(i), "down") => (i + 1).min(rows.len() - 1),
                    (Some(i), _) => i.saturating_sub(1),
                    (None, _) => 0,
                };
                let id = self.all[rows[next]].id().to_string();
                self.list_scroll.scroll_to_item(next, ScrollStrategy::Center);
                self.select_card(id, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn deck_column(&mut self, now: i64, t: &Theme, cx: &mut Context<Self>) -> Div {
        let rows = deck_rows(&self.all, &self.open, cx);
        let me = cx.entity();
        let current = self.deck.clone();
        let all_counts = cards::counts("", now);
        let hover = t.ink.opacity(0.05);
        let total = self.all.len();
        let head = div()
            .id("deck-all")
            .h(px(control::M + space::XS))
            .flex_none()
            .px(px(space::M))
            .flex()
            .items_center()
            .gap(px(space::S))
            .cursor_pointer()
            .text_size(px(font::CALLOUT))
            .font_weight(FontWeight::MEDIUM)
            .when(current.is_empty(), |d| d.bg(t.ink.opacity(0.08)))
            .when(!current.is_empty(), |d| d.soft(hover, 0.))
            .child(icon("layers", iconsize::S, t.secondary))
            .child(div().flex_1().min_w_0().truncate().child(tr("Alle Karten")))
            .child(counts_pills(all_counts, t))
            .tooltip(tip(trf("%@ Karten", &[&total.to_string()])))
            .on_click(cx.listener(|p, _, _, cx| p.select_deck(String::new(), cx)));
        let list = Rc::new(rows);
        let count = list.len();
        let lines = list.clone();
        let tree = uniform_list("deck-tree", count, move |range, _, cx| {
            let t = Theme::get(cx);
            let hover = t.ink.opacity(0.05);
            let now = cards::now();
            range
                .map(|i| {
                    let row = &lines[i];
                    let active = me.read(cx).deck == row.deck;
                    let (a, b, deck_a, deck_b, deck_c, deck_d, m_c, m_d) = (me.clone(), me.clone(), row.deck.clone(), row.deck.clone(), row.deck.clone(), row.deck.clone(), me.clone(), me.clone());
                    let chip: AnyElement = match &row.module {
                        Some((badge, color)) => {
                            let tint = t.palette(color);
                            div().flex_none().w(px(control::S)).h(px(iconsize::M)).rounded(px(radius::SMALL)).bg(tint.opacity(0.22)).flex().items_center().justify_center().text_size(px(font::CAPTION2)).font_weight(FontWeight::BOLD).text_color(t.readable(tint)).child(badge.clone()).into_any_element()
                        }
                        None => icon(if row.deck.ends_with(".md") { "file-text" } else { "folder" }, iconsize::S, t.secondary).into_any_element(),
                    };
                    div()
                        .id(("deck-row", i))
                        .h(px(control::M))
                        .w_full()
                        .pl(px(space::XS + row.depth as f32 * space::M))
                        .pr(px(space::M))
                        .flex()
                        .items_center()
                        .gap(px(space::XS))
                        .cursor_pointer()
                        .text_size(px(font::CALLOUT))
                        .when(active, |d| d.bg(t.ink.opacity(0.08)))
                        .when(!active, |d| d.soft(hover, 0.))
                        .child(if row.has_children {
                            div()
                                .id(("deck-fold", i))
                                .flex_none()
                                .size(px(iconsize::M))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(chevron(("deck-chevron", i), row.open, t.secondary))
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    a.update(cx, |p, cx| {
                                        if !p.open.remove(&deck_a) {
                                            p.open.insert(deck_a.clone());
                                        }
                                        cx.notify();
                                    });
                                })
                                .into_any_element()
                        } else {
                            div().flex_none().size(px(iconsize::M)).into_any_element()
                        })
                        .child(chip)
                        .child(div().flex_1().min_w_0().truncate().child(row.name.clone()))
                        .child(counts_pills(cards::counts(&row.deck, now), &t))
                        .tooltip(tip(if row.total == 1 { tr("1 Karte") } else { trf("%@ Karten", &[&row.total.to_string()]) }))
                        .on_click(move |_, _, cx| b.update(cx, |p, cx| p.select_deck(deck_b.clone(), cx)))
                        .context_menu(move |menu, _, _| {
                            let (start, new, deck_start, deck_new) = (m_c.clone(), m_d.clone(), deck_c.clone(), deck_d.clone());
                            menu.item(PopupMenuItem::new(tr("Lernen")).icon(menu_icon("graduation-cap")).on_click(move |_, window, cx| {
                                let deck = deck_start.clone();
                                start.update(cx, |p, cx| p.start(deck, window, cx));
                            }))
                            .item(PopupMenuItem::new(tr("Neue Karte…")).icon(menu_icon("plus")).on_click(move |_, window, cx| {
                                let deck = deck_new.clone();
                                new.update(cx, |p, cx| {
                                    p.deck = deck;
                                    p.compose(None, window, cx);
                                });
                            }))
                        })
                        .into_any_element()
                })
                .collect::<Vec<_>>()
        })
        .size_full();
        div().w(px(260.)).flex_none().h_full().flex().flex_col().bg(t.column).border_r_1().border_color(t.separator).child(div().flex_none().h(px(control::SUBBAR)).px(px(space::M)).flex().items_center().border_b_1().border_color(t.separator).font_weight(FontWeight::SEMIBOLD).child(div().flex_1().child(tr("Stapel"))).child(Button::new("cards-new-side").ghost().small().icon(menu_icon("plus")).tooltip(tr("Neue Karte (N)")).on_click(cx.listener(|p, _, window, cx| p.compose(None, window, cx))))).child(head).child(div().flex_1().min_h_0().child(tree))
    }

    fn stat_tile(label: String, value: String, detail: Option<String>, t: &Theme) -> Div {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(space::XXS))
            .p(px(space::M))
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(t.separator)
            .child(div().text_size(px(font::CAPTION)).text_color(t.secondary).child(label))
            .child(div().text_size(px(font::TITLE)).font_weight(FontWeight::SEMIBOLD).font_features(tabular()).child(value))
            .children(detail.map(|d| div().text_size(px(font::CAPTION)).text_color(t.tertiary).truncate().child(d)))
    }

    fn forecast(stats: &cards::Stats, t: &Theme) -> Div {
        let most = stats.forecast.iter().copied().max().unwrap_or(0).max(1);
        let today = chrono::Local::now().date_naive();
        let bars = stats.forecast.iter().enumerate().map(|(i, n)| {
            let share = *n as f32 / most as f32;
            let day = today + chrono::Days::new(i as u64);
            let label = if i == 0 { tr("Heute") } else { day.format("%d.").to_string() };
            div()
                .id(("forecast", i))
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .justify_end()
                .items_center()
                .gap(px(space::XXS))
                .tooltip(tip(if *n == 1 { trf("%@: 1 Karte", &[&day.format("%d.%m.").to_string()]) } else { trf("%@: %@ Karten", &[&day.format("%d.%m.").to_string(), &n.to_string()]) }))
                .child(div().text_size(px(font::CAPTION2)).text_color(t.secondary).font_features(tabular()).when(*n > 0, |d| d.child(n.to_string())))
                .child(div().w_full().max_w(px(28.)).rounded_t(px(radius::SMALL)).h(px((share * 56.).max(if *n > 0 { 3. } else { 1. }))).bg(if i == 0 { t.accent } else if *n > 0 { t.accent.opacity(0.45) } else { t.separator }))
                .child(div().text_size(px(font::CAPTION2)).text_color(if i == 0 { t.primary } else { t.tertiary }).child(label))
        });
        div().flex().flex_col().gap(px(space::S)).child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(tr("Die nächsten 14 Tage"))).child(div().h(px(92.)).flex().gap(px(space::XS)).children(bars))
    }

    /// Shows how cards are written, for a workspace without any.
    fn empty_room(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let example = |syntax: &str, meaning: String| {
            div().flex().items_center().gap(px(space::M)).px(px(space::M)).py(px(space::S)).child(div().flex_1().min_w_0().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(syntax.to_string())).child(div().w(px(180.)).flex_none().text_size(px(font::CALLOUT)).text_color(t.secondary).child(meaning))
        };
        div()
            .id("cards-empty")
            .size_full()
            .overflow_y_scrollbar()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(space::L))
            .p(px(space::XL))
            .child(icon("layers", iconsize::HERO, t.secondary))
            .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(tr("Noch keine Karten")))
            .child(div().max_w(px(520.)).text_center().text_color(t.secondary).child(tr("Karten stehen als Text in deinen Notizen. So bleiben sie beim Stoff, und alles lässt sich auch in Obsidian lesen.")))
            .child(
                grouped(t)
                    .w_full()
                    .max_w(px(560.))
                    .child(example("Frage :: Antwort", tr("Eine Karte")))
                    .child(divider(t))
                    .child(example("Begriff ::: Erklärung", tr("In beide Richtungen")))
                    .child(divider(t))
                    .child(example("Lange Frage\n?\nAntwort", tr("Über mehrere Zeilen")))
                    .child(divider(t))
                    .child(example("Der {{c1::Satz}} gilt", tr("Lückentext"))),
            )
            .child(Button::new("cards-first").primary().label(tr("Erste Karte schreiben")).on_click(cx.listener(|p, _, window, cx| p.compose(None, window, cx))))
            .into_any_element()
    }

    fn card_row(card: &Card, i: usize, selected: bool, now: i64, me: Entity<Self>, t: &Theme) -> AnyElement {
        let hover = t.ink.opacity(0.05);
        let id = card.id().to_string();
        let status: (String, Hsla) = if card.suspended() {
            (tr("Pausiert"), t.tertiary)
        } else if card.is_leech() {
            (tr("Schwierig"), t.orange)
        } else {
            match card.state.as_ref().filter(|s| s.phase != Phase::New) {
                None => (tr("Neu"), t.blue),
                Some(s) if s.due <= now => (tr("Fällig"), t.green),
                Some(s) => (cards::wait_label(now, s.due), t.secondary),
            }
        };
        let menu_card = card.clone();
        let (m1, m2) = (me.clone(), me.clone());
        let name = Path::new(&card.note_rel).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        div()
            .id(("card-row", i))
            .h(px(control::M + space::XS))
            .w_full()
            .px(px(space::L))
            .flex()
            .items_center()
            .gap(px(space::S))
            .cursor_pointer()
            .text_size(px(font::CALLOUT))
            .when(selected, |d| d.bg(t.ink.opacity(0.08)))
            .when(!selected, |d| d.soft(hover, 0.))
            .child(icon(cards::kind_icon(card.raw.kind), iconsize::S, t.tertiary))
            .child(div().flex_1().min_w_0().truncate().when(card.suspended(), |d| d.text_color(t.tertiary)).child(cards::plain_text(&card.raw.front)))
            .child(caption(name, t).flex_none().max_w(px(140.)).truncate())
            .child(div().flex_none().w(px(64.)).text_right().text_size(px(font::CAPTION)).text_color(status.1).child(status.0))
            .on_click(move |e, window, cx| {
                let id = id.clone();
                let double = e.click_count() >= 2;
                m1.update(cx, |p, cx| {
                    window.focus(&p.focus, cx);
                    p.select_card(id.clone(), cx);
                    if double {
                        let card = p.all.iter().find(|c| c.id() == id).cloned();
                        p.compose(card, window, cx);
                    }
                });
            })
            .context_menu(move |menu, _, _| Self::card_menu(menu, menu_card.clone(), m2.clone()))
            .into_any_element()
    }

    fn card_menu(menu: gpui_kit::component::menu::PopupMenu, card: Card, me: Entity<Self>) -> gpui_kit::component::menu::PopupMenu {
        let suspended = card.suspended();
        let (c1, c2, c3, c4, c5) = (card.clone(), card.clone(), card.clone(), card.clone(), card);
        let (m1, m2, m3, m4) = (me.clone(), me.clone(), me.clone(), me);
        menu.item(PopupMenuItem::new(tr("Bearbeiten")).icon(menu_icon("pencil")).on_click(move |_, window, cx| {
            let card = c1.clone();
            m1.update(cx, |p, cx| p.compose(Some(card), window, cx));
        }))
        .item(PopupMenuItem::new(tr("In der Notiz zeigen")).icon(menu_icon("file-text")).on_click(move |_, _, cx| Self::open_source(&c2, cx)))
        .separator()
        .item(PopupMenuItem::new(if suspended { tr("Fortsetzen") } else { tr("Pausieren") }).icon(menu_icon("pause")).on_click(move |_, _, cx| {
            cards::set_suspended(&c3, !suspended);
            m2.update(cx, |_, cx| cx.notify());
            state::lib(cx).update(cx, |_, cx| cx.notify());
        }))
        .item(PopupMenuItem::new(tr("Fortschritt zurücksetzen")).icon(menu_icon("rotate-ccw")).on_click(move |_, _, cx| {
            cards::reset(&c4);
            m3.update(cx, |_, cx| cx.notify());
            state::lib(cx).update(cx, |_, cx| cx.notify());
        }))
        .separator()
        .item(PopupMenuItem::new(tr("Aus der Notiz löschen")).icon(menu_icon("trash-2")).on_click(move |_, _, cx| {
            delete_card(&c5, cx);
            m4.update(cx, |p, cx| {
                p.selected = None;
                p.preview = None;
                cx.notify();
            });
        }))
    }

    fn detail_panel(&self, card: &Card, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let base = card.note.parent().map(Path::to_path_buf);
        let now = cards::now();
        let fact = |label: String, value: String| div().flex().items_center().justify_between().text_size(px(font::CALLOUT)).child(div().text_color(t.secondary).child(label)).child(div().font_features(tabular()).child(value));
        let mut facts = div().flex().flex_col().gap(px(space::XS));
        match card.state.as_ref().filter(|s| s.phase != Phase::New) {
            Some(s) => {
                let days = s.last_review.map_or(0., |l| (now - l) as f64 / 86400.);
                facts = facts
                    .child(fact(tr("Fällig"), if s.due <= now { tr("Jetzt") } else { cards::wait_label(now, s.due) }))
                    .child(fact(tr("Erinnerung jetzt"), format!("{:.0} %", fsrs::retrievability(s.stability, days) * 100.)))
                    .child(fact(tr("Stabilität"), trf("%@ T.", &[&format!("{:.1}", s.stability)])))
                    .child(fact(tr("Schwierigkeit"), format!("{:.1} / 10", s.difficulty)))
                    .child(fact(tr("Wiederholungen"), s.reps.to_string()))
                    .child(fact(tr("Vergessen"), s.lapses.to_string()));
            }
            None => facts = facts.child(fact(tr("Fällig"), tr("Neu"))),
        }
        let (edit, source, toggle) = (card.clone(), card.clone(), card.clone());
        let me = cx.entity();
        let body = self.preview.as_ref().filter(|(id, ..)| id == card.id()).map(|(_, front, back)| {
            let t = *t;
            div().flex().flex_col().gap(px(space::M)).child(markdown_view::render(front, base.as_deref(), None, &t)).child(div().h(px(1.)).bg(t.separator)).child(markdown_view::render(back, base.as_deref(), None, &t))
        });
        div()
            .id("card-detail")
            .w(px(320.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(t.separator)
            .child(div().id("card-detail-body").flex_1().min_h_0().overflow_y_scrollbar().p(px(space::L)).flex().flex_col().gap(px(space::L)).children(body).child(div().h(px(1.)).flex_none().bg(t.separator)).child(facts).when(!card.raw.context.is_empty(), |d| d.child(caption(card.raw.context.join(" › "), t))))
            .child(
                div()
                    .flex_none()
                    .p(px(space::M))
                    .border_t_1()
                    .border_color(t.separator)
                    .flex()
                    .gap(px(space::S))
                    .child(Button::new("card-edit").small().icon(menu_icon("pencil")).label(tr("Bearbeiten")).on_click(cx.listener(move |p, _, window, cx| p.compose(Some(edit.clone()), window, cx))))
                    .child(Button::new("card-source").small().ghost().icon(menu_icon("file-text")).tooltip(tr("In der Notiz zeigen")).on_click(move |_, _, cx| Self::open_source(&source, cx)))
                    .child(Button::new("card-more").small().ghost().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu(move |menu, _, _| Self::card_menu(menu, toggle.clone(), me.clone()))),
            )
            .into_any_element()
    }

    fn overview(&mut self, now: i64, wide: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let deck = self.deck.clone();
        let counts = cards::counts(&deck, now);
        let stats = cards::stats(&deck, now);
        let total = counts.total();
        let title = deck_title(&deck, cx);
        let next = cards::next_due(&deck, now);
        let status = if total > 0 {
            let mut parts = Vec::new();
            if counts.new > 0 {
                parts.push(trf("%@ neu", &[&counts.new.to_string()]));
            }
            if counts.learning > 0 {
                parts.push(trf("%@ im Lernen", &[&counts.learning.to_string()]));
            }
            if counts.review > 0 {
                parts.push(trf("%@ fällig", &[&counts.review.to_string()]));
            }
            parts.join(" · ")
        } else if let Some(at) = next {
            trf("Alles gelernt. Nächste Karte in %@.", &[&cards::wait_label(now, at)])
        } else {
            tr("Alles gelernt.")
        };
        let start_deck = deck.clone();
        let head = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(space::M))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).truncate().child(title)).child(div().text_color(t.secondary).truncate().child(status)))
            .child(Button::new("cards-new").icon(menu_icon("plus")).label(tr("Neue Karte")).tooltip(tr("Neue Karte (N)")).on_click(cx.listener(|p, _, window, cx| p.compose(None, window, cx))))
            .child(Button::new("cards-learn").primary().icon(menu_icon("graduation-cap")).label(if total > 0 { trf("Lernen (%@)", &[&total.to_string()]) } else { tr("Lernen") }).tooltip(tr("Lernen (Leertaste)")).disabled(total == 0).on_click(cx.listener(move |p, _, window, cx| p.start(start_deck.clone(), window, cx))));
        let retention = stats.true_retention.map_or_else(|| "–".to_string(), |r| format!("{:.0} %", r * 100.));
        let tiles = div()
            .flex_none()
            .flex()
            .gap(px(space::M))
            .child(Self::stat_tile(tr("Heute gelernt"), stats.reviewed_today.to_string(), None, t))
            .child(Self::stat_tile(tr("Erinnert"), retention, Some(tr("letzte 30 Tage")), t))
            .child(Self::stat_tile(tr("Serie"), if stats.streak == 1 { tr("1 Tag") } else { trf("%@ Tage", &[&stats.streak.to_string()]) }, None, t))
            .child(Self::stat_tile(tr("Gefestigt"), format!("{} / {}", stats.mature, stats.total), Some(tr("Abstand ab 3 Wochen")), t));
        let forecast = Self::forecast(&stats, t);
        let rows = self.visible(now, cx);
        let me = cx.entity();
        let selected = self.selected.clone();
        let all = self.all.clone();
        let list: AnyElement = if rows.is_empty() {
            unavailable("search", tr("Keine Karten"), tr("Ändere den Filter oder die Suche."), t).into_any_element()
        } else {
            let (count, shown) = (rows.len(), rows.clone());
            let listed = all.clone();
            uniform_list("cards-list", count, move |range, _, cx| {
                let t = Theme::get(cx);
                range.map(|i| { let card = &listed[shown[i]]; Self::card_row(card, i, selected.as_deref() == Some(card.id()), now, me.clone(), &t) }).collect::<Vec<_>>()
            })
            .track_scroll(&self.list_scroll)
            .size_full()
            .into_any_element()
        };
        let current_filter = self.filter;
        let filters = div().flex().flex_wrap().gap(px(space::XXS)).children(Filter::ALL.iter().map(|f| {
            let (f, on) = (*f, *f == current_filter);
            div().id(SharedString::from(format!("filter-{f:?}"))).px(px(space::S)).h(px(control::S)).flex().items_center().rounded(px(radius::INNER)).cursor_pointer().text_size(px(font::CALLOUT)).when(on, |d| d.bg(t.ink.opacity(0.1))).when(!on, |d| d.text_color(t.secondary).soft(t.ink.opacity(0.05), radius::INNER)).child(f.label()).on_click(cx.listener(move |p, _, _, cx| {
                p.filter = f;
                cx.notify();
            }))
        }));
        let browser = div().flex_1().min_h_0().flex().flex_col().child(div().flex_none().px(px(space::L)).py(px(space::S)).flex().flex_wrap().items_center().gap(px(space::S)).border_t_1().border_b_1().border_color(t.separator).child(div().w(px(220.)).child(Input::new(&self.query).small().cleanable(true).prefix(icon("search", iconsize::XS, t.secondary)))).child(filters).child(div().flex_1()).child(caption(if rows.len() == 1 { tr("1 Karte") } else { trf("%@ Karten", &[&rows.len().to_string()]) }, t))).child(div().flex_1().min_h_0().child(list));
        let chosen = self.selected.as_ref().and_then(|id| self.all.iter().find(|c| c.id() == id)).cloned();
        let summary: AnyElement = if wide { div().flex().gap(px(space::XL)).child(div().flex_1().min_w_0().child(tiles)).child(div().w(px(340.)).flex_none().child(forecast)).into_any_element() } else { tiles.into_any_element() };
        let main = div().flex_1().min_w_0().h_full().flex().flex_col().child(div().flex_none().p(px(space::XL)).pb(px(space::L)).flex().flex_col().gap(px(space::L)).child(head).child(summary)).child(browser);
        div().size_full().flex().child(main).when_some(chosen, |d, card| d.child(self.detail_panel(&card, t, cx))).into_any_element()
    }

    fn done_screen(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(s) = &self.session else { return div().into_any_element() };
        let reviewed: usize = s.ratings.iter().sum();
        let minutes = ((cards::now() - s.started) as f64 / 60.).round().max(1.) as i64;
        let deck = s.deck.clone();
        let now = cards::now();
        let next = cards::next_due(&deck, now);
        let total = reviewed.max(1) as f32;
        let colors = [t.red, t.orange, t.green, t.blue];
        let bar = div().w(px(360.)).h(px(space::S)).rounded_full().overflow_hidden().flex().children(s.ratings.iter().enumerate().filter(|(_, n)| **n > 0).map(|(i, n)| div().h_full().w(relative(*n as f32 / total)).bg(colors[i])));
        let legend = div().flex().gap(px(space::L)).children(Rating::ALL.iter().enumerate().map(|(i, rating)| div().flex().items_center().gap(px(space::XS)).text_size(px(font::CALLOUT)).child(div().size(px(space::S)).rounded_full().bg(colors[i])).child(rating_name(*rating)).child(div().text_color(t.secondary).font_features(tabular()).child(s.ratings[i].to_string()))));
        let can_undo = !s.answered.is_empty();
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(space::L))
            .child(icon("circle-check", iconsize::HERO, t.green))
            .child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(tr("Geschafft")))
            .child(div().text_color(t.secondary).child(if reviewed == 0 { tr("Für jetzt ist nichts mehr dran.") } else { trf("%@ Karten in %@ Min.", &[&reviewed.to_string(), &minutes.to_string()]) }))
            .when(reviewed > 0, |d| d.child(bar).child(legend))
            .children(next.map(|at| caption(trf("Nächste Karte in %@", &[&cards::wait_label(now, at)]), t)))
            .child(
                div()
                    .flex()
                    .gap(px(space::S))
                    .when(can_undo, |d| d.child(Button::new("cards-undo-last").icon(menu_icon("undo-2")).label(tr("Letzte Antwort zurück")).on_click(cx.listener(|p, _, _, cx| p.undo(cx)))))
                    .child(Button::new("cards-done").primary().label(tr("Fertig")).on_click(cx.listener(|p, _, _, cx| {
                        p.session = None;
                        p.cached = None;
                        cx.notify();
                    }))),
            )
            .into_any_element()
    }

    fn review(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(s) = &self.session else { return div().into_any_element() };
        let Some(card) = s.current.clone() else { return self.done_screen(t, cx) };
        let answered: usize = s.ratings.iter().sum();
        let progress = answered as f32 / (answered + s.left.total()).max(1) as f32;
        let phase = card.phase();
        let now = cards::now();
        let base = card.note.parent().map(Path::to_path_buf);
        let front = markdown_view::render(&s.front, base.as_deref(), None, t);
        let back = s.shown.then(|| markdown_view::render(&s.back, base.as_deref(), None, t));
        let shown = s.shown;
        let can_undo = !s.answered.is_empty();
        let count_piece = |n: usize, color: Hsla, on: bool, label: String| div().px(px(space::S)).h(px(control::S)).flex().items_center().rounded(px(radius::INNER)).when(on, |d| d.bg(color.opacity(0.16))).text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).font_features(tabular()).text_color(color).child(format!("{n} {label}"));
        let (menu_card, menu_me) = (card.clone(), cx.entity());
        let bar = div()
            .flex_none()
            .h(px(control::SUBBAR))
            .px(px(space::M))
            .flex()
            .items_center()
            .gap(px(space::S))
            .child(Button::new("review-end").ghost().small().icon(menu_icon("x")).tooltip(tr("Beenden (Esc)")).on_click(cx.listener(|p, _, _, cx| {
                p.session = None;
                p.cached = None;
                cx.notify();
            })))
            .child(div().flex_1().min_w_0().truncate().text_size(px(font::CALLOUT)).text_color(t.secondary).child(deck_title(&s.deck, cx)))
            .child(count_piece(s.left.new, t.blue, phase == Phase::New, tr("neu")))
            .child(count_piece(s.left.learning, t.orange, matches!(phase, Phase::Learning | Phase::Relearning), tr("lernen")))
            .child(count_piece(s.left.review, t.green, phase == Phase::Review, tr("fällig")))
            .child(div().flex_1())
            .child(Button::new("review-undo").ghost().small().icon(menu_icon("undo-2")).tooltip(tr("Letzte Antwort zurück (⌘Z)")).disabled(!can_undo).on_click(cx.listener(|p, _, _, cx| p.undo(cx))))
            .child(Button::new("review-edit").ghost().small().icon(menu_icon("pencil")).tooltip(tr("Karte bearbeiten (E)")).on_click({
                let edit = card.clone();
                cx.listener(move |p, _, window, cx| p.compose(Some(edit.clone()), window, cx))
            }))
            .child(Button::new("review-more").ghost().small().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu(move |menu, _, _| {
                let (a, b, c) = (menu_me.clone(), menu_me.clone(), menu_card.clone());
                menu.item(PopupMenuItem::new(tr("In der Notiz zeigen (O)")).icon(menu_icon("file-text")).on_click(move |_, _, cx| Self::open_source(&c, cx)))
                    .item(PopupMenuItem::new(tr("Für heute zurückstellen (B)")).icon(menu_icon("eye-off")).on_click(move |_, _, cx| a.update(cx, |p, cx| p.set_aside(false, cx))))
                    .item(PopupMenuItem::new(tr("Pausieren (S)")).icon(menu_icon("pause")).on_click(move |_, _, cx| b.update(cx, |p, cx| p.set_aside(true, cx))))
            }));
        let track = div().flex_none().h(px(space::XXS)).w_full().bg(t.separator).child(div().h_full().w(relative(progress)).bg(t.accent));
        let leech = card.is_leech().then(|| {
            let edit = card.clone();
            banner(t.orange).flex().items_center().gap(px(space::S)).py(px(space::S)).child(icon("triangle-alert", iconsize::S, t.orange)).child(div().flex_1().text_size(px(font::CALLOUT)).child(tr("Diese Karte vergisst du oft. Umformulieren oder aufteilen hilft meist mehr als Wiederholen."))).child(Button::new("leech-edit").small().label(tr("Bearbeiten")).on_click(cx.listener(move |p, _, window, cx| p.compose(Some(edit.clone()), window, cx))))
        });
        let crumbs = (!card.raw.context.is_empty()).then(|| caption(card.raw.context.join(" › "), t));
        let body = div()
            .id("review-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(
                div().w_full().min_h_full().flex().flex_col().items_center().justify_center().p(px(space::XL)).child(
                    div()
                        .w_full()
                        .max_w(px(720. * super::theme::note_scale()))
                        .flex()
                        .flex_col()
                        .gap(px(space::L))
                        .children(leech)
                        .children(crumbs)
                        .child(div().text_size(px(note_size(font::TITLE3))).child(front))
                        .when_some(back, |d, back| d.child(div().h(px(1.)).bg(t.separator)).child(back)),
                ),
            );
        let footer: AnyElement = if shown {
            let choices = cards::options(card.id(), card.state.as_ref(), &card.note_rel, &card.raw.front, now);
            let colors = [t.red, t.orange, t.green, t.blue];
            div()
                .flex_none()
                .p(px(space::M))
                .border_t_1()
                .border_color(t.separator)
                .flex()
                .justify_center()
                .gap(px(space::S))
                .children(Rating::ALL.iter().enumerate().map(|(i, rating)| {
                    let rating = *rating;
                    let color = colors[i];
                    let hover = color.opacity(0.16);
                    div()
                        .id(("rate", i))
                        .w(px(132.))
                        .h(px(control::FOOTBAR))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(space::XXS))
                        .rounded(px(radius::CONTROL))
                        .border_1()
                        .border_color(color.opacity(0.45))
                        .bg(color.opacity(0.08))
                        .cursor_pointer()
                        .key_nav()
                        .hover(move |d| d.bg(hover))
                        .active(move |d| d.bg(color.opacity(0.26)))
                        .child(div().text_size(px(font::CAPTION)).text_color(t.secondary).font_features(tabular()).child(cards::wait_label(now, choices[i].due)))
                        .child(div().flex().items_baseline().gap(px(space::XS)).child(div().font_weight(FontWeight::SEMIBOLD).text_color(t.readable(color)).child(rating_name(rating))).child(div().text_size(px(font::CAPTION2)).text_color(t.tertiary).child((i + 1).to_string())))
                        .on_click(cx.listener(move |p, _, _, cx| p.rate(rating, cx)))
                }))
                .into_any_element()
        } else {
            div()
                .flex_none()
                .p(px(space::M))
                .h(px(control::FOOTBAR + space::M * 2. + 1.))
                .border_t_1()
                .border_color(t.separator)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("reveal")
                        .w(px(320.))
                        .h(px(control::FOOTBAR))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(space::S))
                        .rounded(px(radius::CONTROL))
                        .bg(t.accent)
                        .text_color(t.on_accent)
                        .font_weight(FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .key_nav()
                        .hover(|d| d.opacity(0.9))
                        .child(tr("Antwort zeigen"))
                        .child(div().text_size(px(font::CAPTION)).opacity(0.75).child(tr("Leertaste")))
                        .on_click(cx.listener(|p, _, _, cx| p.reveal(cx))),
                )
                .into_any_element()
        };
        div().size_full().flex().flex_col().child(bar).child(track).child(body).child(footer).into_any_element()
    }
}

/// Starts a card from text in a note; it is written into that note.
pub fn card_from_selection(text: String, note: PathBuf, window: &mut Window, cx: &mut App) {
    super::sheets::show_sheet(760., window, cx, move |window, cx| CardComposer::new(String::new(), None, Some((text, note)), window, cx));
}

/// Removes a card's lines from its note; the note's undo brings them back.
pub fn delete_card(card: &Card, cx: &mut App) {
    let (first, last) = card.raw.lines;
    let done = act(cx, |l| {
        l.edit_note(&card.note, "card", tr("Karte gelöscht"), |text| {
            let lines: Vec<&str> = text.split('\n').collect();
            let kept: Vec<&str> = lines.iter().enumerate().filter(|(i, _)| *i < first || *i > last).map(|(_, l)| *l).collect();
            kept.join("\n")
        })
    });
    if done {
        state::reindex(cx);
    }
}

impl Render for CardsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        self.all = cards::all_cards();
        if let Some(request) = state::lib(cx).update(cx, |l, _| l.cards_request.take()) {
            self.select_deck(request.clone(), cx);
            if cards::counts(&request, cards::now()).total() > 0 {
                self.start(request, window, cx);
            }
        }
        if self.session.as_ref().is_none() && !self.deck.is_empty() && !self.all.iter().any(|c| c.in_deck(&self.deck)) {
            self.deck.clear();
        }
        let now = cards::now();
        let width = f32::from(window.viewport_size().width);
        let reviewing = self.session.is_some();
        let body: AnyElement = if reviewing {
            self.review(&t, cx)
        } else if self.all.is_empty() {
            self.empty_room(&t, cx)
        } else {
            self.overview(now, width > 1260., &t, cx)
        };
        let side = (!reviewing && !self.all.is_empty() && width > 860.).then(|| self.deck_column(now, &t, cx));
        div()
            .id("cards-page")
            .track_focus(&self.focus)
            .key_context("CardsPage")
            .size_full()
            .flex()
            .bg(t.content)
            .on_key_down(cx.listener(Self::on_key))
            .capture_action(cx.listener(|p, _: &super::app::Undo, _, cx| {
                if p.session.as_ref().is_some_and(|s| !s.answered.is_empty()) {
                    cx.stop_propagation();
                    p.undo(cx);
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|p, _, window, cx| {
                if !p.query.read(cx).focus_handle(cx).is_focused(window) {
                    window.focus(&p.focus, cx);
                }
            }))
            .children(side)
            .child(div().flex_1().min_w_0().h_full().child(body))
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ComposeKind {
    Basic,
    Both,
    Cloze,
}

/// Writes a card, new or changed, as plain text into a note.
pub struct CardComposer {
    kind: ComposeKind,
    front: Entity<TextareaState>,
    back: Entity<TextareaState>,
    target: PathBuf,
    editing: Option<Card>,
    added: usize,
    focused: bool,
    front_preview: Arc<ParsedNote>,
    back_preview: Arc<ParsedNote>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<super::quick::Dismissed> for CardComposer {}

/// Where cards of a deck go when no note is named: its own note, or a note beside it.
fn default_target(deck: &str) -> PathBuf {
    let root = index::current().root.clone();
    if deck.ends_with(".md") {
        return root.join(deck);
    }
    if deck.is_empty() {
        if let Some(last) = crate::core::prefs::get().card_target.filter(|p| !p.is_empty()) {
            return root.join(last);
        }
    }
    root.join(deck).join(format!("{}.md", tr("Karteikarten")))
}

impl CardComposer {
    fn new(deck: String, editing: Option<Card>, draft: Option<(String, PathBuf)>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let front = cx.new(|cx| TextareaState::new(window, cx).placeholder(tr("Frage, oder Satz mit Lücke")).auto_grow(3, 8));
        let back = cx.new(|cx| TextareaState::new(window, cx).placeholder(tr("Antwort")).auto_grow(3, 8));
        let mut kind = ComposeKind::Basic;
        let mut target = default_target(&deck);
        if let Some(card) = &editing {
            target = card.note.clone();
            let source = std::fs::read_to_string(&card.note).unwrap_or_default();
            let lines: Vec<&str> = source.split('\n').collect();
            let written = lines.get(card.raw.lines.0..=card.raw.lines.1).map(|l| l.join("\n")).unwrap_or_default();
            let (f, b) = match card.raw.kind {
                CardKind::Cloze => {
                    kind = ComposeKind::Cloze;
                    (written.trim().to_string(), String::new())
                }
                CardKind::Reverse => {
                    kind = ComposeKind::Both;
                    (card.raw.back.clone(), card.raw.front.clone())
                }
                CardKind::Basic => {
                    if written.contains(" ::: ") || written.lines().any(|l| l.trim() == "??") {
                        kind = ComposeKind::Both;
                    }
                    (card.raw.front.clone(), card.raw.back.clone())
                }
            };
            front.update(cx, |s, cx| s.set_value(f, window, cx));
            back.update(cx, |s, cx| s.set_value(b, window, cx));
        }
        if let Some((text, note)) = draft {
            target = note;
            front.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        let subs = vec![
            cx.subscribe(&front, |this, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.refresh_preview(cx);
                }
            }),
            cx.subscribe(&back, |this, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.refresh_preview(cx);
                }
            }),
        ];
        let mut this = Self { kind, front, back, target, editing, added: 0, focused: false, front_preview: parsed(""), back_preview: parsed(""), _subs: subs };
        this.refresh_preview(cx);
        this
    }

    fn written(&self, cx: &App) -> String {
        let (front, back) = (self.front.read(cx).value().to_string(), self.back.read(cx).value().to_string());
        match self.kind {
            ComposeKind::Basic => crate::core::cards::parse::write_card(&front, &back, false),
            ComposeKind::Both => crate::core::cards::parse::write_card(&front, &back, true),
            ComposeKind::Cloze => front.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" "),
        }
    }

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        let written = self.written(cx);
        let found = crate::core::cards::parse::cards_in(&written);
        let (front, back) = found.first().map_or((String::new(), String::new()), |c| (c.front.clone(), c.back.clone()));
        self.front_preview = parsed(&front);
        self.back_preview = parsed(&back);
        cx.notify();
    }

    fn is_valid(&self, cx: &App) -> bool {
        !crate::core::cards::parse::cards_in(&self.written(cx)).is_empty()
    }

    /// Turns the selection in the question into a new gap.
    fn make_gap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.kind = ComposeKind::Cloze;
        let (value, range) = {
            let s = self.front.read(cx);
            (s.value().to_string(), s.selected_range())
        };
        let next = regex::Regex::new(r"\{\{c(\d+)::").unwrap().captures_iter(&value).filter_map(|c| c[1].parse::<u32>().ok()).max().unwrap_or(0) + 1;
        let (start, end) = (range.start.min(value.len()), range.end.min(value.len()));
        let chosen = &value[start..end];
        let updated = format!("{}{{{{c{next}::{chosen}}}}}{}", &value[..start], &value[end..]);
        let caret = start + format!("{{{{c{next}::{chosen}").len();
        self.front.update(cx, |s, cx| {
            s.set_value(updated, window, cx);
            s.set_selected_range(caret..caret, cx);
            s.focus(window, cx);
        });
        self.refresh_preview(cx);
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_valid(cx) {
            return;
        }
        let written = self.written(cx);
        let target = self.target.clone();
        if let Some(card) = self.editing.clone() {
            let (first, last) = card.raw.lines;
            let replacement = written.clone();
            let saved = act(cx, |l| {
                l.edit_note(&card.note, "card", tr("Karte geändert"), |text| {
                    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
                    let end = (last + 1).min(lines.len());
                    if first < end {
                        lines.splice(first..end, replacement.split('\n').map(str::to_string));
                    }
                    lines.join("\n")
                })
            });
            if saved {
                let new = crate::core::cards::parse::cards_in(&written);
                let same_kind = new.iter().find(|c| c.kind == card.raw.kind).or(new.first());
                if let Some(next) = same_kind {
                    cards::rename(card.id(), &next.id);
                }
                state::reindex(cx);
            }
            cx.emit(super::quick::Dismissed);
            return;
        }
        if !target.exists() {
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&target, "");
        }
        let line = written.clone();
        let done = act(cx, |l| {
            l.edit_note(&target, "card", tr("Karte angelegt"), |text| {
                let joined = if text.is_empty() { String::new() } else if text.ends_with("\n\n") { text.to_string() } else if text.ends_with('\n') { format!("{text}\n") } else { format!("{text}\n\n") };
                format!("{joined}{line}\n")
            })
        });
        if done {
            let root = index::current().root.clone();
            if let Ok(rel) = target.strip_prefix(&root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                crate::core::prefs::update(|p| p.card_target = Some(rel));
            }
            self.added += 1;
            state::reindex(cx);
            self.front.update(cx, |s, cx| {
                s.set_value("", window, cx);
                s.focus(window, cx);
            });
            self.back.update(cx, |s, cx| s.set_value("", window, cx));
            self.refresh_preview(cx);
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(super::quick::Dismissed);
    }
}

impl Render for CardComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        if !std::mem::replace(&mut self.focused, true) {
            self.front.update(cx, |s, cx| s.focus(window, cx));
        }
        let kind = self.kind;
        let kinds = [(ComposeKind::Basic, tr("Einfach"), "file-question"), (ComposeKind::Both, tr("Beide Richtungen"), "repeat-2"), (ComposeKind::Cloze, tr("Lückentext"), "scan")];
        let switch = div().flex().rounded(px(radius::INNER)).bg(t.ink.opacity(0.06)).p(px(space::XXS)).children(kinds.into_iter().map(|(k, label, name)| {
            let on = k == kind;
            div().id(SharedString::from(format!("kind-{name}"))).h(px(control::S)).px(px(space::S)).flex().items_center().gap(px(space::XS)).rounded(px(radius::SMALL)).cursor_pointer().text_size(px(font::CALLOUT)).when(on, |d| d.bg(t.ink.opacity(0.12))).child(icon(name, iconsize::S, if on { t.primary } else { t.secondary })).child(label).on_click(cx.listener(move |c, _, _, cx| {
                c.kind = k;
                c.refresh_preview(cx);
            }))
        }));
        let root = index::current().root.clone();
        let target_name = self.target.strip_prefix(&root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        let mut notes: Vec<PathBuf> = cards::all_cards().iter().map(|c| c.note.clone()).collect();
        notes.dedup();
        notes.retain(|n| *n != self.target);
        notes.truncate(30);
        let me = cx.entity();
        let editing = self.editing.is_some();
        let picker = Button::new("card-target").small().ghost().icon(menu_icon("file-text")).label(target_name.trim_end_matches(".md").to_string()).dropdown_caret(true).disabled(editing).dropdown_menu(move |mut menu, _, _| {
            let list_root = root.clone();
            for note in &notes {
                let (e, path) = (me.clone(), note.clone());
                let label = note.strip_prefix(&list_root).map(|p| p.to_string_lossy().trim_end_matches(".md").to_string()).unwrap_or_default();
                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    let p = path.clone();
                    e.update(cx, |c, cx| {
                        c.target = p;
                        cx.notify();
                    });
                }));
            }
            menu
        });
        let base = self.target.parent().map(Path::to_path_buf);
        let valid = self.is_valid(cx);
        let cloze = kind == ComposeKind::Cloze;
        let field_label = |text: String| div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.secondary).child(text);
        let form = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .child(div().flex().items_center().justify_between().child(field_label(if cloze { tr("Text mit Lücken") } else { tr("Vorderseite") })).child(Button::new("card-gap").ghost().small().icon(menu_icon("scan")).label(tr("Lücke")).tooltip(tr("Macht aus der Auswahl eine Lücke")).on_click(cx.listener(|c, _, window, cx| c.make_gap(window, cx)))))
            .child(Textarea::new(&self.front))
            .when(!cloze, |d| d.child(field_label(tr("Rückseite"))).child(Textarea::new(&self.back)))
            .child(caption(tr("Markdown, Formeln mit $…$ und Bilder gehen überall."), &t));
        let preview = div()
            .id("card-preview")
            .w(px(300.))
            .flex_none()
            .max_h(px(320.))
            .overflow_y_scrollbar()
            .p(px(space::M))
            .rounded(px(radius::CARD))
            .bg(t.secondary_bg)
            .flex()
            .flex_col()
            .gap(px(space::M))
            .child(field_label(tr("Vorschau")))
            .child(markdown_view::render(&self.front_preview, base.as_deref(), None, &t))
            .child(div().h(px(1.)).flex_none().bg(t.separator))
            .child(markdown_view::render(&self.back_preview, base.as_deref(), None, &t));
        div()
            .w_full()
            .flex()
            .flex_col()
            .key_context("CardComposer")
            .capture_action(cx.listener(|c, a: &gpui_kit::component::input::Enter, window, cx| {
                if a.secondary {
                    c.save(window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_key_down(cx.listener(|c, e: &KeyDownEvent, window, cx| {
                let m = &e.keystroke.modifiers;
                if e.keystroke.key == "escape" {
                    c.close(cx);
                    cx.stop_propagation();
                } else if e.keystroke.key == "tab" && !m.shift && c.front.read(cx).focus_handle(cx).is_focused(window) && c.kind != ComposeKind::Cloze {
                    c.back.update(cx, |s, cx| s.focus(window, cx));
                    cx.stop_propagation();
                } else if e.keystroke.key == "tab" && m.shift && c.back.read(cx).focus_handle(cx).is_focused(window) {
                    c.front.update(cx, |s, cx| s.focus(window, cx));
                    cx.stop_propagation();
                }
            }))
            .child(div().flex().items_center().gap(px(space::M)).px(px(space::XL)).pt(px(space::L)).child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(if editing { tr("Karte bearbeiten") } else { tr("Neue Karte") })).child(div().flex_1()).child(switch))
            .child(div().flex().gap(px(space::L)).p(px(space::XL)).child(form).child(preview))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .p(px(space::L))
                    .border_t_1()
                    .border_color(t.separator)
                    .child(picker)
                    .child(div().flex_1())
                    .when(self.added > 0, |d| d.child(caption(if self.added == 1 { tr("1 Karte angelegt") } else { trf("%@ Karten angelegt", &[&self.added.to_string()]) }, &t)))
                    .child(Button::new("card-close").label(if self.added > 0 { tr("Fertig") } else { tr("Abbrechen") }).on_click(cx.listener(|c, _, _, cx| c.close(cx))))
                    .child(Button::new("card-save").primary().label(if editing { tr("Sichern") } else { tr("Anlegen") }).tooltip(tr("⌘↩")).disabled(!valid).on_click(cx.listener(|c, _, window, cx| c.save(window, cx)))),
            )
    }
}
