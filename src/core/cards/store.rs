use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use chrono::{Duration, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

use super::fsrs::{self, Memory, Rating};
use super::parse::{CardKind, RawCard, SrProgress};
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
    /// When the cards a vault held at its first open were taken as known; stores from before this have none and count as done.
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline: Option<i64>,
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
    pub raw: Arc<RawCard>,
    pub note: Arc<PathBuf>,
    pub note_rel: Arc<String>,
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
        deck.is_empty() || self.note_rel.strip_prefix(deck).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
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
    /// The cards as the notes say, for one state of the vault.
    written: Option<(u64, Arc<Vec<(Arc<RawCard>, Arc<PathBuf>, Arc<String>)>>)>,
    counts: HashMap<(String, i64, u64, u64), Counts>,
    stats: HashMap<(String, i64, u64, u64), Stats>,
    /// Size and time of the file as last read or written; anything else came from outside, like a sync.
    stamps: [Option<(u64, std::time::SystemTime)>; 2],
    /// A new state of the cards still has to reach the disk.
    dirty: bool,
    /// The vault has never had cards remembered: what it holds now is taken as known on the next settle.
    first_open: bool,
    /// Cards taken as known that nobody was told about yet.
    notice: Option<Adopted>,
}

static STORE: LazyLock<RwLock<Store>> = LazyLock::new(Default::default);
/// New cards allowed on top of the daily limit today, with the day it counts for.
static EXTRA_NEW: Mutex<(Option<NaiveDate>, usize)> = Mutex::new((None, 0));
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

fn stamp_of(path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Reads what is known about the cards of a vault, off the main thread.
/// When the files changed behind the app's back, like after a sync, both states are joined: the later review wins.
pub fn open(root: &Path) {
    let (store_path, log_path) = (root.join(STORE_NAME), root.join(LOG_NAME));
    let (store_stamp, log_stamp) = (stamp_of(&store_path), stamp_of(&log_path));
    let same_root = {
        let store = STORE.read().unwrap();
        let same = store.root.as_deref() == Some(root);
        if same && store.stamps == [store_stamp, log_stamp] {
            return;
        }
        if same && store.dirty {
            return;
        }
        same
    };
    let file: StoreFile = std::fs::read(&store_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let log = read_log(&log_path);
    let mut store = STORE.write().unwrap();
    if same_root {
        let theirs = file.baseline;
        for (id, state) in file.cards {
            if store.file.cards.get(&id).is_none_or(|mine| state.last_review > mine.last_review || (state.last_review == mine.last_review && state != *mine)) {
                store.file.cards.insert(id, state);
            }
        }
        if log.len() != store.log.len() {
            store.log = log;
        }
        if store.file.baseline.is_none() {
            store.file.baseline = theirs;
        }
        store.stamps = [store_stamp, log_stamp];
        store.revision += 1;
        store.cache = None;
        store.counts.clear();
        store.stats.clear();
    } else {
        let first_open = never_used(root);
        *store = Store { root: Some(root.to_path_buf()), file, log, revision: store.revision + 1, stamps: [store_stamp, log_stamp], first_open, ..Default::default() };
    }
}

/// No trace of remembered cards at all, not even a sync copy or a file still in the cloud.
fn never_used(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else { return false };
    !entries.flatten().any(|e| e.file_name().to_string_lossy().contains("schrumbi-cards"))
}

fn read_log(path: &Path) -> Vec<ReviewLog> {
    std::fs::read_to_string(path).map(|text| text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// Forgets the open vault's cards, after writing down what is still waiting.
pub fn close() {
    flush();
    let mut store = STORE.write().unwrap();
    *store = Store { revision: store.revision + 1, ..Default::default() };
}

/// Writes the cards' state now, if anything is waiting. Called on quitting.
pub fn flush() {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (root, json) = {
        let mut store = STORE.write().unwrap();
        if !store.dirty {
            return;
        }
        store.dirty = false;
        let Some(root) = store.root.clone() else { return };
        let Ok(json) = serde_json::to_vec_pretty(&store.file) else { return };
        (root, json)
    };
    let path = root.join(STORE_NAME);
    crate::core::workspace::backup(&path);
    let tmp = root.join(format!("{STORE_NAME}.tmp"));
    if std::fs::write(&tmp, json).is_ok() && std::fs::rename(tmp, &path).is_ok() {
        let stamp = stamp_of(&path);
        STORE.write().unwrap().stamps[0] = stamp;
    }
}

/// Marks the state as changed; it reaches the disk a moment later, off the main thread, however fast answers come.
fn save() {
    let start = {
        let mut store = STORE.write().unwrap();
        store.revision += 1;
        store.cache = None;
        store.counts.clear();
        store.stats.clear();
        !std::mem::replace(&mut store.dirty, true)
    };
    if start && !cfg!(test) {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(250));
            flush();
        });
    }
}

fn rewrite_log(store: &mut Store) {
    let Some(root) = store.root.clone() else { return };
    let rest: String = store.log.iter().filter_map(|l| serde_json::to_string(l).ok()).map(|l| l + "\n").collect();
    let (tmp, path) = (root.join(format!("{LOG_NAME}.tmp")), root.join(LOG_NAME));
    if std::fs::write(&tmp, rest).is_ok() && std::fs::rename(tmp, &path).is_ok() {
        store.stamps[1] = stamp_of(&path);
    }
}

fn append_log(root: &Path, entry: &ReviewLog) {
    if let (Ok(mut file), Ok(line)) = (std::fs::OpenOptions::new().create(true).append(true).open(root.join(LOG_NAME)), serde_json::to_string(entry)) {
        let _ = writeln!(file, "{line}");
    }
    let stamp = stamp_of(&root.join(LOG_NAME));
    if let Ok(mut store) = STORE.try_write() {
        store.stamps[1] = stamp;
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
    let cards = Arc::new(collect(&vault, &mut store));
    let revision = store.revision;
    store.cache = Some((vault.generation, revision, cards.clone()));
    cards
}

fn written_cards(vault: &VaultIndex) -> Vec<(Arc<RawCard>, Arc<PathBuf>, Arc<String>)> {
    let mut notes: Vec<(&PathBuf, &index::NoteFacts)> = vault.notes.iter().filter(|(p, f)| !f.cards.is_empty() && !vault.is_template(p) && markdown::is_markdown(p)).collect();
    notes.sort_by(|a, b| a.0.cmp(b.0));
    let mut seen = HashSet::new();
    let mut cards = Vec::new();
    for (path, facts) in notes {
        let rel = path.strip_prefix(&vault.root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        if vault.config.is_ignored(&rel) {
            continue;
        }
        let (path, rel) = (Arc::new(path.clone()), Arc::new(rel));
        for raw in &facts.cards {
            if seen.insert(raw.id.clone()) {
                cards.push((Arc::new(raw.clone()), path.clone(), rel.clone()));
            }
        }
    }
    cards
}

fn collect(vault: &VaultIndex, store: &mut Store) -> Vec<Card> {
    let written = match &store.written {
        Some((generation, list)) if *generation == vault.generation => list.clone(),
        _ => {
            let list = Arc::new(written_cards(vault));
            store.written = Some((vault.generation, list.clone()));
            list
        }
    };
    written.iter().map(|(raw, note, rel)| Card { state: store.file.cards.get(&raw.id).cloned(), raw: raw.clone(), note: note.clone(), note_rel: rel.clone() }).collect()
}

/// Keeps progress when a question was reworded: a card without state takes over the most similar lost one of its note.
pub fn reconcile() {
    let vault = index::current();
    let mut store = STORE.write().unwrap();
    if store.root.is_none() || store.file.cards.is_empty() {
        return;
    }
    let cards = collect(&vault, &mut store);
    let known: HashSet<&str> = cards.iter().map(|c| c.raw.id.as_str()).collect();
    let mut moved: Vec<(String, String)> = Vec::new();
    let mut claimed: HashSet<&str> = HashSet::new();
    for card in cards.iter().filter(|c| c.state.is_none()) {
        if let Some(old) = card.raw.legacy.as_deref().filter(|old| !known.contains(old) && store.file.cards.contains_key(*old) && claimed.insert(old)) {
            moved.push((old.to_string(), card.raw.id.clone()));
        }
    }
    let mut lost: HashMap<String, Vec<String>> = HashMap::new();
    for (id, state) in &store.file.cards {
        if !known.contains(id.as_str()) && !claimed.contains(id.as_str()) {
            lost.entry(state.note.clone()).or_default().push(id.clone());
        }
    }
    let mut seen_here: Vec<(String, String)> = Vec::new();
    for card in &cards {
        match &card.state {
            Some(state) if state.note != *card.note_rel || state.question != card.raw.front => seen_here.push((card.raw.id.clone(), card.note_rel.to_string())),
            Some(_) => {}
            None if moved.iter().any(|(_, new)| *new == card.raw.id) => {}
            None => {
                let Some(candidates) = lost.get_mut(card.note_rel.as_str()) else { continue };
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
    let notes: HashMap<&str, &str> = cards.iter().map(|c| (c.raw.id.as_str(), c.note_rel.as_str())).collect();
    let mut logged = false;
    for (old, new) in moved {
        if let Some(mut state) = store.file.cards.remove(&old) {
            state.question = fronts.get(new.as_str()).unwrap_or(&"").to_string();
            state.note = notes.get(new.as_str()).unwrap_or(&"").to_string();
            for entry in store.log.iter_mut().filter(|l| l.card == old) {
                entry.card = new.clone();
                logged = true;
            }
            store.file.cards.insert(new, state);
        }
    }
    if logged {
        rewrite_log(&mut store);
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

/// What taking cards as known did, to tell about and to take back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Adopted {
    /// Cards that got a state.
    pub count: usize,
    /// Of those, the ones spread as reviews over the coming days.
    pub spread: usize,
    /// The last of those days, counted from today.
    pub days: i64,
    /// How the spread cards stood before.
    pub before: Vec<(String, Option<CardState>)>,
}

impl Adopted {
    /// How far the cards were spread, as the end of a sentence.
    pub fn spread_words(&self) -> String {
        use crate::core::i18n::{tr, trf};
        match self.days {
            ..=1 => tr("fällig ab morgen"),
            d @ 2..=13 => trf("verteilt auf die nächsten %@ Tage", &[&d.to_string()]),
            d => trf("verteilt auf die nächsten %@ Wochen", &[&((d + 3) / 7).to_string()]),
        }
    }

    /// One calm sentence for a toast.
    pub fn message(&self) -> String {
        use crate::core::i18n::{tr, trf};
        let head = if self.count == 1 { tr("1 Karte übernommen") } else { trf("%@ Karten übernommen", &[&self.count.to_string()]) };
        if self.spread == 0 { head } else { format!("{head}, {}", self.spread_words()) }
    }
}

/// The moment a study day begins.
fn day_start(day: NaiveDate) -> i64 {
    day.and_hms_opt(DAY_STARTS_AT as u32, 0, 0).and_then(|d| Local.from_local_datetime(&d).earliest()).map_or_else(|| day.and_hms_opt(12, 0, 0).map_or(0, |d| d.and_utc().timestamp()), |d| d.timestamp())
}

/// The progress the Obsidian Spaced Repetition plugin noted: its interval stands in for stability, its ease for difficulty.
fn sr_state(sr: &SrProgress, note: &str, question: &str) -> CardState {
    let due = day_start(sr.due);
    let interval = sr.interval.max(1) as f64;
    CardState {
        phase: Phase::Review,
        stability: interval.min(36500.),
        difficulty: (10. - (sr.ease as f64 - 130.) * 9. / 220.).clamp(1., 10.),
        due,
        last_review: Some(due - sr.interval.max(1) as i64 * 86400),
        reps: 1,
        note: note.to_string(),
        question: question.to_string(),
        ..Default::default()
    }
}

/// Known cards as reviews over the coming two to four weeks, in a fixed order, never more on one day than the daily limit allows.
fn spread(items: &[(String, String, String)], states: &BTreeMap<String, CardState>, at: i64, per_day: usize, retention: f64) -> (Vec<(String, CardState)>, i64) {
    let mut order: Vec<&(String, String, String)> = items.iter().collect();
    order.sort_by(|a, b| a.0.cmp(&b.0));
    order.dedup_by(|a, b| a.0 == b.0);
    let count = order.len();
    if count == 0 {
        return (Vec::new(), 0);
    }
    let (span, per_day, today) = (count.clamp(14, 28), per_day.max(1), day_of(at));
    let mut load: HashMap<i64, usize> = HashMap::new();
    for state in states.values().filter(|s| s.phase != Phase::New && !s.suspended) {
        *load.entry((day_of(state.due) - today).num_days()).or_default() += 1;
    }
    let unit = fsrs::interval(1., retention).max(0.01);
    let mut last = 0;
    let mut out = Vec::with_capacity(count);
    for (i, (id, note, question)) in order.into_iter().enumerate() {
        let mut day = 1 + (i * span / count) as i64;
        while load.get(&day).copied().unwrap_or(0) >= per_day {
            day += 1;
        }
        *load.entry(day).or_default() += 1;
        last = last.max(day);
        out.push((id.clone(), CardState { phase: Phase::Review, stability: day as f64 / unit, difficulty: 5., due: day_start(today + Duration::days(day)), last_review: Some(at), note: note.clone(), question: question.clone(), ..Default::default() }));
    }
    (out, last)
}

/// Gives stateless cards what is already known about them: the plugin's progress, and on a first open, that they are known.
fn settle_file(file: &mut StoreFile, cards: &[Card], first_open: bool, at: i64, per_day: usize, retention: f64) -> Option<Adopted> {
    let mut adopted = Adopted::default();
    for card in cards.iter().filter(|c| c.state.is_none()) {
        if let Some(sr) = card.raw.sr.as_ref().filter(|_| !file.cards.contains_key(&card.raw.id)) {
            file.cards.insert(card.raw.id.clone(), sr_state(sr, &card.note_rel, &card.raw.front));
            adopted.count += 1;
        }
    }
    if first_open {
        let fresh: Vec<(String, String, String)> = cards.iter().filter(|c| !file.cards.contains_key(&c.raw.id)).map(|c| (c.raw.id.clone(), c.note_rel.to_string(), c.raw.front.clone())).collect();
        let (states, days) = spread(&fresh, &file.cards, at, per_day, retention);
        adopted.spread = states.len();
        adopted.count += states.len();
        adopted.days = days;
        for (id, state) in states {
            adopted.before.push((id.clone(), None));
            file.cards.insert(id, state);
        }
        file.baseline = Some(at);
    }
    (adopted.count > 0 || first_open).then_some(adopted)
}

/// Gives cards their first state where it is already known: once for all cards a vault holds when it is first opened,
/// and whenever a card without progress carries the Obsidian Spaced Repetition plugin's. The notes are never written.
pub fn settle(at: i64) {
    let vault = index::current();
    let mut store = STORE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(root) = store.root.clone() else { return };
    if vault.root != root {
        return;
    }
    let first_open = std::mem::take(&mut store.first_open);
    let cards = collect(&vault, &mut store);
    let p = prefs::get();
    let Some(adopted) = settle_file(&mut store.file, &cards, first_open, at, p.card_reviews_per_day as usize, p.card_retention.clamp(0.7, 0.99)) else { return };
    if first_open && adopted.count > 0 {
        store.notice = Some(adopted);
    }
    drop(store);
    save();
}

/// Cards taken as known on a first open that nobody was told about yet.
pub fn take_notice() -> Option<Adopted> {
    STORE.write().unwrap_or_else(std::sync::PoisonError::into_inner).notice.take()
}

/// New cards of a deck that could be taken as known; paused ones stay as they are.
pub fn fresh_count(deck: &str) -> usize {
    all_cards().iter().filter(|c| c.in_deck(deck) && c.phase() == Phase::New && !c.suspended()).count()
}

/// Takes the new cards of a deck as already known, spread as reviews over the coming weeks. `restore(before)` takes it back.
pub fn adopt_new_as_known(deck: &str, at: i64) -> Option<Adopted> {
    let cards = all_cards();
    let fresh: Vec<(String, String, String)> = cards.iter().filter(|c| c.in_deck(deck) && c.phase() == Phase::New && !c.suspended()).map(|c| (c.raw.id.clone(), c.note_rel.to_string(), c.raw.front.clone())).collect();
    if fresh.is_empty() {
        return None;
    }
    let p = prefs::get();
    let mut store = STORE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (states, days) = spread(&fresh, &store.file.cards, at, p.card_reviews_per_day as usize, p.card_retention.clamp(0.7, 0.99));
    let mut adopted = Adopted { count: states.len(), spread: states.len(), days, before: Vec::new() };
    for (id, state) in states {
        let before = store.file.cards.insert(id.clone(), state);
        adopted.before.push((id, before));
    }
    drop(store);
    save();
    Some(adopted)
}

/// Moves a card's progress to a new identity after it was edited here.
pub fn rename(old: &str, new: &str) {
    if old == new {
        return;
    }
    let mut store = STORE.write().unwrap();
    if let Some(state) = store.file.cards.remove(old) {
        store.file.cards.insert(new.to_string(), state);
        let mut logged = false;
        for entry in store.log.iter_mut().filter(|l| l.card == old) {
            entry.card = new.to_string();
            logged = true;
        }
        if logged {
            rewrite_log(&mut store);
        }
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

/// Short words for how long until a card comes back. Days count as study days, so tomorrow is one day at any hour.
pub fn wait_label(from: i64, due: i64) -> String {
    use crate::core::i18n::{Lang, lang, tr, trf};
    let minutes = (due - from) / 60;
    if minutes < 60 {
        return trf("%@ Min.", &[&minutes.max(1).to_string()]);
    }
    let days = days_between(from, due);
    if minutes < 120 || days == 0 {
        return trf("%@ Std.", &[&(minutes / 60).to_string()]);
    }
    let number = |value: f64| {
        let text = format!("{value:.1}");
        let text = text.strip_suffix(".0").unwrap_or(&text).to_string();
        if lang() == Lang::En { text } else { text.replace('.', ",") }
    };
    match days {
        1 => tr("1 Tag"),
        d if d < 31 => trf("%@ Tage", &[&d.to_string()]),
        d if d < 365 => trf("%@ Mon.", &[&number(d as f64 / 30.4)]),
        d => trf("%@ J.", &[&number(d as f64 / 365.)]),
    }
}

/// Everything needed to take an answer back.
#[derive(Debug, Clone)]
pub struct Answered {
    pub id: String,
    before: Option<CardState>,
    buried: Vec<(String, Option<CardState>)>,
    logged: bool,
}

/// Remembers a card as it is, so a change that is not an answer can be taken back too.
pub fn before_change(card: &Card) -> Answered {
    Answered { id: card.raw.id.clone(), before: STORE.read().unwrap().file.cards.get(&card.raw.id).cloned(), buried: Vec::new(), logged: false }
}

/// Saves an answer and puts the other cards of the same place aside until tomorrow.
pub fn answer(card: &Card, rating: Rating, at: i64) -> Answered {
    let chosen = options(&card.raw.id, card.state.as_ref(), card.note_rel.as_str(), &card.raw.front, at)[rating as usize - 1].clone();
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
            let entry = store.file.cards.entry(sibling.raw.id.clone()).or_insert_with(|| CardState { note: sibling.note_rel.to_string(), question: sibling.raw.front.clone(), ..Default::default() });
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
    Answered { id: card.raw.id.clone(), before, buried, logged: true }
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
    if answered.logged {
        if let Some(at) = store.log.iter().rposition(|l| l.card == answered.id) {
            store.log.remove(at);
        }
        rewrite_log(&mut store);
    }
    drop(store);
    save();
}

fn edit_state(card: &Card, change: impl FnOnce(&mut CardState)) {
    let mut store = STORE.write().unwrap();
    let state = store.file.cards.entry(card.raw.id.clone()).or_insert_with(|| CardState { note: card.note_rel.to_string(), question: card.raw.front.clone(), ..Default::default() });
    change(state);
    drop(store);
    save();
}

pub fn set_suspended(card: &Card, suspended: bool) {
    edit_state(card, |s| s.suspended = suspended);
}

/// How some cards stand now, to bring them back after a change that hits many.
pub fn snapshot(ids: &[String]) -> Vec<(String, Option<CardState>)> {
    let store = STORE.read().unwrap();
    ids.iter().map(|id| (id.clone(), store.file.cards.get(id).cloned())).collect()
}

/// Brings cards back to how a snapshot found them.
pub fn restore(saved: Vec<(String, Option<CardState>)>) {
    let mut store = STORE.write().unwrap();
    for (id, state) in saved {
        match state {
            Some(state) => {
                store.file.cards.insert(id, state);
            }
            None => {
                store.file.cards.remove(&id);
            }
        }
    }
    drop(store);
    save();
}

/// Pauses or resumes many cards in one step.
pub fn set_many_suspended(cards: &[Card], suspended: bool) {
    let mut store = STORE.write().unwrap();
    for card in cards {
        store.file.cards.entry(card.raw.id.clone()).or_insert_with(|| CardState { note: card.note_rel.to_string(), question: card.raw.front.clone(), ..Default::default() }).suspended = suspended;
    }
    drop(store);
    save();
}

/// Starts many cards over.
pub fn reset_many(cards: &[Card]) {
    let mut store = STORE.write().unwrap();
    for card in cards {
        store.file.cards.insert(card.raw.id.clone(), started_over(card));
    }
    drop(store);
    save();
}

/// Brings a card put aside until tomorrow back today.
pub fn unbury(card: &Card) {
    edit_state(card, |s| s.buried_until = None);
}

/// Pauses or resumes all cards of a deck in one step.
pub fn set_deck_suspended(deck: &str, suspended: bool) {
    let cards = all_cards();
    let mut store = STORE.write().unwrap();
    for card in cards.iter().filter(|c| c.in_deck(deck)) {
        store.file.cards.entry(card.raw.id.clone()).or_insert_with(|| CardState { note: card.note_rel.to_string(), question: card.raw.front.clone(), ..Default::default() }).suspended = suspended;
    }
    drop(store);
    save();
}

/// Puts a card aside until tomorrow.
pub fn bury(card: &Card, at: i64) {
    edit_state(card, |s| s.buried_until = Some(next_day_start(at)));
}

/// A card as if it had never been seen, kept so the plugin's progress beside it is not taken over again.
fn started_over(card: &Card) -> CardState {
    CardState { note: card.note_rel.to_string(), question: card.raw.front.clone(), ..Default::default() }
}

/// Starts a card over, as if it had never been seen.
pub fn reset(card: &Card) {
    let mut store = STORE.write().unwrap();
    store.file.cards.insert(card.raw.id.clone(), started_over(card));
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
    let extra = EXTRA_NEW.lock().map_or(0, |e| if e.0 == Some(day_of(at)) { e.1 } else { 0 });
    let new_room = (p.card_new_per_day as usize + extra).saturating_sub(log_counts.0);
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

/// Lets more new cards come today than the daily limit says.
pub fn allow_more_new(count: usize, at: i64) {
    if let Ok(mut extra) = EXTRA_NEW.lock() {
        let today = day_of(at);
        *extra = (Some(today), if extra.0 == Some(today) { extra.1 + count } else { count });
    }
    let mut store = STORE.write().unwrap();
    store.counts.clear();
}

/// New cards of a deck that the daily limit holds back.
pub fn new_waiting(deck: &str, at: i64) -> usize {
    all_cards().iter().filter(|c| c.in_deck(deck) && c.phase() == Phase::New && !c.suspended() && !c.buried(at)).count()
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
    let key = (deck.to_string(), at / 300, index::current().generation, STORE.read().unwrap().revision);
    if let Some(found) = STORE.read().unwrap().stats.get(&key) {
        return found.clone();
    }
    let result = work_out_stats(deck, at);
    let mut store = STORE.write().unwrap();
    if store.stats.len() > 500 {
        store.stats.clear();
    }
    store.stats.insert(key, result.clone());
    result
}

fn work_out_stats(deck: &str, at: i64) -> Stats {
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

/// Adds what an import brought along: progress and past reviews. Cards never reviewed there are taken as known and spread.
pub fn adopt_import(states: Vec<(String, CardState)>, log: Vec<ReviewLog>, fresh: Vec<(String, String, String)>, at: i64) -> Option<Adopted> {
    let p = prefs::get();
    let mut store = STORE.write().unwrap();
    for (id, state) in states {
        store.file.cards.insert(id, state);
    }
    let fresh: Vec<(String, String, String)> = fresh.into_iter().filter(|(id, ..)| !store.file.cards.contains_key(id)).collect();
    let (spread_states, days) = spread(&fresh, &store.file.cards, at, p.card_reviews_per_day as usize, p.card_retention.clamp(0.7, 0.99));
    let adopted = (!spread_states.is_empty()).then(|| Adopted { count: spread_states.len(), spread: spread_states.len(), days, before: spread_states.iter().map(|(id, _)| (id.clone(), None)).collect() });
    for (id, state) in spread_states {
        store.file.cards.insert(id, state);
    }
    let root = store.root.clone();
    store.log.extend(log.iter().cloned());
    store.log.sort_by_key(|l| l.at);
    drop(store);
    if let Some(root) = root {
        for entry in &log {
            append_log(&root, entry);
        }
    }
    save();
    adopted
}

/// The cards known so far, to leave out what an import already finds.
pub fn known_ids() -> HashSet<String> {
    all_cards().iter().map(|c| c.raw.id.clone()).collect()
}

/// Changes whenever the state of any card changed.
pub fn revision() -> u64 {
    STORE.read().unwrap().revision
}

/// When the next card of a deck comes up after today.
pub fn next_due(deck: &str, at: i64) -> Option<i64> {
    all_cards().iter().filter(|c| c.in_deck(deck) && !c.suspended()).filter_map(|c| c.state.as_ref()).filter(|s| s.phase != Phase::New && s.due > at).map(|s| s.due).min()
}

/// Whichever side reviewed a card last wins; the log is simply the union of both.
pub fn merge_store(ours: &str, theirs: &str) -> Option<String> {
    let (mut a, b): (StoreFile, StoreFile) = (serde_json::from_str(ours).ok()?, serde_json::from_str(theirs).ok()?);
    a.baseline = a.baseline.or(b.baseline);
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
    static NOISE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[*_`$]|\[\[|\]\]|==|(?:^|\s)#[\p{L}\p{N}/_-]+").unwrap());
    static HIDDEN: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\\boxed\{\\text\{\?\}\}").unwrap());
    let text = HIDDEN.replace_all(markdown_text, "[...]");
    NOISE.replace_all(&text, "").split_whitespace().collect::<Vec<_>>().join(" ")
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
        let noon = at(0, 12);
        assert_eq!(wait_label(noon, next_day_start(noon)), "1 Tag");
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

#[cfg(test)]
mod first_open {
    use super::*;
    use crate::core::cards::parse;

    fn vault(name: &str, notes: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("schrumbi-cards-first-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in notes {
            std::fs::write(dir.join(file), text).unwrap();
        }
        dunce::canonicalize(dir).unwrap()
    }

    fn scan(root: &Path) {
        index::rebuild(root, &[]);
        open(root);
        reconcile();
        settle(now());
    }

    fn reopen(root: &Path) {
        close();
        scan(root);
    }

    fn state_of(front: &str) -> Option<CardState> {
        all_cards().iter().find(|c| c.raw.front == front).and_then(|c| c.state.clone())
    }

    fn phase_of(front: &str) -> Phase {
        all_cards().iter().find(|c| c.raw.front == front).map(Card::phase).unwrap()
    }

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::core::edge_tests::VAULT_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn what_a_vault_holds_at_its_first_open_is_known_once() {
        let _guard = lock();
        let root = vault("once", &[("A.md", "Eins? :: 1\nZwei? :: 2\nDrei ::: 3\n")]);
        let at = now();
        scan(&root);
        let cards = all_cards();
        assert_eq!(cards.len(), 4);
        for card in cards.iter() {
            let state = card.state.as_ref().unwrap();
            assert_eq!(state.phase, Phase::Review);
            assert!(state.due >= next_day_start(at) && state.due < at + 30 * 86400, "{state:?}");
            assert!(state.stability > 0. && state.last_review.is_some());
            assert_eq!(state.note, "A.md");
        }
        assert_eq!(counts("", at).new, 0);
        assert_eq!(stats("", at).new, 0);
        let told = take_notice().unwrap();
        assert_eq!((told.count, told.spread), (4, 4));
        assert!(take_notice().is_none());
        flush();
        let saved: StoreFile = serde_json::from_slice(&std::fs::read(root.join(STORE_NAME)).unwrap()).unwrap();
        assert!(saved.baseline.is_some());
        let before = state_of("Eins?");
        std::fs::write(root.join("B.md"), "Vier? :: 4\n").unwrap();
        scan(&root);
        assert_eq!(phase_of("Vier?"), Phase::New);
        reopen(&root);
        assert_eq!(phase_of("Vier?"), Phase::New);
        assert_eq!(state_of("Eins?"), before);
        assert!(take_notice().is_none());
        close();
    }

    #[test]
    fn an_empty_vault_remembers_its_first_open_too() {
        let _guard = lock();
        let root = vault("empty", &[("A.md", "Nur Text\n")]);
        scan(&root);
        assert!(take_notice().is_none());
        flush();
        assert!(root.join(STORE_NAME).exists());
        std::fs::write(root.join("B.md"), "Neu? :: ja\n").unwrap();
        reopen(&root);
        assert_eq!(phase_of("Neu?"), Phase::New);
        close();
    }

    #[test]
    fn stores_from_older_versions_change_nothing() {
        let _guard = lock();
        let root = vault("older", &[("A.md", "Alt? :: 1\nNeu? :: 2\n")]);
        let old = parse::cards_in("Alt? :: 1")[0].id.clone();
        let kept = CardState { phase: Phase::Review, stability: 9., difficulty: 4., due: 1, last_review: Some(0), reps: 3, note: "A.md".into(), question: "Alt?".into(), ..Default::default() };
        let file = format!("{{\"version\":0,\"cards\":{{\"{old}\":{}}}}}", serde_json::to_string(&kept).unwrap());
        std::fs::write(root.join(STORE_NAME), &file).unwrap();
        scan(&root);
        assert_eq!(state_of("Alt?"), Some(kept));
        assert_eq!(phase_of("Neu?"), Phase::New);
        assert!(take_notice().is_none());
        close();
        let only_log = vault("only-log", &[("A.md", "F? :: A\n")]);
        std::fs::write(only_log.join(LOG_NAME), "").unwrap();
        scan(&only_log);
        assert_eq!(phase_of("F?"), Phase::New, "a vault with reviews was used before");
        close();
        let synced = vault("synced", &[("A.md", "F? :: A\n")]);
        std::fs::write(synced.join(".schrumbi-cards (Konflikt).json"), "{}").unwrap();
        scan(&synced);
        assert_eq!(phase_of("F?"), Phase::New, "a copy from a sync counts as used");
        close();
    }

    #[test]
    fn plugin_progress_is_taken_over_without_touching_the_note() {
        let _guard = lock();
        let text = "Plugin? :: ja <!--SR:!2030-01-15,40,290-->\nOhne? :: nein\n";
        let root = vault("plugin", &[("A.md", text)]);
        scan(&root);
        let state = state_of("Plugin?").unwrap();
        assert_eq!(state.due, day_start(NaiveDate::from_ymd_opt(2030, 1, 15).unwrap()));
        assert_eq!(state.stability, 40.);
        assert!(state.difficulty < 5.);
        assert_eq!(state.last_review, Some(state.due - 40 * 86400));
        assert_eq!(take_notice().map(|n| (n.count, n.spread)), Some((2, 1)));
        std::fs::write(root.join("B.md"), "Später? :: x <!--SR:!2030-02-01,5,250-->\nLeer? :: y\n").unwrap();
        scan(&root);
        assert_eq!(state_of("Später?").map(|s| s.stability), Some(5.));
        assert_eq!(phase_of("Leer?"), Phase::New);
        let later = all_cards().iter().find(|c| c.raw.front == "Später?").cloned().unwrap();
        reset(&later);
        scan(&root);
        assert_eq!(phase_of("Später?"), Phase::New, "starting over is not undone by the comment");
        assert_eq!(std::fs::read_to_string(root.join("A.md")).unwrap(), text);
        close();
    }

    #[test]
    fn progress_under_an_id_that_held_the_comment_moves_over() {
        let _guard = lock();
        let text = "Begriff ::: Erklärung <!--SR:!2030-01-15,40,290!2030-01-20,50,250-->\n";
        let root = vault("legacy", &[("A.md", text)]);
        let reverse = parse::cards_in(text)[1].clone();
        let old = reverse.legacy.clone().unwrap();
        let mine = CardState { phase: Phase::Review, stability: 77., difficulty: 3., due: 5, last_review: Some(4), reps: 6, note: "A.md".into(), question: "Erklärung <!--SR:!2030-01-15,40,290!2030-01-20,50,250-->".into(), ..Default::default() };
        std::fs::write(root.join(STORE_NAME), format!("{{\"version\":0,\"cards\":{{\"{old}\":{}}}}}", serde_json::to_string(&mine).unwrap())).unwrap();
        let line = |id: &str| format!("{{\"card\":\"{id}\",\"at\":4,\"rating\":\"good\",\"phase\":\"review\",\"days\":1.0}}\n");
        std::fs::write(root.join(LOG_NAME), line(&old)).unwrap();
        scan(&root);
        let moved = state_of("Erklärung").unwrap();
        assert_eq!((moved.stability, moved.reps, moved.question.as_str()), (77., 6, "Erklärung"));
        assert_eq!(state_of("Begriff").map(|s| s.stability), Some(40.), "the other direction takes the plugin's progress");
        assert!(!STORE.read().unwrap().file.cards.contains_key(&old));
        assert_eq!(read_log(&root.join(LOG_NAME))[0].card, reverse.id);
        close();
    }

    #[test]
    fn a_reworded_card_keeps_what_the_first_open_gave_it() {
        let _guard = lock();
        let root = vault("reword", &[("A.md", "Wie heißt die Hauptstadt? :: Berlin\n")]);
        scan(&root);
        let before = state_of("Wie heißt die Hauptstadt?").unwrap();
        std::fs::write(root.join("A.md"), "Wie heißt die Hauptstadt hier? :: Berlin\n").unwrap();
        scan(&root);
        let after = state_of("Wie heißt die Hauptstadt hier?").unwrap();
        assert_eq!((after.phase, after.due, after.stability), (before.phase, before.due, before.stability));
        close();
    }

    #[test]
    fn new_cards_taken_as_known_come_back_on_undo() {
        let _guard = lock();
        let root = vault("manual", &[("A.md", "Text\n")]);
        scan(&root);
        std::fs::write(root.join("A.md"), "Eins? :: 1\nZwei? :: 2\n").unwrap();
        std::fs::create_dir_all(root.join("Sub")).unwrap();
        std::fs::write(root.join("Sub/B.md"), "Drei? :: 3\n").unwrap();
        scan(&root);
        assert_eq!(fresh_count(""), 3);
        assert_eq!(fresh_count("Sub"), 1);
        let paused = all_cards().iter().find(|c| c.raw.front == "Zwei?").cloned().unwrap();
        set_suspended(&paused, true);
        let at = now();
        let done = adopt_new_as_known("", at).unwrap();
        assert_eq!((done.count, done.spread), (2, 2));
        assert_eq!(phase_of("Eins?"), Phase::Review);
        assert_eq!(phase_of("Drei?"), Phase::Review);
        assert_eq!(phase_of("Zwei?"), Phase::New, "paused cards stay as they are");
        assert_eq!(counts("", at).new, 0);
        assert!(adopt_new_as_known("", at).is_none());
        restore(done.before);
        assert_eq!(phase_of("Eins?"), Phase::New);
        assert_eq!(state_of("Eins?"), None);
        assert_eq!(fresh_count(""), 2);
        close();
    }

    #[test]
    fn imported_cards_never_reviewed_are_spread() {
        let _guard = lock();
        let root = vault("import", &[("A.md", "Text\n")]);
        scan(&root);
        let reviewed = CardState { phase: Phase::Review, stability: 12., due: 9, last_review: Some(1), reps: 2, ..Default::default() };
        let fresh = vec![("n1".to_string(), "Anki/D.md".to_string(), "F1".to_string()), ("n2".to_string(), "Anki/D.md".to_string(), "F2".to_string())];
        let done = adopt_import(vec![("r".to_string(), reviewed.clone())], Vec::new(), fresh, now()).unwrap();
        assert_eq!((done.count, done.spread), (2, 2));
        let store = STORE.read().unwrap();
        assert_eq!(store.file.cards["r"], reviewed);
        assert_eq!(store.file.cards["n1"].phase, Phase::Review);
        assert_eq!(store.file.cards["n2"].note, "Anki/D.md");
        drop(store);
        assert!(adopt_import(Vec::new(), Vec::new(), Vec::new(), now()).is_none());
        close();
    }
}

#[cfg(test)]
mod spreading {
    use super::*;

    fn items(n: usize) -> Vec<(String, String, String)> {
        (0..n).map(|i| (format!("{i:04}"), "n.md".to_string(), format!("F{i}"))).collect()
    }

    #[test]
    fn few_cards_spread_over_two_weeks_many_over_four() {
        let at = now();
        let today = day_of(at);
        let day = |s: &CardState| (day_of(s.due) - today).num_days();
        let (few, last) = spread(&items(5), &BTreeMap::new(), at, 200, 0.9);
        assert_eq!(few.len(), 5);
        assert!(few.iter().all(|(_, s)| (1..=14).contains(&day(s))));
        assert!(last <= 14);
        let (many, last) = spread(&items(500), &BTreeMap::new(), at, 200, 0.9);
        assert_eq!(last, 28);
        assert!(many.iter().all(|(_, s)| (1..=28).contains(&day(s)) && s.phase == Phase::Review));
        let mut per_day: HashMap<i64, usize> = HashMap::new();
        for (_, s) in &many {
            *per_day.entry(day(s)).or_default() += 1;
        }
        assert!(per_day.values().all(|n| *n <= 19), "{per_day:?}");
        assert_eq!(spread(&items(500), &BTreeMap::new(), at, 200, 0.9).0, many, "the same cards land on the same days");
        let (one, last) = spread(&items(1), &BTreeMap::new(), at, 200, 0.9);
        assert_eq!((day(&one[0].1), last), (1, 1));
    }

    #[test]
    fn no_day_gets_more_than_the_review_limit() {
        let at = now();
        let today = day_of(at);
        let busy: BTreeMap<String, CardState> = (0..10).map(|i| (format!("old{i}"), CardState { phase: Phase::Review, due: day_start(today + Duration::days(1)), ..Default::default() })).collect();
        let (placed, last) = spread(&items(100), &busy, at, 10, 0.9);
        let mut per_day: HashMap<i64, usize> = HashMap::new();
        for (_, s) in &placed {
            *per_day.entry((day_of(s.due) - today).num_days()).or_default() += 1;
        }
        assert_eq!(per_day.get(&1), None, "tomorrow is already full");
        assert!(per_day.values().all(|n| *n <= 10));
        assert!(last > 10);
        assert_eq!(placed.len(), 100);
    }

    #[test]
    fn the_memory_matches_the_day_it_comes_back() {
        let at = now();
        let (placed, _) = spread(&items(28), &BTreeMap::new(), at, 200, 0.9);
        for (_, s) in &placed {
            let days = (day_of(s.due) - day_of(at)).num_days() as f64;
            assert!((fsrs::interval(s.stability, 0.9) - days).abs() < 1e-6);
            assert_eq!(s.last_review, Some(at));
            assert_eq!(s.reps, 0);
        }
        let (strict, _) = spread(&items(1), &BTreeMap::new(), at, 200, 0.95);
        assert!(strict[0].1.stability > 1.);
    }

    #[test]
    fn plugin_ease_and_interval_read_as_difficulty_and_stability() {
        let sr = |interval, ease| SrProgress { due: NaiveDate::from_ymd_opt(2030, 3, 1).unwrap(), interval, ease };
        let easy = sr_state(&sr(10, 350), "n.md", "F");
        let hard = sr_state(&sr(10, 130), "n.md", "F");
        let normal = sr_state(&sr(10, 250), "n.md", "F");
        assert_eq!((easy.difficulty, hard.difficulty), (1., 10.));
        assert!(normal.difficulty > 4. && normal.difficulty < 6.);
        assert_eq!(sr_state(&sr(0, 250), "n.md", "F").stability, 1.);
        assert_eq!(sr_state(&sr(90000, 250), "n.md", "F").stability, 36500.);
        assert_eq!((normal.phase, normal.reps, normal.note.as_str(), normal.question.as_str()), (Phase::Review, 1, "n.md", "F"));
    }

    #[test]
    fn the_message_says_how_far_cards_were_spread() {
        let words = |days| Adopted { count: 3, spread: 3, days, before: Vec::new() }.spread_words();
        assert_eq!(words(1), "fällig ab morgen");
        assert_eq!(words(9), "verteilt auf die nächsten 9 Tage");
        assert_eq!(words(28), "verteilt auf die nächsten 4 Wochen");
        assert_eq!(words(14), "verteilt auf die nächsten 2 Wochen");
        assert_eq!(Adopted { count: 1, spread: 0, days: 0, before: Vec::new() }.message(), "1 Karte übernommen");
        assert_eq!(Adopted { count: 30, spread: 20, days: 28, before: Vec::new() }.message(), "30 Karten übernommen, verteilt auf die nächsten 4 Wochen");
    }

    #[test]
    fn merging_keeps_the_mark_of_the_first_open() {
        let a = r#"{"version":0,"cards":{}}"#;
        let b = r#"{"version":0,"cards":{},"baseline":42}"#;
        let merged: StoreFile = serde_json::from_str(&merge_store(a, b).unwrap()).unwrap();
        assert_eq!(merged.baseline, Some(42));
        let none: StoreFile = serde_json::from_str(&merge_store(a, a).unwrap()).unwrap();
        assert_eq!(none.baseline, None);
        assert!(!merge_store(a, a).unwrap().contains("baseline"));
    }

    #[test]
    fn thirty_new_cards_a_day_unless_the_settings_say_otherwise() {
        assert_eq!(prefs::Prefs::default().card_new_per_day, 30);
        let stored: prefs::Prefs = serde_json::from_str(r#"{"cardNewPerDay":20}"#).unwrap();
        assert_eq!(stored.card_new_per_day, 20);
        let missing: prefs::Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.card_new_per_day, 30);
    }
}

#[cfg(test)]
mod speed {
    use super::*;

    /// Run with `cargo test --release speed -- --ignored --nocapture`: prints how long the page's work takes.
    #[test]
    #[ignore]
    fn with_many_cards() {
        for total in [1_000usize, 10_000, 100_000] {
            let dir = std::env::temp_dir().join(format!("schrumbi-cards-speed-{total}"));
            let _ = std::fs::remove_dir_all(&dir);
            for note in 0..(total / 100) {
                let folder = dir.join(format!("Modul{}", note % 20));
                std::fs::create_dir_all(&folder).unwrap();
                let text: String = (0..100).map(|i| format!("Frage {note}-{i}? :: Antwort {i} mit $x^2$\n\n")).collect();
                std::fs::write(folder.join(format!("Notiz{note}.md")), text).unwrap();
            }
            let clock = std::time::Instant::now();
            index::rebuild(&dir, &[]);
            open(&dir);
            let rebuild = clock.elapsed();
            assert_eq!(all_cards().len(), total);
            let at = now();
            let took = |label: &str, f: &mut dyn FnMut()| {
                let clock = std::time::Instant::now();
                f();
                eprintln!("{total:>7} {label:<22} {:>9.2} ms", clock.elapsed().as_secs_f64() * 1000.);
            };
            eprintln!("{total:>7} {:<22} {:>9.2} ms", "index", rebuild.as_secs_f64() * 1000.);
            took("all_cards", &mut || drop(all_cards()));
            took("queue", &mut || drop(queue("", at)));
            took("counts", &mut || drop(counts("", at)));
            took("counts one deck", &mut || drop(counts("Modul3", at)));
            took("stats", &mut || drop(stats("", at)));
            let card = queue("", at).remove(0);
            took("answer", &mut || drop(answer(&card, Rating::Good, at)));
            took("all_cards after answer", &mut || drop(all_cards()));
            took("queue after answer", &mut || drop(queue("", at)));
            close();
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
