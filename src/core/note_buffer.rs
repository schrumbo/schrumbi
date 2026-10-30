use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// An open note. The editor owns the live text while typing and reports edits here;
/// `text` is the snapshot used by the reader, export and saving.
#[derive(Debug)]
pub struct NoteBuffer {
    pub path: PathBuf,
    pub text: String,
    pub dirty: bool,
    pub missing: bool,
    last_saved: String,
    known: Option<SystemTime>,
    /// Bumped whenever the text changes from outside the editor, so editors reload it.
    pub revision: u64,
}

impl NoteBuffer {
    pub fn open(path: &Path) -> Self {
        let mut b = Self { path: path.to_path_buf(), text: String::new(), dirty: false, missing: false, last_saved: String::new(), known: None, revision: 0 };
        b.load();
        b
    }

    fn modified(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.path).and_then(|m| m.modified()).ok()
    }

    pub fn load(&mut self) {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => {
                self.missing = false;
                if s != self.text {
                    self.text = s.clone();
                    self.revision += 1;
                }
                self.last_saved = s;
                self.known = self.modified();
                self.dirty = false;
            }
            Err(_) => self.missing = !self.path.exists(),
        }
    }

    /// Picks up edits made in another app, unless there are unsaved changes here.
    pub fn reload_if_changed(&mut self) -> bool {
        match (self.dirty, self.modified(), self.known) {
            (false, Some(m), Some(k)) if m > k + Duration::from_millis(500) => {
                self.load();
                true
            }
            _ => false,
        }
    }

    /// The editor's text after an edit.
    pub fn edited(&mut self, text: String) {
        self.dirty = text != self.last_saved;
        self.text = text;
    }

    /// A deliberate change from outside the editor, e.g. ticking a task in the reader.
    pub fn replace(&mut self, text: String) {
        self.dirty = text != self.last_saved;
        self.text = text;
        self.revision += 1;
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        if self.text == self.last_saved && self.path.exists() {
            self.dirty = false;
            return Ok(());
        }
        let tmp = self.path.with_file_name(format!(".{}.tmp", self.path.file_name().unwrap_or_default().to_string_lossy()));
        std::fs::write(&tmp, &self.text)?;
        std::fs::rename(&tmp, &self.path)?;
        self.last_saved = self.text.clone();
        self.known = self.modified();
        self.dirty = false;
        Ok(())
    }

    pub fn name(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }
}
