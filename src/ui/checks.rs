use std::path::PathBuf;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::fields::Fields;
use super::pages::Pages;
use super::state::act;
use super::theme::{Theme, font, radius};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::library::Library;
use crate::core::model::Status;

/// Edited file names and whether the missing-folder list is open.
#[derive(Default)]
pub struct ChecksState {
    pub names: std::collections::HashMap<PathBuf, String>,
    pub show_folders: bool,
}

fn card(title: String, subtitle: String, icon_name: &str, tint: Hsla, actions: Option<AnyElement>, content: Div, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(16.))
        .rounded(px(radius::CARD))
        .bg(t.secondary_bg)
        .border_1()
        .border_color(t.separator.opacity(0.5))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .child(div().pt(px(2.)).child(icon(icon_name, 15., tint)))
                .child(div().flex_1().flex().flex_col().gap(px(2.)).child(div().font_weight(FontWeight::BOLD).child(title)).child(div().text_size(px(font::CALLOUT)).text_color(t.secondary).child(subtitle)))
                .children(actions),
        )
        .child(content)
}

pub fn render(lib_entity: &Entity<Library>, st: &ChecksState, fields: &mut Fields, window: &mut Window, cx: &mut Context<Pages>, t: &Theme) -> AnyElement {
    let (issues, unconfirmed, overdue, missing, suggestions) = {
        let lib = lib_entity.read(cx);
        let issues: Vec<_> = lib.misnamed().into_iter().map(|(a, f)| (a.clone(), f)).collect();
        let suggestions: Vec<Option<String>> = issues.iter().map(|(a, f)| lib.suggested_name(f, a)).collect();
        let unconfirmed: Vec<_> = lib.unconfirmed().into_iter().cloned().collect();
        let overdue: Vec<_> = lib.action_items().iter().filter(|a| lib.status(a) == Status::Overdue).cloned().collect();
        (issues, unconfirmed, overdue, lib.missing_folders(), suggestions)
    };
    if issues.is_empty() && unconfirmed.is_empty() && overdue.is_empty() && missing.is_empty() {
        return unavailable("badge-check", tr("Alles in Ordnung"), tr("Dateinamen stimmen, Uploads sind bestätigt, alle Ordner existieren."), t).into_any_element();
    }
    let mut col = div()
        .w_full()
        .max_w(px(900.))
        .p(px(24.))
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(div().flex().flex_col().gap(px(4.)).child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(tr("Prüfen"))).child(div().text_color(t.secondary).child(tr("Hier landet alles, was ein kurzes Aufräumen braucht."))));

    if !issues.is_empty() {
        let view = cx.entity();
        let names: Vec<(PathBuf, String)> = issues.iter().zip(&suggestions).map(|((_, f), s)| (f.clone(), st.names.get(f).cloned().or_else(|| s.clone()).unwrap_or_else(|| f.file_name().unwrap_or_default().to_string_lossy().to_string()))).collect();
        let field_states: Vec<_> = issues
            .iter()
            .enumerate()
            .map(|(i, (_, f))| {
                let (v, path) = (view.clone(), f.clone());
                fields.input(&format!("rename-{}", f.display()), &names[i].1, &tr("Neuer Name"), window, cx, move |text, cx| {
                    let p = path.clone();
                    v.update(cx, |pg, _| {
                        pg.checks.names.insert(p, text);
                    })
                })
            })
            .collect();
        let lib = lib_entity.read(cx);
        let all = names.clone();
        let mut rows = div().flex().flex_col().gap(px(10.));
        for (i, (a, f)) in issues.iter().enumerate() {
            if i > 0 {
                rows = rows.child(divider(t));
            }
            let name = names[i].1.clone();
            let field = &field_states[i];
            let valid = lib.naming(&a.module_id).is_none_or(|n| n.is_valid(&name));
            let (p1, n1) = (f.clone(), name.clone());
            let module = lib.module(a);
            rows = rows.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().flex().items_center().gap(px(6.)).child(module_chip(module.map_or("?", |m| m.code.as_str()), t.palette(module.map_or("blue", |m| m.color.as_str())))).child(caption(a.title.clone(), t)))
                    .child(div().flex().items_center().gap(px(8.)).child(file_icon(f, 18.)).child(div().truncate().child(f.file_name().unwrap_or_default().to_string_lossy().to_string())).child(icon("triangle-alert", 12., t.orange)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(icon("corner-down-right", 13., t.secondary))
                            .child(div().flex_1().font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(Input::new(field).small()))
                            .child(
                                div()
                                    .id(SharedString::from(format!("valid-{i}")))
                                    .child(icon(if valid { "badge-check" } else { "triangle-alert" }, 14., if valid { t.green } else { t.orange }))
                                    .tooltip(tip(if valid { tr("Entspricht der Konvention") } else { tr("Entspricht noch nicht der Konvention") })),
                            )
                            .child(Button::new(SharedString::from(format!("rename-btn-{i}"))).small().label(tr("Umbenennen")).on_click(move |_, _, cx| act(cx, |l| l.rename(&p1, &n1)))),
                    ),
            );
        }
        let v = view.clone();
        let rename_all = Button::new("rename-all").label(tr("Alle umbenennen")).on_click(move |_, _, cx| {
            let items = all.clone();
            act(cx, |l| l.rename_all(items));
            v.update(cx, |p, _| p.checks.names.clear());
        });
        col = col.child(card(tr("Dateinamen"), trf("%lld Dateien passen nicht zur Konvention", &[&issues.len()]), "type", t.orange, Some(rename_all.into_any_element()), rows, t));
    }

    let view = cx.entity();
    let lib = lib_entity.read(cx);
    if !unconfirmed.is_empty() {
        let ids: Vec<String> = unconfirmed.iter().map(|a| a.id.clone()).collect();
        let confirm = Button::new("confirm-all-checks").label(tr("Alle bestätigen")).on_click(move |_, _, cx| act(cx, |l| l.confirm_all(&ids)));
        let rows = div().flex().flex_col().children(unconfirmed.iter().map(|a| assignment_row(lib, a, true, true, true, t)));
        col = col.child(card(tr("Upload bestätigen"), tr("Datei liegt bereit, Deadline ist vorbei"), "circle-check", t.blue, Some(confirm.into_any_element()), rows, t));
    }

    if !overdue.is_empty() {
        let rows = div().flex().flex_col().children(overdue.iter().map(|a| {
            let (u, s) = (a.id.clone(), a.id.clone());
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().child(assignment_row(lib, a, true, true, true, t)))
                .child(Button::new(SharedString::from(format!("up-{}", a.id))).small().label(tr("Abgegeben")).on_click(move |_, _, cx| act(cx, |l| l.set_status(Some(Status::Uploaded), &u, true))))
                .child(Button::new(SharedString::from(format!("skip-{}", a.id))).small().label(tr("Entfällt")).on_click(move |_, _, cx| act(cx, |l| l.set_status(Some(Status::Skipped), &s, true))))
        }));
        col = col.child(card(tr("Überfällig"), tr("Keine Datei gefunden. Abgegeben, entfallen oder noch offen?"), "circle-alert", t.red, None, rows, t));
    }

    if !missing.is_empty() {
        let create = Button::new("create-folders").label(tr("Alle anlegen")).on_click(|_, _, cx| {
            act(cx, |l| l.create_missing_folders());
        });
        let view = view.clone();
        let mut list = div().flex().flex_col().gap(px(3.)).child(
            link_button("show-folders", tr("Liste anzeigen"), t)
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(font::CALLOUT))
                .child(icon(if st.show_folders { "chevron-down" } else { "chevron-right" }, 11., t.accent))
                .on_click(move |_, _, cx| view.update(cx, |p, cx| {
                    p.checks.show_folders = !p.checks.show_folders;
                    cx.notify();
                })),
        );
        if st.show_folders {
            for p in missing.iter().take(40) {
                list = list.child(div().text_size(px(font::CAPTION)).font_family(Theme::mono_family()).text_color(t.secondary).child(lib.relative_path(p)));
            }
            if missing.len() > 40 {
                list = list.child(caption(trf("… und %lld weitere", &[&(missing.len() - 40)]), t));
            }
        }
        let name = lib.semester().map(|s| s.name.clone()).unwrap_or_default();
        col = col.child(card(tr("Ordnerstruktur"), trf("%lld Ordner für %@ fehlen", &[&missing.len(), &name]), "folder-search", t.secondary, Some(create.into_any_element()), list, t));
    }
    div().id("checks-scroll").size_full().overflow_y_scroll().child(col).into_any_element()
}
