use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use super::i18n::tr;
use super::library::unique_path;
use super::naming::{extension, stem};

/// The text of a note at one version, shared without copying, with what is worked out from it once.
#[derive(Debug)]
pub struct Snapshot {
    pub text: Arc<str>,
    words: OnceLock<usize>,
    fingerprint: OnceLock<u64>,
}

impl Snapshot {
    pub fn words(&self) -> usize {
        *self.words.get_or_init(|| self.text.split_whitespace().count())
    }

    pub fn fingerprint(&self) -> u64 {
        *self.fingerprint.get_or_init(|| super::find::fingerprint(&self.text))
    }
}

/// An open note. The editor owns the live text while typing and reports edits here;
/// `text` is the snapshot used by the reader, export and saving.
#[derive(Debug)]
pub struct NoteBuffer {
    pub path: PathBuf,
    pub text: String,
    pub dirty: bool,
    pub missing: bool,
    /// The file is not UTF-8. It shows as unavailable and is never written, so it stays intact.
    pub undecodable: bool,
    last_saved: String,
    known: Option<SystemTime>,
    /// Bumped whenever the text changes from outside the editor, so editors reload it.
    pub revision: u64,
    /// The version on disk when it changed while there were unsaved edits here; saving waits for a decision.
    pub conflict: Option<String>,
    /// Saves are only remembered, because a sync is rewriting the workspace.
    held: bool,
    save_wanted: bool,
    /// Written in since it was opened, also when that is saved already.
    pub touched: bool,
    version: u64,
    snapshot: Mutex<Option<(u64, Arc<Snapshot>)>>,
    inflight: Option<Inflight>,
    save_again: bool,
}

/// A write running on its own thread, and the text it writes.
#[derive(Debug)]
struct Inflight {
    handle: std::thread::JoinHandle<SaveOutcome>,
    text: Arc<str>,
}

#[derive(Debug)]
enum SaveOutcome {
    Written,
    Recheck,
    Failed(std::io::Error),
}

/// What `NoteBuffer::save_in_background` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStart {
    /// Nothing needed writing, or the save was handled on the spot.
    Done,
    Started,
    /// A write is still running; the text is written once it is done.
    Busy,
}

impl Drop for NoteBuffer {
    fn drop(&mut self) {
        let _ = self.drain();
    }
}

impl NoteBuffer {
    pub fn open(path: &Path) -> Self {
        let mut b = Self { path: path.to_path_buf(), text: String::new(), dirty: false, missing: false, undecodable: false, last_saved: String::new(), known: None, revision: 0, conflict: None, held: false, save_wanted: false, touched: false, version: 0, snapshot: Mutex::new(None), inflight: None, save_again: false };
        b.load();
        b
    }

    /// Whether a write is running in the background.
    pub fn saving(&self) -> bool {
        self.inflight.is_some()
    }

    /// Whether another save was asked for while one was running, once.
    pub fn take_save_again(&mut self) -> bool {
        std::mem::take(&mut self.save_again)
    }

    /// Takes in the result of the background write if it is done: `None` while it runs or when there is none.
    pub fn poll_save(&mut self) -> Option<std::io::Result<()>> {
        if self.inflight.as_ref().is_some_and(|f| f.handle.is_finished()) { Some(self.drain()) } else { None }
    }

    /// Waits for a background write and takes in its result.
    fn drain(&mut self) -> std::io::Result<()> {
        let Some(job) = self.inflight.take() else { return Ok(()) };
        let outcome = job.handle.join().unwrap_or_else(|_| SaveOutcome::Failed(std::io::Error::other("save thread stopped")));
        match outcome {
            SaveOutcome::Written => {
                self.missing = false;
                self.last_saved = job.text.to_string();
                self.known = self.modified();
                self.dirty = self.text != self.last_saved;
                Ok(())
            }
            SaveOutcome::Recheck => self.save(),
            SaveOutcome::Failed(e) => Err(e),
        }
    }

    /// Like `save`, but the file is written on another thread, so a large note does not stall the window.
    /// Writes never overlap and every other way of saving waits for a running one, so the last text is the one that stays.
    pub fn save_in_background(&mut self) -> std::io::Result<SaveStart> {
        if let Some(done) = self.poll_save() {
            done?;
        }
        if self.inflight.is_some() {
            if self.dirty {
                self.save_again = true;
            }
            return Ok(SaveStart::Busy);
        }
        if self.conflict.is_some() || self.undecodable || self.held || self.text == self.last_saved {
            self.save()?;
            return Ok(SaveStart::Done);
        }
        let (path, known, text) = (self.path.clone(), self.known, self.snapshot().text.clone());
        let job_text = text.clone();
        let handle = std::thread::Builder::new().spawn(move || {
            let unchanged = known.is_some() && std::fs::metadata(&path).and_then(|m| m.modified()).ok() == known;
            if !unchanged {
                return SaveOutcome::Recheck;
            }
            match write_atomic(&path, job_text.as_bytes()) {
                Ok(()) => SaveOutcome::Written,
                Err(e) => SaveOutcome::Failed(e),
            }
        })?;
        self.inflight = Some(Inflight { handle, text });
        Ok(SaveStart::Started)
    }

    /// The current text, shared; copied only when it changed since the last call.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        let mut cached = self.snapshot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((version, snapshot)) = cached.as_ref()
            && *version == self.version
        {
            return snapshot.clone();
        }
        let snapshot = Arc::new(Snapshot { text: Arc::from(self.text.as_str()), words: OnceLock::new(), fingerprint: OnceLock::new() });
        *cached = Some((self.version, snapshot.clone()));
        snapshot
    }

    fn modified(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.path).and_then(|m| m.modified()).ok()
    }

    fn read_disk(&self) -> Option<String> {
        std::fs::read(&self.path).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    pub fn load(&mut self) {
        let _ = self.drain();
        match std::fs::read_to_string(&self.path) {
            Ok(s) => {
                self.missing = false;
                if s != self.text {
                    self.text = s.clone();
                    self.version += 1;
                    self.revision += 1;
                }
                self.last_saved = s;
                self.known = self.modified();
                self.dirty = false;
                self.conflict = None;
            }
            Err(e) => {
                self.missing = !self.path.exists();
                self.undecodable = e.kind() == std::io::ErrorKind::InvalidData;
                return;
            }
        }
        self.undecodable = false;
    }

    /// Follows a rename or move made by the app itself.
    pub fn moved_to(&mut self, path: &Path) {
        let _ = self.drain();
        self.path = path.to_path_buf();
        self.known = self.modified();
    }

    /// Picks up edits made in another app; with unsaved changes here the other version becomes a conflict.
    pub fn reload_if_changed(&mut self) -> bool {
        if self.held {
            return false;
        }
        if self.inflight.is_some() {
            let _ = self.poll_save();
            return false;
        }
        if self.undecodable {
            self.load();
            return !self.undecodable;
        }
        let modified = self.modified();
        if modified.is_none() {
            let gone = !self.dirty && !self.missing;
            self.missing |= gone;
            return gone;
        }
        if self.missing {
            self.load();
            return true;
        }
        if modified == self.known {
            return false;
        }
        self.known = modified;
        let Some(disk) = self.read_disk() else { return false };
        if disk == self.last_saved {
            return self.conflict.take().is_some();
        }
        if !self.dirty {
            self.load();
            return true;
        }
        if disk == self.text {
            self.last_saved = disk;
            self.dirty = false;
            self.conflict = None;
            return true;
        }
        if let Some(merged) = super::live_text::merge(&self.last_saved, &self.text, &disk) {
            self.dirty = merged != disk;
            self.text = merged;
            self.version += 1;
            self.last_saved = disk;
            self.conflict = None;
            self.revision += 1;
            return true;
        }
        let changed = self.conflict.as_deref() != Some(disk.as_str());
        self.conflict = Some(disk);
        changed
    }

    /// The editor's text after an edit.
    pub fn edited(&mut self, text: String) {
        self.dirty = text != self.last_saved;
        let changed = text != self.text;
        self.touched |= changed;
        self.version += u64::from(changed);
        self.text = text;
    }

    /// A deliberate change from outside the editor, e.g. ticking a task in the reader.
    pub fn replace(&mut self, text: String) {
        self.dirty = text != self.last_saved;
        self.touched |= text != self.text;
        self.text = text;
        self.version += 1;
        self.revision += 1;
    }

    /// Writes the note, unless another app changed the file since it was read: that becomes a `conflict`.
    pub fn save(&mut self) -> std::io::Result<()> {
        let _ = self.drain();
        if self.conflict.is_some() || self.undecodable {
            return Ok(());
        }
        if self.held {
            self.save_wanted = true;
            return Ok(());
        }
        if self.text == self.last_saved {
            self.dirty = false;
            return Ok(());
        }
        if let Some(disk) = self.read_disk()
            && disk != self.last_saved
            && disk != self.text
        {
            self.known = self.modified();
            self.conflict = Some(disk);
            return Ok(());
        }
        write_atomic(&self.path, self.text.as_bytes())?;
        self.missing = false;
        self.last_saved = self.text.clone();
        self.known = self.modified();
        self.dirty = false;
        Ok(())
    }

    /// Resolves a conflict in favour of the text here; the next save replaces the other version.
    pub fn keep_mine(&mut self) {
        if let Some(disk) = self.conflict.take() {
            self.last_saved = disk;
            self.known = self.modified();
            self.dirty = self.text != self.last_saved;
        }
    }

    /// Resolves a conflict in favour of the file on disk and hands back the text it replaced.
    pub fn take_disk(&mut self) -> Option<String> {
        self.conflict.take()?;
        let mine = self.text.clone();
        self.dirty = false;
        self.load();
        Some(mine)
    }

    /// Writes the text here to a separate file next to the note, then adopts the version on disk.
    pub fn save_conflict_copy(&mut self) -> std::io::Result<PathBuf> {
        let name = self.path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let date = chrono::Local::now().format("%Y-%m-%d");
        let ext = extension(&name);
        let suffix = if ext.is_empty() { String::new() } else { format!(".{ext}") };
        let target = unique_path(&self.path.with_file_name(format!("{} ({} {date}){suffix}", stem(&name), tr("Konflikt"))));
        std::fs::write(&target, &self.text)?;
        self.take_disk();
        Ok(target)
    }

    /// Takes on `text` as a deliberate step, e.g. from the undo history, and writes it.
    pub fn restore(&mut self, text: String) -> std::io::Result<()> {
        self.keep_mine();
        self.replace(text);
        self.save()
    }

    /// Starts remembering saves instead of writing them.
    pub fn hold(&mut self) {
        self.held = true;
    }

    /// Ends `hold`. With `changed` the file is compared even if its time did not move forward,
    /// since a sync may give it an older time; then remembered saves are written.
    pub fn release(&mut self, changed: bool) -> std::io::Result<bool> {
        self.held = false;
        let mut reloaded = false;
        if changed {
            self.known = None;
            reloaded = self.reload_if_changed();
        }
        if std::mem::take(&mut self.save_wanted) {
            self.save()?;
        }
        Ok(reloaded)
    }

    pub fn name(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }
}

/// Replaces the file in one step, so a crash leaves the old or the new version and never a part of one.
/// Follows links and keeps the file's permissions; a read-only file stays untouched.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let target = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let existing = std::fs::metadata(&target).ok();
    if existing.as_ref().is_some_and(|m| m.permissions().readonly()) {
        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    }
    let tmp = target.with_file_name(format!(".{}.tmp", target.file_name().unwrap_or_default().to_string_lossy()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Some(m) = &existing {
            std::fs::set_permissions(&tmp, m.permissions())?;
        }
        std::fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("schrumbi-note-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("n.md");
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Changes the file like another app and moves its time, whatever the clock resolution.
    fn outside(path: &Path, text: &str, seconds: u64) {
        std::fs::write(path, text).unwrap();
        let time = SystemTime::now() + std::time::Duration::from_secs(seconds);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(time).unwrap();
    }

    #[test]
    fn save_never_overwrites_a_change_from_outside() {
        let path = note("save", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("mine".into());
        outside(&path, "theirs", 5);
        b.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
        assert_eq!(b.conflict.as_deref(), Some("theirs"));
        assert!(b.dirty);
        assert_eq!(b.text, "mine");
    }

    #[test]
    fn clean_buffer_follows_the_file_also_to_an_older_time() {
        let path = note("older", "a");
        let mut b = NoteBuffer::open(&path);
        std::fs::write(&path, "b").unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000)).unwrap();
        assert!(b.reload_if_changed());
        assert_eq!(b.text, "b");
        assert!(b.conflict.is_none());
    }

    #[test]
    fn dirty_buffer_reports_conflict_on_reload() {
        let path = note("dirty", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("mine".into());
        outside(&path, "theirs", 5);
        assert!(b.reload_if_changed());
        assert_eq!(b.text, "mine");
        assert_eq!(b.conflict.as_deref(), Some("theirs"));
        assert!(!b.reload_if_changed());
    }

    #[test]
    fn same_text_on_both_sides_is_no_conflict() {
        let path = note("same", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("x".into());
        outside(&path, "x", 5);
        assert!(b.reload_if_changed());
        assert!(b.conflict.is_none());
        assert!(!b.dirty);
    }

    #[test]
    fn keeping_mine_overwrites_on_the_next_save() {
        let path = note("mine", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("mine".into());
        outside(&path, "theirs", 5);
        b.save().unwrap();
        b.keep_mine();
        b.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
        assert!(b.conflict.is_none());
        assert!(!b.dirty);
    }

    #[test]
    fn taking_disk_hands_back_my_text_for_undo() {
        let path = note("disk", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("mine".into());
        outside(&path, "theirs", 5);
        b.save().unwrap();
        assert_eq!(b.take_disk().as_deref(), Some("mine"));
        assert_eq!(b.text, "theirs");
        assert!(!b.dirty);
        b.replace("mine".into());
        assert!(b.dirty);
    }

    #[test]
    fn conflict_copy_keeps_both_versions() {
        let path = note("copy", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("mine".into());
        outside(&path, "theirs", 5);
        b.save().unwrap();
        let copy = b.save_conflict_copy().unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
        assert_eq!(extension(&copy.file_name().unwrap().to_string_lossy()), "md");
        assert!(b.conflict.is_none());
    }

    fn settle(b: &mut NoteBuffer) {
        while b.saving() {
            if let Some(done) = b.poll_save() {
                done.unwrap();
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn background_saves_in_a_row_keep_the_last_text() {
        let path = note("background", "a");
        let mut b = NoteBuffer::open(&path);
        for round in 0..30 {
            b.edited(format!("version {round}"));
            if b.save_in_background().unwrap() == SaveStart::Busy {
                assert!(b.dirty);
            }
            if round % 3 == 0 {
                settle(&mut b);
            }
        }
        let last = "version 29";
        while b.saving() || b.dirty {
            settle(&mut b);
            if b.take_save_again() || b.dirty {
                b.save_in_background().unwrap();
            }
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), last);
        assert_eq!(b.text, last);
        assert!(!b.dirty);
    }

    #[test]
    fn a_save_while_one_runs_waits_for_it_and_wins() {
        let path = note("overlap", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("first".into());
        assert_eq!(b.save_in_background().unwrap(), SaveStart::Started);
        b.edited("second".into());
        b.save().unwrap();
        assert!(!b.saving());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert!(!b.dirty);
    }

    #[test]
    fn dropping_a_buffer_finishes_its_write() {
        let path = note("drop", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("kept".into());
        assert_eq!(b.save_in_background().unwrap(), SaveStart::Started);
        drop(b);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept");
    }

    #[test]
    fn a_background_save_still_notices_a_change_from_outside() {
        let path = note("background-conflict", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("mine".into());
        outside(&path, "theirs", 5);
        b.save_in_background().unwrap();
        settle(&mut b);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
        assert_eq!(b.conflict.as_deref(), Some("theirs"));
        assert!(b.dirty);
    }

    #[test]
    fn held_saves_wait_and_a_pull_is_noticed() {
        let path = note("held", "a");
        let mut b = NoteBuffer::open(&path);
        b.hold();
        b.edited("typed during sync".into());
        b.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a");
        std::fs::write(&path, "pulled").unwrap();
        assert!(!b.reload_if_changed());
        b.release(true).unwrap();
        assert_eq!(b.conflict.as_deref(), Some("pulled"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "pulled");
    }

    #[test]
    fn held_save_is_written_when_nothing_was_pulled() {
        let path = note("held2", "a");
        let mut b = NoteBuffer::open(&path);
        b.hold();
        b.edited("typed".into());
        b.save().unwrap();
        b.release(false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "typed");
        assert!(!b.dirty);
    }

    #[test]
    fn a_file_deleted_outside_does_not_come_back_by_itself() {
        let path = note("gone", "a");
        let mut b = NoteBuffer::open(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(b.reload_if_changed());
        assert!(b.missing);
        b.save().unwrap();
        assert!(!path.exists());
        std::fs::write(&path, "a").unwrap();
        assert!(b.reload_if_changed());
        assert!(!b.missing);
    }

    #[test]
    fn typing_keeps_a_deleted_file_and_saving_brings_it_back() {
        let path = note("gone2", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("wichtig".into());
        std::fs::remove_file(&path).unwrap();
        assert!(!b.reload_if_changed());
        assert!(!b.missing);
        b.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "wichtig");
    }

    #[test]
    fn conflict_copy_of_a_file_without_extension_has_no_trailing_dot() {
        let path = note("noext", "a").with_file_name("README");
        std::fs::write(&path, "a").unwrap();
        let mut b = NoteBuffer::open(&path);
        b.edited("mine".into());
        outside(&path, "theirs", 5);
        b.save().unwrap();
        let copy = b.save_conflict_copy().unwrap();
        assert!(!copy.file_name().unwrap().to_string_lossy().ends_with('.'), "{copy:?}");
    }

    #[test]
    fn restore_is_a_full_round_trip() {
        let path = note("restore", "a");
        let mut b = NoteBuffer::open(&path);
        b.edited("mine".into());
        outside(&path, "theirs", 5);
        b.save().unwrap();
        let mine = b.take_disk().unwrap();
        b.restore(mine).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
        assert!(!b.dirty);
        b.restore("theirs".into()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
    }

    #[cfg(unix)]
    #[test]
    fn saving_keeps_links_and_permissions_and_respects_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let real = note("link", "a");
        let link = real.with_file_name("verweis.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o640)).unwrap();
        let mut b = NoteBuffer::open(&link);
        b.edited("neu".into());
        b.save().unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "neu");
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o640);
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o440)).unwrap();
        b.edited("gesperrt".into());
        assert!(b.save().is_err());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "neu");
        assert!(b.dirty);
        let names: Vec<_> = std::fs::read_dir(real.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(!names.iter().any(|n| n.to_string_lossy().ends_with(".tmp")), "{names:?}");
    }

    #[test]
    fn saving_keeps_bytes_exact() {
        let text = "\u{feff}a\r\nb\n\nkein Ende";
        let path = note("bytes", text);
        let mut b = NoteBuffer::open(&path);
        b.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        b.edited(format!("{text}!"));
        b.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{text}!"));
    }
}
