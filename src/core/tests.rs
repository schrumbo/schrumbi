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
    println!("missing {}", lib.missing_folders().len());
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
