use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use chrono::{Duration, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

use super::fsrs::{self, Memory, Rating};
use super::parse::{CardKind, RawCard};
use crate::core::markdown;
use crate::core::prefs;
use crate::core::vault::index::{self, VaultIndex};

pub const STORE_NAME: &str = ".schrumbi-cards.json";
pub const LOG_NAME: &str = ".schrumbi-cards-log.jsonl";
/// A new day starts at 4 in the morning, so a late night still counts as today.
const DAY_STARTS_AT: i64 = 4;
const LEARNING_STEPS: [i64; 2] = [1, 10];
const RELEARNING_STEPS: [i64; 1] = [10];
/// Waiting cards may come early when nothing else is left.
const LEARN_AHEAD_MINUTES: i64 = 20;
/// Failing this often marks a card as a leech: it wastes time and needs rewriting.
pub const LEECH_LAPSES: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    #[default]
    New,
    Learning,
    Review,
    Relearning,
}

/// How far a card has come; everything else about it stands in the note.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CardState {
    pub phase: Phase,
    pub stability: f64,
    pub difficulty: f64,
    /// Seconds since 1970.
    pub due: i64,
    pub last_review: Option<i64>,
    pub step: usize,
    pub reps: u32,
    pub lapses: u32,
    pub suspended: bool,
    pub buried_until: Option<i64>,
    /// Where the card was seen last and what it asked, to find it again after an edit.
    pub note: String,
    pub question: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct StoreFile {
    version: u32,
    cards: BTreeMap<String, CardState>,
}

/// One answered card, kept for statistics and for fitting the scheduler later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewLog {
    pub card: String,
    pub at: i64,
    pub rating: Rating,
    pub phase: Phase,
    pub days: f64,
}

/// One card with everything needed to show it.
#[derive(Debug, Clone)]
pub struct Card {
    pub raw: RawCard,
    pub note: PathBuf,
    pub note_rel: String,
    pub state: Option<CardState>,
}

impl Card {
    pub fn phase(&self) -> Phase {
        self.state.as_ref().map_or(Phase::New, |s| s.phase)
    }

    pub fn suspended(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.suspended)
    }

    pub fn buried(&self, now: i64) -> bool {
        self.state.as_ref().and_then(|s| s.buried_until).is_some_and(|until| until > now)
    }

    pub fn is_leech(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.lapses >= LEECH_LAPSES)
    }

    pub fn id(&self) -> &str {
        &self.raw.id
    }

    /// The folder and note the card belongs to, to sort it into decks.
    pub fn in_deck(&self, deck: &str) -> bool {
        deck.is_empty() || self.note_rel == deck || self.note_rel.starts_with(&format!("{deck}/"))
    }
}

/// How many cards wait, as in the session the next start would build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub new: usize,
    pub learning: usize,
    pub review: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.new + self.learning + self.review
    }
}

#[derive(Default)]
struct Store {
    root: Option<PathBuf>,
    file: StoreFile,
    log: Vec<ReviewLog>,
    revision: u64,
    cache: Option<(u64, u64, Arc<Vec<Card>>)>,
    counts: HashMap<(String, i64, u64, u64), Counts>,
}

static STORE: LazyLock<RwLock<Store>> = LazyLock::new(Default::default);
static SAVED: AtomicU64 = AtomicU64::new(0);
static SAVE_LOCK: Mutex<()> = Mutex::new(());

pub fn now() -> i64 {
    Local::now().timestamp()
}

fn day_of(at: i64) -> NaiveDate {
    Local.timestamp_opt(at - DAY_STARTS_AT * 3600, 0).single().map_or_else(|| Local::now().date_naive(), |d| d.date_naive())
}

/// Whole days between two moments, the way a person counts them.
fn days_between(from: i64, to: i64) -> i64 {
    (day_of(to) - day_of(from)).num_days().max(0)
}

/// When the next study day begins.
pub fn next_day_start(at: i64) -> i64 {
    let next = day_of(at) + Duration::days(1);
    next.and_hms_opt(DAY_STARTS_AT as u32, 0, 0).and_then(|d| Local.from_local_datetime(&d).earliest()).map_or(at + 86400, |d| d.timestamp())
}

/// Reads what is known about the cards of a vault, off the main thread.
pub fn open(root: &Path) {
    {
        let store = STORE.read().unwrap();
        if store.root.as_deref() == Some(root) {
            return;
        }
    }
    let file: StoreFile = std::fs::read(root.join(STORE_NAME)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let log = read_log(&root.join(LOG_NAME));
    let mut store = STORE.write().unwrap();
    *store = Store { root: Some(root.to_path_buf()), file, log, revision: store.revision + 1, cache: None, counts: HashMap::new() };
}

fn read_log(path: &Path) -> Vec<ReviewLog> {
    std::fs::read_to_string(path).map(|text| text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// Forgets the open vault's cards.
pub fn close() {
    let mut store = STORE.write().unwrap();
    *store = Store { revision: store.revision + 1, ..Default::default() };
}

fn save() {
    let (root, json, version) = {
        let mut store = STORE.write().unwrap();
        store.revision += 1;
        store.cache = None;
        store.counts.clear();
        let Some(root) = store.root.clone() else { return };
        let Ok(json) = serde_json::to_vec_pretty(&store.file) else { return };
        (root, json, store.revision)
    };
    std::thread::spawn(move || {
        let _guard = SAVE_LOCK.lock().unwrap();
        if SAVED.load(Ordering::Relaxed) > version {
            return;
        }
        SAVED.store(version, Ordering::Relaxed);
        let path = root.join(STORE_NAME);
        crate::core::workspace::backup(&path);
        let tmp = root.join(format!("{STORE_NAME}.tmp"));
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    });
}

fn append_log(root: &Path, entry: &ReviewLog) {
    if let (Ok(mut file), Ok(line)) = (std::fs::OpenOptions::new().create(true).append(true).open(root.join(LOG_NAME)), serde_json::to_string(entry)) {
        let _ = writeln!(file, "{line}");
    }
}

/// Every card of the vault, worked out once per change.
pub fn all_cards() -> Arc<Vec<Card>> {
    let vault = index::current();
    {
        let store = STORE.read().unwrap();
        if let Some((generation, revision, cards)) = &store.cache
            && *generation == vault.generation
            && *revision == store.revision
        {
            return cards.clone();
        }
    }
    let mut store = STORE.write().unwrap();
    let cards = Arc::new(collect(&vault, &store.file));
    let revision = store.revision;
    store.cache = Some((vault.generation, revision, cards.clone()));
    cards
}

fn collect(vault: &VaultIndex, file: &StoreFile) -> Vec<Card> {
    let mut notes: Vec<(&PathBuf, &index::NoteFacts)> = vault.notes.iter().filter(|(p, f)| !f.cards.is_empty() && !vault.is_template(p) && markdown::is_markdown(p)).collect();
    notes.sort_by(|a, b| a.0.cmp(b.0));
    let mut seen = HashSet::new();
    let mut cards = Vec::new();
    for (path, facts) in notes {
        let rel = path.strip_prefix(&vault.root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        if vault.config.is_ignored(&rel) {
            continue;
        }
        for raw in &facts.cards {
            if seen.insert(raw.id.clone()) {
                cards.push(Card { state: file.cards.get(&raw.id).cloned(), raw: raw.clone(), note: path.clone(), note_rel: rel.clone() });
            }
        }
    }
    cards
}

/// Keeps progress when a question was reworded: a card without state takes over the most similar lost one of its note.
pub fn reconcile() {
    let vault = index::current();
    let mut store = STORE.write().unwrap();
    if store.root.is_none() || store.file.cards.is_empty() {
        return;
    }
    let cards = collect(&vault, &store.file);
    let known: HashSet<&str> = cards.iter().map(|c| c.raw.id.as_str()).collect();
    let mut lost: HashMap<String, Vec<String>> = HashMap::new();
    for (id, state) in &store.file.cards {
        if !known.contains(id.as_str()) {
            lost.entry(state.note.clone()).or_default().push(id.clone());
        }
    }
    let mut moved: Vec<(String, String)> = Vec::new();
    let mut seen_here: Vec<(String, String)> = Vec::new();
    for card in &cards {
        match &card.state {
            Some(state) if state.note != card.note_rel || state.question != card.raw.front => seen_here.push((card.raw.id.clone(), card.note_rel.clone())),
            Some(_) => {}
            None => {
                let Some(candidates) = lost.get_mut(&card.note_rel) else { continue };
                let best = candidates
                    .iter()
                    .enumerate()
                    .map(|(i, id)| (i, similar::TextDiff::from_chars(store.file.cards[id].question.as_str(), card.raw.front.as_str()).ratio()))
                    .max_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((at, ratio)) = best.filter(|(_, r)| *r >= 0.6) {
                    moved.push((candidates.remove(at), card.raw.id.clone()));
                }
            }
        }
    }
    if moved.is_empty() && seen_here.is_empty() {
        return;
    }
    let fronts: HashMap<&str, &str> = cards.iter().map(|c| (c.raw.id.as_str(), c.raw.front.as_str())).collect();
    for (old, new) in moved {
        if let Some(mut state) = store.file.cards.remove(&old) {
            state.question = fronts.get(new.as_str()).unwrap_or(&"").to_string();
            store.file.cards.insert(new, state);
        }
    }
    for (id, note) in seen_here {
        let front = fronts.get(id.as_str()).unwrap_or(&"").to_string();
        if let Some(state) = store.file.cards.get_mut(&id) {
            state.note = note;
            state.question = front;
        }
    }
    drop(store);
    save();
}

/// Moves a card's progress to a new identity after it was edited here.
pub fn rename(old: &str, new: &str) {
    if old == new {
        return;
    }
    let mut store = STORE.write().unwrap();
    if let Some(state) = store.file.cards.remove(old) {
        store.file.cards.insert(new.to_string(), state);
        drop(store);
        save();
    }
}

fn fuzz(id: &str, reps: u32, days: f64) -> i64 {
    if days < 2.5 {
        return days.round() as i64;
    }
    let seed: u64 = id.bytes().fold(reps as u64 + 7, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    let spread = (days * 0.05).max(1.).min(days * 0.25);
    let unit = (seed % 2001) as f64 / 1000. - 1.;
    (days + unit * spread).round() as i64
}

fn retention() -> f64 {
    prefs::get().card_retention.clamp(0.7, 0.99)
}

/// What each answer would do, in the order of `Rating::ALL`. The buttons show exactly this.
pub fn options(id: &str, state: Option<&CardState>, note: &str, question: &str, at: i64) -> [CardState; 4] {
    let base = state.cloned().unwrap_or_default();
    let days = base.last_review.map_or(0, |last| days_between(last, at)) as f64;
    let target = retention();
    let mut result: Vec<CardState> = Rating::ALL
        .iter()
        .map(|rating| {
            let memory = if base.phase == Phase::New && base.last_review.is_none() {
                fsrs::first(*rating)
            } else {
                fsrs::next(Memory { stability: base.stability, difficulty: base.difficulty }, days, *rating)
            };
            let mut next = base.clone();
            next.stability = memory.stability;
            next.difficulty = memory.difficulty;
            next.last_review = Some(at);
            next.reps += 1;
            next.note = note.to_string();
            next.question = question.to_string();
            next.buried_until = None;
            next
        })
        .collect();
    let graduate = |s: &mut CardState| {
        s.phase = Phase::Review;
        s.step = 0;
    };
    let wait_minutes = |s: &mut CardState, minutes: i64| s.due = at + minutes * 60;
    let review_days: Vec<i64> = {
        let mut planned: Vec<i64> = result.iter().map(|s| fuzz(id, s.reps, fsrs::interval(s.stability, target)).max(1)).collect();
        planned[2] = planned[2].max(planned[1] + 1);
        planned[3] = planned[3].max(planned[2] + 1);
        planned
    };
    let set_review = |s: &mut CardState, days: i64| {
        graduate(s);
        s.due = next_day_start(at) + (days - 1).max(0) * 86400;
    };
    match base.phase {
        Phase::New | Phase::Learning => {
            let step = base.step.min(LEARNING_STEPS.len() - 1);
            result[0].phase = Phase::Learning;
            result[0].step = 0;
            wait_minutes(&mut result[0], LEARNING_STEPS[0]);
            result[1].phase = Phase::Learning;
            result[1].step = step;
            wait_minutes(&mut result[1], if step == 0 { (LEARNING_STEPS[0] + LEARNING_STEPS[1]) / 2 } else { LEARNING_STEPS[step] * 3 / 2 });
            if step + 1 < LEARNING_STEPS.len() {
                result[2].phase = Phase::Learning;
                result[2].step = step + 1;
                wait_minutes(&mut result[2], LEARNING_STEPS[step + 1]);
            } else {
                set_review(&mut result[2], review_days[2]);
            }
            set_review(&mut result[3], review_days[3]);
        }
        Phase::Review => {
            result[0].phase = Phase::Relearning;
            result[0].step = 0;
            result[0].lapses += 1;
            wait_minutes(&mut result[0], RELEARNING_STEPS[0]);
            for i in 1..4 {
                set_review(&mut result[i], review_days[i]);
            }
        }
        Phase::Relearning => {
            result[0].phase = Phase::Relearning;
            result[0].step = 0;
            wait_minutes(&mut result[0], RELEARNING_STEPS[0]);
            result[1].phase = Phase::Relearning;
            wait_minutes(&mut result[1], RELEARNING_STEPS[0] * 3 / 2);
            set_review(&mut result[2], review_days[2]);
            set_review(&mut result[3], review_days[3]);
        }
    }
    [result[0].clone(), result[1].clone(), result[2].clone(), result[3].clone()]
}

/// Short words for how long until a card comes back.
pub fn wait_label(from: i64, due: i64) -> String {
    use crate::core::i18n::trf;
    let minutes = (due - from) / 60;
    if minutes < 60 {
        return trf("%@ Min.", &[&minutes.max(1).to_string()]);
    }
    if minutes < 60 * 20 {
        return trf("%@ Std.", &[&(minutes / 60).to_string()]);
    }
    let days = ((due - from) as f64 / 86400.).round().max(1.) as i64;
    match days {
        d if d < 31 => trf("%@ T.", &[&d.to_string()]),
        d if d < 365 => trf("%@ Mon.", &[&format!("{:.1}", d as f64 / 30.4).replace(".0", "")]),
        d => trf("%@ J.", &[&format!("{:.1}", d as f64 / 365.).replace(".0", "")]),
    }
}

/// Everything needed to take an answer back.
#[derive(Debug, Clone)]
pub struct Answered {
    pub id: String,
    before: Option<CardState>,
    buried: Vec<(String, Option<CardState>)>,
}

/// Saves an answer and puts the other cards of the same place aside until tomorrow.
pub fn answer(card: &Card, rating: Rating, at: i64) -> Answered {
    let chosen = options(&card.raw.id, card.state.as_ref(), &card.note_rel, &card.raw.front, at)[rating as usize - 1].clone();
    let days = card.state.as_ref().and_then(|s| s.last_review).map_or(0, |last| days_between(last, at)) as f64;
    let log = ReviewLog { card: card.raw.id.clone(), at, rating, phase: card.phase(), days };
    let cards = all_cards();
    let siblings: Vec<&Card> = cards.iter().filter(|c| c.raw.group == card.raw.group && c.raw.id != card.raw.id).collect();
    let mut buried = Vec::new();
    let mut store = STORE.write().unwrap();
    let before = store.file.cards.insert(card.raw.id.clone(), chosen);
    for sibling in siblings {
        if !matches!(sibling.phase(), Phase::Learning | Phase::Relearning) && !sibling.suspended() {
            let previous = store.file.cards.get(&sibling.raw.id).cloned();
            let entry = store.file.cards.entry(sibling.raw.id.clone()).or_insert_with(|| CardState { note: sibling.note_rel.clone(), question: sibling.raw.front.clone(), ..Default::default() });
            entry.buried_until = Some(next_day_start(at));
            buried.push((sibling.raw.id.clone(), previous));
        }
    }
    store.log.push(log.clone());
    let root = store.root.clone();
    drop(store);
    if let Some(root) = root {
        append_log(&root, &log);
    }
    save();
    Answered { id: card.raw.id.clone(), before, buried }
}

/// Takes an answer back, also out of the log.
pub fn undo(answered: Answered) {
    let mut store = STORE.write().unwrap();
    match answered.before {
        Some(state) => {
            store.file.cards.insert(answered.id.clone(), state);
        }
        None => {
            store.file.cards.remove(&answered.id);
        }
    }
    for (id, state) in answered.buried {
        match state {
            Some(state) => {
                store.file.cards.insert(id, state);
            }
            None => {
                store.file.cards.remove(&id);
            }
        }
    }
    if let Some(at) = store.log.iter().rposition(|l| l.card == answered.id) {
        store.log.remove(at);
    }
    let rest: String = store.log.iter().filter_map(|l| serde_json::to_string(l).ok()).map(|l| l + "\n").collect();
    if let Some(root) = store.root.clone() {
        let (tmp, path) = (root.join(format!("{LOG_NAME}.tmp")), root.join(LOG_NAME));
        if std::fs::write(&tmp, rest).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
    drop(store);
    save();
}

fn edit_state(card: &Card, change: impl FnOnce(&mut CardState)) {
    let mut store = STORE.write().unwrap();
    let state = store.file.cards.entry(card.raw.id.clone()).or_insert_with(|| CardState { note: card.note_rel.clone(), question: card.raw.front.clone(), ..Default::default() });
    change(state);
    drop(store);
    save();
}

pub fn set_suspended(card: &Card, suspended: bool) {
    edit_state(card, |s| s.suspended = suspended);
}

/// Pauses or resumes all cards of a deck in one step.
pub fn set_deck_suspended(deck: &str, suspended: bool) {
    let cards = all_cards();
    let mut store = STORE.write().unwrap();
    for card in cards.iter().filter(|c| c.in_deck(deck)) {
        store.file.cards.entry(card.raw.id.clone()).or_insert_with(|| CardState { note: card.note_rel.clone(), question: card.raw.front.clone(), ..Default::default() }).suspended = suspended;
    }
    drop(store);
    save();
}

/// Puts a card aside until tomorrow.
pub fn bury(card: &Card, at: i64) {
    edit_state(card, |s| s.buried_until = Some(next_day_start(at)));
}

/// Starts a card over, as if it had never been seen.
pub fn reset(card: &Card) {
    let mut store = STORE.write().unwrap();
    store.file.cards.remove(&card.raw.id);
    drop(store);
    save();
}

/// Reviews and new cards already done today, for the daily limits.
fn done_today(log: &[ReviewLog], at: i64) -> (usize, usize) {
    let today = day_of(at);
    let mut new = 0;
    let mut reviews = 0;
    for entry in log.iter().rev() {
        if day_of(entry.at) != today {
            break;
        }
        match entry.phase {
            Phase::New => new += 1,
            Phase::Review => reviews += 1,
            _ => {}
        }
    }
    (new, reviews)
}

/// What is left to study in a deck, in the order it comes: waiting cards first, new ones spread among reviews.
pub fn queue(deck: &str, at: i64) -> Vec<Card> {
    let cards = all_cards();
    let p = prefs::get();
    let log_counts = done_today(&STORE.read().unwrap().log, at);
    let new_room = (p.card_new_per_day as usize).saturating_sub(log_counts.0);
    let review_room = (p.card_reviews_per_day as usize).saturating_sub(log_counts.1);
    let usable = cards.iter().filter(|c| c.in_deck(deck) && !c.suspended() && !c.buried(at));
    let mut learning: Vec<&Card> = Vec::new();
    let mut reviews: Vec<&Card> = Vec::new();
    let mut new: Vec<&Card> = Vec::new();
    let mut groups = HashSet::new();
    for card in usable {
        match card.phase() {
            Phase::Learning | Phase::Relearning => {
                groups.insert(card.raw.group.clone());
                if card.state.as_ref().is_some_and(|s| s.due <= at + LEARN_AHEAD_MINUTES * 60) {
                    learning.push(card);
                }
            }
            Phase::Review => {
                if card.state.as_ref().is_some_and(|s| s.due < next_day_start(at)) {
                    reviews.push(card);
                }
            }
            Phase::New => new.push(card),
        }
    }
    learning.sort_by_key(|c| c.state.as_ref().map_or(0, |s| s.due));
    reviews.sort_by_key(|c| c.state.as_ref().map_or(0, |s| s.due));
    let mut picked_reviews = Vec::new();
    for card in reviews {
        if picked_reviews.len() < review_room && groups.insert(card.raw.group.clone()) {
            picked_reviews.push(card);
        }
    }
    let mut picked_new = Vec::new();
    for card in new {
        if picked_new.len() < new_room && groups.insert(card.raw.group.clone()) {
            picked_new.push(card);
        }
    }
    let mut out: Vec<Card> = learning.iter().map(|c| (*c).clone()).collect();
    let every = if picked_new.is_empty() { usize::MAX } else { (picked_reviews.len() / picked_new.len()).max(1) };
    let mut next_new = picked_new.into_iter();
    for (i, card) in picked_reviews.iter().enumerate() {
        out.push((*card).clone());
        if (i + 1) % every == 0
            && let Some(n) = next_new.next()
        {
            out.push(n.clone());
        }
    }
    out.extend(next_new.cloned());
    out
}

/// The counts of a deck for right now; the sidebar asks often, so they are kept for a minute.
pub fn counts(deck: &str, at: i64) -> Counts {
    let key = {
        let store = STORE.read().unwrap();
        (deck.to_string(), at / 60, index::current().generation, store.revision)
    };
    if let Some(found) = STORE.read().unwrap().counts.get(&key) {
        return *found;
    }
    let mut result = Counts::default();
    for card in queue(deck, at) {
        match card.phase() {
            Phase::New => result.new += 1,
            Phase::Learning | Phase::Relearning => result.learning += 1,
            Phase::Review => result.review += 1,
        }
    }
    let mut store = STORE.write().unwrap();
    if store.counts.len() > 2000 {
        store.counts.clear();
    }
    store.counts.insert(key, result);
    result
}

/// How a deck is doing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub total: usize,
    pub new: usize,
    /// In learning or with an interval below three weeks.
    pub young: usize,
    pub mature: usize,
    pub suspended: usize,
    pub leeches: usize,
    pub reviewed_today: usize,
    /// How often reviewed cards were remembered in the last 30 days; `None` before there is enough to say.
    pub true_retention: Option<f64>,
    /// Days in a row with at least one review, ending today or yesterday.
    pub streak: usize,
    /// Cards coming due on each of the next days, today first.
    pub forecast: Vec<usize>,
}

pub fn stats(deck: &str, at: i64) -> Stats {
    let cards = all_cards();
    let in_deck: Vec<&Card> = cards.iter().filter(|c| c.in_deck(deck)).collect();
    let ids: HashSet<&str> = in_deck.iter().map(|c| c.raw.id.as_str()).collect();
    let mut result = Stats { total: in_deck.len(), forecast: vec![0; 14], ..Default::default() };
    let today = day_of(at);
    for card in &in_deck {
        if card.suspended() {
            result.suspended += 1;
        }
        if card.is_leech() {
            result.leeches += 1;
        }
        let Some(state) = card.state.as_ref().filter(|s| s.phase != Phase::New) else {
            result.new += 1;
            continue;
        };
        let days_left = (state.due - state.last_review.unwrap_or(state.due)) / 86400;
        if state.phase == Phase::Review && days_left >= 21 {
            result.mature += 1;
        } else {
            result.young += 1;
        }
        if !card.suspended() {
            let slot = (day_of(state.due) - today).num_days().max(0) as usize;
            if let Some(count) = result.forecast.get_mut(slot) {
                *count += 1;
            }
        }
    }
    let store = STORE.read().unwrap();
    let (mut asked, mut remembered) = (0usize, 0usize);
    let mut days: Vec<NaiveDate> = Vec::new();
    let month = at - 30 * 86400;
    for entry in store.log.iter().filter(|l| deck.is_empty() || ids.contains(l.card.as_str())) {
        let day = day_of(entry.at);
        if day == today {
            result.reviewed_today += 1;
        }
        if days.last() != Some(&day) {
            days.push(day);
        }
        if entry.at >= month && entry.phase == Phase::Review {
            asked += 1;
            if entry.rating != Rating::Again {
                remembered += 1;
            }
        }
    }
    if asked >= 10 {
        result.true_retention = Some(remembered as f64 / asked as f64);
    }
    days.dedup();
    let mut expected = today;
    if days.last() != Some(&today) {
        expected = today - Duration::days(1);
    }
    for day in days.iter().rev() {
        if *day == expected {
            result.streak += 1;
            expected -= Duration::days(1);
        } else {
            break;
        }
    }
    result
}

/// When the next card of a deck comes up after today.
pub fn next_due(deck: &str, at: i64) -> Option<i64> {
    all_cards().iter().filter(|c| c.in_deck(deck) && !c.suspended()).filter_map(|c| c.state.as_ref()).filter(|s| s.phase != Phase::New && s.due > at).map(|s| s.due).min()
}

/// Whichever side reviewed a card last wins; the log is simply the union of both.
pub fn merge_store(ours: &str, theirs: &str) -> Option<String> {
    let (mut a, b): (StoreFile, StoreFile) = (serde_json::from_str(ours).ok()?, serde_json::from_str(theirs).ok()?);
    for (id, state) in b.cards {
        let newer = a.cards.get(&id).is_none_or(|mine| state.last_review > mine.last_review);
        if newer {
            a.cards.insert(id, state);
        }
    }
    serde_json::to_string_pretty(&a).ok()
}

/// Both sides' reviews, in the order they happened.
pub fn merge_log(ours: &str, theirs: &str) -> String {
    let mut lines: Vec<(i64, &str)> = ours.lines().chain(theirs.lines()).filter_map(|l| serde_json::from_str::<ReviewLog>(l).ok().map(|e| (e.at, l))).collect();
    lines.sort();
    lines.dedup();
    lines.into_iter().map(|(_, l)| format!("{l}\n")).collect()
}

/// A plain sentence for lists: formulas, links and marks are written out briefly.
pub fn plain_text(markdown_text: &str) -> String {
    static NOISE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[*_`]|\[\[|\]\]|==").unwrap());
    NOISE.replace_all(markdown_text, "").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The kind as a short word for lists.
pub fn kind_icon(kind: CardKind) -> &'static str {
    match kind {
        CardKind::Basic => "file-question",
        CardKind::Reverse => "repeat-2",
        CardKind::Cloze => "scan",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: i64, hour: i64) -> i64 {
        let start = Local.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap().timestamp();
        start + day * 86400 + hour * 3600
    }

    fn ask(state: Option<&CardState>, moment: i64) -> [CardState; 4] {
        options("abc", state, "n.md", "Frage", moment)
    }

    #[test]
    fn a_new_card_walks_through_the_learning_steps() {
        let now = at(0, 12);
        let [again, hard, good, easy] = ask(None, now);
        assert_eq!((again.phase, again.due), (Phase::Learning, now + 60));
        assert_eq!(hard.phase, Phase::Learning);
        assert!(hard.due > again.due && hard.due < good.due);
        assert_eq!((good.phase, good.due, good.step), (Phase::Learning, now + 600, 1));
        assert_eq!(easy.phase, Phase::Review);
        assert!(easy.due >= next_day_start(now));
        let later = now + 600;
        let graduated = ask(Some(&good), later)[2].clone();
        assert_eq!(graduated.phase, Phase::Review);
        assert!(graduated.due >= next_day_start(later));
    }

    #[test]
    fn longer_waits_for_better_answers() {
        let now = at(0, 12);
        let state = CardState { phase: Phase::Review, stability: 20., difficulty: 5., due: now, last_review: Some(now - 20 * 86400), reps: 5, ..Default::default() };
        let [again, hard, good, easy] = ask(Some(&state), now);
        assert_eq!(again.phase, Phase::Relearning);
        assert_eq!(again.lapses, 1);
        assert!(hard.due < good.due && good.due < easy.due);
        assert!(again.due < hard.due);
    }

    #[test]
    fn the_buttons_tell_the_truth() {
        let now = at(3, 9);
        let state = CardState { phase: Phase::Review, stability: 40., difficulty: 6., due: now, last_review: Some(now - 40 * 86400), reps: 9, ..Default::default() };
        assert_eq!(ask(Some(&state), now), ask(Some(&state), now));
    }

    #[test]
    fn a_new_day_starts_at_four() {
        assert_eq!(days_between(at(0, 23), at(1, 3)), 0);
        assert_eq!(days_between(at(0, 23), at(1, 5)), 1);
    }

    #[test]
    fn labels_read_like_a_person_says_them() {
        let now = at(0, 0);
        assert!(wait_label(now, now + 600).contains("10"));
        assert!(wait_label(now, now + 10 * 86400).contains("10"));
    }

    #[test]
    fn merging_keeps_the_later_review_and_all_of_the_log() {
        let a = r#"{"version":0,"cards":{"x":{"lastReview":5,"stability":1.0},"y":{"lastReview":9}}}"#;
        let b = r#"{"version":0,"cards":{"x":{"lastReview":7,"stability":2.0},"y":{"lastReview":1},"z":{"lastReview":1}}}"#;
        let merged: StoreFile = serde_json::from_str(&merge_store(a, b).unwrap()).unwrap();
        assert_eq!(merged.cards["x"].stability, 2.);
        assert_eq!(merged.cards["y"].last_review, Some(9));
        assert!(merged.cards.contains_key("z"));
        let line = |t: i64| format!("{{\"card\":\"c\",\"at\":{t},\"rating\":\"good\",\"phase\":\"review\",\"days\":1.0}}\n");
        let log = merge_log(&(line(1) + &line(3)), &(line(2) + &line(3)));
        assert_eq!(log.lines().count(), 3);
    }
}
