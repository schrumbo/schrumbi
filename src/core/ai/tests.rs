use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use super::tools::{self, version_of};
use super::turn::Turns;
use crate::core::library::Library;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("schrumbi-ai-{}", std::process::id())).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dunce::canonicalize(&dir).unwrap()
}

fn config() -> Value {
    let today = chrono::Local::now().date_naive();
    json!({
        "version": 2,
        "name": "Test",
        "author": "Muster-M",
        "inbox": "_Inbox",
        "semesters": [{ "id": "s", "name": "WS", "start": (today - chrono::Days::new(50)).format("%Y-%m-%d").to_string(), "end": (today + chrono::Days::new(55)).format("%Y-%m-%d").to_string(), "breaks": [], "holidays": [] }],
        "modules": [{ "id": "m0", "code": "MA", "name": "Analysis", "semester": "s", "folder": "MA", "color": "blue", "rules": [] }],
    })
}

fn open(root: &Path) -> Library {
    std::fs::write(root.join("schrumbi.json"), config().to_string()).unwrap();
    let mut lib = Library::new();
    lib.open(root);
    if let Some(job) = lib.scan_job() {
        lib.apply_scan(job.run());
    }
    lib
}

fn read(lib: &mut Library, turns: &mut Turns, path: &str) -> Value {
    turns.call(lib, "s", "read_note", &json!({ "path": path })).unwrap()
}

#[test]
fn a_stale_version_is_refused_and_nothing_is_written() {
    let root = folder("stale");
    let mut lib = open(&root);
    let mut turns = Turns::default();
    std::fs::write(root.join("N.md"), "Eins\n").unwrap();
    let old = read(&mut lib, &mut turns, "N.md")["version"].as_str().unwrap().to_string();
    assert!(turns.call(&mut lib, "s", "write_note", &json!({ "path": "N.md", "version": "0000", "mode": "append", "text": "Zwei" })).is_err());
    lib.buffer(&root.join("N.md")).edited("Eins\nGetippt\n".into());
    let err = turns.call(&mut lib, "s", "write_note", &json!({ "path": "N.md", "version": old, "mode": "append", "text": "Zwei" })).unwrap_err();
    assert!(err.contains("version"), "{err}");
    assert!(tools::preview(&lib, "write_note", &json!({ "path": "N.md", "version": old, "mode": "append", "text": "Zwei" })).is_err());
    assert_eq!(lib.buffer(&root.join("N.md")).text, "Eins\nGetippt\n");
    assert!(!lib.can_undo());
}

#[test]
fn two_writes_in_a_turn_are_one_undo_step() {
    let root = folder("turn");
    let mut lib = open(&root);
    let mut turns = Turns::default();
    std::fs::write(root.join("A.md"), "# A\n").unwrap();
    std::fs::write(root.join("B.md"), "# B\n").unwrap();
    let a = read(&mut lib, &mut turns, "A.md")["version"].as_str().unwrap().to_string();
    turns.call(&mut lib, "s", "write_note", &json!({ "path": "A.md", "version": a, "mode": "append", "text": "Neu in A" })).unwrap();
    turns.call(&mut lib, "s", "add_cards", &json!({ "note": "B.md", "cards": [{ "front": "Frage", "back": "Antwort" }] })).unwrap();
    assert_eq!(lib.undo_label(), Some("KI: 2 Änderungen"));
    assert!(lib.buffer(&root.join("A.md")).text.contains("Neu in A"));
    assert!(lib.buffer(&root.join("B.md")).text.contains("Frage :: Antwort"));
    lib.undo();
    assert_eq!(lib.buffer(&root.join("A.md")).text, "# A\n");
    assert_eq!(lib.buffer(&root.join("B.md")).text, "# B\n");
    assert!(!lib.can_undo(), "one undo takes back the whole turn");
}

#[test]
fn a_step_of_the_user_between_two_writes_starts_a_new_turn() {
    let root = folder("between");
    let mut lib = open(&root);
    let mut turns = Turns::default();
    std::fs::write(root.join("A.md"), "# A\n").unwrap();
    std::fs::write(root.join("B.md"), "# B\n").unwrap();
    turns.call(&mut lib, "s", "add_cards", &json!({ "note": "A.md", "cards": [{ "front": "F1", "back": "A1" }] })).unwrap();
    assert!(lib.edit_note(&root.join("B.md"), "note", "Getippt".into(), |t| format!("{t}Text\n")));
    turns.call(&mut lib, "s", "add_cards", &json!({ "note": "A.md", "cards": [{ "front": "F2", "back": "A2" }] })).unwrap();
    assert!(lib.undo_label().is_some_and(|l| l.starts_with("KI: 1 Karte in")), "{:?}", lib.undo_label());
    lib.undo();
    assert!(lib.buffer(&root.join("A.md")).text.contains("F1"));
    assert!(!lib.buffer(&root.join("A.md")).text.contains("F2"));
    assert!(lib.buffer(&root.join("B.md")).text.ends_with("Text\n"), "the user's step stays");
}

#[test]
fn another_sessions_write_does_not_join_the_turn() {
    let root = folder("sessions");
    let mut lib = open(&root);
    let mut turns = Turns::default();
    std::fs::write(root.join("A.md"), "# A\n").unwrap();
    turns.call(&mut lib, "s", "add_cards", &json!({ "note": "A.md", "cards": [{ "front": "F1", "back": "A1" }] })).unwrap();
    turns.call(&mut lib, "t", "add_cards", &json!({ "note": "A.md", "cards": [{ "front": "F2", "back": "A2" }] })).unwrap();
    lib.undo();
    assert!(lib.buffer(&root.join("A.md")).text.contains("F1"));
    assert!(!lib.buffer(&root.join("A.md")).text.contains("F2"));
}

#[test]
fn a_change_from_another_app_is_not_overwritten() {
    let root = folder("conflict");
    let mut lib = open(&root);
    let mut turns = Turns::default();
    let note = root.join("N.md");
    std::fs::write(&note, "Eins\n").unwrap();
    let version = read(&mut lib, &mut turns, "N.md")["version"].as_str().unwrap().to_string();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&note, "Eins\nAus Obsidian\n").unwrap();
    assert!(turns.call(&mut lib, "s", "write_note", &json!({ "path": "N.md", "version": version, "mode": "replace", "text": "KI\n" })).is_err());
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "Eins\nAus Obsidian\n");

    lib.buffer(&note).edited("Hier ungesichert\n".into());
    let version = version_of("Hier ungesichert\n");
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&note, "Dort gesichert\n").unwrap();
    assert!(turns.call(&mut lib, "s", "write_note", &json!({ "path": "N.md", "version": version, "mode": "replace", "text": "KI\n" })).is_err());
    assert!(turns.call(&mut lib, "s", "add_cards", &json!({ "note": "N.md", "cards": [{ "front": "F", "back": "A" }] })).is_err());
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "Dort gesichert\n");
    assert!(!lib.can_undo());
}

#[test]
fn paths_never_leave_the_study_folder() {
    let root = folder("outside");
    let mut lib = open(&root);
    let mut turns = Turns::default();
    let secret = root.parent().unwrap().join("geheim.md");
    std::fs::write(&secret, "geheim").unwrap();
    for path in ["../geheim.md", "MA/../../geheim.md", secret.to_str().unwrap(), "C:/Windows/win.ini", "\\\\server\\share\\x.md"] {
        assert!(turns.call(&mut lib, "s", "read_note", &json!({ "path": path })).is_err(), "{path}");
        assert!(turns.call(&mut lib, "s", "create_note", &json!({ "title": "X", "folder": path })).is_err(), "{path}");
    }
    assert!(turns.call(&mut lib, "s", "list_folder", &json!({ "path": ".." })).is_err());
}

#[test]
fn reads_see_unsaved_text_and_write_modes_land_where_asked() {
    let root = folder("modes");
    let mut lib = open(&root);
    let mut turns = Turns::default();
    std::fs::write(root.join("N.md"), "# Titel\nText\n\n## Teil\nAlt\n\n## Ende\nSchluss\n").unwrap();
    lib.buffer(&root.join("N.md")).edited("# Titel\nText\n\n## Teil\nAlt\n\n## Ende\nSchluss ungesichert\n".into());
    let note = read(&mut lib, &mut turns, "N.md");
    assert!(note["text"].as_str().unwrap().contains("ungesichert"));
    let v = note["version"].as_str().unwrap().to_string();
    let out = turns.call(&mut lib, "s", "write_note", &json!({ "path": "N.md", "version": v, "mode": "insert_after_heading", "heading": "Teil", "text": "Neu" })).unwrap();
    assert_eq!(lib.buffer(&root.join("N.md")).text, "# Titel\nText\n\n## Teil\nAlt\n\nNeu\n\n## Ende\nSchluss ungesichert\n");
    let v = out["version"].as_str().unwrap().to_string();
    turns.call(&mut lib, "s", "write_note", &json!({ "path": "N.md", "version": v, "mode": "replace_lines", "from_line": 5, "to_line": 5, "text": "Ersetzt" })).unwrap();
    assert!(lib.buffer(&root.join("N.md")).text.contains("## Teil\nErsetzt\n"));
}

#[test]
fn deadlines_are_listed_and_added_through_the_library() {
    let root = folder("deadlines");
    let mut lib = open(&root);
    let mut turns = Turns::default();
    let due = (chrono::Local::now() + chrono::Duration::days(3)).format("%Y-%m-%d").to_string();
    let added = turns.call(&mut lib, "s", "add_deadline", &json!({ "title": "Blatt 3", "module": "MA", "due": due })).unwrap();
    let id = added["id"].as_str().unwrap().to_string();
    let list = turns.call(&mut lib, "s", "list_deadlines", &json!({ "days": 7 })).unwrap();
    assert!(list["deadlines"].as_array().unwrap().iter().any(|d| d["id"] == id.as_str()));
    turns.call(&mut lib, "s", "set_status", &json!({ "id": id, "status": "done" })).unwrap();
    assert_eq!(lib.undo_label(), Some("KI: 2 Änderungen"));
    lib.undo();
    assert!(lib.assignment(&id).is_none());
}

#[test]
fn every_tool_has_a_valid_schema_and_writes_are_known() {
    let defs = tools::definitions();
    for d in &defs {
        assert_eq!(d.schema["type"], "object", "{}", d.name);
        assert!(d.description.len() > 10);
    }
    for w in ["write_note", "create_note", "add_cards", "add_deadline", "set_status", "file_document"] {
        assert!(tools::is_write(w), "{w}");
    }
    assert!(!tools::is_write("read_note"));
    assert!(!defs.iter().any(|d| d.name.contains("delete") || d.name.contains("rate") || d.name.contains("shell")));
}

#[test]
fn the_bridge_answers_cleanly_without_a_running_app() {
    let err = super::mcp::forward("s", Some(u32::MAX), "list_deadlines", &json!({})).unwrap_err();
    assert!(err.contains("not running"), "{err}");
}

/// `cargo test search_stays_fast -- --ignored`: 20000 notes, a search through the tool under 100 ms.
#[test]
#[ignore]
fn search_stays_fast_at_twenty_thousand_notes() {
    let root = folder("many");
    for d in 0..100 {
        let dir = root.join(format!("Ordner {d}"));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..200 {
            std::fs::write(dir.join(format!("Notiz {d}-{i}.md")), format!("# Notiz {i}\nGrenzwert Folge Reihe {d} {i} Konvergenz\n")).unwrap();
        }
    }
    let mut lib = open(&root);
    lib.search_index = Arc::new(crate::core::search_index::SearchIndex::open_at(&root.join("index.sqlite")));
    let job = lib.index_job().unwrap();
    let (count, catalog) = job.run();
    lib.apply_index(count, catalog);
    assert!(count >= 20_000, "{count}");
    let mut turns = Turns::default();
    let _ = turns.call(&mut lib, "s", "search", &json!({ "query": "konvergenz" }));
    let start = std::time::Instant::now();
    let out = turns.call(&mut lib, "s", "search", &json!({ "query": "grenzwert 42" })).unwrap();
    let took = start.elapsed();
    println!("search at {count} files: {took:?}");
    assert!(!out["hits"].as_array().unwrap().is_empty());
    assert!(took.as_millis() < 100, "{took:?}");
}
