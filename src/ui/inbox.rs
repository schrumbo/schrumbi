use std::collections::HashSet;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::filing_form::{FilingEvent, FilingForm};
use super::state::{self, act};
use super::motion::{self, Soft};
use super::theme::{Theme, font, space, iconsize, radius};
use super::widgets::*;
use crate::core::day;
use crate::core::i18n::{tr, trf};
use crate::core::library;

/// Height of a row in the file list.
const ROW: f32 = 40.;

/// Inbox: files waiting to be filed on the left, the filing form for the selection (or the first file) on the right.
pub struct InboxPage {
    selection: HashSet<PathBuf>,
    range_start: Option<PathBuf>,
    shown: Vec<PathBuf>,
    leaving: Vec<(usize, PathBuf)>,
    peak: usize,
    cleared: usize,
    form: Option<(Vec<PathBuf>, Entity<FilingForm>, Subscription)>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    anchor: ScrollAnchor,
    _sub: Subscription,
}

impl InboxPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let lib = state::lib(cx);
        let scroll = ScrollHandle::new();
        Self { selection: HashSet::new(), range_start: None, shown: Vec::new(), leaving: Vec::new(), peak: 0, cleared: 0, form: None, focus: cx.focus_handle(), anchor: ScrollAnchor::for_handle(scroll.clone()), scroll, _sub: cx.observe(&lib, |_, _, cx| cx.notify()) }
    }

    /// Takes the keyboard when the page is entered, so its keys work without a click first.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let inbox = state::read(cx, |l| l.inbox.clone());
        if (k.modifiers.platform || k.modifiers.control) && !k.modifiers.alt && k.key == "a" {
            self.selection = inbox.into_iter().collect();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
            return;
        }
        match k.key.as_str() {
            "up" | "down" if !inbox.is_empty() => {
                let current = inbox.iter().rposition(|p| self.selection.contains(p));
                let next = match (current, k.key.as_str()) {
                    (None, "up") => 0,
                    (None, _) => 1.min(inbox.len() - 1),
                    (Some(i), "up") => i.saturating_sub(1),
                    (Some(i), _) => (i + 1).min(inbox.len() - 1),
                };
                self.pick(&inbox, next, k.modifiers.shift);
                self.anchor.scroll_to(window, cx);
                cx.notify();
            }
            "space" => {
                if let Some(path) = self.queue(&inbox).first() {
                    let index = inbox.iter().position(|f| f == path).unwrap_or(0);
                    super::preview::show_in(inbox, index, cx);
                }
            }
            "enter" if !e.is_held => {
                if let Some((_, form, _)) = &self.form {
                    form.update(cx, |f, cx| f.confirm(window, cx));
                }
            }
            "e" | "1" | "2" | "3" if !e.is_held => {
                let Some(form) = self.form.as_ref().map(|(_, form, _)| form.clone()) else { return };
                if !form.update(cx, |f, cx| f.quick_key(&k.key, window, cx)) {
                    return;
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Selects the row at `index`; with `extend` everything between it and where the selection started.
    fn pick(&mut self, inbox: &[PathBuf], index: usize, extend: bool) {
        let start = self.range_start.as_ref().and_then(|s| inbox.iter().position(|p| p == s));
        match start.filter(|_| extend) {
            Some(start) => self.selection = inbox[start.min(index)..=start.max(index)].iter().cloned().collect(),
            None => {
                self.selection = HashSet::from([inbox[index].clone()]);
                self.range_start = Some(inbox[index].clone());
            }
        }
    }

    /// Notes which rows disappeared, so they can fold away, and how far the current round of filing got.
    fn track_changes(&mut self, inbox: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        self.leaving.retain(|(_, p)| !inbox.contains(p));
        let gone: Vec<(usize, PathBuf)> = self.shown.iter().enumerate().filter(|(_, p)| !inbox.contains(p)).map(|(i, p)| (i, p.clone())).collect();
        if !gone.is_empty() && !inbox.is_empty() {
            let paths: Vec<PathBuf> = gone.iter().map(|(_, p)| p.clone()).collect();
            self.leaving.extend(gone);
            let wait = motion::timed(motion::SLOW, cx);
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |this, cx| {
                    this.leaving.retain(|(_, p)| !paths.contains(p));
                    cx.notify();
                });
            })
            .detach();
        }
        if inbox.is_empty() {
            if self.peak > 0 {
                self.cleared = self.peak;
            }
            self.peak = 0;
            self.leaving.clear();
        } else {
            self.cleared = 0;
            self.peak = self.peak.max(inbox.len());
        }
        self.shown = inbox.to_vec();
    }

    fn queue(&self, inbox: &[PathBuf]) -> Vec<PathBuf> {
        let picked: Vec<PathBuf> = inbox.iter().filter(|p| self.selection.contains(*p)).cloned().collect();
        if picked.is_empty() { inbox.iter().take(1).cloned().collect() } else { picked }
    }
}

fn add_files(cx: &mut App) {
    super::platform::pick(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None }, cx, |paths, cx| act(cx, |l| l.import_to_inbox(&paths)));
}

impl Render for InboxPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let me = cx.entity();
        let list_focused = self.focus.is_focused(window);
        let (inbox, inbox_name, now) = state::read(cx, |l| (l.inbox.clone(), l.config.as_ref().map(|c| c.inbox.clone()).unwrap_or_else(|| "_Inbox".into()), l.now));
        self.track_changes(&inbox, window, cx);
        self.selection.retain(|p| inbox.contains(p));
        let queue = self.queue(&inbox);
        let current = self.form.as_ref().is_some_and(|(q, form, _)| {
            let remaining: Vec<&PathBuf> = q.iter().filter(|p| inbox.contains(p)).collect();
            remaining == queue.iter().collect::<Vec<_>>() && form.read(cx).path().is_some_and(|p| queue.contains(p))
        });
        if !current {
            self.form = (!queue.is_empty()).then(|| {
                let form = cx.new(|cx| FilingForm::new(queue.clone(), None, !list_focused, window, cx));
                let sub = cx.subscribe(&form, |this, _, _: &FilingEvent, cx| {
                    this.selection.clear();
                    this.form = None;
                    cx.notify();
                });
                (queue.clone(), form, sub)
            });
        }
        let accent = t.accent;
        let base = div()
            .id("inbox")
            .group("inbox")
            .size_full()
            .relative()
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                let paths = paths.paths().to_vec();
                act(cx, |l| l.import_to_inbox(&paths));
            });
        let content: AnyElement = if inbox.is_empty() {
            let hint = trf("Zieh Dateien hierher oder lege sie in „%@“ ab. Die App schlägt Abgabe, Titel und Namen vor.", &[&inbox_name]);
            if self.cleared > 0 {
                let travel = motion::timed(motion::SLOW, cx);
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(space::S))
                    .p(px(space::XL))
                    .child(icon("circle-check", iconsize::HERO, t.green))
                    .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::BOLD).child(tr("Alles abgelegt")))
                    .child(div().text_color(t.secondary).child(if self.cleared == 1 { tr("1 Datei abgelegt") } else { trf("%lld Dateien abgelegt", &[&self.cleared]) }))
                    .with_animation(("inbox-cleared", self.cleared), Animation::new(travel).with_easing(ease_out_quint()), |d, p| d.opacity(p).mt(px(space::M * (1. - p))))
                    .into_any_element()
            } else {
                unavailable("inbox", tr("Inbox leer"), hint, &t).child(Button::new("inbox-add").label(tr("Dateien hinzufügen…")).on_click(|_, _, cx| add_files(cx))).into_any_element()
            }
        } else {
            let mut list = div().flex().flex_col().p(px(space::S));
            let travel = motion::timed(motion::SLOW, cx);
            let folding = |list: Div, i: usize, leaving: &[(usize, PathBuf)]| {
                leaving.iter().filter(|(at, _)| *at == i).fold(list, |list, (_, p)| {
                    list.child(
                        div()
                            .h(px(ROW))
                            .flex()
                            .flex_none()
                            .overflow_hidden()
                            .items_center()
                            .gap(px(space::S))
                            .px(px(space::S))
                            .child(file_icon(p, iconsize::L))
                            .child(div().flex_1().min_w_0().truncate().child(p.file_name().unwrap_or_default().to_string_lossy().to_string()))
                            .with_animation(SharedString::from(format!("inbox-leaving-{}", p.display())), Animation::new(travel).with_easing(ease_out_quint()), |d, p| d.h(px(ROW * (1. - p))).opacity(1. - p).ml(px(-2. * space::XL * p))),
                    )
                })
            };
            for (i, p) in inbox.iter().enumerate() {
                list = folding(list, i, &self.leaving);
                let selected = self.selection.contains(p) || (self.selection.is_empty() && i == 0);
                let created = std::fs::metadata(p).ok().and_then(|m| m.created().or_else(|_| m.modified()).ok()).map(chrono::DateTime::<chrono::Local>::from);
                let (e, path, menu_path) = (me.clone(), p.clone(), p.clone());
                let hover = t.ink.opacity(0.05);
                list = list.child(
                    div()
                        .id(SharedString::from(format!("inbox-{}", p.display())))
                        .h(px(ROW))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(space::S))
                        .px(px(space::S))
                        .rounded(px(radius::INNER))
                        .cursor_pointer().key_nav()
                        .when(selected, |d| d.bg(if list_focused { t.accent.opacity(0.32) } else { t.ink.opacity(0.09) }).anchor_scroll(Some(self.anchor.clone())))
                        .when(!selected, |d| d.soft(hover, radius::INNER))
                        .child(file_icon(p, iconsize::L))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(space::XXS))
                                .child(div().truncate().child(p.file_name().unwrap_or_default().to_string_lossy().to_string()))
                                .when_some(created, |d, c| d.child(caption(day::due(c, now), &t))),
                        )
                        .on_click(move |ev: &ClickEvent, window, cx| {
                            e.update(cx, |this, cx| window.focus(&this.focus, cx));
                            if ev.click_count() >= 2 {
                                super::preview::show(&path, cx);
                                return;
                            }
                            let (toggle, extend) = (ev.modifiers().platform || ev.modifiers().control, ev.modifiers().shift);
                            let inbox = state::read(cx, |l| l.inbox.clone());
                            e.update(cx, |this, cx| {
                                if toggle {
                                    if !this.selection.remove(&path) {
                                        this.selection.insert(path.clone());
                                    }
                                    this.range_start = Some(path.clone());
                                } else if let Some(index) = inbox.iter().position(|p| *p == path) {
                                    this.pick(&inbox, index, extend);
                                }
                                cx.notify();
                            })
                        })
                        .context_menu(move |menu, _, _| {
                            let (a, b, c, d) = (menu_path.clone(), menu_path.clone(), menu_path.clone(), menu_path.clone());
                            menu.item(PopupMenuItem::new(tr("Öffnen")).on_click(move |_, _, cx| act(cx, |l| l.open_file(&a))))
                                .item(PopupMenuItem::new(tr("Vorschau")).on_click(move |_, _, cx| super::preview::show(&b, cx)))
                                .item(PopupMenuItem::new(super::platform::reveal_label()).on_click(move |_, _, _| library::reveal(&c)))
                                .separator()
                                .item(PopupMenuItem::new(tr("In den Papierkorb")).on_click(move |_, _, cx| act(cx, |l| l.trash(&d))))
                        }),
                );
            }
            for (at, p) in self.leaving.clone() {
                if at >= inbox.len() {
                    list = folding(list, at, &[(at, p)]);
                }
            }
            let count = self.selection.len();
            let done = self.peak.saturating_sub(inbox.len());
            let share = motion::follow("inbox-progress-share", done as f32 / self.peak.max(1) as f32, motion::MEDIUM, window, cx);
            let progress = div()
                .flex()
                .flex_col()
                .gap(px(space::XS))
                .px(px(space::M))
                .pt(px(space::S))
                .child(caption(trf("%lld von %lld erledigt", &[&done, &self.peak]), &t))
                .child(div().h(px(space::XS)).rounded_full().bg(t.ink.opacity(0.08)).child(div().h_full().rounded_full().bg(t.green).w(relative(share))));
            div()
                .size_full()
                .flex()
                .child(
                    div()
                        .w(px(if f32::from(window.viewport_size().width) < 1200. { 220. } else { 280. }))
                        .h_full()
                        .flex()
                        .flex_col()
                        .border_r_1()
                        .border_color(t.separator)
                        .child(div().relative().flex_1().min_h_0().child(div().id("inbox-list").size_full().overflow_y_scroll().track_focus(&self.focus).track_scroll(&self.scroll).on_key_down(cx.listener(Self::on_key)).child(list)).vertical_scrollbar(&self.scroll))
                        .child(motion::reveal("inbox-progress", done > 0, div().pb(px(space::S)).child(progress)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .p(px(space::S))
                                .border_t_1()
                                .border_color(t.separator)
                                .child(Button::new("inbox-add-files").ghost().icon(menu_icon("plus")).label(tr("Dateien hinzufügen…")).on_click(|_, _, cx| add_files(cx)))
                                .child(div().flex_1())
                                .when(count > 1, |d| d.child(caption(trf("%lld ausgewählt", &[&count]), &t))),
                        ),
                )
                .child(div().flex_1().min_w_0().h_full().children(self.form.as_ref().map(|(_, f, _)| f.clone())))
                .into_any_element()
        };
        base.child(content).child(
                div()
                    .absolute()
                    .inset(px(10.))
                    .invisible()
                    .hover(|s| s)
                    .group_drag_over::<ExternalPaths>("inbox", |s| s.visible())
                    .rounded(px(radius::CARD))
                    .border(px(3.))
                    .border_dashed()
                    .border_color(accent)
                    .bg(t.content.blend(accent.opacity(0.12)).opacity(0.94))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(space::S))
                    .text_size(px(font::TITLE3))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(accent)
                    .child(icon("inbox", iconsize::M, accent))
                    .child(tr("In die Inbox legen")),
        )
    }
}
