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
}

impl NoteBuffer {
    pub fn open(path: &Path) -> Self {
        let mut b = Self { path: path.to_path_buf(), text: String::new(), dirty: false, missing: false, undecodable: false, last_saved: String::new(), known: None, revision: 0, conflict: None, held: false, save_wanted: false, touched: false, version: 0, snapshot: Mutex::new(None) };
        b.load();
        b
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
        self.path = path.to_path_buf();
        self.known = self.modified();
    }

    /// Picks up edits made in another app; with unsaved changes here the other version becomes a conflict.
    pub fn reload_if_changed(&mut self) -> bool {
        if self.held {
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

    /// Counts every change of the text; an editor that last saw another count shows an older text.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The text of an editor that last saw `version`. Refused when the note changed since, e.g. in another view:
    /// that text would write over the newer one, so the editor has to show the current text first.
    pub fn edited_on(&mut self, version: u64, text: String) -> bool {
        if version != self.version {
            return false;
        }
        self.edited(text);
        true
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
    fn a_second_editor_with_an_older_text_cannot_write_over_the_newer() {
        let path = note("stale", "a");
        let mut b = NoteBuffer::open(&path);
        let (first, second) = (b.version(), b.version());
        assert!(b.edited_on(first, "a MAIN".into()));
        let first = b.version();
        assert!(!b.edited_on(second, "a OWN".into()));
        assert_eq!(b.text, "a MAIN");
        assert!(b.edited_on(first, "a MAIN again".into()));
        assert_eq!(b.text, "a MAIN again");
        assert!(b.dirty);
    }

    #[test]
    fn every_change_of_the_text_moves_the_version() {
        let path = note("version", "a");
        let mut b = NoteBuffer::open(&path);
        let v0 = b.version();
        b.edited("b".into());
        let v1 = b.version();
        assert_ne!(v0, v1);
        b.edited("b".into());
        assert_eq!(b.version(), v1);
        b.replace("c".into());
        assert_ne!(b.version(), v1);
        let v2 = b.version();
        b.save().unwrap();
        assert_eq!(b.version(), v2);
        outside(&path, "d", 5);
        assert!(b.reload_if_changed());
        assert_ne!(b.version(), v2);
        let v3 = b.version();
        b.edited("d

x".into());
        outside(&path, "y

d", 10);
        assert!(b.reload_if_changed());
        assert_ne!(b.version(), v3);
    }

    #[test]
    fn text_that_differs_from_the_disk_is_dirty_until_saved() {
        let path = note("dot", "a");
        let mut b = NoteBuffer::open(&path);
        outside(&path, "new", 5);
        assert!(b.reload_if_changed());
        assert!(!b.dirty);
        b.edited("a".into());
        assert!(b.dirty);
        b.save().unwrap();
        assert!(!b.dirty);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a");
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

    #[test]
    fn a_large_note_saves_whole_and_atomically() {
        let path = note("large", "a");
        let mut b = NoteBuffer::open(&path);
        let big = "line of text ".repeat(40_000);
        b.edited(big.clone());
        b.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), big);
        assert!(!b.dirty);
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        b.edited(format!("{big}more"));
        outside(&path, "theirs", 5);
        b.save().unwrap();
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
