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
        let _ = filing::default_title(&a, &[]);
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
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(b.clone()), Some(a.clone())), "opening the note on the right keeps both");
    lib.close_document(&a);
    assert_eq!(lib.split_document, None, "closing the note on the right folds the split");
    lib.open_document(&a);
    lib.set_split(Some(b.clone()));
    lib.close_document(&a);
    assert_eq!(lib.active_document, Some(b.clone()));
    assert_eq!(lib.split_document, None, "the last left tab gone, the note on the right moves over");
}

#[test]
fn dropped_documents_open_here_or_beside() {
    use super::library::DocDrop::{Beside, Here};
    let root = dunce::canonicalize(folder("split-drop")).unwrap();
    let (a, b, c, d) = (root.join("a.md"), root.join("b.md"), root.join("c.pdf"), root.join("d"));
    for p in [&a, &b, &c] {
        std::fs::write(p, "x").unwrap();
    }
    std::fs::create_dir_all(&d).unwrap();
    let mut lib = Library::new();
    lib.open_document(&a);
    assert!(!lib.can_drop_document(&a, Beside, None), "a single tab has nothing to sit beside");
    assert!(!lib.can_drop_document(&a, Here, None));
    assert!(!lib.can_drop_document(&d, Here, None), "folders do not open as documents");
    assert!(!lib.can_drop_document(&root.join("gone.md"), Beside, None));
    lib.drop_document(&c, Beside, None);
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(a.clone()), Some(c.clone())));
    assert_eq!(lib.open_documents, vec![a.clone(), c.clone()]);
    assert!(!lib.can_drop_document(&c, Beside, None), "already beside");
    lib.drop_document(&b, Beside, None);
    assert_eq!(lib.split_document, Some(b.clone()), "a drop into the split replaces the right side");
    assert_eq!(lib.open_documents, vec![a.clone(), b.clone(), c.clone()]);
    lib.drop_document(&a, Beside, None);
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(b.clone()), Some(a.clone())), "the shown document moving right swaps the sides");
    lib.drop_document(&a, Here, None);
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(a.clone()), Some(b.clone())), "the right document dropped left swaps back");
    lib.set_split(None);
    lib.drop_document(&a, Beside, Some(&c));
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(c.clone()), Some(a.clone())), "the previous tab takes the left side");
    lib.drop_document(&b, Here, None);
    assert_eq!(lib.active_document, Some(b.clone()));
    assert_eq!(lib.open_documents.len(), 3, "nothing opened twice, nothing closed");
}

#[test]
fn drops_follow_the_document_on_screen() {
    use super::library::DocDrop::{Beside, Here};
    let root = dunce::canonicalize(folder("split-drop-route")).unwrap();
    let (a, b, c) = (root.join("a.md"), root.join("b.md"), root.join("c.md"));
    for p in [&a, &b, &c] {
        std::fs::write(p, "x").unwrap();
    }
    let mut lib = Library::new();
    for p in [&a, &b, &c] {
        lib.open_document(p);
    }
    lib.open_document(&a);
    lib.drop_document(&a, Beside, None);
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(b.clone()), Some(a.clone())), "the neighbouring tab takes the left side");
    lib.set_split(None);
    lib.set_route(Route::Doc(c.clone()));
    assert!(lib.can_drop_document(&b, Here, None), "the routed pane counts, not the last active tab");
    assert!(!lib.can_drop_document(&c, Here, None));
    lib.drop_document(&b, Beside, None);
    assert_eq!(*lib.route(), Route::Doc(c.clone()));
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(c.clone()), Some(b.clone())));
    lib.set_split(None);
    std::fs::remove_file(&a).unwrap();
    lib.drop_document(&a, Here, None);
    assert_eq!((lib.route().clone(), lib.open_documents.len()), (Route::Doc(a.clone()), 3), "a vanished tab still comes to the front");
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

#[test]
fn restoring_a_version_can_be_undone() {
    let root = folder("restore");
    std::fs::write(root.join("schrumbi.json"), config(1, 1).to_string()).unwrap();
    let pdf = dunce::canonicalize(&root).unwrap().join("a.pdf");
    std::fs::write(&pdf, [1u8, 0, 2]).unwrap();
    let note = pdf.with_file_name("n.md");
    std::fs::write(&note, "neu").unwrap();
    let mut lib = open(&root);
    lib.restore_version(&pdf, vec![9, 0, 9]);
    assert_eq!(std::fs::read(&pdf).unwrap(), [9, 0, 9]);
    lib.undo();
    assert_eq!(std::fs::read(&pdf).unwrap(), [1, 0, 2]);
    lib.redo();
    assert_eq!(std::fs::read(&pdf).unwrap(), [9, 0, 9]);
    lib.restore_version(&note, b"alt".to_vec());
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "alt");
    lib.undo();
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "neu");
}

#[test]
fn renaming_keeps_links_alive_and_undo_takes_all_of_it_back() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let root = dunce::canonicalize(folder("relink")).unwrap();
    std::fs::write(root.join("schrumbi.json"), config(1, 1).to_string()).unwrap();
    std::fs::write(root.join("Alt.md"), "Ziel").unwrap();
    std::fs::write(root.join("Quelle.md"), "Siehe [[Alt#H]] und [x](Alt.md)\n").unwrap();
    std::fs::write(root.join("Offen.md"), "[[Alt]]").unwrap();
    std::fs::write(root.join("Board.canvas"), r#"{"nodes":[{"id":"a","type":"file","file":"Alt.md","x":0,"y":0,"width":1,"height":1}],"edges":[]}"#).unwrap();
    let mut lib = open(&root);
    crate::core::vault::index::rebuild(&root, &[]);
    lib.buffer(&root.join("Offen.md"));
    lib.rename(&root.join("Alt.md"), "Neu.md");
    let read = |n: &str| std::fs::read_to_string(root.join(n)).unwrap();
    assert_eq!(read("Quelle.md"), "Siehe [[Neu#H]] und [x](Neu.md)\n");
    assert_eq!(read("Offen.md"), "[[Neu]]");
    assert!(read("Board.canvas").contains("Neu.md"));
    lib.undo();
    assert!(root.join("Alt.md").exists());
    assert_eq!(read("Quelle.md"), "Siehe [[Alt#H]] und [x](Alt.md)\n");
    assert_eq!(read("Offen.md"), "[[Alt]]");
    assert!(read("Board.canvas").contains("Alt.md"));
    lib.redo();
    assert_eq!(read("Offen.md"), "[[Neu]]");
}

pub(crate) static VAULT_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn vault_folder(name: &str) -> (PathBuf, Library) {
    let root = dunce::canonicalize(folder(name)).unwrap();
    std::fs::write(root.join("schrumbi.json"), config(1, 1).to_string()).unwrap();
    std::fs::create_dir_all(root.join(".obsidian")).unwrap();
    std::fs::create_dir_all(root.join("Vorlagen")).unwrap();
    std::fs::write(root.join(".obsidian/daily-notes.json"), r#"{"folder":"Tage","format":"YYYY-MM-DD","template":"Vorlagen/Tag"}"#).unwrap();
    std::fs::write(root.join(".obsidian/templates.json"), r#"{"folder":"Vorlagen","dateFormat":"YYYY","timeFormat":"HH"}"#).unwrap();
    std::fs::write(root.join(".obsidian/zk-prefixer.json"), r#"{"folder":"Zettel","format":"YYYY"}"#).unwrap();
    std::fs::write(root.join("Vorlagen/Tag.md"), "# {{title}}\n{{date}}\n").unwrap();
    std::fs::write(root.join("Vorlagen/Mitschrift.md"), "# Mitschrift {{title}}\n").unwrap();
    let lib = open(&root);
    (root, lib)
}

#[test]
fn daily_and_unique_notes_follow_the_vault_settings_once() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault_folder("daily");
    lib.open_daily_note();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let year = chrono::Local::now().format("%Y").to_string();
    let path = root.join("Tage").join(format!("{today}.md"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("# {today}\n{year}\n"));
    std::fs::write(&path, "Eigener Text").unwrap();
    lib.open_daily_note();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Eigener Text", "an existing daily note stays");
    lib.open_unique_note();
    assert!(root.join("Zettel").join(format!("{year}.md")).exists());
    lib.open_unique_note();
    assert_eq!(std::fs::read_dir(root.join("Zettel")).unwrap().count(), 2, "a second one gets a free name");
}

#[test]
fn new_notes_use_the_vaults_note_template() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault_folder("newnote");
    let path = lib.create_note("Radar", &root, "md").unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "# Mitschrift Radar\n");
}

#[test]
fn templates_extracting_and_merging_undo_as_one_step() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault_folder("composer");
    let read = |n: &str| std::fs::read_to_string(root.join(n)).unwrap();
    std::fs::write(root.join("A.md"), "Eins\nZwei\n").unwrap();
    std::fs::write(root.join("Ziel.md"), "Ziel\n").unwrap();
    std::fs::write(root.join("Quelle.md"), "Siehe [[A]]\n").unwrap();
    crate::core::vault::index::rebuild(&root, &[]);

    lib.insert_template(&root.join("A.md"), &root.join("Vorlagen/Mitschrift.md"));
    assert_eq!(read("A.md"), "Eins\nZwei\n# Mitschrift A\n");
    lib.undo();
    assert_eq!(read("A.md"), "Eins\nZwei\n");

    let made = lib.extract_to_note(&root.join("A.md"), 5..9).unwrap();
    assert_eq!(made.file_name().unwrap(), "Zwei.md");
    assert_eq!(read("Zwei.md"), "Zwei");
    assert_eq!(read("A.md"), "Eins\n[[Zwei]]\n");
    lib.undo();
    assert_eq!(read("A.md"), "Eins\nZwei\n");
    assert!(!root.join("Zwei.md").exists());

    crate::core::vault::index::rebuild(&root, &[]);
    lib.merge_note(&root.join("A.md"), &root.join("Ziel.md"));
    assert_eq!(read("Ziel.md"), "Ziel\n\nEins\nZwei\n");
    assert!(!root.join("A.md").exists());
    assert!(read("Quelle.md").contains("Ziel"), "{}", read("Quelle.md"));
    lib.undo();
    assert_eq!(read("A.md"), "Eins\nZwei\n");
    assert_eq!(read("Ziel.md"), "Ziel\n");
    assert_eq!(read("Quelle.md"), "Siehe [[A]]\n");
}

#[test]
fn a_vault_outside_the_workspace_is_searched_and_filed_under_its_module() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let root = dunce::canonicalize(folder("extern-root")).unwrap();
    let vault = dunce::canonicalize(folder("extern-vault")).unwrap();
    std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
    std::fs::create_dir_all(vault.join("Notizen/Mathe")).unwrap();
    std::fs::write(vault.join("Notizen/Mathe/Satz.md"), "Pythagoras [[Beweis]]").unwrap();
    std::fs::write(vault.join("Notizen/Mathe/Beweis.md"), "Text").unwrap();
    std::fs::write(root.join("Eigen.md"), "Im Workspace").unwrap();
    std::fs::write(root.join("schrumbi.json"), config(1, 1).to_string()).unwrap();
    let mut study: crate::core::model::StudyConfig = serde_json::from_value(config(1, 1)).unwrap();
    study.modules[0].notes_folder = Some("Notizen/Mathe".to_string());
    let module = study.modules[0].id.clone();
    let index = std::sync::Arc::new(crate::core::search_index::SearchIndex::open_at(&vault.join("index.sqlite")));
    let job = crate::core::library::IndexJob { index: index.clone(), root: root.clone(), exclude: Vec::new(), config: study, vault: Some(vault.clone()) };
    let (count, catalog) = job.run();
    assert!(count >= 3);
    let note = catalog.iter().find(|f| f.file.path == vault.join("Notizen/Mathe/Satz.md")).expect("vault note is in the catalog");
    assert_eq!(note.module_id.as_deref(), Some(module.as_str()));
    assert!(catalog.iter().any(|f| f.file.path == root.join("Eigen.md")), "workspace files stay known");
    let vault_index = crate::core::vault::index::current();
    assert_eq!(vault_index.root, vault);
    assert!(vault_index.backlinks.contains_key(&vault.join("Notizen/Mathe/Beweis.md")));
    assert!(index.content_search(&["pythagoras".to_string()], None, 10).iter().any(|h| h.file.path == vault.join("Notizen/Mathe/Satz.md")));
}

#[test]
fn a_change_from_another_app_is_never_overwritten_by_an_edit_here() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault_folder("stale");
    let note = root.join("N.md");
    std::fs::write(&note, "---\nstatus: alt\n---\n- [ ] Aufgabe\n").unwrap();
    lib.buffer(&note);
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&note, "---\nstatus: alt\n---\n- [ ] Aufgabe\nAus Obsidian\n").unwrap();
    assert!(lib.edit_note(&note, "task", "x".into(), |t| crate::core::markdown::toggle_task(t, 3)));
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "---\nstatus: alt\n---\n- [x] Aufgabe\nAus Obsidian\n");
    lib.undo();
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "---\nstatus: alt\n---\n- [ ] Aufgabe\nAus Obsidian\n");
    lib.buffer(&note).edited("Hier ungesichert".to_string());
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&note, "Dort gesichert\n").unwrap();
    assert!(!lib.edit_note(&note, "task", "x".into(), |t| format!("{t}!")));
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "Dort gesichert\n");
}

#[test]
fn a_tab_follows_a_rename_made_in_another_app() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault_folder("follow");
    let (old, new) = (root.join("Alt.md"), root.join("Vorlagen/Neu.md"));
    std::fs::write(&old, "Inhalt").unwrap();
    std::fs::write(root.join("Anders.md"), "Anderer Inhalt, andere Größe").unwrap();
    lib.open_document(&old);
    lib.buffer(&old);
    assert!(!lib.follow_outside_moves());
    std::fs::rename(&old, &new).unwrap();
    assert!(lib.follow_outside_moves());
    assert_eq!(lib.active_document.as_ref(), Some(&new));
    assert_eq!(lib.open_documents, vec![new.clone()]);
    assert_eq!(lib.buffer(&new).text, "Inhalt");
    std::fs::remove_file(&new).unwrap();
    assert!(!lib.follow_outside_moves(), "a deleted file has nowhere to go");
    assert_eq!(lib.open_documents, vec![new]);
}

#[test]
fn ticking_a_task_from_the_overview_finds_it_after_the_note_moved_on() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault_folder("tasks");
    let note = root.join("T.md");
    let read = || std::fs::read_to_string(&note).unwrap();
    std::fs::write(&note, "- [ ] Eins\n- [ ] Zwei\n- [ ] Zwei\n").unwrap();
    lib.buffer(&note);
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&note, "Neue Zeile aus Obsidian\n- [ ] Eins\n- [ ] Zwei\n- [ ] Zwei\n").unwrap();
    assert!(lib.finish_task(&note, 0, "Eins"));
    assert_eq!(read(), "Neue Zeile aus Obsidian\n- [x] Eins\n- [ ] Zwei\n- [ ] Zwei\n");
    lib.undo();
    assert_eq!(read(), "Neue Zeile aus Obsidian\n- [ ] Eins\n- [ ] Zwei\n- [ ] Zwei\n");
    assert!(!lib.finish_task(&note, 0, "Zwei"), "two equal tasks and a stale line: touch nothing");
    assert!(lib.finish_task(&note, 3, "Zwei"), "the line still fits");
    assert_eq!(read(), "Neue Zeile aus Obsidian\n- [ ] Eins\n- [ ] Zwei\n- [x] Zwei\n");
    assert!(!lib.finish_task(&note, 1, "Gibt es nicht"));
}

#[test]
fn a_note_made_here_takes_the_name_of_its_heading() {
    use crate::core::markdown::{first_heading, with_first_heading};
    assert_eq!(first_heading("---\na: 1\n---\n\n```\n# nein\n```\n\n## klein\n\n# Die **echte** `H1` [[a|b]]\n# zweite").as_deref(), Some("Die echte H1 b"));
    assert_eq!(first_heading("Titel\n===\n").as_deref(), Some("Titel"));
    assert_eq!(first_heading("nur Text\n"), None);
    assert_eq!(with_first_heading("---\na: 1\n---\n# Alt\n\nText", "Neu"), "---\na: 1\n---\n# Neu\n\nText");
    assert_eq!(with_first_heading("Alt\n===\n\nText", "Neu"), "# Neu\n\nText");
    assert_eq!(with_first_heading("Text", "Neu"), "# Neu\n\nText");
    assert_eq!(crate::core::text::safe_file_stem(" a/b: c? . "), "a b c");
}

#[test]
fn existing_configs_keep_organizing() {
    let (root, lib) = workspace("organize-default", &config(1, 1));
    let c = lib.config.unwrap();
    assert!(c.rename_on_filing && c.create_folders);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_file_with_its_own_name_is_ready_and_never_flagged() {
    let (root, mut lib) = workspace("organize-checks", &config(1, 1));
    let a = lib.assignments.first().cloned().unwrap();
    std::fs::create_dir_all(&a.folder).unwrap();
    std::fs::write(a.folder.join("Mein Entwurf.pdf"), "x").unwrap();
    if let Some(job) = lib.scan_job() {
        lib.apply_scan(job.run());
    }
    assert_eq!(lib.evaluation(&a).status, crate::core::model::Status::Ready);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn own_structure_files_only_into_existing_folders() {
    let (root, mut lib) = workspace("organize-filing", &config(1, 1));
    let a = lib.assignments.first().cloned().unwrap();
    assert_eq!(lib.filing_folder(&a), Some(a.folder.clone()));
    let mut c = lib.config.clone().unwrap();
    c.create_folders = false;
    lib.update_config(c);
    assert_eq!(lib.filing_folder(&a), None);
    std::fs::create_dir_all(root.join("M0")).unwrap();
    assert_eq!(lib.filing_folder(&a), Some(root.join("M0")));
    let week = a.folder.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&week).unwrap();
    assert_eq!(lib.filing_folder(&a), Some(week));
    std::fs::create_dir_all(&a.folder).unwrap();
    assert_eq!(lib.filing_folder(&a), Some(a.folder.clone()));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn filing_a_file_where_it_already_is_keeps_its_name() {
    let (root, mut lib) = workspace("organize-same-place", &config(1, 1));
    std::fs::create_dir_all(root.join("M0")).unwrap();
    let file = root.join("M0/Entwurf.pdf");
    std::fs::write(&file, "x").unwrap();
    assert_eq!(lib.file(&file, &root.join("M0"), "Entwurf.pdf", true), Some(file.clone()));
    assert!(file.exists() && !root.join("M0/Entwurf-2.pdf").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_file_filed_into_a_fallback_folder_still_counts_and_undo_takes_both_back() {
    let (root, mut lib) = workspace("organize-fallback", &config(1, 1));
    let mut c = lib.config.clone().unwrap();
    (c.rename_on_filing, c.create_folders) = (false, false);
    lib.update_config(c);
    std::fs::create_dir_all(root.join("M0")).unwrap();
    let a = lib.assignments.first().cloned().unwrap();
    let source = root.join("Poster.pdf");
    std::fs::write(&source, "x").unwrap();
    let folder = lib.filing_folder(&a).unwrap();
    assert_eq!(folder, root.join("M0"));
    let target = lib.file_for(&source, &folder, "Poster.pdf", Some(&a.id), true).unwrap();
    assert!(lib.is_linked(&target, &a.id));
    if let Some(job) = lib.scan_job() {
        lib.apply_scan(job.run());
    }
    assert!(lib.evaluation(&a).files.contains(&target));
    lib.undo();
    assert!(source.exists() && !target.exists());
    assert!(!lib.is_linked(&target, &a.id));
    assert!(!a.folder.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn older_settings_read_notes_in_full_width() {
    let old: super::prefs::Prefs = serde_json::from_value(json!({ "spellCheck": false, "editorFontSize": 15.0 })).unwrap();
    assert!(old.reader_full_width);
    assert!(super::prefs::Prefs::default().reader_full_width);
    let off: super::prefs::Prefs = serde_json::from_value(json!({ "readerFullWidth": false })).unwrap();
    assert!(!off.reader_full_width);
}

#[test]
fn details_follow_the_page_and_the_selection() {
    let (root, mut lib) = workspace("inspector", &config(2, 1));
    let a = lib.assignments.iter().find(|a| a.module_id == "m0").unwrap().id.clone();
    let b = lib.assignments.iter().find(|a| a.module_id == "m1").unwrap().id.clone();
    lib.set_route(Route::Today);
    assert!(!lib.show_inspector());
    lib.select(Some(a.clone()));
    assert!(lib.show_inspector() && lib.is_selected(&a));
    lib.dismiss_inspector();
    assert!(!lib.show_inspector() && lib.selection().is_none());

    lib.select(Some(a.clone()));
    lib.set_route(Route::Cards);
    assert!(!lib.show_inspector() && lib.selection().is_none(), "the selection stays on its page");
    lib.toggle_inspector();
    assert!(!lib.show_inspector(), "a page without details cannot open them");
    lib.set_route(Route::Notes);
    lib.set_route(Route::Today);
    assert_eq!(lib.selection(), Some(a.as_str()), "coming back brings the selection back");
    assert!(lib.show_inspector());

    lib.set_route(Route::Grid);
    lib.select(Some(b.clone()));
    lib.set_route(Route::Module("m0".into()));
    assert!(lib.selection().is_none() && !lib.show_inspector());
    lib.reveal(&b);
    assert_eq!(*lib.route(), Route::Module("m1".into()));
    assert!(lib.is_selected(&b));
    lib.set_route(Route::Search);
    lib.reveal(&a);
    assert_eq!(*lib.route(), Route::Module("m0".into()));
    assert!(lib.is_selected(&a) && lib.show_inspector());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn kept_open_details_survive_esc_and_are_per_kind() {
    let (root, mut lib) = workspace("inspector-pinned", &config(1, 1));
    let a = lib.assignments[0].id.clone();
    lib.set_route(Route::Grid);
    lib.toggle_inspector();
    assert!(lib.show_inspector());
    lib.select(Some(a.clone()));
    lib.dismiss_inspector();
    assert!(lib.show_inspector() && lib.selection().is_none(), "Esc lets go of the selection but kept-open details stay");
    lib.set_route(Route::Graph);
    assert!(!lib.show_inspector(), "the graph remembers its own choice");
    lib.graph_selection = Some(root.join("n.md"));
    assert!(lib.show_inspector());
    lib.dismiss_inspector();
    assert!(!lib.show_inspector());
    lib.set_route(Route::Doc(root.join("n.md")));
    assert!(!lib.show_inspector());
    lib.peek_inspector();
    assert!(lib.show_inspector());
    lib.set_route(Route::Today);
    assert!(lib.show_inspector(), "deadline pages stay open as chosen");
    lib.select(Some(a));
    lib.toggle_inspector();
    assert!(!lib.show_inspector() && lib.selection().is_none(), "closing lets go of the selection");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cards_taken_as_known_undo_and_redo_in_order_with_the_rest() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (root, mut lib) = workspace("known-history", &config(1, 1));
    std::fs::write(root.join("Alt.md"), "Alt? :: a\n").unwrap();
    let index = |lib: &mut Library| {
        let (count, catalog) = lib.index_job().unwrap().run();
        lib.apply_index(count, catalog);
    };
    let phase = |front: &str| super::cards::all_cards().iter().find(|c| c.raw.front == front).map(super::cards::Card::phase).unwrap();
    let message = |lib: &Library| lib.toast.as_ref().map(|t| t.message.clone()).unwrap_or_default();
    index(&mut lib);
    assert_eq!(phase("Alt?"), super::cards::Phase::Review);
    assert!(message(&lib).starts_with("1 Karte als bekannt übernommen"));
    lib.undo();
    assert_eq!(phase("Alt?"), super::cards::Phase::New, "the first open's baseline can be taken back");
    assert_eq!(message(&lib), "Rückgängig: 1 Karte als bekannt übernommen");
    lib.redo();
    assert_eq!(phase("Alt?"), super::cards::Phase::Review);
    assert_eq!(message(&lib), "Wiederholt: 1 Karte als bekannt übernommen");

    let id = lib.assignments[0].id.clone();
    lib.toggle_done(&id);
    std::fs::write(root.join("Neu.md"), "Neu? :: n\n").unwrap();
    index(&mut lib);
    assert_eq!(phase("Neu?"), super::cards::Phase::New, "cards written after the first open are new");
    let done = super::cards::adopt_new_as_known("", super::cards::now()).unwrap();
    lib.cards_known(done);
    assert_eq!(phase("Neu?"), super::cards::Phase::Review);
    lib.undo();
    assert_eq!(phase("Neu?"), super::cards::Phase::New);
    assert!(lib.state.overrides.contains_key(&id), "undo takes back the latest step, not the one before");
    lib.redo();
    assert_eq!(phase("Neu?"), super::cards::Phase::Review);
    lib.undo();
    lib.undo();
    assert!(!lib.state.overrides.contains_key(&id));
    assert_eq!(phase("Neu?"), super::cards::Phase::New);

    let done = super::cards::adopt_new_as_known("", super::cards::now()).unwrap();
    lib.cards_known(done);
    assert!(lib.edit_note(&root.join("Alt.md"), "note", "Getippt".into(), |t| format!("{t}Text\n")));
    lib.perform_toast_action();
    assert_eq!(phase("Neu?"), super::cards::Phase::Review, "a toast does not take back a later edit");
    assert!(std::fs::read_to_string(root.join("Alt.md")).unwrap().ends_with("Text\n"));
    lib.undo();
    lib.undo();
    assert_eq!(phase("Neu?"), super::cards::Phase::New);

    let neu = super::cards::all_cards().iter().find(|c| c.raw.front == "Neu?").cloned().unwrap();
    let saved = super::cards::snapshot(&[neu.id().to_string()]);
    super::cards::set_suspended(&neu, true);
    lib.cards_changed("Karte pausiert".into(), "pause", super::cards::since(saved));
    lib.undo();
    assert!(!super::cards::all_cards().iter().any(|c| c.raw.front == "Neu?" && c.suspended()));
    lib.redo();
    assert!(super::cards::all_cards().iter().any(|c| c.raw.front == "Neu?" && c.suspended()));
    super::cards::set_suspended(&neu, false);
    lib.last_error = None;
    lib.undo();
    assert!(lib.last_error.is_some(), "an undo that changes nothing says so");
    assert!(!message(&lib).starts_with("Rückgängig"));
    assert!(!lib.can_redo(), "nothing to repeat when nothing was taken back");
    super::cards::close();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn first_open_baseline_stays_out_of_a_history_already_in_use() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (root, mut lib) = workspace("known-late", &config(1, 1));
    std::fs::write(root.join("Alt.md"), "Alt? :: a\n").unwrap();
    let id = lib.assignments[0].id.clone();
    lib.toggle_done(&id);
    let (count, catalog) = lib.index_job().unwrap().run();
    lib.apply_index(count, catalog);
    let phase = || super::cards::all_cards().iter().find(|c| c.raw.front == "Alt?").map(super::cards::Card::phase).unwrap();
    assert_eq!(phase(), super::cards::Phase::Review);
    assert!(lib.toast.as_ref().is_some_and(|t| t.message.starts_with("1 Karte als bekannt übernommen")));
    lib.undo();
    assert!(!lib.state.overrides.contains_key(&id), "Cmd+Z still takes back what the user did");
    assert_eq!(phase(), super::cards::Phase::Review);
    assert!(lib.can_redo());
    super::cards::close();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn taking_back_a_cards_import_removes_its_notes_and_what_was_taken_as_known() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (root, mut lib) = workspace("import-history", &config(1, 1));
    let (count, catalog) = lib.index_job().unwrap().run();
    lib.apply_index(count, catalog);
    let note = root.join("Anki/Deck.md");
    std::fs::create_dir_all(note.parent().unwrap()).unwrap();
    std::fs::write(&note, "F1 :: B1\n").unwrap();
    let fresh = vec![("n1".to_string(), "Anki/Deck.md".to_string(), "F1".to_string())];
    let spread = super::cards::adopt_import(Vec::new(), Vec::new(), fresh, super::cards::now());
    assert!(super::cards::snapshot(&["n1".to_string()])[0].1.is_some());
    lib.cards_imported(vec![note.clone()], 1, 0, spread);
    assert!(lib.toast.as_ref().unwrap().message.starts_with("1 Karte importiert · "));
    lib.undo();
    assert!(!note.exists());
    assert_eq!(super::cards::snapshot(&["n1".to_string()])[0].1, None);
    lib.redo();
    assert!(note.exists());
    assert!(super::cards::snapshot(&["n1".to_string()])[0].1.is_some());
    super::cards::close();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn names_stay_as_typed_and_take_the_convention_only_when_asked() {
    let (root, mut lib) = workspace("convention-heading", &config(1, 1));
    let today = chrono::Local::now().date_naive();
    let a = lib.assignments.iter().filter(|a| a.upload && crate::core::day::local_date(a.due) >= today).min_by_key(|a| a.due).cloned().unwrap();
    std::fs::create_dir_all(&a.folder).unwrap();
    let naming = lib.naming(&a.module_id).unwrap();
    let read = |p: &Path| std::fs::read_to_string(p).unwrap();

    let made = lib.create_note("TEST", &a.folder, "md").unwrap();
    assert_eq!(name_of(&made), "TEST.md", "a new note in a checked folder keeps the name typed");
    std::fs::write(&made, "# Neuer Titel

Text
").unwrap();
    lib.follow_heading(&made);
    let titled = a.folder.join("Neuer Titel.md");
    assert!(titled.exists(), "it still takes the name of its heading");
    lib.rename(&titled, "Anders.md");
    let typed = a.folder.join("Anders.md");
    assert!(typed.exists(), "a typed name stays as typed");
    assert!(read(&typed).starts_with("# Anders
"), "{}", read(&typed));

    let plan = lib.convention_plan(&typed).expect("the folder has a convention");
    let wanted = naming.render("Anders", today, "md", &[], a.week, a.iso_week, None);
    assert_eq!(plan.name(&typed), wanted, "the suggestion takes the heading as title");
    assert!(plan.fits(&wanted) && !plan.fits("Anders.md"));
    assert_eq!(plan.module, lib.config.as_ref().unwrap().module(&a.module_id).unwrap().code);
    lib.rename(&typed, &wanted);
    let conventional = a.folder.join(&wanted);
    assert!(conventional.exists() && !typed.exists());
    assert!(read(&conventional).starts_with("# Anders
"), "the heading stays");

    std::fs::write(&conventional, "# Dritter
").unwrap();
    lib.follow_heading(&conventional);
    assert!(conventional.exists(), "a convention name no longer follows the heading");
    lib.undo();
    assert!(typed.exists() && !conventional.exists(), "undo brings the typed name back");

    let off = format!("{}_Beispiel.md", crate::core::naming::stem(&wanted));
    assert!(!plan.fits(&off));
    let before = read(&typed);
    lib.rename_keeping_heading(&typed, &off);
    let picked = a.folder.join(&off);
    assert_eq!(read(&picked), before, "a name picked by hand leaves the heading alone");
    let upper = off.to_uppercase();
    lib.rename_keeping_heading(&picked, &upper);
    let names: Vec<String> = std::fs::read_dir(&a.folder).unwrap().map(|e| name_of(&e.unwrap().path())).collect();
    assert!(names.contains(&upper) && names.len() == 1, "a case-only rename keeps the name: {names:?}");

    let sheet = a.folder.join(lib.brief_naming(&a.module_id).unwrap().render("Blatt 3", today, "pdf", &[], a.week, a.iso_week, Some(crate::core::filing::BRIEF_TAG)));
    std::fs::write(&sheet, b"pdf").unwrap();
    let brief = lib.convention_plan(&sheet).unwrap();
    assert!(brief.fits(&brief.name(&sheet)), "a task sheet fits its own suggestion");

    let elsewhere = lib.create_note("Lose", &root, "md").unwrap();
    assert!(lib.convention_plan(&elsewhere).is_none(), "outside a module there is nothing to suggest");
    lib.config.as_mut().unwrap().set_naming_convention(false);
    assert!(lib.convention_plan(&typed).is_none(), "without a convention neither");
}

fn name_of(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().to_string()
}

#[test]
fn trashed_files_leave_the_lists_before_the_slow_part_and_come_back_on_undo() {
    let (root, mut lib) = workspace("trash-later", &config(1, 1));
    let inbox = root.join("_Inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    let file = inbox.join("Blatt.pdf");
    std::fs::write(&file, b"pdf").unwrap();
    let rescan = |lib: &mut Library| {
        let job = lib.scan_job().unwrap();
        lib.apply_scan(job.run());
    };
    rescan(&mut lib);
    assert!(lib.inbox.contains(&file));

    lib.trashing(std::slice::from_ref(&file));
    assert!(!lib.inbox.contains(&file), "gone from the list at once");
    assert!(file.exists(), "the file itself goes later, off the main thread");
    rescan(&mut lib);
    assert!(!lib.inbox.contains(&file), "a scan in between does not bring it back");

    let results = crate::core::library::trash_files(std::slice::from_ref(&file));
    lib.trashed(results);
    assert!(!file.exists());
    lib.undo();
    assert_eq!(std::fs::read(&file).unwrap(), b"pdf");
    rescan(&mut lib);
    assert!(lib.inbox.contains(&file), "listed again after undo");
}

#[test]
fn a_task_sheet_note_keeps_its_role_when_the_heading_is_followed() {
    let (_root, mut lib) = workspace("convention-brief", &config(1, 1));
    let today = chrono::Local::now().date_naive();
    let a = lib.assignments.iter().filter(|a| a.upload && crate::core::day::local_date(a.due) >= today).min_by_key(|a| a.due).cloned().unwrap();
    std::fs::create_dir_all(&a.folder).unwrap();
    let made = lib.create_note("Blatt", &a.folder, "md").unwrap();
    lib.set_role(crate::core::filing::FileRole::Brief, &made, &a.id);
    let brief = lib.last_renamed.clone().unwrap();
    assert!(brief.exists() && brief != made);
    assert!(std::fs::read_to_string(&brief).unwrap().starts_with("# Blatt\n"));
    lib.follow_heading(&brief);
    assert!(brief.exists(), "following the heading keeps the task sheet name");
}

#[test]
fn undo_waits_for_a_running_trash_and_then_takes_it_back() {
    let (root, mut lib) = workspace("trash-undo", &config(1, 1));
    let made = lib.create_note("Eins", &root, "md").unwrap();
    let file = root.join("Zwei.md");
    std::fs::write(&file, "# Zwei\n").unwrap();
    lib.open_document(&made);
    let going = lib.trashing(&[made.clone(), file.clone()]);
    assert_eq!(going.len(), 2);
    assert!(!lib.open_documents.contains(&made), "its tab closes at once");
    lib.undo();
    assert!(made.exists(), "the step before the trash stays");
    lib.trashed(crate::core::library::trash_files(&going));
    assert!(!made.exists() && !file.exists());
    lib.undo();
    assert!(made.exists() && file.exists(), "the trash itself is taken back");
    lib.undo();
    assert!(!made.exists(), "and only then the step before it");
}

#[test]
fn trashed_notes_stay_out_of_the_tree_until_a_later_scan() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    use crate::core::vault::index;
    let root = dunce::canonicalize(folder("hidden-tree")).unwrap();
    let (gone, kept) = (root.join("Weg.md"), root.join("Bleibt.md"));
    std::fs::write(&gone, "# Weg\n").unwrap();
    std::fs::write(&kept, "# Bleibt\n").unwrap();
    index::rebuild(&root, &[]);
    let started = std::time::Instant::now();
    index::hide(&[gone.clone(), kept.clone()]);
    assert!(started.elapsed() < std::time::Duration::from_millis(5), "hiding does not touch the index");
    assert!(index::hidden().hides(&gone) && index::hidden().hides(&kept));
    std::fs::remove_file(&gone).unwrap();
    index::trash_ended(std::slice::from_ref(&gone), std::slice::from_ref(&kept));
    assert!(!index::hidden().hides(&kept), "a failed trash shows the file again at once");
    assert!(index::hidden().hides(&gone), "a trashed file stays hidden while the index still lists it");
    index::rebuild(&root, &[]);
    assert!(!index::hidden().hides(&gone), "a scan after the trash ends the hiding");
    assert!(!index::current().files.contains(&gone));
    let sub = root.join("Ordner");
    std::fs::create_dir_all(&sub).unwrap();
    index::hide(std::slice::from_ref(&sub));
    assert!(index::hidden().hides(&sub.join("Tief/Notiz.md")), "a hidden folder hides what lies below it");
    index::trash_ended(&[], std::slice::from_ref(&sub));
    assert!(!index::hidden().hides(&sub.join("Tief/Notiz.md")));
}

#[test]
fn new_workspaces_start_without_a_naming_convention_and_keep_that_after_a_reload() {
    let root = folder("naming-new");
    std::fs::create_dir_all(root.join("Mathe")).unwrap();
    let c = open(&root).config.unwrap();
    assert!(!c.naming_convention() && !c.rename_on_filing);
    assert!(c.create_folders, "folder help stays on");
    let raw: Value = serde_json::from_slice(&std::fs::read(root.join("schrumbi.json")).unwrap()).unwrap();
    assert_eq!(raw["renameOnFiling"], json!(false), "written out, so a reload does not fall back to the legacy default");
    assert!(!open(&root).config.unwrap().naming_convention());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn configs_from_before_the_switch_keep_their_convention() {
    let (root, lib) = workspace("naming-legacy", &config(1, 1));
    assert!(lib.config.as_ref().unwrap().naming_convention());
    let explicit = json!({ "version": 2, "name": "T", "renameOnFiling": false, "checkNames": false });
    let c: crate::core::model::StudyConfig = serde_json::from_value(explicit).unwrap();
    assert!(!c.naming_convention(), "a choice once written stays");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn without_a_convention_nothing_is_flagged_and_new_files_keep_plain_names() {
    let (root, mut lib) = workspace("naming-off", &config(1, 1));
    let today = chrono::Local::now().date_naive();
    let a = lib.assignments.iter().filter(|a| a.upload && crate::core::day::local_date(a.due) >= today).min_by_key(|a| a.due).cloned().unwrap();
    std::fs::create_dir_all(&a.folder).unwrap();
    std::fs::write(a.folder.join("Mein Entwurf.pdf"), "x").unwrap();
    let rescan = |lib: &mut Library| {
        if let Some(job) = lib.scan_job() {
            lib.apply_scan(job.run());
        }
    };
    rescan(&mut lib);
    let mut c = lib.config.clone().unwrap();
    c.set_naming_convention(false);
    lib.update_config(c.clone());
    rescan(&mut lib);
    assert_eq!(lib.evaluation(&a).status, crate::core::model::Status::Ready, "a plain name counts");
    let read = |p: &Path| std::fs::read_to_string(p).unwrap();

    let made = lib.create_note("TEST", &a.folder, "md").unwrap();
    assert_eq!(name_of(&made), "TEST.md", "a note in a checked folder keeps the plain name");
    assert_eq!(lib.note_name("Liste", "txt"), "Liste.txt", "other new files too");
    assert_eq!(lib.note_name("meine Übung", "txt"), "meine Übung.txt", "typed as typed");
    assert_eq!(lib.note_name("Frage? Was ist \"X\".", "txt"), "Frage Was ist X.txt", "nothing Windows refuses");

    std::fs::write(&made, "# Neuer Titel\n\nText\n").unwrap();
    lib.follow_heading(&made);
    let followed = a.folder.join("Neuer Titel.md");
    assert!(followed.exists(), "the name follows the heading as typed");

    lib.rename(&followed, "Anders.md");
    let typed = a.folder.join("Anders.md");
    assert!(typed.exists(), "a typed name stays as typed");
    assert!(read(&typed).starts_with("# Anders\n"), "{}", read(&typed));

    c.set_naming_convention(true);
    lib.update_config(c);
    rescan(&mut lib);
    assert!(typed.exists() && a.folder.join("Mein Entwurf.pdf").exists(), "switching never renames anything by itself");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn without_a_convention_a_role_only_adds_or_takes_out_the_task_word() {
    use crate::core::filing::FileRole::{self, Brief, Material, Work};
    use crate::core::library::plain_role_name;
    assert_eq!(plain_role_name("Mein Entwurf.pdf", Brief, "Muster-M"), "Mein Entwurf Aufgabe.pdf");
    assert_eq!(plain_role_name("Blatt 3 Muster-M.pdf", Brief, "Muster-M"), "Blatt 3 Aufgabe.pdf");
    assert_eq!(plain_role_name("Übungsblatt 3.pdf", Brief, "Muster-M"), "Übungsblatt 3.pdf", "already a task sheet");
    assert_eq!(plain_role_name("Muster-M.pdf", Brief, "Muster-M"), "Aufgabe.pdf");
    assert_eq!(plain_role_name("Blatt_3_Aufgabe.pdf", Work, "Muster-M"), "Blatt_3.pdf");
    assert_eq!(plain_role_name("Aufgabe.pdf", Work, ""), "Abgabe.pdf");
    assert_eq!(plain_role_name("Mein Entwurf.pdf", Work, "Muster-M"), "Mein Entwurf.pdf", "already counts as the submission");
    assert_eq!(plain_role_name("Aufgabe.pdf", Material, ""), "Aufgabe.pdf");
    assert_eq!(plain_role_name("Übungsblatt 3.pdf", Work, "Muster-M"), "Übungsblatt 3 Muster-M.pdf", "tagged as own work, nothing removed");
    assert_eq!(plain_role_name("Übungsblatt 3.pdf", Work, ""), "Übungsblatt 3.pdf");
    assert_eq!(plain_role_name("_Entwurf.pdf", Brief, "Muster-M"), "_Entwurf Aufgabe.pdf", "own separators stay");
    assert_eq!(plain_role_name("Entwurf_Muster-M_v2.pdf", Brief, "Muster-M"), "Entwurf_v2 Aufgabe.pdf");
    for (name, role) in [("Mein Entwurf.pdf", Brief), ("Blatt_3_Aufgabe.pdf", Work), ("Notiz", Brief), ("Übungsblatt 3.pdf", Work), ("Aufgabenstellung.pdf", Work)] {
        let new = plain_role_name(name, role, "Muster-M");
        assert_eq!(FileRole::is_brief(&new, "Muster-M"), role == Brief, "{name} → {new}");
    }

    let (_root, mut lib) = workspace("naming-off-role", &config(1, 1));
    let mut c = lib.config.clone().unwrap();
    c.set_naming_convention(false);
    lib.update_config(c);
    let today = chrono::Local::now().date_naive();
    let a = lib.assignments.iter().filter(|a| a.upload && crate::core::day::local_date(a.due) >= today).min_by_key(|a| a.due).cloned().unwrap();
    std::fs::create_dir_all(&a.folder).unwrap();
    let file = a.folder.join("Mein Entwurf.pdf");
    std::fs::write(&file, "x").unwrap();
    lib.set_role(Brief, &file, &a.id);
    assert_eq!(std::fs::read(a.folder.join("Mein Entwurf Aufgabe.pdf")).unwrap(), b"x");
    lib.set_role(Work, &a.folder.join("Mein Entwurf Aufgabe.pdf"), &a.id);
    assert_eq!(std::fs::read(&file).unwrap(), b"x", "back to the name it had");
}

#[test]
fn a_plain_role_name_takes_out_only_the_author_token_and_only_the_added_word() {
    use crate::core::filing::FileRole::{self, Brief, Work};
    use crate::core::library::plain_role_name;
    assert_eq!(plain_role_name("muster-m Entwurf.pdf", Brief, "Muster-M"), "Entwurf Aufgabe.pdf");
    assert_eq!(plain_role_name("Entwurf_MUSTER-M_v2.pdf", Brief, "Muster-M"), "Entwurf_v2 Aufgabe.pdf");
    let filed = plain_role_name("Blatt3.pdf", Brief, "Muster-M");
    assert_eq!(filed, "Blatt3 Aufgabe.pdf");
    assert!(FileRole::is_brief(&filed, "Muster-M"), "a task sheet filed without a convention counts as one");
    assert_eq!(plain_role_name("Blatt3.pdf", Work, "Muster-M"), "Blatt3.pdf");
    assert_eq!(plain_role_name("Maximierung.pdf", Brief, "Max"), "Maximierung.pdf", "a name that holds the author stays as it is");
    assert_eq!(plain_role_name("Max Entwurf.pdf", Brief, "Max"), "Entwurf Aufgabe.pdf");
    assert_eq!(plain_role_name("Lösung,Aufgabe 2.pdf", Work, "Muster-M"), "Lösung,Aufgabe 2 Muster-M.pdf");
    assert_eq!(plain_role_name("Blatt-3_Lösung Aufgabe.pdf", Work, "Muster-M"), "Blatt-3_Lösung.pdf");
    assert_eq!(plain_role_name("Aufgabenstellung Aufgabe.pdf", Work, ""), "Aufgabenstellung Aufgabe.pdf", "still reads as a brief");
    for name in ["Mein Entwurf.pdf", "Blatt-3_Lösung.pdf", "Notiz", "Übung 3.pdf"] {
        let brief = plain_role_name(name, Brief, "Muster-M");
        assert!(FileRole::is_brief(&brief, "Muster-M"), "{brief}");
        assert_eq!(plain_role_name(&brief, Work, "Muster-M"), name, "round trip");
    }
}

#[test]
fn the_convention_follows_renaming_on_filing_and_ignores_the_retired_name_check() {
    let mut c = crate::core::model::StudyConfig::default();
    assert!(!c.naming_convention());
    c.set_naming_convention(true);
    assert!(c.naming_convention() && c.rename_on_filing);
    let old: crate::core::model::StudyConfig = serde_json::from_str(r#"{"renameOnFiling":false,"checkNames":true}"#).unwrap();
    assert!(!old.naming_convention(), "a name check alone no longer switches the convention on");
}

/// One module whose single rule hands in to the shared `Abgaben` folder.
fn shared_config() -> Value {
    let mut c = config(1, 1);
    c["modules"][0]["rules"][0]["folder"] = json!("Abgaben");
    c
}

fn rescan(lib: &mut Library) {
    if let Some(job) = lib.scan_job() {
        lib.apply_scan(job.run());
    }
}

fn upcoming(lib: &Library) -> crate::core::model::Assignment {
    lib.assignments.iter().find(|a| a.due > lib.now).cloned().unwrap()
}

fn dated(a: &crate::core::model::Assignment, stem: &str) -> String {
    format!("{stem} {}.pdf", super::day::local_date(a.due).format("%Y-%m-%d"))
}

#[test]
fn in_a_shared_folder_only_linked_files_count_and_others_are_suggested() {
    let (root, mut lib) = workspace("links-only", &shared_config());
    let a = upcoming(&lib);
    assert_eq!(a.folder, root.join("M0/Abgaben"));
    std::fs::create_dir_all(&a.folder).unwrap();
    let file = a.folder.join(dated(&a, "Entwurf"));
    std::fs::write(&file, "x").unwrap();
    rescan(&mut lib);
    let e = lib.evaluation(&a);
    assert!(e.files.is_empty(), "a file in the shared folder does not count by its date");
    assert_eq!(e.suggested, vec![file.clone()]);
    lib.link(std::slice::from_ref(&file), &a.id);
    let e = lib.evaluation(&a);
    assert_eq!(e.files, vec![file.clone()]);
    assert!(e.suggested.is_empty());
    let other = lib.assignments.iter().find(|b| b.id != a.id && b.tracks_files() && b.due > a.due).cloned().unwrap();
    assert!(!lib.evaluation(&other).suggested.contains(&file), "a linked file is no suggestion for another deadline");
    lib.undo();
    assert!(lib.evaluation(&a).files.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn files_older_versions_found_by_date_are_adopted_once() {
    let root = folder("links-adopt");
    std::fs::write(root.join("schrumbi.json"), serde_json::to_vec_pretty(&shared_config()).unwrap()).unwrap();
    std::fs::write(root.join(".schrumbi-state.json"), "{}").unwrap();
    let mut lib = open(&root);
    let a = upcoming(&lib);
    let file = a.folder.join(dated(&a, "Entwurf"));
    drop(lib);
    std::fs::create_dir_all(&a.folder).unwrap();
    std::fs::write(&file, "x").unwrap();
    std::fs::write(root.join(".schrumbi-state.json"), "{}").unwrap();
    lib = open(&root);
    assert_eq!(lib.evaluation(&a).files, vec![file.clone()], "what counted before still counts");
    assert!(lib.is_linked(&file, &a.id));
    lib.unlink(&file, &a.id);
    drop(lib);
    let lib = open(&root);
    assert!(!lib.is_linked(&file, &a.id), "adopting happens only once");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_new_workspace_guesses_nothing() {
    let root = folder("links-fresh");
    std::fs::write(root.join("schrumbi.json"), serde_json::to_vec_pretty(&shared_config()).unwrap()).unwrap();
    let lib = open(&root);
    let a = upcoming(&lib);
    drop(lib);
    std::fs::create_dir_all(&a.folder).unwrap();
    std::fs::write(a.folder.join(dated(&a, "Entwurf")), "x").unwrap();
    let lib = open(&root);
    assert!(lib.evaluation(&a).files.is_empty());
    assert_eq!(lib.evaluation(&a).suggested.len(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_folder_per_week_still_counts_without_links() {
    let (root, mut lib) = workspace("links-per-week", &config(1, 1));
    let a = upcoming(&lib);
    assert!(a.owns_folder());
    std::fs::create_dir_all(&a.folder).unwrap();
    let file = a.folder.join("Irgendwas.pdf");
    std::fs::write(&file, "x").unwrap();
    rescan(&mut lib);
    assert_eq!(lib.evaluation(&a).files, vec![file]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn filing_for_a_deadline_goes_into_the_submissions_folder_and_links() {
    let (root, mut lib) = workspace("links-filing", &shared_config());
    let a = upcoming(&lib);
    let folder = lib.filing_folder(&a).unwrap();
    assert_eq!(folder, root.join("M0/Abgaben"));
    assert!(!folder.exists(), "the folder appears only when something goes into it");
    let source = root.join("Blatt.pdf");
    std::fs::write(&source, "x").unwrap();
    let target = lib.file_for(&source, &folder, "Blatt.pdf", Some(&a.id), true).unwrap();
    assert!(lib.is_linked(&target, &a.id));
    assert_eq!(lib.evaluation(&a).files, vec![target.clone()]);
    lib.undo();
    assert!(source.exists() && !target.exists() && !lib.is_linked(&target, &a.id));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_note_made_for_a_deadline_belongs_to_it_and_so_does_its_pdf() {
    let (root, mut lib) = workspace("links-note", &shared_config());
    let a = upcoming(&lib);
    let note = lib.create_assignment_note(&a.id).unwrap();
    assert!(note.starts_with(root.join("M0/Abgaben")));
    assert!(lib.is_linked(&note, &a.id));
    assert_eq!(lib.evaluation(&a).status, crate::core::model::Status::InProgress);
    assert_eq!(lib.deadlines_of(&note).first().map(|d| d.id.clone()), Some(a.id.clone()));
    let pdf = note.with_extension("pdf");
    std::fs::write(&pdf, "x").unwrap();
    lib.record_export(&pdf, None, &note);
    assert!(lib.is_linked(&pdf, &a.id));
    assert!(lib.evaluation(&a).files.contains(&pdf));
    lib.undo();
    assert!(!pdf.exists() && !lib.is_linked(&pdf, &a.id));
    lib.undo();
    assert!(!note.exists() && !lib.is_linked(&note, &a.id));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_pdf_exported_outside_the_workspace_belongs_to_nothing() {
    let (root, mut lib) = workspace("links-export-out", &shared_config());
    let a = upcoming(&lib);
    let note = lib.create_assignment_note(&a.id).unwrap();
    let outside = std::env::temp_dir().join("schrumbi-links-export-out.pdf");
    std::fs::write(&outside, "x").unwrap();
    let linked = lib.state.linked_files.clone();
    lib.record_export(&outside, None, &note);
    assert_eq!(lib.state.linked_files, linked);
    assert!(!lib.evaluation(&a).files.contains(&outside));
    let _ = std::fs::remove_file(outside);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn links_follow_a_rename_and_leave_with_the_trash() {
    let (root, mut lib) = workspace("links-move", &shared_config());
    let a = upcoming(&lib);
    let note = lib.create_assignment_note(&a.id).unwrap();
    lib.rename(&note, "Anders.md");
    let renamed = lib.evaluation(&a).files.first().cloned().unwrap();
    assert!(renamed != note && renamed.exists() && !note.exists());
    assert!(lib.is_linked(&renamed, &a.id) && !lib.is_linked(&note, &a.id));
    let going = lib.trashing(std::slice::from_ref(&renamed));
    let results = super::library::trash_files(&going);
    lib.trashed(results);
    assert!(!lib.is_linked(&renamed, &a.id));
    lib.undo();
    assert!(renamed.exists() && lib.is_linked(&renamed, &a.id), "undo brings back the file and its link");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_linked_file_gone_outside_the_app_is_reported_not_counted() {
    let (root, mut lib) = workspace("links-gone", &shared_config());
    let a = upcoming(&lib);
    std::fs::create_dir_all(&a.folder).unwrap();
    let file = a.folder.join("Blatt.pdf");
    std::fs::write(&file, "x").unwrap();
    lib.link(std::slice::from_ref(&file), &a.id);
    std::fs::remove_file(&file).unwrap();
    rescan(&mut lib);
    let e = lib.evaluation(&a);
    assert!(e.files.is_empty());
    assert_eq!(e.missing, vec![file]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn choices_for_a_file_are_its_modules_open_deadlines_with_the_linked_ones_checked() {
    let (root, mut lib) = workspace("links-choices", &shared_config());
    let a = upcoming(&lib);
    std::fs::create_dir_all(root.join("M0")).unwrap();
    let file = root.join("M0/Notiz.md");
    std::fs::write(&file, "x").unwrap();
    let choices = lib.link_choices(&file);
    assert!(!choices.is_empty() && choices.len() <= 8);
    assert!(choices.iter().all(|(_, on)| !on));
    assert!(choices.windows(2).all(|w| w[0].0.due <= w[1].0.due));
    lib.toggle_link(&file, &a.id);
    assert!(lib.link_choices(&file).iter().any(|(c, on)| c.id == a.id && *on));
    lib.toggle_link(&file, &a.id);
    assert!(!lib.is_linked(&file, &a.id));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn links_follow_a_rename_when_the_workspace_path_is_not_canonical() {
    let real = folder("links-detour");
    std::fs::create_dir_all(real.join("umweg")).unwrap();
    std::fs::write(real.join("schrumbi.json"), serde_json::to_vec_pretty(&shared_config()).unwrap()).unwrap();
    let root = real.join("umweg").join("..");
    let mut lib = open(&root);
    let a = upcoming(&lib);
    let note = lib.create_assignment_note(&a.id).unwrap();
    lib.rename(&note, "Anders.md");
    let renamed = note.with_file_name("Anders.md");
    assert!(renamed.exists() && lib.is_linked(&renamed, &a.id) && !lib.is_linked(&note, &a.id), "the link follows the rename");
    lib.open_document(&renamed);
    let tab = lib.active_document.clone().unwrap();
    assert_ne!(tab, renamed, "a tab spells the path resolved");
    assert!(lib.is_linked(&tab, &a.id));
    lib.rename(&tab, "Zweite.md");
    let second = note.with_file_name("Zweite.md");
    assert!(second.exists() && lib.is_linked(&second, &a.id), "renaming the open tab keeps the link");
    let modules = lib.config.as_ref().unwrap().modules.iter().map(|m| m.folder.clone()).collect::<Vec<_>>();
    let module = real.join(&modules[0]);
    lib.rename(&dunce::canonicalize(&module).unwrap(), "Kurs");
    assert_eq!(lib.config.as_ref().unwrap().modules[0].folder, "Kurs", "the module follows its folder renamed by its resolved path");
    let pdf = root.join("Kurs").join("Abgaben").join("Blatt.pdf");
    std::fs::write(&pdf, "x").unwrap();
    lib.link(&[dunce::canonicalize(&pdf).unwrap()], &a.id);
    assert!(lib.is_linked(&pdf, &a.id), "a file given by its resolved path can be linked");
    let _ = std::fs::remove_dir_all(real);
}

#[test]
fn links_inside_a_renamed_folder_follow_it() {
    let (root, mut lib) = workspace("links-folder", &shared_config());
    let a = upcoming(&lib);
    let note = lib.create_assignment_note(&a.id).unwrap();
    let folder = note.parent().unwrap().to_path_buf();
    lib.rename(&folder, "Abgaben WS");
    let moved = folder.with_file_name("Abgaben WS").join(note.file_name().unwrap());
    assert!(moved.exists());
    assert!(lib.is_linked(&moved, &a.id), "the link moves with its folder");
    lib.undo();
    assert!(note.exists() && lib.is_linked(&note, &a.id), "undo moves it back");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_workspace_from_before_links_without_a_state_file_keeps_what_counted() {
    let root = folder("links-no-state");
    std::fs::write(root.join("schrumbi.json"), serde_json::to_vec_pretty(&shared_config()).unwrap()).unwrap();
    let lib = open(&root);
    let a = upcoming(&lib);
    drop(lib);
    let _ = std::fs::remove_file(root.join(".schrumbi-state.json"));
    std::fs::create_dir_all(&a.folder).unwrap();
    let file = a.folder.join(dated(&a, "Entwurf"));
    std::fs::write(&file, "x").unwrap();
    let lib = open(&root);
    assert!(lib.is_linked(&file, &a.id));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_week_counts_what_the_semester_counts() {
    use crate::core::model::{Kind, ManualAssignment};
    let (_root, mut lib) = workspace("week-count", &config(1, 0));
    let due = chrono::Local::now().with_timezone(&chrono::Utc);
    for (kind, optional) in [(Kind::Task, false), (Kind::Exam, false), (Kind::Submission, true), (Kind::Submission, false)] {
        lib.add_manual(ManualAssignment { module: "m0".into(), title: format!("{kind:?}"), due, kind, optional, upload: kind == Kind::Submission, ..Default::default() });
    }
    assert_eq!(lib.week_progress(), (0, 2), "the task and the required submission, like the semester count");
}

#[test]
fn settings_written_before_the_cleanup_page_went_still_load() {
    let mut raw = config(1, 1);
    raw["checkFolders"] = json!(true);
    let (root, lib) = workspace("old-check-folders", &raw);
    assert!(lib.config.is_some(), "an old checkFolders key is ignored, not an error");
    let _ = std::fs::remove_dir_all(root);
}
