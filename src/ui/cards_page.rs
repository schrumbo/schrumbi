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
use super::theme::{Theme, control, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::cards::fsrs::{self, Rating};
use crate::core::cards::{self, Card, CardKind, Counts, Phase};
use crate::core::cards::parse;
use crate::core::i18n::{tr, trf};
use crate::core::prefs;
use crate::core::text::fold;
use crate::core::vault::index;

const STUDY_SCALE: f32 = 1.6;
const STUDY_WIDTH: f32 = 400.;

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
            Filter::Due => !card.suspended() && !card.buried(now) && card.state.as_ref().is_some_and(|s| s.phase != Phase::New && (s.due <= now || (s.phase == Phase::Review && s.due < cards::next_day_start(now)))),
            Filter::Hard => card.is_leech(),
            Filter::Suspended => card.suspended(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Order {
    Note,
    Due,
    Hardest,
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
    /// What can be taken back, with the answer it counted as.
    answered: Vec<(cards::Answered, Option<usize>)>,
    /// Answers taken back and not yet replaced by a new one.
    undone: Vec<(String, Rating)>,
    ratings: [usize; 4],
    started: i64,
    left: Counts,
    /// Everything once, before an exam: nothing about the schedule changes.
    cram: Option<Vec<Card>>,
    /// The cards already gone through in that run, and whether they went back to the end.
    crammed: Vec<(Card, bool)>,
    /// A reveal does not count as an answer when a key bounces.
    shown_at: std::time::Instant,
}

impl Session {
    fn new(deck: String, cram: Option<Vec<Card>>) -> Self {
        Self { deck, current: None, front: parsed(""), back: parsed(""), shown: false, answered: Vec::new(), undone: Vec::new(), ratings: [0; 4], started: cards::now(), left: Counts::default(), cram, crammed: Vec::new(), shown_at: std::time::Instant::now() }
    }

    fn show(&mut self, card: Card, answered: bool) {
        self.front = parsed(&card.raw.front);
        self.back = parsed(&card.raw.back);
        self.current = Some(card);
        self.shown = answered;
        self.shown_at = std::time::Instant::now();
    }
}

/// Where the arrow keys act.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Zone {
    Decks,
    Cards,
}

/// The flashcards room: decks on the left, a deck's state and cards on the right, or a learning session.
pub struct CardsPage {
    deck: String,
    open: BTreeSet<String>,
    session: Option<Session>,
    query: Entity<InputState>,
    filter: Filter,
    order: Order,
    selected: Option<String>,
    /// Where the selected card is written, to stay on it when editing changes its identity.
    selected_place: Option<(Arc<PathBuf>, usize, CardKind)>,
    /// Several cards chosen for one action; the selected one stays what the panel shows.
    marked: BTreeSet<String>,
    zone: Zone,
    preview: Option<(String, Arc<ParsedNote>, Arc<ParsedNote>)>,
    focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    deck_scroll: UniformListScrollHandle,
    review_scroll: ScrollHandle,
    cached: Option<((usize, u64, u64, i64, String, Filter, Order, String), Rc<Vec<usize>>, [usize; 6])>,
    decks: Option<((u64, Vec<String>, String), Rc<Vec<DeckRow>>)>,
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
        sorted.sort_by_key(|d| (d.ends_with(".md"), natural_key(d)));
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

fn every_deck(all: &[Card]) -> BTreeSet<String> {
    let mut open = BTreeSet::new();
    for card in all {
        let mut parent = String::new();
        for part in card.note_rel.split('/') {
            parent = if parent.is_empty() { part.to_string() } else { format!("{parent}/{part}") };
            open.insert(parent.clone());
        }
    }
    open
}

fn counts_text(counts: Counts) -> String {
    [counts.new, counts.learning, counts.review].iter().filter(|n| **n > 0).map(ToString::to_string).collect::<Vec<_>>().join(" · ")
}

fn counts_pills(counts: Counts, t: &Theme) -> Div {
    let piece = |n: usize, color: Hsla| (n > 0).then(|| div().text_color(color).font_features(tabular()).child(n.to_string()));
    div().flex_none().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).children(piece(counts.new, t.blue)).children(piece(counts.learning, t.orange)).children(piece(counts.review, t.green))
}

thread_local! {
    /// How many cards are marked, for the wording of the row menu.
    static MARKED_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A wait as it reads after "in": 7 Tagen, not 7 Tage.
fn in_time(now: i64, at: i64) -> String {
    let label = cards::wait_label(now, at);
    if label.ends_with(" Tage") { format!("{label}n") } else { label }
}

/// The note and the headings a card stands under.
fn card_path(card: &Card) -> String {
    let name = Path::new(card.note_rel.as_str()).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut parts = vec![name];
    parts.extend(card.raw.context.iter().filter(|h| **h != parts[0]).cloned().collect::<Vec<_>>());
    parts.join(" › ")
}

/// Says in words what a deck's coloured numbers mean.
fn deck_help(total: usize, waiting: Counts) -> String {
    let all = if total == 1 { tr("1 Karte") } else { trf("%@ Karten", &[&total.to_string()]) };
    if waiting.total() == 0 {
        return all;
    }
    format!("{all}: {}", waiting_words(waiting))
}

fn waiting_words(counts: Counts) -> String {
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
}

fn deck_pause_all(deck: &str, pause: bool, cx: &mut App) {
    let chosen: Vec<Card> = cards::all_cards().iter().filter(|c| c.in_deck(deck) && c.suspended() != pause).cloned().collect();
    if chosen.is_empty() {
        return;
    }
    let saved = cards::snapshot(&chosen.iter().map(|c| c.id().to_string()).collect::<Vec<_>>());
    cards::set_many_suspended(&chosen, pause);
    let message = if pause { trf("%@ Karten pausiert", &[&chosen.len().to_string()]) } else { trf("%@ Karten fortgesetzt", &[&chosen.len().to_string()]) };
    act(cx, |l| l.cards_changed(message, "pause", cards::since(saved)));
}

/// Sorts `Modul2` before `Modul10`.
fn natural_key(text: &str) -> String {
    let mut out = String::new();
    let mut digits = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            if !digits.is_empty() {
                out.push_str(&format!("{digits:0>8}"));
                digits.clear();
            }
            out.push(c);
        }
    }
    if !digits.is_empty() {
        out.push_str(&format!("{digits:0>8}"));
    }
    out
}

fn order_label(order: Order) -> String {
    match order {
        Order::Note => tr("In der Notiz"),
        Order::Due => tr("Nach Fälligkeit"),
        Order::Hardest => tr("Schwierigste zuerst"),
    }
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
        let saved = prefs::get();
        Self { deck: saved.card_deck, open: saved.card_open.into_iter().collect(), session: None, query, filter: Filter::All, order: Order::Note, selected: None, selected_place: None, marked: BTreeSet::new(), zone: Zone::Cards, preview: None, focus: cx.focus_handle(), list_scroll: UniformListScrollHandle::new(), deck_scroll: UniformListScrollHandle::new(), review_scroll: ScrollHandle::new(), cached: None, decks: None, all: Arc::new(Vec::new()), _subs: subs }
    }

    /// Takes the keyboard, so the keys of the page work right away.
    pub fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn select_deck(&mut self, deck: String, cx: &mut Context<Self>) {
        if self.deck != deck {
            self.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
        }
        self.deck = deck;
        self.clear_selection();
        self.remember_decks();
        cx.notify();
    }

    fn clear_selection(&mut self) {
        self.selected = None;
        self.preview = None;
        self.marked.clear();
    }

    fn remember_decks(&self) {
        let (deck, open) = (self.deck.clone(), self.open.iter().cloned().collect::<Vec<_>>());
        prefs::update(|p| {
            p.card_deck = deck;
            p.card_open = open;
        });
    }

    fn toggle_fold(&mut self, deck: &str, cx: &mut Context<Self>) {
        if !self.open.remove(deck) {
            self.open.insert(deck.to_string());
        }
        self.remember_decks();
        cx.notify();
    }

    fn deck_list(&mut self, cx: &App) -> Rc<Vec<DeckRow>> {
        let modules = state::read(cx, |l| l.config.as_ref().map_or(String::new(), |c| c.modules.iter().map(|m| format!("{}|{}|{}|{}", m.folder, m.name, m.code, m.color)).collect::<Vec<_>>().join("\n")));
        let key = (index::current().generation, self.open.iter().cloned().collect::<Vec<_>>(), modules);
        if let Some((known, rows)) = &self.decks
            && *known == key
        {
            return rows.clone();
        }
        let rows = Rc::new(deck_rows(&self.all, &self.open, cx));
        self.decks = Some((key, rows.clone()));
        rows
    }

    fn start(&mut self, deck: String, window: &mut Window, cx: &mut Context<Self>) {
        let mut session = Session::new(deck, None);
        Self::advance(&mut session, None);
        self.review_scroll.set_offset(point(px(0.), px(0.)));
        self.session = Some(session);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Goes through all cards of a deck once, in random order, leaving their schedule as it is.
    fn start_cram(&mut self, deck: String, window: &mut Window, cx: &mut Context<Self>) {
        let now = cards::now();
        let mut all: Vec<Card> = cards::all_cards().iter().filter(|c| c.in_deck(&deck) && !c.suspended()).cloned().collect();
        all.sort_by_key(|c| c.id().bytes().fold(now as u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64)));
        let mut session = Session::new(deck, Some(all));
        Self::advance(&mut session, None);
        self.review_scroll.set_offset(point(px(0.), px(0.)));
        self.session = Some(session);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Moves on to whatever is due now; the card just answered waits unless nothing else is left.
    fn advance(session: &mut Session, last: Option<&str>) {
        if let Some(rest) = &mut session.cram {
            let next = (!rest.is_empty()).then(|| rest.remove(0));
            session.left = Counts { new: 0, learning: 0, review: rest.len() };
            match next {
                Some(card) => session.show(card, false),
                None => session.current = None,
            }
            return;
        }
        let now = cards::now();
        let queue = cards::queue(&session.deck, now);
        session.left = cards::counts(&session.deck, now);
        let due_now = queue.iter().find(|c| Some(c.id()) != last && c.state.as_ref().is_none_or(|s| s.due <= now || c.phase() == Phase::Review));
        let next = due_now.or_else(|| queue.iter().find(|c| Some(c.id()) != last)).or_else(|| queue.first()).cloned();
        match next {
            Some(card) => session.show(card, false),
            None => session.current = None,
        }
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = &mut self.session {
            s.shown = true;
            s.shown_at = std::time::Instant::now();
            cx.notify();
        }
    }

    fn rate(&mut self, rating: Rating, cx: &mut Context<Self>) {
        self.answer(rating, false, cx);
    }

    fn answer(&mut self, rating: Rating, redoing: bool, cx: &mut Context<Self>) {
        let Some(s) = &mut self.session else { return };
        let Some(card) = s.current.clone().filter(|_| s.shown) else { return };
        if !redoing && s.shown_at.elapsed() < std::time::Duration::from_millis(120) {
            return;
        }
        if let Some(rest) = &mut s.cram {
            let again = rating == Rating::Again;
            if again {
                rest.push(card.clone());
            }
            s.crammed.push((card.clone(), again));
        } else {
            let done = cards::answer(&card, rating, cards::now());
            s.answered.push((done, Some(rating as usize - 1)));
        }
        if !redoing {
            s.undone.clear();
        }
        s.ratings[rating as usize - 1] += 1;
        Self::advance(s, Some(card.id()));
        self.review_scroll.set_offset(point(px(0.), px(0.)));
        state::lib(cx).update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn can_undo(&self) -> bool {
        self.session.as_ref().is_some_and(|s| !s.answered.is_empty() || !s.crammed.is_empty())
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &mut self.session else { return };
        if let Some(rest) = &mut s.cram {
            let Some((card, again)) = s.crammed.pop() else { return };
            if again {
                rest.pop();
            }
            let rating = if again { 0 } else { 2 };
            s.ratings[rating] = s.ratings[rating].saturating_sub(1);
            if let Some(current) = s.current.take() {
                rest.insert(0, current);
            }
            s.left = Counts { new: 0, learning: 0, review: rest.len() };
            s.show(card, true);
            cx.notify();
            return;
        }
        let Some((done, rating)) = s.answered.pop() else { return };
        let id = done.id.clone();
        match rating {
            Some(rating) => {
                s.ratings[rating] = s.ratings[rating].saturating_sub(1);
                s.undone.push((id.clone(), Rating::ALL[rating]));
            }
            None => s.undone.clear(),
        }
        cards::undo(done);
        let card = cards::all_cards().iter().find(|c| c.id() == id).cloned();
        match card {
            Some(card) => s.show(card, rating.is_some()),
            None => s.undone.clear(),
        }
        s.left = cards::counts(&s.deck, cards::now());
        self.review_scroll.set_offset(point(px(0.), px(0.)));
        state::lib(cx).update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        let next = self.session.as_mut().and_then(|s| {
            let ready = s.shown && s.current.as_ref().is_some_and(|c| s.undone.last().is_some_and(|(id, _)| *id == c.id()));
            if ready { s.undone.pop().map(|(_, rating)| rating) } else { s.undone.clear(); None }
        });
        match next {
            Some(rating) => self.answer(rating, true, cx),
            None => act(cx, |l| l.notify(tr("Nichts zu wiederholen"), "info", None, None)),
        }
    }

    fn set_aside(&mut self, suspend: bool, cx: &mut Context<Self>) {
        let Some(s) = &mut self.session else { return };
        let Some(card) = s.current.clone() else { return };
        if s.cram.is_some() {
            return;
        }
        s.answered.push((cards::before_change(&card), None));
        s.undone.clear();
        if suspend {
            cards::set_suspended(&card, true);
        } else {
            cards::bury(&card, cards::now());
        }
        Self::advance(s, Some(card.id()));
        self.review_scroll.set_offset(point(px(0.), px(0.)));
        state::lib(cx).update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    /// The cards an action is meant for: all marked ones, or just the selected one.
    fn chosen_cards(&self) -> Vec<Card> {
        if self.marked.len() > 1 {
            return self.all.iter().filter(|c| self.marked.contains(c.id())).cloned().collect();
        }
        self.selected.as_ref().and_then(|id| self.all.iter().find(|c| c.id() == id)).cloned().into_iter().collect()
    }

    /// Pauses what is chosen, or resumes it when all of it is paused. The toast takes it back.
    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        let chosen = self.chosen_cards();
        if chosen.is_empty() {
            return;
        }
        let pause = chosen.iter().any(|c| !c.suspended());
        let saved = cards::snapshot(&chosen.iter().map(|c| c.id().to_string()).collect::<Vec<_>>());
        cards::set_many_suspended(&chosen, pause);
        let message = match (pause, chosen.len()) {
            (true, 1) => tr("Karte pausiert"),
            (true, n) => trf("%@ Karten pausiert", &[&n.to_string()]),
            (false, 1) => tr("Karte fortgesetzt"),
            (false, n) => trf("%@ Karten fortgesetzt", &[&n.to_string()]),
        };
        act(cx, |l| l.cards_changed(message, "pause", cards::since(saved)));
        cx.notify();
    }

    fn reset_chosen(&mut self, cx: &mut Context<Self>) {
        let chosen = self.chosen_cards();
        if chosen.is_empty() {
            return;
        }
        let saved = cards::snapshot(&chosen.iter().map(|c| c.id().to_string()).collect::<Vec<_>>());
        cards::reset_many(&chosen);
        let message = if chosen.len() == 1 { tr("Fortschritt zurückgesetzt") } else { trf("Fortschritt von %@ Karten zurückgesetzt", &[&chosen.len().to_string()]) };
        act(cx, |l| l.cards_changed(message, "rotate-ccw", cards::since(saved)));
        cx.notify();
    }

    fn adopt_known(&mut self, deck: String, window: &mut Window, cx: &mut Context<Self>) {
        let count = cards::fresh_count(&deck);
        if count == 0 {
            return;
        }
        let me = cx.entity();
        window.focus(&self.focus, cx);
        let title = if count == 1 { tr("1 neue Karte als bekannt übernehmen?") } else { trf("%@ neue Karten als bekannt übernehmen?", &[&count.to_string()]) };
        super::sheets::ask(window, cx, title, tr("Sie zählen nicht mehr als neu und kommen als Wiederholungen, verteilt auf die nächsten Wochen. Pausierte Karten bleiben, wie sie sind."), tr("Als bekannt übernehmen"), false, move |cx| {
            me.update(cx, |p, cx| p.adopt_known_now(&deck, cx));
        });
    }

    fn adopt_known_now(&mut self, deck: &str, cx: &mut Context<Self>) {
        let Some(done) = cards::adopt_new_as_known(deck, cards::now()) else { return };
        act(cx, |l| l.cards_known(done));
        cx.notify();
    }

    /// Deletes what is chosen from its notes and moves on to the next card in the list.
    fn delete_chosen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = self.chosen_cards();
        if chosen.is_empty() {
            return;
        }
        if chosen.len() == 1 {
            self.delete_now(chosen, cx);
            return;
        }
        let (me, count) = (cx.entity(), chosen.len());
        window.focus(&self.focus, cx);
        super::sheets::ask(window, cx, trf("%@ Karten löschen?", &[&count.to_string()]), tr("Die Zeilen verschwinden aus den Notizen. ⌘Z bringt sie Notiz für Notiz zurück."), tr("Löschen"), true, move |cx| {
            let cards = chosen.clone();
            me.update(cx, |p, cx| p.delete_now(cards, cx));
        });
    }

    fn delete_now(&mut self, chosen: Vec<Card>, cx: &mut Context<Self>) {
        let now = cards::now();
        let rows = self.visible(now, cx);
        let gone: BTreeSet<&str> = chosen.iter().map(|c| c.id()).collect();
        let at = rows.iter().position(|i| gone.contains(self.all[*i].id()));
        let next = at.and_then(|at| rows[at..].iter().find(|i| !gone.contains(self.all[**i].id())).or_else(|| rows[..at].iter().rev().find(|i| !gone.contains(self.all[**i].id())))).map(|i| self.all[*i].id().to_string());
        delete_cards(&chosen, cx);
        self.clear_selection();
        if let Some(id) = next {
            self.select_card(id, cx);
        }
        cx.notify();
    }

    /// Selects a row: a plain click takes it alone, ⌘ adds or removes it, ⇧ takes everything up to it.
    fn click_row(&mut self, id: String, position: usize, modifiers: Modifiers, cx: &mut Context<Self>) {
        self.zone = Zone::Cards;
        if modifiers.shift {
            let rows = self.visible(cards::now(), cx);
            let from = self.selected.as_ref().and_then(|s| rows.iter().position(|i| self.all[*i].id() == s)).unwrap_or(position);
            let (a, b) = (from.min(position), from.max(position));
            self.marked = rows[a..=b].iter().map(|i| self.all[*i].id().to_string()).collect();
            cx.notify();
            return;
        }
        if modifiers.platform || modifiers.control {
            if self.marked.is_empty() {
                self.marked.extend(self.selected.clone());
            }
            if !self.marked.remove(&id) {
                self.marked.insert(id.clone());
            }
            self.select_card(id, cx);
            return;
        }
        self.marked.clear();
        if self.selected.as_deref() == Some(id.as_str()) {
            self.selected = None;
            self.preview = None;
            cx.notify();
            return;
        }
        self.select_card(id, cx);
    }

    fn open_source(card: &Card, cx: &mut App) {
        super::doc_side::open_at(card.note.as_path(), card.raw.lines.0, cx);
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
        let key = (Arc::as_ptr(&self.all) as usize, index::current().generation, cards::revision(), now / 60, self.deck.clone(), self.filter, self.order, query.clone());
        if let Some((k, rows, _)) = &self.cached
            && *k == key
        {
            return rows.clone();
        }
        let vault = index::current();
        let tag = query.strip_prefix('#').map(str::to_string);
        let fits = |c: &Card| match &tag {
            Some(tag) => vault.notes.get(c.note.as_path()).is_some_and(|f| f.tags.iter().any(|t| fold(t).starts_with(tag.as_str()))),
            None => query.is_empty() || fold(&c.raw.front).contains(&query) || fold(&c.raw.back).contains(&query),
        };
        let mut amounts = [0usize; 6];
        let mut rows: Vec<usize> = Vec::new();
        for (i, card) in self.all.iter().enumerate() {
            if !card.in_deck(&self.deck) || !fits(card) {
                continue;
            }
            for (slot, filter) in Filter::ALL.iter().enumerate() {
                if filter.keeps(card, now) {
                    amounts[slot] += 1;
                }
            }
            if self.filter.keeps(card, now) {
                rows.push(i);
            }
        }
        match self.order {
            Order::Note => {}
            Order::Due => rows.sort_by_key(|i| self.all[*i].state.as_ref().filter(|s| s.phase != Phase::New).map_or(i64::MAX, |s| s.due)),
            Order::Hardest => rows.sort_by(|a, b| {
                let hard = |i: &usize| self.all[*i].state.as_ref().map_or(0., |s| s.difficulty + s.lapses as f64);
                hard(b).total_cmp(&hard(a))
            }),
        }
        let rows = Rc::new(rows);
        self.cached = Some((key, rows.clone(), amounts));
        rows
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let key = e.keystroke.key.as_str();
        let m = &e.keystroke.modifiers;
        if self.session.is_some() {
            if m.platform || m.control || m.alt || e.is_held && !matches!(key, "up" | "down" | "pageup" | "pagedown") {
                return;
            }
            self.session_key(key, window, cx);
            return;
        }
        if (m.platform || m.control) && key == "a" {
            let rows = self.visible(cards::now(), cx);
            self.marked = rows.iter().map(|i| self.all[*i].id().to_string()).collect();
            self.zone = Zone::Cards;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if m.platform || m.control || m.alt {
            return;
        }
        match (self.zone, key) {
            (_, "n") => self.compose(None, window, cx),
            (_, "i") => choose_import(window, cx),
            (_, "/") => self.query.update(cx, |q, cx| q.focus(window, cx)),
            (Zone::Decks, _) => {
                if !self.decks_key(key, window, cx) {
                    return;
                }
            }
            (Zone::Cards, _) => {
                if !self.cards_key(key, m.shift, window, cx) {
                    return;
                }
            }
        }
        cx.stop_propagation();
    }

    fn session_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.session.as_ref().is_some_and(|s| s.shown);
        let has_card = self.session.as_ref().is_some_and(|s| s.current.is_some());
        let cramming = self.session.as_ref().is_some_and(|s| s.cram.is_some());
        match key {
            "space" | "enter" if has_card && !shown => self.reveal(cx),
            "space" | "enter" if has_card => self.rate(Rating::Good, cx),
            "1" if shown => self.rate(Rating::Again, cx),
            "2" if shown && !cramming => self.rate(Rating::Hard, cx),
            "3" if shown => self.rate(Rating::Good, cx),
            "4" if shown && !cramming => self.rate(Rating::Easy, cx),
            "enter" | "space" => self.end_session(cx),
            "up" | "down" | "pageup" | "pagedown" => {
                let step = match key {
                    "up" => 48.,
                    "down" => -48.,
                    "pageup" => 320.,
                    _ => -320.,
                };
                let now = self.review_scroll.offset();
                let lowest = -self.review_scroll.max_offset().y;
                self.review_scroll.set_offset(point(now.x, (now.y + px(step)).clamp(lowest, px(0.))));
                cx.notify();
            }
            "e" | "o" => {
                if let Some(card) = self.session.as_ref().and_then(|s| s.current.clone()) {
                    if key == "e" {
                        self.compose(Some(card), window, cx);
                    } else {
                        Self::open_source(&card, cx);
                    }
                }
            }
            "s" if has_card => self.set_aside(true, cx),
            "b" if has_card => self.set_aside(false, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn end_session(&mut self, cx: &mut Context<Self>) {
        self.session = None;
        self.cached = None;
        cx.notify();
    }

    /// Arrow keys in the deck tree: up and down walk it, right opens or moves on to the cards, left closes or goes up.
    fn decks_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let rows = self.deck_list(cx);
        let at = rows.iter().position(|r| r.deck == self.deck);
        let current = at.map(|i| rows[i].clone());
        match key {
            "down" => {
                let next = at.map_or(0, |i| (i + 1).min(rows.len().saturating_sub(1)));
                if let Some(row) = rows.get(next) {
                    self.deck_scroll.scroll_to_item(next, ScrollStrategy::Center);
                    self.select_deck(row.deck.clone(), cx);
                }
            }
            "up" => match at {
                Some(0) | None => self.select_deck(String::new(), cx),
                Some(i) => {
                    self.deck_scroll.scroll_to_item(i - 1, ScrollStrategy::Center);
                    self.select_deck(rows[i - 1].deck.clone(), cx);
                }
            },
            "right" => match current.filter(|r| r.has_children && !r.open) {
                Some(row) => self.toggle_fold(&row.deck, cx),
                None => {
                    self.zone = Zone::Cards;
                    cx.notify();
                }
            },
            "left" => {
                if let Some(row) = current {
                    if row.open {
                        self.toggle_fold(&row.deck, cx);
                    } else {
                        let up = row.deck.rsplit_once('/').map_or(String::new(), |(up, _)| up.to_string());
                        self.select_deck(up, cx);
                    }
                }
            }
            "enter" | "space" => {
                let deck = self.deck.clone();
                if cards::counts(&deck, cards::now()).total() > 0 {
                    self.start(deck, window, cx);
                }
            }
            "escape" => {
                self.zone = Zone::Cards;
                cx.notify();
            }
            _ => return false,
        }
        true
    }

    /// Keys of the card list.
    fn cards_key(&mut self, key: &str, shift: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let now = cards::now();
        match key {
            "space" => {
                let deck = self.deck.clone();
                if cards::counts(&deck, now).total() > 0 {
                    self.start(deck, window, cx);
                }
            }
            "enter" | "e" => match self.selected.as_ref().and_then(|id| self.all.iter().find(|c| c.id() == id)).cloned() {
                Some(card) => self.compose(Some(card), window, cx),
                None if key == "enter" => {
                    let deck = self.deck.clone();
                    if cards::counts(&deck, now).total() > 0 {
                        self.start(deck, window, cx);
                    }
                }
                None => return false,
            },
            "s" => self.toggle_pause(cx),
            "backspace" | "delete" => self.delete_chosen(window, cx),
            "left" => {
                self.zone = Zone::Decks;
                cx.notify();
            }
            "down" | "up" | "home" | "end" | "pagedown" | "pageup" => {
                let rows = self.visible(now, cx);
                if rows.is_empty() {
                    return false;
                }
                let last = rows.len() - 1;
                let at = self.selected.as_ref().and_then(|id| rows.iter().position(|i| self.all[*i].id() == id));
                let next = match (at, key) {
                    (_, "home") => 0,
                    (_, "end") => last,
                    (Some(i), "down") => (i + 1).min(last),
                    (Some(i), "up") => i.saturating_sub(1),
                    (Some(i), "pagedown") => (i + 15).min(last),
                    (Some(i), "pageup") => i.saturating_sub(15),
                    (None, _) => 0,
                    (Some(i), _) => i,
                };
                let id = self.all[rows[next]].id().to_string();
                self.list_scroll.scroll_to_item(next, ScrollStrategy::Center);
                if shift {
                    self.marked.extend(self.selected.clone());
                    self.marked.insert(id.clone());
                } else {
                    self.marked.clear();
                }
                self.select_card(id, cx);
            }
            _ => return false,
        }
        true
    }

    fn deck_column(&mut self, now: i64, t: &Theme, cx: &mut Context<Self>) -> Div {
        let list = self.deck_list(cx);
        let me = cx.entity();
        let current = self.deck.clone();
        let deck_zone = self.zone == Zone::Decks;
        let ring = t.accent;
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
            .when(current.is_empty() && deck_zone, |d| d.border_l_2().border_color(ring))
            .when(!current.is_empty(), |d| d.soft(hover, 0.))
            .child(icon("layers", iconsize::S, t.secondary))
            .child(div().flex_1().min_w_0().truncate().child(tr("Alle Karten")))
            .child(counts_pills(all_counts, t))
            .tooltip(tip(deck_help(total, all_counts)))
            .on_click(cx.listener(|p, _, _, cx| {
                p.zone = Zone::Decks;
                p.select_deck(String::new(), cx)
            }));
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
                    let in_zone = active && me.read(cx).zone == Zone::Decks;
                    let waiting = cards::counts(&row.deck, now);
                    let (right_me, right_deck) = (me.clone(), row.deck.clone());
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
                        .when(in_zone, |d| d.border_l_2().border_color(t.accent))
                        .when(!active, |d| d.soft(hover, 0.))
                        .on_mouse_down(MouseButton::Right, move |_, _, cx| right_me.update(cx, |p, cx| p.select_deck(right_deck.clone(), cx)))
                        .child(if row.has_children {
                            div()
                                .id(("deck-fold", i))
                                .flex_none()
                                .size(px(iconsize::M))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .rounded(px(radius::SMALL))
                                .soft(t.ink.opacity(0.08), radius::SMALL)
                                .child(chevron(("deck-chevron", i), row.open, t.secondary))
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    a.update(cx, |p, cx| p.toggle_fold(&deck_a, cx));
                                })
                                .into_any_element()
                        } else {
                            div().flex_none().size(px(iconsize::M)).into_any_element()
                        })
                        .child(chip)
                        .child(div().flex_1().min_w_0().truncate().child(row.name.clone()))
                        .child(counts_pills(waiting, &t))
                        .tooltip(tip(deck_help(row.total, waiting)))
                        .on_click(move |_, _, cx| {
                            b.update(cx, |p, cx| {
                                p.zone = Zone::Decks;
                                p.select_deck(deck_b.clone(), cx)
                            })
                        })
                        .context_menu(move |menu, _, _| {
                            let (start, new, deck_start, deck_new) = (m_c.clone(), m_d.clone(), deck_c.clone(), deck_d.clone());
                            let (cram, deck_cram, pause, deck_pause, resume, deck_resume) = (m_c.clone(), deck_c.clone(), m_c.clone(), deck_c.clone(), m_c.clone(), deck_c.clone());
                            let (known, deck_known, fresh) = (m_c.clone(), deck_c.clone(), cards::fresh_count(&deck_c));
                            menu.item(PopupMenuItem::new(tr("Lernen")).icon(menu_icon("graduation-cap")).on_click(move |_, window, cx| {
                                let deck = deck_start.clone();
                                start.update(cx, |p, cx| p.start(deck, window, cx));
                            }))
                            .item(PopupMenuItem::new(tr("Alles wiederholen")).icon(menu_icon("repeat")).on_click(move |_, window, cx| {
                                let deck = deck_cram.clone();
                                cram.update(cx, |p, cx| p.start_cram(deck, window, cx));
                            }))
                            .when(fresh > 0, |m| m.item(PopupMenuItem::new(tr("Neue Karten als bekannt übernehmen…")).icon(menu_icon("calendar-check")).on_click(move |_, window, cx| {
                                let deck = deck_known.clone();
                                known.update(cx, |p, cx| p.adopt_known(deck, window, cx));
                            })))
                            .item(PopupMenuItem::new(tr("Alle pausieren")).icon(menu_icon("pause")).on_click(move |_, _, cx| {
                                deck_pause_all(&deck_pause, true, cx);
                                pause.update(cx, |_, cx| cx.notify());
                            }))
                            .item(PopupMenuItem::new(tr("Alle fortsetzen")).icon(menu_icon("rotate-ccw")).on_click(move |_, _, cx| {
                                deck_pause_all(&deck_resume, false, cx);
                                resume.update(cx, |_, cx| cx.notify());
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
        .track_scroll(&self.deck_scroll)
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
            .bg(t.ink.opacity(0.04))
            .child(div().text_size(px(font::CAPTION)).text_color(t.secondary).truncate().child(label))
            .child(div().text_size(px(font::TITLE)).font_weight(FontWeight::SEMIBOLD).font_features(tabular()).truncate().child(value))
            .children(detail.map(|d| div().text_size(px(font::CAPTION)).text_color(t.tertiary).truncate().child(d)))
    }

    fn forecast(stats: &cards::Stats, backlog: usize, t: &Theme) -> Div {
        let shown: Vec<usize> = stats.forecast.iter().enumerate().map(|(i, n)| if i == 0 { n.saturating_sub(backlog) } else { *n }).collect();
        let most = shown.iter().copied().max().unwrap_or(0).max(1);
        let today = chrono::Local::now().date_naive();
        let bars = shown.iter().enumerate().map(|(i, n)| {
            let share = *n as f32 / most as f32;
            let day = today + chrono::Days::new(i as u64);
            let label = if i == 0 { tr("Heute") } else { crate::core::day::weekday_short(day).to_string() };
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
            .child(
                div()
                    .flex()
                    .gap(px(space::S))
                    .child(Button::new("cards-import-first").icon(menu_icon("download")).label(tr("Aus Anki importieren…")).on_click(|_, window, cx| choose_import(window, cx)))
                    .child(Button::new("cards-first").primary().label(tr("Erste Karte schreiben")).on_click(cx.listener(|p, _, window, cx| p.compose(None, window, cx)))),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn card_row(card: &Card, i: usize, selected: bool, marked: bool, now: i64, me: Entity<Self>, t: &Theme) -> AnyElement {
        let hover = t.ink.opacity(0.05);
        let id = card.id().to_string();
        let status: (String, Hsla) = if card.suspended() {
            (tr("Pausiert"), t.tertiary)
        } else if card.buried(now) {
            (tr("Morgen"), t.tertiary)
        } else if card.is_leech() {
            (tr("Schwierig"), t.orange)
        } else {
            match card.state.as_ref().filter(|s| s.phase != Phase::New) {
                None => (tr("Neu"), t.blue),
                Some(s) if s.due <= now || (s.phase == Phase::Review && s.due < cards::next_day_start(now)) => (tr("Fällig"), t.green),
                Some(s) => (cards::wait_label(now, s.due), t.secondary),
            }
        };
        let menu_card = card.clone();
        let (m1, m2, m3) = (me.clone(), me.clone(), me.clone());
        let right_id = id.clone();
        let name = Path::new(card.note_rel.as_str()).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let quiet = card.suspended() || card.buried(now);
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
            .when(marked, |d| d.bg(t.accent.opacity(0.18)))
            .when(selected && !marked, |d| d.bg(t.ink.opacity(0.08)))
            .when(!selected && !marked, |d| d.soft(hover, 0.))
            .child(icon(cards::kind_icon(card.raw.kind), iconsize::S, t.tertiary))
            .child(div().flex_1().min_w_0().truncate().when(quiet, |d| d.text_color(t.tertiary)).child(cards::plain_text(&card.raw.front)))
            .child(caption(name, t).flex_none().max_w(px(140.)).truncate())
            .child(div().flex_none().w(px(64.)).text_right().text_size(px(font::CAPTION)).text_color(status.1).child(status.0))
            .on_mouse_down(MouseButton::Right, move |_, _, cx| {
                let id = right_id.clone();
                m3.update(cx, |p, cx| {
                    p.zone = Zone::Cards;
                    if !p.marked.contains(&id) {
                        p.marked.clear();
                        if p.selected.as_deref() != Some(id.as_str()) {
                            p.select_card(id, cx);
                        }
                    }
                })
            })
            .on_click(move |e, window, cx| {
                let id = id.clone();
                let double = e.click_count() >= 2;
                let modifiers = e.modifiers();
                m1.update(cx, |p, cx| {
                    window.focus(&p.focus, cx);
                    if double {
                        p.zone = Zone::Cards;
                        p.select_card(id.clone(), cx);
                        let card = p.all.iter().find(|c| c.id() == id).cloned();
                        p.compose(card, window, cx);
                    } else {
                        p.click_row(id, i, modifiers, cx);
                    }
                });
            })
            .context_menu(move |menu, _, _| Self::card_menu(menu, menu_card.clone(), m2.clone()))
            .into_any_element()
    }

    fn card_menu(menu: gpui_kit::component::menu::PopupMenu, card: Card, me: Entity<Self>) -> gpui_kit::component::menu::PopupMenu {
        let many = MARKED_COUNT.with(std::cell::Cell::get);
        let suspended = card.suspended();
        let buried = card.buried(cards::now());
        let (c1, c2, c6) = (card.clone(), card.clone(), card);
        let (m1, m2, m3, m4) = (me.clone(), me.clone(), me.clone(), me);
        let mut menu = menu;
        if many <= 1 {
            menu = menu
                .item(PopupMenuItem::new(tr("Bearbeiten")).icon(menu_icon("pencil")).on_click(move |_, window, cx| {
                    let card = c1.clone();
                    m1.update(cx, |p, cx| p.compose(Some(card), window, cx));
                }))
                .item(PopupMenuItem::new(tr("In der Notiz zeigen")).icon(menu_icon("file-text")).on_click(move |_, _, cx| Self::open_source(&c2, cx)))
                .separator();
            if buried {
                menu = menu.item(PopupMenuItem::new(tr("Heute wieder lernen")).icon(menu_icon("eye")).on_click(move |_, _, cx| {
                    cards::unbury(&c6);
                    state::lib(cx).update(cx, |_, cx| cx.notify());
                }));
            }
        }
        let label = |one: String, more: &str| if many > 1 { trf(more, &[&many.to_string()]) } else { one };
        menu.item(PopupMenuItem::new(if suspended { label(tr("Fortsetzen"), "%@ Karten fortsetzen") } else { label(tr("Pausieren"), "%@ Karten pausieren") }).icon(menu_icon("pause")).on_click(move |_, _, cx| m2.update(cx, |p, cx| p.toggle_pause(cx))))
            .item(PopupMenuItem::new(label(tr("Fortschritt zurücksetzen"), "Fortschritt von %@ Karten zurücksetzen")).icon(menu_icon("rotate-ccw")).on_click(move |_, _, cx| m3.update(cx, |p, cx| p.reset_chosen(cx))))
            .separator()
            .item(PopupMenuItem::new(label(tr("Aus der Notiz löschen"), "%@ Karten löschen")).icon(menu_icon("trash-2")).on_click(move |_, window, cx| m4.update(cx, |p, cx| p.delete_chosen(window, cx))))
    }

    /// Several marked cards: what can be done to all of them.
    fn many_panel(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let count = self.marked.len();
        div()
            .id("cards-many")
            .w(px(320.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(space::M))
            .p(px(space::L))
            .border_l_1()
            .border_color(t.separator)
            .child(div().flex().items_center().child(div().flex_1().text_size(px(font::TITLE3)).font_weight(FontWeight::SEMIBOLD).child(trf("%@ Karten ausgewählt", &[&count.to_string()]))).child(Button::new("many-close").ghost().small().icon(menu_icon("x")).tooltip(tr("Auswahl aufheben (Esc)")).on_click(cx.listener(|p, _, _, cx| {
                p.clear_selection();
                cx.notify();
            }))))
            .child(Button::new("many-pause").icon(menu_icon("pause")).label(tr("Pausieren oder fortsetzen")).tooltip(tr("S")).on_click(cx.listener(|p, _, _, cx| p.toggle_pause(cx))))
            .child(Button::new("many-reset").icon(menu_icon("rotate-ccw")).label(tr("Fortschritt zurücksetzen")).on_click(cx.listener(|p, _, _, cx| p.reset_chosen(cx))))
            .child(Button::new("many-delete").icon(menu_icon("trash-2")).label(tr("Löschen")).tooltip(tr("⌫")).on_click(cx.listener(|p, _, window, cx| p.delete_chosen(window, cx))))
            .child(caption(tr("⌘-Klick nimmt einzelne Karten dazu, ⇧-Klick einen Bereich, ⌘A alle in der Liste."), t))
            .into_any_element()
    }

    fn detail_panel(&self, card: &Card, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let base = card.note.parent().map(Path::to_path_buf);
        let now = cards::now();
        let fact = |label: String, value: String| div().flex().items_center().justify_between().text_size(px(font::CALLOUT)).child(div().text_color(t.secondary).child(label)).child(div().font_features(tabular()).child(value));
        let mut facts = div().flex().flex_col().gap(px(space::XS));
        match card.state.as_ref().filter(|s| s.phase != Phase::New) {
            Some(s) => {
                let days = s.last_review.map_or(0., |l| ((now - l) as f64 / 86400.).max(0.));
                facts = facts
                    .child(fact(tr("Fällig"), if s.due <= now { tr("Jetzt") } else { cards::wait_label(now, s.due) }))
                    .child(fact(tr("Erinnerung jetzt"), format!("{:.0} %", fsrs::retrievability(s.stability, days) * 100.)))
                    .child(fact(tr("Stabilität"), trf("%@ Tage", &[&format!("{:.0}", s.stability.max(1.))])))
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
            .child(div().flex_none().h(px(control::SUBBAR)).px(px(space::M)).flex().items_center().border_b_1().border_color(t.separator).child(div().flex_1().min_w_0().truncate().text_size(px(font::CALLOUT)).text_color(t.secondary).child(card_path(card))).child(Button::new("card-close").ghost().small().icon(menu_icon("x")).tooltip(tr("Schließen (Esc)")).on_click(cx.listener(|p, _, _, cx| {
                p.clear_selection();
                cx.notify();
            }))))
            .child(div().id("card-detail-body").flex_1().min_h_0().overflow_y_scrollbar().p(px(space::L)).flex().flex_col().gap(px(space::L)).children(body).child(div().h(px(1.)).flex_none().bg(t.separator)).child(facts))
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

    fn overview(&mut self, now: i64, room: f32, switcher: bool, height: f32, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let room = room - if self.selected.is_some() || self.marked.len() > 1 { 320. } else { 0. };
        let deck = self.deck.clone();
        let counts = cards::counts(&deck, now);
        let stats = cards::stats(&deck, now);
        let total = counts.total();
        let title = deck_title(&deck, cx);
        let next = cards::next_due(&deck, now);
        let held_back = if total == 0 { cards::new_waiting(&deck, now) } else { 0 };
        let status = if stats.total == 0 {
            tr("Keine Karten")
        } else if total > 0 {
            waiting_words(counts)
        } else if stats.suspended == stats.total {
            tr("Alle Karten sind pausiert.")
        } else if held_back > 0 {
            trf("Tageslimit erreicht. %@ neue warten.", &[&held_back.to_string()])
        } else if let Some(at) = next {
            trf("Nächste Karte in %@", &[&in_time(now, at)])
        } else {
            tr("Alles gelernt.")
        };
        let exam = state::read(cx, |l| {
            let module = l.config.as_ref()?.modules.iter().find(|m| m.folder == deck)?.id.clone();
            l.semester_assignments().iter().filter(|a| a.module_id == module && a.kind == crate::core::model::Kind::Exam && a.due.timestamp() > now).map(|a| a.due.timestamp()).min()
        });
        let status = match exam {
            Some(at) => {
                let days = ((at - now) as f64 / 86_400.).ceil().max(1.) as usize;
                let open = stats.new + stats.young;
                let pace = if open > 0 { format!(" · {}", trf("%@ Karten noch offen, etwa %@ pro Tag", &[&open.to_string(), &open.div_ceil(days).to_string()])) } else { String::new() };
                format!("{status} · {}{pace}", trf("Prüfung in %@", &[&in_time(now, at)]))
            }
            None => status,
        };
        let backlog = stats.forecast.first().copied().unwrap_or(0).saturating_sub(counts.learning + counts.review);
        let status = if backlog > 0 { format!("{status} · {}", trf("%@ weitere folgen an den nächsten Tagen", &[&backlog.to_string()])) } else { status };
        let (start_deck, cram_deck) = (deck.clone(), deck.clone());
        let head = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(space::M))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).truncate().child(title)).child(div().text_color(t.secondary).truncate().child(if stats.total == 0 { status } else { format!("{} · {status}", if stats.total == 1 { tr("1 Karte") } else { trf("%@ Karten", &[&stats.total.to_string()]) }) })))
            .when(switcher, |d| {
                let (me, all, current) = (cx.entity(), self.all.clone(), deck.clone());
                d.child(Button::new("cards-deck").ghost().icon(menu_icon("layers")).label(tr("Stapel")).dropdown_caret(true).tooltip(tr("Stapel wählen")).dropdown_menu(move |menu, _, cx| {
                    let (e, now) = (me.clone(), cards::now());
                    let mut menu = menu.item(PopupMenuItem::new(format!("{}  {}", tr("Alle Karten"), counts_text(cards::counts("", now)))).checked(current.is_empty()).on_click({
                        let e = e.clone();
                        move |_, _, cx| e.update(cx, |p, cx| p.select_deck(String::new(), cx))
                    }));
                    for row in deck_rows(&all, &every_deck(&all), cx) {
                        let (e, deck) = (e.clone(), row.deck.clone());
                        let label = format!("{}{}  {}", "\u{2003}".repeat(row.depth), row.name, counts_text(cards::counts(&row.deck, now)));
                        menu = menu.item(PopupMenuItem::new(label).checked(row.deck == current).on_click(move |_, _, cx| e.update(cx, |p, cx| p.select_deck(deck.clone(), cx))));
                    }
                    menu
                }))
            })
            .child(Button::new("cards-more").ghost().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu({
                let me = cx.entity();
                move |menu, _, _| {
                    let (e, deck) = (me.clone(), cram_deck.clone());
                    let (known_me, known_deck, fresh) = (me.clone(), cram_deck.clone(), cards::fresh_count(&cram_deck));
                    menu.item(PopupMenuItem::new(tr("Alles wiederholen")).icon(menu_icon("repeat")).on_click(move |_, window, cx| {
                        let deck = deck.clone();
                        e.update(cx, |p, cx| p.start_cram(deck, window, cx));
                    }))
                    .when(fresh > 0, |m| m.item(PopupMenuItem::new(tr("Neue Karten als bekannt übernehmen…")).icon(menu_icon("calendar-check")).on_click(move |_, window, cx| known_me.update(cx, |p, cx| p.adopt_known(known_deck.clone(), window, cx)))))
                    .separator()
                    .item(PopupMenuItem::new(tr("Aus Anki importieren…")).icon(menu_icon("download")).on_click(|_, window, cx| choose_import(window, cx)))
                }
            }))
            .when(held_back > 0, |d| {
                let deck = deck.clone();
                d.child(Button::new("cards-limit").icon(menu_icon("plus")).label(trf("%@ neue mehr", &[&held_back.min(10).to_string()])).tooltip(tr("Heute mehr neue Karten als das Tageslimit")).on_click(cx.listener(move |p, _, window, cx| {
                    cards::allow_more_new(held_back.min(10), cards::now());
                    p.start(deck.clone(), window, cx);
                })))
            })
            .child(Button::new("cards-new").icon(menu_icon("plus")).label(tr("Neue Karte")).tooltip(tr("Neue Karte (N)")).on_click(cx.listener(|p, _, window, cx| p.compose(None, window, cx))))
            .child(Button::new("cards-learn").primary().min_w(px(control::COUNTED)).icon(menu_icon("graduation-cap")).label(if total > 0 { trf("Lernen (%@)", &[&total.to_string()]) } else { tr("Lernen") }).tooltip(tr("Lernen (Leertaste)")).disabled(total == 0).on_click(cx.listener(move |p, _, window, cx| p.start(start_deck.clone(), window, cx))));
        let retention = stats.true_retention.map_or_else(|| "–".to_string(), |r| format!("{:.0} %", r * 100.));
        let streak = if stats.streak == 1 { tr("1 Tag") } else { trf("%@ Tage", &[&stats.streak.to_string()]) };
        let tiles = div()
            .flex_none()
            .flex()
            .gap(px(space::M))
            .child(Self::stat_tile(tr("Heute gelernt"), stats.reviewed_today.to_string(), None, t))
            .child(Self::stat_tile(tr("Trefferquote"), retention.clone(), Some(tr("letzte 30 Tage")), t))
            .when(stats.streak >= 3, |d| d.child(Self::stat_tile(tr("Serie"), streak.clone(), None, t)))
            .when(stats.mature > 0, |d| d.child(Self::stat_tile(tr("Gefestigt"), stats.mature.to_string(), Some(trf("von %@", &[&stats.total.to_string()])), t)));
        let line = {
            let mut parts = vec![format!("{} {}", tr("Heute gelernt"), stats.reviewed_today), format!("{} {retention}", tr("Trefferquote"))];
            if stats.streak >= 3 {
                parts.push(format!("{} {streak}", tr("Serie")));
            }
            caption(parts.join(" · "), t).truncate()
        };
        let forecast = Self::forecast(&stats, backlog, t);
        let rows = self.visible(now, cx);
        let me = cx.entity();
        let selected = self.selected.clone();
        let marked = Rc::new(self.marked.clone());
        MARKED_COUNT.with(|c| c.set(self.marked.len()));
        let amounts = self.cached.as_ref().map_or([0; 6], |c| c.2);
        let all = self.all.clone();
        let list: AnyElement = if rows.is_empty() {
            unavailable("search", tr("Keine Karten"), tr("Ändere den Filter oder die Suche."), t).into_any_element()
        } else {
            let (count, shown) = (rows.len(), rows.clone());
            let listed = all.clone();
            uniform_list("cards-list", count, move |range, _, cx| {
                let t = Theme::get(cx);
                range.filter_map(|i| { let card = listed.get(shown[i])?; Some(Self::card_row(card, i, selected.as_deref() == Some(card.id()), marked.contains(card.id()), now, me.clone(), &t)) }).collect::<Vec<_>>()
            })
            .track_scroll(&self.list_scroll)
            .size_full()
            .into_any_element()
        };
        let current_filter = self.filter;
        let current_order = self.order;
        let filters = div().flex().flex_wrap().gap(px(space::XXS)).children(Filter::ALL.iter().enumerate().filter(|(slot, f)| **f == Filter::All || **f == current_filter || amounts[*slot] > 0).map(|(slot, f)| {
            let (f, on) = (*f, *f == current_filter);
            div().id(SharedString::from(format!("filter-{f:?}"))).px(px(space::S)).h(px(control::S)).flex().items_center().rounded(px(radius::INNER)).cursor_pointer().text_size(px(font::CALLOUT)).when(on, |d| d.bg(t.ink.opacity(0.1))).when(!on, |d| d.text_color(t.secondary).soft(t.ink.opacity(0.05), radius::INNER)).child(f.label()).when(f != Filter::All, |d| d.child(div().ml(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.tertiary).font_features(tabular()).child(amounts[slot].to_string()))).on_click(cx.listener(move |p, _, window, cx| {
                p.filter = f;
                p.clear_selection();
                window.focus(&p.focus, cx);
                cx.notify();
            }))
        }));
        let narrow = room < 640.;
        let browser = div().flex_1().min_h_0().flex().flex_col().child(div().flex_none().px(px(space::L)).py(px(space::S)).flex().flex_wrap().items_center().gap(px(space::S)).border_t_1().border_b_1().border_color(t.separator).child(div().w(px(if narrow { 170. } else { 220. })).child(Input::new(&self.query).small().cleanable(true).prefix(icon("search", iconsize::XS, t.secondary)))).child(filters).child(div().flex_1()).child(Button::new("cards-order").ghost().small().when(narrow, |b| b.icon(menu_icon("arrow-up-down"))).when(!narrow, |b| b.label(order_label(current_order)).dropdown_caret(true)).tooltip(trf("Sortierung: %@", &[&order_label(current_order)])).dropdown_menu({
                    let me = cx.entity();
                    move |mut menu, _, _| {
                        for choice in [Order::Note, Order::Due, Order::Hardest] {
                            let e = me.clone();
                            menu = menu.item(PopupMenuItem::new(order_label(choice)).checked(choice == current_order).on_click(move |_, _, cx| e.update(cx, |p, cx| {
                                p.order = choice;
                                cx.notify();
                            })));
                        }
                        menu
                    }
                }))).child(div().flex_1().min_h_0().w_full().max_w(px(960.)).child(list));
        let chosen = self.selected.as_ref().filter(|_| self.marked.len() <= 1).and_then(|id| self.all.iter().find(|c| c.id() == id)).cloned();
        let many = (self.marked.len() > 1).then(|| self.many_panel(t, cx));
        let tight = room < 700. || height < 640.;
        let summary: Option<AnyElement> = if room >= 900. && height >= 640. {
            Some(div().flex().gap(px(space::XL)).child(div().flex_1().min_w_0().child(tiles)).child(div().w(px(340.)).flex_none().child(forecast)).into_any_element())
        } else if !tight {
            Some(tiles.into_any_element())
        } else {
            Some(line.into_any_element())
        };
        let main = div().flex_1().min_w_0().h_full().flex().flex_col().child(div().flex_none().p(px(if tight { space::L } else { space::XL })).pb(px(space::L)).flex().flex_col().gap(px(space::L)).child(head).children(summary)).child(browser);
        div().size_full().flex().child(main).when_some(chosen, |d, card| d.child(self.detail_panel(&card, t, cx))).children(many).into_any_element()
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
        let legend = div().flex().flex_wrap().justify_center().gap(px(space::L)).children(Rating::ALL.iter().enumerate().map(|(i, rating)| div().flex().items_center().gap(px(space::XS)).text_size(px(font::CALLOUT)).child(div().size(px(space::S)).rounded_full().bg(colors[i])).child(rating_name(*rating)).child(div().text_color(t.secondary).font_features(tabular()).child(s.ratings[i].to_string()))));
        let can_undo = !s.answered.is_empty() || !s.crammed.is_empty();
        let cramming = s.cram.is_some();
        let more = if cramming { 0 } else { cards::new_waiting(&deck, now) };
        let next = next.filter(|_| !cramming);
        let more_deck = deck.clone();
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
            .when(reviewed > 0, |d| d.child(bar).when(!cramming, |d| d.child(legend)))
            .children(next.map(|at| caption(trf("Nächste Karte in %@", &[&in_time(now, at)]), t)))
            .child(
                div()
                    .flex()
                    .gap(px(space::S))
                    .when(can_undo, |d| d.child(Button::new("cards-undo-last").icon(menu_icon("undo-2")).label(tr("Letzte Antwort zurück")).on_click(cx.listener(|p, _, _, cx| p.undo(cx)))))
                    .when(more > 0, |d| {
                        d.child(Button::new("cards-more-new").icon(menu_icon("plus")).label(trf("%@ neue mehr", &[&more.min(10).to_string()])).tooltip(tr("Heute mehr neue Karten als das Tageslimit")).on_click(cx.listener(move |p, _, window, cx| {
                            cards::allow_more_new(more.min(10), cards::now());
                            let deck = more_deck.clone();
                            p.start(deck, window, cx);
                        })))
                    })
                    .child(Button::new("cards-done").primary().label(tr("Fertig")).on_click(cx.listener(|p, _, _, cx| p.end_session(cx)))),
            )
            .into_any_element()
    }

    fn review(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(s) = &self.session else { return div().into_any_element() };
        let Some(card) = s.current.clone() else { return self.done_screen(t, cx) };
        let answered: usize = s.ratings.iter().sum();
        let progress = answered as f32 / (answered + s.left.total()).max(1) as f32;
        let phase = card.phase();
        let now = cards::now();
        let base = card.note.parent().map(Path::to_path_buf);
        let front = super::theme::with_note_scale(STUDY_SCALE, || markdown_view::render(&s.front, base.as_deref(), None, t));
        let back = s.shown.then(|| super::theme::with_note_scale(STUDY_SCALE, || markdown_view::render(&s.back, base.as_deref(), None, t)));
        let shown = s.shown;
        let can_undo = !s.answered.is_empty() || !s.crammed.is_empty();
        let cramming = s.cram.is_some();
        let count_piece = |n: usize, color: Hsla, on: bool, label: String| div().px(px(space::S)).h(px(control::S)).flex().items_center().rounded(px(radius::INNER)).when(on, |d| d.bg(color.opacity(0.12))).text_size(px(font::CALLOUT)).font_weight(FontWeight::MEDIUM).font_features(tabular()).text_color(if n == 0 { t.secondary } else { t.readable_on(color, t.content) }).child(format!("{n} {label}"));
        let (menu_card, menu_me) = (card.clone(), cx.entity());
        let bar = div()
            .flex_none()
            .h(px(control::SUBBAR))
            .px(px(space::M))
            .flex()
            .items_center()
            .gap(px(space::S))
            .child(Button::new("review-end").ghost().small().icon(menu_icon("x")).tooltip(tr("Beenden (Esc)")).on_click(cx.listener(|p, _, _, cx| p.end_session(cx))))
            .child(div().flex_1().min_w_0().truncate().text_size(px(font::CALLOUT)).text_color(t.secondary).child(if s.cram.is_some() { format!("{} · {}", deck_title(&s.deck, cx), tr("Prüfungsmodus")) } else { deck_title(&s.deck, cx) }))
            .when(cramming, |d| d.child(count_piece(s.left.review + 1, t.accent_text, true, tr("übrig"))))
            .when(!cramming, |d| {
                d.child(count_piece(s.left.new, t.blue, phase == Phase::New, tr("neu")))
                    .child(count_piece(s.left.learning, t.orange, matches!(phase, Phase::Learning | Phase::Relearning), tr("lernen")))
                    .child(count_piece(s.left.review, t.green, phase == Phase::Review, tr("fällig")))
            })
            .child(div().flex_1())
            .child(Button::new("review-undo").ghost().small().icon(menu_icon("undo-2")).tooltip(tr("Letzte Antwort zurück (⌘Z)")).disabled(!can_undo).on_click(cx.listener(|p, _, _, cx| p.undo(cx))))
            .child(Button::new("review-redo").ghost().small().icon(menu_icon("redo-2")).tooltip(tr("Antwort erneut geben (⌘⇧Z)")).disabled(s.undone.is_empty()).on_click(cx.listener(|p, _, _, cx| p.redo(cx))))
            .child(Button::new("review-edit").ghost().small().icon(menu_icon("pencil")).tooltip(tr("Karte bearbeiten (E)")).on_click({
                let edit = card.clone();
                cx.listener(move |p, _, window, cx| p.compose(Some(edit.clone()), window, cx))
            }))
            .child(Button::new("review-more").ghost().small().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu(move |menu, _, _| {
                let (a, b, c) = (menu_me.clone(), menu_me.clone(), menu_card.clone());
                menu.item(PopupMenuItem::new(tr("In der Notiz zeigen (O)")).icon(menu_icon("file-text")).on_click(move |_, _, cx| Self::open_source(&c, cx)))
                    .when(!cramming, |menu| {
                        menu.item(PopupMenuItem::new(tr("Für heute zurückstellen (B)")).icon(menu_icon("eye-off")).on_click(move |_, _, cx| a.update(cx, |p, cx| p.set_aside(false, cx))))
                            .item(PopupMenuItem::new(tr("Pausieren (S)")).icon(menu_icon("pause")).on_click(move |_, _, cx| b.update(cx, |p, cx| p.set_aside(true, cx))))
                    })
            }));
        let track = div().flex_none().h(px(space::XXS)).w_full().bg(t.separator).child(div().h_full().w(relative(progress)).bg(t.accent));
        let leech = card.is_leech().then(|| {
            let edit = card.clone();
            banner(t.orange).flex().items_center().gap(px(space::S)).py(px(space::S)).child(icon("triangle-alert", iconsize::S, t.orange)).child(div().flex_1().text_size(px(font::CALLOUT)).child(tr("Diese Karte vergisst du oft. Umformulieren oder aufteilen hilft meist mehr als Wiederholen."))).child(Button::new("leech-edit").small().label(tr("Bearbeiten")).on_click(cx.listener(move |p, _, window, cx| p.compose(Some(edit.clone()), window, cx))))
        });
        let crumbs = caption(card_path(&card), t).truncate();
        let body = div()
            .id("review-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.review_scroll)
            .child(
                div().w_full().min_h(relative(1.)).flex().flex_col().items_center().justify_center().px(px(space::XL)).py(px(space::XL)).child(
                    div()
                        .w_full()
                        .max_w(px(640. * super::theme::note_scale()))
                        .flex()
                        .flex_col()
                        .gap(px(space::L))
                        .children(leech)
                        .child(crumbs)
                        .child(front)
                        .when_some(back, |d, back| d.child(div().h(px(1.)).bg(t.separator)).child(back)),
                ),
            );
        let footer: AnyElement = if shown {
            let choices = cards::options(card.id(), card.state.as_ref(), &card.note_rel, &card.raw.front, now);
            let cramming = self.session.as_ref().is_some_and(|s| s.cram.is_some());
            let colors = [t.red, t.orange, t.green, t.blue];
            let line = (font::BODY * font::LINE).round();
            let drop = baseline_gap(window, font::BODY, FontWeight::SEMIBOLD, font::CAPTION2);
            div()
                .flex_none()
                .p(px(space::M))
                .h(px(control::FOOTBAR + space::M * 2. + 1.))
                .border_t_1()
                .border_color(t.separator)
                .flex()
                .items_center()
                .justify_center()
                .child(div().w(px(STUDY_WIDTH)).max_w_full().flex().gap(px(space::XS)).children(Rating::ALL.iter().enumerate().filter(|(_, r)| !cramming || matches!(r, Rating::Again | Rating::Good)).map(|(i, rating)| {
                    let rating = *rating;
                    let color = colors[i];
                    let hover = color.opacity(0.16);
                    div()
                        .id(("rate", i))
                        .flex_1()
                        .min_w_0()
                        .h(px(control::FOOTBAR))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(space::XXS))
                        .rounded(px(radius::CONTROL))
                        .border_1()
                        .border_color(color.opacity(if rating == Rating::Good { 0.9 } else { 0.4 }))
                        .bg(color.opacity(if rating == Rating::Good { 0.16 } else { 0.07 }))
                        .cursor_pointer()
                        .key_nav()
                        .soft(hover, radius::CONTROL)
                        .when(rating == Rating::Good, |d| d.tooltip(tip(tr("Gut (3 oder Leertaste)"))))
                        .when(!cramming, |d| d.child(div().text_size(px(font::CAPTION)).text_color(t.secondary).font_features(tabular()).child({ cards::wait_label(now, choices[i].due) })))
                        .child(div().flex().items_start().gap(px(space::XS)).child(div().text_size(px(font::BODY)).line_height(px(line)).font_weight(FontWeight::SEMIBOLD).text_color(t.readable(color)).child(if cramming && rating == Rating::Good { tr("Gewusst") } else { rating_name(rating) })).child(div().relative().top(px(drop)).text_size(px(font::CAPTION2)).line_height(px(line)).text_color(t.tertiary).child((i + 1).to_string())))
                        .on_click(cx.listener(move |p, e: &ClickEvent, _, cx| {
                            if e.click_count() <= 1 {
                                p.rate(rating, cx);
                            }
                        }))
                })))
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
                        .w(px(STUDY_WIDTH))
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
                        .soft(t.on_accent.opacity(0.14), radius::CONTROL)
                        .child(tr("Antwort zeigen"))
                        .child(div().text_size(px(font::CAPTION)).opacity(0.75).child(tr("Leertaste")))
                        .on_click(cx.listener(|p, _, _, cx| p.reveal(cx))),
                )
                .into_any_element()
        };
        let scrollable = div().relative().flex_1().min_h_0().flex().flex_col().child(body).vertical_scrollbar(&self.review_scroll);
        div().size_full().flex().flex_col().child(bar).child(track).child(scrollable).child(footer).into_any_element()
    }
}

fn baseline_gap(window: &Window, large: f32, weight: FontWeight, small: f32) -> f32 {
    thread_local!(static GAPS: std::cell::RefCell<std::collections::HashMap<(u32, u16, u32), f32>> = std::cell::RefCell::new(std::collections::HashMap::new()));
    let key = (large.to_bits(), weight.0 as u16, small.to_bits());
    if let Some(gap) = GAPS.with(|g| g.borrow().get(&key).copied()) {
        return gap;
    }
    let gap = measure_gap(window, large, weight, small);
    GAPS.with(|g| g.borrow_mut().insert(key, gap));
    gap
}

fn measure_gap(window: &Window, large: f32, weight: FontWeight, small: f32) -> f32 {
    let system = window.text_system();
    let family: SharedString = Theme::font_family().into();
    let id = |weight: FontWeight| system.resolve_font(&Font { weight, ..font(family.clone()) });
    let (large_id, small_id) = (id(weight), id(FontWeight::NORMAL));
    let at = |id: FontId, size: f32| (f32::from(system.ascent(id, px(size))) - f32::from(system.descent(id, px(size))).abs()) / 2.;
    (at(large_id, large) - at(small_id, small)).max(0.)
}

/// Starts a card from text in a note; it is written into that note.
pub fn card_from_selection(text: String, note: PathBuf, window: &mut Window, cx: &mut App) {
    super::sheets::show_sheet(760., window, cx, move |window, cx| CardComposer::new(String::new(), None, Some((text, note)), window, cx));
}

/// Removes cards' lines from their notes, going by what the notes say now; each note's undo brings them back.
pub fn delete_cards(chosen: &[Card], cx: &mut App) {
    let mut by_note: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for card in chosen {
        by_note.entry(card.note.as_ref().clone()).or_default().push(card.id().to_string());
    }
    let mut changed = false;
    let label = if chosen.len() == 1 { tr("Karte gelöscht") } else { trf("%@ Karten gelöscht", &[&chosen.len().to_string()]) };
    for (note, ids) in by_note {
        changed |= act(cx, |l| {
            l.edit_note(&note, "card", label.clone(), |text| {
                let mut ranges: Vec<(usize, usize)> = parse::cards_in(text).into_iter().filter(|c| ids.contains(&c.id)).map(|c| c.lines).collect();
                ranges.sort();
                ranges.dedup();
                let mut lines: Vec<&str> = text.split('\n').collect();
                for (first, last) in ranges.into_iter().rev() {
                    let end = (last + 1).min(lines.len());
                    if first >= end {
                        continue;
                    }
                    lines.drain(first..end);
                    if first > 0 && lines.get(first).is_some_and(|l| l.trim().is_empty()) && lines[first - 1].trim().is_empty() {
                        lines.remove(first);
                    }
                }
                lines.join("\n")
            })
        });
    }
    if changed {
        state::reindex(cx);
    }
}

/// Starts a new card from wherever, in the deck last used.
pub fn new_card(window: &mut Window, cx: &mut App) {
    let deck = prefs::get().card_deck;
    super::sheets::show_sheet(760., window, cx, move |window, cx| CardComposer::new(deck, None, None, window, cx));
}

impl Render for CardsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        self.all = cards::all_cards();
        match self.selected.as_ref().and_then(|id| self.all.iter().find(|c| c.id() == id)) {
            Some(card) => self.selected_place = Some((card.note.clone(), card.raw.lines.0, card.raw.kind)),
            None if self.selected.is_some() => {
                let again = self.selected_place.as_ref().and_then(|(note, line, kind)| self.all.iter().find(|c| c.note == *note && c.raw.lines.0 == *line && c.raw.kind == *kind)).map(|c| c.id().to_string());
                self.selected = None;
                self.preview = None;
                if let Some(id) = again {
                    self.select_card(id, cx);
                }
            }
            None => {}
        }
        if !self.marked.is_empty() {
            let known: std::collections::HashSet<&str> = self.all.iter().map(|c| c.id()).collect();
            self.marked.retain(|id| known.contains(id.as_str()));
        }
        if let Some(request) = state::lib(cx).update(cx, |l, _| l.cards_request.take()) {
            self.select_deck(request.clone(), cx);
            if cards::counts(&request, cards::now()).total() > 0 {
                self.start(request, window, cx);
            }
        }
        if let Some(s) = &mut self.session
            && s.cram.is_none()
            && s.current.is_none()
            && cards::counts(&s.deck, cards::now()).total() > 0
        {
            Self::advance(s, None);
        }
        if let Some(s) = &mut self.session
            && s.cram.is_none()
            && let Some(current) = s.current.clone()
        {
            match self.all.iter().find(|c| c.id() == current.id()) {
                Some(found) if found.raw != current.raw => {
                    s.front = parsed(&found.raw.front);
                    s.back = parsed(&found.raw.back);
                    s.current = Some(found.clone());
                }
                Some(_) => {}
                None => match self.all.iter().find(|c| c.note == current.note && c.raw.lines.0 == current.raw.lines.0 && c.raw.kind == current.raw.kind) {
                    Some(found) => {
                        s.front = parsed(&found.raw.front);
                        s.back = parsed(&found.raw.back);
                        s.current = Some(found.clone());
                    }
                    None => Self::advance(s, None),
                },
            }
        }
        if self.session.is_none() && !self.deck.is_empty() && !self.all.is_empty() && !self.all.iter().any(|c| c.in_deck(&self.deck)) {
            self.deck.clear();
        }
        let now = cards::now();
        let width = f32::from(window.viewport_size().width);
        let reviewing = self.session.is_some();
        let body: AnyElement = if reviewing {
            self.review(&t, window, cx)
        } else if self.all.is_empty() {
            self.empty_room(&t, cx)
        } else {
            let room = width - super::sidebar::rail_for(width) - if width > 860. { 260. } else { 0. };
            self.overview(now, room, width <= 860., f32::from(window.viewport_size().height), &t, cx)
        };
        let side = (!reviewing && !self.all.is_empty() && width > 860.).then(|| self.deck_column(now, &t, cx));
        let accent = t.accent;
        div()
            .id("cards-page")
            .track_focus(&self.focus)
            .key_context("CardsPage")
            .size_full()
            .flex()
            .bg(t.content)
            .on_key_down(cx.listener(Self::on_key))
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(accent.opacity(0.06)))
            .on_drop(cx.listener(|_, paths: &ExternalPaths, window, cx| {
                if let Some(path) = paths.paths().iter().find(|p| p.extension().is_some_and(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "apkg" | "colpkg" | "txt" | "tsv" | "csv"))) {
                    open_import(path.clone(), window, cx);
                }
            }))
            .capture_action(cx.listener(|p, _: &super::main_window::Dismiss, window, cx| {
                if p.query.read(cx).focus_handle(cx).is_focused(window) {
                    p.query.update(cx, |q, cx| q.set_value("", window, cx));
                    window.focus(&p.focus, cx);
                    cx.stop_propagation();
                } else if p.session.is_some() {
                    p.end_session(cx);
                    cx.stop_propagation();
                } else if p.selected.is_some() || !p.marked.is_empty() {
                    p.clear_selection();
                    cx.stop_propagation();
                    cx.notify();
                } else if p.zone == Zone::Decks {
                    p.zone = Zone::Cards;
                    cx.stop_propagation();
                    cx.notify();
                } else if p.focus.is_focused(window) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|p, _: &super::app::Undo, _, cx| {
                if p.session.is_some() {
                    cx.stop_propagation();
                    if p.can_undo() {
                        p.undo(cx);
                    }
                }
            }))
            .capture_action(cx.listener(|p, _: &super::app::Redo, _, cx| {
                if p.session.is_some() {
                    cx.stop_propagation();
                    p.redo(cx);
                }
            }))
            .capture_key_down(cx.listener(|p, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "down" && p.query.read(cx).focus_handle(cx).is_focused(window) {
                    window.focus(&p.focus, cx);
                    p.zone = Zone::Cards;
                    p.cards_key("home", false, window, cx);
                    cx.stop_propagation();
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

thread_local! {
    static DRAFT: std::cell::RefCell<Option<(String, String, ComposeKind)>> = const { std::cell::RefCell::new(None) };
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
    error: Option<String>,
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
            target = card.note.as_ref().clone();
            let source = state::lib(cx).update(cx, |l, _| l.buffer(card.note.as_path()).text.clone());
            let lines: Vec<&str> = source.split('\n').collect();
            let written = lines.get(card.raw.lines.0..=card.raw.lines.1).map(|l| l.iter().map(|x| &x[parse::line_marks(x).len()..]).collect::<Vec<_>>().join("\n")).unwrap_or_default();
            let (f, b) = match card.raw.kind {
                CardKind::Cloze => {
                    kind = ComposeKind::Cloze;
                    (parse::without_schedules(&written).trim().to_string(), String::new())
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
        } else if editing.is_none()
            && let Some((f, b, k)) = DRAFT.with(|d| d.borrow_mut().take())
        {
            kind = k;
            front.update(cx, |s, cx| s.set_value(f, window, cx));
            back.update(cx, |s, cx| s.set_value(b, window, cx));
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
        let mut this = Self { kind, front, back, target, editing, added: 0, error: None, focused: false, front_preview: parsed(""), back_preview: parsed(""), _subs: subs };
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
        if self.editing.is_none() {
            let (f, b) = (self.front.read(cx).value().to_string(), self.back.read(cx).value().to_string());
            let kind = self.kind;
            DRAFT.with(|d| *d.borrow_mut() = (!f.trim().is_empty() || !b.trim().is_empty()).then_some((f, b, kind)));
        }
        let written = self.written(cx);
        let found = crate::core::cards::parse::cards_in(&written);
        let (front, back) = found.first().map_or_else(|| (self.front.read(cx).value().to_string(), self.back.read(cx).value().to_string()), |c| (c.front.clone(), c.back.clone()));
        self.front_preview = parsed(&front);
        self.back_preview = parsed(&back);
        cx.notify();
    }

    fn is_valid(&self, cx: &App) -> bool {
        !crate::core::cards::parse::cards_in(&self.written(cx)).is_empty()
    }

    /// An image on the clipboard goes into the note's attachment folder, a link to it at the caret.
    fn paste_image(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else { return false };
        let Some(image) = item.entries().iter().find_map(|e| if let ClipboardEntry::Image(image) = e { Some(image.clone()) } else { None }) else { return false };
        let vault = index::current();
        let folder = vault.config.attachment_folder(&vault.root, &self.target);
        let name = format!("{} {}.png", tr("Eingefügtes Bild"), chrono::Local::now().format("%Y%m%d%H%M%S"));
        if std::fs::create_dir_all(&folder).is_err() || std::fs::write(folder.join(&name), &image.bytes).is_err() {
            return false;
        }
        let field = if self.back.read(cx).focus_handle(cx).is_focused(window) { self.back.clone() } else { self.front.clone() };
        field.update(cx, |s, cx| s.replace(format!("![[{name}]]"), window, cx));
        self.refresh_preview(cx);
        state::reindex(cx);
        true
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
            let moved = target != *card.note;
            let replacement = written.clone();
            let id = card.id().to_string();
            let mut found = false;
            let mut kept = (String::new(), None::<String>);
            let saved = act(cx, |l| {
                l.edit_note(card.note.as_path(), "card", if moved { tr("Karte verschoben") } else { tr("Karte geändert") }, |text| {
                    let Some(place) = parse::cards_in(text).into_iter().find(|c| c.id == id).map(|c| c.lines) else { return text.to_string() };
                    found = true;
                    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
                    let mut end = (place.1 + 1).min(lines.len());
                    if place.0 < end {
                        let inline = parse::schedule_comments(&lines[place.0..end]);
                        let below = lines.get(end).filter(|l| parse::is_schedule_line(l)).cloned();
                        if below.is_some() {
                            end += 1;
                        }
                        let marks = parse::line_marks(&lines[place.0]).to_string();
                        let quoted: String = marks.chars().take_while(|c| *c == '>' || *c == ' ').collect();
                        let mut new: Vec<String> = replacement.split('\n').enumerate().map(|(i, line)| if i == 0 { format!("{marks}{line}") } else { format!("{quoted}{line}") }).collect();
                        if let Some(last) = new.last_mut() {
                            last.push_str(&inline);
                        }
                        new.extend(below.clone());
                        lines.splice(place.0..end, if moved { Vec::new() } else { new });
                        kept = (inline, below.map(|b| b.trim_start_matches(['>', ' ', '\t']).to_string()));
                    }
                    lines.join("\n")
                })
            });
            if !found {
                self.error = Some(tr("Die Karte wurde in der Notiz geändert. Schließen und neu öffnen."));
                cx.notify();
                return;
            }
            if moved && saved {
                if !target.exists() {
                    let _ = target.parent().map(std::fs::create_dir_all);
                    let _ = std::fs::write(&target, "");
                }
                let line = match &kept {
                    (inline, Some(below)) => format!("{written}{inline}\n{below}"),
                    (inline, None) => format!("{written}{inline}"),
                };
                act(cx, |l| {
                    l.edit_note(&target, "card", tr("Karte verschoben"), |text| {
                        let joined = if text.is_empty() || text.ends_with("\n\n") { text.to_string() } else if text.ends_with('\n') { format!("{text}\n") } else { format!("{text}\n\n") };
                        format!("{joined}{line}\n")
                    })
                });
            }
            if saved {
                let old: Vec<String> = cards::all_cards().iter().filter(|c| c.note == card.note && c.raw.lines == card.raw.lines).map(|c| c.id().to_string()).collect();
                for (before, after) in old.iter().zip(parse::cards_in(&written)) {
                    cards::rename(before, &after.id);
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
            DRAFT.with(|d| d.borrow_mut().take());
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(super::quick::Dismissed);
    }

    /// Why the card cannot be saved yet, or what to know before saving it.
    fn notice(&self, window: &Window, cx: &App) -> Option<String> {
        if let Some(error) = &self.error {
            return Some(error.clone());
        }
        let written = self.written(cx);
        if self.front.read(cx).value().trim().is_empty() {
            return None;
        }
        let found = crate::core::cards::parse::cards_in(&written);
        if found.is_empty() {
            return Some(match self.kind {
                ComposeKind::Cloze => tr("Markiere ein Wort und drück „Lücke“."),
                _ if self.back.read(cx).value().trim().is_empty() => {
                    if self.front.read(cx).focus_handle(cx).is_focused(window) || self.back.read(cx).focus_handle(cx).is_focused(window) {
                        return None;
                    }
                    tr("Die Rückseite fehlt noch.")
                }
                _ => tr("Das ergibt keine Karte."),
            });
        }
        let editing = self.editing.as_ref().map(|c| c.id().to_string());
        let same = cards::all_cards().iter().find(|c| found.iter().any(|f| f.id == c.id()) && Some(c.id()) != editing.as_deref()).map(|c| c.note_rel.clone());
        same.map(|note| trf("Diese Frage gibt es schon in %@. Der Fortschritt beider wäre derselbe.", &[&note.trim_end_matches(".md").to_string()]))
    }
}

impl Render for CardComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        if !std::mem::replace(&mut self.focused, true) {
            self.front.update(cx, |s, cx| {
                let end = s.value().len();
                s.set_selected_range(end..end, cx);
                s.focus(window, cx);
            });
        }
        let kind = self.kind;
        let kinds = [(ComposeKind::Basic, tr("Eine Richtung"), "file-question"), (ComposeKind::Both, tr("Beide Richtungen"), "repeat-2"), (ComposeKind::Cloze, tr("Lückentext"), "scan")];
        let switch = div().flex().rounded(px(radius::INNER)).bg(t.ink.opacity(0.06)).p(px(space::XXS)).children(kinds.into_iter().map(|(k, label, name)| {
            let on = k == kind;
            div().id(SharedString::from(format!("kind-{name}"))).h(px(control::S)).px(px(space::S)).flex().items_center().gap(px(space::XS)).rounded(px(radius::SMALL)).when(!on, |d| d.soft(t.ink.opacity(0.06), radius::SMALL)).cursor_pointer().text_size(px(font::CALLOUT)).when(on, |d| d.bg(t.ink.opacity(0.12))).child(icon(name, iconsize::S, if on { t.primary } else { t.secondary })).child(label).on_click(cx.listener(move |c, _, _, cx| {
                c.kind = k;
                c.refresh_preview(cx);
            }))
        }));
        let root = index::current().root.clone();
        let target_name = self.target.strip_prefix(&root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        let mut notes: Vec<PathBuf> = cards::all_cards().iter().map(|c| c.note.as_ref().clone()).collect();
        notes.dedup();
        let in_modules: Vec<PathBuf> = state::read(cx, |l| l.config.as_ref().map(|c| c.modules.iter().map(|m| root.join(&m.folder).join(format!("{}.md", tr("Karteikarten")))).collect()).unwrap_or_default());
        for note in in_modules {
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
        notes.retain(|n| *n != self.target);
        notes.truncate(40);
        let me = cx.entity();
        let editing = self.editing.is_some();
        let picker = Button::new("card-target").small().ghost().icon(menu_icon("file-text")).label(target_name.trim_end_matches(".md").to_string()).dropdown_caret(true).tooltip(tr("Notiz, in der die Karte steht")).dropdown_menu(move |mut menu, _, _| {
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
            .child(div().flex().items_center().justify_between().child(field_label(if cloze { tr("Text mit Lücken") } else { tr("Vorderseite") })).child(Button::new("card-gap").ghost().small().icon(menu_icon("scan")).label(tr("Lücke")).tooltip(tr("Macht aus der Auswahl eine Lücke (⌘⇧C)")).on_click(cx.listener(|c, _, window, cx| c.make_gap(window, cx)))))
            .child(Textarea::new(&self.front))
            .when(!cloze, |d| d.child(field_label(tr("Rückseite"))).child(Textarea::new(&self.back)))
            .child(match self.notice(window, cx) {
                Some(text) => div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.orange_text).child(icon("triangle-alert", iconsize::XS, t.orange_text)).child(div().flex_1().min_w_0().child(text)),
                None => caption(tr("Formatierung, Formeln mit $…$ und Bilder gehen überall."), &t),
            });
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
            .child(field_label(match crate::core::cards::parse::cards_in(&self.written(cx)).len() {
                0 | 1 => tr("Vorschau"),
                n => trf("Vorschau · %@ Karten", &[&n.to_string()]),
            }))
            .child(markdown_view::render(&self.front_preview, base.as_deref(), None, &t))
            .when(!self.back.read(cx).value().trim().is_empty(), |d| d.child(div().h(px(1.)).flex_none().bg(t.separator)).child(markdown_view::render(&self.back_preview, base.as_deref(), None, &t)));
        div()
            .w_full()
            .flex()
            .flex_col()
            .key_context("CardComposer")
            .capture_action(cx.listener(|c, _: &gpui_kit::component::input::Paste, window, cx| {
                if c.paste_image(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|c, a: &gpui_kit::component::input::Enter, window, cx| {
                if a.secondary {
                    c.save(window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_key_down(cx.listener(|c, e: &KeyDownEvent, window, cx| {
                let m = &e.keystroke.modifiers;
                if e.keystroke.key == "c" && m.shift && (m.platform || m.control) {
                    c.make_gap(window, cx);
                    cx.stop_propagation();
                } else if e.keystroke.key == "escape" {
                    c.close(cx);
                    cx.stop_propagation();
                } else if e.keystroke.key == "tab" && c.kind != ComposeKind::Cloze {
                    if c.front.read(cx).focus_handle(cx).is_focused(window) {
                        c.back.update(cx, |s, cx| s.focus(window, cx));
                    } else {
                        c.front.update(cx, |s, cx| s.focus(window, cx));
                    }
                    cx.stop_propagation();
                } else if e.keystroke.key == "tab" {
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
                    .child(caption(if self.kind == ComposeKind::Cloze { tr("⌘⇧C · ⌘↩") } else { tr("Tab · ⌘↩") }, &t))
                    .child(Button::new("card-save").primary().label(if editing { tr("Sichern") } else { tr("Anlegen") }).disabled(!valid).on_click(cx.listener(|c, _, window, cx| c.save(window, cx)))),
            )
    }
}

/// Asks for an Anki export and shows what it holds.
pub fn choose_import(window: &mut Window, cx: &mut App) {
    let handle = window.window_handle();
    let options = PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(tr("Importieren").into()) };
    super::platform::pick(options, cx, move |paths, cx| {
        if let Some(path) = paths.into_iter().next() {
            let _ = handle.update(cx, |_, window, cx| open_import(path, window, cx));
        }
    });
}

/// Shows what an Anki export holds, before anything is written.
pub fn open_import(path: PathBuf, window: &mut Window, cx: &mut App) {
    super::sheets::show_sheet(560., window, cx, move |window, cx| CardImport::new(path, window, cx));
}

/// One Anki export on its way into the notes.
pub struct CardImport {
    path: PathBuf,
    package: Option<cards::anki::Package>,
    error: Option<String>,
    working: bool,
    keep_progress: bool,
    pending: bool,
    folder: Entity<InputState>,
    _load: Option<Task<()>>,
    _enter: Subscription,
}

impl EventEmitter<super::quick::Dismissed> for CardImport {}

impl CardImport {
    fn folder_outside(&self, cx: &App) -> bool {
        folder_outside(self.folder.read(cx).value().trim())
    }

    fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let folder = cx.new(|cx| InputState::new(window, cx).default_value("Anki"));
        let enter = cx.subscribe(&folder, |this: &mut Self, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::PressEnter { .. }) && !this.working {
                this.start(cx);
            }
            cx.notify();
        });
        folder.update(cx, |f, cx| f.focus(window, cx));
        let source = path.clone();
        let task = cx.background_spawn(async move { cards::anki::read(&source) });
        let load = cx.spawn(async move |this, cx| {
            let read = task.await;
            let _ = this.update(cx, |s, cx| {
                match read {
                    Ok(package) => s.package = Some(package),
                    Err(e) => s.error = Some(e.to_string()),
                }
                if std::mem::take(&mut s.pending) && s.package.is_some() {
                    s.start(cx);
                }
                cx.notify();
            });
        });
        Self { path, package: None, error: None, working: false, keep_progress: true, pending: false, folder, _load: Some(load), _enter: enter }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Some(package) = self.package.take() else {
            self.pending = self.error.is_none();
            return;
        };
        let root = index::current().root.clone();
        let name = self.folder.read(cx).value().trim().to_string();
        if root.as_os_str().is_empty() || folder_outside(&name) {
            self.package = Some(package);
            cx.notify();
            return;
        }
        let name = name.trim_matches('/').to_string();
        let target = root.join(if name.is_empty() { "Anki" } else { &name });
        let (keep, known) = (self.keep_progress, cards::known_ids());
        self.working = true;
        cx.notify();
        let rel_root = root.clone();
        let task = cx.background_spawn(async move {
            let mut done = cards::anki::write(package, &target, keep, &known, |p| p.strip_prefix(&rel_root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default())?;
            let back = cards::adopt_import(std::mem::take(&mut done.states), std::mem::take(&mut done.log));
            anyhow::Ok((done, back))
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let failed = match result {
                Ok((done, back)) => {
                    cx.update(|cx| {
                        act(cx, |l| l.cards_imported(done, back));
                        state::reindex(cx);
                    });
                    None
                }
                Err(e) => Some(e.to_string()),
            };
            let _ = this.update(cx, |s, cx| {
                s.working = false;
                match failed {
                    Some(e) => s.error = Some(e),
                    None => cx.emit(super::quick::Dismissed),
                }
                cx.notify();
            });
        }));
    }
}

fn folder_outside(folder: &str) -> bool {
    let path = std::path::Path::new(folder);
    path.is_absolute() || folder.starts_with(['/', '\\']) || folder.contains(':') || folder.split(['/', '\\']).any(|part| part == "..") || path.components().any(|c| !matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir))
}

impl Render for CardImport {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let name = self.path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let body: AnyElement = if let Some(error) = &self.error {
            div().flex().items_center().gap(px(space::S)).text_color(t.red).child(icon("triangle-alert", iconsize::S, t.red)).child(div().flex_1().child(error.clone())).into_any_element()
        } else if let Some(package) = &self.package {
            let decks = package.decks.len();
            let summary = [
                if package.cards == 1 { tr("1 Karte") } else { trf("%@ Karten", &[&package.cards.to_string()]) },
                if decks == 1 { tr("1 Stapel") } else { trf("%@ Stapel", &[&decks.to_string()]) },
                if package.media.len() == 1 { tr("1 Mediendatei") } else { trf("%@ Mediendateien", &[&package.media.len().to_string()]) },
            ]
            .join(" · ");
            let mut list = div().flex().flex_col();
            for (deck, notes) in package.decks.iter().take(100) {
                list = list.child(div().flex().items_center().gap(px(space::S)).px(px(space::M)).py(px(space::XS)).child(icon("layers", iconsize::S, t.secondary)).child(div().flex_1().min_w_0().truncate().child(deck.join(" › "))).child(caption(notes.len().to_string(), &t).font_features(tabular())));
            }
            let (keep, with_progress) = (self.keep_progress, package.with_progress);
            let me = cx.entity();
            div()
                .flex()
                .flex_col()
                .gap(px(space::M))
                .child(div().font_weight(FontWeight::MEDIUM).child(summary))
                .child(div().id("import-decks").max_h(px(180.)).overflow_y_scrollbar().rounded(px(radius::CONTROL)).border_1().border_color(t.separator).child(list))
                .child(div().flex().items_center().gap(px(space::M)).child(div().flex_1().child(tr("Zielordner"))).child(div().w(px(200.)).child(Input::new(&self.folder).small())))
                .when(self.folder_outside(cx), |d| d.child(div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(t.red).child(icon("triangle-alert", iconsize::XS, t.red)).child(tr("Der Zielordner muss im Studienordner liegen."))))
                .when(with_progress > 0, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::M))
                            .child(div().flex_1().flex().flex_col().child(tr("Lernfortschritt übernehmen")).child(caption(trf("%@ Karten wurden schon gelernt. Termine und Stabilität kommen mit.", &[&with_progress.to_string()]), &t)))
                            .child(gpui_kit::component::switch::Switch::new("import-progress").checked(keep).on_click(move |v, _, cx| {
                                let on = *v;
                                me.update(cx, |s, cx| {
                                    s.keep_progress = on;
                                    cx.notify();
                                });
                            })),
                    )
                })
                .child(caption(tr("Die Karten landen als Text in Notizen, je Stapel eine. Doppelte werden übersprungen."), &t))
                .into_any_element()
        } else {
            div().h(px(80.)).flex().items_center().justify_center().child(super::motion::loading(iconsize::L, t.secondary)).into_any_element()
        };
        let ready = self.package.is_some() && !self.working;
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(div().px(px(space::XL)).pt(px(space::L)).flex().flex_col().gap(px(space::XXS)).child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(tr("Aus Anki importieren"))).child(caption(name, &t).truncate()))
            .child(div().p(px(space::XL)).child(body))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(space::S))
                    .p(px(space::L))
                    .border_t_1()
                    .border_color(t.separator)
                    .child(Button::new("import-cancel").label(tr("Abbrechen")).on_click(cx.listener(|_, _, _, cx| cx.emit(super::quick::Dismissed))))
                    .child(Button::new("import-start").primary().label(if self.working { tr("Importiere …") } else { tr("Importieren") }).disabled(!ready || self.folder_outside(cx)).on_click(cx.listener(|s, _, _, cx| s.start(cx)))),
            )
    }
}

#[cfg(test)]
mod import_tests {
    #[test]
    fn an_import_folder_never_leaves_the_study_folder() {
        for inside in ["Anki", "Anki/Bio", "Anki/", "./Anki"] {
            assert!(!super::folder_outside(inside), "{inside}");
        }
        for outside in ["/etc", "C:/Users", "C:Anki", "../Anki", "Anki/../../x", "//server/share"] {
            assert!(super::folder_outside(outside), "{outside}");
        }
    }
}
