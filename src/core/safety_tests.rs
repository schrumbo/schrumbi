use std::path::{Path, PathBuf};

use super::edge_tests::VAULT_TESTS;
use super::library::Library;
use super::library::Route;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("schrumbi-safety-{}", std::process::id())).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dunce::canonicalize(&dir).unwrap()
}

fn open(root: &Path) -> Library {
    let mut lib = Library::new();
    lib.open(root);
    if let Some(job) = lib.scan_job() {
        lib.apply_scan(job.run());
    }
    lib
}

fn vault(name: &str) -> (PathBuf, Library) {
    let root = folder(name);
    std::fs::create_dir_all(root.join(".obsidian")).unwrap();
    let lib = open(&root);
    (root, lib)
}

fn outside(path: &Path, text: &str, seconds: u64) {
    std::fs::write(path, text).unwrap();
    let time = std::time::SystemTime::now() + std::time::Duration::from_secs(seconds);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(time).unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).filter(|n| !n.starts_with(".schrumbi") && n != "schrumbi.json").collect();
    names.sort();
    names
}

#[test]
fn renaming_a_link_target_keeps_the_other_apps_version_of_an_open_note() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault("relink-conflict");
    std::fs::write(root.join("B.md"), "Ziel").unwrap();
    std::fs::write(root.join("A.md"), "Siehe [[B]]\n").unwrap();
    super::vault::index::rebuild(&root, &[]);
    let a = root.join("A.md");
    lib.buffer(&a).edited("Siehe [[B]]\nMEIN neuer Absatz\n".into());
    outside(&a, "Siehe [[B]]\nFREMDER Absatz oben\nund mehr\n", 5);
    lib.rename(&root.join("B.md"), "C.md");
    let disk = std::fs::read_to_string(&a).unwrap();
    assert!(disk.contains("FREMDER"), "the other version was written over: {disk:?}");
    let buffer = lib.buffer(&a);
    assert!(buffer.conflict.is_some(), "the decision has to stay open");
    assert!(buffer.text.contains("MEIN"), "my unsaved text has to stay: {:?}", buffer.text);
    assert!(root.join("C.md").exists());
}

#[test]
fn undo_waits_while_another_apps_version_waits_for_a_decision() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault("undo-conflict");
    let a = root.join("A.md");
    std::fs::write(&a, "eins\nzwei\n").unwrap();
    super::vault::index::rebuild(&root, &[]);
    lib.buffer(&a);
    lib.edit_note(&a, "k", "Schritt".to_string(), |t| format!("{t}drei\n"));
    lib.buffer(&a).edited("eins\nzwei\ndrei\nMEIN vierter\n".into());
    outside(&a, "eins\nzwei\ndrei\nFREMD vierter\nund fuenf\n", 5);
    lib.buffer(&a).reload_if_changed();
    assert!(lib.buffer(&a).conflict.is_some());
    lib.undo();
    assert!(std::fs::read_to_string(&a).unwrap().contains("FREMD vierter"));
    let buffer = lib.buffer(&a);
    assert!(buffer.conflict.is_some() && buffer.text.contains("MEIN vierter"), "{:?}", buffer.text);
    assert!(lib.toast.as_ref().is_some_and(|t| t.message.contains("Entscheide")), "{:?}", lib.toast.as_ref().map(|t| &t.message));
    lib.keep_note_mine(&a);
    assert!(std::fs::read_to_string(&a).unwrap().contains("MEIN vierter"));
}

#[test]
fn a_note_whose_last_edits_cannot_be_saved_does_not_go_to_the_trash() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault("trash-unsaved");
    let a = root.join("A.md");
    std::fs::write(&a, "alt\n").unwrap();
    super::vault::index::rebuild(&root, &[]);
    lib.buffer(&a).edited("alt\nungesichert\n".into());
    lib.buffer(&a).hold();
    let going = lib.trashing(std::slice::from_ref(&a));
    assert!(going.is_empty(), "the note has to stay while its edits are not on disk");
    assert!(lib.last_error.is_some());
    assert!(a.exists());
    assert_eq!(names(&root), vec![".obsidian".to_string(), "A.md".to_string()]);
}

#[test]
fn names_windows_cannot_use_are_never_created_or_renamed_to() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault("names");
    for title in ["CON", "NUL", "Aux", "prn", "Com1", "...", "  ", "a."] {
        let path = lib.create_note(title, &root, "md").expect(title);
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(super::text::file_name_problem(&name), None, "{title:?} -> {name:?}");
        assert!(!name.starts_with('.'), "{title:?} -> {name:?}");
    }
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("r.md"), "x").unwrap();
    let r = root.join("sub").join("r.md");
    for bad in ["NUL.md", "x.md:ads", "..\\esc.md", "../esc.md", "C:\\b1evil.md", "/tmp/evil.md"] {
        assert!(lib.rename_checked(&r, bad).is_err(), "{bad:?}");
        assert!(r.exists(), "{bad:?}");
    }
    assert_eq!(names(&root.join("sub")), vec!["r.md".to_string()]);
    assert!(lib.create_folder("LPT1", &root).is_none());
    assert!(lib.file(&r, &root, "NUL", true).is_none());
    assert!(r.exists());
}

#[test]
fn renaming_a_folder_points_links_into_it_at_the_new_place_and_a_moved_note_keeps_its_own_links() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault("folder-links");
    std::fs::create_dir_all(root.join("A")).unwrap();
    std::fs::write(root.join("A/n.md"), "Text [x](../x.md) ![](pic.png) [[n]]\n").unwrap();
    std::fs::write(root.join("A/pic.png"), "p").unwrap();
    std::fs::write(root.join("x.md"), "X\n").unwrap();
    std::fs::write(root.join("q.md"), "[[A/n]] und [md](A/n.md) und [[n]]\n").unwrap();
    super::vault::index::rebuild(&root, &[]);
    lib.rename(&root.join("A"), "Bee");
    assert_eq!(lib.last_error, None);
    let q = std::fs::read_to_string(root.join("q.md")).unwrap();
    assert!(!q.contains("A/n"), "{q:?}");
    assert!(q.contains("[[n]]"), "a link that still works stays as it was: {q:?}");
    let resolver = |root: &Path| super::vault::index::rebuild(root, &[]);
    resolver(&root);
    let vault = super::vault::index::current();
    for link in super::vault::links::find_links(&q) {
        assert_eq!(vault.resolver.resolve(&link.target, Some(&root.join("q.md"))), Some(root.join("Bee/n.md")), "{q:?}");
    }
    std::fs::create_dir_all(root.join("Z/Y")).unwrap();
    super::vault::index::rebuild(&root, &[]);
    let moved = lib.file(&root.join("Bee/n.md"), &root.join("Z/Y"), "n.md", true).unwrap();
    let text = std::fs::read_to_string(&moved).unwrap();
    super::vault::index::rebuild(&root, &[]);
    let vault = super::vault::index::current();
    let reached: Vec<Option<PathBuf>> = super::vault::links::find_links(&text).iter().map(|l| vault.resolver.resolve(&l.target, Some(&moved))).collect();
    assert_eq!(reached, vec![Some(root.join("x.md")), Some(root.join("Bee/pic.png")), Some(moved.clone())], "{text:?}");
    lib.undo();
    assert_eq!(std::fs::read_to_string(root.join("Bee/n.md")).unwrap(), "Text [x](../x.md) ![](pic.png) [[n]]\n");
    lib.redo();
    assert_eq!(std::fs::read_to_string(&moved).unwrap(), text, "{:?}", lib.last_error);
    assert!(!root.join("Bee/n.md").exists());
    assert_eq!(names(&root.join("Z/Y")), vec!["n.md".to_string()]);
}

#[test]
fn undoing_a_rename_that_only_changes_case_gives_the_old_name_back() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault("case");
    std::fs::write(root.join("notiz.md"), "x").unwrap();
    super::vault::index::rebuild(&root, &[]);
    lib.rename(&root.join("notiz.md"), "Notiz.md");
    assert!(names(&root).contains(&"Notiz.md".to_string()));
    lib.undo();
    assert_eq!(names(&root), vec![".obsidian".to_string(), "notiz.md".to_string()], "{:?}", lib.last_error);
    lib.redo();
    assert_eq!(names(&root), vec![".obsidian".to_string(), "Notiz.md".to_string()], "{:?}", lib.last_error);
}

#[test]
fn a_move_never_replaces_a_file_that_appeared_at_the_target() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault("noreplace");
    std::fs::write(root.join("a.md"), "a").unwrap();
    super::vault::index::rebuild(&root, &[]);
    lib.rename(&root.join("a.md"), "b.md");
    std::fs::write(root.join("a.md"), "neu da").unwrap();
    lib.undo();
    assert_eq!(std::fs::read_to_string(root.join("a.md")).unwrap(), "neu da");
    assert!(names(&root).iter().any(|n| n.starts_with("a-") || n.starts_with("a ")) || root.join("b.md").exists(), "{:?}", names(&root));
}

#[cfg(windows)]
#[test]
fn renaming_a_folder_with_a_file_open_elsewhere_leaves_no_copy_behind() {
    use std::os::windows::fs::OpenOptionsExt;
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault("locked");
    let d = root.join("Vorlesung");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("a.pdf"), vec![1u8; 100_000]).unwrap();
    std::fs::write(d.join("b.pdf"), vec![2u8; 100]).unwrap();
    let held = std::fs::OpenOptions::new().read(true).share_mode(1 | 2).open(d.join("a.pdf")).unwrap();
    super::vault::index::rebuild(&root, &[]);
    lib.rename(&d, "Vorlesung neu");
    assert!(lib.last_error.is_some());
    assert_eq!(names(&root), vec![".obsidian".to_string(), "Vorlesung".to_string()]);
    assert_eq!(names(&d), vec!["a.pdf".to_string(), "b.pdf".to_string()]);
    drop(held);
}

#[cfg(windows)]
fn junction(link: &Path, target: &Path) {
    let out = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[cfg(windows)]
#[test]
fn trashing_a_folder_never_copies_what_a_link_inside_points_at() {
    let root = folder("junction");
    let big = root.join("big");
    std::fs::create_dir_all(&big).unwrap();
    std::fs::write(big.join("huge.bin"), vec![7u8; 100_000]).unwrap();
    let f = root.join("Ordner");
    std::fs::create_dir_all(&f).unwrap();
    std::fs::write(f.join("a.md"), "a").unwrap();
    junction(&f.join("link"), &big);
    junction(&f.join("loop"), &f);
    let results = super::library::trash_files(std::slice::from_ref(&f));
    match &results[0].1 {
        Ok(super::library::Change::Restore { backup, .. }) => {
            assert!(backup.join("a.md").exists());
            assert!(!backup.join("link").join("huge.bin").exists());
            assert!(!backup.join("loop").exists());
        }
        other => panic!("{other:?}"),
    }
    assert!(big.join("huge.bin").exists());
}

#[test]
fn a_file_too_large_for_a_copy_goes_to_the_trash_without_an_undo_step() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault("large");
    let big = root.join("video.mp4");
    std::fs::write(&big, vec![0u8; 2 << 20]).unwrap();
    super::vault::index::rebuild(&root, &[]);
    let going = lib.trashing(std::slice::from_ref(&big));
    let results = super::library::trash_files(&going);
    assert!(matches!(results[0].1, Ok(super::library::Change::InTrash(_))), "{:?}", results[0].1);
    let label = lib.undo_label().map(String::from);
    lib.trashed(results);
    assert_eq!(lib.undo_label().map(String::from), label, "only the trash can bring it back, so there is no undo step that fails");
    assert!(lib.toast.as_ref().is_some_and(|t| t.message.contains("Papierkorb")));
}

#[test]
fn a_note_in_an_old_encoding_gets_a_readable_copy_and_stays_as_it_is() {
    let _guard = VAULT_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (root, mut lib) = vault("encoding");
    let latin = root.join("Bio.md");
    std::fs::write(&latin, b"M\xfcller \x80 Gr\xfc\xdfe\n").unwrap();
    let wide = root.join("Chemie.md");
    let mut utf16 = vec![0xff, 0xfe];
    utf16.extend("Säure\n".encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&wide, &utf16).unwrap();
    super::vault::index::rebuild(&root, &[]);
    let copy = lib.utf8_copy(&latin).unwrap();
    assert_eq!(copy.file_name().unwrap(), "Bio (UTF-8).md");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "Müller € Grüße\n");
    assert_eq!(std::fs::read(&latin).unwrap(), b"M\xfcller \x80 Gr\xfc\xdfe\n");
    assert_eq!(std::fs::read_to_string(lib.utf8_copy(&wide).unwrap()).unwrap(), "Säure\n");
    assert_eq!(std::fs::read(&wide).unwrap(), utf16);
    assert_eq!(lib.utf8_copy(&latin).unwrap(), copy, "a second click opens the same copy");
    let mixed = root.join("Physik.md");
    std::fs::write(&mixed, b"Gr\xc3\xbc\xc3\x9fe und M\xfcller\n").unwrap();
    assert_eq!(std::fs::read_to_string(lib.utf8_copy(&mixed).unwrap()).unwrap(), "Grüße und Müller\n", "UTF-8 with one stray byte keeps its umlauts");
    let marked = root.join("Mathe.md");
    std::fs::write(&marked, b"\xef\xbb\xbfGr\xc3\xbc\xc3\x9fe M\xfcller").unwrap();
    assert_eq!(std::fs::read_to_string(lib.utf8_copy(&marked).unwrap()).unwrap(), "Grüße Müller");
    std::fs::write(&copy, "bearbeitet").unwrap();
    assert_eq!(lib.utf8_copy(&latin).unwrap().file_name().unwrap(), "Bio (UTF-8)-2.md");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "bearbeitet");
}

#[test]
fn finishing_the_setup_list_travels_with_the_folder_and_leaves_old_files_as_they_are() {
    let plain = serde_json::to_string(&super::model::StudyState::default()).unwrap();
    assert!(!plain.contains("setup"), "{plain}");
    let done = super::model::StudyState { setup_completed: true, ..Default::default() };
    let read: super::model::StudyState = serde_json::from_str(&serde_json::to_string(&done).unwrap()).unwrap();
    assert!(read.setup_completed && !read.setup_dismissed);
}

#[test]
fn saving_after_the_study_folder_disappeared_names_the_missing_folder() {
    let root = folder("gone-while-open");
    let mut lib = open(&root);
    lib.config.get_or_insert_with(Default::default);
    std::fs::remove_dir_all(&root).unwrap();
    lib.write_config();
    let say = crate::core::i18n::tr("Der Studienordner fehlt. Wurde er verschoben oder gelöscht?");
    assert_eq!(lib.last_error.as_deref(), Some(say.as_str()));
    assert!(!root.exists(), "no folder is created behind the user's back");
}

#[test]
fn a_search_does_not_follow_the_user_to_other_pages() {
    let root = folder("search-stays-home");
    let mut lib = open(&root);
    lib.set_route(Route::Module("a".into()));
    lib.module_query = "alt".into();
    lib.set_route(Route::Module("b".into()));
    assert!(lib.module_query.is_empty(), "another module starts without the old filter");
    lib.search_query = "alt".into();
    lib.set_route(Route::Search);
    lib.set_route(Route::Doc(root.join("a.md")));
    assert_eq!(lib.search_query, "alt", "going into a result keeps the query for the way back");
    lib.set_route(Route::Today);
    assert!(lib.search_query.is_empty(), "another page starts without it");
}
