use std::path::{Path, PathBuf};

use gpui_kit::{App, AppContext};

use super::state::{self, act};
use crate::core::i18n::{tr, trf};
use crate::core::pdf_export::{self, Meta, Options};
use crate::core::{day, library};

fn meta(path: &Path, cx: &App) -> Meta {
    let date = path.file_name().and_then(|n| day::date_in_name(&n.to_string_lossy())).unwrap_or_else(|| chrono::Local::now().date_naive());
    state::read(cx, |l| {
        let module = l.module_containing(path).map(|m| if m.name == m.code { m.code.clone() } else { format!("{} · {}", m.code, m.name) });
        let week = l.lecture_week_containing(date).map(|w| format!("W{:02}", w.index));
        let author = l.config.as_ref().map(|c| c.author.clone()).unwrap_or_default();
        Meta { left: [module, week].into_iter().flatten().collect::<Vec<_>>().join(" · "), right: [author, day::long(date)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ") }
    })
}

/// Exports a Markdown note as PDF next to it, in the background.
pub fn export(path: &Path, text: Option<String>, cx: &mut App) {
    let source = path.to_path_buf();
    if state::read(cx, |l| l.exporting_pdfs.contains(&source)) {
        return;
    }
    let Some(text) = text.or_else(|| std::fs::read_to_string(path).ok()) else { return };
    let beside = path.with_extension("pdf");
    let target = if beside.exists() && !pdf_export::is_export(&beside) { library::unique_path(&beside) } else { beside };
    let meta = meta(path, cx);
    let options = Options::current();
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let replaced = state::read(cx, |l| l.keep_for_undo(&target));
    act(cx, |l| {
        l.exporting_pdfs.insert(source.clone());
    });
    let out = target.clone();
    let task = cx.background_spawn(async move { pdf_export::export(&text, &base, &out, Some(&meta), &options) });
    cx.spawn(async move |cx| {
        let result = task.await;
        let _ = cx.update(|cx| {
            act(cx, |l| {
                l.exporting_pdfs.remove(&source);
                match result {
                    Ok(()) => {
                        l.refresh();
                        l.record_export(&target, replaced);
                        let name = target.file_name().unwrap_or_default().to_string_lossy().to_string();
                        l.notify(trf("PDF erstellt: %@", &[&name]), "file-text", Some(tr("Öffnen")), Some(Box::new(move |l| l.open_file(&target))));
                    }
                    Err(e) => l.last_error = Some(trf("PDF-Export fehlgeschlagen: %@", &[&e])),
                }
            });
        });
    })
    .detach();
}

/// Renders a sample note with the current PDF settings and opens it.
pub fn sample(cx: &mut App) {
    let target: PathBuf = std::env::temp_dir().join(format!("{}.pdf", tr("Beispiel-Notiz")));
    let text = sample_text();
    let meta = Meta { left: "P1 · Projekt".into(), right: day::long(chrono::Local::now().date_naive()) };
    let options = Options::current();
    let out = target.clone();
    let task = cx.background_spawn(async move { pdf_export::export(&text, &std::env::temp_dir(), &out, Some(&meta), &options) });
    cx.spawn(async move |cx| {
        let result = task.await;
        let _ = cx.update(|cx| match result {
            Ok(()) => library::open_path(&target),
            Err(e) => act(cx, |l| l.last_error = Some(e)),
        });
    })
    .detach();
}

fn sample_text() -> String {
    [
        format!("# {}", tr("Beispiel-Notiz")),
        tr("Ein kurzer Absatz mit **Betonung**, *Kursivem*, `Code` und einem [Link](https://example.com)."),
        format!("## {}", tr("Aufgaben")),
        format!("- [x] {}\n- [ ] {}\n- [ ] {}", tr("Recherche"), tr("Entwurf"), tr("Abgabe")),
        format!("## {}", tr("Übersicht")),
        format!("| {} | {} |\n|---|---|\n| {} | {} |\n| {} | {} |", tr("Punkt"), tr("Status"), tr("Interviews"), tr("fertig"), tr("Auswertung"), tr("offen")),
        format!("> {}", tr("Ein Zitat, das hervorgehoben wird.")),
        format!("1. {}\n2. {}", tr("Erstens"), tr("Zweitens")),
    ]
    .join("\n\n")
}
