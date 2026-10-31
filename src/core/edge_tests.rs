use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{Value, json};

use super::library::{Library, Route};

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("schrumbi-edge-{}", std::process::id())).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rule(i: usize) -> Value {
    json!({ "id": format!("r{i}"), "title": format!("Abgabe {i}"), "short": format!("A{i}"), "weekday": (i % 7) + 1, "time": "18:00", "weekOffset": 0, "interval": 1, "upload": true, "folder": format!("W{{w}}/R{i}"), "filePattern": "*" })
}

/// A 15-week semester around today.
fn config(modules: usize, rules: usize) -> Value {
    let today = chrono::Local::now().date_naive();
    json!({
        "version": 2,
        "name": "Test",
        "author": "Muster-M",
        "inbox": "_Inbox",
        "semesters": [{ "id": "s", "name": "WS", "start": (today - chrono::Days::new(50)).format("%Y-%m-%d").to_string(), "end": (today + chrono::Days::new(55)).format("%Y-%m-%d").to_string(), "breaks": [], "holidays": [] }],
        "modules": (0..modules).map(|m| json!({ "id": format!("m{m}"), "code": format!("M{m}"), "name": format!("Modul {m}"), "semester": "s", "folder": format!("M{m}"), "color": "blue", "naming": "{code}_{yy}_{title}_{author}_{date}", "rules": (0..rules).map(rule).collect::<Vec<_>>() })).collect::<Vec<_>>(),
    })
}

fn open(root: &Path) -> Library {
    let mut lib = Library::new();
    lib.open(root);
    if let Some(job) = lib.scan_job() {
        lib.apply_scan(job.run());
    }
    lib
}

fn workspace(name: &str, config: &Value) -> (PathBuf, Library) {
    let root = folder(name);
    std::fs::write(root.join("schrumbi.json"), serde_json::to_vec_pretty(config).unwrap()).unwrap();
    let lib = open(&root);
    (root, lib)
}

/// Touches everything the pages and the tray read, so a panic in any of them fails the test.
fn read_everything(lib: &mut Library) {
    let _ = lib.lecture_weeks();
    let _ = lib.semester_modules();
    let _ = lib.action_items();
    let _ = lib.next_deadline();
    let _ = lib.upcoming();
    let _ = lib.upcoming_days(7);
    let _ = lib.upcoming_exams();
    let _ = lib.misnamed();
    let _ = lib.tray_items();
    let _ = lib.reminders();
    let _ = lib.ics();
    let today = super::day::local_date(lib.now);
    let _ = lib.assignments_on(today);
    let ids: Vec<String> = lib.assignments.iter().take(50).map(|a| a.id.clone()).collect();
    for id in ids {
        if let Some(a) = lib.assignment(&id).cloned() {
            let _ = lib.evaluation(&a);
            let _ = lib.needs_action(&a);
        }
    }
}

#[test]
fn handles_any_number_of_modules() {
    for modules in [0, 1, 10, 25, 50] {
        let start = Instant::now();
        let (_, mut lib) = workspace(&format!("modules-{modules}"), &config(modules, 4));
        assert_eq!(lib.last_error, None);
        assert!(!lib.config_broken);
        assert_eq!(lib.semester_modules().len(), modules);
        assert_eq!(lib.assignments.is_empty(), modules == 0);
        read_everything(&mut lib);
        assert!(lib.tray_items().len() <= 8);
        assert!(lib.reminders().len() <= 60);
        println!("{modules} modules, {} deadlines: {:?}", lib.assignments.len(), start.elapsed());
    }
}

#[test]
fn stays_usable_with_many_deadlines() {
    let (_, mut lib) = workspace("many", &config(50, 20));
    assert!(lib.assignments.len() > 1000, "{}", lib.assignments.len());
    let start = Instant::now();
    read_everything(&mut lib);
    println!("{} deadlines read in {:?}", lib.assignments.len(), start.elapsed());
    assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
}

#[test]
fn asks_for_setup_in_an_empty_folder() {
    let mut lib = open(&folder("empty"));
    assert!(lib.needs_setup);
    assert!(!lib.config_broken);
    read_everything(&mut lib);
}

#[test]
fn keeps_a_broken_config_untouched() {
    for (name, content) in [("garbage", "{ das ist kein json"), ("empty-file", ""), ("wrong-type", "[1, 2, 3]"), ("wrong-field", r#"{"modules": "keine Liste"}"#)] {
        let root = folder(name);
        std::fs::write(root.join("schrumbi.json"), content).unwrap();
        let mut lib = open(&root);
        assert!(lib.config_broken, "{name}");
        assert!(lib.last_error.is_some(), "{name}");
        read_everything(&mut lib);
        assert_eq!(std::fs::read_to_string(root.join("schrumbi.json")).unwrap(), content, "{name}");
    }
}

#[test]
fn opens_a_config_from_a_newer_version() {
    let mut c = config(1, 1);
    c["version"] = json!(999);
    c["etwasNeues"] = json!({ "a": 1 });
    c["modules"][0]["unbekannt"] = json!(true);
    let (_, mut lib) = workspace("future", &c);
    assert!(!lib.config_broken);
    assert_eq!(lib.semester_modules().len(), 1);
    read_everything(&mut lib);
}

#[test]
fn survives_a_missing_workspace() {
    let mut lib = Library::new();
    lib.open(&std::env::temp_dir().join("schrumbi-gibt-es-nicht-1234"));
    read_everything(&mut lib);
}

#[test]
fn survives_nonsense_values() {
    let mut c = config(1, 1);
    let r = &mut c["modules"][0]["rules"][0];
    r["weekday"] = json!(99);
    r["time"] = json!("99:99");
    r["weekOffset"] = json!(-400);
    r["interval"] = json!(0);
    r["folder"] = json!("../../../{w}/{kw}/{gibtsnicht}");
    r["filePattern"] = json!("[");
    let (_, mut lib) = workspace("nonsense", &c);
    read_everything(&mut lib);

    for (name, start, end) in [("reversed", "2027-02-05", "2026-10-12"), ("invalid-date", "morgen", "irgendwann"), ("empty-date", "", ""), ("single-day", "2026-10-12", "2026-10-12")] {
        let mut c = config(2, 2);
        c["semesters"][0]["start"] = json!(start);
        c["semesters"][0]["end"] = json!(end);
        let (_, mut lib) = workspace(name, &c);
        read_everything(&mut lib);
    }

    let mut c = config(1, 1);
    c["semesters"][0]["breaks"] = json!([{ "name": "Alles", "start": "2000-01-01", "end": "2100-01-01" }, { "name": "Kaputt", "start": "x", "end": "y" }]);
    c["semesters"][0]["holidays"] = json!([{ "name": "Kaputt", "date": "x" }]);
    let (_, mut lib) = workspace("all-break", &c);
    read_everything(&mut lib);
}

#[test]
fn survives_a_module_without_a_semester() {
    let mut c = config(2, 1);
    c["modules"][0]["semester"] = json!("gibt-es-nicht");
    c["semesters"] = json!([]);
    let (_, mut lib) = workspace("orphan", &c);
    read_everything(&mut lib);
}

#[test]
fn handles_special_characters_in_paths_and_names() {
    let root = folder("Prüfung 🐌 & Übung (1) [x] 'a' #1");
    let mut c = config(1, 1);
    let long = "Sehr langer Titel ".repeat(30);
    c["modules"][0]["name"] = json!(long);
    c["modules"][0]["code"] = json!("Ä/ö\\:*?");
    c["modules"][0]["folder"] = json!("Modul 🐌 Ü");
    c["modules"][0]["rules"][0]["title"] = json!(long);
    std::fs::write(root.join("schrumbi.json"), serde_json::to_vec(&c).unwrap()).unwrap();
    let dir = root.join("Modul 🐌 Ü");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Übung 🐌 final (2).pdf"), b"x").unwrap();
    let mut lib = open(&root);
    assert_eq!(lib.last_error, None);
    assert!(!lib.config_broken);
    read_everything(&mut lib);
}

#[test]
fn saves_changes_only_as_valid_json() {
    let (root, mut lib) = workspace("save", &config(10, 2));
    let id = lib.assignments.first().map(|a| a.id.clone()).unwrap();
    lib.set_status(Some(super::model::Status::Uploaded), &id, false);
    let state = std::fs::read(root.join(".schrumbi-state.json")).unwrap();
    assert!(serde_json::from_slice::<Value>(&state).is_ok());
    let again = open(&root);
    let a = again.assignment(&id).cloned().unwrap();
    assert_eq!(again.evaluation(&a).status, super::model::Status::Uploaded);
}

/// Runs in its own process because the time zone is global; see `keeps_wall_clock_times_across_dst`.
#[test]
#[ignore]
fn dst_child() {
    use chrono::Timelike;
    let mut c = config(1, 0);
    c["timeZone"] = json!("Europe/Berlin");
    c["semesters"][0]["start"] = json!("2026-03-02");
    c["semesters"][0]["end"] = json!("2026-11-27");
    c["modules"][0]["rules"] = json!([
        { "id": "a", "title": "Abend", "short": "A", "weekday": 7, "time": "18:00", "interval": 1, "folder": "W{w}/a", "filePattern": "*" },
        { "id": "b", "title": "Lücke", "short": "B", "weekday": 7, "time": "02:30", "interval": 1, "folder": "W{w}/b", "filePattern": "*" },
    ]);
    let (_, mut lib) = workspace("dst", &c);
    read_everything(&mut lib);
    let mut count = 0;
    for a in lib.assignments.iter().filter(|a| a.rule_id.as_deref() == Some("a")) {
        assert_eq!((a.due.with_timezone(&chrono_tz::Europe::Berlin).hour(), a.due.with_timezone(&chrono_tz::Europe::Berlin).minute()), (18, 0), "{}", a.due);
        count += 1;
    }
    assert!(count > 30, "{count}");
    assert!(lib.assignments.iter().any(|a| a.rule_id.as_deref() == Some("b")));
}

#[test]
fn keeps_wall_clock_times_across_dst() {
    let status = std::process::Command::new(std::env::current_exe().unwrap()).args(["--ignored", "--exact", "core::edge_tests::dst_child"]).status().unwrap();
    assert!(status.success());
}

/// A small deterministic generator, so a failure can be replayed.
struct Fuzz(u64);

impl Fuzz {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn text(&mut self, max: u64) -> String {
        const PIECES: [&str; 44] = [
            "a", "Z", "ä", "Ö", "ß", "é", "🐌", "\u{301}", "\u{200d}", " ", "  ", "\n", "\t", "-", "_", ".", "/", "\\", ":", "%", "%@", "%d", "{", "}", "{code}", "{date:", "{date:%", "{title}", "{w}", "yyyy", "[", "]", "(", ")", "[ ]", "[x]", "- ", "==", "*", "`", "#", "|", "0", "261030",
        ];
        (0..self.next() % max).map(|_| PIECES[(self.next() % PIECES.len() as u64) as usize]).collect()
    }
}

#[test]
fn survives_random_text_everywhere() {
    use super::model::{CoauthorStyle, Module, Rule};
    use super::naming::{self, Naming};
    use super::{catalog, day, edit_ops, filing, markdown, text};

    let mut f = Fuzz(0x5c0b);
    let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
    for _ in 0..4000 {
        let a = f.text(24);
        let b = f.text(12);
        let _ = Naming::clean(&a);
        let _ = Naming::clean_author(&a, CoauthorStyle::Initial);
        let _ = Naming::clean_author(&a, CoauthorStyle::Free);
        let _ = Naming::guess_title(&a);
        let _ = naming::stem(&a);
        let _ = naming::extension(&a);
        let module = Module { code: b.clone(), naming: a.clone(), ..Default::default() };
        let n = Naming::with_author(&module, None, &b, &b, CoauthorStyle::Initial);
        let name = n.render(&b, date, "pdf", &[b.clone()], Some(3), Some(44), None);
        let _ = n.is_valid(&name);
        let _ = n.is_valid(&a);
        let _ = n.parse(&a);
        let _ = day::date_in_name(&a);
        let _ = catalog::fuzzy(&a, &b);
        let _ = catalog::Query::parse(&a, &[], &[]);
        let _ = catalog::week_index(&a);
        let _ = markdown::runs(&a);
        let _ = markdown::title(&a);
        let _ = markdown::task_counts(&a);
        let _ = markdown::to_html(&a, None);
        let _ = markdown::percent_decode(&a);
        let _ = markdown::highlight(&a, Some("rust"));
        let _ = markdown::toggle_task(&a, (f.next() % 5) as usize);
        let _ = filing::task_titles(&a);
        let _ = filing::clean_file_name(&a);
        let _ = filing::headline(&a, &[b.clone()]);
        let _ = text::middle_ellipsis(&a, (f.next() % 30) as usize);
        let _ = text::fold(&a);
        let _ = text::natural_cmp(&a, &b);
        let _ = crate::ui::markdown_view::parse(&a);
        let _ = crate::ui::fields::parse_number(&a);
        let _ = super::i18n::trf(&a, &[&b]);
        let _ = Rule::default();

        let boundaries: Vec<usize> = a.char_indices().map(|(i, _)| i).chain([a.len()]).collect();
        let x = boundaries[(f.next() as usize) % boundaries.len()];
        let y = boundaries[(f.next() as usize) % boundaries.len()];
        let sel = x.min(y)..x.max(y);
        let edits = [
            edit_ops::wrap(&a, sel.clone(), "**"),
            edit_ops::link(&a, sel.clone()),
            edit_ops::edit_lines(&a, sel.clone(), edit_ops::toggled_task),
            edit_ops::toggle_comment(&a, sel.clone()),
            edit_ops::copy_lines(&a, sel.clone(), true),
            edit_ops::copy_lines(&a, sel.clone(), false),
            edit_ops::delete_lines(&a, sel.clone()),
            edit_ops::shift_lines(&a, sel.clone(), 1),
            edit_ops::shift_lines(&a, sel.clone(), -1),
        ];
        for e in edits.iter().chain(edit_ops::move_lines(&a, sel.clone(), true).iter()).chain(edit_ops::move_lines(&a, sel.clone(), false).iter()).chain(edit_ops::newline(&a, sel.start).iter()) {
            let out = edit_ops::apply(&a, e);
            assert!(out.is_char_boundary(e.select.start.min(out.len())), "{a:?} {sel:?}");
        }
        let _ = edit_ops::select_lines(&a, sel.clone());
        let _ = edit_ops::tab_indents(&a, &sel);
    }
}

#[test]
fn survives_random_settings() {
    let mut f = Fuzz(0xbeef);
    for i in 0..120 {
        let mut c = config(2, 2);
        let m = &mut c["modules"][0];
        m["code"] = json!(f.text(8));
        m["name"] = json!(f.text(8));
        m["folder"] = json!(f.text(8));
        m["naming"] = json!(f.text(10));
        m["rules"][0]["title"] = json!(f.text(8));
        m["rules"][0]["folder"] = json!(f.text(8));
        m["rules"][0]["filePattern"] = json!(f.text(6));
        m["rules"][0]["time"] = json!(f.text(4));
        c["author"] = json!(f.text(6));
        c["inbox"] = json!(f.text(6));
        c["semesters"][0]["name"] = json!(f.text(6));
        c["semesters"][0]["breaks"] = json!([{ "name": f.text(4), "start": f.text(3), "end": f.text(3) }]);
        let (_, mut lib) = workspace(&format!("settings-{i}"), &c);
        read_everything(&mut lib);
    }
}

#[test]
fn undo_and_redo_restore_status_and_keep_typed_notes() {
    let (root, mut lib) = workspace("undo-status", &config(1, 1));
    let id = lib.assignments[0].id.clone();
    assert!(!lib.can_undo());
    lib.toggle_done(&id);
    let done = lib.state.overrides.contains_key(&id);
    assert!(done);
    lib.set_note(&id, "getippt".into());
    lib.undo();
    assert!(!lib.state.overrides.contains_key(&id));
    assert_eq!(lib.note(&id), "getippt");
    assert!(lib.can_redo());
    lib.redo();
    assert!(lib.state.overrides.contains_key(&id));
    lib.undo();
    lib.toggle_done(&id);
    assert!(!lib.can_redo(), "a new step ends the redo branch");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn undo_and_redo_move_renamed_files() {
    let (root, mut lib) = workspace("undo-rename", &config(1, 1));
    let file = root.join("a.txt");
    std::fs::write(&file, "x").unwrap();
    lib.rename(&file, "b.txt");
    assert!(root.join("b.txt").exists());
    lib.undo();
    assert!(file.exists() && !root.join("b.txt").exists());
    lib.redo();
    assert!(!file.exists() && root.join("b.txt").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quick_config_edits_become_one_undo_step() {
    let (root, mut lib) = workspace("undo-config", &config(1, 1));
    let original = lib.config.clone().unwrap();
    for name in ["A", "AB", "ABC"] {
        let before = lib.config_snapshot().unwrap();
        let mut c = lib.config.clone().unwrap();
        c.name = name.into();
        lib.update_config(c);
        lib.record_edit("config", "Einstellung".into(), before);
    }
    lib.undo();
    assert_eq!(lib.config.as_ref(), Some(&original));
    assert!(!lib.can_undo());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn back_and_forward_walk_the_route_history() {
    let mut lib = Library::new();
    lib.set_route(Route::Grid);
    lib.set_route(Route::Inbox);
    assert!(lib.can_go_back());
    lib.go_back();
    assert_eq!(*lib.route(), Route::Grid);
    lib.go_forward();
    assert_eq!(*lib.route(), Route::Inbox);
    lib.go_back();
    lib.set_route(Route::Record);
    assert!(!lib.can_go_forward());
}

#[test]
fn back_skips_closed_documents() {
    let mut lib = Library::new();
    lib.set_route(Route::Grid);
    lib.set_route(Route::Doc("/gone.md".into()));
    lib.set_route(Route::Inbox);
    lib.go_back();
    assert_eq!(*lib.route(), Route::Grid);
}

#[test]
fn tabs_reorder_and_closed_tabs_reopen() {
    let root = dunce::canonicalize(folder("tabs")).unwrap();
    let (a, b, c) = (root.join("a.md"), root.join("b.md"), root.join("c.md"));
    for p in [&a, &b, &c] {
        std::fs::write(p, "x").unwrap();
    }
    let mut lib = Library::new();
    for p in [&a, &b, &c] {
        lib.open_document(p);
    }
    lib.move_document(&c, &a);
    assert_eq!(lib.open_documents, vec![c.clone(), a.clone(), b.clone()]);
    lib.close_document(&a);
    std::fs::remove_file(&b).unwrap();
    lib.close_document(&b);
    lib.reopen_document();
    assert_eq!(lib.open_documents, vec![c.clone(), a.clone()]);
}

#[test]
fn split_view_follows_the_tabs() {
    let root = dunce::canonicalize(folder("split")).unwrap();
    let (a, b) = (root.join("a.md"), root.join("b.md"));
    for p in [&a, &b] {
        std::fs::write(p, "x").unwrap();
    }
    let mut lib = Library::new();
    lib.open_document(&a);
    lib.open_document(&b);
    lib.set_split(Some(b.clone()));
    assert_eq!(lib.split_document, None, "the current tab cannot sit beside itself");
    lib.set_split(Some(a.clone()));
    assert_eq!(lib.split_document, Some(a.clone()));
    lib.open_document(&a);
    assert_eq!(lib.split_document, None, "bringing it to the front ends the split");
    lib.set_split(Some(b.clone()));
    lib.close_document(&b);
    assert_eq!(lib.split_document, None);
}

#[test]
fn sync_gate_holds_writes_and_reloads_pulled_files() {
    let root = folder("sync-gate");
    std::fs::write(root.join("schrumbi.json"), config(1, 1).to_string()).unwrap();
    let note = root.join("n.md");
    std::fs::write(&note, "alt").unwrap();
    let mut lib = open(&root);
    let key = dunce::canonicalize(&note).unwrap();
    lib.buffer(&key);
    lib.begin_sync();
    std::fs::write(&key, "gezogen").unwrap();
    std::fs::File::options().write(true).open(&key).unwrap().set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000)).unwrap();
    assert!(!lib.reload_external());
    lib.finish_sync(std::slice::from_ref(&key));
    assert_eq!(lib.buffer(&key).text, "gezogen");
    assert!(lib.buffer(&key).conflict.is_none());
}

#[test]
fn deciding_about_another_apps_version_can_be_undone_and_redone() {
    let root = folder("note-undo");
    std::fs::write(root.join("schrumbi.json"), config(1, 1).to_string()).unwrap();
    let note = root.join("n.md");
    std::fs::write(&note, "alt").unwrap();
    let mut lib = open(&root);
    let key = dunce::canonicalize(&note).unwrap();
    lib.buffer(&key).edited("meins".into());
    std::fs::write(&key, "fremd").unwrap();
    let _ = lib.buffer(&key).save();
    assert_eq!(lib.buffer(&key).conflict.as_deref(), Some("fremd"));
    lib.keep_note_mine(&key);
    assert_eq!(std::fs::read_to_string(&key).unwrap(), "meins");
    lib.undo();
    assert_eq!(std::fs::read_to_string(&key).unwrap(), "fremd");
    assert_eq!(lib.buffer(&key).text, "fremd");
    lib.redo();
    assert_eq!(std::fs::read_to_string(&key).unwrap(), "meins");
    lib.buffer(&key).edited("meins 2".into());
    std::fs::write(&key, "fremd 2").unwrap();
    let _ = lib.buffer(&key).save();
    lib.load_note_disk(&key);
    assert_eq!(lib.buffer(&key).text, "fremd 2");
    lib.undo();
    assert_eq!(lib.buffer(&key).text, "meins 2");
    assert_eq!(std::fs::read_to_string(&key).unwrap(), "meins 2");
}

#[test]
fn a_config_from_a_newer_version_is_not_overwritten_after_a_pull() {
    let root = folder("newer");
    std::fs::write(root.join("schrumbi.json"), config(1, 1).to_string()).unwrap();
    let mut lib = open(&root);
    let mut newer = config(1, 1);
    newer["version"] = json!(99);
    std::fs::write(root.join("schrumbi.json"), newer.to_string()).unwrap();
    assert!(lib.reload_external());
    assert!(lib.last_error.is_some());
    lib.write_config();
    let on_disk: Value = serde_json::from_slice(&std::fs::read(root.join("schrumbi.json")).unwrap()).unwrap();
    assert_eq!(on_disk["version"], 99);
}
