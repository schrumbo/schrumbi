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
    for path in ["../geheim.md", "MA/../../geheim.md", secret.to_str().unwrap(), "C:/Windows/win.ini", "\\C:/Windows/win.ini", "\\\\server\\share\\x.md"] {
        assert!(turns.call(&mut lib, "s", "read_note", &json!({ "path": path })).is_err(), "{path}");
        assert!(turns.call(&mut lib, "s", "create_note", &json!({ "title": "X", "folder": path })).is_err(), "{path}");
    }
    assert!(turns.call(&mut lib, "s", "list_folder", &json!({ "path": ".." })).is_err());
    assert!(turns.call(&mut lib, "s", "list_folder", &json!({ "path": "/" })).is_ok());
    std::fs::create_dir_all(root.join("MA")).unwrap();
    std::fs::write(root.join("MA/N.md"), "Hallo\n").unwrap();
    assert!(turns.call(&mut lib, "s", "read_note", &json!({ "path": "/MA/N.md" })).is_ok());
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

#[test]
fn a_bounded_turn_joins_every_write_until_it_ends() {
    let root = folder("bounded");
    let mut lib = open(&root);
    let mut turns = Turns::default();
    std::fs::write(root.join("A.md"), "# A
").unwrap();
    turns.begin("c");
    turns.call(&mut lib, "c", "add_cards", &json!({ "note": "A.md", "cards": [{ "front": "F1", "back": "A1" }] })).unwrap();
    let seen = version_of(&tools::read_file(&lib, &root.join("A.md"), None, None).unwrap());
    let text = format!("{}Mehr
", lib.buffer(&root.join("A.md")).text);
    turns.run(&mut lib, "c", |l| tools::write_file(l, &root.join("A.md"), &text, Some(&seen))).unwrap();
    assert_eq!(lib.undo_label(), Some("KI: 2 Änderungen"));
    turns.end("c");
    turns.call(&mut lib, "c", "add_cards", &json!({ "note": "A.md", "cards": [{ "front": "F2", "back": "A2" }] })).unwrap();
    lib.undo();
    assert!(lib.buffer(&root.join("A.md")).text.contains("Mehr"), "the ended turn stays");
    lib.undo();
    assert_eq!(lib.buffer(&root.join("A.md")).text, "# A
");
}

#[test]
fn whole_file_writes_need_a_fresh_read_and_never_leave_the_folder() {
    let root = folder("fswrite");
    let mut lib = open(&root);
    let note = root.join("N.md");
    std::fs::write(&note, "Eins
").unwrap();
    assert!(tools::write_file(&mut lib, &note, "KI
", None).is_err(), "unread files are not replaced");
    let seen = version_of(&tools::read_file(&lib, &note, None, None).unwrap());
    lib.buffer(&note).edited("Eins
Getippt
".into());
    assert!(tools::write_file(&mut lib, &note, "KI
", Some(&seen)).is_err(), "typing since the read is kept");
    assert_eq!(lib.buffer(&note).text, "Eins
Getippt
");
    let seen = version_of(&tools::read_file(&lib, &note, None, None).unwrap());
    assert!(tools::write_file(&mut lib, &note, "Eins
Getippt
KI
", Some(&seen)).is_ok());
    assert_eq!(lib.buffer(&note).text, "Eins
Getippt
KI
");
    assert!(tools::write_file(&mut lib, &root.join("..").join("x.md"), "x", None).is_err());
    assert!(tools::write_file(&mut lib, &root.join("a.pdf"), "x", None).is_err());
    let made = tools::write_file(&mut lib, &root.join("Neu.md"), "Hallo
", None).unwrap();
    assert_eq!(made.value["path"], "Neu.md");
    assert_eq!(lib.buffer(&root.join("Neu.md")).text, "Hallo
");
    assert_eq!(tools::read_file(&lib, &note, Some(2), Some(1)).unwrap(), "Getippt
");
}

#[test]
fn a_session_saved_as_note_quotes_the_user_and_lists_the_steps() {
    use super::transcript::{Entry, Role, markdown};
    let e = |role, text: &str| Entry { role, text: text.into(), at: 0 };
    let md = markdown("KI-Sitzung", &[e(Role::User, "Mach Karten"), e(Role::Tool, "Liest VL3.pdf"), e(Role::Tool, "Ergänzt 2 Karten"), e(Role::Agent, "Fertig.")]);
    assert_eq!(md, "# KI-Sitzung

> Mach Karten

- Liest VL3.pdf
- Ergänzt 2 Karten

Fertig.
");
}

fn lecture_pdf(path: &Path, pages: &[&str]) {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
    let mut kids: Vec<Object> = Vec::new();
    for line in pages {
        let ops = vec![Operation::new("BT", vec![]), Operation::new("Tf", vec!["F1".into(), 12.into()]), Operation::new("Td", vec![72.into(), 700.into()]), Operation::new("Tj", vec![Object::string_literal(*line)]), Operation::new("ET", vec![])];
        let contents = doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()));
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => contents, "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } });
        kids.push(page.into());
    }
    let count = kids.len() as i64;
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}

#[test]
fn cards_from_a_pdf_page_land_in_the_modules_card_note_as_one_step() {
    let root = folder("cards-page");
    std::fs::create_dir_all(root.join("MA")).unwrap();
    let pdf = root.join("MA").join("VL3.pdf");
    lecture_pdf(&pdf, &["Einleitung", "Cauchy-Kriterium: eine Folge konvergiert genau dann", "Ende"]);
    let mut lib = open(&root);
    let mut turns = Turns::default();
    let page = turns.call(&mut lib, "s", "read_pdf_text", &json!({ "path": "MA/VL3.pdf", "pages": "2" })).unwrap();
    assert!(page["content"][0]["text"].as_str().unwrap().contains("Cauchy"), "{page}");
    let target = tools::card_note(&lib, &pdf);
    assert_eq!(target, root.join("MA").join("Karteikarten.md"), "a PDF in a module files its cards in the module's card note");
    let rel = lib.relative_path(&target);
    let preview = tools::preview(&lib, "add_cards", &json!({ "note": rel, "cards": [{ "front": "Cauchy-Kriterium?", "back": "Konvergenz ohne Grenzwert prüfen" }] })).unwrap();
    assert!(preview.lines.iter().any(|l| l.contains("Karteikarten.md")), "{preview:?}");
    assert!(!target.exists(), "a preview writes nothing");
    let cards = json!([{ "front": "Cauchy-Kriterium?", "back": "Konvergenz ohne Grenzwert prüfen" }, { "front": "Was ist eine Cauchy-Folge?", "back": "Glieder kommen sich beliebig nahe" }]);
    let added = turns.call(&mut lib, "s", "add_cards", &json!({ "note": rel, "cards": cards })).unwrap();
    assert_eq!(added["added"], 2);
    let text = lib.buffer(&target).text.clone();
    assert!(text.contains("Cauchy-Kriterium? :: Konvergenz ohne Grenzwert prüfen"), "{text}");
    assert_eq!(crate::core::cards::parse::cards_in(&text).len(), 2);
    assert_eq!(lib.undo_label(), Some("KI: 2 Karten in Karteikarten.md ergänzt"));
    lib.undo();
    assert!(!target.exists(), "one undo takes back the new note with its cards");
    std::fs::write(root.join("MA").join("VL3.md"), "# VL3\n").unwrap();
    assert_eq!(tools::card_note(&lib, &pdf), root.join("MA").join("VL3.md"), "a note of the same name beside the PDF wins");
}

fn proposal() -> Value {
    json!({
        "semester": { "name": "WS 2026/27", "start": "2026-10-19", "end": "2027-02-12", "breaks": [{ "name": "Weihnachten", "start": "2026-12-21", "end": "2027-01-03" }] },
        "modules": [
            { "code": "MA", "name": "Analysis 1", "folder": "MA", "naming": "{code}_{yy}_{title}_{author}_{date}", "weekly": [{ "title": "Übungsblatt", "weekday": 4, "time": "12:00" }] },
            { "code": "LA", "name": "Lineare Algebra", "folder": "LA", "moodle": "https://moodle.example.org/course/view.php?id=7", "weekly": [{ "title": "Abgabe", "weekday": 2, "time": "10:00", "week_offset": 1 }] }
        ],
        "naming": { "on": true, "author": "Muster-M" },
        "moodle_calendar": "https://moodle.example.org/calendar/export_execute.php?token=abc",
        "renames": [{ "from": "MA/blatt1.pdf", "to": "MA/MA_26_Blatt-1_Muster-M_261020.pdf" }],
        "workflow": "Arbeitet wochenweise."
    })
}

#[test]
fn a_setup_proposal_applies_chosen_items_as_one_step_and_never_moves_files() {
    let root = folder("setup");
    std::fs::create_dir_all(root.join("MA")).unwrap();
    std::fs::write(root.join("MA").join("blatt1.pdf"), b"%PDF-1.4").unwrap();
    let mut lib = open(&root);
    let mut turns = Turns::default();
    let before = lib.config.clone().unwrap();
    let on_disk = std::fs::read(root.join("schrumbi.json")).unwrap();
    let shown = turns.call(&mut lib, "s", "propose_setup", &json!({ "proposal": proposal() })).unwrap();
    let keys: Vec<&str> = shown["items"].as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap()).collect();
    assert_eq!(keys, vec!["semester", "module:0", "module:1", "naming", "calendar"]);
    assert_eq!(shown["renames"][0]["from"], "MA/blatt1.pdf");
    assert_eq!(lib.config.as_ref(), Some(&before), "proposing changes nothing");
    assert_eq!(std::fs::read(root.join("schrumbi.json")).unwrap(), on_disk);
    assert!(!lib.can_undo());
    assert!(!tools::is_write("propose_setup") && tools::is_write("apply_setup"));
    let preview = tools::preview(&lib, "apply_setup", &json!({ "proposal": proposal(), "items": ["semester", "module:0", "naming"] })).unwrap();
    assert_eq!(preview.lines.len(), 3);
    turns.call(&mut lib, "s", "apply_setup", &json!({ "proposal": proposal(), "items": ["semester", "module:0", "module:1", "naming", "calendar"] })).unwrap();
    let c = lib.config.clone().unwrap();
    let s = lib.semester().unwrap().clone();
    assert_eq!((s.start.as_str(), s.end.as_str(), s.breaks.len()), ("2026-10-19", "2027-02-12", 1));
    let ma = c.modules.iter().find(|m| m.folder == "MA").unwrap();
    assert_eq!(ma.name, "Analysis 1");
    assert_eq!(ma.rules.len(), 1);
    assert_eq!((ma.rules[0].weekday, ma.rules[0].time.as_str()), (4, "12:00"));
    let today = lib.today().format("%Y-%m-%d").to_string();
    if today.as_str() > "2026-10-30" && today.as_str() < "2027-02-01" {
        assert!(ma.rules[0].start_week.is_some_and(|w| w > 1), "a weekly deadline set up mid-semester starts now, not with an overdue wall");
    }
    assert_eq!(c.modules.iter().filter(|m| m.folder == "MA").count(), 1, "the existing module is changed, not doubled");
    let la = c.modules.iter().find(|m| m.folder == "LA").unwrap();
    assert_eq!(la.semester, s.id);
    assert!(la.links.iter().any(|l| l.address.contains("moodle.example.org") && l.upload));
    assert!(c.naming_convention() && c.author == "Muster-M");
    let key = root.to_string_lossy().to_string();
    assert!(crate::core::prefs::with(|p| p.calendar_sources.get(&key).cloned()).is_some_and(|u| u.contains("token=abc")));
    assert!(root.join("MA").join("blatt1.pdf").exists() && !root.join("MA").join("MA_26_Blatt-1_Muster-M_261020.pdf").exists(), "renames are only shown");
    assert!(!root.join("LA").exists(), "a setup creates no folders");
    assert!(lib.undo_label().is_some_and(|l| l.starts_with("KI: ")));
    lib.undo();
    assert_eq!(lib.config.as_ref(), Some(&before), "one undo takes back every applied setting");
    assert!(crate::core::prefs::with(|p| p.calendar_sources.get(&key).cloned()).is_none(), "the calendar address goes with it");
    assert!(!lib.can_undo());
}

#[test]
fn a_setup_proposal_applies_only_the_items_taken_and_refuses_what_leaves_the_folder() {
    let root = folder("setup-part");
    let mut lib = open(&root);
    let mut turns = Turns::default();
    let before = lib.config.clone().unwrap();
    turns.call(&mut lib, "s", "apply_setup", &json!({ "proposal": proposal(), "items": ["module:1"] })).unwrap();
    let c = lib.config.clone().unwrap();
    assert_eq!(c.modules.len(), before.modules.len() + 1);
    assert_eq!(c.semesters, before.semesters);
    assert_eq!((c.naming_convention(), c.author.as_str()), (before.naming_convention(), before.author.as_str()));
    let mut outside = proposal();
    outside["modules"][0]["folder"] = json!("../anderswo");
    assert!(turns.call(&mut lib, "s", "propose_setup", &json!({ "proposal": outside })).is_err());
    let mut bad = proposal();
    bad["semester"]["end"] = json!("2026-01-01");
    assert!(turns.call(&mut lib, "s", "apply_setup", &json!({ "proposal": bad })).is_err());
    assert_eq!(lib.config.as_ref(), Some(&c), "a refused proposal changes nothing");
    assert!(turns.call(&mut lib, "s", "propose_setup", &json!({ "proposal": {} })).is_err());
    let read = turns.call(&mut lib, "s", "read_setup", &json!({})).unwrap();
    assert!(read["modules"].as_array().unwrap().iter().any(|m| m["folder"] == "LA"));
}

#[test]
fn inbox_files_get_proposals_and_a_filing_turn_undoes_as_one() {
    let root = folder("inbox");
    std::fs::create_dir_all(root.join("_Inbox")).unwrap();
    std::fs::create_dir_all(root.join("MA")).unwrap();
    std::fs::write(root.join("_Inbox").join("Blatt 2.md"), "# Blatt 2\n").unwrap();
    std::fs::write(root.join("_Inbox").join("Skript.txt"), "Analysis Skript").unwrap();
    let mut lib = open(&root);
    let mut turns = Turns::default();
    let listed = turns.call(&mut lib, "s", "list_inbox", &json!({})).unwrap();
    assert_eq!(listed["total"], 2, "{listed}");
    let proposed = turns.call(&mut lib, "s", "propose_filing", &json!({})).unwrap();
    assert_eq!(proposed["files"].as_array().unwrap().len(), 2);
    assert!(proposed["files"][0]["titles"].as_array().is_some());
    assert!(!lib.can_undo(), "proposing moves nothing");
    for f in ["_Inbox/Blatt 2.md", "_Inbox/Skript.txt"] {
        turns.call(&mut lib, "s", "file_document", &json!({ "path": f, "folder": "MA" })).unwrap();
    }
    assert!(root.join("MA").join("Blatt 2.md").exists() && root.join("MA").join("Skript.txt").exists());
    lib.undo();
    assert!(root.join("_Inbox").join("Blatt 2.md").exists() && root.join("_Inbox").join("Skript.txt").exists(), "one undo puts both back");
}

#[test]
fn a_new_note_can_follow_the_folders_naming() {
    let root = folder("named-note");
    std::fs::create_dir_all(root.join("MA")).unwrap();
    let mut lib = open(&root);
    let mut turns = Turns::default();
    let made = turns.call(&mut lib, "s", "create_note", &json!({ "title": "Grenzwerte", "folder": "MA", "name": "VL03 Grenzwerte", "text": "Kern" })).unwrap();
    assert_eq!(made["path"], "MA/VL03 Grenzwerte.md");
    let text = lib.buffer(&root.join("MA").join("VL03 Grenzwerte.md")).text.clone();
    assert!(text.starts_with("# Grenzwerte\n") && text.contains("Kern"), "{text}");
    assert!(turns.call(&mut lib, "s", "create_note", &json!({ "title": "X", "name": "../X" })).is_err());
    lib.undo();
    assert!(!root.join("MA").join("VL03 Grenzwerte.md").exists());
}

#[test]
fn terminal_users_get_one_prompt_per_flow_with_its_guide() {
    use super::prompts::{Flow, mcp_prompts, mcp_text};
    let names: Vec<&str> = mcp_prompts().iter().map(|p| p.0).collect();
    assert_eq!(names, vec!["karten", "erklaeren", "zusammenfassen", "woche", "inbox", "abfragen", "einrichten"]);
    let mut args = serde_json::Map::new();
    args.insert("path".into(), json!("MA/VL3.pdf"));
    args.insert("pages".into(), json!("3-4"));
    let text = mcp_text(Flow::Cards, &args);
    assert!(text.contains("MA/VL3.pdf") && text.contains("3-4") && text.contains("add_cards"), "{text}");
    assert!(mcp_text(Flow::Quiz, &serde_json::Map::new()).contains("read-only"));
    assert!(Flow::Quiz.read_only() && Flow::Explain.read_only() && !Flow::Cards.read_only());
    assert!(mcp_text(Flow::Setup, &serde_json::Map::new()).contains("propose_setup"));
}

#[test]
fn earlier_sessions_are_listed_newest_first_and_continue_with_a_recap() {
    use super::transcript::{Entry, Role, append, list, load, recap};
    let root = folder("history");
    let dir = super::transcript::dir(&root).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let e = |role, text: &str| Entry { role, text: text.into(), at: 0 };
    append(&dir.join("2026-11-01-090000-a.jsonl"), &[e(Role::User, "Erste Frage"), e(Role::Agent, "Antwort")]).unwrap();
    append(&dir.join("2026-11-02-090000-b.jsonl"), &[e(Role::User, "Zweite Frage")]).unwrap();
    append(&dir.join("2026-11-03-090000-c.jsonl"), &[e(Role::Tool, "nur ein Schritt")]).unwrap();
    let past = list(&root, 10);
    assert_eq!(past.iter().map(|p| p.first.as_str()).collect::<Vec<_>>(), vec!["Zweite Frage", "Erste Frage"]);
    assert_eq!(past[0].started.map(|d| d.format("%d.%m.").to_string()).as_deref(), Some("02.11."));
    let entries = load(&past[1].file);
    assert_eq!(entries.len(), 2);
    assert_eq!(recap(&entries, 1000), "User: Erste Frage\nYou: Antwort\n");
    assert_eq!(recap(&entries, 14), "You: Antwort\n", "a long session keeps its newest part");
}

#[test]
fn inbox_files_read_by_the_agent_file_through_the_app_with_new_deadlines_as_one_step() {
    let root = folder("inbox-ai");
    std::fs::create_dir_all(root.join("_Inbox")).unwrap();
    std::fs::create_dir_all(root.join("MA")).unwrap();
    lecture_pdf(&root.join("_Inbox").join("scan1.pdf"), &["Vorlesung 4: Reihen"]);
    lecture_pdf(&root.join("_Inbox").join("blatt.pdf"), &["Uebungsblatt 4, Abgabe bis 20.11."]);
    std::fs::write(root.join("_Inbox").join("loesung.md"), "# Lösung Blatt 4\n").unwrap();
    let mut lib = open(&root);
    let mut turns = Turns::default();
    let due = (chrono::Local::now() + chrono::Duration::days(9)).format("%Y-%m-%d").to_string();
    let files = json!({ "files": [
        { "path": "_Inbox/scan1.pdf", "module": "MA", "kind": "lecture", "date": "2026-11-03", "week": 4, "title": "Reihen" },
        { "path": "_Inbox/blatt.pdf", "module": "MA", "kind": "exercise", "title": "Blatt 4", "new_deadline": { "title": "Blatt 4", "due": due } },
        { "path": "_Inbox/loesung.md", "module": "MA", "kind": "submission", "title": "Blatt 4", "new_deadline": { "title": "Blatt 4", "due": due } }
    ] });
    let shown = turns.call(&mut lib, "s", "propose_inbox", &files).unwrap();
    assert_eq!(shown["files"].as_array().unwrap().len(), 3);
    assert!(root.join("_Inbox").join("blatt.pdf").exists() && !lib.can_undo(), "proposing moves and creates nothing");
    let before = lib.semester_assignments().len();
    let preview = tools::preview(&lib, "apply_inbox", &files).unwrap();
    assert_eq!(preview.lines.len(), 3);
    turns.call(&mut lib, "s", "apply_inbox", &files).unwrap();
    assert_eq!(lib.semester_assignments().len(), before + 1, "the new deadline exists once, though two files named it");
    assert!(std::fs::read_dir(root.join("_Inbox")).unwrap().next().is_none(), "the inbox is empty");
    assert!(walkdir::WalkDir::new(root.join("MA")).into_iter().flatten().any(|e| e.file_name() == "scan1.pdf"));
    lib.undo();
    assert!(root.join("_Inbox").join("scan1.pdf").exists() && root.join("_Inbox").join("blatt.pdf").exists() && root.join("_Inbox").join("loesung.md").exists(), "one undo puts every file back");
    assert_eq!(lib.semester_assignments().len(), before, "and takes the new deadline away");
    let outside = json!({ "files": [{ "path": "../x.pdf", "kind": "other", "module": "MA" }] });
    assert!(turns.call(&mut lib, "s", "propose_inbox", &outside).is_err());
}

#[test]
fn an_inbox_row_edited_in_the_table_finds_its_place_again() {
    use crate::core::ai::inbox::{self, Change, FileKind};
    let root = folder("inbox-edit");
    std::fs::create_dir_all(root.join("_Inbox")).unwrap();
    std::fs::create_dir_all(root.join("MA")).unwrap();
    std::fs::write(root.join("_Inbox").join("scan.md"), "# Notizen
").unwrap();
    let lib = open(&root);
    let rows = inbox::plan(&lib, &json!({ "files": [{ "path": "_Inbox/scan.md", "module": "MA", "kind": "notes", "title": "Reihen", "name": "VL04 Reihen.md" }] })).unwrap();
    let row = &rows[0];
    let dated = inbox::revise(&lib, row, Change::Date(chrono::NaiveDate::from_ymd_opt(2026, 11, 3))).unwrap();
    assert_eq!(dated.name, "VL04 Reihen.md", "a name the agent chose stays when only the date changes");
    assert_eq!(dated.folder, row.folder);
    let titled = inbox::revise(&lib, &dated, Change::Title("Potenzreihen".into())).unwrap();
    assert_eq!(titled.name, "Potenzreihen.md");
    assert!(inbox::revise(&lib, &titled, Change::Title("  ".into())).is_err(), "an empty title is refused");
    let lecture = inbox::revise(&lib, &titled, Change::Kind(FileKind::Lecture)).unwrap();
    assert_eq!(lecture.kind, FileKind::Lecture);
    assert!(lecture.deadline.is_none() && lecture.new_deadline.is_none());
    assert!(root.join("_Inbox").join("scan.md").exists(), "editing a row moves nothing");
}

#[test]
fn agent_errors_say_what_the_user_can_do() {
    use crate::core::ai::agents::{Trouble, trouble};
    assert_eq!(trouble("Authentication required"), Trouble::SignIn);
    assert_eq!(trouble("Invalid API key · Please run /login"), Trouble::SignIn);
    assert_eq!(trouble("TypeError: fetch failed (getaddrinfo ENOTFOUND api.anthropic.com)"), Trouble::Offline);
    assert_eq!(trouble("API Error: 529 Overloaded"), Trouble::Busy);
    assert_eq!(trouble("Claude AI usage limit reached"), Trouble::Busy);
    assert_eq!(trouble("Internal error"), Trouble::Other);
}

#[test]
#[cfg(unix)]
fn finder_path_gains_the_install_folders_that_exist() {
    let home = folder("home");
    std::fs::create_dir_all(home.join(".local/bin")).unwrap();
    std::fs::create_dir_all(home.join(".nvm/versions/node/v22.1.0/bin")).unwrap();
    let path = super::detect::widened_path(std::ffi::OsStr::new("/usr/bin:/bin"), &home).unwrap();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    assert_eq!(&dirs[..2], &[PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
    assert!(dirs.contains(&home.join(".local/bin")));
    assert!(dirs.contains(&home.join(".nvm/versions/node/v22.1.0/bin")));
    assert!(!dirs.contains(&home.join(".bun/bin")));
    let again = std::env::join_paths(&dirs).unwrap();
    assert_eq!(super::detect::widened_path(&again, &home), None);
}
