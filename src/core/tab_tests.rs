use std::path::PathBuf;

use super::library::{Library, Route};

fn notes(name: &str, files: &[&str]) -> (PathBuf, Vec<PathBuf>) {
    let dir = std::env::temp_dir().join(format!("schrumbi-tabs-{}", std::process::id())).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = dunce::canonicalize(&dir).unwrap();
    let paths = files.iter().map(|f| root.join(f)).collect::<Vec<_>>();
    for p in &paths {
        std::fs::write(p, "x").unwrap();
    }
    (root, paths)
}

fn opened(paths: &[PathBuf]) -> Library {
    let mut lib = Library::new();
    for p in paths {
        lib.open_document(p);
    }
    lib
}

#[test]
fn an_edited_preview_stays_after_it_was_saved() {
    let (_, p) = notes("edited-preview", &["a.md", "b.md"]);
    let mut lib = Library::new();
    lib.open_preview(&p[0]);
    lib.buffer(&p[0]).edited("neu".into());
    lib.buffer(&p[0]).save().unwrap();
    lib.open_preview(&p[1]);
    assert_eq!(lib.open_documents, vec![p[0].clone(), p[1].clone()], "a note the user wrote in is never swapped out");
}

#[test]
fn a_link_opens_beside_and_never_replaces_a_kept_tab() {
    let (_, p) = notes("link-kept", &["a.md", "b.md"]);
    let mut lib = opened(&p[..1]);
    lib.follow_path(&p[1], None);
    assert_eq!(lib.open_documents, vec![p[0].clone(), p[1].clone()]);
    assert_eq!(lib.preview_document, Some(p[1].clone()), "a followed link is a preview");
    assert_eq!(*lib.route(), Route::Doc(p[1].clone()));
}

#[test]
fn links_walk_inside_a_preview_and_back() {
    let (_, p) = notes("link-preview", &["a.md", "b.md", "c.md"]);
    let mut lib = opened(&p[..1]);
    lib.open_preview(&p[1]);
    lib.follow_path(&p[2], None);
    assert_eq!(lib.open_documents, vec![p[0].clone(), p[2].clone()], "the preview walks on");
    lib.go_back();
    assert_eq!(lib.open_documents, vec![p[0].clone(), p[1].clone()]);
    assert_eq!(*lib.route(), Route::Doc(p[1].clone()));
}

#[test]
fn back_never_swaps_out_a_tab_that_was_kept() {
    let (_, p) = notes("link-pinned", &["a.md", "b.md"]);
    let mut lib = Library::new();
    lib.open_preview(&p[0]);
    lib.follow_path(&p[1], None);
    lib.pin_document(&p[1]);
    lib.go_back();
    assert!(lib.open_documents.contains(&p[1]), "kept tabs stay");
}

#[test]
fn a_link_in_the_right_pane_opens_on_the_right() {
    let (_, p) = notes("link-right", &["a.md", "b.md", "c.md"]);
    let mut lib = opened(&p[..2]);
    lib.open_document(&p[0]);
    lib.set_split(Some(p[1].clone()));
    lib.beside_active = true;
    let folder = p[1].parent().unwrap().join(".");
    lib.follow_path(&p[2], Some(&folder));
    assert_eq!(lib.active_document, Some(p[0].clone()), "the left note stays");
    assert_eq!(lib.split_document, Some(p[2].clone()));
    assert!(lib.open_documents.contains(&p[1]), "the note that was on the right keeps its tab");
    lib.set_route(Route::Today);
    lib.follow_path(&p[1], Some(&folder));
    assert_eq!((lib.route().clone(), lib.split_document.clone()), (Route::Doc(p[1].clone()), Some(p[2].clone())), "away from the notes, a link never lands in the hidden split");
}

#[test]
fn links_from_the_details_open_on_the_left() {
    let (_, p) = notes("link-details", &["a.md", "b.md", "c.md"]);
    let mut lib = opened(&p[..2]);
    lib.open_document(&p[0]);
    lib.set_split(Some(p[1].clone()));
    lib.beside_active = true;
    lib.follow_path(&p[2], Some(&p[0]));
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(p[2].clone()), Some(p[1].clone())));
}

#[test]
fn back_never_swaps_out_a_preview_that_was_written_in() {
    let (_, p) = notes("back-touched", &["a.md", "b.md"]);
    let mut lib = Library::new();
    lib.open_preview(&p[0]);
    lib.follow_path(&p[1], None);
    lib.buffer(&p[1]).replace("neu".into());
    lib.go_back();
    assert!(lib.open_documents.contains(&p[1]));
}

#[test]
fn closed_tabs_come_back_in_order_after_closing_all() {
    let (_, p) = notes("close-all", &["a.md", "b.md", "c.md", "d.md"]);
    let mut lib = opened(&p);
    lib.close_other_documents(&p[2]);
    for _ in 0..3 {
        lib.reopen_document();
    }
    assert_eq!(lib.open_documents, p);
    lib.close_all_documents();
    for _ in 0..4 {
        lib.reopen_document();
    }
    assert_eq!(lib.open_documents, p);
}

#[test]
fn back_between_tabs_moves_the_shown_tab() {
    let (_, p) = notes("back-tabs", &["a.md", "b.md", "c.md"]);
    let mut lib = opened(&p);
    lib.open_document(&p[0]);
    lib.open_document(&p[1]);
    lib.go_back();
    assert_eq!((lib.route().clone(), lib.active_document.clone()), (Route::Doc(p[0].clone()), Some(p[0].clone())));
    lib.set_split(Some(p[1].clone()));
    lib.open_document(&p[2]);
    lib.go_back();
    assert_eq!(*lib.route(), Route::Doc(p[0].clone()));
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(p[0].clone()), Some(p[1].clone())));
    lib.go_forward();
    lib.set_split(Some(p[0].clone()));
    lib.go_back();
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(p[0].clone()), Some(p[2].clone())), "the note on the right comes left, the other goes right");
}

#[test]
fn opening_the_right_note_keeps_the_split() {
    let (_, p) = notes("split-open", &["a.md", "b.md"]);
    let mut lib = opened(&p);
    lib.open_document(&p[0]);
    lib.set_split(Some(p[1].clone()));
    lib.open_document(&p[1]);
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(p[0].clone()), Some(p[1].clone())));
    assert!(lib.focus_beside, "the right pane takes the focus");
}

#[test]
fn closing_the_left_note_keeps_the_split_while_other_tabs_are_left() {
    let (_, p) = notes("split-close", &["a.md", "b.md", "c.md"]);
    let mut lib = opened(&p);
    lib.open_document(&p[1]);
    lib.set_split(Some(p[2].clone()));
    lib.close_document(&p[1]);
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(p[0].clone()), Some(p[2].clone())));
    lib.close_document(&p[0]);
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(p[2].clone()), None), "the last left tab gone, the split folds");
}

#[test]
fn a_dragged_tab_takes_the_place_it_was_dropped_on() {
    let (_, p) = notes("drag", &["a.md", "b.md", "c.md"]);
    let mut lib = opened(&p);
    lib.move_document(&p[0], &p[2]);
    assert_eq!(lib.open_documents, vec![p[1].clone(), p[2].clone(), p[0].clone()], "dragged to the end");
    lib.move_document(&p[0], &p[1]);
    assert_eq!(lib.open_documents, vec![p[0].clone(), p[1].clone(), p[2].clone()], "dragged to the front");
}

#[test]
fn a_restart_shows_the_tab_that_was_shown() {
    let (root, p) = notes("restart", &["a.md", "b.md", "c.md"]);
    let mut lib = Library::new();
    lib.open(&root);
    for d in &p {
        lib.open_document(d);
    }
    lib.open_document(&p[1]);
    let mut again = Library::new();
    again.open(&root);
    assert_eq!(again.active_document, Some(p[1].clone()));
}

#[test]
fn a_reopened_tab_comes_back_where_it_was() {
    let (_, p) = notes("reopen", &["a.md", "b.md", "c.md"]);
    let mut lib = opened(&p);
    lib.close_document(&p[0]);
    lib.reopen_document();
    assert_eq!(lib.open_documents, p);
    assert_eq!(lib.active_document, Some(p[0].clone()));
}

#[test]
fn a_preview_shown_on_the_right_is_kept() {
    let (_, p) = notes("split-preview", &["a.md", "b.md", "c.md"]);
    let mut lib = opened(&p[..1]);
    lib.open_preview(&p[1]);
    lib.open_document(&p[0]);
    lib.set_split(Some(p[1].clone()));
    lib.open_preview(&p[2]);
    assert!(lib.open_documents.contains(&p[1]), "the note on the right keeps its tab");
    assert_eq!(lib.split_document, Some(p[1].clone()));
}

#[test]
fn cycling_tabs_skips_the_right_note() {
    let (_, p) = notes("cycle", &["a.md", "b.md", "c.md"]);
    let mut lib = opened(&p);
    lib.open_document(&p[0]);
    lib.set_split(Some(p[1].clone()));
    lib.cycle_document(1);
    assert_eq!((lib.active_document.clone(), lib.split_document.clone()), (Some(p[2].clone()), Some(p[1].clone())));
    lib.cycle_document(1);
    assert_eq!(lib.active_document, Some(p[0].clone()));
}

#[test]
fn a_preview_changed_from_outside_is_kept_and_offers_no_dead_steps() {
    let (_, p) = notes("touched-steps", &["a.md", "b.md"]);
    let mut lib = Library::new();
    lib.open_preview(&p[0]);
    lib.follow_path(&p[1], None);
    lib.go_back();
    assert!(lib.can_go_forward());
    lib.buffer(&p[0]).restore("neu".into()).unwrap();
    assert!(!lib.is_preview(&p[0]), "a changed note no longer looks like a preview");
    assert!(!lib.can_go_forward(), "Forward is off when it would do nothing");
}

#[test]
fn back_and_forward_never_stop_on_the_page_already_shown() {
    let (_, p) = notes("dead-step", &["a.md", "b.md"]);
    let mut lib = opened(&p);
    lib.open_document(&p[0]);
    lib.close_document(&p[1]);
    let before = lib.route().clone();
    lib.go_back();
    assert_ne!(*lib.route(), before, "Back always shows something else");
    lib.go_forward();
    assert_eq!(*lib.route(), before);
    assert!(!lib.can_go_forward(), "nothing left ahead");
}

#[test]
fn the_focused_document_follows_the_active_pane() {
    let (_, p) = notes("focused-pane", &["a.md", "b.pdf"]);
    let mut lib = opened(&p);
    lib.open_document(&p[0]);
    lib.set_split(Some(p[1].clone()));
    lib.beside_active = false;
    assert_eq!(lib.focused_document(), Some(p[0].as_path()));
    lib.beside_active = true;
    assert_eq!(lib.focused_document(), Some(p[1].as_path()));
}

#[test]
fn a_note_in_its_own_window_is_raised_and_never_opened_as_a_second_editor() {
    let (_, p) = notes("own-window", &["a.md", "b.md"]);
    let mut lib = opened(&p);
    lib.note_windows.insert(p[0].clone());
    lib.close_document(&p[0]);
    assert_eq!(lib.open_documents, vec![p[1].clone()]);
    lib.open_preview(&p[0]);
    assert_eq!(lib.raise_note_window.take(), Some(p[0].clone()));
    lib.open_document(&p[0]);
    assert_eq!(lib.raise_note_window.take(), Some(p[0].clone()));
    lib.open_file(&p[0]);
    assert_eq!(lib.raise_note_window.take(), Some(p[0].clone()));
    assert_eq!(lib.open_documents, vec![p[1].clone()]);
    lib.note_windows.remove(&p[0]);
    lib.open_document(&p[0]);
    assert!(lib.raise_note_window.is_none());
    assert!(lib.open_documents.contains(&p[0]));
}

#[test]
fn a_closed_tab_of_a_note_that_got_its_own_window_comes_back_as_that_window() {
    let (_, p) = notes("reopen-windowed", &["a.md", "b.md"]);
    let mut lib = opened(&p);
    lib.close_document(&p[0]);
    lib.note_windows.insert(p[0].clone());
    lib.reopen_document();
    assert_eq!(lib.raise_note_window.take(), Some(p[0].clone()));
    assert_eq!(lib.open_documents, vec![p[1].clone()], "one editor for the file");
    assert!(!lib.can_reopen_document());
}

#[test]
fn dropping_a_note_that_has_its_own_window_raises_it_and_adds_no_second_editor() {
    use super::library::DocDrop;
    let (_, p) = notes("drop-windowed", &["a.md", "b.md", "c.md"]);
    let mut lib = opened(&p[..2]);
    lib.note_windows.insert(p[2].clone());
    lib.drop_document(&p[2], DocDrop::Beside, None);
    assert_eq!(lib.raise_note_window.take(), Some(p[2].clone()));
    assert_eq!(lib.split_document, None);
    assert_eq!(lib.open_documents, vec![p[0].clone(), p[1].clone()]);
    lib.drop_document(&p[2], DocDrop::Here, None);
    assert_eq!(lib.raise_note_window.take(), Some(p[2].clone()));
    assert_eq!(lib.open_documents.len(), 2);
}

#[test]
fn a_rename_leaves_a_trail_for_views_that_still_show_the_old_path() {
    let (root, p) = notes("rename-trail", &["a.md", "b.md"]);
    let mut lib = opened(&p[..1]);
    lib.buffer(&p[0]);
    lib.note_windows.insert(p[0].clone());
    lib.rename(&p[0], "neu.md");
    let new = root.join("neu.md");
    assert_eq!(lib.moved_path(&p[0]), Some(new.clone()));
    assert!(lib.note_windows.contains(&new) && lib.buffers.contains_key(&new));
    lib.rename(&new, "neuer.md");
    assert_eq!(lib.moved_path(&p[0]), Some(root.join("neuer.md")), "a second move extends the trail");
    lib.rename(&root.join("neuer.md"), "a.md");
    assert_eq!(lib.moved_path(&p[0]), None, "back at the start nothing moved");
    assert_eq!(lib.moved_path(&new), Some(p[0].clone()));
    assert_eq!(lib.moved_path(&p[1]), None);
}

#[test]
fn a_rename_of_a_tab_that_has_no_buffer_yet_leaves_a_trail_too() {
    let (root, p) = notes("rename-trail-bare", &["a.md"]);
    let mut lib = opened(&p);
    lib.buffers.remove(&p[0]);
    lib.rename_checked(&p[0], "neu.md").unwrap();
    let new = root.join("neu.md");
    assert_eq!(lib.open_documents, vec![new.clone()]);
    assert_eq!(lib.moved_path(&p[0]), Some(new));
}

#[test]
fn renaming_a_folder_moves_the_buffers_windows_and_trails_of_the_notes_inside() {
    let (root, _) = notes("rename-folder", &[]);
    let dir = root.join("kurs");
    std::fs::create_dir_all(dir.join("tief")).unwrap();
    let (a, b) = (dir.join("a.md"), dir.join("tief").join("b.md"));
    std::fs::write(&a, "x").unwrap();
    std::fs::write(&b, "y").unwrap();
    let mut lib = Library::new();
    lib.buffer(&a);
    lib.buffer(&b).edited("neu".into());
    lib.note_windows.insert(b.clone());
    lib.rename(&dir, "neu");
    let moved = root.join("neu");
    let (new_a, new_b) = (moved.join("a.md"), moved.join("tief").join("b.md"));
    assert!(lib.last_error.is_none(), "{:?}", lib.last_error);
    assert!(lib.buffers.contains_key(&new_a) && lib.buffers.contains_key(&new_b));
    assert!(!lib.buffers.contains_key(&a) && !lib.buffers.contains_key(&b));
    assert!(lib.note_windows.contains(&new_b) && !lib.note_windows.contains(&b));
    assert_eq!(lib.moved_path(&b), Some(new_b.clone()));
    assert_eq!(lib.moved_path(&a), Some(new_a));
    let buffer = lib.buffer(&new_b);
    assert!(buffer.dirty);
    buffer.save().unwrap();
    assert_eq!(std::fs::read_to_string(&new_b).unwrap(), "neu");
    assert!(!dir.exists());
}

#[test]
fn a_new_note_at_a_former_path_does_not_inherit_the_trail() {
    let (_, p) = notes("rename-trail-new", &["a.md"]);
    let mut lib = opened(&p);
    lib.buffer(&p[0]);
    lib.rename(&p[0], "neu.md");
    assert!(lib.moved_path(&p[0]).is_some());
    std::fs::write(&p[0], "x").unwrap();
    lib.buffer(&p[0]);
    assert_eq!(lib.moved_path(&p[0]), None);
}

fn folder_with_notes(name: &str) -> (PathBuf, PathBuf, [PathBuf; 4]) {
    let (root, _) = notes(name, &["c.md"]);
    let dir = root.join("kurs");
    std::fs::create_dir_all(dir.join("tief")).unwrap();
    let files = [root.join("c.md"), dir.join("a.md"), dir.join("tief").join("b.md"), dir.join("tief").join("d.md")];
    for f in &files[1..] {
        std::fs::write(f, "x").unwrap();
    }
    (root, dir, files)
}

#[test]
fn renaming_a_folder_carries_every_tab_and_remembered_position_inside() {
    use super::prefs::{self, PdfPosition};
    let (root, dir, [c, a, b, d]) = folder_with_notes("rename-folder-tabs");
    let mut lib = opened(&[c.clone(), d.clone(), b.clone(), a.clone()]);
    lib.close_document(&d);
    lib.split_document = Some(b.clone());
    let key = |p: &PathBuf| p.to_string_lossy().to_string();
    prefs::update(|p| {
        p.heading_titled.insert(key(&a));
        p.note_wraps.insert(key(&a), false);
        p.pdf_positions.insert(key(&b), PdfPosition { page: 3, fraction: 0.5, scale: None, fit_page: false, used: 1 });
    });
    let moved = |p: &PathBuf| p.strip_prefix(&dir).map_or(p.clone(), |r| root.join("neu").join(r));
    let (open, active, split) = (lib.open_documents.clone(), lib.active_document.clone().unwrap(), lib.split_document.clone().unwrap());
    lib.rename(&dir, "neu");
    assert!(lib.last_error.is_none(), "{:?}", lib.last_error);
    assert_eq!(lib.open_documents, open.iter().map(moved).collect::<Vec<_>>());
    assert_eq!(lib.active_document, Some(moved(&active)));
    assert_eq!(lib.split_document, Some(moved(&split)));
    assert_eq!(*lib.route(), Route::Doc(moved(&active)));
    let (new_a, new_b) = (moved(&a), moved(&b));
    prefs::with(|p| {
        assert!(p.heading_titled.contains(&key(&new_a)) && !p.heading_titled.contains(&key(&a)));
        assert_eq!(p.pdf_positions.get(&key(&new_b)).map(|x| x.page), Some(3));
        assert!(!p.pdf_positions.contains_key(&key(&b)));
        assert_eq!(p.note_wraps.get(&key(&new_a)), Some(&false));
        assert!(!p.note_wraps.contains_key(&key(&a)));
    });
    assert!(lib.can_reopen_document());
    lib.reopen_document();
    assert_eq!(lib.active_document, Some(moved(&d)), "a closed tab comes back at its new place");
    let target = lib.active_document.clone().unwrap();
    lib.rename(&target, "e.md");
    assert!(moved(&d).with_file_name("e.md").exists() && lib.last_error.is_none(), "{:?}", lib.last_error);
    assert_eq!(lib.active_document, Some(moved(&d).with_file_name("e.md")), "renaming the shown note after its folder changed targets the new path");
}

#[test]
fn a_preview_trail_follows_a_renamed_folder() {
    let (root, dir, [c, a, b, _]) = folder_with_notes("rename-folder-trail");
    let mut lib = opened(&[c.clone()]);
    lib.open_preview(&a);
    lib.follow_path(&b, None);
    assert_eq!(lib.preview_document, Some(b.clone()));
    lib.rename(&dir, "neu");
    let (new_a, new_b) = (root.join("neu").join("a.md"), root.join("neu").join("tief").join("b.md"));
    assert_eq!(lib.preview_document, Some(new_b));
    lib.go_back();
    assert_eq!(lib.open_documents, vec![c, new_a.clone()], "Back leads to the preview's earlier note at its new path");
    assert_eq!(*lib.route(), Route::Doc(new_a));
}

#[test]
fn closing_all_tabs_never_drops_a_dirty_buffer() {
    let (_, dir, [c, a, ..]) = folder_with_notes("close-all-dirty");
    let mut lib = opened(&[c.clone(), a.clone()]);
    lib.buffer(&a).edited("neu".into());
    lib.rename(&dir, "neu");
    let new_a = dir.with_file_name("neu").join("a.md");
    assert_eq!(lib.buffer_key(&a), new_a, "a view still on the old path finds the buffer where it went");
    assert_eq!(lib.buffer_key(&c), c);
    lib.toggle_mode = true;
    lib.close_other_documents(&c);
    assert!(lib.buffers.get(&new_a).is_some_and(|b| b.dirty), "closing the others keeps a buffer that was not saved");
    lib.close_all_documents();
    assert!(lib.buffers.get(&new_a).is_some_and(|b| b.dirty), "closing all keeps a buffer that was not saved");
    assert!(!lib.toggle_mode, "a pending switch between reading and writing does not wait for the next note");
}
