use std::collections::HashSet;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::fields::{Fields, format_grade, format_number, parse_number};
use super::pages::Pages;
use super::state::edit_config;
use super::theme::{Theme, font};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::model::{GradeComponent, GradingSystem, StudyConfig};

/// Which modules show their partial grades.
#[derive(Default)]
pub struct RecordState {
    pub expanded: HashSet<String>,
}

fn sum(it: impl Iterator<Item = f64>) -> f64 {
    it.sum()
}

pub fn render(c: &StudyConfig, st: &RecordState, fields: &mut Fields, window: &mut Window, cx: &mut Context<Pages>, t: &Theme) -> Div {
    let system = c.grading;
    let credit = c.credit_label.clone();
    let mut semesters = c.semesters.clone();
    semesters.sort_by(|a, b| b.start.cmp(&a.start));
    let earned = sum(c.modules.iter().filter(|m| m.passed).filter_map(|m| m.ects));
    let planned = sum(c.modules.iter().filter_map(|m| m.ects));
    let graded: Vec<_> = c.modules.iter().filter(|m| m.effective_grade().is_some() && m.ects.unwrap_or(0.) > 0.).collect();
    let weight = sum(graded.iter().filter_map(|m| m.ects));
    let average = (weight > 0.).then(|| graded.iter().map(|m| m.effective_grade().unwrap_or(0.) * m.ects.unwrap_or(0.)).sum::<f64>() / weight);
    let passed = c.modules.iter().filter(|m| m.passed).count();

    let tiles = div()
        .flex()
        .gap(px(12.))
        .child(
            tile("earned", tr("Punkte erreicht"), "badge-check", t.green, false, t)
                .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(format!("{} / {} {credit}", format_number(earned, 1), format_number(planned, 1))))
                .child(progress_bar((earned / planned.max(1.)) as f32, t.green, t)),
        )
        .when(system.uses_grades(), |d| {
            d.child(
                tile("average", tr("Durchschnitt"), "graduation-cap", t.accent, false, t)
                    .child(div().text_size(px(font::TITLE)).font_weight(FontWeight::BOLD).child(average.map_or("–".into(), |a| format_grade(a, system))))
                    .child(caption(if graded.is_empty() { tr("Noch keine Noten eingetragen") } else { trf("gewichtet nach %@, %lld Module", &[&credit, &graded.len()]) }, t)),
            )
        })
        .child(
            tile("modules", tr("Module"), "library", t.secondary, false, t)
                .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(trf("%lld von %lld", &[&passed, &c.modules.len()])))
                .child(caption(tr("bestanden"), t)),
        );

    let mut col = div().w_full().max_w(px(980.)).px(px(32.)).py(px(24.)).flex().flex_col().gap(px(24.)).child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).child(tr("Leistungen"))).child(tiles);

    for s in &semesters {
        let ms: Vec<_> = c.modules.iter().filter(|m| m.semester == s.id).collect();
        if ms.is_empty() {
            continue;
        }
        let mut table = grouped(t).child(
            div()
                .flex()
                .items_center()
                .px(px(14.))
                .py(px(8.))
                .text_size(px(font::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.secondary)
                .child(div().flex_1().child(tr("Modul")))
                .child(div().w(px(80.)).text_right().child(credit.clone()))
                .when(system.uses_grades(), |d| d.child(div().w(px(90.)).text_right().child(tr("Note"))))
                .child(div().w(px(90.)).text_center().child(tr("Bestanden"))),
        );
        for m in ms {
            table = table.child(divider(t)).child(module_row(c, &m.id, st.expanded.contains(&m.id), system, fields, window, cx, t));
        }
        col = col.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().flex().child(section_title(s.name.clone())).child(div().flex_1()).child(div().text_color(t.secondary).child(format!("{} {credit}", format_number(sum(c.modules.iter().filter(|m| m.semester == s.id).filter_map(|m| m.ects)), 1)))))
                .child(table),
        );
    }
    col = col.child(caption(tr("Notensystem und Bezeichnung der Punkte änderst du unter Einstellungen → Allgemein. Teilleistungen (z. B. Klausur 60 %, Projekt 40 %) ergeben die Modulnote automatisch."), t));
    div().w_full().flex().justify_center().child(col)
}

#[allow(clippy::too_many_arguments)]
fn module_row(c: &StudyConfig, id: &str, expanded: bool, system: GradingSystem, fields: &mut Fields, window: &mut Window, cx: &mut Context<Pages>, t: &Theme) -> Div {
    let Some(m) = c.module(id) else { return div() };
    let id = id.to_string();
    let view = cx.entity();
    let (i1, i2, i3) = (id.clone(), id.clone(), id.clone());
    let ects = fields.input(&format!("ects-{id}"), &m.ects.map(|v| format_number(v, 1)).unwrap_or_default(), "–", window, cx, move |v, cx| {
        let id = i1.clone();
        edit_config(cx, |c| {
            if let Some(m) = c.module_mut(&id) {
                m.ects = parse_number(&v);
            }
        });
    });
    let has_component_grades = m.components.iter().any(|x| x.grade.is_some());
    let grade = (!has_component_grades && system.uses_grades()).then(|| {
        fields.input(&format!("grade-{id}"), &m.grade.map(|g| format_number(g, 2)).unwrap_or_default(), "–", window, cx, move |v, cx| {
            let id = i2.clone();
            edit_config(cx, |c| {
                let (lo, hi) = system.range();
                if let Some(m) = c.module_mut(&id) {
                    m.grade = parse_number(&v).map(|g| g.clamp(lo, hi));
                    if let Some(g) = m.grade {
                        m.passed = system.passes(g);
                    }
                }
            });
        })
    });
    let failing = m.grade.is_some_and(|g| !system.passes(g));
    let header = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .id(SharedString::from(format!("expand-{id}")))
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(4.))
                .px(px(6.))
                .ml(px(-6.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(t.ink.opacity(0.06)))
                .tooltip(tip(tr("Teilleistungen anzeigen")))
                .on_click({
                    let id = id.clone();
                    move |_, _, cx| view.update(cx, |p, cx| {
                        if !p.record.expanded.remove(&id) {
                            p.record.expanded.insert(id.clone());
                        }
                        cx.notify();
                    })
                })
                .child(icon(if expanded { "chevron-down" } else { "chevron-right" }, 11., t.secondary))
                .child(div().size(px(9.)).rounded_full().bg(t.palette(&m.color)))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(m.code.clone()))
                .child(div().truncate().text_color(t.secondary).child(m.name.clone()))
                .when(!m.components.is_empty(), |d| d.child(div().text_size(px(font::CAPTION)).text_color(t.tertiary).child(trf("%lld Teilleistungen", &[&m.components.len()])))),
        )
        .child(div().w(px(80.)).child(Input::new(&ects).small().text_right()))
        .when(system.uses_grades(), |d| {
            d.child(match grade {
                Some(g) => div().w(px(90.)).child(Input::new(&g).small().text_right()).when(failing, |d| d.text_color(t.red)).into_any_element(),
                None => div().id(SharedString::from(format!("computed-{id}"))).w(px(90.)).text_right().child(m.effective_grade().map_or("–".into(), |g| format_grade(g, system))).tooltip(tip(tr("Aus Teilleistungen berechnet"))).into_any_element(),
            })
        })
        .child(div().w(px(90.)).flex().justify_center().child(Checkbox::new(SharedString::from(format!("passed-{id}"))).checked(m.passed).on_click(move |v, _, cx| {
            let (id, v) = (i3.clone(), *v);
            edit_config(cx, |c| {
                if let Some(m) = c.module_mut(&id) {
                    m.passed = v;
                }
            });
        })));
    div().flex().flex_col().gap(px(8.)).px(px(14.)).py(px(8.)).child(header).when(expanded, |d| d.child(div().pl(px(34.)).child(components_editor(m, system, fields, window, cx, t))))
}

fn edit_components(id: &str, cx: &mut App, f: impl FnOnce(&mut Vec<GradeComponent>), system: GradingSystem) {
    let id = id.to_string();
    edit_config(cx, |c| {
        if let Some(m) = c.module_mut(&id) {
            f(&mut m.components);
            if let Some(g) = m.effective_grade().filter(|_| m.components.iter().any(|x| x.grade.is_some())) {
                m.passed = system.passes(g);
            }
        }
    });
}

fn components_editor(m: &crate::core::model::Module, system: GradingSystem, fields: &mut Fields, window: &mut Window, cx: &mut Context<Pages>, t: &Theme) -> Div {
    let mut col = div().flex().flex_col().gap(px(6.));
    for (j, comp) in m.components.iter().enumerate() {
        let id = m.id.clone();
        let (a, b, cc, d) = (id.clone(), id.clone(), id.clone(), id.clone());
        let name = fields.input(&format!("comp-name-{id}-{j}"), &comp.name, &tr("Teilleistung"), window, cx, move |v, cx| edit_components(&a, cx, |cs| if let Some(x) = cs.get_mut(j) { x.name = v }, system));
        let weight = fields.input(&format!("comp-weight-{id}-{j}"), &format_number(comp.weight, 2), &tr("Gewicht"), window, cx, move |v, cx| {
            edit_components(&b, cx, |cs| if let Some(x) = cs.get_mut(j) { x.weight = parse_number(&v).unwrap_or(0.).max(0.) }, system)
        });
        let grade = fields.input(&format!("comp-grade-{id}-{j}"), &comp.grade.map(|g| format_number(g, 2)).unwrap_or_default(), &tr("Note"), window, cx, move |v, cx| {
            edit_components(&cc, cx, |cs| if let Some(x) = cs.get_mut(j) { x.grade = parse_number(&v) }, system)
        });
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().child(Input::new(&name).small()))
                .child(div().id(SharedString::from(format!("w-{id}-{j}"))).w(px(70.)).tooltip(tip(tr("Gewicht, z. B. 60 für 60 %"))).child(Input::new(&weight).small().text_right()))
                .when(system.uses_grades(), |r| r.child(div().w(px(80.)).child(Input::new(&grade).small().text_right())))
                .child(Button::new(SharedString::from(format!("rm-{id}-{j}"))).ghost().small().icon(menu_icon("circle-minus")).tooltip(tr("Teilleistung entfernen")).on_click(move |_, _, cx| {
                    edit_components(&d, cx, |cs| {
                        if j < cs.len() {
                            cs.remove(j);
                        }
                    }, system)
                })),
        );
    }
    let id = m.id.clone();
    let mut footer = div().flex().items_center().gap(px(12.)).child(
        Button::new(SharedString::from(format!("add-comp-{id}"))).ghost().small().icon(menu_icon("plus")).label(tr("Teilleistung hinzufügen")).on_click(move |_, _, cx| {
            edit_components(&id, cx, |cs| cs.push(GradeComponent { name: tr("Klausur"), weight: 100., ..Default::default() }), system)
        }),
    );
    if system.uses_grades() && m.components.len() > 1 {
        let id = m.id.clone();
        let target = fields.input(&format!("target-{}", m.id), &m.target_grade.map(|g| format_number(g, 2)).unwrap_or_default(), &tr("Zielnote"), window, cx, move |v, cx| {
            let id = id.clone();
            edit_config(cx, |c| {
                if let Some(m) = c.module_mut(&id) {
                    m.target_grade = parse_number(&v);
                }
            })
        });
        footer = footer.child(div().flex_1()).child(div().id("target-wrap").w(px(80.)).tooltip(tip(tr("Wunschnote für das Modul"))).child(Input::new(&target).small()));
        if let Some(need) = m.required_for_target() {
            let (lo, hi) = system.range();
            let (too_good, too_easy) = if system.higher_is_better() { (need > hi, need < lo) } else { (need < lo, need > hi) };
            let (text, color) = if too_good {
                (tr("Ziel nicht mehr erreichbar"), t.red)
            } else if too_easy {
                (tr("Ziel schon sicher"), t.secondary)
            } else {
                (trf("Offene Teile brauchen Ø %@", &[&format_grade(need, system)]), t.secondary)
            };
            footer = footer.child(div().text_size(px(font::CALLOUT)).text_color(color).child(text));
        }
    }
    col.child(footer)
}
