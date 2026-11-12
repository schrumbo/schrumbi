use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::quick::Dismissed;
use super::theme::{Theme, font, space};
use super::widgets::caption;

use super::state::{self, act};
use crate::core::i18n::{tr, trf};
use crate::core::pdf_export::{self, Meta, Options};
use crate::core::{day, library};

fn meta(path: &Path, cx: &App) -> Meta {
    let date = path.file_name().and_then(|n| day::date_in_name(&n.to_string_lossy())).unwrap_or_else(|| chrono::Local::now().date_naive());
    state::read(cx, |l| {
        let module = l.module_containing(path).map(|m| if m.name == m.code { m.code.clone() } else { format!("{} · {}", m.code, m.name) });
        let week = module.as_ref().and_then(|_| l.lecture_week_containing(date)).map(|w| format!("W{:02}", w.index));
        let author = l.config.as_ref().map(|c| c.author.clone()).unwrap_or_default();
        Meta { left: [module, week].into_iter().flatten().collect::<Vec<_>>().join(" · "), right: [author, day::long(date)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "), title: path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default() }
    })
}

/// Exports a Markdown note as PDF next to it, in the background.
pub fn export(path: &Path, text: Option<String>, window: &mut Window, cx: &mut App) {
    let source = path.to_path_buf();
    if state::read(cx, |l| l.exporting_pdfs.contains(&source)) {
        return;
    }
    let Some(text) = text.or_else(|| std::fs::read_to_string(path).ok()) else { return };
    let wanted = path.with_extension("pdf");
    if !wanted.exists() {
        return run(path, text, wanted, cx);
    }
    let Some(beside) = earlier_export(&wanted) else {
        return run(path, text, library::unique_path(&wanted), cx);
    };
    if !crate::core::prefs::get().pdf_replace_ask {
        return run(path, text, beside, cx);
    }
    super::sheets::show_sheet(460., window, cx, move |_, cx| ReplaceExport { source, text, target: beside, ask_again: true, focus: cx.focus_handle(), focused: false });
}

/// The export made earlier for this note: the PDF beside it, or a numbered one when another file had that name.
fn earlier_export(wanted: &Path) -> Option<PathBuf> {
    if pdf_export::is_export(wanted) {
        return Some(wanted.to_path_buf());
    }
    let name = wanted.file_name()?.to_string_lossy().to_string();
    let numbered = regex::Regex::new(r"-\d+").ok()?;
    let mut found: Vec<PathBuf> = std::fs::read_dir(wanted.parent()?).ok()?.flatten().map(|e| e.path()).filter(|p| p.file_name().map(|n| n.to_string_lossy().to_string()).is_some_and(|n| numbered.find_iter(&n).any(|m| format!("{}{}", &n[..m.start()], &n[m.end()..]) == name))).filter(|p| pdf_export::is_export(p)).collect();
    found.sort();
    found.into_iter().next()
}

/// Asks before an export replaces the one made earlier; it can be told not to ask again.
struct ReplaceExport {
    source: PathBuf,
    text: String,
    target: PathBuf,
    ask_again: bool,
    focus: FocusHandle,
    focused: bool,
}

impl EventEmitter<Dismissed> for ReplaceExport {}

impl ReplaceExport {
    fn replace(&mut self, cx: &mut Context<Self>) {
        if state::read(cx, |l| l.exporting_pdfs.contains(&self.source)) {
            return cx.emit(Dismissed);
        }
        if !self.ask_again {
            crate::core::prefs::update(|p| p.pdf_replace_ask = false);
        }
        let target = if self.target.exists() && !pdf_export::is_export(&self.target) { library::unique_path(&self.target) } else { self.target.clone() };
        run(&self.source, std::mem::take(&mut self.text), target, cx);
        cx.emit(Dismissed);
    }
}

impl Render for ReplaceExport {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !std::mem::replace(&mut self.focused, true) {
            window.focus(&self.focus, cx);
        }
        let t = Theme::get(cx);
        let name = self.target.file_name().unwrap_or_default().to_string_lossy().to_string();
        let ask_again = self.ask_again;
        div()
            .w_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .on_action(cx.listener(|s, _: &gpui_kit::base::actions::Confirm, window, cx| if s.focus.is_focused(window) { s.replace(cx) } else { cx.propagate() }))
            .child(div().px(px(space::XL)).pt(px(space::L)).flex().flex_col().gap(px(space::XXS)).child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(tr("Früheren Export ersetzen?"))).child(caption(name, &t).truncate()))
            .child(
                div()
                    .px(px(space::XL))
                    .py(px(space::L))
                    .flex()
                    .flex_col()
                    .gap(px(space::M))
                    .child(div().text_size(px(font::BODY)).text_color(t.primary).child(tr("Das PDF neben der Notiz wird überschrieben. Rückgängig holt es zurück.")))
                    .child(Checkbox::new("export-ask").label(tr("Nicht mehr fragen")).checked(!ask_again).on_click(cx.listener(|s, v: &bool, _, cx| {
                        s.ask_again = !*v;
                        cx.notify();
                    }))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(space::S))
                    .p(px(space::L))
                    .border_t_1()
                    .border_color(t.separator)
                    .child(Button::new("export-cancel").label(tr("Abbrechen")).on_click(cx.listener(|_, _, _, cx| cx.emit(Dismissed))))
                    .child(Button::new("export-replace").primary().label(tr("Ersetzen")).on_click(cx.listener(|s, _, _, cx| s.replace(cx)))),
            )
    }
}

fn run(path: &Path, text: String, target: PathBuf, cx: &mut App) {
    let source = path.to_path_buf();
    let meta = meta(path, cx);
    let options = Options::current();
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let backup = state::read(cx, |l| l.keep_for_undo(&target));
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
                        let replaced = backup.is_some();
                        l.record_export(&target, backup, &source);
                        let name = target.file_name().unwrap_or_default().to_string_lossy().to_string();
                        if replaced {
                            let undo = l.undo_latest();
                            l.notify(trf("PDF ersetzt: %@", &[&name]), "file-text", None, Some(undo));
                            return;
                        }
                        let message = match l.deadlines_of(&target).first() {
                            Some(a) => trf("PDF erstellt für %@: %@", &[&library::deadline_label(a), &name]),
                            None => trf("PDF erstellt: %@", &[&name]),
                        };
                        l.notify(message, "file-text", Some(tr("Öffnen")), Some(Box::new(move |l| l.open_file(&target))));
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
    let meta = Meta { left: "P1 · Projekt".into(), right: day::long(chrono::Local::now().date_naive()), title: tr("Beispiel-Notiz") };
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn a_numbered_export_is_found_again() {
        let dir = std::env::temp_dir().join(format!("schrumbi-earlier-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wanted = dir.join("Mitschrift_261103.pdf");
        assert_eq!(super::earlier_export(&wanted), None);
        std::fs::write(&wanted, b"%PDF-1.4 Abgabe").unwrap();
        assert_eq!(super::earlier_export(&wanted), None, "another file of that name is not an export");
        let numbered = crate::core::library::unique_path(&wanted);
        crate::core::pdf_export::export("# Titel", Path::new("/"), &numbered, None, &crate::core::pdf_export::Options { accent: "#0EA5E9".into(), header: true, page_numbers: true, lang: "de".into(), properties: false }).unwrap();
        assert_eq!(super::earlier_export(&wanted), Some(numbered), "the numbered export is the one to replace");
    }
}
