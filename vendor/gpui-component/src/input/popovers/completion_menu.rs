use std::{ops::Range, rc::Rc, sync::Arc, time::Duration};

use gpui::{
    Action, AnyElement, App, AppContext, Context, DismissEvent, ElementId, Empty, Entity,
    EventEmitter, HighlightStyle, InteractiveElement as _, IntoElement, ParentElement, Pixels,
    Render, RenderOnce, Role, ScrollStrategy, SharedString, StatefulInteractiveElement as _,
    Styled, StyledText, Subscription, TextRun, WeakEntity, Window, deferred, div,
    prelude::FluentBuilder, px, relative,
};
use gpui_base::TestSupportExt as _;
use gpui_base::input::CompletionHighlighter;
use lsp_types::CompletionItem;

const ROW_HEIGHT: Pixels = px(28.);
const ROW_GAP: Pixels = px(2.);
const MAX_MENU_HEIGHT: Pixels = px(255.);
const MENU_CHROME: Pixels = px(12.);
const MIN_MENU_WIDTH: Pixels = px(120.);
const ROW_PADDING: Pixels = px(8.);
const ROW_SPACING: Pixels = px(8.);
const ICON_SIZE: Pixels = px(16.);
const SCROLLBAR_ROOM: Pixels = px(10.);
const POPOVER_GAP: Pixels = px(4.);

fn natural_width(items: &[CompletionItem], window: &mut Window) -> Pixels {
    let style = window.text_style();
    let rem = window.rem_size();
    let mut shape = |text: &str, size: Pixels| {
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(SharedString::from(text.replace('\n', " ")), size, &[run], None)
            .width
    };
    items
        .iter()
        .map(|item| {
            let label = shape(&item.label, rem * 0.875);
            let icon = if item_icon(item).is_some() {
                ICON_SIZE + ROW_SPACING
            } else {
                Pixels::ZERO
            };
            let detail = item
                .detail
                .as_ref()
                .map_or(Pixels::ZERO, |detail| ROW_SPACING + shape(detail, rem * 0.75));
            ROW_PADDING * 2. + icon + label + detail
        })
        .fold(Pixels::ZERO, Pixels::max)
        + MENU_CHROME
}

fn item_icon(item: &CompletionItem) -> Option<SharedString> {
    let icon = item.data.as_ref()?.get("icon")?.as_str()?;
    Some(SharedString::from(icon.to_string()))
}

/// Drop ranges a custom highlighter cannot safely paint, then order the rest.
///
/// A range past the label, or one that begins or ends inside a character,
/// would split a glyph. Empty ranges paint nothing, so they go too. What
/// remains is sorted and merged: GPUI builds text runs assuming highlights
/// are ordered and do not overlap, and an overlap makes the run longer than
/// the label.
fn retained_highlight_ranges(
    label: &str,
    ranges: impl IntoIterator<Item = Range<usize>>,
) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = ranges
        .into_iter()
        .filter(|range| {
            range.start < range.end
                && range.end <= label.len()
                && label.is_char_boundary(range.start)
                && label.is_char_boundary(range.end)
        })
        .collect();
    ranges.sort_by_key(|range| range.start);

    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut() {
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }
    merged
}

use crate::{
    ActiveTheme, Icon, IndexPath, Selectable, ThemeStyled as _, actions, h_flex,
    input::{
        self, EditorState,
        popovers::{CaretAnchoredPopup, editor_popover, render_markdown},
    },
    label::Label,
    list::{List, ListDelegate, ListEvent, ListState},
};

struct ContextMenuDelegate {
    query: SharedString,
    menu: Entity<CompletionMenu>,
    items: Vec<Rc<CompletionItem>>,
    selected_ix: usize,
    highlight: Rc<CompletionHighlighter>,
}

impl ContextMenuDelegate {
    fn set_items(&mut self, items: Vec<CompletionItem>) {
        self.items = items.into_iter().map(Rc::new).collect();
        self.selected_ix = 0;
    }

    fn selected_item(&self) -> Option<&Rc<CompletionItem>> {
        self.items.get(self.selected_ix)
    }
}

#[derive(IntoElement)]
struct CompletionMenuItem {
    ix: usize,
    item: Rc<CompletionItem>,
    children: Vec<AnyElement>,
    selected: bool,
    query: SharedString,
    highlight: Rc<CompletionHighlighter>,
}

impl CompletionMenuItem {
    fn new(
        ix: usize,
        item: Rc<CompletionItem>,
        query: SharedString,
        highlight: Rc<CompletionHighlighter>,
    ) -> Self {
        Self {
            ix,
            item,
            children: vec![],
            selected: false,
            query,
            highlight,
        }
    }
}
impl Selectable for CompletionMenuItem {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl ParentElement for CompletionMenuItem {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}
impl RenderOnce for CompletionMenuItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let item = self.item;

        let deprecated = item.deprecated.unwrap_or(false);
        let highlights = retained_highlight_ranges(
            &item.label,
            (self.highlight)(self.query.as_ref(), item.as_ref()),
        )
        .into_iter()
        .map(|range| {
            (
                range,
                HighlightStyle {
                    color: Some(cx.theme().blue),
                    ..Default::default()
                },
            )
        })
        .collect::<Vec<_>>();

        let radius = cx.theme().radius;
        let hover_layer = {
            let key = ElementId::NamedInteger("completion-item-soft".into(), self.ix as u64);
            let hovered = window.use_keyed_state(key.clone(), cx, |_, _| false);
            let level = gpui_base::transition(
                ElementId::NamedChild(Arc::new(key), "level".into()),
                if *hovered.read(cx) { 1. } else { 0. },
                gpui_base::Transition::new(Duration::from_millis(120)),
                window,
                cx,
            );
            div()
                .id("soft")
                .absolute()
                .inset_0()
                .rounded(radius)
                .when(level > 0. && !self.selected, |this| {
                    this.bg(cx.theme().list_hover).opacity(level.min(1.))
                })
                .on_hover(move |over, _, cx| {
                    hovered.update(cx, |hovered, cx| {
                        *hovered = *over;
                        cx.notify();
                    })
                })
        };

        h_flex()
            .id(self.ix)
            .test_support()
            .cursor_pointer()
            .role(Role::MenuItem)
            .aria_label(item.label.clone())
            .aria_selected(self.selected)
            .relative()
            .gap(ROW_SPACING)
            .px(ROW_PADDING)
            .h(ROW_HEIGHT)
            .mb(ROW_GAP)
            .rounded(radius)
            .text_sm()
            .text_color(cx.theme().foreground)
            .when(self.selected, |this| this.bg(cx.theme().list_active))
            .child(hover_layer)
            .when_some(item_icon(&item), |this, icon| {
                this.child(
                    Icon::empty()
                        .path(icon)
                        .size(ICON_SIZE)
                        .flex_none()
                        .text_color(cx.theme().muted_foreground),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(deprecated, |this| this.line_through())
                    .child(StyledText::new(item.label.clone()).with_highlights(highlights)),
            )
            .when_some(item.detail.as_ref(), |this, detail| {
                this.child(
                    div().min_w_0().max_w(relative(0.5)).truncate().child(
                        Label::new(detail.clone())
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .when(deprecated, |this| this.line_through()),
                    ),
                )
            })
            .children(self.children)
    }
}

impl EventEmitter<DismissEvent> for ContextMenuDelegate {}

impl ListDelegate for ContextMenuDelegate {
    type Item = CompletionMenuItem;

    fn items_count(&self, _: usize, _: &gpui::App) -> usize {
        self.items.len()
    }

    fn render_item(
        &mut self,
        ix: crate::IndexPath,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let item = self.items.get(ix.row)?;
        Some(CompletionMenuItem::new(
            ix.row,
            item.clone(),
            self.query.clone(),
            self.highlight.clone(),
        ))
    }

    fn set_selected_index(
        &mut self,
        ix: Option<crate::IndexPath>,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_ix = ix.map(|i| i.row).unwrap_or(0);
        cx.notify();
    }

    fn confirm(&mut self, _: bool, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        let Some(item) = self.selected_item() else {
            return;
        };

        self.menu.update(cx, |this, cx| {
            this.select_item(&item, window, cx);
        });
    }
}

/// A context menu for code completions and code actions.
pub struct CompletionMenu {
    offset: usize,
    editor: WeakEntity<EditorState>,
    list: Entity<ListState<ContextMenuDelegate>>,
    open: bool,

    /// The offset of the first character that triggered the completion.
    pub(crate) trigger_start_offset: Option<usize>,
    query: SharedString,
    opened_with: usize,
    natural_width: Pixels,
    _subscriptions: Vec<Subscription>,
}

impl CompletionMenu {
    /// Creates a new `CompletionMenu` with the given offset and completion items.
    ///
    /// NOTE: This element should not call from EditorState::new, unless that will stack overflow.
    pub(crate) fn new(
        editor: Entity<EditorState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let view = cx.entity();
            let menu = ContextMenuDelegate {
                query: SharedString::default(),
                menu: view,
                items: vec![],
                selected_ix: 0,
                highlight: Rc::new(gpui_base::input::highlight_prefix),
            };

            let list = cx.new(|cx| ListState::new(menu, window, cx));

            let _subscriptions =
                vec![
                    cx.subscribe(&list, |this: &mut Self, _, ev: &ListEvent, cx| {
                        match ev {
                            ListEvent::Confirm(_) => {
                                this.dismiss(cx);
                            }
                            _ => {}
                        }
                        cx.notify();
                    }),
                ];

            Self {
                offset: 0,
                editor: editor.downgrade(),
                list,
                open: false,
                trigger_start_offset: None,
                query: SharedString::default(),
                opened_with: 0,
                natural_width: MIN_MENU_WIDTH,
                _subscriptions,
            }
        })
    }

    fn select_item(&mut self, item: &CompletionItem, window: &mut Window, cx: &mut Context<Self>) {
        let item = item.clone();
        let range = self.trigger_start_offset.unwrap_or(self.offset)..self.offset;

        let editor = self.editor.clone();

        cx.spawn_in(window, async move |_, cx| {
            editor.update_in(cx, |editor, window, cx| {
                editor.insert_completion(&item, range, window, cx);
            })
        })
        .detach();

        self.dismiss(cx);
    }

    pub(crate) fn handle_action(
        &mut self,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.open {
            return false;
        }

        if input::Enter::is_primary(&*action) {
            self.on_action_enter(window, cx);
        } else if action.partial_eq(&input::Escape) {
            self.on_action_escape(window, cx);
        } else if action.partial_eq(&input::MoveUp) {
            self.on_action_up(window, cx);
        } else if action.partial_eq(&input::MoveDown) {
            self.on_action_down(window, cx);
        } else {
            return false;
        }

        true
    }

    fn on_action_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.list.read(cx).delegate().selected_item().cloned() else {
            return;
        };
        self.select_item(&item, window, cx);
    }

    fn on_action_escape(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(cx);
    }

    fn on_action_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |this, cx| {
            this.on_action_select_prev(&actions::SelectUp, window, cx)
        });
    }

    fn on_action_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |this, cx| {
            this.on_action_select_next(&actions::SelectDown, window, cx)
        });
    }

    /// Synchronize a hidden editor menu without dismissing a newer response.
    pub(crate) fn hide(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.trigger_start_offset = None;
        cx.notify();
    }

    /// Dismiss the menu in response to a user action.
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.hide(cx);
        let editor = self.editor.clone();
        cx.spawn(async move |_, cx| {
            let _ = editor.update(cx, |editor, cx| editor.dismiss_completion_overlay(cx));
        })
        .detach();
    }

    /// Sets the trigger start offset if it is not already set.
    pub(crate) fn update_query(&mut self, start_offset: usize, query: impl Into<SharedString>) {
        if self.trigger_start_offset.is_none() {
            self.trigger_start_offset = Some(start_offset);
        }
        self.query = query.into();
    }

    pub(crate) fn show(
        &mut self,
        offset: usize,
        items: impl Into<Vec<CompletionItem>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = items.into();
        let width = natural_width(&items, window);
        if self.open {
            self.natural_width = self.natural_width.max(width);
        } else {
            self.opened_with = items.len();
            self.natural_width = width;
        }
        self.offset = offset;
        self.open = true;
        self.list.update(cx, |this, cx| {
            let longest_ix = items
                .iter()
                .enumerate()
                .max_by_key(|(_, item)| {
                    item.label.len() + item.detail.as_ref().map(|d| d.len()).unwrap_or(0)
                })
                .map(|(ix, _)| ix)
                .unwrap_or(0);

            this.delegate_mut().query = self.query.clone();
            this.delegate_mut().set_items(items);
            this.set_selected_index(Some(IndexPath::new(0)), window, cx);
            this.scroll_to_item(IndexPath::new(0), ScrollStrategy::Top, window, cx);
            this.set_item_to_measure_index(IndexPath::new(longest_ix), window, cx);
        });

        cx.notify();
    }

    /// Replace the label highlighter and repaint an open menu.
    pub(crate) fn set_highlight(
        &mut self,
        highlight: Rc<CompletionHighlighter>,
        cx: &mut Context<Self>,
    ) {
        self.list.update(cx, |list, cx| {
            if Rc::ptr_eq(&list.delegate().highlight, &highlight) {
                return;
            }
            list.delegate_mut().highlight = highlight;
            cx.notify();
        });
    }
}

impl Render for CompletionMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return Empty.into_any_element();
        }

        if self.list.read(cx).delegate().items.is_empty() {
            self.open = false;
            return Empty.into_any_element();
        }

        let Some(editor) = self.editor.upgrade() else {
            return Empty.into_any_element();
        };
        // The anchor is resolved in the popup's deferred prepaint from the
        // input's current-frame geometry; the content below is built there.
        let configured_max = editor.read(cx).lsp().completion_menu.max_width;
        let rows = self.list.read(cx).delegate().items.len();
        let natural = self.natural_width;
        let list = self.list.clone();
        let menu = cx.weak_entity();

        deferred(
            CaretAnchoredPopup::new(self.editor.clone(), move |anchor, room, window, cx| {
                // `anchor` is in window coordinates, from the current frame.
                let max_width =
                    configured_max.min(window.bounds().size.width - POPOVER_GAP * 2.);
                let list_height = MAX_MENU_HEIGHT.min(room - MENU_CHROME).max(ROW_HEIGHT);
                let scrolls = (ROW_HEIGHT + ROW_GAP) * rows as f32 > list_height;
                let vertical_layout =
                    anchor.x + configured_max + POPOVER_GAP + configured_max + POPOVER_GAP
                        > window.bounds().size.width;

                let selected_documentation = list
                    .read(cx)
                    .delegate()
                    .selected_item()
                    .and_then(|item| item.documentation.clone());

                div()
                    .flex()
                    .flex_row()
                    .gap(POPOVER_GAP)
                    .items_start()
                    .when(vertical_layout, |this| this.flex_col())
                    .child(
                        editor_popover("completion-menu", cx)
                            .popover_style(cx)
                            .text_sm()
                            // Each virtual row includes the menu's inter-item gap.
                            .pb_0p5()
                            .test_support()
                            .w((natural + if scrolls { SCROLLBAR_ROOM } else { Pixels::ZERO })
                                .clamp(MIN_MENU_WIDTH, max_width.max(MIN_MENU_WIDTH)))
                            .child(
                                List::new(&list)
                                    .max_h(list_height)
                                    .when(scrolls, |this| this.pr(SCROLLBAR_ROOM)),
                            ),
                    )
                    .when_some(selected_documentation, |this, documentation| {
                        let mut doc = match documentation {
                            lsp_types::Documentation::String(s) => s.clone(),
                            lsp_types::Documentation::MarkupContent(mc) => mc.value.clone(),
                        };
                        if vertical_layout {
                            doc = doc.split("\n").next().unwrap_or_default().to_string();
                        }

                        this.child(
                            div().child(
                                editor_popover("completion-menu", cx)
                                    .popover_style(cx)
                                    .text_sm()
                                    .w(configured_max)
                                    .px_2()
                                    .child(render_markdown("doc", doc, window, cx)),
                            ),
                        )
                    })
                    .on_mouse_down_out({
                        let menu = menu.clone();
                        move |_, _, cx: &mut App| {
                            if let Some(menu) = menu.upgrade() {
                                menu.update(cx, |menu, cx| menu.dismiss(cx));
                            }
                        }
                    })
                    .into_any_element()
            })
            .planned_height(
                ((ROW_HEIGHT + ROW_GAP) * self.opened_with as f32).min(MAX_MENU_HEIGHT)
                    + MENU_CHROME,
            ),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_ranges_are_ordered_and_do_not_overlap() {
        assert_eq!(
            retained_highlight_ranges("flag", [2..3, 0..1]),
            vec![0..1, 2..3]
        );
        assert_eq!(retained_highlight_ranges("flag", [0..3, 2..4]), vec![0..4]);
        assert_eq!(
            retained_highlight_ranges("flag", [0..1, 1..2, 5..9, 2..3]),
            vec![0..3]
        );
        assert_eq!(retained_highlight_ranges("你", [0..1, 0..3]), vec![0..3]);
    }

    #[gpui::test]
    fn hiding_an_old_menu_does_not_dismiss_a_new_completion_response(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(crate::init);
        let window = cx.add_empty_window();
        let (editor, menu) = window.update(|window, cx| {
            let editor = cx.new(|cx| EditorState::new(window, cx).default_value("co"));
            let menu = CompletionMenu::new(editor.clone(), window, cx);
            let old_items = vec![CompletionItem {
                label: "const".into(),
                ..Default::default()
            }];
            editor.update(cx, |editor, cx| {
                editor.present_completion_items(0, "c", old_items.clone(), cx);
            });
            menu.update(cx, |menu, cx| {
                menu.update_query(0, "c");
                menu.show(1, old_items, window, cx);
                // The overlay observes the old request being hidden while a
                // replacement request is pending. Its deferred dismissal must
                // not act on the replacement response that arrives next.
                menu.hide(cx);
            });
            editor.update(cx, |editor, cx| {
                editor.present_completion_items(
                    0,
                    "co",
                    vec![CompletionItem {
                        label: "const".into(),
                        ..Default::default()
                    }],
                    cx,
                );
            });
            (editor, menu)
        });

        cx.run_until_parked();

        cx.read(|cx| {
            let completion = editor.read(cx).completion_menu_state();
            assert!(completion.open, "an old menu dismissed the new response");
            assert_eq!(completion.query, "co");
            assert_eq!(completion.items.len(), 1);
            assert_eq!(completion.items[0].label, "const");
        });
        drop(menu);
    }
}
