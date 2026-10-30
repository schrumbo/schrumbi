use std::collections::HashSet;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::filing_form::{FilingEvent, FilingForm};
use super::state::{self, act};
use super::motion::Soft;
use super::theme::{Theme, font, space, iconsize, radius};
use super::widgets::*;
use crate::core::day;
use crate::core::i18n::{tr, trf};
use crate::core::library;

/// Inbox: files waiting to be filed on the left, the filing form for the selection (or the first file) on the right.
pub struct InboxPage {
    selection: HashSet<PathBuf>,
    form: Option<(Vec<PathBuf>, Entity<FilingForm>, Subscription)>,
    drop: bool,
    focus: FocusHandle,
    scroll: ScrollHandle,
    anchor: ScrollAnchor,
    _sub: Subscription,
}

impl InboxPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let lib = state::lib(cx);
        let scroll = ScrollHandle::new();
        Self { selection: HashSet::new(), form: None, drop: false, focus: cx.focus_handle(), anchor: ScrollAnchor::for_handle(scroll.clone()), scroll, _sub: cx.observe(&lib, |_, _, cx| cx.notify()) }
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
            return;
        }
        let inbox = state::read(cx, |l| l.inbox.clone());
        match k.key.as_str() {
            "up" | "down" if !inbox.is_empty() => {
                let current = inbox.iter().rposition(|p| self.selection.contains(p));
                let next = match (current, k.key.as_str()) {
                    (None, "up") => 0,
                    (None, _) => 1.min(inbox.len() - 1),
                    (Some(i), "up") => i.saturating_sub(1),
                    (Some(i), _) => (i + 1).min(inbox.len() - 1),
                };
                self.selection = HashSet::from([inbox[next].clone()]);
                self.anchor.scroll_to(window, cx);
                cx.notify();
            }
            "space" => {
                if let Some(path) = self.queue(&inbox).first() {
                    let index = inbox.iter().position(|f| f == path).unwrap_or(0);
                    super::preview::show_in(inbox, index, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
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
        self.selection.retain(|p| inbox.contains(p));
        let queue = self.queue(&inbox);
        if self.form.as_ref().is_none_or(|(q, _, _)| *q != queue) {
            self.form = (!queue.is_empty()).then(|| {
                let form = cx.new(|cx| FilingForm::new(queue.clone(), None, window, cx));
                let sub = cx.subscribe(&form, |this, _, _: &FilingEvent, cx| {
                    this.selection.clear();
                    this.form = None;
                    cx.notify();
                });
                (queue.clone(), form, sub)
            });
        }
        let accent = t.accent;
        let (m1, m2) = (me.clone(), me.clone());
        let base = div()
            .id("inbox")
            .size_full()
            .relative()
            .on_drag_move::<ExternalPaths>(move |_, _, cx| m1.update(cx, |p, cx| {
                if !p.drop {
                    p.drop = true;
                    cx.notify();
                }
            }))
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                let paths = paths.paths().to_vec();
                m2.update(cx, |p, cx| {
                    p.drop = false;
                    cx.notify();
                });
                act(cx, |l| l.import_to_inbox(&paths));
            });
        let content: AnyElement = if inbox.is_empty() {
            unavailable("inbox", tr("Inbox leer"), trf("Zieh Dateien hierher oder lege sie in „%@“ ab. Die App schlägt Abgabe, Titel und Namen vor.", &[&inbox_name]), &t)
                .child(Button::new("inbox-add").label(tr("Dateien hinzufügen…")).on_click(|_, _, cx| add_files(cx)))
                .into_any_element()
        } else {
            let mut list = div().flex().flex_col().p(px(space::S));
            for (i, p) in inbox.iter().enumerate() {
                let selected = self.selection.contains(p) || (self.selection.is_empty() && i == 0);
                let created = std::fs::metadata(p).ok().and_then(|m| m.created().or_else(|_| m.modified()).ok()).map(chrono::DateTime::<chrono::Local>::from);
                let (e, path, menu_path) = (me.clone(), p.clone(), p.clone());
                let hover = t.ink.opacity(0.05);
                list = list.child(
                    div()
                        .id(SharedString::from(format!("inbox-{}", p.display())))
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .px(px(space::S))
                        .py(px(space::XS))
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
                            let extend = ev.modifiers().platform || ev.modifiers().control || ev.modifiers().shift;
                            e.update(cx, |this, cx| {
                                if !extend {
                                    this.selection.clear();
                                }
                                if !this.selection.remove(&path) {
                                    this.selection.insert(path.clone());
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
            let count = self.selection.len();
            div()
                .size_full()
                .flex()
                .child(
                    div()
                        .w(px(280.))
                        .h_full()
                        .flex()
                        .flex_col()
                        .border_r_1()
                        .border_color(t.separator)
                        .child(div().relative().flex_1().min_h_0().child(div().id("inbox-list").size_full().overflow_y_scroll().track_focus(&self.focus).track_scroll(&self.scroll).on_key_down(cx.listener(Self::on_key)).child(list)).vertical_scrollbar(&self.scroll))
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
        base.child(content).when(self.drop, |d| {
            d.child(
                div()
                    .absolute()
                    .inset(px(10.))
                    .rounded(px(radius::CARD))
                    .border(px(3.))
                    .border_dashed()
                    .border_color(accent)
                    .bg(accent.opacity(0.06))
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
        })
    }
}
