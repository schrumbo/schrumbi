use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Editor, EditorState, InputEvent, TextDecoration, TextDecorationCollection};
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
use super::bar_fit::{self, BarFit, Room};
use crate::core::vault::index;
use super::theme::{Theme, control, font, note_size, note_scale, radius, space, iconsize, tab};
use super::widgets::*;
use crate::core::live_text::{self, LiveStyle, Segment, SegmentKind};
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
        ListNewline, ListTab, ListBacktab, ExtractSelection, CardFromSelection, PlainNewline, Strike, Mark, InlineMath, Quote, BulletList, NumberedList, Heading1, Heading2, Heading3
    ]
);

const NOTE_PLACES: usize = 500;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Read,
    Live,
    Source,
    Split,
}

/// What the field leaves free on its left, taken back so text stays where the drawn block had it.
const LIVE_GUTTER: f32 = 12.;
/// What the field leaves free above and below its text.
const LIVE_INSET: f32 = 9.5;

/// What is being written in the live view.
#[derive(Clone, Copy)]
struct LiveBlock {
    /// Its place in the list of segments.
    index: usize,
    first_line: usize,
    line_count: usize,
    /// A line, not something that opens as a whole.
    single: bool,
    cell: Option<LiveCell>,
    /// Several lines opened together to hold a selection; a plain caret closes them again.
    selecting: bool,
}

/// A cell of a table being written.
#[derive(Clone, Copy)]
struct LiveCell {
    row: usize,
    column: usize,
    line: usize,
}

/// The room the handles beside a block take.
const GUTTER_WIDTH: f32 = 48.;
/// The room a fold chevron takes left of a heading or list, between its text and the handles.
const FOLD_ROOM: f32 = iconsize::L;
/// What the live view keeps free left of the text, so every handle stays inside the pane.
const LIVE_ROOM: f32 = GUTTER_WIDTH + iconsize::L + space::XXS;

/// Lines being dragged to another place in the note.
#[derive(Clone)]
struct DraggedLines {
    line: usize,
    count: usize,
    label: String,
}

impl Render for DraggedLines {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        div().max_w(px(360.)).px(px(space::M)).py(px(space::XS)).rounded(px(radius::INNER)).bg(t.material).border_1().border_color(t.separator).shadow_md().text_size(px(font::CALLOUT)).text_color(t.primary).truncate().child(self.label.clone())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum LineAction {
    Up,
    Down,
    Copy,
    Delete,
}

/// A block's menu: what it can become and where it can go.
fn block_menu(menu: gpui_kit::component::menu::PopupMenu, me: Entity<NoteView>, line: usize, count: usize, is_table: bool) -> gpui_kit::component::menu::PopupMenu {
    use live_text::LineKind as K;
    let mut menu = menu;
    if !is_table {
        for (label, name, kind) in [(tr("Text"), "text", K::Text), (tr("Überschrift 1"), "heading-1", K::Heading(1)), (tr("Überschrift 2"), "heading-2", K::Heading(2)), (tr("Überschrift 3"), "heading-3", K::Heading(3)), (tr("Liste"), "list", K::Bullet), (tr("Nummerierte Liste"), "list-ordered", K::Numbered), (tr("Aufgabe"), "square-check", K::Task), (tr("Zitat"), "quote", K::Quote), (tr("Callout"), "message-square-warning", K::Callout), (tr("Code"), "code", K::Code), (tr("Formel"), "sigma", K::Formula)] {
            let e = me.clone();
            menu = menu.item(PopupMenuItem::new(label).icon(menu_icon(name)).on_click(move |_, window, cx| e.update(cx, |n, cx| n.turn_lines(line, count, kind, window, cx))));
        }
        menu = menu.separator();
    }
    let (copy, up, down, delete) = (me.clone(), me.clone(), me.clone(), me);
    menu.item(PopupMenuItem::new(tr("Duplizieren")).icon(menu_icon("copy")).on_click(move |_, window, cx| copy.update(cx, |n, cx| {
        let old = n.lines_at(line, count, cx);
        let mut new = old.clone();
        new.extend(old);
        n.edit_lines(line, count, new, None, window, cx);
    })))
    .item(PopupMenuItem::new(tr("Nach oben")).icon(menu_icon("arrow-up")).on_click(move |_, window, cx| up.update(cx, |n, cx| n.move_lines_to(line, count, line.saturating_sub(1), window, cx))))
    .item(PopupMenuItem::new(tr("Nach unten")).icon(menu_icon("arrow-down")).on_click(move |_, window, cx| down.update(cx, |n, cx| n.move_lines_to(line, count, line + count + 1, window, cx))))
    .separator()
    .item(PopupMenuItem::new(tr("Löschen")).icon(menu_icon("trash-2")).on_click(move |_, window, cx| delete.update(cx, |n, cx| n.edit_lines(line, count, Vec::new(), None, window, cx))))
}

/// Buttons that show on a table to make it bigger.
fn table_tools(me: &Entity<NoteView>, i: usize, line: usize, count: usize, group: SharedString, t: &Theme) -> AnyElement {
    let (row_me, column_me) = (me.clone(), me.clone());
    let tool = |id: (&'static str, usize), label: String| div().id(id).h(px(control::S)).px(px(space::S)).flex().items_center().rounded(px(radius::SMALL)).bg(t.secondary_bg).border_1().border_color(t.separator).text_size(px(font::CAPTION)).text_color(t.secondary).cursor_pointer().soft(t.ink.opacity(0.06), radius::SMALL).child(label);
    div()
        .flex()
        .gap(px(space::XS))
        .mt(px(space::XS))
        .invisible()
        .group_hover(group, |s| s.visible())
        .capture_any_mouse_down(|_, _, _| markdown_view::press_control())
        .child(tool(("table-row", i), tr("+ Zeile")).on_click(move |_, window, cx| row_me.update(cx, |n, cx| {
            let old = n.lines_at(line, count, cx);
            let refs: Vec<&str> = old.iter().map(String::as_str).collect();
            n.edit_lines(line, count, live_text::table_with_row(&refs), None, window, cx);
        })))
        .child(tool(("table-column", i), tr("+ Spalte")).on_click(move |_, window, cx| column_me.update(cx, |n, cx| {
            let old = n.lines_at(line, count, cx);
            let refs: Vec<&str> = old.iter().map(String::as_str).collect();
            n.edit_lines(line, count, live_text::table_with_column(&refs), None, window, cx);
        })))
        .into_any_element()
}

/// How far a nested list line moves in.
const LIVE_INDENT: f32 = 24.;

/// A Markdown note: read, edit or both side by side. Saving follows Settings → Notizen.
pub struct NoteView {
    /// A nearly empty note opens ready to continue writing below what is there.
    caret_to_end: bool,
    reader_list: ListState,
    /// The text scale the list measured its items with.
    measured_scale: f32,
    measured_full: bool,
    /// The formula or diagram being written, drawn while it is written.
    live_preview: Option<(String, Rc<ParsedNote>)>,
    /// A block the reader should bring into view once it knows the block.
    reveal_line: Option<usize>,
    item_hashes: Vec<u64>,
    split_sync: (f32, f32),
    reparse: Option<Task<()>>,
    /// What the live view draws, one entry per list item between start and end.
    segments: Rc<Vec<Segment>>,
    source_lines: Rc<Vec<String>>,
    /// Reference and footnote definitions, which a line drawn on its own still needs.
    definitions: Rc<String>,
    /// Lines already parsed for drawing.
    fragments: Rc<std::cell::RefCell<std::collections::HashMap<String, Rc<ParsedNote>>>>,
    parsed_text: Option<String>,
    marks: Option<TextDecorationCollection>,
    /// A source line to bring back to the top after switching views.
    keep_line: Option<usize>,
    /// The source view is only open for a selection over several lines.
    source_detour: bool,
    /// Which editor the settings asked for when this view last looked.
    editor_kind: prefs::NoteEditor,
    reader_width: Pixels,
    spell_task: Option<Task<()>>,
    /// Where writing last happened in this note.
    last_line: Option<usize>,
    /// Whether this view has looked for changes it inherited, as after an outside move.
    took_over: bool,
    /// What is being written in the live view, in a field of its own.
    block: Entity<EditorState>,
    live: Option<LiveBlock>,
    /// Whole-note states to go back to across blocks; the field only knows its own block.
    history: Vec<(String, usize)>,
    future: Vec<(String, usize)>,
    last_snapshot: Option<std::time::Instant>,
    found: Option<markdown_view::Found>,
    /// Editor and reader offsets at the last sync of the split view.
    link_focus: markdown_view::LinkFocus,
    place_restored: bool,
    fit: BarFit,
    toc: Rc<Vec<(String, usize)>>,
    toc_of: Option<(Rc<ParsedNote>, Mode)>,
    path: PathBuf,
    mode: Mode,
    editor: Entity<EditorState>,
    parsed: Option<(u64, Rc<ParsedNote>)>,
    revision: u64,
    syncing: bool,
    autosave: Option<Task<()>>,
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
        let block = cx.new(|cx| EditorState::new(window, cx).soft_wrap(true).line_number(false).folding(false).smart_indent(false).scroll_beyond_last_line(Some(0)).placeholder(tr("Schreibe oder tippe / für Befehle")));
        block.update(cx, |s, _| s.lsp_mut().completion_provider = Some(super::completion::provider(path.clone())));
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
                            if l.preview_document.as_ref() == Some(&path) {
                                l.preview_document = None;
                            }
                        }
                        cx.notify();
                    });
                    this.schedule_autosave(cx);
                    this.schedule_spelling(cx);
                    this.schedule_reparse(cx);
                }
                InputEvent::Blur => this.focus_lost(cx),
                _ => {}
            }),
            cx.subscribe_in(&block, window, |this, e, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change if !this.syncing => {
                    let (value, caret) = (e.read(cx).value().to_string(), e.read(cx).cursor());
                    this.block_changed(value, caret, window, cx);
                }
                InputEvent::Blur if !this.syncing => {
                    let field = e.clone();
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(Duration::from_millis(60)).await;
                        let _ = this.update_in(cx, |this, window, cx| {
                            if this.live.is_some() && !field.read(cx).focus_handle(cx).contains_focused(window, cx) {
                                this.finish_block(cx);
                                this.focus_lost(cx);
                            }
                        });
                    })
                    .detach();
                }
                _ => {}
            }),
            cx.observe_in(&lib, window, move |this, lib, window, cx| {
                if lib.read(cx).active_document.as_ref() == Some(&p) && lib.update(cx, |l, _| std::mem::take(&mut l.toggle_mode)) {
                    let writing = if this.editor_kind == prefs::NoteEditor::Live { Mode::Live } else { Mode::Source };
                    let next = if this.mode == Mode::Read { writing } else { Mode::Read };
                    this.set_mode(next, window, cx);
                }
                if lib.read(cx).active_document.as_ref() != Some(&p) && this.place_restored {
                    this.remember_place();
                }
                let Some((rev, text)) = lib.read(cx).buffers.get(&p).map(|b| (b.revision, b.text.clone())) else { return };
                if rev != this.revision {
                    this.revision = rev;
                    let reopen = this.live.take().map(|l| (l.first_line, this.block.read(cx).cursor()));
                    if let Some((line, caret)) = reopen {
                        this.parsed = None;
                        window.on_next_frame({
                            let me = cx.entity();
                            move |window, cx| me.update(cx, |n, cx| n.open_line(line, Some(caret), window, cx))
                        });
                    }
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
        let writing = match prefs::get().note_editor {
            prefs::NoteEditor::Live => Mode::Live,
            prefs::NoteEditor::Code => Mode::Source,
        };
        let just_made = state::lib(cx).update(cx, |l, _| l.take_fresh_note(&path));
        let mode = if just_made {
            prefs::update(|p| p.note_mode = prefs::NoteMode::Write);
            writing
        } else {
            match prefs::get().note_mode {
                prefs::NoteMode::Read => Mode::Read,
                prefs::NoteMode::Split if prefs::get().note_editor == prefs::NoteEditor::Code => Mode::Split,
                _ => writing,
            }
        };
        let revision = state::lib(cx).update(cx, |l, _| l.buffer(&path).revision);
        let reader_list = ListState::new(0, ListAlignment::Top, px(1000.));
        let list_for_focus = reader_list.clone();
        let saved_folds = prefs::get().note_folds.get(&path.to_string_lossy().to_string()).cloned().unwrap_or_default().into_iter().collect();
        let own_path = dunce::canonicalize(&path).unwrap_or_else(|_| path.clone());
        Self { path, caret_to_end: mode == Mode::Source && text.trim().chars().count() < 40, mode, block: block.clone(), live: None, history: Vec::new(), future: Vec::new(), last_snapshot: None, editor, reader_list, measured_scale: note_scale(), measured_full: prefs::get().reader_full_width, live_preview: None, reveal_line: None, item_hashes: Vec::new(), split_sync: (0., 0.), reparse: None, segments: Rc::default(), source_lines: Rc::default(), definitions: Rc::default(), fragments: Rc::default(), parsed_text: None, marks: None, keep_line: None, source_detour: false, editor_kind: prefs::get().note_editor, reader_width: px(720.), spell_task: None, last_line: None, took_over: false, found: None, link_focus: markdown_view::LinkFocus { index: None, note: Some(own_path), list: Some(list_for_focus), text_click: None, edit_item: None, folds: Rc::new(std::cell::RefCell::new(saved_folds)) }, place_restored: false, parsed: None, revision, syncing: false, autosave: None, embedded, focus: cx.focus_handle(), needs_focus: !embedded, window_title: String::new(), fit: BarFit::default(), toc: Rc::default(), toc_of: None, _subs: subs }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Selects a search match in the editor and scrolls the reader to its block.
    fn reveal(&mut self, term: &str, nth: usize, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = search_index::occurrences(text, term).into_iter().nth(nth) else { return };
        self.editor.update(cx, |s, cx| s.set_selected_range(range.clone(), cx));
        let token = self.found.as_ref().map_or(0, |f| f.token + 1);
        let _ = (&window, &cx);
        let block = markdown_view::block_at(text, range.start);
        let line = text.get(..range.start).unwrap_or_default().matches('\n').count();
        self.reveal_line = Some(line);
        self.found = Some(markdown_view::Found { block, line, token });
    }

    /// Focuses the editor when editing, otherwise the note itself so its shortcuts work.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = match prefs::get().note_mode {
            prefs::NoteMode::Read => Mode::Read,
            prefs::NoteMode::Split if self.editor_kind == prefs::NoteEditor::Code => Mode::Split,
            _ if self.editor_kind == prefs::NoteEditor::Live => Mode::Live,
            _ => Mode::Source,
        };
        if self.mode != wanted && !self.source_detour {
            self.set_mode(wanted, window, cx);
        }
        if self.mode == Mode::Live && state::read(cx, |l| l.buffers.get(&self.path).is_some_and(|b| b.text.trim().is_empty())) {
            self.open_line(0, None, window, cx);
        } else if !matches!(self.mode, Mode::Source | Mode::Split) {
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
        let remembered = match mode {
            Mode::Read => prefs::NoteMode::Read,
            Mode::Split => prefs::NoteMode::Split,
            Mode::Live | Mode::Source => prefs::NoteMode::Write,
        };
        if prefs::get().note_mode != remembered {
            prefs::update(|p| p.note_mode = remembered);
        }
        if mode == Mode::Read {
            self.finish_block(cx);
            let path = self.path.clone();
            act(cx, |l| l.follow_heading(&path));
            if !self.path.exists() {
                return;
            }
        }
        let keep_line = self.top_line();
        self.finish_block(cx);
        if matches!(mode, Mode::Source | Mode::Split) {
            let text = state::read(cx, |l| l.buffers.get(&self.path).map(|b| b.text.clone()).unwrap_or_default());
            self.syncing = true;
            self.editor.update(cx, |s, cx| s.set_value(text, window, cx));
            self.syncing = false;
        }
        self.parsed = None;
        self.item_hashes.clear();
        self.reader_list.reset(0);
        self.keep_line = Some(keep_line);
        self.mode = mode;
        if matches!(mode, Mode::Source | Mode::Split) {
            self.editor.update(cx, |s, cx| s.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
            self.focus_lost(cx);
        }
        cx.notify();
    }

    /// The source line at the top of the reader.
    fn top_line(&self) -> usize {
        let item = self.reader_list.logical_scroll_top().item_ix.saturating_sub(1);
        if self.mode == Mode::Live {
            return self.segments.get(item).map_or(0, |s| s.line);
        }
        self.parsed.as_ref().and_then(|(_, p)| p.rows().get(item).map(|r| r.line)).unwrap_or(0)
    }

    /// The list item a source line is drawn in.
    fn item_of_line(&self, line: usize) -> usize {
        if self.mode == Mode::Live {
            return self.segments.iter().position(|s| line < s.line + s.count).map_or(self.segments.len(), |i| i + 1);
        }
        self.parsed.as_ref().map_or(1, |(_, p)| p.row_of_line(line) + 1)
    }

    /// The text of the note right now.
    fn current_text(&self, cx: &App) -> String {
        state::read(cx, |l| l.buffers.get(&self.path).map(|b| b.text.clone()).unwrap_or_default())
    }

    fn put_text(&self, text: String, cx: &mut Context<Self>) {
        let path = self.path.clone();
        state::lib(cx).update(cx, |l, cx| {
            if let Some(b) = l.buffers.get_mut(&path) {
                b.edited(text);
            }
            if l.preview_document.as_ref() == Some(&path) {
                l.preview_document = None;
            }
            cx.notify();
        });
    }

    /// Opens what stands at a source line for writing, with the caret at a byte of it or at its end.
    fn open_line(&mut self, line: usize, caret: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_block(cx);
        let text = self.current_text(cx);
        self.parse_now(&text);
        let lines: Vec<&str> = text.split('\n').collect();
        let line = line.min(lines.len().saturating_sub(1));
        let found = self.segments.iter().position(|s| line < s.line + s.count);
        let (index, first_line, line_count, kind) = match found {
            Some(i) => {
                let s = self.segments[i];
                (i, s.line, s.count, s.kind)
            }
            None => (0, 0, lines.len(), SegmentKind::Blank),
        };
        if let SegmentKind::Table { .. } = kind {
            let row = line - first_line;
            self.open_cell(index, if row >= 2 { row - 1 } else { 0 }, 0, caret, window, cx);
            return;
        }
        let source = lines.get(first_line..(first_line + line_count).min(lines.len())).unwrap_or_default().join("\n");
        let before: usize = lines.get(first_line..line).unwrap_or_default().iter().map(|l| l.len() + 1).sum();
        let at = caret.map_or(source.len(), |c| before + c).min(source.len());
        let at = (0..=at).rev().find(|i| source.is_char_boundary(*i)).unwrap_or(0);
        let single = !matches!(kind, SegmentKind::Whole { .. });
        self.fill_field(source, at..at, window, cx);
        self.live = Some(LiveBlock { index, first_line, line_count, single, cell: None, selecting: false });
        self.after_open(index, cx);
    }

    /// Opens one cell of a table. Row 0 is the head.
    fn open_cell(&mut self, index: usize, row: usize, column: usize, caret: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(segment) = self.segments.get(index).copied() else { return };
        let text = self.current_text(cx);
        let lines: Vec<&str> = text.split('\n').collect();
        let line_no = segment.line + if row == 0 { 0 } else { row + 1 };
        let Some(line) = lines.get(line_no) else { return };
        let cells = live_text::table_cells(line);
        let Some(cell) = cells.get(column).cloned() else { return };
        let value = line[cell].to_string();
        let at = caret.unwrap_or(value.len()).min(value.len());
        self.fill_field(value, at..at, window, cx);
        self.live = Some(LiveBlock { index, first_line: segment.line, line_count: segment.count, single: true, cell: Some(LiveCell { row, column, line: line_no }), selecting: false });
        self.after_open(index, cx);
    }

    fn fill_field(&mut self, value: String, selection: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.syncing = true;
        self.block.update(cx, |b, cx| {
            b.set_value(value, window, cx);
            b.set_selected_range(selection, cx);
            b.focus(window, cx);
        });
        self.syncing = false;
    }

    fn after_open(&mut self, index: usize, cx: &mut Context<Self>) {
        self.paint_marks(cx);
        self.schedule_spelling(cx);
        self.reader_list.remeasure();
        if self.segments.is_empty() {
            self.reader_list.reset(3);
        } else {
            self.reader_list.scroll_to_reveal_item(index + 1);
        }
        cx.notify();
    }

    /// Shows what the written marks mean while the line is open, and keeps the marks themselves quiet.
    fn paint_marks(&mut self, cx: &mut Context<Self>) {
        let t = Theme::get(cx);
        let value = self.block.read(cx).value().to_string();
        let decorations: Vec<TextDecoration> = live_text::styles(&value)
            .into_iter()
            .map(|(range, style)| {
                let look = match style {
                    LiveStyle::Marker => HighlightStyle { color: Some(t.tertiary), ..Default::default() },
                    LiveStyle::Bold => HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() },
                    LiveStyle::Italic => HighlightStyle { font_style: Some(FontStyle::Italic), ..Default::default() },
                    LiveStyle::Strike => HighlightStyle { strikethrough: Some(StrikethroughStyle { thickness: px(1.), color: Some(t.secondary) }), ..Default::default() },
                    LiveStyle::Code => HighlightStyle { background_color: Some(t.ink.opacity(0.08)), ..Default::default() },
                    LiveStyle::Mark => HighlightStyle { background_color: Some(t.orange.opacity(0.3)), ..Default::default() },
                    LiveStyle::Link => HighlightStyle { color: Some(t.accent_text), ..Default::default() },
                };
                TextDecoration::new(range, look)
            })
            .collect();
        match &self.marks {
            Some(known) => known.set(decorations, cx),
            None => {
                let made = self.block.update(cx, |b, cx| b.create_decorations_collection(decorations, cx));
                self.marks = Some(made);
            }
        }
    }

    /// The note parsed as it is right now, with the list told what changed.
    fn parse_now(&mut self, text: &str) -> Rc<ParsedNote> {
        if let Some((rev, p)) = &self.parsed
            && *rev == self.revision
            && self.parsed_text.as_deref() == Some(text)
        {
            return p.clone();
        }
        let p = Rc::new(markdown_view::parse(text));
        let hashes: Vec<u64> = if self.mode == Mode::Live {
            self.segments = Rc::new(live_text::segments(text, &p.shapes()));
            self.definitions = Rc::new(text.split('\n').filter(|l| l.starts_with('[') && l.contains("]:")).collect::<Vec<_>>().join("\n"));
            self.segments.iter().map(|s| s.hash).collect()
        } else {
            p.rows().iter().map(|r| r.hash).collect()
        };
        self.source_lines = Rc::new(text.split('\n').map(str::to_string).collect());
        self.adopt_items(hashes);
        self.parsed = Some((self.revision, p.clone()));
        self.parsed_text = Some(text.to_string());
        p
    }

    /// Closes what is being written; the note is drawn fresh from what is in it now.
    fn finish_block(&mut self, cx: &mut Context<Self>) {
        if let Some(live) = self.live.take() {
            self.last_line = Some(live.first_line);
            self.reader_list.remeasure();
            cx.notify();
        }
    }

    /// Puts what was typed into the note. A line that became two opens at the part with the caret.
    fn block_changed(&mut self, value: String, caret: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(live) = &self.live else { return };
        let (first_line, line_count, single, cell) = (live.first_line, live.line_count, live.single, live.cell);
        let before = self.current_text(cx);
        let mut lines: Vec<String> = before.split('\n').map(str::to_string).collect();
        let caret_line = value.get(..caret).unwrap_or_default().matches('\n').count();
        if let Some(cell) = cell {
            let Some((changed, _)) = lines.get(cell.line).and_then(|l| live_text::set_table_cell(l, cell.column, &value)) else { return };
            lines[cell.line] = changed;
        } else {
            let end = (first_line + line_count).min(lines.len());
            lines.splice(first_line.min(end)..end, value.split('\n').map(str::to_string));
        }
        let full = lines.join("\n");
        if self.last_snapshot.is_none_or(|t| t.elapsed() > Duration::from_millis(800)) {
            self.history.push((before, first_line + caret_line));
            if self.history.len() > 500 {
                self.history.remove(0);
            }
            self.future.clear();
        }
        self.last_snapshot = Some(std::time::Instant::now());
        self.put_text(full, cx);
        if cell.is_none()
            && let Some(live) = &mut self.live
        {
            live.line_count = value.split('\n').count();
        }
        self.paint_marks(cx);
        self.schedule_autosave(cx);
        self.schedule_spelling(cx);
        if single && cell.is_none() && value.contains('\n') {
            let line_start = value[..caret.min(value.len())].rfind('\n').map_or(0, |i| i + 1);
            self.last_snapshot = None;
            self.open_line(first_line + caret_line, Some(caret - line_start), window, cx);
        }
        cx.notify();
    }

    /// Takes the note back to what it was before the last burst of typing, wherever that was.
    fn step_history(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((text, line)) = (if back { self.history.pop() } else { self.future.pop() }) else { return };
        let here = self.live.as_ref().map_or(line, |l| l.first_line);
        let current = self.current_text(cx);
        if back {
            self.future.push((current, here));
        } else {
            self.history.push((current, here));
        }
        self.live = None;
        self.put_text(text, cx);
        self.last_snapshot = None;
        self.open_line(line, None, window, cx);
        self.schedule_autosave(cx);
    }

    /// Esc closes what is open and leaves the note ready for the keyboard; a suggestion list closes first.
    fn leave_block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.live.is_none() || self.block.read(cx).completion_menu_state().open {
            return;
        }
        cx.stop_propagation();
        self.finish_block(cx);
        window.focus(&self.focus, cx);
    }

    /// The text height of a line of the given kind.
    fn field_font_size(&self, value: &str) -> f32 {
        let hashes = value.chars().take_while(|c| *c == '#').count();
        if !value.contains('\n') && hashes > 0 && value[hashes..].starts_with(' ') {
            return note_size(match hashes {
                1 => font::TITLE,
                2 => font::TITLE2,
                3 => font::TITLE3,
                4 => font::BODY,
                _ => font::CALLOUT,
            });
        }
        note_size(font::BODY)
    }

    /// Whether the caret stands on the first or last drawn line of what is open.
    fn at_block_edge(&self, down: bool, window: &mut Window, cx: &App) -> bool {
        if self.live.is_none() {
            return false;
        }
        let b = self.block.read(cx);
        let (value, range) = (b.value().to_string(), b.selected_range());
        if !range.is_empty() || b.completion_menu_state().open {
            return false;
        }
        let line_start = value[..range.start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = value[range.start..].find('\n').map_or(value.len(), |i| range.start + i);
        if (down && line_end != value.len()) || (!down && line_start != 0) {
            return false;
        }
        let line = &value[line_start..line_end];
        if line.is_empty() {
            return true;
        }
        let size = self.field_font_size(&value);
        let style = window.text_style();
        let run = TextRun { len: line.len(), font: style.font(), color: style.color, background_color: None, underline: None, strikethrough: None };
        let width = self.field_width(&value, window);
        let Ok(shaped) = window.text_system().shape_text(line.to_string().into(), px(size), &[run], Some(width), None) else { return true };
        let Some(wrapped) = shaped.first() else { return true };
        let height = px(size * 1.5);
        let row = |at: usize| wrapped.position_for_index(at, height).map_or(0., |p| f32::from(p.y));
        if down { row(range.start - line_start) >= row(line.len()) } else { row(range.start - line_start) < 1. }
    }

    /// The left edge of a line's text once it is drawn, measured from the column.
    fn drawn_text_left(&self, value: &str, depth: usize) -> f32 {
        let bare = value.trim_start();
        let is_task = bare.len() > 2 && live_text::prefix_len(bare) >= 6 && bare[2..].starts_with('[');
        let is_list = bare.starts_with("- ") || bare.starts_with("* ") || bare.starts_with("+ ") || bare.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        let indent = depth as f32 * LIVE_INDENT;
        if is_task {
            indent + 14. + space::XXS + space::S
        } else if is_list {
            indent + 16. + space::S
        } else {
            indent
        }
    }

    /// How wide the marks in front of the text are; they hang to its left, so the text stays where it was drawn.
    fn hang_width(&self, value: &str, size: f32, window: &mut Window) -> f32 {
        let prefix = &value[..live_text::prefix_len(value)];
        if prefix.is_empty() {
            return 0.;
        }
        let style = window.text_style();
        let run = TextRun { len: prefix.len(), font: style.font(), color: style.color, background_color: None, underline: None, strikethrough: None };
        f32::from(window.text_system().shape_line(prefix.to_string().into(), px(size), &[run], None).width)
    }

    /// The column's room left of the text: the live view takes what its handles need beyond the space beside the column.
    fn column_left(&self) -> f32 {
        if self.mode != Mode::Live {
            return space::XXL;
        }
        let beside = if self.measured_full { 0. } else { ((f32::from(self.reader_width) - READER_COLUMN) / 2.).max(0.) };
        (LIVE_ROOM - beside).max(space::XXL)
    }

    fn field_width(&self, value: &str, window: &mut Window) -> Pixels {
        let wide = if self.measured_full { self.reader_width } else { self.reader_width.min(px(READER_COLUMN)) };
        let column = (wide - px(self.column_left() + space::XXL)).max(px(120.));
        let single = self.live.as_ref().is_some_and(|l| l.single && l.cell.is_none());
        if !single {
            return column;
        }
        let depth = live_text::depth_while_writing(value, self.live.as_ref().and_then(|l| self.segments.get(l.index)).map_or(0, |s| if let SegmentKind::Line { depth } = s.kind { depth } else { 0 }));
        let size = self.field_font_size(value);
        column - px(self.drawn_text_left(value, depth)) + px(self.hang_width(value, size, window))
    }

    /// Typing with nothing open starts where writing last happened, so no keystroke is lost.
    fn open_for_typing(&mut self, inserted: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.last_line.unwrap_or_else(|| self.top_line());
        let line = self.segments.iter().find(|seg| seg.line >= wanted && matches!(seg.kind, SegmentKind::Line { .. } | SegmentKind::Blank)).map_or(wanted, |seg| seg.line);
        self.open_line(line, None, window, cx);
        if let Some(text) = inserted {
            self.block.update(cx, |b, cx| b.replace(text, window, cx));
        }
    }

    fn at_block_edge_selecting(&self, down: bool, cx: &App) -> bool {
        let Some(live) = self.live.filter(|l| (l.single || l.selecting) && l.cell.is_none()) else { return false };
        if live.single {
            return true;
        }
        let b = self.block.read(cx);
        let range = b.selected_range();
        if down { range.end == b.value().len() } else { range.start == 0 }
    }

    /// The byte where a source line starts in the whole note.
    fn line_offset(text: &str, line: usize) -> usize {
        text.split_inclusive('\n').take(line).map(str::len).sum()
    }

    /// Selecting past the open lines opens the next one too: the text stays in its place, only these lines show as written.
    fn select_beyond_line(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(live) = self.live else { return };
        let range = self.block.read(cx).selected_range();
        let text = self.current_text(cx);
        let total = text.split('\n').count();
        let lines: Vec<&str> = text.split('\n').collect();
        let (first, count) = if down {
            (live.first_line, (live.line_count + 1).min(total - live.first_line))
        } else {
            (live.first_line.saturating_sub(1), live.line_count + usize::from(live.first_line > 0))
        };
        if first == live.first_line && count == live.line_count {
            return;
        }
        let value = lines[first..first + count].join("\n");
        let added_before = if down { 0 } else { lines[first].len() + 1 };
        let selection = if down { range.start..value.len() } else { 0..range.end + added_before };
        self.open_selecting(first, count, value, selection, live.index, window, cx);
    }

    fn select_whole_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.current_text(cx);
        let count = text.split('\n').count();
        let end = text.len();
        self.open_selecting(0, count, text, 0..end, 0, window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn open_selecting(&mut self, first: usize, count: usize, value: String, selection: std::ops::Range<usize>, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.segments.iter().position(|s| first < s.line + s.count).unwrap_or(index);
        self.fill_field(value, selection, window, cx);
        self.live = Some(LiveBlock { index: at, first_line: first, line_count: count, single: false, cell: None, selecting: true });
        self.after_open(at, cx);
    }

    /// When a many-line selection became a plain caret, only the line with the caret stays open.
    fn settle_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(live) = self.live.filter(|l| l.selecting) else { return };
        let b = self.block.read(cx);
        if !b.selected_range().is_empty() {
            return;
        }
        let at = b.cursor();
        let value = b.value().to_string();
        let row = value[..at.min(value.len())].matches('\n').count();
        let start = value[..at.min(value.len())].rfind('\n').map_or(0, |i| i + 1);
        self.open_line(live.first_line + row, Some(at - start), window, cx);
    }

    fn detour_to_source(&mut self, selection: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(Mode::Source, window, cx);
        self.source_detour = true;
        self.editor.update(cx, |s, cx| s.set_selected_range(selection, cx));
    }

    /// Arrow left at the start or right at the end moves on to the next line.
    fn cross_to_edge(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(live) = self.live else { return };
        let target = if forward { live.index + 1 } else { live.index.wrapping_sub(1) };
        let Some(segment) = self.segments.get(target).copied() else { return };
        let line = if forward { segment.line } else { segment.line + segment.count - 1 };
        self.open_line(line, if forward { Some(0) } else { None }, window, cx);
    }

    /// Goes to the line above or below, as close to the same place in the line as it can.
    fn cross_block(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(live) = self.live else { return };
        let index = live.index;
        if let Some(cell) = live.cell {
            let rows = self.segments.get(index).map_or(1, |s| s.count.saturating_sub(1));
            let row = if down { cell.row + 1 } else { cell.row.wrapping_sub(1) };
            if row < rows {
                self.finish_block(cx);
                self.open_cell(index, row, cell.column, None, window, cx);
                return;
            }
        }
        let column = {
            let b = self.block.read(cx);
            let value = b.value();
            let at = b.selected_range().start.min(value.len());
            let start = value[..at].rfind('\n').map_or(0, |i| i + 1);
            let line = &value[start..];
            let line = &line[..line.find('\n').unwrap_or(line.len())];
            let marks = live_text::prefix_len(line).min(at - start);
            value[start + marks..at].chars().count()
        };
        let target = if down { index + 1 } else { index.wrapping_sub(1) };
        let Some(segment) = self.segments.get(target).copied() else {
            let end = if down { self.block.read(cx).value().len() } else { 0 };
            self.block.update(cx, |b, cx| b.set_selected_range(end..end, cx));
            return;
        };
        let line = if down { segment.line } else { segment.line + segment.count - 1 };
        let row = self.source_lines.get(line).cloned().unwrap_or_default();
        let marks = live_text::prefix_len(&row);
        let caret = row[marks..].char_indices().nth(column).map_or(row.len(), |(i, _)| marks + i);
        match segment.kind {
            SegmentKind::Whole { .. } => self.open_line(line, if down { Some(caret) } else { None }, window, cx),
            SegmentKind::Table { .. } => {
                let row = if down { 0 } else { segment.count.saturating_sub(2) };
                self.open_cell(target, row, 0, None, window, cx);
            }
            _ => self.open_line(line, Some(caret), window, cx),
        }
    }

    /// Tab and Enter in a table walk its cells; behind the last one a new row starts.
    fn step_cell(&mut self, by_row: bool, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(LiveBlock { index, first_line, cell: Some(cell), .. }) = self.live.as_ref().map(|l| LiveBlock { selecting: false, index: l.index, first_line: l.first_line, line_count: l.line_count, single: l.single, cell: l.cell }) else { return };
        let text = self.current_text(cx);
        let lines: Vec<&str> = text.split('\n').collect();
        let columns = lines.get(cell.line).map_or(1, |l| live_text::table_cells(l).len());
        let rows = self.segments.get(index).map_or(1, |s| s.count.saturating_sub(1));
        let (mut row, mut column) = (cell.row, cell.column);
        if by_row {
            row += 1;
        } else if back {
            if column > 0 {
                column -= 1;
            } else if row > 0 {
                row -= 1;
                column = columns - 1;
            }
        } else if column + 1 < columns {
            column += 1;
        } else {
            row += 1;
            column = 0;
        }
        if row >= rows {
            let new_line = first_line + row + 1;
            let added = format!("|{}", "  |".repeat(columns));
            let mut all: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
            all.insert(new_line.min(all.len()), added);
            self.history.push((text.clone(), cell.line));
            self.future.clear();
            self.live = None;
            self.put_text(all.join("\n"), cx);
            self.schedule_autosave(cx);
            self.parse_now(&self.current_text(cx));
        } else {
            self.finish_block(cx);
            self.parse_now(&text);
        }
        self.open_cell(index, row, column, None, window, cx);
    }

    /// Backspace at the start of a line joins it to the line above; forward delete at its end joins the next.
    fn join_lines(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(live) = &self.live else { return };
        if !live.single || live.cell.is_some() {
            return;
        }
        let first_line = live.first_line;
        let text = self.current_text(cx);
        let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        let (upper, lower) = if down { (first_line, first_line + 1) } else { (first_line.wrapping_sub(1), first_line) };
        if upper >= lines.len() || lower >= lines.len() {
            return;
        }
        let segment_above = self.segments.iter().find(|s| upper >= s.line && upper < s.line + s.count).copied();
        if matches!(segment_above.map(|s| s.kind), Some(SegmentKind::Whole { .. } | SegmentKind::Table { .. })) {
            self.open_line(upper, None, window, cx);
            return;
        }
        let at = lines[upper].len();
        let joined = format!("{}{}", lines[upper], lines[lower]);
        lines[upper] = joined;
        lines.remove(lower);
        self.history.push((text, first_line));
        self.future.clear();
        self.live = None;
        self.put_text(lines.join("\n"), cx);
        self.open_line(upper, Some(at), window, cx);
        self.schedule_autosave(cx);
    }

    /// The field of the line being written, as tall as its text, with the text exactly where it was drawn.
    fn live_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let value = self.block.read(cx).value().to_string();
        let cell = self.live.as_ref().is_some_and(|l| l.cell.is_some());
        let single = self.live.as_ref().is_some_and(|l| l.single) && !cell;
        let depth = live_text::depth_while_writing(&value, self.live.as_ref().and_then(|l| self.segments.get(l.index)).map_or(0, |s| if let SegmentKind::Line { depth } = s.kind { depth } else { 0 }));
        let size = if cell { note_size(font::BODY) } else { self.field_font_size(&value) };
        let width = if cell { px(160.) } else { self.field_width(&value, window) };
        let style = window.text_style();
        let run = TextRun { len: value.len(), font: style.font(), color: style.color, background_color: None, underline: None, strikethrough: None };
        let rows: usize = window.text_system().shape_text(value.clone().into(), px(size), &[run], Some(width), None).map_or(1, |lines| lines.iter().map(|l| l.wrap_boundaries().len() + 1).sum());
        let line = px(size * 1.5);
        let height = line * (rows.max(1) as f32);
        let left = if single { self.drawn_text_left(&value, depth) - self.hang_width(&value, size, window) } else { 0. };
        let heading = single && value.starts_with('#') && size != note_size(font::BODY);
        let field = writing_menu(
            Editor::new(&self.block)
                .appearance(false)
                .font_family(Theme::font_family())
                .text_size(px(size))
                .line_height(line)
                .when(heading, |e| e.font_weight(FontWeight::BOLD))
                .when(!cell, |e| e.w(width + px(LIVE_GUTTER)))
                .when(cell, |e| e.w_full())
                .h(height + px(LIVE_INSET * 2. + 4.))
                .mt(px(-LIVE_INSET))
                .ml(px(left - LIVE_GUTTER)),
        );
        if self.live.is_some_and(|l| l.selecting) && self.block.read(cx).selected_range().is_empty() {
            let me = cx.entity();
            window.on_next_frame(move |window, cx| me.update(cx, |n, cx| n.settle_selection(window, cx)));
        }
        let t = Theme::get(cx);
        let selection = self.block.read(cx).selected_range();
        let caret = self.block.read(cx).cursor();
        let toolbar = (!selection.is_empty() && !self.block.read(cx).completion_menu_state().open).then(|| self.format_bar(&t, cx));
        let preview = self.live_preview_of(&value, caret, single, &t);
        div()
            .key_context("LiveBlock")
            .relative()
            .child(div().relative().h(height).child(self.note_editor_wrapper(field.into_any_element(), cx)).children(toolbar))
            .children(preview)
            .into_any_element()
    }

    /// What a formula or diagram being written looks like. Text under the caret shows only when the caret is in a formula.
    fn live_preview_of(&mut self, value: &str, caret: usize, single: bool, t: &Theme) -> Option<AnyElement> {
        let trimmed = value.trim_start();
        let whole = !single && (trimmed.starts_with("$$") || trimmed.starts_with("```mermaid"));
        let source = if whole {
            value.to_string()
        } else if single {
            match live_text::formula_at(value, caret) {
                Some(formula) => format!("$${formula}$$"),
                None => {
                    let line = self.live?.first_line;
                    live_text::open_formula(&self.source_lines, line)?
                }
            }
        } else {
            return None;
        };
        if self.live_preview.as_ref().is_none_or(|(known, _)| *known != source) {
            self.live_preview = Some((source.clone(), Rc::new(markdown_view::parse(&source))));
        }
        let parsed = self.live_preview.as_ref()?.1.clone();
        let base = self.path.parent().map(Path::to_path_buf);
        Some(div().mt(px(space::S)).p(px(space::S)).rounded(px(radius::INNER)).bg(t.secondary_bg).border_1().border_color(t.separator).child(markdown_view::render(&parsed, base.as_deref(), None, t)).into_any_element())
    }

    /// Floats over a selection with the formats it can get.
    fn format_bar(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let hover = t.ink.opacity(0.1);
        let button = |id: &'static str, name: &'static str, help: String, token: &'static str, cx: &mut Context<Self>| {
            div()
                .id(id)
                .size(px(control::S))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(radius::SMALL))
                .cursor_pointer()
                .soft(hover, radius::SMALL)
                .tooltip(tip(help))
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(move |n, _, w, cx| {
                    let (s, r) = n.with_text(cx);
                    match token {
                        "link" => {
                            let edit = if state::read(cx, |l| l.is_vault) && !index::current().config.use_markdown_links { edit_ops::wikilink(&s, r) } else { edit_ops::link(&s, r) };
                            n.apply(edit, w, cx);
                        }
                        "card" => {
                            let chosen = s.get(r).unwrap_or_default().trim().to_string();
                            let path = n.path.clone();
                            super::cards_page::card_from_selection(chosen, path, w, cx);
                        }
                        token => n.apply(edit_ops::wrap(&s, r, token), w, cx),
                    }
                }))
                .child(icon(name, iconsize::S, Theme::get(cx).primary))
        };
        let below = self.live.is_some_and(|l| l.first_line == 0);
        deferred(
            div()
                .absolute()
                .when(!below, |d| d.top(px(-(control::M + space::M))))
                .when(below, |d| d.top(px(control::M)))
                .left_0()
                .flex()
                .items_center()
                .gap(px(space::XXS))
                .p(px(space::XXS))
                .rounded(px(radius::CONTROL))
                .bg(t.material)
                .border_1()
                .border_color(t.separator)
                .shadow_md()
                .child(button("fmt-bold", "bold", tr("Fett"), "**", cx))
                .child(button("fmt-italic", "italic", tr("Kursiv"), "*", cx))
                .child(button("fmt-strike", "strikethrough", tr("Durchgestrichen"), "~~", cx))
                .child(button("fmt-mark", "highlighter", tr("Markiert"), "==", cx))
                .child(button("fmt-code", "code", tr("Code"), "`", cx))
                .child(button("fmt-math", "sigma", tr("Formel"), "$", cx))
                .child(button("fmt-link", "link", tr("Link"), "link", cx))
                .child(button("fmt-card", "layers", tr("Karte aus Auswahl…"), "card", cx)),
        )
        .with_priority(1)
        .into_any_element()
    }

    /// Replaces some lines of the note in one undoable step and writes on at `open`.
    fn edit_lines(&mut self, first: usize, count: usize, new: Vec<String>, open: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_block(cx);
        let before = self.current_text(cx);
        let mut lines: Vec<String> = before.split('\n').map(str::to_string).collect();
        let start = first.min(lines.len());
        let end = (first + count).min(lines.len());
        lines.splice(start..end, new);
        self.history.push((before, first));
        self.future.clear();
        self.last_snapshot = None;
        self.put_text(lines.join("\n"), cx);
        self.schedule_autosave(cx);
        if let Some(line) = open {
            self.open_line(line, None, window, cx);
        }
        cx.notify();
    }

    fn lines_at(&self, first: usize, count: usize, cx: &App) -> Vec<String> {
        self.current_text(cx).split('\n').skip(first).take(count).map(str::to_string).collect()
    }

    fn turn_lines(&mut self, first: usize, count: usize, kind: live_text::LineKind, window: &mut Window, cx: &mut Context<Self>) {
        let old = self.lines_at(first, count, cx);
        let refs: Vec<&str> = old.iter().map(String::as_str).collect();
        self.edit_lines(first, count, live_text::turn_into(&refs, kind), None, window, cx);
    }

    /// Moves lines to stand before `target`.
    fn move_lines_to(&mut self, first: usize, count: usize, target: usize, window: &mut Window, cx: &mut Context<Self>) {
        if target >= first && target <= first + count {
            return;
        }
        self.finish_block(cx);
        let before = self.current_text(cx);
        let mut lines: Vec<String> = before.split('\n').map(str::to_string).collect();
        let end = (first + count).min(lines.len());
        let moved: Vec<String> = lines.drain(first.min(end)..end).collect();
        let at = if target > first { target - moved.len() } else { target }.min(lines.len());
        lines.splice(at..at, moved);
        self.history.push((before, first));
        self.future.clear();
        self.last_snapshot = None;
        self.put_text(lines.join("\n"), cx);
        self.schedule_autosave(cx);
        let _ = window;
        cx.notify();
    }

    /// The line under the caret in the live view moves, copies or goes, and writing goes on there.
    fn live_line_action(&mut self, action: LineAction, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(live) = self.live.as_ref().filter(|l| l.cell.is_none()) else { return false };
        let (first, count) = (live.first_line, live.line_count);
        let caret = self.block.read(cx).cursor();
        let total = self.current_text(cx).split('\n').count();
        match action {
            LineAction::Up if first > 0 => {
                self.move_lines_to(first, count, first - 1, window, cx);
                self.open_line(first - 1, Some(caret), window, cx);
            }
            LineAction::Down if first + count < total => {
                self.move_lines_to(first, count, first + count + 1, window, cx);
                self.open_line(first + 1, Some(caret), window, cx);
            }
            LineAction::Copy => {
                let old = self.lines_at(first, count, cx);
                let mut new = old.clone();
                new.extend(old);
                self.edit_lines(first, count, new, Some(first + count), window, cx);
            }
            LineAction::Delete => {
                let after = first.min(total.saturating_sub(count + 1));
                self.edit_lines(first, count, Vec::new(), Some(after), window, cx);
            }
            _ => {}
        }
        true
    }

    /// The handles beside a block, after `extra`: + adds below, the grip can be dragged and opens the block's menu.
    #[allow(clippy::too_many_arguments)]
    fn gutter(me: &Entity<Self>, i: usize, line: usize, count: usize, is_table: bool, label: String, group: SharedString, extra: Option<AnyElement>, t: &Theme) -> Div {
        let (add_me, menu_me) = (me.clone(), me.clone());
        let hover = t.ink.opacity(0.1);
        let handle = |id: (&'static str, usize), name: &'static str| div().id(id).size(px(iconsize::L)).flex().items_center().justify_center().rounded(px(radius::SMALL)).cursor_pointer().soft(hover, radius::SMALL).child(icon(name, iconsize::S, t.tertiary));
        div()
            .flex()
            .justify_end()
            .items_center()
            .gap(px(space::XXS))
            .pr(px(space::XS))
            .invisible()
            .group_hover(group, |s| s.visible())
            .capture_any_mouse_down(|_, _, _| markdown_view::press_control())
            .children(extra)
            .child(handle(("live-add", i), "plus").tooltip(tip(tr("Darunter einfügen"))).on_click(move |_, window, cx| {
                add_me.update(cx, |n, cx| n.edit_lines(line + count, 0, vec![String::new(), String::new()], None, window, cx));
                let open_me = add_me.clone();
                window.on_next_frame(move |window, cx| {
                    open_me.update(cx, |n, cx| n.open_line(line + count + 1, None, window, cx))
                });
            }))
            .child(
                div()
                    .id(("live-grip", i))
                    .on_drag(DraggedLines { line, count, label }, |d, _, _, cx| cx.new(|_| d.clone()))
                    .child(Button::new(("live-grip-menu", i)).ghost().xsmall().icon(menu_icon("grip-vertical")).tooltip(tr("Ziehen zum Verschieben, Klick für das Menü")).dropdown_menu(move |m, _, _| block_menu(m, menu_me.clone(), line, count, is_table))),
            )
    }

    /// Marks unknown words a moment after typing stops.
    fn schedule_spelling(&mut self, cx: &mut Context<Self>) {
        if !prefs::get().spell_check {
            return;
        }
        let field = self.target();
        self.spell_task = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            cx.update(|cx| {
                field.update(cx, |s, cx| {
                    let text = s.value().to_string();
                    let position = |byte: usize| {
                        let before = &text[..byte];
                        let start = before.rfind('\n').map_or(0, |i| i + 1);
                        lsp_types::Position { line: before.matches('\n').count() as u32, character: before[start..].encode_utf16().count() as u32 }
                    };
                    let found: Vec<lsp_types::Diagnostic> = super::spelling::misspelled(&text)
                        .into_iter()
                        .map(|r| lsp_types::Diagnostic {
                            range: lsp_types::Range { start: position(r.start), end: position(r.end) },
                            severity: Some(lsp_types::DiagnosticSeverity::HINT),
                            message: tr("Unbekanntes Wort"),
                            ..Default::default()
                        })
                        .collect();
                    if let Some(set) = s.diagnostics_mut() {
                        set.clear();
                        set.extend(found);
                    }
                    cx.notify();
                })
            });
        }));
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

    /// Keeps editor and reader at the same relative position; whichever moved last leads.
    fn sync_split(&mut self, text: &str, cx: &mut Context<Self>) {
        let (editor_y, line) = {
            let e = self.editor.read(cx);
            (-f32::from(e.scroll_offset().y), e.line_height().map_or(0., f32::from))
        };
        let reader_y = -f32::from(self.reader_list.scroll_px_offset_for_scrollbar().y);
        let reader_max = f32::from(self.reader_list.max_offset_for_scrollbar().y);
        let (last_editor, last_reader) = self.split_sync;
        let (editor_moved, reader_moved) = ((editor_y - last_editor).abs() > 1., (reader_y - last_reader).abs() > 1.);
        if line <= 0. || reader_max <= 0. || !(editor_moved || reader_moved) {
            return;
        }
        let view = self.reader_list.viewport_bounds().size;
        let per_row = ((f32::from(view.width) - 56.) / (prefs::get().editor_font_size * 0.6)).max(20.);
        let rows: f32 = text.lines().map(|l| (l.chars().count() as f32 / per_row).ceil().max(1.)).sum();
        let editor_max = (rows * line - f32::from(view.height)).max(1.);
        if editor_moved {
            let y = (editor_y / editor_max).clamp(0., 1.) * reader_max;
            self.reader_list.set_offset_from_scrollbar(point(px(0.), px(-y)));
            self.split_sync = (editor_y, y);
        } else {
            let y = (reader_y / reader_max).clamp(0., 1.) * editor_max;
            self.editor.update(cx, |e, cx| e.set_scroll_offset(point(px(0.), px(-y)), cx));
            self.split_sync = (y, reader_y);
        }
    }

    fn focus_lost(&mut self, cx: &mut Context<Self>) {
        self.save(cx);
        self.remember_place();
    }

    /// Keeps where reading stopped, for coming back to it after a restart.
    fn remember_place(&self) {
        let folds: Vec<String> = self.link_focus.folds.borrow().iter().cloned().collect();
        let note = self.path.to_string_lossy().to_string();
        if prefs::get().note_folds.get(&note).map_or(!folds.is_empty(), |known| known.len() != folds.len() || known.iter().any(|k| !folds.contains(k))) {
            prefs::update(|p| {
                if folds.is_empty() {
                    p.note_folds.remove(&note);
                } else {
                    p.note_folds.insert(note.clone(), folds.clone());
                }
            });
        }
        let top = self.reader_list.logical_scroll_top();
        let offset = top.item_ix as f32 + f32::from(top.offset_in_item).clamp(0., 4095.) / 4096.;
        let key = self.path.to_string_lossy().to_string();
        let known = prefs::get().note_places.get(&key).copied().unwrap_or(0.);
        if self.place_restored && (known - offset).abs() > 0.001 {
            prefs::update(|p| {
                p.note_places.insert(key, offset);
                while p.note_places.len() > NOTE_PLACES {
                    let Some(first) = p.note_places.keys().next().cloned() else { break };
                    p.note_places.remove(&first);
                }
            });
        }
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

    fn conflict_notice(&self, open: bool, t: &Theme) -> impl IntoElement + use<> {
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
        let links: Vec<(String, usize)> = self.parsed.as_ref().map(|(_, p)| p.links().to_vec()).unwrap_or_default();
        let count = links.len();
        if matches!(self.mode, Mode::Source) || self.live.is_some() || count == 0 {
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
        let _ = window;
        let item = match (&self.parsed, self.mode) {
            (Some((_, p)), mode) if mode != Mode::Live => p.row_of_link(next) + 1,
            (p, _) => self.item_of_line(p.as_ref().and_then(|(_, p)| p.sources().get(links[next].1).map(|s| s.line)).unwrap_or(0)),
        };
        markdown_view::reveal_item(&self.reader_list, item);
        cx.notify();
    }

    /// Enter follows the focused link, Space shows the preview, Escape ends.
    fn link_keys(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::Source) || self.live.is_some() || window.has_active_dialog(cx) {
            return;
        }
        let k = &e.keystroke;
        if self.link_focus.index.is_none() {
            let plain = !k.modifiers.platform && !k.modifiers.control;
            match k.key.as_str() {
                "escape" if self.embedded => {
                    act(cx, |l| l.focus_explorer = true);
                    cx.stop_propagation();
                    return;
                }
                "enter" | "up" | "down" if plain && self.mode == Mode::Live => {
                    self.open_for_typing(None, window, cx);
                    cx.stop_propagation();
                    return;
                }
                _ if plain && self.mode == Mode::Live && k.key_char.as_deref().is_some_and(|c| !c.is_empty() && c != "\t") => {
                    self.open_for_typing(k.key_char.clone(), window, cx);
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }
        let url = self.link_focus.index.and_then(|i| self.parsed.as_ref().and_then(|(_, p)| p.links().get(i).map(|(u, _)| u.clone())));
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

    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        self.save(cx);
        super::pdf::export(&self.path, Some(text), window, cx);
    }

    fn apply(&mut self, edit: Edit, window: &mut Window, cx: &mut Context<Self>) {
        self.target().update(cx, |s, cx| {
            s.set_selected_range(edit.range.clone(), cx);
            s.replace(edit.text.clone(), window, cx);
            s.set_selected_range(edit.select.clone(), cx);
        });
    }

    /// The field typing goes to: the open block in the live view, the whole note in source view.
    fn target(&self) -> Entity<EditorState> {
        if self.live.is_some() { self.block.clone() } else { self.editor.clone() }
    }

    fn with_text(&mut self, cx: &mut Context<Self>) -> (String, std::ops::Range<usize>) {
        let target = self.target();
        let s = target.read(cx);
        (s.value().to_string(), s.selected_range())
    }

    fn zoom(delta: f32, cx: &mut App) {
        let size = if delta == 0. { 13. } else { (prefs::get().editor_font_size + delta).clamp(10., 24.) };
        set_font_size(size, cx);
    }

    fn toc_entries(&self) -> Option<Vec<(String, usize)>> {
        if matches!(self.mode, Mode::Source) {
            return Some(Vec::new());
        }
        let (_, p) = self.parsed.as_ref()?;
        let headings = p.headings();
        let anchors: Vec<usize> = p.heading_blocks().into_iter().filter_map(|b| p.sources().get(b).map(|s| self.item_of_line(s.line))).collect();
        if headings.len() < 2 || anchors.len() != headings.len() {
            return Some(Vec::new());
        }
        let top = headings.iter().map(|(level, _)| *level).min().unwrap_or(1);
        Some(headings.iter().zip(&anchors).map(|((level, text), block)| (format!("{}{text}", "    ".repeat((level - top) as usize)), *block)).collect())
    }

    fn header(&self, exporting: bool, room: Option<Room>, t: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let me = cx.entity();
        let keyed = |label: &str, m: Mode| format!("{label}{}", if m == Mode::Split { ShortcutAction::SplitPreview } else { ShortcutAction::TogglePreview }.hint());
        let edit_mode = if self.editor_kind == prefs::NoteEditor::Live { Mode::Live } else { Mode::Source };
        let mode_list: Vec<(Mode, String, &'static str)> = [(Mode::Read, tr("Lesen"), "eye"), (edit_mode, tr("Bearbeiten"), "pencil"), (Mode::Split, tr("Geteilte Ansicht"), "columns-2")].into_iter().filter(|(m, ..)| *m != Mode::Split || self.editor_kind == prefs::NoteEditor::Code).collect();
        let entries = self.toc.clone();
        let has_toc = !entries.is_empty();
        let segment = 2. * space::XXS + mode_list.len() as f32 * control::S;
        let toc_w = if has_toc { control::S } else { 0. };
        let pdf_w = bar_fit::button_width(&tr("PDF"), window);
        let more = control::S;
        let steps = [bar_fit::row(&[segment, toc_w, pdf_w, more], space::S), bar_fit::row(&[segment, toc_w, more], space::S), bar_fit::row(&[segment, more], space::S), bar_fit::row(&[control::S, more], space::S), more];
        let fold = room.map_or_else(|| actions_fold(&self.path, &steps, 2, window, cx), |room| bar_fit::fold_room(room, &steps, 2));
        let (pdf_out, toc_out, picker, modes_out) = (fold >= 1, fold >= 2, fold == 3, fold >= 4);
        let mut modes = div().flex().flex_none().rounded(px(radius::INNER)).bg(t.ink.opacity(0.06)).p(px(space::XXS));
        for (m, label, icon_name) in mode_list.iter().cloned() {
            let active = m == self.mode;
            let e = me.clone();
            let hint = keyed(&label, m);
            modes = modes.child(
                div()
                    .id(SharedString::from(format!("mode-{icon_name}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(control::S))
                    .h(px(control::S - 2. * space::XXS))
                    .rounded(px(radius::SMALL))
                    .cursor_pointer().key_nav()
                    .when(!active, |d| d.soft(t.ink.opacity(0.06), radius::SMALL))
                    .when(active, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm())
                    .child(icon(icon_name, iconsize::XS, if active { t.primary } else { t.secondary }))
                    .tooltip(tip(hint))
                    .on_click(move |_, window, cx| e.update(cx, |n, cx| n.set_mode(m, window, cx))),
            );
        }
        let (e1, e2, path, p2, p3) = (me.clone(), me.clone(), self.path.clone(), self.path.clone(), self.path.clone());
        let current_mode = self.mode;
        let live_editor = self.editor_kind == prefs::NoteEditor::Live;
        let source_me = me.clone();
        let modes_me = me.clone();
        let embedded = self.embedded;
        let in_vault = super::vault_tools::in_vault(&self.path, cx);
        let list = self.reader_list.clone();
        let toc_menu = {
            move |mut menu: gpui_kit::component::menu::PopupMenu| {
                for (text, block) in entries.iter() {
                    let (block, list) = (*block, list.clone());
                    menu = menu.item(PopupMenuItem::new(text.clone()).on_click(move |_, _, _| list.scroll_to(ListOffset { item_ix: block, offset_in_item: px(0.) })));
                }
                menu
            }
        };
        let toc = (has_toc && !toc_out).then(|| {
            let toc_menu = toc_menu.clone();
            Button::new("note-toc").ghost().small().icon(menu_icon("list")).tooltip(tr("Inhalt")).dropdown_menu(move |menu, _, _| toc_menu(menu))
        });
        let pdf_tip = tr("Als PDF neben der Notiz sichern (⇧⌘E)");
        let shown_mode = mode_list.iter().find(|(m, ..)| *m == current_mode || (*m == edit_mode && current_mode == Mode::Source)).cloned();
        let mode_picker = picker.then(|| {
            let (list, e) = (mode_list.clone(), me.clone());
            let (label, icon_name) = shown_mode.as_ref().or_else(|| mode_list.first()).map_or_else(|| (String::new(), "eye"), |(_, l, i)| (l.clone(), *i));
            Button::new("note-mode").ghost().small().icon(menu_icon(icon_name)).tooltip(keyed(&label, current_mode)).dropdown_menu(move |mut menu, _, _| {
                for (m, label, _) in list.iter().cloned() {
                    let e = e.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(m == current_mode || (m == edit_mode && current_mode == Mode::Source)).on_click(move |_, window, cx| e.update(cx, |n, cx| n.set_mode(m, window, cx))));
                }
                menu
            })
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(space::S))
            .when(!picker && !modes_out, |d| d.child(modes))
            .children(mode_picker)
            .children(toc)
            .when(!pdf_out, |d| d.child(Button::new("note-pdf").small().icon(menu_icon("share")).label(tr("PDF")).tooltip(pdf_tip.clone()).disabled(exporting).on_click(move |_, window, cx| e1.update(cx, |n, cx| n.export(window, cx)))))
            .child(Button::new("note-more").ghost().small().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu(move |menu, window, cx| {
                let (a, b, c, d) = (path.clone(), p2.clone(), p3.clone(), path.clone());
                let mut menu = menu;
                if modes_out {
                    for (m, label, _) in mode_list.iter().cloned() {
                        let e = modes_me.clone();
                        menu = menu.item(PopupMenuItem::new(label).checked(m == current_mode || (m == edit_mode && current_mode == Mode::Source)).on_click(move |_, window, cx| e.update(cx, |n, cx| n.set_mode(m, window, cx))));
                    }
                    menu = menu.separator();
                }
                if toc_out && has_toc {
                    let toc_menu = toc_menu.clone();
                    menu = menu.submenu_with_icon(Some(menu_icon("list")), tr("Inhalt"), window, cx, move |m, _, _| toc_menu(m));
                }
                if pdf_out {
                    let e = e2.clone();
                    menu = menu.item(PopupMenuItem::new(pdf_tip.clone()).icon(menu_icon("share")).disabled(exporting).on_click(move |_, window, cx| e.update(cx, |n, cx| n.export(window, cx))));
                }
                if modes_out || toc_out || pdf_out {
                    menu = menu.separator();
                }
                if embedded {
                    menu = menu.item(PopupMenuItem::new(tr("In eigenem Fenster öffnen")).icon(menu_icon("app-window")).on_click(move |_, _, cx| {
                        super::note_window::open(&a, cx);
                        act(cx, |l| l.close_document(&a));
                    }));
                }
                menu = menu
                    .item(PopupMenuItem::new(super::platform::reveal_label()).icon(menu_icon("folder")).on_click(move |_, _, _| library::reveal(&b)))
                    .item(PopupMenuItem::new(tr("Mit Standard-App öffnen")).icon(menu_icon("square-arrow-out-up-right")).on_click(move |_, _, _| library::open_path(&c)));
                if super::convention_rename::available(&d, cx) {
                    let n = d.clone();
                    menu = menu.item(PopupMenuItem::new(tr("Nach Konvention benennen…")).icon(menu_icon("wand-sparkles")).on_click(move |_, window, cx| super::convention_rename::open(n.clone(), window, cx)));
                }
                let source_view = source_me.clone();
                let is_source = current_mode == Mode::Source;
                menu = menu.when(live_editor, |menu| menu.item(PopupMenuItem::new(tr("Quelltext")).icon(menu_icon("code")).checked(is_source).on_click(move |_, window, cx| source_view.update(cx, |n, cx| n.set_mode(if is_source { Mode::Live } else { Mode::Source }, window, cx)))));
                let full = prefs::get().reader_full_width;
                menu = menu.item(PopupMenuItem::new(tr("Text in voller Breite")).checked(full).on_click(move |_, _, cx| set_reader_full_width(!full, cx)));
                let g = d.clone();
                menu = menu.item(PopupMenuItem::new(tr("Graph um diese Notiz")).icon(menu_icon("waypoints")).on_click(move |_, _, cx| {
                            let p = g.clone();
                            act(cx, |l| l.show_graph(Some(p)));
                        }));
                if in_vault {
                    let o = d.clone();
                    menu = menu.item(PopupMenuItem::new(tr("In Obsidian öffnen")).icon(menu_icon("external-link")).on_click(move |_, _, cx| super::vault_tools::open_in_obsidian(&o, cx)));
                }
                menu = menu.item(PopupMenuItem::new(tr("Verlauf")).icon(menu_icon("history")).on_click(|_, _, cx| {
                    act(cx, |l| {
                        l.peek_inspector();
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

    fn reader(&mut self, lib_text: &str, t: &Theme, window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let me = _cx.entity();
        if lib_text.trim().is_empty() && self.live.is_none() && self.mode != Mode::Live {
            let empty_me = me.clone();
            return div()
                .id("empty-note")
                .size_full()
                .p(px(space::XXL))
                .cursor_text()
                .text_color(t.secondary)
                .child(tr("Leere Notiz. Klicke hier, um zu schreiben."))
                .when(self.mode == Mode::Live, |d| d.on_click(move |_, window, cx| empty_me.update(cx, |n, cx| n.open_line(0, None, window, cx))))
                .into_any_element();
        }
        let parsed = match (&self.parsed, &self.live) {
            (Some((_, p)), Some(_)) => p.clone(),
            _ => self.parse_now(lib_text),
        };
        if let Some(line) = self.keep_line.take() {
            self.reader_list.scroll_to(ListOffset { item_ix: self.item_of_line(line), offset_in_item: px(0.) });
        }
        if !std::mem::replace(&mut self.place_restored, true)
            && let Some(offset) = prefs::get().note_places.get(&self.path.to_string_lossy().to_string()).copied()
        {
            let item_ix = (offset.max(0.).trunc() as usize).min(self.reader_list.item_count().saturating_sub(1));
            self.reader_list.scroll_to(ListOffset { item_ix, offset_in_item: px(offset.fract() * 4096.) });
        }
        if let Some(line) = self.reveal_line.take() {
            markdown_view::reveal_item(&self.reader_list, self.item_of_line(line));
        }
        let path = self.path.clone();
        let toggle: markdown_view::OnToggle = Rc::new(move |line, cx: &mut App| {
            let path = path.clone();
            act(cx, |l| {
                l.edit_note(&path, "task", tr("Aufgabe geändert"), |text| markdown::toggle_task(text, line));
            });
        });
        let base = self.path.parent().map(Path::to_path_buf);
        let plugin = super::embeds::note_plugin(&self.path, lib_text).map(|name| (name.to_string(), self.path.clone()));
        let found = self.found.clone();
        let mut focus = self.link_focus.clone();
        let segments = self.segments.clone();
        let source_lines = self.source_lines.clone();
        if self.mode == Mode::Live {
            let (click_me, lines, shown) = (me.clone(), source_lines.clone(), segments.clone());
            focus.text_click = Some(Rc::new(move |item, cell, drawn, at, window, cx| {
                let Some(segment) = shown.get(item).copied() else { return };
                let text = lines.get(segment.line..(segment.line + segment.count).min(lines.len())).unwrap_or_default().join("\n");
                match cell {
                    Some((row, column)) => {
                        let row_line = segment.line + if row == 0 { 0 } else { row + 1 };
                        let raw = lines.get(row_line).map_or(String::new(), |l| live_text::table_cells(l).get(column).map_or(String::new(), |c| l[c.clone()].to_string()));
                        let caret = markdown_view::source_offset(&raw, drawn, at);
                        click_me.update(cx, |n, cx| n.open_cell(item, row, column, Some(caret), window, cx));
                    }
                    None => {
                        let caret = markdown_view::source_offset(&text, drawn, at);
                        let (in_block, line) = match segment.kind {
                            SegmentKind::Whole { .. } => (caret, segment.line),
                            _ => (caret, segment.line),
                        };
                        click_me.update(cx, |n, cx| n.open_line(line, Some(in_block), window, cx));
                    }
                }
            }));
        }
        if self.mode == Mode::Live {
            let (shown, note) = (segments.clone(), self.path.clone());
            focus.edit_item = Some(Rc::new(move |item, change, cx| {
                let Some(segment) = shown.get(item).copied() else { return };
                let note = note.clone();
                act(cx, |l| {
                    l.edit_note(&note, "live", tr("Notiz geändert"), |text| {
                        let lines: Vec<&str> = text.split('\n').collect();
                        let end = (segment.line + segment.count).min(lines.len());
                        let part = lines[segment.line.min(end)..end].join("\n");
                        let changed = change(&part);
                        [&lines[..segment.line.min(end)], &changed.split('\n').collect::<Vec<_>>()[..], &lines[end..]].concat().join("\n")
                    });
                });
            }));
        }
        let live_mode = self.mode == Mode::Live;
        let live_item = self.live.as_ref().filter(|l| l.cell.is_none()).map(|l| l.index);
        let covered = self.live.as_ref().filter(|l| l.selecting).map(|l| (l.first_line, l.line_count));
        let live_cell = self.live.as_ref().and_then(|l| l.cell.map(|c| (l.index, c.row, c.column)));
        let hidden = parsed.hidden_blocks(&self.link_focus.folds);
        let hidden_lines: Rc<Vec<bool>> = Rc::new({
            let mut lines = vec![false; source_lines.len()];
            for (source, is_hidden) in parsed.sources().iter().zip(&hidden) {
                if *is_hidden {
                    for slot in lines.iter_mut().take(source.end_line).skip(source.line) {
                        *slot = true;
                    }
                }
            }
            for i in 1..lines.len() {
                if lines[i - 1] && source_lines[i].trim().is_empty() {
                    lines[i] = true;
                }
            }
            lines
        });
        let items = if live_mode { (segments.len() + 2).max(if self.live.is_some() { 3 } else { 2 }) } else { parsed.rows().len() + 2 };
        let measured = self.reader_list.viewport_bounds().size.width;
        if measured > px(0.) {
            self.reader_width = measured;
        }
        let footnote_back: Rc<dyn Fn(usize) -> usize> = if live_mode {
            let shown = segments.clone();
            Rc::new(move |line| shown.iter().position(|s| line < s.line + s.count).map_or(shown.len(), |i| i + 1))
        } else {
            let note = parsed.clone();
            Rc::new(move |line| note.row_of_line(line) + 1)
        };
        let footnote_list = (self.reader_list.clone(), footnote_back);
        let full = self.measured_full;
        let left = self.column_left();
        let content = move |inner: AnyElement| reader_column(full, left, inner);
        let (fragments, definitions) = (self.fragments.clone(), self.definitions.clone());
        let open_me = me.clone();
        let opener = move |line: usize| {
            let me = open_me.clone();
            move |_: &MouseUpEvent, window: &mut Window, cx: &mut App| {
                if markdown_view::take_control_press() || !gpui_kit::base::TextSelection::selected_text(window, cx).is_empty() {
                    return;
                }
                me.update(cx, |n, cx| n.open_line(line, None, window, cx));
            }
        };
        let drop_me = me.clone();
        let gutter_me = me.clone();
        let row = list(self.reader_list.clone(), move |i, window, cx| {
            let t = Theme::get(cx);
            if i == 0 {
                let notice_row = plugin.as_ref().map(|(name, path)| plugin_notice(name, path, &t));
                return content(div().pt(px(space::XL)).children(notice_row).into_any_element());
            }
            if i + 1 == items {
                let last_line = source_lines.len().saturating_sub(1);
                let footer = div().id("note-end").min_h(px(160.)).pb(px(space::XXL)).children(markdown_view::render_footnotes(&parsed, Some(footnote_list.clone()), &t)).when(live_mode, |d| d.cursor_text().on_mouse_up(MouseButton::Left, opener(last_line)));
                return content(footer.into_any_element());
            }
            if !live_mode {
                let Some(row) = parsed.rows().get(i - 1).copied() else { return div().into_any_element() };
                if hidden.get(row.block).copied().unwrap_or(false) {
                    return div().into_any_element();
                }
                return content(div().id(("read-item", i)).pb(px(parsed.row_gap(i - 1))).child(markdown_view::render_row(&parsed, &row, base.as_deref(), Some(toggle.clone()), found.as_ref(), Some(&focus), &t)).into_any_element());
            }
            let Some(segment) = segments.get(i - 1).copied() else {
                let field = me.update(cx, |n, cx| n.live_field(window, cx));
                return content(field);
            };
            if hidden_lines.get(segment.line).copied().unwrap_or(false) {
                return div().into_any_element();
            }
            if let Some((first, count)) = covered
                && live_item != Some(i - 1)
                && segment.line >= first
                && segment.line < first + count
            {
                return div().into_any_element();
            }
            if live_item == Some(i - 1) {
                let field = me.update(cx, |n, cx| n.live_field(window, cx));
                let text = source_lines.get(segment.line).map_or("", |s| s.as_str());
                let bare = text.trim_start();
                let top = if bare.starts_with("# ") || bare.starts_with("## ") { space::S } else if bare.starts_with('#') { space::XS } else { 0. };
                let gap = if live_text::prefix_len(text) > 0 && !bare.starts_with('#') { space::XS } else { 0. };
                return content(div().pt(px(top)).pb(px(gap)).child(field).into_any_element());
            }
            let line = segment.line;
            let raw_middle = note_size(font::CALLOUT) * 0.75;
            let (body, lead, middle, fold, extra): (AnyElement, f32, Option<f32>, bool, Option<AnyElement>) = match segment.kind {
                SegmentKind::Blank => (div().h(px(note_size(font::BODY) * 1.5)).into_any_element(), 0., None, false, None),
                SegmentKind::Raw => (div().text_size(px(note_size(font::CALLOUT))).line_height(relative(1.5)).text_color(t.tertiary).child(source_lines.get(line).cloned().unwrap_or_default()).into_any_element(), 0., Some(raw_middle), false, None),
                SegmentKind::Line { depth } => {
                    let written = source_lines.get(line).map_or("", |s| s.as_str());
                    let continues = source_lines.get(line + 1).is_some_and(|next| live_text::continues_paragraph(written, next));
                    let shown = live_text::drawn_line(written, continues);
                    let fragment = {
                        let mut known = fragments.borrow_mut();
                        if known.len() > 20_000 {
                            known.clear();
                        }
                        known.entry(shown.to_string()).or_insert_with(|| Rc::new(markdown_view::parse(&format!("{shown}\n\n{definitions}")))).clone()
                    };
                    let lead = depth as f32 * LIVE_INDENT;
                    if fragment.len() == 0 {
                        (div().text_size(px(note_size(font::CALLOUT))).line_height(relative(1.5)).text_color(t.tertiary).child(written.to_string()).into_any_element(), 0., Some(raw_middle), false, None)
                    } else {
                        let offset_toggle: markdown_view::OnToggle = {
                            let inner = toggle.clone();
                            Rc::new(move |_, cx: &mut App| inner(line, cx))
                        };
                        let draw = markdown_view::LiveDraw { item: Some(i - 1), line, footnotes: Some(parsed.footnote_order()), open_cell: None, count: fragment.line_blocks() };
                        let drawn = markdown_view::render_live(&fragment, 0, base.as_deref(), Some(offset_toggle), None, Some(&focus), draw, &t);
                        let is_list = live_text::prefix_len(written) > 0 && !written.trim_start().starts_with('#') && !written.trim_start().starts_with('>');
                        (div().pl(px(lead)).when(is_list, |d| d.pb(px(space::XS))).child(drawn).into_any_element(), lead, fragment.first_line_middle(0), fragment.has_margin_control(0), None)
                    }
                }
                SegmentKind::Whole { block } => {
                    let (middle, fold) = (parsed.first_line_middle(block), parsed.has_margin_control(block));
                    if live_item == Some(i - 1) {
                        (div().into_any_element(), 0., middle, fold, None)
                    } else {
                        let drawn = markdown_view::render_live(&parsed, block, base.as_deref(), Some(toggle.clone()), found.as_ref(), Some(&focus), markdown_view::LiveDraw { item: Some(i - 1), count: parsed.blocks_sharing(block), ..Default::default() }, &t);
                        let fence = source_lines.get(line).map_or("", |l| l.trim_start().trim_start_matches('`').trim());
                        if ["mermaid", "query", "base"].contains(&fence) && source_lines.get(line).is_some_and(|l| l.trim_start().starts_with("```")) {
                            let edit_me = me.clone();
                            let edit = div()
                                .id(("live-fence-edit", i))
                                .size(px(iconsize::L))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(radius::SMALL))
                                .cursor_pointer()
                                .soft(t.ink.opacity(0.1), radius::SMALL)
                                .tooltip(tip(tr("Quelltext bearbeiten")))
                                .on_click(move |_, window, cx| edit_me.update(cx, |n, cx| n.open_line(line, Some(0), window, cx)))
                                .child(icon("code-xml", iconsize::S, t.tertiary))
                                .into_any_element();
                            (div().pb(px(space::XS)).capture_any_mouse_down(|_, _, _| markdown_view::press_control()).child(drawn).into_any_element(), 0., middle, fold, Some(edit))
                        } else {
                            (div().pb(px(space::XS)).child(drawn).into_any_element(), 0., middle, fold, None)
                        }
                    }
                }
                SegmentKind::Table { block } => {
                    let open_cell = live_cell.filter(|(item, ..)| *item == i - 1).map(|(_, row, column)| {
                        let field = me.update(cx, |n, cx| n.live_field(window, cx));
                        (row, column, std::cell::RefCell::new(Some(field)))
                    });
                    let drawn = markdown_view::render_live(&parsed, block, base.as_deref(), Some(toggle.clone()), None, Some(&focus), markdown_view::LiveDraw { item: Some(i - 1), open_cell, ..Default::default() }, &t);
                    (div().pb(px(space::XS)).child(drawn).into_any_element(), 0., parsed.first_line_middle(block), false, None)
                }
            };
            let group = SharedString::from(format!("live-row-{i}"));
            let accent = t.accent;
            let count = segment.count;
            let is_table = matches!(segment.kind, SegmentKind::Table { .. });
            let label = source_lines.get(line).map_or(String::new(), |l| l.trim().chars().take(60).collect());
            let table_tools = is_table.then(|| table_tools(&gutter_me, i, line, count, group.clone(), &t));
            let drop_target = drop_me.clone();
            let handle = (!matches!(segment.kind, SegmentKind::Blank)).then(|| {
                let width = GUTTER_WIDTH + if extra.is_some() { iconsize::L + space::XXS } else { 0. };
                let right = lead - if fold { FOLD_ROOM } else { 0. };
                let gutter = Self::gutter(&gutter_me, i, line, count, is_table, label, group.clone(), extra, &t).absolute().left(px(right - width)).w(px(width));
                match middle {
                    Some(middle) => gutter.top(px(middle - iconsize::L / 2.)),
                    None => gutter.top_0().bottom_0(),
                }
                .into_any_element()
            });
            content(
                div()
                    .id(("live-item", i))
                    .relative()
                    .ml(px(-LIVE_ROOM))
                    .pl(px(LIVE_ROOM))
                    .group(group)
                    .cursor_text()
                    .on_mouse_up(MouseButton::Left, opener(line))
                    .drag_over::<DraggedLines>(move |s, _, _, _| s.border_t_2().border_color(accent))
                    .on_drop(move |d: &DraggedLines, window, cx| drop_target.update(cx, |n, cx| n.move_lines_to(d.line, d.count, line, window, cx)))
                    .child(div().relative().child(body).children(handle).children(table_tools))
                    .into_any_element(),
            )
        });
        div().relative().size_full().bg(t.content).child(row.size_full()).vertical_scrollbar(&self.reader_list).into_any_element()
    }

    /// Tells the list which items changed, so everything else keeps its measured height.
    fn adopt_items(&mut self, new: Vec<u64>) {
        let old = std::mem::take(&mut self.item_hashes);
        let target = new.len() + 2;
        if self.reader_list.item_count() != old.len() + 2 {
            self.reader_list.reset(target);
        } else {
            let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
            let suffix = old.iter().rev().zip(new.iter().rev()).take_while(|(a, b)| a == b).count().min(old.len().min(new.len()) - prefix);
            if prefix + suffix != old.len() || old.len() != new.len() {
                self.reader_list.splice(prefix + 1..old.len() - suffix + 1, new.len() - prefix - suffix);
            }
        }
        self.item_hashes = new;
    }

    /// Pasting an image puts it into the attachment folder, an address over a selection makes a link.
    fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else { return };
        for entry in item.entries() {
            if let ClipboardEntry::Image(image) = entry {
                let name = format!("{} {}.png", tr("Eingefügtes Bild"), chrono::Local::now().format("%Y%m%d%H%M%S"));
                let target = std::env::temp_dir().join(&name);
                if std::fs::write(&target, &image.bytes).is_ok() {
                    self.drop_images(std::slice::from_ref(&target), window, cx);
                    let _ = std::fs::remove_file(&target);
                    cx.stop_propagation();
                }
                return;
            }
        }
        let Some(pasted) = item.text() else { return };
        let (s, r) = self.with_text(cx);
        if let Some(edit) = edit_ops::pasted_link(&s, r, &pasted) {
            self.apply(edit, window, cx);
            cx.stop_propagation();
        }
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
        let field = writing_menu(Editor::new(&self.editor).appearance(false).h_full().bg(Theme::get(cx).content));
        self.note_editor_wrapper(div().size_full().child(field).into_any_element(), cx)
    }

    /// Everything a writing field reacts to, for the whole note and for a single block.
    fn note_editor_wrapper(&self, field: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let accent = Theme::get(cx).accent;
        div()
            .w_full()
            .when(self.live.is_none(), |d| d.h_full())
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
            .on_action(cx.listener(|n, _: &CardFromSelection, w, cx| {
                let (text, range) = n.with_text(cx);
                let chosen = text.get(range).unwrap_or_default().trim().to_string();
                let path = n.path.clone();
                super::cards_page::card_from_selection(chosen, path, w, cx);
            }))
            .on_action(cx.listener(|n, _: &ExtractSelection, w, cx| {
                let (_, range) = n.with_text(cx);
                let path = n.path.clone();
                if act(cx, |l| l.extract_to_note(&path, range)).is_some() {
                    state::reindex(cx);
                    let text = state::lib(cx).update(cx, |l, _| l.buffer(&path).text.clone());
                    n.target().update(cx, |e, cx| e.set_value(text, w, cx));
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
                if n.live_line_action(LineAction::Up, w, cx) {
                    return;
                }
                let (s, r) = n.with_text(cx);
                if let Some(e) = edit_ops::move_lines(&s, r, true) {
                    n.apply(e, w, cx)
                }
            }))
            .on_action(cx.listener(|n, _: &MoveLineDown, w, cx| {
                if n.live_line_action(LineAction::Down, w, cx) {
                    return;
                }
                let (s, r) = n.with_text(cx);
                if let Some(e) = edit_ops::move_lines(&s, r, false) {
                    n.apply(e, w, cx)
                }
            }))
            .on_action(cx.listener(|n, _: &CopyLineUp, w, cx| {
                if n.live_line_action(LineAction::Copy, w, cx) {
                    return;
                }
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::copy_lines(&s, r, true), w, cx)
            }))
            .on_action(cx.listener(|n, _: &CopyLineDown, w, cx| {
                if n.live_line_action(LineAction::Copy, w, cx) {
                    return;
                }
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::copy_lines(&s, r, false), w, cx)
            }))
            .on_action(cx.listener(|n, _: &DeleteLine, w, cx| {
                if n.live_line_action(LineAction::Delete, w, cx) {
                    return;
                }
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::delete_lines(&s, r), w, cx)
            }))
            .on_action(cx.listener(|n, _: &SelectLine, _, cx| {
                let (s, r) = n.with_text(cx);
                let sel = edit_ops::select_lines(&s, r);
                n.target().update(cx, |e, cx| e.set_selected_range(sel, cx));
            }))
            .on_action(cx.listener(|n, _: &IndentLines, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::shift_lines(&s, r, 1), w, cx)
            }))
            .on_action(cx.listener(|n, _: &OutdentLines, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::shift_lines(&s, r, -1), w, cx)
            }))
            .capture_key_down(cx.listener(|n, e: &KeyDownEvent, w, cx| {
                let k = &e.keystroke;
                if k.modifiers.platform || k.modifiers.control || !n.target().read(cx).focus_handle(cx).is_focused(w) {
                    return;
                }
                if n.live.is_some() && !n.target().read(cx).completion_menu_state().open {
                    let (s, r) = n.with_text(cx);
                    if r.is_empty() {
                        let edit = if k.key == "backspace" { edit_ops::empty_pair(&s, r.start) } else { k.key_char.as_deref().and_then(|typed| edit_ops::typed_pair(&s, r.start, typed)) };
                        if let Some(edit) = edit {
                            n.apply(edit, w, cx);
                            cx.stop_propagation();
                            return;
                        }
                    }
                }
                let Some((open, close)) = k.key_char.as_deref().and_then(edit_ops::surround_pair) else { return };
                let (s, r) = n.with_text(cx);
                if r.is_empty() {
                    return;
                }
                n.apply(edit_ops::surround(&s, r, open, close), w, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|n, _: &PlainNewline, w, cx| {
                if n.live.is_some_and(|l| l.cell.is_some()) {
                    return;
                }
                let (_, r) = n.with_text(cx);
                let end = r.start + 1;
                n.apply(Edit { range: r, text: "\n".into(), select: end..end }, w, cx);
            }))
            .on_action(cx.listener(|n, _: &Strike, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::wrap(&s, r, "~~"), w, cx)
            }))
            .on_action(cx.listener(|n, _: &Mark, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::wrap(&s, r, "=="), w, cx)
            }))
            .on_action(cx.listener(|n, _: &InlineMath, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::wrap(&s, r, "$"), w, cx)
            }))
            .on_action(cx.listener(|n, _: &Quote, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::toggle_line_mark(&s, r, "> "), w, cx)
            }))
            .on_action(cx.listener(|n, _: &BulletList, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::toggle_line_mark(&s, r, "- "), w, cx)
            }))
            .on_action(cx.listener(|n, _: &NumberedList, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::toggle_line_mark(&s, r, "1. "), w, cx)
            }))
            .on_action(cx.listener(|n, _: &Heading1, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::set_heading(&s, r, 1), w, cx)
            }))
            .on_action(cx.listener(|n, _: &Heading2, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::set_heading(&s, r, 2), w, cx)
            }))
            .on_action(cx.listener(|n, _: &Heading3, w, cx| {
                let (s, r) = n.with_text(cx);
                n.apply(edit_ops::set_heading(&s, r, 3), w, cx)
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::Paste, w, cx| n.paste(w, cx)))
            .on_action(cx.listener(|n, _: &ListNewline, w, cx| {
                if n.live.is_some_and(|l| l.cell.is_some()) {
                    if n.target().read(cx).completion_menu_state().open {
                        cx.propagate();
                    } else {
                        n.step_cell(true, false, w, cx);
                    }
                    return;
                }
                if n.target().read(cx).completion_menu_state().open {
                    cx.propagate();
                    return;
                }
                let (s, r) = n.with_text(cx);
                if r.is_empty() {
                    let (full, line) = match n.live {
                        Some(live) => (n.current_text(cx), live.first_line + s[..r.start].matches('\n').count()),
                        None => (s.clone(), s[..r.start].matches('\n').count()),
                    };
                    if let Some(mark) = edit_ops::closing_mark(&full, line).filter(|_| s[r.start..].split('\n').next().is_some_and(|rest| rest.trim().is_empty())) {
                        let inside = r.start + 1;
                        n.apply(Edit { range: r.clone(), text: format!("\n\n{mark}"), select: inside..inside }, w, cx);
                        return;
                    }
                }
                match (r.is_empty(), edit_ops::newline(&s, r.start)) {
                    (true, Some(e)) => n.apply(e, w, cx),
                    _ => n.apply(Edit { range: r.clone(), text: "\n".into(), select: r.start + 1..r.start + 1 }, w, cx),
                }
            }))
            .on_action(cx.listener(|n, _: &ListTab, w, cx| {
                if n.live.is_some_and(|l| l.cell.is_some()) {
                    n.step_cell(false, false, w, cx);
                    return;
                }
                if n.target().read(cx).completion_menu_state().open {
                    cx.propagate();
                    return;
                }
                let (s, r) = n.with_text(cx);
                if let Some(next) = edit_ops::table_cell(&s, r.start, false) {
                    n.target().update(cx, |t, cx| t.set_selected_range(next.clone(), cx));
                    return;
                }
                if let Some(edit) = edit_ops::new_table_row(&s, r.start) {
                    n.apply(edit, w, cx);
                    return;
                }
                if edit_ops::tab_indents(&s, &r) {
                    n.apply(edit_ops::shift_lines(&s, r, 1), w, cx)
                } else {
                    n.apply(Edit { range: r.clone(), text: "    ".into(), select: r.start + 4..r.start + 4 }, w, cx)
                }
            }))
            .on_action(cx.listener(|n, _: &ListBacktab, w, cx| {
                if n.live.is_some_and(|l| l.cell.is_some()) {
                    n.step_cell(false, true, w, cx);
                    return;
                }
                let (s, r) = n.with_text(cx);
                if let Some(next) = edit_ops::table_cell(&s, r.start, true) {
                    n.target().update(cx, |t, cx| t.set_selected_range(next.clone(), cx));
                    return;
                }
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
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::Undo, w, cx| {
                if n.live.is_some() {
                    cx.stop_propagation();
                    n.step_history(true, w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::Redo, w, cx| {
                if n.live.is_some() {
                    cx.stop_propagation();
                    n.step_history(false, w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::MoveToStart, w, cx| {
                if n.live.is_some_and(|l| !l.selecting) {
                    cx.stop_propagation();
                    n.open_line(0, Some(0), w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::MoveToEnd, w, cx| {
                if n.live.is_some_and(|l| !l.selecting) {
                    cx.stop_propagation();
                    let total = n.current_text(cx).split('\n').count();
                    n.open_line(total.saturating_sub(1), None, w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::MoveUp, w, cx| {
                if n.at_block_edge(false, w, cx) {
                    cx.stop_propagation();
                    n.cross_block(false, w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::MoveDown, w, cx| {
                if n.at_block_edge(true, w, cx) {
                    cx.stop_propagation();
                    n.cross_block(true, w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &super::main_window::Dismiss, w, cx| {
                if std::mem::take(&mut n.source_detour) {
                    cx.stop_propagation();
                    n.set_mode(Mode::Live, w, cx);
                } else {
                    n.leave_block(w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::Escape, w, cx| n.leave_block(w, cx)))
            .capture_action(cx.listener(|n, _: &gpui_kit::base::actions::SelectUp, w, cx| {
                if n.at_block_edge_selecting(false, cx) {
                    cx.stop_propagation();
                    n.select_beyond_line(false, w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::base::actions::SelectDown, w, cx| {
                if n.at_block_edge_selecting(true, cx) {
                    cx.stop_propagation();
                    n.select_beyond_line(true, w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::SelectAll, w, cx| {
                if n.live.is_some() {
                    let b = n.block.read(cx);
                    if b.selected_range() == (0..b.value().len()) {
                        cx.stop_propagation();
                        n.select_whole_note(w, cx);
                    }
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::MoveLeft, w, cx| {
                if n.live.is_some_and(|l| l.cell.is_none()) && n.block.read(cx).selected_range() == (0..0) {
                    cx.stop_propagation();
                    n.cross_to_edge(false, w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::MoveRight, w, cx| {
                if n.live.is_some_and(|l| l.cell.is_none()) {
                    let b = n.block.read(cx);
                    let end = b.value().len();
                    if b.selected_range() == (end..end) {
                        cx.stop_propagation();
                        n.cross_to_edge(true, w, cx);
                    }
                }
            }))
            .capture_key_down(cx.listener(|n, e: &KeyDownEvent, w, cx| {
                let m = &e.keystroke.modifiers;
                if n.live.is_none() || m.platform || m.control || m.shift || m.alt {
                    return;
                }
                let (range, end) = {
                    let b = n.block.read(cx);
                    (b.selected_range(), b.value().len())
                };
                match e.keystroke.key.as_str() {
                    "backspace" if range == (0..0) => {
                        cx.stop_propagation();
                        n.join_lines(false, w, cx);
                    }
                    "delete" if range == (end..end) => {
                        cx.stop_propagation();
                        n.join_lines(true, w, cx);
                    }
                    _ => {}
                }
            }))
            .child(field)
            .into_any_element()
    }
}

impl Focusable for NoteView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Estimated rows of soft-wrapped monospaced text in an editor column of `width`, gutter included.

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
        let settings = prefs::get();
        let full = settings.reader_full_width;
        if self.measured_scale != note_scale() || self.measured_full != full {
            self.measured_scale = note_scale();
            self.measured_full = full;
            self.reader_list.remeasure();
        }
        let wanted = settings.note_editor;
        if wanted != self.editor_kind {
            self.editor_kind = wanted;
            let next = match (self.mode, wanted) {
                (Mode::Read, _) => Mode::Read,
                (_, prefs::NoteEditor::Live) => Mode::Live,
                (_, prefs::NoteEditor::Code) => Mode::Source,
            };
            self.set_mode(next, window, cx);
        }
        if !std::mem::replace(&mut self.took_over, true) && state::read(cx, |l| l.buffers.get(&self.path).is_some_and(|b| b.dirty)) {
            self.schedule_autosave(cx);
        }
        let t = Theme::get(cx);
        let lib = state::lib(cx);
        let wheel = super::image_view::wheel_zoom(|factor, _, _, cx| scale_font(factor, cx));
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
        let room = if self.embedded { None } else { Some(self.fit.bar().map_or(Room::OPEN, |w| Room::exact(w - 2. * space::M))) };
        let toc_current = match (&self.toc_of, &self.parsed) {
            (Some((made, mode)), Some((_, now))) => *mode == self.mode && Rc::ptr_eq(made, now),
            _ => false,
        };
        if !toc_current
            && let Some(toc) = self.toc_entries()
        {
            self.toc_of = self.parsed.as_ref().map(|(_, p)| (p.clone(), self.mode));
            if *self.toc != toc {
                self.toc = Rc::new(toc);
            }
        }
        let header = self.header(exporting, room, &t, window, cx);
        if self.mode == Mode::Split && !missing {
            self.sync_split(&text, cx);
        }
        let undecodable = state::read(cx, |l| l.buffers.get(&self.path).is_some_and(|b| b.undecodable));
        let content: AnyElement = if undecodable {
            unavailable("file-warning", tr("Kodierung wird nicht unterstützt"), tr("Die Datei ist nicht in UTF-8 gespeichert. Sie bleibt unverändert, öffne sie in einer anderen App."), &t).into_any_element()
        } else if missing {
            let closing = self.path.clone();
            unavailable("file-question", tr("Datei nicht gefunden"), trf("„%@“ wurde gelöscht oder außerhalb dieses Ordners verschoben.", &[&self.path.file_name().unwrap_or_default().to_string_lossy()]), &t).child(div().pt(px(space::M)).child(Button::new("missing-close").small().label(tr("Tab schließen")).on_click(move |_, _, cx| close_tab(&closing, cx)))).into_any_element()
        } else {
            match self.mode {
                Mode::Read | Mode::Live => self.reader(&text, &t, window, cx),
                Mode::Split => div().size_full().flex().child(div().flex_1().min_w_0().h_full().flex().flex_col().border_r_1().border_color(t.separator).child(div().flex_1().min_h_0().child(self.editor_view(cx))).child(word_count(&text, &t))).child(div().flex_1().min_w_0().h_full().child(self.reader(&text, &t, window, cx))).into_any_element(),
                Mode::Source => div().size_full().flex().flex_col().child(div().flex_1().min_h_0().child(self.editor_view(cx))).child(word_count(&text, &t)).into_any_element(),
            }
        };
        let top = if self.embedded {
            document_bar(&self.path, header, &t, window, cx)
        } else {
            div().relative().overflow_hidden().h(px(control::SUBBAR)).flex_none().flex().items_center().justify_center().border_b_1().border_color(t.separator).child(self.fit.measure_bar()).child(header)
        };
        let hover_card = (!self.embedded).then(|| super::link_card::overlay(cx)).flatten();
        let view = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .children(hover_card)
            .track_focus(&self.focus)
            .key_context("NoteView")
            .capture_action(cx.listener(|n, _: &super::app::Undo, w, cx| {
                if n.mode == Mode::Live && !n.history.is_empty() {
                    cx.stop_propagation();
                    n.step_history(true, w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &super::app::Redo, w, cx| {
                if n.mode == Mode::Live && !n.future.is_empty() {
                    cx.stop_propagation();
                    n.step_history(false, w, cx);
                }
            }))
            .on_action(cx.listener(|n, _: &super::app::TogglePreview, w, cx| {
                let writing = if n.editor_kind == prefs::NoteEditor::Live { Mode::Live } else { Mode::Source };
                let m = if n.mode == Mode::Read { writing } else { Mode::Read };
                n.set_mode(m, w, cx)
            }))
            .on_action(cx.listener(|n, _: &super::app::SplitPreview, w, cx| {
                if n.editor_kind == prefs::NoteEditor::Code {
                    let m = if n.mode == Mode::Split { Mode::Source } else { Mode::Split };
                    n.set_mode(m, w, cx)
                }
            }))
            .on_action(cx.listener(|n, _: &super::app::ExportPdf, window, cx| n.export(window, cx)))
            .on_action(cx.listener(|n, _: &super::app::Save, _, cx| n.save(cx)))
            .on_key_down(cx.listener(|n, e: &KeyDownEvent, window, cx| n.link_keys(e, window, cx)))
            .when(!matches!(self.mode, Mode::Source) && self.live.is_none(), |d| d.key_context("WalkItems"))
            .on_action(cx.listener(|n, _: &super::canvas_view::NextItem, window, cx| n.step_link(false, window, cx)))
            .on_action(cx.listener(|n, _: &super::canvas_view::PreviousItem, window, cx| n.step_link(true, window, cx)))
            .when(!self.embedded, |d| {
                d.on_action(|_: &super::app::CloseTab, window, cx| super::note_window::request_close(window, cx))
                    .on_action(|_: &super::app::CloseWindow, window, cx| super::note_window::request_close(window, cx))
            })
            .child(top)
            .child(self.conflict_notice(conflict, &t))
            .child(div().flex_1().min_h_0().child(content));
        let view = view.child(wheel);
        if self.embedded {
            view.into_any_element()
        } else {
            super::chrome::frame_with_close(self.window_title.clone(), view, super::note_window::request_close, &t)
        }
    }
}

/// ⌘ plus wheel over a note or text: the same font size the settings change.
pub fn scale_font(factor: f32, cx: &mut App) {
    thread_local! {
        static PENDING: std::cell::Cell<f32> = const { std::cell::Cell::new(0.) };
    }
    let moved = PENDING.with(|p| {
        let total = p.get() + factor.ln();
        let steps = (total / 0.12).trunc();
        p.set(total - steps * 0.12);
        steps
    });
    if moved != 0. {
        set_font_size((prefs::get().editor_font_size + moved).clamp(10., 24.), cx);
    }
}

/// The widest a reading column gets while notes are not shown in full width.
pub const READER_COLUMN: f32 = 720.;

/// Turns full-width reading on or off in every window.
pub fn set_reader_full_width(on: bool, cx: &mut App) {
    prefs::update(|p| p.reader_full_width = on);
    cx.refresh_windows();
}

/// The reading column: the whole pane, or a centered column when full width is off; `left` is its room before the text.
pub fn reader_column(full: bool, left: f32, inner: AnyElement) -> AnyElement {
    let column = div().w_full().pl(px(left)).pr(px(space::XXL)).child(inner);
    if full {
        return column.into_any_element();
    }
    div().w_full().flex().justify_center().child(column.max_w(px(READER_COLUMN))).into_any_element()
}

/// Sets the editor font size everywhere.
pub fn set_font_size(size: f32, cx: &mut App) {
    prefs::update(|p| p.editor_font_size = size);
    gpui_kit::component::Theme::global_mut(cx).mono_font_size = px(size);
    super::theme::set_note_scale(size);
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

/// A tab being moved along the tab bar or into the split view.
#[derive(Clone)]
pub struct DraggedTab {
    pub path: PathBuf,
    title: String,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        div().flex().items_center().gap(px(space::S)).px(px(space::M)).py(px(space::XS)).rounded(px(radius::INNER)).bg(t.material).text_color(t.primary).shadow_md().text_size(px(font::CALLOUT)).child(file_icon(&self.path, iconsize::M)).child(self.title.clone())
    }
}

struct TabPlan {
    docs: Vec<PathBuf>,
    titles: Vec<String>,
    dirty: Vec<bool>,
    natural: Vec<f32>,
    widths: Vec<f32>,
    active: Option<usize>,
    show_files: bool,
    list_all: bool,
}

fn is_split(current: &Path, cx: &App) -> bool {
    state::read(cx, |l| l.split_document.as_deref() == Some(current))
}

fn tab_plan(current: &Path, fit: &BarFit, window: &Window, cx: &App) -> TabPlan {
    let (docs, titles, dirty) = state::read(cx, |l| {
        let docs = l.open_documents.clone();
        let titles: Vec<String> = docs.iter().map(|d| l.note_title(d)).collect();
        let dirty: Vec<bool> = docs.iter().map(|d| l.buffers.get(d).is_some_and(|b| b.dirty)).collect();
        (docs, titles, dirty)
    });
    let active = docs.iter().position(|d| d == current);
    let natural: Vec<f32> = titles.iter().map(|t| bar_fit::tab_natural(t, window)).collect();
    let widths = fit.strip().map_or_else(|| natural.clone(), |strip| bar_fit::tab_widths(&natural, active, strip, space::XXS, space::XL));
    let roomy = fit.bar().is_none_or(|bar| bar >= tab::LIST_MIN_BAR);
    let overflowing = fit.strip().is_some_and(|strip| bar_fit::overflows(&natural, active, strip));
    let list_all = roomy && (docs.len() > 4 || overflowing);
    TabPlan { docs, titles, dirty, natural, widths, active, show_files: prefs::get().notes_explorer_hidden, list_all }
}

/// Which of the ever more compact action rows `steps` (their widths) fits next to the tabs or the title of
/// the shown document, and whether the list of all tabs takes room too. Both follow from the row's width and
/// the tabs alone, never from what was drawn last, so the row cannot flip between two layouts.
pub fn actions_fold(current: &Path, steps: &[f32], keep: usize, window: &Window, cx: &App) -> usize {
    let split = is_split(current, cx);
    let fit = bar_fit::slot(split);
    let Some(bar) = fit.bar() else { return 0 };
    if split {
        let title = state::read(cx, |l| l.note_title(current));
        let natural = bar_fit::text_width(&title, font::CALLOUT, FontWeight::NORMAL, window);
        let rest = bar - 2. * space::M - iconsize::M - bar_fit::ICON_BUTTON - 3. * space::S;
        return bar_fit::fold_room(Room { comfortable: rest - natural.min(tab::SPLIT_TITLE), tight: rest - natural.min(tab::SPLIT_TITLE_TIGHT) }, steps, keep);
    }
    let plan = tab_plan(current, &fit, window, cx);
    let natural = plan.active.map_or(0., |i| plan.natural[i]);
    let sides = plan.active.map_or(0, |a| usize::from(a > 0) + usize::from(a + 1 < plan.docs.len()));
    let extra = |on: bool| if on { bar_fit::ICON_BUTTON + space::S } else { 0. };
    let base = bar - space::S - space::M - extra(plan.show_files) - space::S;
    let settle = |list: bool| {
        let rest = base - extra(list);
        let room = Room { comfortable: rest - space::XL * sides as f32 - natural.min(tab::ACTIVE_MIN), tight: rest - natural.min(tab::MIN) };
        let fold = bar_fit::fold_room(room, steps, keep);
        (fold, rest - steps.get(fold).copied().unwrap_or(0.))
    };
    let roomy = bar >= tab::LIST_MIN_BAR;
    let list = roomy && (plan.docs.len() > 4 || bar_fit::overflows(&plan.natural, plan.active, settle(false).1));
    fit.decide(list);
    settle(list).0
}

/// One row: open tabs on the left, the shown document's actions on the right.
/// Tabs shrink and then scroll; the row never draws past its own edges.
pub fn document_bar(current: &Path, actions: AnyElement, t: &Theme, window: &Window, cx: &App) -> Div {
    let split = is_split(current, cx);
    let fit = bar_fit::slot(split);
    if split {
        let title = state::read(cx, |l| l.note_title(current));
        return div()
            .relative()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::M))
            .flex_none().h(px(control::SUBBAR))
            .border_b_1()
            .border_color(t.separator)
            .child(fit.measure_bar())
            .child(div().flex_none().child(file_icon(current, iconsize::M)))
            .child(div().id("split-title").flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis_middle().text_size(px(font::CALLOUT)).tooltip(tip(title.clone())).child(title))
            .child(div().flex_none().child(actions))
            .child(Button::new("split-close").ghost().small().icon(menu_icon("x")).tooltip(tr("Geteilte Ansicht schließen")).on_click(|_, _, cx| act(cx, |l| l.set_split(None))));
    }
    let preview = state::read(cx, |l| l.preview_document.clone());
    let TabPlan { docs, titles, dirty, natural, widths, active: shown_index, show_files, list_all, .. } = tab_plan(current, &fit, window, cx);
    let list_all = fit.decided().unwrap_or(list_all);
    let overflow = list_all.then(|| {
        let (all, names, shown) = (docs.clone(), titles.clone(), current.to_path_buf());
        Button::new("doc-tabs-all").ghost().small().icon(menu_icon("chevron-down")).tooltip(trf("Alle %lld Tabs", &[&docs.len()])).dropdown_menu(move |mut menu, _, _| {
            for (doc, title) in all.iter().zip(&names) {
                let open = doc.clone();
                menu = menu.item(PopupMenuItem::new(title.clone()).checked(*doc == shown).on_click(move |_, _, cx| act(cx, |l| l.open_document(&open))));
            }
            menu
        })
    });
    let show_files = show_files.then(|| Button::new("explorer-show").ghost().small().icon(menu_icon("panel-left")).tooltip(tr("Dateien einblenden")).on_click(|_, _, cx| super::notes_page::set_hidden(false, cx)));
    let index = shown_index.unwrap_or(0);
    let scroll = state::tab_scroll(current, index, cx);
    let strip = fit.strip();
    let total = widths.iter().sum::<f32>() + space::XXS * widths.len().saturating_sub(1) as f32;
    let mut offset = f32::from(scroll.offset().x);
    if let (Some(strip), Some(shown)) = (strip, shown_index) {
        let wanted = if fit.needs_reveal(current) { bar_fit::reveal(&widths, space::XXS, shown, strip, offset, space::XL) } else { offset.clamp(-(total - strip).max(0.), 0.) };
        if wanted != offset {
            offset = wanted;
            scroll.set_offset(point(px(wanted), px(0.)));
        }
    }
    let mut left = 0.;
    let mut tabs = div().flex().gap(px(space::XXS)).py(px(space::S));
    for (i, doc) in docs.iter().enumerate() {
        let active = doc.as_path() == current;
        let (start, end) = (left + offset, left + offset + widths[i]);
        left += widths[i] + space::XXS;
        if !active && strip.is_some_and(|strip| end < -strip || start > 2. * strip) {
            tabs = tabs.child(div().flex_none().w(px(widths[i])));
            continue;
        }
        let (open, close, menu, middle, target) = (doc.clone(), doc.clone(), doc.clone(), doc.clone(), doc.clone());
        let accent = t.accent;
        let is_dirty = dirty[i];
        let hover = t.ink.opacity(0.05);
        let rel = state::read(cx, |l| l.relative_path(doc));
        let is_preview = preview.as_ref() == Some(doc);
        let squeezed = widths[i] + 0.5 < natural[i] && !is_dirty;
        let close_button = div()
            .id(SharedString::from(format!("tab-close-{}", doc.display())))
            .flex_none()
            .size(px(iconsize::M))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .soft(t.ink.opacity(0.1), radius::FULL)
            .child(if is_dirty {
                div().relative().size(px(iconsize::M)).flex().items_center().justify_center().child(div().size(px(space::S)).rounded_full().bg(t.secondary).group_hover("doc-tab", Styled::invisible)).child(div().absolute().invisible().group_hover("doc-tab", Styled::visible).child(icon("x", iconsize::XS, t.secondary))).into_any_element()
            } else {
                icon("x", iconsize::XS, t.secondary).into_any_element()
            })
            .tooltip(tip(if is_dirty { tr("Ungesicherte Änderungen – Tab schließen") } else { tr("Tab schließen") }))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                close_tab(&close, cx);
            });
        let under = t.content.blend(t.ink.opacity(if active { 0.1 } else { 0.05 }));
        tabs = tabs.child(
            div()
                .id(SharedString::from(format!("tab-{}", doc.display())))
                .group("doc-tab")
                .relative()
                .flex()
                .flex_none()
                .w(px(widths[i]))
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
                .on_click(move |e, _, cx| {
                    act(cx, |l| {
                        l.open_document(&open);
                        if e.click_count() >= 2 {
                            l.pin_document(&open);
                        }
                    })
                })
                .on_mouse_down(MouseButton::Middle, move |_, _, cx| close_tab(&middle, cx))
                .on_drag(DraggedTab { path: doc.clone(), title: titles[i].clone() }, |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(accent.opacity(0.15)))
                .on_drop(move |dragged: &DraggedTab, _, cx| {
                    let path = dragged.path.clone();
                    act(cx, |l| l.move_document(&path, &target));
                })
                .child(div().flex_none().child(file_icon(doc, iconsize::M)))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis_middle().when(is_preview, Styled::italic).text_size(px(font::CALLOUT)).text_color(if active { t.primary } else { t.secondary }).child(titles[i].clone()))
                .child(if squeezed {
                    div().absolute().top_0().bottom_0().right_0().pl(px(space::XS)).pr(px(space::S)).rounded_r(px(radius::INNER)).flex().items_center().bg(under).when(!active, |d| d.invisible().group_hover("doc-tab", Styled::visible)).child(close_button).into_any_element()
                } else {
                    close_button.when(!active && !is_dirty, |d| d.invisible().group_hover("doc-tab", Styled::visible)).into_any_element()
                })
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
    let fade = |left: bool| div().absolute().top_0().bottom_0().w(px(space::XL)).when(left, Styled::left_0).when(!left, Styled::right_0).bg(linear_gradient(if left { 270. } else { 90. }, linear_color_stop(t.content.opacity(0.), 0.), linear_color_stop(t.content, 1.)));
    let fades = strip.filter(|strip| shown_index.is_none_or(|i| widths[i] + 2. * space::XL <= *strip)).map(|strip| [(offset < -0.5).then(|| fade(true)), (total + offset > strip + 0.5).then(|| fade(false))]);
    div()
        .relative()
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(space::S))
        .pl(px(space::S))
        .pr(px(space::M))
        .flex_none().h(px(control::SUBBAR))
        .border_b_1()
        .border_color(t.separator)
        .child(fit.measure_bar())
        .children(show_files)
        .child(div().flex_1().min_w_0().relative().child(tabs.id("doc-tabs").w_full().overflow_x_scroll().track_scroll(&scroll)).children(fades.into_iter().flatten().flatten()).child(fit.measure_strip()))
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
        } else {
            self.preview.update(cx, |p, cx| p.take_focus(window, cx));
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
        let (p1, p2, p3, p4) = (self.path.clone(), self.path.clone(), self.path.clone(), self.path.clone());
        let open_label = self.app.as_ref().map_or_else(|| tr("Mit Standard-App öffnen"), |a| trf("In %@ öffnen", &[a]));
        let actions = Button::new("viewer-more").ghost().small().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu(move |m, _, cx| {
            let (a, b, c, o, n) = (p1.clone(), p2.clone(), p3.clone(), p4.clone(), p4.clone());
            let named = super::convention_rename::available(&n, cx);
            m.item(PopupMenuItem::new(open_label.clone()).icon(menu_icon("square-arrow-out-up-right")).on_click(move |_, _, _| library::open_path(&o)))
                .item(PopupMenuItem::new(super::platform::reveal_label()).icon(menu_icon("folder")).on_click(move |_, _, _| library::reveal(&a)))
                .item(PopupMenuItem::new(tr("Vorschau-Fenster (Leertaste)")).icon(menu_icon("eye")).on_click(move |_, _, cx| super::preview::show(&b, cx)))
                .when(named, |m| m.item(PopupMenuItem::new(tr("Nach Konvention benennen…")).icon(menu_icon("wand-sparkles")).on_click(move |_, window, cx| super::convention_rename::open(n.clone(), window, cx))))
                .separator()
                .item(PopupMenuItem::new(tr("Tab schließen")).icon(menu_icon("x")).on_click(move |_, _, cx| close_tab(&c, cx)))
        });
        let body: AnyElement = if !self.path.exists() {
            unavailable("file-question", tr("Datei nicht gefunden"), self.path.file_name().unwrap_or_default().to_string_lossy().to_string(), &t).into_any_element()
        } else if let Some(pdf) = &self.pdf {
            pdf.clone().into_any_element()
        } else {
            self.preview.clone().into_any_element()
        };
        div().size_full().flex().flex_col().child(document_bar(&self.path, actions.into_any_element(), &t, window, cx)).child(div().flex_1().min_h_0().child(body))
    }
}

/// Saves or asks about every open note, then quits.
pub fn prepare_to_quit(cx: &mut App) {
    state::flush_pending(cx);
    act(cx, |l| {
        if let Some(path) = l.active_document.clone() {
            l.follow_heading(&path);
        }
    });
    let all = state::read(cx, |l| l.buffers.keys().cloned().collect());
    resolve_unsaved(all, cx, |cx| super::sync::quit_after(|cx| cx.quit(), cx));
}

/// Says which Obsidian plugin a note belongs to.
fn plugin_notice(name: &str, path: &Path, t: &Theme) -> AnyElement {
    let path = path.to_path_buf();
    div().pb(px(space::L)).child(notice("puzzle", t.blue, trf("Diese Notiz gehört zum Obsidian-Plugin %@", &[&name.to_string()]), Some(tr("Hier siehst du nur ihren Text. Darstellen kann sie Obsidian.")), Button::new("plugin-open").small().label(tr("In Obsidian öffnen")).on_click(move |_, _, cx| super::vault_tools::open_in_obsidian(&path, cx)), t)).into_any_element()
}

/// The right-click menu of a writing field.
fn writing_menu(field: Editor) -> Editor {
    use gpui_kit::component::input::{Copy, Cut, Paste};
    field.context_menu(|menu, _, _| {
        menu.menu(tr("Ausschneiden"), Box::new(Cut))
            .menu(tr("Kopieren"), Box::new(Copy))
            .menu(tr("Einfügen"), Box::new(Paste))
            .separator()
            .menu(tr("Fett"), Box::new(Bold))
            .menu(tr("Kursiv"), Box::new(Italic))
            .menu(tr("Durchgestrichen"), Box::new(Strike))
            .menu(tr("Markiert"), Box::new(Mark))
            .menu(tr("Code"), Box::new(InlineCode))
            .menu(tr("Formel"), Box::new(InlineMath))
            .separator()
            .menu(tr("Überschrift 1"), Box::new(Heading1))
            .menu(tr("Überschrift 2"), Box::new(Heading2))
            .menu(tr("Überschrift 3"), Box::new(Heading3))
            .menu(tr("Zitat"), Box::new(Quote))
            .menu(tr("Liste"), Box::new(BulletList))
            .menu(tr("Nummerierte Liste"), Box::new(NumberedList))
            .menu(tr("Aufgabe"), Box::new(ToggleTask))
            .menu(tr("Link"), Box::new(InsertLink))
            .separator()
            .menu(tr("Karte aus Auswahl…"), Box::new(CardFromSelection))
    })
}
