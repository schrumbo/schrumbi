//! Keeps the calendar „Schrumbi“ in Apple Kalender in step with the current semester, so it reaches iCloud and the iPhone. Runs on its own thread and waits for bursts of changes to settle.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use gpui_kit::App;

use super::state;
use crate::core::calendar_sync::{self, Deadline, Entry, Links};
use crate::core::model::Status;

pub const AVAILABLE: bool = cfg!(target_os = "macos");
const SETTLE: Duration = Duration::from_millis(800);
const LONGEST_WAIT: Duration = Duration::from_secs(5);

/// Where the calendar stands, for settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Off,
    /// Outside an app bundle, where macOS cannot ask for access.
    Unavailable,
    Asking,
    Denied,
    Restricted,
    Working,
    Synced(usize),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum Access {
    Unavailable,
    Undetermined,
    Denied,
    Restricted,
    Full,
}

enum Message {
    Want(BTreeMap<String, Entry>),
    Off,
    Recheck,
}

static STATE: Mutex<State> = Mutex::new(State::Off);
static SENT: Mutex<Option<BTreeMap<String, Entry>>> = Mutex::new(None);
static WORKER: OnceLock<mpsc::Sender<Message>> = OnceLock::new();
static CHANGED: OnceLock<async_channel::Sender<()>> = OnceLock::new();
static LEFT_BEHIND: AtomicBool = AtomicBool::new(false);

/// Redraws settings whenever the calendar's state changes.
pub fn init(cx: &mut App) {
    if !AVAILABLE {
        return;
    }
    let (tx, rx) = async_channel::unbounded::<()>();
    let _ = CHANGED.set(tx);
    if !crate::core::prefs::get().apple_calendar && !calendar_sync::load().events.is_empty() {
        send(Message::Off);
    }
    cx.spawn(async move |cx| {
        while rx.recv().await.is_ok() {
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
}

pub fn state() -> State {
    STATE.lock().unwrap().clone()
}

fn set(next: State) {
    let changed = {
        let mut s = STATE.lock().unwrap();
        std::mem::replace(&mut *s, next.clone()) != next
    };
    if changed && let Some(tx) = CHANGED.get() {
        let _ = tx.try_send(());
    }
}

fn send(message: Message) {
    let tx = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new().name("apple-calendar".into()).spawn(move || work(rx)).ok();
        tx
    });
    let _ = tx.send(message);
}

/// Hands the current semester and every deadline still ahead to the calendar when they changed. Does nothing while the workspace is not loaded, so a missing configuration never empties the calendar.
pub fn push(cx: &App) {
    if !AVAILABLE {
        return;
    }
    let prefs = crate::core::prefs::get();
    if !prefs.apple_calendar {
        return;
    }
    let wanted = state::read(cx, |l| {
        let (Some(config), Some(_), false) = (l.config.as_ref(), l.root.as_ref(), l.needs_setup) else { return None };
        let Some(semester) = config.current_semester(l.now) else { return Some(BTreeMap::new()) };
        let deadlines: Vec<Deadline> = l
            .assignments
            .iter()
            .filter(|a| a.semester_id == semester.id || a.due > l.now)
            .map(|a| (a, l.status(a)))
            .filter(|(_, s)| *s != Status::Skipped)
            .map(|(assignment, s)| Deadline { assignment, done: s.is_closed() })
            .collect();
        Some(calendar_sync::wanted(&deadlines, config, semester, prefs.reminders_enabled.then_some(prefs.reminder_hours)))
    });
    let Some(wanted) = wanted else {
        *SENT.lock().unwrap() = None;
        set(State::Off);
        return;
    };
    {
        let mut sent = SENT.lock().unwrap();
        if sent.as_ref() == Some(&wanted) {
            return;
        }
        *sent = Some(wanted.clone());
    }
    if state() == State::Off {
        set(State::Working);
    }
    send(Message::Want(wanted));
}

/// Takes everything Schrumbi made out of the calendar again; what the user made stays.
pub fn turn_off() {
    if !AVAILABLE {
        return;
    }
    *SENT.lock().unwrap() = None;
    send(Message::Off);
}

/// Looks again whether access changed, e.g. after the user came back from System Settings, and tries again after a failure or a turn-off that could not remove everything.
pub fn recheck() {
    if !AVAILABLE {
        return;
    }
    let on = crate::core::prefs::get().apple_calendar;
    if (on && !matches!(state(), State::Synced(_) | State::Working)) || (!on && LEFT_BEHIND.load(Ordering::SeqCst)) {
        send(Message::Recheck);
    }
}

pub fn open_privacy_settings() {
    let _ = opener::open("x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars");
}

pub fn open_calendar_app() {
    let _ = opener::open("/System/Applications/Calendar.app");
}

fn work(rx: mpsc::Receiver<Message>) {
    let (mut wanted, mut on, mut asked, mut swept) = (None::<BTreeMap<String, Entry>>, false, false, false);
    let mut links = calendar_sync::load();
    while let Ok(first) = rx.recv() {
        let (mut next, until) = (Some(first), Instant::now() + LONGEST_WAIT);
        while let Some(message) = next.take() {
            match message {
                Message::Want(w) => (wanted, on) = (Some(w), true),
                Message::Off => on = false,
                Message::Recheck => {}
            }
            next = rx.recv_timeout(SETTLE.min(until.saturating_duration_since(Instant::now()))).ok();
        }
        let target = if on { wanted.clone() } else { Some(BTreeMap::new()) };
        let Some(target) = target else { continue };
        if !on && links.events.is_empty() && swept {
            set(State::Off);
            continue;
        }
        let access = platform::access();
        let blocked = |s: State| if on { s } else { State::Off };
        match access {
            Access::Unavailable => set(blocked(State::Unavailable)),
            Access::Undetermined if on && !asked => {
                asked = true;
                set(State::Asking);
                platform::request(|| {
                    if let Some(tx) = WORKER.get() {
                        let _ = tx.send(Message::Recheck);
                    }
                });
            }
            Access::Undetermined => set(blocked(State::Asking)),
            Access::Denied => set(blocked(State::Denied)),
            Access::Restricted => set(blocked(State::Restricted)),
            Access::Full => {
                if on && !matches!(state(), State::Synced(_)) {
                    set(State::Working);
                }
                set(match sync(&target, &mut links) {
                    Ok(()) if on => State::Synced(links.events.len()),
                    Ok(()) => State::Off,
                    Err(e) => State::Failed(e),
                });
            }
        }
        swept = !on && matches!(access, Access::Full) && state() == State::Off;
        LEFT_BEHIND.store(!on && !links.events.is_empty(), Ordering::SeqCst);
    }
}

fn sync(target: &BTreeMap<String, Entry>, links: &mut Links) -> Result<(), String> {
    let result = platform::apply(target, links);
    let saved = calendar_sync::save(links).map_err(|e| e.to_string());
    result.and(saved)
}

#[cfg(target_os = "macos")]
mod platform {
    use block2::RcBlock;
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::runtime::{Bool, NSObjectProtocol};
    use objc2::sel;
    use objc2_event_kit::{EKAlarm, EKAuthorizationStatus, EKCalendar, EKEntityType, EKEvent, EKEventStore, EKSource, EKSourceType, EKSpan};
    use objc2_foundation::{NSArray, NSBundle, NSDate, NSError, NSString, NSURL, ns_string};

    use std::collections::BTreeMap;

    use super::Access;
    use crate::core::calendar_sync::{self, Calendar, Entry, Link, Links};
    use crate::core::i18n::tr;

    const TITLE: &str = "Schrumbi";
    const SPAN: f64 = 700. * 86400.;

    fn bundled() -> bool {
        let bundle = NSBundle::mainBundle();
        bundle.bundleIdentifier().is_some() && (bundle.objectForInfoDictionaryKey(ns_string!("NSCalendarsFullAccessUsageDescription")).is_some() || bundle.objectForInfoDictionaryKey(ns_string!("NSCalendarsUsageDescription")).is_some())
    }

    pub fn access() -> Access {
        autoreleasepool(|_| {
            if !bundled() {
                return Access::Unavailable;
            }
            match unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) } {
                EKAuthorizationStatus::FullAccess => Access::Full,
                EKAuthorizationStatus::NotDetermined => Access::Undetermined,
                EKAuthorizationStatus::Restricted => Access::Restricted,
                _ => Access::Denied,
            }
        })
    }

    /// Asks once; macOS shows its prompt and calls back on its own thread.
    pub fn request(done: impl Fn() + 'static) {
        autoreleasepool(|_| ask(done));
    }

    fn ask(done: impl Fn() + 'static) {
        let store = unsafe { EKEventStore::new() };
        let keep = store.clone();
        let block = RcBlock::new(move |_: Bool, _: *mut NSError| {
            let _ = &keep;
            done();
        });
        unsafe {
            if store.respondsToSelector(sel!(requestFullAccessToEventsWithCompletion:)) {
                store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&block));
            } else {
                #[allow(deprecated)]
                store.requestAccessToEntityType_completion(EKEntityType::Event, RcBlock::as_ptr(&block));
            }
        }
    }

    fn text(s: &str) -> Retained<NSString> {
        NSString::from_str(s)
    }

    fn error(e: Retained<NSError>) -> String {
        e.localizedDescription().to_string()
    }

    struct Store {
        store: Retained<EKEventStore>,
        calendar: Retained<EKCalendar>,
        staged: Vec<Retained<EKEvent>>,
    }

    impl Store {
        fn ours(&self, event: &EKEvent) -> bool {
            unsafe { event.calendar() }.is_some_and(|c| unsafe { c.calendarIdentifier() } == unsafe { self.calendar.calendarIdentifier() })
        }

        fn event(&self, id: &str) -> Option<Retained<EKEvent>> {
            unsafe { self.store.eventWithIdentifier(&text(id)) }.filter(|e| self.ours(e))
        }
    }

    impl Calendar for Store {
        fn find(&mut self, link: &Link) -> Option<String> {
            if !link.id.is_empty() && self.event(&link.id).is_some() {
                return Some(link.id.clone());
            }
            if link.external.is_empty() {
                return None;
            }
            let items = unsafe { self.store.calendarItemsWithExternalIdentifier(&text(&link.external)) };
            items.iter().filter_map(|i| i.downcast::<EKEvent>().ok()).find(|e| self.ours(e)).and_then(|e| unsafe { e.eventIdentifier() }).map(|s| s.to_string())
        }

        fn marked(&mut self) -> Vec<(String, String, Entry)> {
            let calendars = NSArray::from_retained_slice(std::slice::from_ref(&self.calendar));
            let (from, to) = (NSDate::dateWithTimeIntervalSinceNow(-SPAN), NSDate::dateWithTimeIntervalSinceNow(SPAN));
            let events = unsafe { self.store.eventsMatchingPredicate(&self.store.predicateForEventsWithStartDate_endDate_calendars(&from, &to, Some(&calendars))) };
            events
                .iter()
                .filter_map(|e| {
                    let key = calendar_sync::marked_key(&unsafe { e.URL() }?.absoluteString()?.to_string())?;
                    let seen = unsafe { Entry { title: e.title().to_string(), start: e.startDate().timeIntervalSince1970() as i64, end: e.endDate().timeIntervalSince1970() as i64, ..Default::default() } };
                    Some((key, unsafe { e.eventIdentifier() }?.to_string(), seen))
                })
                .collect()
        }

        fn save(&mut self, id: Option<&str>, key: &str, entry: &Entry) -> Result<(), String> {
            let event = match id {
                Some(id) => self.event(id).ok_or_else(|| tr("Termin nicht mehr im Kalender"))?,
                None => {
                    let e = unsafe { EKEvent::eventWithEventStore(&self.store) };
                    unsafe { e.setCalendar(Some(&self.calendar)) };
                    e
                }
            };
            let optional = |s: &str| (!s.is_empty()).then(|| text(s));
            unsafe {
                event.setTitle(Some(&text(&entry.title)));
                event.setStartDate(Some(&NSDate::dateWithTimeIntervalSince1970(entry.start as f64)));
                event.setEndDate(Some(&NSDate::dateWithTimeIntervalSince1970(entry.end as f64)));
                event.setLocation(optional(&entry.location).as_deref());
                event.setNotes(optional(&entry.notes).as_deref());
                let alarms = entry.alarm.map(|m| NSArray::from_retained_slice(&[EKAlarm::alarmWithRelativeOffset(-(m as f64) * 60.)]));
                event.setAlarms(alarms.as_deref());
                event.setURL(NSURL::URLWithString(&text(&calendar_sync::mark(key))).as_deref());
                self.store.saveEvent_span_commit_error(&event, EKSpan::ThisEvent, false).map_err(error)?;
            }
            self.staged.push(event);
            Ok(())
        }

        fn remove(&mut self, id: &str) -> Result<(), String> {
            let Some(event) = self.event(id) else { return Ok(()) };
            unsafe { self.store.removeEvent_span_commit_error(&event, EKSpan::ThisEvent, false) }.map_err(error)
        }

        fn commit(&mut self) -> Result<Vec<(String, String)>, String> {
            if let Err(e) = unsafe { self.store.commit() } {
                unsafe { self.store.reset() };
                self.staged.clear();
                return Err(error(e));
            }
            Ok(self
                .staged
                .drain(..)
                .map(|e| unsafe { (e.eventIdentifier().map_or(String::new(), |s| s.to_string()), e.calendarItemExternalIdentifier().map_or(String::new(), |s| s.to_string())) })
                .collect())
        }
    }

    fn writable(c: &EKCalendar) -> bool {
        unsafe { c.allowsContentModifications() && !c.isSubscribed() }
    }

    /// The calendar Schrumbi used before, else one called „Schrumbi“, else with `create` a new one in iCloud when there is an iCloud account. While the account of the known calendar is missing, e.g. signed out, nothing is touched so its events are not lost track of.
    fn calendar(store: &EKEventStore, links: &Links, create: bool) -> Result<Option<Retained<EKCalendar>>, String> {
        if let Some(c) = (!links.calendar.is_empty()).then(|| unsafe { store.calendarWithIdentifier(&text(&links.calendar)) }).flatten() {
            return match (writable(&c), create) {
                (true, _) => Ok(Some(c)),
                (false, true) => Err(tr("Der Kalender „Schrumbi“ ist schreibgeschützt. Schalte den Abgleich aus und wieder ein, um ihn neu anzulegen.")),
                (false, false) => Ok(None),
            };
        }
        let away = !links.events.is_empty() && !links.source.is_empty() && unsafe { store.sourceWithIdentifier(&text(&links.source)) }.is_none();
        if away && create {
            return Err(tr("Das Konto mit dem Kalender „Schrumbi“ fehlt auf diesem Mac. Schalte den Abgleich aus und wieder ein, um ihn neu anzulegen."));
        }
        let calendars = unsafe { store.calendarsForEntityType(EKEntityType::Event) };
        if let Some(c) = calendars.iter().find(|c| unsafe { c.title() }.to_string() == TITLE && writable(c)) {
            return Ok(Some(c));
        }
        if !create {
            return Ok(None);
        }
        let sources: Vec<Retained<EKSource>> = unsafe { store.sources() }.iter().collect();
        let kind = |s: &EKSource| unsafe { s.sourceType() };
        let icloud = sources.iter().filter(|s| kind(s) == EKSourceType::CalDAV && unsafe { s.title() }.to_string() == "iCloud").cloned();
        let default = unsafe { store.defaultCalendarForNewEvents() }.and_then(|c| unsafe { c.source() });
        let local = sources.iter().filter(|s| kind(s) == EKSourceType::Local).cloned();
        for source in icloud.chain(default).chain(local) {
            let c = unsafe { EKCalendar::calendarForEntityType_eventStore(EKEntityType::Event, store) };
            unsafe {
                c.setTitle(&text(TITLE));
                c.setSource(Some(&source));
            }
            if unsafe { store.saveCalendar_commit_error(&c, true) }.is_ok() {
                return Ok(Some(c));
            }
        }
        Err(tr("Der Kalender „Schrumbi“ konnte nicht angelegt werden."))
    }

    pub fn apply(target: &BTreeMap<String, Entry>, links: &mut Links) -> Result<(), String> {
        autoreleasepool(|_| {
            let store = unsafe { EKEventStore::new() };
            let Some(calendar) = calendar(&store, links, !target.is_empty())? else {
                links.events.clear();
                return Ok(());
            };
            links.calendar = unsafe { calendar.calendarIdentifier() }.to_string();
            links.source = unsafe { calendar.source() }.map_or(String::new(), |s| unsafe { s.sourceIdentifier() }.to_string());
            calendar_sync::apply(&mut Store { store, calendar, staged: Vec::new() }, target, links)
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use std::collections::BTreeMap;

    use super::Access;
    use crate::core::calendar_sync::{Entry, Links};

    pub fn access() -> Access {
        Access::Unavailable
    }

    pub fn request(_: impl Fn() + 'static) {}

    pub fn apply(_: &BTreeMap<String, Entry>, _: &mut Links) -> Result<(), String> {
        Ok(())
    }
}
