use std::path::PathBuf;

use super::library::Library;

/// Loads the workspace in `SCHRUMBI_TEST_ROOT`, if set, and prints what the overview shows.
#[test]
fn real_workspace_summary() {
    let Some(root) = std::env::var_os("SCHRUMBI_TEST_ROOT").map(PathBuf::from) else { return };
    let mut lib = Library::new();
    lib.open(&root);
    if let Some(job) = lib.scan_job() {
        lib.apply_scan(job.run());
    }
    let (done, due, total) = lib.progress();
    println!("progress {done}/{total} due {due}");
    println!("week {:?}", lib.week_progress());
    println!("streak {}", lib.streak());
    println!("actions {}", lib.action_items().len());
    println!("next {:?}", lib.next_deadline().map(|a| (&a.title, a.due)));
    println!("inbox {}", lib.inbox.len());
    for a in lib.semester_assignments().iter().take(60) {
        println!("{} {} {:?}", a.id, a.due.format("%Y-%m-%d %H:%M"), lib.status(a));
    }
}

/// Renders the first page of the PDF in `SCHRUMBI_TEST_PDF`, if set.
#[test]
fn renders_pdf_page() {
    let Some(path) = std::env::var_os("SCHRUMBI_TEST_PDF").map(PathBuf::from) else { return };
    let t = std::time::Instant::now();
    let doc = super::pdf_render::PdfDoc::open(&path).expect("opens");
    let png = doc.render_png(0, 2.).expect("renders");
    println!("pages {} png {} bytes in {:?}", doc.page_count(), png.len(), t.elapsed());
    std::fs::write(std::env::temp_dir().join("schrumbi-page.png"), png).unwrap();
}

#[test]
fn every_translation_key_exists_in_english() {
    let catalog: std::collections::HashMap<String, serde_json::Value> = serde_json::from_str(include_str!("../../assets/i18n/en.json")).unwrap();
    let call = regex::Regex::new(r#"\btrf?\(\s*"((?:[^"\\]|\\.)*)""#).unwrap();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut missing = Vec::new();
    for file in walkdir::WalkDir::new(root).into_iter().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "rs")) {
        let source = std::fs::read_to_string(file.path()).unwrap();
        for c in call.captures_iter(&source) {
            let key = c[1].replace("\\\"", "\"").replace("\\n", "\n");
            if !catalog.contains_key(&key) {
                missing.push(key);
            }
        }
    }
    assert!(missing.is_empty(), "missing translations: {missing:?}");
}

#[test]
fn cuts_long_names_at_the_end() {
    use super::text::end_ellipsis;
    assert_eq!(end_ellipsis("Analysis", 28), "Analysis");
    assert_eq!(end_ellipsis("Einführung in die Betriebswirtschaftslehre", 20), "Einführung in die B…");
    assert_eq!(end_ellipsis("Übung-Übung-Übung", 8), "Übung-Ü…");
    assert_eq!(end_ellipsis("abc", 1), "abc");
}

#[test]
fn shortens_file_names_in_the_middle() {
    use super::text::middle_ellipsis;
    assert_eq!(middle_ellipsis("kurz.pdf", 26), "kurz.pdf");
    assert_eq!(middle_ellipsis("P1_26_Fragebogen-Lang_Muster-M_261028.pdf", 26), "P1_26_Fragebo…M_261028.pdf");
    assert_eq!(middle_ellipsis("Übung-Übung-Übung-Übung.pdf", 11), "Übung…g.pdf");
    assert_eq!(middle_ellipsis("abc", 2), "abc");
}
