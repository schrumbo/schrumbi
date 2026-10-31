use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::markdown_view::{self, ParsedNote};
use super::pdf_view::PdfView;
use super::preview::FilePreview;
use super::state::{self, act};
use super::motion::Soft;
use crate::core::vault::index;
use super::theme::{Theme, font, radius, space, iconsize};
use super::widgets::*;
use crate::core::edit_ops::{self, Edit};
use crate::core::i18n::{tr, trf};
use crate::core::library;
use crate::core::markdown;
use crate::core::naming::extension;
use crate::core::prefs::{self, NoteSaveMode};
use crate::core::search_index;
use crate::core::shortcuts::ShortcutAction;

actions!(
    schrumbi_editor,
    [
        Bold, Italic, InsertLink, InlineCode, ToggleTask, ToggleComment, MoveLineUp, MoveLineDown, CopyLineUp, CopyLineDown, DeleteLine, SelectLine, IndentLines, OutdentLines, ZoomIn, ZoomOut, ZoomReset,
        ListNewline, ListTab, ListBacktab, ExtractSelection
    ]
);

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Read,
    Edit,
    Split,
}

/// A Markdown note: read, edit or both side by side. Saving follows Settings → Notizen.
pub struct NoteView {
    /// A nearly empty note opens ready to continue writing below what is there.
    caret_to_end: bool,
    reader_scroll: ScrollHandle,
    heading_anchors: Vec<ScrollAnchor>,
    found: Option<markdown_view::Found>,
    /// Editor and reader offsets at the last sync of the split view.
    split_sync: (f32, f32),
    link_focus: markdown_view::LinkFocus,
    path: PathBuf,
    mode: Mode,
    editor: Entity<EditorState>,
    parsed: Option<(u64, Rc<ParsedNote>)>,
    revision: u64,
    syncing: bool,
    autosave: Option<Task<()>>,
    reparse: Option<Task<()>>,
    /// Shown inside the main window with tabs, or in its own window.
    pub embedded: bool,
    focus: FocusHandle,
    needs_focus: bool,
    window_title: String,
    _subs: Vec<Subscription>,
}

impl NoteView {
    pub fn new(path: PathBuf, embedded: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = state::lib(cx).update(cx, |l, _| l.buffer(&path).text.clone());
        let editor = cx.new(|cx| EditorState::new(window, cx).language("markdown").soft_wrap(true).line_number(true).default_value(text.clone()));
        editor.update(cx, |s, _| s.lsp_mut().completion_provider = Some(super::completion::provider(path.clone())));
        let lib = state::lib(cx);
        let p = path.clone();
        let subs = vec![
            cx.subscribe_in(&editor, window, |this, e, ev: &InputEvent, _, cx| match ev {
                InputEvent::Change if !this.syncing => {
                    let text = e.read(cx).value().to_string();
                    let path = this.path.clone();
                    state::lib(cx).update(cx, |l, cx| {
                        if let Some(b) = l.buffers.get_mut(&path) {
                            b.edited(text);
                        }
                        cx.notify();
                    });
                    this.schedule_autosave(cx);
                    this.schedule_reparse(cx);
                }
                InputEvent::Blur => this.focus_lost(cx),
                _ => {}
            }),
            cx.observe_in(&lib, window, move |this, lib, window, cx| {
                let Some((rev, text)) = lib.read(cx).buffers.get(&p).map(|b| (b.revision, b.text.clone())) else { return };
                if rev != this.revision {
                    this.revision = rev;
                    this.syncing = true;
                    this.editor.update(cx, |s, cx| {
                        let old = s.value().to_string();
                        let (changed, inserted) = edit_ops::minimal_change(&old, &text);
                        if changed.is_empty() && inserted.is_empty() {
                            return;
                        }
                        let (selected, scroll) = (s.selected_range(), s.scroll_offset());
                        let start = edit_ops::shift_offset(selected.start, &changed, inserted.len());
                        let end = edit_ops::shift_offset(selected.end, &changed, inserted.len());
                        s.set_selected_range(changed.clone(), cx);
                        s.replace(inserted.to_string(), window, cx);
                        s.set_selected_range(start..end, cx);
                        s.set_scroll_offset(scroll, cx);
                    });
                    this.syncing = false;
                    this.parsed = None;
                }
                cx.notify();
            }),
            cx.observe(&editor, |this, _, cx| {
                if this.mode == Mode::Split {
                    cx.notify();
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.focus_lost(cx);
                } else {
                    let path = this.path.clone();
                    state::lib(cx).update(cx, |l, cx| {
                        if l.buffers.get_mut(&path).is_some_and(|b| b.reload_if_changed()) {
                            cx.notify();
                        }
                    });
                }
            }),
        ];
        let mode = if text.trim().chars().count() < 40 { Mode::Edit } else { Mode::Read };
        let revision = state::lib(cx).update(cx, |l, _| l.buffer(&path).revision);
        let reader_scroll = ScrollHandle::new();
        let own_path = dunce::canonicalize(&path).unwrap_or_else(|_| path.clone());
        Self { path, caret_to_end: mode == Mode::Edit, mode, editor, reader_scroll: reader_scroll.clone(), heading_anchors: Vec::new(), found: None, split_sync: (0., 0.), link_focus: markdown_view::LinkFocus { index: None, anchor: ScrollAnchor::for_handle(reader_scroll.clone()), found: Rc::default(), note: Some(own_path) }, parsed: None, revision, syncing: false, autosave: None, reparse: None, embedded, focus: cx.focus_handle(), needs_focus: !embedded, window_title: String::new(), _subs: subs }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Selects a search match in the editor and scrolls the reader to its block.
    fn reveal(&mut self, term: &str, nth: usize, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = search_index::occurrences(text, term).into_iter().nth(nth) else { return };
        self.editor.update(cx, |s, cx| s.set_selected_range(range.clone(), cx));
        let token = self.found.as_ref().map_or(0, |f| f.token + 1);
        let anchor = ScrollAnchor::for_handle(self.reader_scroll.clone());
        anchor.scroll_to(window, cx);
        self.found = Some(markdown_view::Found { block: markdown_view::block_at(text, range.start), anchor, token });
    }

    /// Keeps editor and reader at the same relative position; whichever moved last leads.
    fn sync_split(&mut self, text: &str, cx: &mut Context<Self>) {
        let (editor_y, line) = {
            let e = self.editor.read(cx);
            (-f32::from(e.scroll_offset().y), e.line_height().map_or(0., f32::from))
        };
        let reader_y = -f32::from(self.reader_scroll.offset().y);
        let reader_max = f32::from(self.reader_scroll.max_offset().y);
        let (last_editor, last_reader) = self.split_sync;
        let (editor_moved, reader_moved) = ((editor_y - last_editor).abs() > 1., (reader_y - last_reader).abs() > 1.);
        if line <= 0. || reader_max <= 0. || !(editor_moved || reader_moved) {
            return;
        }
        let view = self.reader_scroll.bounds().size;
        let rows = wrapped_rows(text, f32::from(view.width), prefs::get().editor_font_size);
        let editor_max = (rows * line - f32::from(view.height)).max(1.);
        if editor_moved {
            let y = (editor_y / editor_max).clamp(0., 1.) * reader_max;
            self.reader_scroll.set_offset(point(px(0.), px(-y)));
            self.split_sync = (editor_y, y);
        } else {
            let y = (reader_y / reader_max).clamp(0., 1.) * editor_max;
            self.editor.update(cx, |e, cx| e.set_scroll_offset(point(px(0.), px(-y)), cx));
            self.split_sync = (y, reader_y);
        }
    }

    /// Focuses the editor when editing, otherwise the note itself so its shortcuts work.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Mode::Read {
            window.focus(&self.focus, cx);
        } else {
            self.editor.update(cx, |s, cx| {
                s.focus(window, cx);
                if std::mem::take(&mut self.caret_to_end) {
                    let end = s.value().len();
                    s.set_selected_range(end..end, cx);
                }
            });
        }
    }

    pub fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode != Mode::Read {
            self.editor.update(cx, |s, cx| s.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
            self.focus_lost(cx);
        }
        cx.notify();
    }

    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        let p = prefs::get();
        if p.note_save_mode != NoteSaveMode::Delay {
            return;
        }
        let delay = Duration::from_secs_f64(p.note_save_delay.max(1.));
        self.autosave = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |n, cx| n.save(cx));
        }));
    }

    fn schedule_reparse(&mut self, cx: &mut Context<Self>) {
        if self.mode != Mode::Split {
            self.parsed = None;
            return;
        }
        self.reparse = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let _ = this.update(cx, |n, cx| {
                n.parsed = None;
                cx.notify();
            });
        }));
    }

    fn focus_lost(&mut self, cx: &mut Context<Self>) {
        self.save(cx);
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        let result = state::lib(cx).update(cx, |l, cx| {
            let r = l.buffers.get_mut(&path).map_or(Ok(()), |b| b.save());
            cx.notify();
            r
        });
        if let Err(e) = result {
            act(cx, |l| l.last_error = Some(trf("Notiz konnte nicht gesichert werden: %@", &[&e])));
        }
    }

    fn conflict_notice(&self, open: bool, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (compare, reload, keep) = (self.path.clone(), self.path.clone(), self.path.clone());
        let actions = div()
            .flex()
            .items_center()
            .gap(px(space::S))
            .child(Button::new("conflict-compare").small().label(tr("Vergleichen")).on_click(move |_, _, cx| compare_note(compare.clone(), cx)))
            .child(Button::new("conflict-reload").small().label(tr("Neu laden")).on_click(move |_, _, cx| act(cx, |l| l.load_note_disk(&reload))))
            .child(Button::new("conflict-keep").small().primary().label(tr("Meine behalten")).on_click(move |_, _, cx| act(cx, |l| l.keep_note_mine(&keep))));
        super::motion::reveal("note-conflict", open, div().p(px(space::S)).child(notice("triangle-alert", t.orange, tr("Diese Notiz wurde außerhalb von Schrumbi geändert."), Some(tr("Deine Änderungen sind noch da. Gesichert wird erst, wenn du dich entschieden hast.")), actions, t)))
    }

    /// ⌘-click in the editor: follows the link under the caret.
    fn open_link_at_caret(&mut self, cx: &mut Context<Self>) {
        let (text, range) = self.with_text(cx);
        let Some(link) = crate::core::vault::links::find_links(&text).into_iter().find(|l| l.range.start <= range.start && range.start <= l.range.end) else { return };
        let destination = format!("{}{}", link.target.path, link.target.subpath());
        let base = self.path.parent().map(Path::to_path_buf);
        markdown_view::link_action(&destination, base.as_deref())(cx);
    }

    /// Tab walks through the links of the reader.
    fn step_link(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.link_focus.found.borrow().len();
        if self.mode != Mode::Read || count == 0 {
            return;
        }
        let next = match (self.link_focus.index, back) {
            (None, false) => 0,
            (None, true) => count - 1,
            (Some(i), false) => (i + 1) % count,
            (Some(i), true) => (i + count - 1) % count,
        };
        self.link_focus.index = Some(next);
        super::link_card::hide(cx);
        let anchor = self.link_focus.anchor.clone();
        window.on_next_frame(move |window, cx| anchor.scroll_to(window, cx));
        cx.notify();
    }

    /// Enter follows the focused link, Space shows the preview, Escape ends.
    fn link_keys(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::Read || window.has_active_dialog(cx) {
            return;
        }
        let k = &e.keystroke;
        let url = self.link_focus.index.and_then(|i| self.link_focus.found.borrow().get(i).cloned());
        match k.key.as_str() {
            "enter" if url.is_some() => {
                let base = self.path.parent().map(Path::to_path_buf);
                let action = markdown_view::link_action(url.as_deref().unwrap_or_default(), base.as_deref());
                action(cx);
                self.link_focus.index = None;
                if k.modifiers.platform || k.modifiers.control {
                    let opened = state::read(cx, |l| l.active_document.clone());
                    let here = self.path.clone();
                    act(cx, |l| {
                        l.open_document(&here);
                        l.set_split(opened);
                    });
                }
            }
            "space" if url.is_some() => {
                let position = window.mouse_position();
                super::link_card::show_now(url.unwrap_or_default(), self.path.parent().map(Path::to_path_buf), position, cx);
            }
            "escape" if self.link_focus.index.is_some() => {
                self.link_focus.index = None;
                super::link_card::hide(cx);
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        self.save(cx);
        super::pdf::export(&self.path, Some(text), cx);
    }

    fn apply(&mut self, edit: Edit, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |s, cx| {
            s.set_selected_range(edit.range.clone(), cx);
            s.replace(edit.text.clone(), window, cx);
            s.set_selected_range(edit.select.clone(), cx);
        });
    }

    fn with_text(&mut self, cx: &mut Context<Self>) -> (String, std::ops::Range<usize>) {
        let s = self.editor.read(cx);
        (s.value().to_string(), s.selected_range())
    }

    fn zoom(delta: f32, cx: &mut App) {
        let size = if delta == 0. { 13. } else { (prefs::get().editor_font_size + delta).clamp(10., 24.) };
        set_font_size(size, cx);
    }

    fn header(&self, exporting: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let me = cx.entity();
        let mut modes = div().flex().rounded(px(radius::INNER)).bg(t.ink.opacity(0.06)).p(px(space::XXS));
        let help = ShortcutAction::TogglePreview.shortcut().map_or_else(|| tr("Lesen, Bearbeiten oder beides"), |s| trf("Lesen, Bearbeiten oder beides (%@ wechselt)", &[&s.display()]));
        for (m, label, icon_name) in [(Mode::Read, tr("Lesen"), "eye"), (Mode::Edit, tr("Bearbeiten"), "pencil"), (Mode::Split, tr("Geteilt"), "columns-2")] {
            let active = m == self.mode;
            let e = me.clone();
            modes = modes.child(
                div()
                    .id(SharedString::from(format!("mode-{label}")))
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .px(px(space::S))
                    .py(px(space::XXS))
                    .rounded(px(radius::INNER))
                    .cursor_pointer().key_nav()
                    .text_size(px(font::CALLOUT))
                    .when(active, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm().font_weight(FontWeight::MEDIUM))
                    .child(icon(icon_name, iconsize::XS, if active { t.primary } else { t.secondary }))
                    .child(label)
                    .tooltip(tip(help.clone()))
                    .on_click(move |_, window, cx| e.update(cx, |n, cx| n.set_mode(m, window, cx))),
            );
        }
        let (e1, path, p2, p3) = (me.clone(), self.path.clone(), self.path.clone(), self.path.clone());
        let embedded = self.embedded;
        let in_vault = state::read(cx, |l| l.is_vault) && self.path.starts_with(&index::current().root);
        let headings = self.parsed.as_ref().filter(|_| self.mode != Mode::Edit).map(|(_, p)| p.headings()).unwrap_or_default();
        let anchors = self.heading_anchors.clone();
        let toc = (headings.len() >= 2 && anchors.len() == headings.len()).then(|| {
            Button::new("note-toc").ghost().small().icon(menu_icon("list")).tooltip(tr("Inhalt")).dropdown_menu(move |mut menu, _, _| {
                let top = headings.iter().map(|(level, _)| *level).min().unwrap_or(1);
                for ((level, text), anchor) in headings.iter().zip(&anchors) {
                    let anchor = anchor.clone();
                    let indent = "    ".repeat((level - top) as usize);
                    menu = menu.item(PopupMenuItem::new(format!("{indent}{text}")).on_click(move |_, window, cx| anchor.scroll_to(window, cx)));
                }
                menu
            })
        });
        div()
            .flex()
            .items_center()
            .gap(px(space::S))
            .child(modes)
            .children(toc)
            .child(Button::new("note-pdf").small().icon(menu_icon("share")).label("PDF").tooltip(tr("Als PDF neben der Notiz sichern (⇧⌘E)")).disabled(exporting).on_click(move |_, _, cx| e1.update(cx, |n, cx| n.export(cx))))
            .child(Button::new("note-more").ghost().small().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu(move |menu, _, _| {
                let (a, b, c, d) = (path.clone(), p2.clone(), p3.clone(), path.clone());
                let mut menu = menu;
                if embedded {
                    menu = menu.item(PopupMenuItem::new(tr("In eigenem Fenster öffnen")).icon(menu_icon("app-window")).on_click(move |_, _, cx| {
                        super::note_window::open(&a, cx);
                        act(cx, |l| l.close_document(&a));
                    }));
                }
                menu = menu
                    .item(PopupMenuItem::new(super::platform::reveal_label()).icon(menu_icon("folder")).on_click(move |_, _, _| library::reveal(&b)))
                    .item(PopupMenuItem::new(tr("Mit Standard-App öffnen")).icon(menu_icon("square-arrow-out-up-right")).on_click(move |_, _, _| library::open_path(&c)));
                if in_vault {
                    let (o, h, g) = (d.clone(), d.clone(), d.clone());
                    menu = menu
                        .item(PopupMenuItem::new(tr("In Obsidian öffnen")).icon(menu_icon("external-link")).on_click(move |_, _, cx| cx.open_url(&crate::core::vault::config::open_uri(&o))))
                        .item(PopupMenuItem::new(tr("Graph um diese Notiz")).icon(menu_icon("waypoints")).on_click(move |_, _, cx| {
                            let p = g.clone();
                            act(cx, |l| l.show_graph(Some(p)));
                        }))
                        ;
                    let _ = h;
                }
                menu = menu.item(PopupMenuItem::new(tr("Verlauf")).icon(menu_icon("history")).on_click(|_, _, cx| {
                    act(cx, |l| {
                        l.show_inspector = true;
                        l.show_versions = true;
                    });
                }));
                if embedded {
                    menu = menu.separator().item(PopupMenuItem::new(tr("Tab schließen")).icon(menu_icon("x")).on_click(move |_, _, cx| close_tab(&d, cx)));
                }
                menu
            }))
            .into_any_element()
    }

    fn reader(&mut self, lib_text: &str, t: &Theme, _cx: &mut Context<Self>) -> AnyElement {
        if lib_text.trim().is_empty() {
            return div().p(px(space::XXL)).text_color(t.secondary).child(tr("Leere Notiz. Zum Schreiben auf „Bearbeiten“ wechseln.")).into_any_element();
        }
        let parsed = match &self.parsed {
            Some((rev, p)) if *rev == self.revision => p.clone(),
            _ => {
                let p = Rc::new(markdown_view::parse(lib_text));
                self.heading_anchors = p.headings().iter().map(|_| ScrollAnchor::for_handle(self.reader_scroll.clone())).collect();
                self.parsed = Some((self.revision, p.clone()));
                p
            }
        };
        let path = self.path.clone();
        let toggle: markdown_view::OnToggle = Rc::new(move |line, cx: &mut App| {
            let path = path.clone();
            state::lib(cx).update(cx, |l, cx| {
                let b = l.buffer(&path);
                let next = markdown::toggle_task(&b.text, line);
                b.replace(next);
                let _ = b.save();
                cx.notify();
            });
        });
        let base = self.path.parent().map(Path::to_path_buf);
        div()
            .relative()
            .size_full()
            .bg(t.content)
            .child(div().id("note-reader").size_full().overflow_y_scroll().track_scroll(&self.reader_scroll).child(div().w_full().flex().justify_center().child(div().w_full().max_w(px(720.)).px(px(space::XXL)).py(px(space::XL)).child(markdown_view::render_with_anchors(&parsed, base.as_deref(), Some(toggle), &self.heading_anchors, self.found.as_ref(), Some(&self.link_focus), t)))))
            .vertical_scrollbar(&self.reader_scroll)
            .into_any_element()
    }

    /// Copies dropped images next to the note and links them at the cursor.
    fn drop_images(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        let Some(note_folder) = self.path.parent().map(Path::to_path_buf) else { return };
        let vault = index::current();
        let in_vault = state::read(cx, |l| l.is_vault) && self.path.starts_with(&vault.root);
        let folder = if in_vault { vault.config.attachment_folder(&vault.root, &self.path) } else { note_folder };
        let _ = std::fs::create_dir_all(&folder);
        let line = |target: &Path, name: &str| if in_vault { format!("!{}", vault.new_link(Some(&self.path), target, "")) } else { edit_ops::image_markdown(name) };
        let mut lines = Vec::new();
        for image in paths.iter().filter(|p| ["png", "jpg", "jpeg", "gif", "webp", "svg"].contains(&extension(&p.to_string_lossy()).to_lowercase().as_str())) {
            let name = image.file_name().unwrap_or_default().to_string_lossy().to_string();
            let target = library::unique_path(&folder.join(&name));
            if *image == target || image.parent() == Some(folder.as_path()) {
                lines.push(line(image, &name));
            } else if std::fs::copy(image, &target).is_ok() {
                lines.push(line(&target, &target.file_name().unwrap_or_default().to_string_lossy()));
            } else {
                act(cx, |l| l.last_error = Some(trf("Bild konnte nicht kopiert werden: %@", &[&name])));
            }
        }
        if lines.is_empty() {
            return;
        }
        let text = lines.join("\n\n");
        let (_, range) = self.with_text(cx);
        let end = range.start + text.len();
        self.apply(edit_ops::Edit { range, text, select: end..end }, window, cx);
    }

    fn editor_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let accent = Theme::get(cx).accent;
        div()
            .size_full()
            .key_context("NoteEditor")
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(accent.opacity(0.08)))
            .on_drop(cx.listener(|n, paths: &ExternalPaths, window, cx| {
                let paths = paths.paths().to_vec();
                n.drop_images(&paths, window, cx);
            }))
            .on_action(cx.listener(|n, _: &Bold, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::wrap(&s, r, "**"), w, cx)
            }))
            .on_action(cx.listener(|n, _: &Italic, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::wrap(&s, r, "*"), w, cx)
            }))
            .on_action(cx.listener(|n, _: &InlineCode, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::wrap(&s, r, "`"), w, cx)
            }))
            .on_action(cx.listener(|n, _: &InsertLink, w, cx| {
                let (s, r) = n.with_text(cx);
                if state::read(cx, |l| l.is_vault) && !index::current().config.use_markdown_links {
                    n.apply(edit_ops::wikilink(&s, r), w, cx)
                } else {
                    n.apply(edit_ops::link(&s, r), w, cx)
                }
            }))
            .on_action(cx.listener(|n, _: &ExtractSelection, w, cx| {
                let (_, range) = n.with_text(cx);
                let path = n.path.clone();
                if act(cx, |l| l.extract_to_note(&path, range)).is_some() {
                    state::reindex(cx);
                    let text = state::lib(cx).update(cx, |l, _| l.buffer(&path).text.clone());
                    n.editor.update(cx, |e, cx| e.set_value(text, w, cx));
                }
            }))
            .on_action(cx.listener(|n, _: &ToggleTask, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::edit_lines(&s, r, edit_ops::toggled_task), w, cx)
            }))
            .on_action(cx.listener(|n, _: &ToggleComment, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::toggle_comment(&s, r), w, cx)
            }))
            .on_action(cx.listener(|n, _: &MoveLineUp, w, cx| {
                let (s, r) = n.with_text(cx);
                if let Some(e) = edit_ops::move_lines(&s, r, true) {
                    n.apply(e, w, cx)
                }
            }))
            .on_action(cx.listener(|n, _: &MoveLineDown, w, cx| {
                let (s, r) = n.with_text(cx);
                if let Some(e) = edit_ops::move_lines(&s, r, false) {
                    n.apply(e, w, cx)
                }
            }))
            .on_action(cx.listener(|n, _: &CopyLineUp, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::copy_lines(&s, r, true), w, cx)
            }))
            .on_action(cx.listener(|n, _: &CopyLineDown, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::copy_lines(&s, r, false), w, cx)
            }))
            .on_action(cx.listener(|n, _: &DeleteLine, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::delete_lines(&s, r), w, cx)
            }))
            .on_action(cx.listener(|n, _: &SelectLine, _, cx| {
                let (s, r) = n.with_text(cx);
                let sel = edit_ops::select_lines(&s, r);
                n.editor.update(cx, |e, cx| e.set_selected_range(sel, cx));
            }))
            .on_action(cx.listener(|n, _: &IndentLines, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::shift_lines(&s, r, 1), w, cx)
            }))
            .on_action(cx.listener(|n, _: &OutdentLines, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::shift_lines(&s, r, -1), w, cx)
            }))
            .on_action(cx.listener(|n, _: &ListNewline, w, cx| {
                if n.editor.read(cx).completion_menu_state().open {
                    cx.propagate();
                    return;
                }
                let (s, r) = n.with_text(cx);
                match (r.is_empty(), edit_ops::newline(&s, r.start)) {
                    (true, Some(e)) => n.apply(e, w, cx),
                    _ => n.apply(Edit { range: r.clone(), text: "\n".into(), select: r.start + 1..r.start + 1 }, w, cx),
                }
            }))
            .on_action(cx.listener(|n, _: &ListTab, w, cx| {
                if n.editor.read(cx).completion_menu_state().open {
                    cx.propagate();
                    return;
                }
                let (s, r) = n.with_text(cx);
                if edit_ops::tab_indents(&s, &r) {
                    n.apply(edit_ops::shift_lines(&s, r, 1), w, cx)
                } else {
                    n.apply(Edit { range: r.clone(), text: "    ".into(), select: r.start + 4..r.start + 4 }, w, cx)
                }
            }))
            .on_action(cx.listener(|n, _: &ListBacktab, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::shift_lines(&s, r, -1), w, cx)
            }))
            .on_action(|_: &ZoomIn, _, cx| NoteView::zoom(1., cx))
            .on_action(|_: &ZoomOut, _, cx| NoteView::zoom(-1., cx))
            .on_action(|_: &ZoomReset, _, cx| NoteView::zoom(0., cx))
            .on_mouse_up(MouseButton::Left, cx.listener(|n, e: &MouseUpEvent, _, cx| {
                if e.modifiers.platform || e.modifiers.control {
                    n.open_link_at_caret(cx);
                }
            }))
            .child(Editor::new(&self.editor).appearance(false).h_full().bg(Theme::get(cx).content))
            .into_any_element()
    }
}

impl Focusable for NoteView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Estimated rows of soft-wrapped monospaced text in an editor column of `width`, gutter included.
fn wrapped_rows(text: &str, width: f32, font_size: f32) -> f32 {
    let per_row = ((width - 56.) / (font_size * 0.6)).max(20.);
    text.lines().map(|l| (l.chars().count() as f32 / per_row).ceil().max(1.)).sum()
}

/// The search match waiting to be shown in this file, if any.
fn take_find_request(path: &Path, cx: &mut App) -> Option<(String, usize)> {
    state::lib(cx).update(cx, |l, _| {
        let matches = l.find_in_file.as_ref().is_some_and(|(p, _, _)| dunce::canonicalize(p).is_ok_and(|p| p == path));
        if matches { l.find_in_file.take().map(|(_, term, nth)| (term, nth)) } else { None }
    })
}

impl Render for NoteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.needs_focus) {
            self.activate(window, cx);
        }
        let t = Theme::get(cx);
        let lib = state::lib(cx);
        let (missing, text, conflict) = lib.update(cx, |l, _| {
            let b = l.buffer(&self.path);
            (b.missing, b.text.clone(), b.conflict.is_some())
        });
        let request = take_find_request(&self.path, cx);
        if let Some((term, nth)) = request {
            self.reveal(&term, nth, &text, window, cx);
        }
        let exporting = lib.read(cx).exporting_pdfs.contains(&self.path);
        if !self.embedded {
            let (title, relative, dirty) = state::read(cx, |l| {
                let folder = self.path.parent().unwrap_or(&self.path);
                (l.note_title(&self.path), l.relative_path(folder), l.buffers.get(&self.path).is_some_and(|b| b.dirty))
            });
            let full = format!("{title} — {}{relative}", if dirty { "● " } else { "" });
            if full != self.window_title {
                window.set_window_title(&full);
                self.window_title = full;
            }
        }
        let header = self.header(exporting, &t, cx);
        if self.mode == Mode::Split && !missing {
            self.sync_split(&text, cx);
        }
        let undecodable = state::read(cx, |l| l.buffers.get(&self.path).is_some_and(|b| b.undecodable));
        let content: AnyElement = if undecodable {
            unavailable("file-warning", tr("Kodierung wird nicht unterstützt"), tr("Die Datei ist nicht in UTF-8 gespeichert. Sie bleibt unverändert, öffne sie in einer anderen App."), &t).into_any_element()
        } else if missing {
            unavailable("file-question", tr("Datei nicht gefunden"), self.path.file_name().unwrap_or_default().to_string_lossy().to_string(), &t).into_any_element()
        } else {
            match self.mode {
                Mode::Read => self.reader(&text, &t, cx),
                Mode::Edit => div().size_full().flex().flex_col().child(div().flex_1().min_h_0().child(self.editor_view(cx))).child(word_count(&text, &t)).into_any_element(),
                Mode::Split => div().size_full().flex().child(div().flex_1().min_w_0().h_full().flex().flex_col().border_r_1().border_color(t.separator).child(div().flex_1().min_h_0().child(self.editor_view(cx))).child(word_count(&text, &t))).child(div().flex_1().min_w_0().h_full().child(self.reader(&text, &t, cx))).into_any_element(),
            }
        };
        let top = if self.embedded {
            document_bar(&self.path, header, &t, cx)
        } else {
            div().h(px(44.)).flex_none().flex().items_center().justify_center().border_b_1().border_color(t.separator).child(header)
        };
        let hover_card = (!self.embedded).then(|| super::link_card::overlay(cx)).flatten();
        let view = div()
            .size_full()
            .flex()
            .flex_col()
            .children(hover_card)
            .track_focus(&self.focus)
            .key_context("NoteView")
            .on_action(cx.listener(|n, _: &super::app::TogglePreview, w, cx| {
                let m = if n.mode == Mode::Read { Mode::Edit } else { Mode::Read };
                n.set_mode(m, w, cx)
            }))
            .on_action(cx.listener(|n, _: &super::app::SplitPreview, w, cx| {
                let m = if n.mode == Mode::Split { Mode::Edit } else { Mode::Split };
                n.set_mode(m, w, cx)
            }))
            .on_action(cx.listener(|n, _: &super::app::ExportPdf, _, cx| n.export(cx)))
            .on_action(cx.listener(|n, _: &super::app::Save, _, cx| n.save(cx)))
            .on_key_down(cx.listener(|n, e: &KeyDownEvent, window, cx| n.link_keys(e, window, cx)))
            .when(self.mode == Mode::Read, |d| d.key_context("WalkItems"))
            .on_action(cx.listener(|n, _: &super::canvas_view::NextItem, window, cx| n.step_link(false, window, cx)))
            .on_action(cx.listener(|n, _: &super::canvas_view::PreviousItem, window, cx| n.step_link(true, window, cx)))
            .when(!self.embedded, |d| {
                d.on_action(|_: &super::app::CloseTab, window, cx| super::note_window::request_close(window, cx))
                    .on_action(|_: &super::app::CloseWindow, window, cx| super::note_window::request_close(window, cx))
            })
            .child(top)
            .child(self.conflict_notice(conflict, &t, cx))
            .child(div().flex_1().min_h_0().child(content));
        if self.embedded {
            view.into_any_element()
        } else {
            super::chrome::frame_with_close(self.window_title.clone(), view, super::note_window::request_close, &t)
        }
    }
}

/// Sets the editor font size everywhere.
pub fn set_font_size(size: f32, cx: &mut App) {
    prefs::update(|p| p.editor_font_size = size);
    gpui_kit::component::Theme::global_mut(cx).mono_font_size = px(size);
    cx.refresh_windows();
}

/// Opens both versions of a note side by side; the diff is worked out in the background, notes can be long.
pub fn compare_note(path: PathBuf, cx: &mut App) {
    let Some((mine, theirs)) = state::read(cx, |l| l.buffers.get(&path).and_then(|b| b.conflict.clone().map(|c| (b.text.clone(), c)))) else { return };
    let Some(window) = cx.active_window() else { return };
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let task = cx.background_spawn(async move {
        let closed = |t: String| if t.ends_with('\n') { t } else { t + "\n" };
        let (theirs, mine) = (closed(theirs), closed(mine));
        let diff = similar::TextDiff::configure().timeout(Duration::from_secs(2)).diff_lines(&theirs, &mine);
        let patch = diff.unified_diff().header(&name, &name).to_string();
        gpui_kit::component::diff::DiffFile::parse(&patch).unwrap_or_default()
    });
    cx.spawn(async move |cx| {
        let files = task.await;
        let _ = window.update(cx, |_, window, cx| {
            if window.has_active_dialog(cx) {
                return;
            }
            let state = cx.new(|cx| gpui_kit::component::diff::DiffState::new(files, cx).with_mode(gpui_kit::component::diff::DiffMode::Split));
            window.open_dialog(cx, move |d, _, cx| {
                let t = Theme::get(cx);
                let (reload, keep) = (path.clone(), path.clone());
                d.entrance(DialogEntrance::FadeSlide).w(px(760.)).p_0().child(
                    div()
                        .h(px(480.))
                        .flex()
                        .flex_col()
                        .child(div().flex_none().py(px(space::M)).flex().border_b_1().border_color(t.separator).text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).text_color(t.secondary).child(div().flex_1().px(px(space::L)).child(tr("Version der anderen App"))).child(div().flex_1().px(px(space::L)).child(tr("Deine Version"))))
                        .child(div().flex_1().min_h_0().child(gpui_kit::component::diff::Diff::new(&state).header_visible(false).hunk_separator(gpui_kit::component::diff::DiffHunkSeparator::Simple)))
                        .child(
                            div()
                                .flex_none()
                                .p(px(space::M))
                                .flex()
                                .justify_end()
                                .gap(px(space::S))
                                .border_t_1()
                                .border_color(t.separator)
                                .child(Button::new("compare-close").small().label(tr("Später")).on_click(|_, window, cx| window.close_dialog(cx)))
                                .child(Button::new("compare-reload").small().label(tr("Neu laden")).on_click(move |_, window, cx| {
                                    act(cx, |l| l.load_note_disk(&reload));
                                    window.close_dialog(cx);
                                }))
                                .child(Button::new("compare-keep").small().primary().label(tr("Meine behalten")).on_click(move |_, window, cx| {
                                    act(cx, |l| l.keep_note_mine(&keep));
                                    window.close_dialog(cx);
                                })),
                        ),
                )
            });
        });
    })
    .detach();
}

/// Saves for closing, also during a sync. An undecided conflict keeps both versions, because nobody can answer anymore.
fn save_buffer(path: &Path, cx: &mut App) -> bool {
    let result = state::lib(cx).update(cx, |l, cx| {
        let r = l.buffers.get_mut(path).map_or(Ok(None), |b| {
            b.release(false)?;
            b.save()?;
            if b.conflict.is_some() { b.save_conflict_copy().map(Some) } else { Ok(None) }
        });
        cx.notify();
        r
    });
    match result {
        Ok(copy) => {
            if let Some(copy) = copy {
                let name = copy.file_name().unwrap_or_default().to_string_lossy().to_string();
                act(cx, |l| l.notify(trf("Beide Versionen behalten: %@", &[&name]), "copy-plus", None, None));
            }
            true
        }
        Err(e) => {
            act(cx, |l| l.last_error = Some(trf("Notiz konnte nicht gesichert werden: %@", &[&e])));
            false
        }
    }
}

/// Saves the given notes, then runs `done`. A failed save keeps everything open.
pub fn resolve_unsaved(paths: Vec<PathBuf>, cx: &mut App, done: impl FnOnce(&mut App) + 'static) {
    let dirty: Vec<PathBuf> = state::read(cx, |l| paths.into_iter().filter(|p| l.buffers.get(p).is_some_and(|b| b.dirty)).collect());
    if dirty.iter().all(|p| save_buffer(p, cx)) {
        done(cx);
    }
}

/// Saves or asks about the note behind a tab, then closes the tab.
pub fn close_tab(path: &Path, cx: &mut App) {
    let target = path.to_path_buf();
    resolve_unsaved(vec![target.clone()], cx, move |cx| act(cx, |l| l.close_document(&target)));
}

fn close_other_tabs(path: &Path, cx: &mut App) {
    let keep = path.to_path_buf();
    let others = state::read(cx, |l| l.open_documents.iter().filter(|d| **d != keep).cloned().collect());
    resolve_unsaved(others, cx, move |cx| act(cx, |l| l.close_other_documents(&keep)));
}

pub fn close_all_tabs(cx: &mut App) {
    let all = state::read(cx, |l| l.open_documents.clone());
    resolve_unsaved(all, cx, |cx| act(cx, |l| l.close_all_documents()));
}

fn word_count(text: &str, t: &Theme) -> Div {
    let words = text.split_whitespace().count();
    div().flex_none().flex().justify_end().px(px(space::M)).py(px(space::XS)).border_t_1().border_color(t.separator).text_size(px(font::CAPTION)).text_color(t.secondary).child(if words == 1 { tr("1 Wort") } else { trf("%lld Wörter", &[&words]) })
}

/// A tab being moved along the tab bar.
#[derive(Clone)]
struct DraggedTab {
    path: PathBuf,
    title: String,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        div().flex().items_center().gap(px(space::S)).px(px(space::M)).py(px(space::XS)).rounded(px(radius::INNER)).bg(t.material).text_color(t.primary).shadow_md().text_size(px(font::CALLOUT)).child(file_icon(&self.path, iconsize::M)).child(self.title.clone())
    }
}

/// One row: open tabs on the left, the shown document's actions on the right.
pub fn document_bar(current: &Path, actions: AnyElement, t: &Theme, cx: &App) -> Div {
    if state::read(cx, |l| l.split_document.as_deref() == Some(current)) {
        let title = state::read(cx, |l| l.note_title(current));
        return div()
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::M))
            .min_h(px(40.))
            .border_b_1()
            .border_color(t.separator)
            .child(file_icon(current, iconsize::M))
            .child(div().flex_1().min_w_0().truncate().text_size(px(font::CALLOUT)).child(title))
            .child(div().flex_none().child(actions))
            .child(Button::new("split-close").ghost().small().icon(menu_icon("x")).tooltip(tr("Geteilte Ansicht schließen")).on_click(|_, _, cx| act(cx, |l| l.set_split(None))));
    }
    let (docs, titles, dirty) = state::read(cx, |l| {
        let docs = l.open_documents.clone();
        let titles: Vec<String> = docs.iter().map(|d| l.note_title(d)).collect();
        let dirty: Vec<bool> = docs.iter().map(|d| l.buffers.get(d).is_some_and(|b| b.dirty)).collect();
        (docs, titles, dirty)
    });
    let overflow = (docs.len() > 4).then(|| {
        let (all, names, shown) = (docs.clone(), titles.clone(), current.to_path_buf());
        Button::new("doc-tabs-all").ghost().small().icon(menu_icon("chevron-down")).tooltip(trf("Alle %lld Tabs", &[&docs.len()])).dropdown_menu(move |mut menu, _, _| {
            for (doc, title) in all.iter().zip(&names) {
                let open = doc.clone();
                menu = menu.item(PopupMenuItem::new(title.clone()).checked(*doc == shown).on_click(move |_, _, cx| act(cx, |l| l.open_document(&open))));
            }
            menu
        })
    });
    let mut tabs = div().flex().gap(px(space::XXS)).py(px(space::S));
    for (i, doc) in docs.iter().enumerate() {
        let active = doc.as_path() == current;
        let (open, close, menu, middle, target) = (doc.clone(), doc.clone(), doc.clone(), doc.clone(), doc.clone());
        let accent = t.accent;
        let is_dirty = dirty[i];
        let hover = t.ink.opacity(0.05);
        let rel = state::read(cx, |l| l.relative_path(doc));
        tabs = tabs.child(
            div()
                .id(SharedString::from(format!("tab-{}", doc.display())))
                .group("doc-tab")
                .flex()
                .items_center()
                .gap(px(space::S))
                .pl(px(space::M))
                .pr(px(space::S))
                .py(px(space::XS))
                .rounded(px(radius::INNER))
                .cursor_pointer().key_nav()
                .when(active, |d| d.bg(t.ink.opacity(0.1)))
                .when(!active, |d| d.soft(hover, radius::INNER))
                .tooltip(tip(rel))
                .on_click(move |_, _, cx| act(cx, |l| l.open_document(&open)))
                .on_mouse_down(MouseButton::Middle, move |_, _, cx| close_tab(&middle, cx))
                .on_drag(DraggedTab { path: doc.clone(), title: titles[i].clone() }, |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(accent.opacity(0.15)))
                .on_drop(move |dragged: &DraggedTab, _, cx| {
                    let path = dragged.path.clone();
                    act(cx, |l| l.move_document(&path, &target));
                })
                .child(file_icon(doc, iconsize::M))
                .child(div().max_w(px(180.)).truncate().text_size(px(font::CALLOUT)).text_color(if active { t.primary } else { t.secondary }).child(titles[i].clone()))
                .child(
                    div()
                        .id(SharedString::from(format!("tab-close-{}", doc.display())))
                        .size(px(16.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .soft(t.ink.opacity(0.1), radius::FULL)
                        .when(!active && !is_dirty, |d| d.invisible().group_hover("doc-tab", |s| s.visible()))
                        .child(if is_dirty {
                            div().relative().size(px(16.)).flex().items_center().justify_center().child(div().size(px(space::S)).rounded_full().bg(t.secondary).group_hover("doc-tab", |s| s.invisible())).child(div().absolute().invisible().group_hover("doc-tab", |s| s.visible()).child(icon("x", iconsize::XS, t.secondary))).into_any_element()
                        } else {
                            icon("x", iconsize::XS, t.secondary).into_any_element()
                        })
                        .tooltip(tip(if is_dirty { tr("Ungesicherte Änderungen – Tab schließen") } else { tr("Tab schließen") }))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            close_tab(&close, cx)
                        }),
                )
                .context_menu(move |m, _, _| {
                    let (a, b, c, d, beside) = (menu.clone(), menu.clone(), menu.clone(), menu.clone(), menu.clone());
                    m.when(!active, |m| m.item(PopupMenuItem::new(tr("Daneben öffnen")).icon(menu_icon("columns-2")).on_click(move |_, _, cx| act(cx, |l| l.set_split(Some(beside.clone()))))))
                        .item(PopupMenuItem::new(tr("Tab schließen")).on_click(move |_, _, cx| close_tab(&a, cx)))
                        .item(PopupMenuItem::new(tr("Andere Tabs schließen")).on_click(move |_, _, cx| close_other_tabs(&b, cx)))
                        .item(PopupMenuItem::new(tr("Alle Tabs schließen")).on_click(|_, _, cx| close_all_tabs(cx)))
                        .separator()
                        .item(PopupMenuItem::new(super::platform::reveal_label()).on_click(move |_, _, _| library::reveal(&c)))
                        .item(PopupMenuItem::new(tr("Mit Standard-App öffnen")).on_click(move |_, _, _| library::open_path(&d)))
                }),
        );
    }
    div()
        .flex()
        .items_center()
        .gap(px(space::S))
        .pl(px(space::S))
        .pr(px(space::M))
        .min_h(px(40.))
        .border_b_1()
        .border_color(t.separator)
        .child(tabs.id("doc-tabs").flex_1().min_w_0().overflow_x_scroll().track_scroll(&state::tab_scroll(current, docs.iter().position(|d| d == current).unwrap_or(0), cx)))
        .children(overflow)
        .child(div().flex_none().child(actions))
}

/// Read-only view of PDFs, images, text and other files inside the main window.
pub struct FileViewer {
    path: PathBuf,
    pdf: Option<Entity<PdfView>>,
    preview: Entity<FilePreview>,
    app: Option<String>,
}

impl FileViewer {
    pub fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let is_pdf = extension(&path.to_string_lossy()).eq_ignore_ascii_case("pdf");
        let pdf = is_pdf.then(|| cx.new(|cx| PdfView::new(&path, cx).take_focus_on_load()));
        let preview = cx.new(|cx| {
            let mut p = FilePreview::new(false);
            if !is_pdf {
                p.set_path(Some(path.clone()), cx);
            }
            p
        });
                let app = super::platform::default_app_name(&path);
        Self { path, pdf, preview, app }
    }
}

impl FileViewer {
    /// Gives the keyboard to the document, so its shortcuts work right after opening.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pdf) = self.pdf.clone() {
            pdf.update(cx, |v, cx| v.take_focus(window, cx));
        }
    }
}

impl Render for FileViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        if let Some(pdf) = self.pdf.clone() {
            if let Some((term, nth)) = take_find_request(&self.path, cx) {
                pdf.update(cx, |v, cx| v.search_for(&term, nth, window, cx));
            }
        }
        let me = cx.entity();
        let (p1, p2, p3, p4) = (self.path.clone(), self.path.clone(), self.path.clone(), self.path.clone());
        let in_vault = state::read(cx, |l| l.is_vault);
        let mut actions = div().flex().items_center().gap(px(space::S));
        actions = actions
            .child(Button::new("open-default").small().icon(menu_icon("square-arrow-out-up-right")).label(self.app.as_ref().map_or_else(|| tr("Mit Standard-App öffnen"), |a| trf("In %@ öffnen", &[a]))).tooltip(tr("Zum Bearbeiten in der Standard-App öffnen")).on_click(move |_, _, _| library::open_path(&p1)))
            .child(Button::new("viewer-more").ghost().small().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu(move |m, _, _| {
                let (a, b, c) = (p2.clone(), p3.clone(), p4.clone());
                m.item(PopupMenuItem::new(super::platform::reveal_label()).icon(menu_icon("folder")).on_click(move |_, _, _| library::reveal(&a)))
                    .item(PopupMenuItem::new(tr("Vorschau-Fenster (Leertaste)")).icon(menu_icon("eye")).on_click(move |_, _, cx| super::preview::show(&b, cx)))
                    .separator()
                    .item(PopupMenuItem::new(tr("Tab schließen")).icon(menu_icon("x")).on_click(move |_, _, cx| close_tab(&c, cx)))
            }));
        let body: AnyElement = if !self.path.exists() {
            unavailable("file-question", tr("Datei nicht gefunden"), self.path.file_name().unwrap_or_default().to_string_lossy().to_string(), &t).into_any_element()
        } else if let Some(pdf) = &self.pdf {
            pdf.clone().into_any_element()
        } else {
            self.preview.clone().into_any_element()
        };
        div().size_full().flex().flex_col().child(document_bar(&self.path, actions.into_any_element(), &t, cx)).child(div().flex_1().min_h_0().child(body))
    }
}

/// Saves or asks about every open note, then quits.
pub fn prepare_to_quit(cx: &mut App) {
    state::flush_pending(cx);
    let all = state::read(cx, |l| l.buffers.keys().cloned().collect());
    resolve_unsaved(all, cx, |cx| super::sync::quit_after(|cx| cx.quit(), cx));
}
