use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
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

use super::find_bar::{CloseFind, FindEvent, FindInNote, FindState, ReplaceInNote};
use super::markdown_view::{self, ParsedNote};
use super::spell_card::{self, AddWordAtCaret, AddWordUnderPointer, SpellHover, SpellState};
use super::browser::DraggedFile;
use super::pdf_view::{DraggedPages, PdfView};
use super::preview::FilePreview;
use super::state::{self, act};
use super::motion::Soft;
use super::bar_fit::{self, BarFit, Room};
use crate::core::vault::index;
use super::theme::{Theme, control, font, hover, note_size, note_scale, pages, radius, space, iconsize, tab, tint};
use super::widgets::*;
use crate::core::live_text::{self, LiveStyle, Segment, SegmentKind};
use crate::core::edit_ops::{self, Edit};
use crate::core::i18n::{tr, trf};
use crate::core::library;
use crate::core::markdown;
use crate::core::note_buffer::{NoteBuffer, Snapshot};
use crate::core::naming::extension;
use crate::core::prefs::{self, NoteSaveMode};
use crate::core::search_index;
use crate::core::shortcuts::ShortcutAction;

actions!(
    schrumbi_editor,
    [
        Bold, Italic, InsertLink, InlineCode, ToggleTask, ToggleComment, MoveLineUp, MoveLineDown, CopyLineUp, CopyLineDown, DeleteLine, SelectLine, IndentLines, OutdentLines, ZoomIn, ZoomOut, ZoomReset,
        ListNewline, ListTab, ListBacktab, ExtractSelection, CardFromSelection, AskAiSelection, CardsAiSelection, ExplainAiSelection, PlainNewline, Strike, Mark, InlineMath, Quote, BulletList, NumberedList, Heading1, Heading2, Heading3, ToggleWrap
    ]
);

const NOTE_PLACES: usize = 500;
fn editor_viewport(s: &EditorState) -> Option<Bounds<Pixels>> {
    let mut view = s.text_bounds()?;
    view.origin.y -= s.scroll_offset().y;
    Some(view)
}

/// Where the note editors of the main window are on screen, so a PDF dragged there is embedded instead of opened beside.
#[derive(Default)]
pub struct NotePanels(std::collections::HashMap<EntityId, Bounds<Pixels>>, bool);

impl NotePanels {
    pub fn contains(&self, at: Point<Pixels>) -> bool {
        self.0.values().any(|b| b.contains(&at))
    }

    /// Whether the document area shows its own drop zone, so the notes draw no line of their own.
    pub fn set_held(&mut self, held: bool) {
        self.1 = held;
    }

    fn held(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|p| p.1)
    }

    /// Forgets every editor; the ones on screen report again while they are drawn.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

impl Global for NotePanels {}

/// Whether a dragged file or tab is a PDF that a note embeds when it is dropped there.
pub fn is_pdf_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

const FIND_IN_BACKGROUND_FROM: usize = 200_000;
const PARSE_IN_BACKGROUND_FROM: usize = 100_000;
const REPARSE_DELAY: Duration = Duration::from_millis(150);
const NOTICE_AFTER: Duration = Duration::from_millis(300);
const RICH_EDITOR_UNTIL: usize = 200_000;
const WRAP_UNTIL: usize = 20_000;
const WRAP_MAX: usize = 35_000;
const LONG_LINE_WRAP: usize = 50_000;
const SPELL_WINDOW: usize = 40_000;

/// Whether a note of this length can be wrapped at all.
fn can_wrap(text: &str) -> bool {
    wraps_note(text, Some(true))
}

/// Whether one line is so long that the editor cannot show its end without wrapping.
fn has_long_line(text: &str) -> bool {
    text.split('\n').any(|line| line.len() > LONG_LINE_WRAP)
}

/// `wraps_lines` for a note whose text is known: a very long line wraps unless the user chose otherwise.
fn wraps_note(text: &str, chosen: Option<bool>) -> bool {
    chosen != Some(false) && has_long_line(text) || wraps_lines(text.len(), chosen)
}

/// Whether the source editor wraps lines: the user's choice for this note while wrapping it takes under a frame, else only while the note is short.
fn wraps_lines(len: usize, chosen: Option<bool>) -> bool {
    chosen.map_or(len <= WRAP_UNTIL, |wrap| wrap && len <= WRAP_MAX)
}

/// The longest line Live opens for writing; a longer one opens in the source view, where it stays fast.
const LIVE_LINE_MAX: usize = 250_000;
/// The room kept free below the last line of a note, which a click opens the last line in Live.
const NOTE_END_ROOM: f32 = 160.;
const SPLIT_HOLD: Duration = Duration::from_millis(300);
const FIND_MARKS_AROUND: usize = 1000;
const CHANGE_MARKS: usize = 200;

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
const GUTTER_WIDTH: f32 = 54.;
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

/// The language the source editor highlights a note of this size in; a very long one stays plain, as a syntax tree of it costs hundreds of megabytes.
fn editor_language(len: usize) -> &'static str {
    if len <= RICH_EDITOR_UNTIL { "markdown" } else { "text" }
}

/// Whether a note of this length is parsed away from the interface in this mode, so the last parse keeps showing meanwhile.
fn parses_in_background(mode: Mode, len: usize) -> bool {
    matches!(mode, Mode::Read | Mode::Split) && len >= PARSE_IN_BACKGROUND_FROM
}

/// Whether the first parse of a note is made away from the interface; Live keeps later ones at hand because its blocks follow every edit.
fn first_parse_in_background(mode: Mode, len: usize, first: bool) -> bool {
    first && mode == Mode::Live && len >= PARSE_IN_BACKGROUND_FROM
}

/// How many rows the source editor is about as tall as; a line is one row when it does not wrap, which is `per_row` infinite.
fn estimated_rows(text: &str, per_row: f32) -> f32 {
    text.lines().map(|l| (l.chars().count() as f32 / per_row).ceil().max(1.)).sum()
}

/// The lines of a long note around the caret that are checked for spelling; a short note is checked whole.
fn spell_window(text: &str, caret: usize) -> std::ops::Range<usize> {
    if text.len() <= 2 * SPELL_WINDOW {
        return 0..text.len();
    }
    let boundary = |at: usize| (0..=at.min(text.len())).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
    let (from, to) = (boundary(caret.saturating_sub(SPELL_WINDOW)), boundary(caret + SPELL_WINDOW));
    let from = text[..from].rfind('\n').map_or(0, |i| i + 1);
    let to = text[to..].find('\n').map_or(text.len(), |i| to + i);
    let is_fence = |line: &str| line.trim_start().starts_with("```");
    if text[..from].split_inclusive('\n').filter(|line| is_fence(line)).count() % 2 == 1 {
        let mut at = from;
        for line in text[from..to].split_inclusive('\n') {
            at += line.len();
            if is_fence(line) {
                return at..to;
            }
        }
        return to..to;
    }
    from..to
}

/// How far a nested list line moves in.
const LIVE_INDENT: f32 = 24.;

/// A Markdown note: read, edit or both side by side. Saving follows Settings → Notizen.
pub struct NoteView {
    /// A nearly empty note opens ready to continue writing below what is there.
    caret_to_end: bool,
    reader_list: ListState,
    /// The height an item not drawn yet counts with, and the list width it was last given at; the list forgets it when its width changes.
    row_estimate: (f32, Option<Pixels>),
    /// The text scale the list measured its items with.
    measured_scale: f32,
    measured_full: bool,
    /// The formula or diagram being written, drawn while it is written.
    live_preview: Option<(String, Rc<ParsedNote>)>,
    /// A block the reader should bring into view once it knows the block.
    reveal_line: Option<usize>,
    /// Which match of that line is to be shown, when several rows share it.
    reveal_nth: usize,
    item_hashes: Vec<u64>,
    split_sync: (f32, f32),
    split_hold: Option<std::time::Instant>,
    split_lead: Option<bool>,
    reparse: Option<Task<()>>,
    /// Edits wait for this to end before the reader parses the note again.
    parse_hold: bool,
    /// Nothing of this note was drawn yet; a long note in Live is then parsed away from the interface once.
    first_parse: bool,
    /// The parse running away from the interface; the next starts when it ends.
    parse_job: Option<Task<()>>,
    /// The parse has taken long enough to say so.
    parse_slow: bool,
    parse_notice: Option<Task<()>>,
    /// Words of the note, with the text they were counted in.
    words: (usize, Option<(usize, usize)>),
    words_task: Option<((usize, usize), Task<()>)>,
    /// Rows the editor needs for the text at an address and length, at a width in characters.
    split_rows: Option<((usize, usize), u32, f32)>,
    /// What the live view draws, one entry per list item between start and end.
    segments: Rc<Vec<Segment>>,
    source_lines: Rc<Vec<String>>,
    /// Reference and footnote definitions, which a line drawn on its own still needs.
    definitions: Rc<String>,
    /// Lines already parsed for drawing.
    fragments: Rc<std::cell::RefCell<std::collections::HashMap<String, Rc<ParsedNote>>>>,
    parsed_text: Option<Arc<str>>,
    marks: Option<TextDecorationCollection>,
    /// A source line to bring back to the top after switching views.
    keep_line: Option<usize>,
    /// The source view is only open for a selection over several lines.
    source_detour: bool,
    /// The source view is open because the line was too long for Live.
    long_line: bool,
    /// The source editor of this view is plain because the note is long, and whether it wraps lines.
    editor_plain: bool,
    editor_wrap: bool,
    wrap_override: Option<bool>,
    wrap_notice: Option<(bool, Task<()>)>,
    /// Which editor the settings asked for when this view last looked.
    editor_kind: prefs::NoteEditor,
    reader_width: Pixels,
    spell_task: Option<Task<()>>,
    spell: SpellState,
    find: FindState,
    /// Where writing last happened in this note.
    last_line: Option<usize>,
    /// Whether this view has looked for changes it inherited, as after an outside move.
    took_over: bool,
    /// What is being written in the live view, in a field of its own.
    block: Entity<EditorState>,
    /// The caret and length of the open field when the view last looked, and whether the caret still has to be brought into view.
    caret_seen: (usize, usize),
    follow_caret: bool,
    live: Option<LiveBlock>,
    /// Whole-note states to go back to across blocks; the field only knows its own block.
    history: Vec<(String, usize)>,
    live_left: Option<Arc<str>>,
    future: Vec<(String, usize)>,
    text_undone: Option<std::time::Instant>,
    app_undone: Option<ropey::Rope>,
    last_snapshot: Option<std::time::Instant>,
    found: Option<markdown_view::Found>,
    change_marks: Option<TextDecorationCollection>,
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
    external: u64,
    /// The buffer's version the editor shows, so a change made in another view is noticed.
    version: u64,
    syncing: bool,
    autosave: Option<Task<()>>,
    save_problem: Option<String>,
    save_pending: Option<Task<()>>,
    save_flash: Option<Task<()>>,
    folder_wait: Option<Task<()>>,
    retry_focus: FocusHandle,
    retry_held: bool,
    /// Shown inside the main window with tabs, or in its own window.
    pub embedded: bool,
    focus: FocusHandle,
    needs_focus: bool,
    window_title: String,
    /// The source line before which a dragged PDF or page lands, and where the writing field draws its line: left and top inside the field's wrapper.
    pdf_drop: Option<usize>,
    pdf_drop_bar: Option<(f32, f32)>,
    _subs: Vec<Subscription>,
}

impl NoteView {
    pub fn new(path: PathBuf, embedded: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = state::lib(cx).update(cx, |l, _| l.buffer(&path).text.clone());
        let wrap_override = prefs::with(|p| p.note_wraps.get(path.to_string_lossy().as_ref()).copied());
        let wrap = wraps_note(&text, wrap_override);
        let editor = cx.new(|cx| EditorState::new(window, cx).language(editor_language(text.len())).soft_wrap(wrap).line_number(true).default_value(text.clone()));
        editor.update(cx, |s, _| s.lsp_mut().completion_provider = Some(super::completion::provider(path.clone())));
        let block = cx.new(|cx| EditorState::new(window, cx).soft_wrap(true).line_number(false).folding(false).smart_indent(false).scroll_beyond_last_line(Some(0)));
        block.update(cx, |s, _| s.lsp_mut().completion_provider = Some(super::completion::provider(path.clone())));
        let lib = state::lib(cx);
        let subs = vec![
            cx.subscribe_in(&editor, window, |this, e, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change if !this.syncing => {
                    let text = e.read(cx).text().to_string();
                    let path = this.path.clone();
                    let seen = this.version;
                    if state::read(cx, |l| l.buffers.get(&path).is_some_and(|b| b.shows(seen, &text))) {
                        return;
                    }
                    let written = state::lib(cx).update(cx, |l, cx| {
                        let written = l.buffers.get_mut(&path).map(|b| (b.edited_on(seen, text), b.version()));
                        cx.notify();
                        written
                    });
                    match written {
                        Some((true, version)) => this.version = version,
                        Some((false, _)) => {
                            this.follow_buffer(window, cx);
                            return;
                        }
                        None => {}
                    }
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
            cx.observe(&block, |this, block, cx| {
                if this.live.as_ref().is_some_and(|l| l.cell.is_some()) {
                    cx.notify();
                }
                let seen = (block.read(cx).cursor(), block.read(cx).text().len());
                if this.live.is_some() && seen != this.caret_seen {
                    this.caret_seen = seen;
                    this.follow_caret = true;
                    cx.notify();
                }
            }),
            cx.observe_in(&lib, window, move |this, lib, window, cx| {
                if lib.read(cx).active_document.as_ref() == Some(&this.path) && lib.update(cx, |l, _| std::mem::take(&mut l.toggle_mode)) {
                    let writing = if this.editor_kind == prefs::NoteEditor::Live { Mode::Live } else { Mode::Source };
                    let next = if this.mode == Mode::Read { writing } else { Mode::Read };
                    this.set_mode(next, window, cx);
                }
                if lib.read(cx).active_document.as_ref() != Some(&this.path) && this.place_restored {
                    this.remember_place();
                }
                if this.save_problem.is_some() && lib.read(cx).buffers.get(&this.path).is_some_and(|b| !b.dirty) {
                    this.set_problem(None, cx);
                    this.save_flash = None;
                }
                this.follow_buffer(window, cx);
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
        let (revision, version, external) = state::lib(cx).update(cx, |l, _| {
            let b = l.buffer(&path);
            (b.revision, b.version(), b.external)
        });
        let reader_list = ListState::new(0, ListAlignment::Top, px(1000.));
        let list_for_focus = reader_list.clone();
        let saved_folds = prefs::get().note_folds.get(&path.to_string_lossy().to_string()).cloned().unwrap_or_default().into_iter().collect();
        let own_path = dunce::canonicalize(&path).unwrap_or_else(|_| path.clone());
        Self { editor_plain: text.len() > RICH_EDITOR_UNTIL, editor_wrap: wrap, wrap_override, wrap_notice: None, path, caret_to_end: mode == Mode::Source && text.trim().chars().count() < 40, mode, block: block.clone(), caret_seen: (0, 0), follow_caret: false, live: None, history: Vec::new(), live_left: None, future: Vec::new(), text_undone: None, app_undone: None, last_snapshot: None, editor, reader_list, row_estimate: (0., None), measured_scale: note_scale(), measured_full: prefs::get().reader_full_width, live_preview: None, reveal_line: None, reveal_nth: 0, item_hashes: Vec::new(), split_sync: (0., 0.), split_hold: None, split_lead: None, reparse: None, parse_hold: false, first_parse: true, parse_job: None, parse_slow: false, parse_notice: None, words: (0, None), words_task: None, split_rows: None, segments: Rc::default(), source_lines: Rc::default(), definitions: Rc::default(), fragments: Rc::default(), parsed_text: None, marks: None, keep_line: None, source_detour: false, long_line: false, editor_kind: prefs::get().note_editor, reader_width: px(720.), spell_task: None, spell: SpellState::default(), find: FindState::default(), last_line: None, took_over: false, found: None, change_marks: None, link_focus: markdown_view::LinkFocus { index: None, note: Some(own_path), list: Some(list_for_focus), text_click: None, edit_item: None, folds: Rc::new(std::cell::RefCell::new(saved_folds)), find: None }, place_restored: false, parsed: None, revision, version, external, syncing: false, autosave: None, save_problem: None, save_pending: None, save_flash: None, folder_wait: None, retry_focus: cx.focus_handle(), retry_held: false, embedded, focus: cx.focus_handle(), needs_focus: !embedded, window_title: String::new(), pdf_drop: None, pdf_drop_bar: None, fit: BarFit::default(), toc: Rc::default(), toc_of: None, _subs: subs }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Selects a search match in the editor and scrolls the reader to its block.
    fn reveal(&mut self, term: &str, nth: usize, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = search_index::occurrences(text, term).into_iter().nth(nth) else { return };
        self.editor.update(cx, |s, cx| s.set_selected_range(range.clone(), cx));
        let token = self.found.as_ref().map_or(0, |f| f.token + 1);
        let block = markdown_view::block_at(text, range.start);
        let line = text.get(..range.start).unwrap_or_default().matches('\n').count();
        self.reveal_line = Some(line);
        if self.mode == Mode::Split {
            self.split_hold = Some(std::time::Instant::now());
        }
        let still = cx.reduce_motion();
        self.found = Some(markdown_view::Found::one(block, line, token, still));
        if still {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(markdown_view::FOUND_HOLD).await;
                let _ = this.update(cx, |n, cx| {
                    if n.found.as_ref().is_some_and(|f| f.token == token) {
                        n.found = None;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    /// Shows what the assistant changed: the first changed line comes into view and every changed line is marked for a moment.
    fn show_changes(&mut self, lines: &[(usize, usize)], text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
        let offset = |line: usize| starts.get(line).copied().unwrap_or(text.len());
        let ranges: Vec<std::ops::Range<usize>> = lines.iter().take(CHANGE_MARKS).map(|(from, to)| offset(*from)..offset(to + 1).saturating_sub(1).max(offset(*from))).filter(|r| r.start < text.len()).collect();
        let Some(first) = ranges.first().cloned() else { return };
        self.editor.update(cx, |s, cx| s.set_selected_range(first.start..first.start, cx));
        let token = self.found.as_ref().map_or(0, |f| f.token + 1);
        let line_of = |offset: usize| text.get(..offset).unwrap_or_default().matches('\n').count();
        let first_line = line_of(first.start);
        self.reveal_line = Some(first_line);
        if self.mode == Mode::Split {
            self.split_hold = Some(std::time::Instant::now());
        }
        let still = cx.reduce_motion();
        let mut blocks = Vec::new();
        let mut marked = Vec::new();
        for r in &ranges {
            for line in line_of(r.start)..=line_of(r.end) {
                marked.push(line);
                let at = offset(line);
                let block = markdown_view::block_at(text, at);
                if !blocks.contains(&block) {
                    blocks.push(block);
                }
            }
        }
        self.found = Some(markdown_view::Found { block: markdown_view::block_at(text, first.start), line: first_line, token, still, more: Rc::from(blocks), lines: Rc::from(marked) });
        let t = Theme::get(cx);
        let fill = HighlightStyle { background_color: Some(t.accent.opacity(0.22)), ..Default::default() };
        let decorations: Vec<TextDecoration> = ranges.iter().filter(|r| r.end > r.start).map(|r| TextDecoration::new(r.clone(), fill)).collect();
        match &self.change_marks {
            Some(known) => known.set(decorations, cx),
            None if decorations.is_empty() => {}
            None => self.change_marks = Some(self.editor.update(cx, |e, cx| e.create_decorations_collection(decorations, cx))),
        }
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(markdown_view::FOUND_HOLD).await;
            let _ = this.update(cx, |n, cx| {
                if n.found.as_ref().is_some_and(|f| f.token == token) {
                    if let Some(marks) = &n.change_marks {
                        marks.set(Vec::new(), cx);
                    }
                    if still {
                        n.found = None;
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn find_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<super::find_bar::FindBar> {
        if let Some(bar) = &self.find.bar {
            return bar.clone();
        }
        let bar = cx.new(|cx| super::find_bar::FindBar::new(window, cx));
        self._subs.push(cx.subscribe_in(&bar, window, |this, _, e: &FindEvent, window, cx| this.on_find(e, window, cx)));
        self.find.bar = Some(bar.clone());
        bar
    }

    /// Opens the find bar, searching for the selected text if there is a short one, and puts the caret in it.
    fn open_find(&mut self, replacing: bool, window: &mut Window, cx: &mut Context<Self>) {
        let bar = self.find_bar(window, cx);
        let seed = matches!(self.mode, Mode::Source | Mode::Split).then(|| self.editor.read(cx).selected_text().to_string()).filter(|s| !s.trim().is_empty() && !s.contains('\n') && s.chars().count() <= 100);
        if !self.find.open {
            self.find.open = true;
            self.find.anchor = matches!(self.mode, Mode::Source | Mode::Split).then(|| self.editor.read(cx).selected_range().start);
        }
        let can_replace = self.mode != Mode::Read;
        let (count, current) = (self.find.matches.len(), self.find.current);
        bar.update(cx, |b, cx| {
            b.show(count, current, can_replace, cx);
            b.open(seed.clone(), replacing, window, cx);
        });
        if seed.is_some() {
            self.find.signature = None;
        }
        let text = self.current_text(cx);
        self.sync_find(&text, true, true, window, cx);
        cx.notify();
    }

    fn focus_writing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Mode::Live && self.live.is_some() {
            self.block.update(cx, |b, cx| b.focus(window, cx));
        } else if matches!(self.mode, Mode::Source | Mode::Split) {
            self.editor.update(cx, |s, cx| s.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find = FindState { bar: self.find.bar.take(), marks: self.find.marks.take(), ..FindState::default() };
        self.paint_find("", cx);
        if matches!(self.mode, Mode::Source | Mode::Split) {
            self.editor.update(cx, |s, cx| s.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn on_find(&mut self, event: &FindEvent, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.current_text(cx);
        match event {
            FindEvent::Query => {
                self.find.signature = None;
                self.sync_find(&text, true, true, window, cx);
            }
            FindEvent::Step(forward) => {
                if self.find.pending.is_some() {
                    return;
                }
                self.sync_find(&text, false, false, window, cx);
                self.find.current = crate::core::find::step(self.find.current, self.find.matches.len(), *forward);
                self.paint_find(&text, cx);
                self.reveal_find(&text, cx);
                let (count, current, can_replace) = (self.find.matches.len(), self.find.current, self.mode != Mode::Read);
                if let Some(bar) = self.find.bar.clone() {
                    bar.update(cx, |b, cx| b.show(count, current, can_replace, cx));
                }
            }
            FindEvent::Replace => self.replace_found(false, window, cx),
            FindEvent::ReplaceAll => self.replace_found(true, window, cx),
            FindEvent::Close => self.close_find(window, cx),
        }
    }

    /// Finds the matches again when the text or the query changed; `reveal` also brings the current one into view.
    fn sync_find(&mut self, text: &str, reveal: bool, later: bool, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(bar) = self.find.bar.clone().filter(|_| self.find.open) else { return };
        let (query, exact) = bar.read_with(cx, |b, cx| (b.text(cx), b.exact()));
        let mode = std::mem::discriminant(&self.mode);
        let reveal = reveal || self.find.mode.is_some_and(|seen| seen != mode);
        self.find.mode = Some(mode);
        let signature = (self.fingerprint_of(text, cx), query, exact);
        if self.find.signature.as_ref() == Some(&signature) {
            self.find.pending = None;
            self.find.task = None;
        }
        if later && text.len() > FIND_IN_BACKGROUND_FROM && self.find.signature.as_ref() != Some(&signature) {
            if self.find.pending.as_ref() != Some(&signature) || reveal {
                self.find_in_background(text, signature, reveal, cx);
            }
            return;
        }
        if self.find.signature.as_ref() != Some(&signature) || reveal {
            self.find.pending = None;
            self.find.task = None;
            let anchor = self.find.anchor.take().or_else(|| self.find.matches.get(self.find.current).map(|m| m.start)).unwrap_or(0);
            let found = crate::core::find::matches(text, &signature.1, exact);
            self.find.current = crate::core::find::first_from(&found, anchor).unwrap_or(0);
            self.find.matches = found;
            self.find.signature = Some(signature);
            self.paint_find(text, cx);
            if reveal {
                self.reveal_find(text, cx);
            }
        }
        let (count, current, can_replace) = (self.find.matches.len(), self.find.current, self.mode != Mode::Read);
        bar.update(cx, |b, cx| b.show(count, current, can_replace, cx));
    }

    fn find_in_background(&mut self, text: &str, signature: (u64, String, bool), reveal: bool, cx: &mut Context<Self>) {
        let anchor = self.find.anchor.or_else(|| self.find.matches.get(self.find.current).map(|m| m.start)).unwrap_or(0);
        self.find.pending = Some(signature.clone());
        if let Some(bar) = self.find.bar.clone() {
            bar.update(cx, |b, cx| b.searching(cx));
        }
        let owned: Arc<str> = self.shared_snapshot(text, cx).map_or_else(|| Arc::from(text), |s| s.text.clone());
        let (query, exact) = (signature.1.clone(), signature.2);
        self.find.task = Some(cx.spawn(async move |this, cx| {
            let (found, owned) = cx
                .background_spawn(async move {
                    let found = crate::core::find::matches(&owned, &query, exact);
                    (found, owned)
                })
                .await;
            let _ = this.update(cx, |n, cx| {
                if n.find.pending.as_ref() != Some(&signature) {
                    return;
                }
                n.find.pending = None;
                n.find.anchor = None;
                n.find.current = crate::core::find::first_from(&found, anchor).unwrap_or(0);
                n.find.matches = found;
                n.find.signature = Some(signature);
                n.paint_find(&owned, cx);
                if reveal {
                    n.reveal_find(&owned, cx);
                }
                let (count, current, can_replace) = (n.find.matches.len(), n.find.current, n.mode != Mode::Read);
                if let Some(bar) = n.find.bar.clone() {
                    bar.update(cx, |b, cx| b.show(count, current, can_replace, cx));
                }
                cx.notify();
            });
        }));
    }

    /// Marks the matches in the editor and in the reader.
    fn paint_find(&mut self, text: &str, cx: &mut Context<Self>) {
        let t = Theme::get(cx);
        let current = self.find.current;
        let in_editor = matches!(self.mode, Mode::Source | Mode::Split);
        let decorations: Vec<TextDecoration> = self.find.matches.iter().enumerate().skip(current.saturating_sub(FIND_MARKS_AROUND)).take(2 * FIND_MARKS_AROUND).filter(|_| in_editor).map(|(i, r)| TextDecoration::new(r.clone(), HighlightStyle { background_color: Some(markdown_view::find_fill(i == current, &t)), ..Default::default() })).collect();
        match &self.find.marks {
            Some(known) => known.set(decorations, cx),
            None if decorations.is_empty() => {}
            None => {
                let made = self.editor.update(cx, |e, cx| e.create_decorations_collection(decorations, cx));
                self.find.marks = Some(made);
            }
        }
        let query = self.find.bar.as_ref().map(|b| b.read_with(cx, |b, cx| (b.text(cx), b.exact())));
        self.link_focus.find = match (query, self.find.matches.get(current)) {
            _ if self.mode == Mode::Source => None,
            (Some((query, exact)), Some(at)) if self.find.open => {
                let starts = markdown_view::block_starts(text);
                let block_of = |offset: usize| starts.partition_point(|s| *s <= offset).saturating_sub(1);
                let undrawn = markdown_view::fence_lines(text);
                let drawn = |m: &&std::ops::Range<usize>| !undrawn.iter().any(|f| f.contains(&m.start));
                let block = if drawn(&at) { block_of(at.start) } else { usize::MAX };
                let block_start = starts.get(block).copied().unwrap_or(0);
                let before = &self.find.matches[..current];
                let nth = before.iter().rev().take_while(|m| m.start >= block_start).filter(drawn).count();
                let line_start = text.get(..at.start).and_then(|t| t.rfind('\n')).map_or(0, |i| i + 1);
                let line = text.get(..at.start).unwrap_or_default().matches('\n').count();
                let line_nth = before.iter().rev().take_while(|m| m.start >= line_start).count();
                Some(Rc::new(markdown_view::FindMarks { query, exact, block, nth, line, line_nth }))
            }
            (Some((query, exact)), None) if self.find.open => Some(Rc::new(markdown_view::FindMarks { query, exact, block: usize::MAX, nth: 0, line: usize::MAX, line_nth: 0 })),
            _ => None,
        };
    }

    fn center_find(&mut self, cx: &mut Context<Self>) {
        if self.find.center == 0 {
            return;
        }
        let Some(at) = self.find.matches.get(self.find.current).cloned() else { return };
        let done = self.editor.update(cx, |s, cx| {
            let (Some(b), Some(view)) = (s.range_to_bounds(&at), editor_viewport(s)) else { return false };
            if b.top() < view.top() || b.bottom() > view.bottom() {
                return false;
            }
            let off = b.center().y - view.center().y;
            if off.abs() > view.size.height / 6. {
                let mut o = s.scroll_offset();
                o.y -= off;
                s.set_scroll_offset(o, cx);
            }
            true
        });
        self.find.center = if done { 0 } else { self.find.center - 1 };
        if !done {
            cx.notify();
        }
    }

    /// Selects the current match in the editor and brings it into view there and in the reader.
    fn reveal_find(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(at) = self.find.matches.get(self.find.current).cloned() else { return };
        if matches!(self.mode, Mode::Source | Mode::Split) {
            self.editor.update(cx, |s, cx| {
                let shown = s.range_to_bounds(&at).zip(editor_viewport(s)).is_some_and(|(b, view)| b.top() >= view.top() && b.bottom() <= view.bottom());
                s.set_selected_range(at.clone(), cx);
                if !shown {
                    self.find.center = 8;
                }
            });
        }
        if self.mode != Mode::Source {
            self.reveal_line = Some(text.get(..at.start).unwrap_or_default().matches('\n').count());
            let line_start = text.get(..at.start).and_then(|t| t.rfind('\n')).map_or(0, |i| i + 1);
            self.reveal_nth = self.find.matches[..self.find.current].iter().rev().take_while(|m| m.start >= line_start).count();
        }
        if self.mode == Mode::Split {
            self.split_hold = Some(std::time::Instant::now());
        }
        cx.notify();
    }

    fn replace_found(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bar) = self.find.bar.clone().filter(|_| self.find.open && self.mode != Mode::Read && self.find.pending.is_none()) else { return };
        let text = self.current_text(cx);
        self.find.signature = None;
        self.sync_find(&text, false, false, window, cx);
        let replacement = bar.read(cx).replacement(cx);
        let (new, next) = if all {
            (crate::core::find::replace_all(&text, &self.find.matches, &replacement), 0)
        } else {
            let Some(at) = self.find.matches.get(self.find.current).cloned() else { return };
            (crate::core::find::replace_all(&text, std::slice::from_ref(&at), &replacement), at.start + replacement.len())
        };
        if *new == *text {
            return;
        }
        self.change_text(&text, new.clone(), window, cx);
        self.find.anchor = Some(next);
        self.find.signature = None;
        self.sync_find(&new, true, true, window, cx);
    }

    /// Changes the note's text in one undoable step, in whichever editor is in use.
    fn change_text(&mut self, old: &str, new: String, window: &mut Window, cx: &mut Context<Self>) {
        let (changed, inserted) = edit_ops::minimal_change(old, &new);
        if self.mode == Mode::Live {
            self.finish_block(cx);
            let line = old.get(..changed.start).unwrap_or_default().matches('\n').count();
            self.history.push((old.to_string(), line));
            self.future.clear();
            self.last_snapshot = None;
            self.put_text(new, cx);
            self.schedule_autosave(cx);
        } else {
            let inserted = inserted.to_string();
            self.editor.update(cx, |s, cx| {
                s.set_selected_range(changed, cx);
                s.replace(inserted, window, cx);
            });
        }
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
        } else if self.mode == Mode::Live && self.live.is_some() {
            self.block.update(cx, |b, cx| b.focus(window, cx));
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
        self.long_line = false;
        let remembered = match mode {
            Mode::Read => prefs::NoteMode::Read,
            Mode::Split => prefs::NoteMode::Split,
            Mode::Live | Mode::Source => prefs::NoteMode::Write,
        };
        if prefs::get().note_mode != remembered {
            prefs::update(|p| p.note_mode = remembered);
            super::app::rebuild_menus(cx);
        }
        if mode == Mode::Read {
            self.finish_block(cx);
            let path = self.path.clone();
            act(cx, |l| l.follow_heading(&path));
            if !self.path.exists() {
                return;
            }
        }
        self.finish_block(cx);
        let keep_line = if self.mode == Mode::Source { self.editor_top_line(cx) } else { self.top_line() };
        if self.mode == Mode::Live && mode != Mode::Live {
            self.live_left = Some(self.current_text(cx));
        }
        self.drop_spell_marks(cx);
        let kept_scroll = (self.mode == Mode::Source && mode == Mode::Split).then(|| self.editor.read(cx).scroll_offset());
        let line_scroll = (mode == Mode::Source && self.mode != Mode::Source).then(|| self.editor_scroll_for(keep_line, cx)).flatten();
        if matches!(mode, Mode::Source | Mode::Split) {
            let text = state::read(cx, |l| l.buffers.get(&self.path).map(|b| b.text.clone()).unwrap_or_default());
            let (plain, language) = (text.len() > RICH_EDITOR_UNTIL, editor_language(text.len()));
            let wrap = wraps_note(&text, self.wrap_override);
            (self.editor_plain, self.editor_wrap) = (plain, wrap);
            self.syncing = true;
            self.editor.update(cx, |s, cx| {
                if s.language_name() != language {
                    s.set_highlighter(language, cx);
                }
                s.set_soft_wrap(wrap, window, cx);
                let old = s.value().to_string();
                if old != *text {
                    let (changed, inserted) = edit_ops::minimal_change(&old, &text);
                    let selected = s.selected_range();
                    let start = edit_ops::shift_offset(selected.start, &changed, inserted.len());
                    let end = edit_ops::shift_offset(selected.end, &changed, inserted.len());
                    s.set_selected_range(changed, cx);
                    s.replace(inserted.to_string(), window, cx);
                    if s.value().to_string() != *text {
                        s.set_value(text.clone(), window, cx);
                    }
                    s.set_selected_range(start.min(text.len())..end.min(text.len()), cx);
                }
                if let Some(offset) = kept_scroll {
                    s.set_scroll_offset(offset, cx);
                }
                if let Some(y) = line_scroll {
                    s.set_scroll_offset(point(px(0.), px(-y)), cx);
                }
            });
            self.syncing = false;
        }
        if mode == Mode::Live
            && self.mode != Mode::Live
            && let Some(left) = self.live_left.take()
        {
            let now = self.current_text(cx);
            if *left != *now {
                let line = left.get(..edit_ops::minimal_change(&left, &now).0.start).unwrap_or_default().matches('\n').count();
                self.history.push((left.to_string(), line));
                if self.history.len() > 500 {
                    self.history.remove(0);
                }
                self.future.clear();
                self.last_snapshot = None;
            }
        }
        self.parsed = None;
        self.parse_job = None;
        self.parse_slow = false;
        self.parse_hold = false;
        self.item_hashes.clear();
        self.reader_list.reset(0);
        self.keep_line = Some(keep_line);
        let from = std::mem::replace(&mut self.mode, mode);
        if mode == Mode::Split {
            self.split_lead = Some(from == Mode::Source);
        }
        if let Some(bar) = self.find.bar.clone().filter(|_| self.find.open) {
            self.find.signature = None;
            bar.update(cx, |b, cx| b.focus_query(window, cx));
        } else if matches!(mode, Mode::Source | Mode::Split) {
            self.editor.update(cx, |s, cx| s.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
            self.focus_lost(cx);
        }
        cx.notify();
    }

    /// Turns line wrapping of the source editor on or off for this note; the editor of a long note starts without.
    fn toggle_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.mode, Mode::Source | Mode::Split) {
            return;
        }
        let wrap = !self.editor_wrap;
        let text = self.current_text(cx);
        if wrap && !can_wrap(&text) {
            act(cx, |l| l.notify(tr("Lange Notiz: hier geht kein Zeilenumbruch. Lesen zeigt alles umgebrochen."), "text", None, None));
            return;
        }
        self.editor_wrap = wrap;
        self.wrap_override = Some(wrap).filter(|w| *w != wraps_note(&text, None));
        let (key, chosen) = (self.path.to_string_lossy().to_string(), self.wrap_override);
        prefs::update(|p| match chosen {
            Some(w) => {
                prefs::remember_capped(&mut p.note_wraps, key, w, NOTE_PLACES);
            }
            None => {
                p.note_wraps.remove(&key);
            }
        });
        self.editor.update(cx, |s, cx| s.set_soft_wrap(wrap, window, cx));
        self.wrap_notice = Some((
            wrap,
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(super::motion::WRAP_NOTICE_FOR).await;
                let _ = this.update(cx, |n, cx| {
                    n.wrap_notice = None;
                    cx.notify();
                });
            }),
        ));
        cx.notify();
    }

    /// The quiet line in the word count bar that says what a long note gives up.
    fn plain_caption(&self, len: usize, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notice = self.wrap_notice.as_ref().map(|(on, _)| if *on { tr("Zeilenumbruch an") } else { tr("Zeilenumbruch aus") });
        if self.long_line && self.mode == Mode::Source {
            if let Some(notice) = notice {
                return Some(div().id("wrap-notice").min_w_0().min_h(px(control::S)).flex().items_center().truncate().px(px(space::XS)).child(notice).into_any_element());
            }
            let me = cx.entity();
            let caption = div().id("long-line").min_w_0().min_h(px(control::S)).flex().items_center().truncate().rounded(px(radius::SMALL)).px(px(space::XS)).cursor_pointer().key_nav().soft(t.ink.opacity(hover::FLOAT), radius::SMALL).tooltip(tip(tr("Eine so lange Zeile bleibt im Quelltext schnell. Klicken geht zurück zur Live-Ansicht.")));
            return Some(
                caption
                    .on_click(move |_, window, cx| {
                        me.update(cx, |n, cx| {
                            n.source_detour = false;
                            n.set_mode(Mode::Live, window, cx);
                        })
                    })
                    .child(notice.unwrap_or_else(|| tr("Diese Zeile ist zu lang für die Live-Ansicht. Esc: zurück zur Live-Ansicht")))
                    .into_any_element(),
            );
        }
        let spelling = prefs::get().spell_check && len > 2 * SPELL_WINDOW;
        let caption = div().id("plain-note").min_w_0().truncate().rounded(px(radius::SMALL)).px(px(space::XS));
        if !self.editor_plain {
            if let Some(notice) = notice {
                return Some(div().id("wrap-notice").min_w_0().min_h(px(control::S)).flex().items_center().truncate().px(px(space::XS)).child(notice).into_any_element());
            }
            return spelling.then(|| caption.tooltip(tip(tr("Die Rechtschreibung prüft in langen Notizen nur den Text um die Schreibmarke."))).child(tr("Rechtschreibung nur um die Schreibmarke")).into_any_element());
        }
        let help = if spelling {
            tr("Lange Notiz: ohne Zeilenumbruch und Hervorhebung, Rechtschreibung nur um die Schreibmarke. Lesen zeigt alles umgebrochen.")
        } else {
            tr("Lange Notiz: ohne Zeilenumbruch und Hervorhebung. Lesen zeigt alles umgebrochen.")
        };
        Some(caption.tooltip(tip(help)).child(notice.unwrap_or_else(|| tr("Lange Notiz: ohne Zeilenumbruch und Hervorhebung"))).into_any_element())
    }

    /// The source line at the top of the reader.
    fn top_line(&self) -> usize {
        let item = self.reader_list.logical_scroll_top().item_ix.saturating_sub(1);
        if self.mode == Mode::Live {
            return self.segments.get(item).map_or(0, |s| s.line);
        }
        self.parsed.as_ref().and_then(|(_, p)| p.rows().get(item).map(|r| r.line)).unwrap_or(0)
    }

    fn editor_rows_of(&self, cx: &App) -> impl Fn(&str) -> f32 {
        let (width, size) = (f32::from(self.editor.read(cx).input_bounds().size.width), prefs::get().editor_font_size);
        let per_row = if self.editor_wrap && width > 0. { (width / (size * 0.6)).max(20.) } else { f32::INFINITY };
        move |l: &str| (l.chars().count() as f32 / per_row).ceil().max(1.)
    }

    /// The first source line in view of the source editor.
    fn editor_top_line(&self, cx: &App) -> usize {
        let (y, line) = {
            let e = self.editor.read(cx);
            (-f32::from(e.scroll_offset().y), e.line_height().map_or(0., f32::from))
        };
        if line <= 0. || y <= 0. {
            return 0;
        }
        let text = self.current_text(cx);
        let rows_of = self.editor_rows_of(cx);
        let (target, mut rows) = (y / line, 0.);
        for (i, l) in text.lines().enumerate() {
            rows += rows_of(l);
            if rows > target {
                return i;
            }
        }
        text.lines().count().saturating_sub(1)
    }

    /// How far the source editor scrolls to show `line` at the top.
    fn editor_scroll_for(&self, line: usize, cx: &App) -> Option<f32> {
        let height = f32::from(self.editor.read(cx).line_height()?);
        let text = self.current_text(cx);
        let rows_of = self.editor_rows_of(cx);
        Some(text.lines().take(line).map(rows_of).sum::<f32>() * height)
    }

    /// The list item a source line is drawn in.
    fn item_of_line(&self, line: usize) -> usize {
        if self.mode == Mode::Live {
            return self.segments.iter().position(|s| line < s.line + s.count).map_or(self.segments.len(), |i| i + 1);
        }
        self.parsed.as_ref().map_or(1, |(_, p)| p.row_of_line(line) + 1)
    }

    /// Takes on the new path of the note when it was renamed or moved, in a tab or in its own window.
    fn follow_move(&mut self, to: PathBuf, cx: &mut Context<Self>) {
        let settled = dunce::canonicalize(&to).unwrap_or_else(|_| to.clone());
        self.link_focus.note = Some(settled);
        self.editor.update(cx, |s, _| s.lsp_mut().completion_provider = Some(super::completion::provider(to.clone())));
        self.block.update(cx, |s, _| s.lsp_mut().completion_provider = Some(super::completion::provider(to.clone())));
        self.path = to;
        cx.notify();
    }

    /// Shows the buffer's text in the editor when it changed in another view or from outside.
    /// The editor's undo history ends only when the text came from outside: an older step would write over it.
    fn follow_buffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((rev, version, external)) = state::read(cx, |l| l.buffers.get(&self.path).map(|b| (b.revision, b.version(), b.external))) else {
            if let Some(to) = state::read(cx, |l| l.moved_path(&self.path)) {
                self.follow_move(to, cx);
                self.follow_buffer(window, cx);
            }
            return;
        };
        if rev == self.revision && version == self.version {
            return;
        }
        let Some(text) = state::read(cx, |l| l.buffers.get(&self.path).map(|b| b.text.clone())) else { return };
        self.revision = rev;
        self.version = version;
        let outside = std::mem::replace(&mut self.external, external) != external;
        let reopen = self.live.take().map(|l| (l.first_line, self.block.read(cx).cursor()));
        if let Some((line, caret)) = reopen {
            self.parsed = None;
            window.on_next_frame({
                let me = cx.entity();
                move |window, cx| me.update(cx, |n, cx| n.open_line(line, Some(caret), window, cx))
            });
        }
        let old = self.editor.read(cx).value().to_string();
        if old != text {
            self.history.clear();
            self.future.clear();
            self.live_left = None;
            let (changed, inserted) = edit_ops::minimal_change(&old, &text);
            self.drop_spell_marks(cx);
            self.syncing = true;
            self.editor.update(cx, |s, cx| {
                let (selected, scroll) = (s.selected_range(), s.scroll_offset());
                let start = edit_ops::shift_offset(selected.start, &changed, inserted.len());
                let end = edit_ops::shift_offset(selected.end, &changed, inserted.len());
                if outside {
                    s.set_value(text.clone(), window, cx);
                } else {
                    s.set_selected_range(changed.clone(), cx);
                    s.replace(inserted.to_string(), window, cx);
                    if s.value().to_string() != text {
                        s.set_value(text.clone(), window, cx);
                    }
                }
                let (start, end) = (start.min(text.len()), end.min(text.len()));
                s.set_selected_range(start..end, cx);
                s.set_scroll_offset(scroll, cx);
            });
            self.syncing = false;
            self.schedule_spelling(cx);
        }
        if !parses_in_background(self.mode, text.len()) {
            self.parsed = None;
        }
        cx.notify();
    }

    /// The text of the note right now.
    fn current_text(&self, cx: &App) -> Arc<str> {
        state::read(cx, |l| l.buffers.get(&self.path).map_or_else(|| Arc::from(""), |b| b.snapshot().text.clone()))
    }

    fn shared_snapshot(&self, text: &str, cx: &App) -> Option<Arc<Snapshot>> {
        state::read(cx, |l| l.buffers.get(&self.path).map(NoteBuffer::snapshot).filter(|s| std::ptr::eq(s.text.as_ptr(), text.as_ptr()) && s.text.len() == text.len()))
    }

    fn fingerprint_of(&self, text: &str, cx: &App) -> u64 {
        self.shared_snapshot(text, cx).map_or_else(|| crate::core::find::fingerprint(text), |s| s.fingerprint())
    }

    fn put_text(&mut self, text: String, cx: &mut Context<Self>) {
        let path = self.path.clone();
        let version = state::lib(cx).update(cx, |l, cx| {
            let version = l.buffers.get_mut(&path).map(|b| {
                b.edited(text);
                b.version()
            });
            cx.notify();
            version
        });
        if let Some(version) = version {
            self.version = version;
        }
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
        let at = edit_ops::before_cr(&source, at);
        let at = (0..=at).rev().find(|i| source.is_char_boundary(*i)).unwrap_or(0);
        if self.mode == Mode::Live && source.len() > LIVE_LINE_MAX {
            let start: usize = lines.get(..first_line).unwrap_or_default().iter().map(|l| l.len() + 1).sum();
            self.detour_to_source(start + at..start + at, window, cx);
            self.long_line = true;
            return;
        }
        let single = !matches!(kind, SegmentKind::Whole { .. });
        self.fill_field(source, at..at, true, window, cx);
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
        self.fill_field(value, at..at, false, window, cx);
        self.live = Some(LiveBlock { index, first_line: segment.line, line_count: segment.count, single: true, cell: Some(LiveCell { row, column, line: line_no }), selecting: false });
        self.after_open(index, cx);
    }

    fn fill_field(&mut self, value: String, selection: std::ops::Range<usize>, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_spell_marks(cx);
        self.syncing = true;
        self.block.update(cx, |b, cx| {
            b.set_soft_wrap(wrap, window, cx);
            b.set_value(value, window, cx);
            b.set_selected_range(selection, cx);
            b.focus(window, cx);
        });
        self.syncing = false;
        self.caret_seen = (self.block.read(cx).cursor(), self.block.read(cx).text().len());
        self.follow_caret = false;
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

    /// Whether the parse kept is of this very text.
    fn is_parsed(&self, text: &str) -> bool {
        self.parsed.as_ref().is_some_and(|(rev, _)| *rev == self.revision) && self.parsed_text.as_deref().is_some_and(|known| (std::ptr::eq(known.as_ptr(), text.as_ptr()) && known.len() == text.len()) || known == text)
    }

    /// The note parsed as it is right now, with the list told what changed.
    fn parse_now(&mut self, text: &Arc<str>) -> Rc<ParsedNote> {
        if self.is_parsed(text) {
            if let Some((_, p)) = &self.parsed {
                return p.clone();
            }
        }
        let p = Rc::new(markdown_view::parse(text));
        self.adopt_parsed(text, p.clone());
        p
    }

    /// Keeps a parse of `text` and tells the list which of its rows changed.
    fn adopt_parsed(&mut self, text: &Arc<str>, p: Rc<ParsedNote>) {
        self.first_parse = false;
        let (hashes, mut spans): (Vec<u64>, Vec<usize>) = if self.mode == Mode::Live {
            self.segments = Rc::new(live_text::segments(text, &p.shapes()));
            self.definitions = Rc::new(text.split('\n').filter(|l| l.starts_with('[') && l.contains("]:")).collect::<Vec<_>>().join("\n"));
            self.source_lines = Rc::new(text.split('\n').map(str::to_string).collect());
            self.segments.iter().map(|s| (s.hash, s.count)).unzip()
        } else {
            p.rows().iter().map(|r| (r.hash, r.end_line.saturating_sub(r.line).max(r.bytes / 90))).unzip()
        };
        let middle = spans.len() / 2;
        let per_item = if spans.is_empty() { 0 } else { *spans.select_nth_unstable(middle).1 };
        self.adopt_items(hashes, per_item.max(1) as f32 * note_size(font::BODY) * 1.5);
        self.parsed = Some((self.revision, p));
        self.parsed_text = Some(text.clone());
    }

    /// What the reader draws for the text: a parse made now for a short note, for a long one the last parse while a new one is made away from the interface, `None` before there is any.
    fn parsed_for_reader(&mut self, text: &Arc<str>, cx: &mut Context<Self>) -> Option<Rc<ParsedNote>> {
        if !parses_in_background(self.mode, text.len()) && !first_parse_in_background(self.mode, text.len(), self.first_parse) {
            self.parse_job = None;
            self.parse_notice = None;
            self.parse_slow = false;
            return Some(self.parse_now(text));
        }
        if self.is_parsed(text) {
            return self.parsed.as_ref().map(|(_, p)| p.clone());
        }
        let waiting = self.parse_hold && self.parsed.is_some();
        if !waiting && self.parse_job.is_none() {
            let (revision, mode, text) = (self.revision, self.mode, text.clone());
            if self.parsed.is_none() {
                self.parse_slow = false;
                self.parse_notice = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(NOTICE_AFTER).await;
                    let _ = this.update(cx, |n, cx| {
                        n.parse_slow = true;
                        cx.notify();
                    });
                }));
            }
            let job = cx.spawn(async move |this, cx| {
                let source = text.clone();
                let parsed = cx.background_spawn(async move { markdown_view::parse(&source) }).await;
                let _ = this.update(cx, |n, cx| {
                    n.parse_job = None;
                    n.parse_notice = None;
                    n.parse_slow = false;
                    if n.mode == mode && n.revision == revision && !n.is_parsed(&text) {
                        n.adopt_parsed(&text, Rc::new(parsed));
                    }
                    cx.notify();
                });
            });
            self.parse_job = Some(job);
        }
        self.parsed.as_ref().map(|(_, p)| p.clone())
    }

    /// Closes what is being written; the note is drawn fresh from what is in it now.
    fn finish_block(&mut self, cx: &mut Context<Self>) {
        if let Some(live) = self.live.take() {
            self.drop_spell_marks(cx);
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
            self.history.push((before.to_string(), first_line + caret_line));
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
            self.follow_caret = true;
        }
        cx.notify();
    }

    fn app_history_when_unchanged(&self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (editor, me) = (self.editor.clone(), cx.entity().downgrade());
        let before = editor.read(cx).text().clone();
        window.defer(cx, move |_, cx| {
            let unchanged = *editor.read(cx).text() == before;
            let _ = me.update(cx, |n, cx| if unchanged { n.app_history(back, cx) } else if back { n.text_undone = Some(std::time::Instant::now()) });
        });
    }

    fn app_history(&mut self, back: bool, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).text().clone();
        if back {
            if self.text_undone.is_some_and(|at| at.elapsed() < Duration::from_millis(600)) {
                return;
            }
            act(cx, library::Library::undo);
            self.app_undone = Some(text);
        } else if self.app_undone.as_ref() == Some(&text) {
            act(cx, library::Library::redo);
        }
    }

    /// Takes the note back to what it was before the last burst of typing, wherever that was.
    fn step_history(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((text, line)) = (if back { self.history.pop() } else { self.future.pop() }) else { return };
        let here = self.live.as_ref().map_or(line, |l| l.first_line);
        let current = self.current_text(cx);
        if back {
            self.future.push((current.to_string(), here));
        } else {
            self.history.push((current.to_string(), here));
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
        let height = px((size * 1.5).round());
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
        self.open_selecting(0, count, text.to_string(), 0..end, 0, window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn open_selecting(&mut self, first: usize, count: usize, value: String, selection: std::ops::Range<usize>, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.segments.iter().position(|s| first < s.line + s.count).unwrap_or(index);
        self.fill_field(value, selection, true, window, cx);
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
            self.history.push((text.to_string(), cell.line));
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
        self.history.push((text.to_string(), first_line));
        self.future.clear();
        self.live = None;
        self.put_text(lines.join("\n"), cx);
        self.open_line(upper, Some(at), window, cx);
        self.schedule_autosave(cx);
    }

    /// Scrolls the note so the caret of the open field stands at least one line away from the top and the bottom edge.
    fn keep_caret_in_view(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((caret, line)) = self.block.read(cx).cursor_layout() else { return };
        let view = self.reader_list.viewport_bounds();
        if view.size.height <= line * 3. {
            return;
        }
        let below = caret.bottom() + line - view.bottom();
        let above = view.top() + line - caret.top();
        if below > px(0.) {
            self.reader_list.scroll_by(below);
            cx.notify();
        } else if above > px(0.) {
            self.reader_list.scroll_by(-above);
            cx.notify();
        }
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
        let rows: usize = if cell { 1 } else { window.text_system().shape_text(value.clone().into(), px(size), &[run.clone()], Some(width), None).map_or(1, |lines| lines.iter().map(|l| l.wrap_boundaries().len() + 1).sum()) };
        let line = px((size * 1.5).round());
        let height = line * (rows.max(1) as f32);
        let hidden: gpui_kit::base::input::EditorScrollbarRenderer = Rc::new(|_, _, _| div().into_any_element());
        self.block.update(cx, |s, _| s.set_scrollbar_renderer(cell.then_some(hidden)));
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
                .when(cell, |e| e.w_full().mt(px(-LIVE_INSET)))
                .h(height + px(LIVE_INSET * 2. + 4.))
                .when(!cell, |e| e.ml(px(left - LIVE_GUTTER))),
        );
        if std::mem::take(&mut self.follow_caret) {
            let me = cx.entity();
            window.on_next_frame(move |window, cx| me.update(cx, |n, cx| n.keep_caret_in_view(window, cx)));
        }
        if self.live.is_some_and(|l| l.selecting) && self.block.read(cx).selected_range().is_empty() {
            let me = cx.entity();
            window.on_next_frame(move |window, cx| me.update(cx, |n, cx| n.settle_selection(window, cx)));
        }
        let t = Theme::get(cx);
        let selection = self.block.read(cx).selected_range();
        let caret = self.block.read(cx).cursor();
        let preview = self.live_preview_of(&value, caret, single, &t);
        let bar = !selection.is_empty() && !self.block.read(cx).completion_menu_state().open;
        let room_above = if self.live.is_some_and(|l| l.cell.is_some_and(|c| c.row >= 1)) { 0. } else { self.blank_room_above() };
        let below = bar && room_above < control::M + space::XS;
        let deficit = if below && preview.is_none() && !cell { (control::M + 2. * space::XS - self.blank_room_below()).max(0.) } else { 0. };
        let toolbar = bar.then(|| self.format_bar(&t, height, below, cell, cx));
        let hint = value.is_empty().then(|| div().absolute().top(px(if cell { LIVE_INSET } else { 0. })).left(px(if cell { LIVE_GUTTER } else { left })).when(cell, |d| d.right_0().overflow_hidden().whitespace_nowrap()).font_family(Theme::font_family()).text_size(px(size)).line_height(line).text_color(t.secondary).child(if cell { tr("Text") } else { tr("Schreibe oder tippe / für Befehle") }));
        let more_right = cell && {
            let shown = self.block.read(cx);
            let room = f32::from(shown.input_bounds().size.width);
            let text = f32::from(window.text_system().shape_line(value.clone().into(), px(size), &[run.clone()], None).width);
            let scrolled = f32::from(shown.scroll_offset().x);
            if room <= 0. {
                let me = cx.entity();
                window.on_next_frame(move |_, cx| me.update(cx, |_, cx| cx.notify()));
            }
            room > 0. && text + scrolled > room - LIVE_GUTTER
        };
        let ground = if self.live.as_ref().and_then(|l| l.cell).is_some_and(|c| c.row == 0) { t.secondary_bg } else { t.content };
        let cue = more_right.then(|| div().absolute().top_0().bottom_0().right_0().w(px(space::XL)).bg(linear_gradient(90., linear_color_stop(ground.opacity(0.), 0.), linear_color_stop(ground, 1.))));
        let wrapper = self.note_editor_wrapper(field.into_any_element(), cx);
        let editor = if cell { wrapper } else { div().absolute().top(px(-LIVE_INSET)).left_0().right_0().child(wrapper).into_any_element() };
        div()
            .key_context(if cell { "LiveCell" } else { "LiveBlock" })
            .relative()
            .when(cell, |d| d.mx(px(-LIVE_GUTTER)))
            .pb(px(deficit))
            .child(div().relative().h(height).child(editor).children(hint).children(cue).children(toolbar))
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

    /// How much empty room the open line has above it before text stands there.
    fn blank_room_above(&self) -> f32 {
        let Some(live) = self.live else { return 0. };
        let blank = |k: usize| self.segments.get(k).filter(|s| matches!(s.kind, SegmentKind::Blank)).map(|_| note_size(font::BODY) * 1.5);
        (0..live.index).rev().map_while(blank).sum()
    }

    /// How much empty room the open line has below it before text stands there; the end of the note leaves its own.
    fn blank_room_below(&self) -> f32 {
        let Some(live) = self.live else { return 0. };
        let rest = self.segments.get(live.index + 1..).unwrap_or_default();
        let blanks = rest.iter().take_while(|s| matches!(s.kind, SegmentKind::Blank)).count();
        note_size(font::BODY) * 1.5 * blanks as f32 + if blanks == rest.len() { NOTE_END_ROOM } else { 0. }
    }

    /// Floats over a selection with the formats it can get.
    fn format_bar(&self, t: &Theme, height: Pixels, below: bool, cell: bool, cx: &mut Context<Self>) -> AnyElement {
        let lit = t.ink.opacity(hover::FLOAT);
        let button = |id: &'static str, name: &'static str, help: String, token: &'static str, cx: &mut Context<Self>| {
            div()
                .id(id)
                .size(px(control::S))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(radius::SMALL))
                .cursor_pointer()
                .key_nav()
                .soft(lit, radius::SMALL)
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
        deferred(
            div()
                .absolute()
                .when(!below, |d| d.top(px(-(control::M + space::XS))))
                .when(below, |d| d.top(height + px(space::XS + if cell { LIVE_INSET + space::XS } else { 0. })))
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
        self.history.push((before.to_string(), first));
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
        self.history.push((before.to_string(), first));
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
        let lit = t.ink.opacity(hover::FLOAT);
        let handle = |id: (&'static str, usize), name: &'static str| div().id(id).size(px(control::S)).flex().items_center().justify_center().rounded(px(radius::SMALL)).cursor_pointer().key_nav().soft(lit, radius::SMALL).child(icon(name, iconsize::S, t.tertiary));
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
        self.spell.hover = None;
        self.spell.hide = None;
        if !prefs::get().spell_check {
            self.clear_spelling(cx);
            return;
        }
        self.spell_after(Duration::from_millis(500), cx);
    }

    fn spell_after(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let field = self.target();
        self.spell_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |n, cx| n.mark_misspelled(&field, cx));
        }));
    }

    fn mark_misspelled(&mut self, field: &Entity<EditorState>, cx: &mut Context<Self>) {
        if !prefs::get().spell_check {
            return;
        }
        let words = prefs::get().spell_words;
        let (value, caret) = (field.read(cx).value(), field.read(cx).cursor());
        let window = spell_window(&value, caret);
        let text = &value[window.clone()];
        self.spell.words_seen = Some(words.clone());
        let found: Vec<std::ops::Range<usize>> = crate::core::dictionary::unknown(&words, text, super::spelling::misspelled(text)).into_iter().map(|r| r.start + window.start..r.end + window.start).collect();
        let wavy = HighlightStyle { underline: Some(UnderlineStyle { thickness: px(1.), color: Some(Theme::get(cx).orange), wavy: true }), ..Default::default() };
        let decorations: Vec<TextDecoration> = found.into_iter().map(|r| TextDecoration::new(r, wavy)).collect();
        match &self.spell.marks {
            Some((id, known)) if *id == field.entity_id() => known.set(decorations, cx),
            _ => {
                if let Some((_, old)) = self.spell.marks.take() {
                    old.clear(cx);
                }
                let made = field.update(cx, |e, cx| e.create_decorations_collection(decorations, cx));
                self.spell.marks = Some((field.entity_id(), made));
            }
        }
    }

    fn drop_spell_marks(&mut self, cx: &mut Context<Self>) {
        if let Some((_, marks)) = &self.spell.marks {
            marks.clear(cx);
        }
        self.spell.hover = None;
        self.spell.pending = None;
        self.spell.show = None;
        self.spell.hide = None;
        self.spell.card_hot = false;
        self.spell.words_seen = None;
        spell_card::set_pointer_word(None, cx);
    }

    fn clear_spelling(&mut self, cx: &mut Context<Self>) {
        if let Some((_, old)) = self.spell.marks.take() {
            old.clear(cx);
        }
        self.spell.hover = None;
        self.spell.words_seen = None;
        spell_card::set_pointer_word(None, cx);
    }

    fn spell_pointer(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let field = self.target();
        let Some((_, marks)) = self.spell.marks.clone().filter(|(id, _)| *id == field.entity_id()) else {
            spell_card::set_pointer_word(None, cx);
            return;
        };
        let ranges = marks.get_ranges(cx);
        let under = field.read_with(cx, |s, _| {
            ranges.into_iter().find_map(|r| {
                let b = s.range_to_bounds(&r)?;
                b.contains(&at).then(|| (s.value().get(r.clone()).map(str::to_string), r, b.bottom_left()))
            })
        });
        let Some((Some(word), range, anchor)) = under else {
            spell_card::set_pointer_word(None, cx);
            self.spell.pending = None;
            if self.spell.hover.is_some() && !self.spell.card_hot && self.spell.hide.is_none() {
                self.spell_hide_later(cx);
            }
            return;
        };
        spell_card::set_pointer_word(Some(word.clone()), cx);
        self.spell.hide = None;
        if self.spell.hover.as_ref().is_some_and(|h| h.range == range) || self.spell.pending.as_ref() == Some(&range) {
            return;
        }
        self.spell.pending = Some(range.clone());
        let delay = if self.spell.hover.is_some() { 0 } else { 350 };
        self.spell.show = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(delay)).await;
            let _ = this.update(cx, |n, cx| {
                if n.spell.pending.take().as_ref() == Some(&range) {
                    n.spell.hover = Some(SpellHover { range, word, at: anchor });
                    cx.notify();
                }
            });
        }));
    }

    fn spell_hide_later(&mut self, cx: &mut Context<Self>) {
        self.spell.hide = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(250)).await;
            let _ = this.update(cx, |n, cx| {
                if !n.spell.card_hot {
                    n.spell.hover = None;
                    cx.notify();
                }
            });
        }));
    }

    fn add_marked_word(&mut self, word: Option<String>, cx: &mut Context<Self>) {
        let Some(word) = word else { return };
        self.spell.hover = None;
        self.spell.card_hot = false;
        spell_card::add_word(&word, cx);
        cx.notify();
    }

    fn word_at_caret(&self, cx: &App) -> Option<String> {
        let field = self.target();
        let (_, marks) = self.spell.marks.as_ref().filter(|(id, _)| *id == field.entity_id())?;
        let s = field.read(cx);
        let caret = s.selected_range().start;
        marks.get_ranges(cx).into_iter().find(|r| r.start <= caret && caret <= r.end).and_then(|r| s.value().get(r).map(str::to_string))
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
        self.parse_hold = true;
        self.reparse = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REPARSE_DELAY).await;
            let _ = this.update(cx, |n, cx| {
                n.parse_hold = false;
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
        if let Some(since) = self.split_hold {
            let settled = (editor_y - self.split_sync.0).abs() <= 1. && (reader_y - self.split_sync.1).abs() <= 1.;
            self.split_hold = if !settled { Some(std::time::Instant::now()) } else if since.elapsed() >= SPLIT_HOLD { None } else { Some(since) };
            self.split_sync = (editor_y, reader_y);
            if self.split_hold.is_some() {
                return;
            }
        }
        if line > 0. && reader_max > 0. && self.split_hold.is_none() {
            if let Some(editor_leads) = self.split_lead.filter(|_| self.parsed.is_some() && self.keep_line.is_none()) {
                let width = f32::from(self.reader_list.viewport_bounds().size.width);
                let per_row = if self.editor_wrap { ((width - 56.) / (prefs::get().editor_font_size * 0.6)).max(20.) } else { f32::INFINITY };
                let rows_of = |l: &str| (l.chars().count() as f32 / per_row).ceil().max(1.);
                if editor_leads {
                    let target = editor_y / line;
                    let mut rows = 0.;
                    let mut at = 0;
                    for (i, l) in text.lines().enumerate() {
                        at = i;
                        rows += rows_of(l);
                        if rows > target {
                            break;
                        }
                    }
                    self.keep_line = Some(at);
                } else {
                    let rows: f32 = text.lines().take(self.top_line()).map(rows_of).sum();
                    self.editor.update(cx, |e, cx| e.set_scroll_offset(point(px(0.), px(-rows * line)), cx));
                }
                self.split_hold = Some(std::time::Instant::now());
                self.split_lead = None;
            }
        }
        let (last_editor, last_reader) = self.split_sync;
        let (editor_moved, reader_moved) = ((editor_y - last_editor).abs() > 1., (reader_y - last_reader).abs() > 1.);
        if line <= 0. || reader_max <= 0. || !(editor_moved || reader_moved) {
            return;
        }
        let view = self.reader_list.viewport_bounds().size;
        let per_row = if self.editor_wrap { ((f32::from(view.width) - 56.) / (prefs::get().editor_font_size * 0.6)).max(20.) } else { f32::INFINITY };
        let key = (text.as_ptr() as usize, text.len());
        let rows = match self.split_rows {
            Some((known, width, rows)) if known == key && width == per_row.to_bits() => rows,
            _ => {
                let rows = estimated_rows(text, per_row);
                self.split_rows = Some((key, per_row.to_bits(), rows));
                rows
            }
        };
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

    /// The words of the note; a long one is counted away from the interface once typing pauses, and until then the last count stands.
    fn words_of(&mut self, snapshot: &Arc<Snapshot>, cx: &mut Context<Self>) -> Option<usize> {
        if snapshot.text.len() < PARSE_IN_BACKGROUND_FROM {
            return Some(snapshot.words());
        }
        let key = (snapshot.text.as_ptr() as usize, snapshot.text.len());
        if self.words.1 == Some(key) {
            return Some(self.words.0);
        }
        if self.words_task.as_ref().is_none_or(|(known, _)| *known != key) {
            let snapshot = snapshot.clone();
            self.words_task = Some((
                key,
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(REPARSE_DELAY).await;
                    let counted = snapshot.clone();
                    let count = cx.background_spawn(async move { counted.words() }).await;
                    let _ = this.update(cx, |n, cx| {
                        n.words = (count, Some(key));
                        n.words_task = None;
                        cx.notify();
                    });
                }),
            ));
        }
        self.words.1.map(|_| self.words.0)
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
                prefs::remember_capped(&mut p.note_places, key, offset, NOTE_PLACES);
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
        let problem = result.err().map(|e| save_reason(&path, &e));
        self.set_problem(problem, cx);
        if self.save_problem.is_some() && folder_gone(&self.path) {
            self.wait_for_folder(cx);
        }
    }

    fn set_problem(&mut self, problem: Option<String>, cx: &mut Context<Self>) {
        if problem == self.save_problem {
            return;
        }
        let shown = problem.is_some();
        self.save_problem = problem;
        let path = self.path.clone();
        state::lib(cx).update(cx, |l, _| {
            l.save_problems.retain(|p| *p != path);
            if shown {
                l.save_problems.push(path);
            }
        });
        cx.notify();
    }

    /// While the folder of a note is gone, looks for it every few seconds and saves as soon as it is back.
    fn wait_for_folder(&mut self, cx: &mut Context<Self>) {
        if self.folder_wait.is_some() {
            return;
        }
        self.folder_wait = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(super::motion::FOLDER_RETRY).await;
                let Ok(done) = this.update(cx, |n, cx| {
                    if n.save_problem.is_some() && !folder_gone(&n.path) {
                        n.save(cx);
                    }
                    let done = n.save_problem.is_none() || !folder_gone(&n.path);
                    if done {
                        n.folder_wait = None;
                    }
                    done
                }) else {
                    return;
                };
                if done {
                    return;
                }
            }
        }));
    }

    fn flash_problem(&mut self, cx: &mut Context<Self>) {
        self.save_flash = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(super::motion::FLASH).await;
            let _ = this.update(cx, |n, cx| {
                n.save_flash = None;
                cx.notify();
            });
        }));
    }

    /// Tries to save again after a short wait that shows the button busy, and lights the notice when it still fails.
    fn retry_save(&mut self, cx: &mut Context<Self>) {
        if self.save_pending.is_some() {
            return;
        }
        self.save_flash = None;
        self.save_pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(super::motion::SAVE_RETRY_WAIT).await;
            let _ = this.update(cx, |n, cx| {
                n.save_pending = None;
                n.save(cx);
                if n.save_problem.is_some() {
                    n.flash_problem(cx);
                    let name = crate::core::text::end_ellipsis(&n.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), TOAST_NAME);
                    let path = n.path.clone();
                    act(cx, |l| l.notify(trf("„%@“ ließ sich noch nicht sichern.", &[&name]), "triangle-alert", Some(tr("Erneut versuchen")), Some(retry_unsaved(vec![path]))));
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Lights the notice of a note that could not be saved while its tab was closing, and puts the focus on trying again.
    fn show_save_problem(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.path.clone();
        let wanted = state::lib(cx).update(cx, |l, _| {
            let known = l.unsaved_shown.len();
            l.unsaved_shown.retain(|p| *p != path);
            known != l.unsaved_shown.len()
        });
        if !wanted {
            return;
        }
        self.save(cx);
        if self.save_problem.is_some() {
            self.flash_problem(cx);
            window.focus(&self.retry_focus, cx);
        }
        cx.notify();
    }

    fn save_notice(&self, window: &mut Window, cx: &mut Context<Self>, t: &Theme) -> impl IntoElement + use<> {
        let me = cx.entity();
        let pending = self.save_pending.is_some();
        let retry = Button::new("save-retry").small().primary().label(tr("Erneut versuchen")).track_focus(&self.retry_focus).loading(pending).disabled(pending).on_click(move |_, _, cx| me.update(cx, |n, cx| n.retry_save(cx)));
        let dirty = state::read(cx, |l| l.buffers.get(&self.path).is_some_and(|b| b.dirty));
        let open = self.save_problem.is_some() && dirty;
        let detail = self.save_problem.clone();
        let lit = super::motion::follow("note-save-flash", if self.save_flash.is_some() { 1. } else { 0. }, super::motion::timed(super::motion::MEDIUM, cx), window, cx);
        let path = self.path.clone();
        let measure = canvas(
            move |bounds, _, cx| {
                let height = f32::from(bounds.size.height);
                if state::read(cx, |l| l.save_notice_height.get(&path).is_none_or(|known| (known - height).abs() > 0.5)) {
                    let path = path.clone();
                    cx.defer(move |cx| act(cx, |l| drop(l.save_notice_height.insert(path, height))));
                }
            },
            |_, _, _, _| {},
        );
        super::motion::reveal("note-save-problem", open, div().relative().p(px(space::S)).child(notice("triangle-alert", t.orange, tr("Diese Notiz ließ sich nicht sichern."), detail, retry, t).bg(t.orange.opacity(tint::SAVE_NOTICE + tint::SAVE_FLASH * lit))).child(measure.absolute().top_0().left_0().size_full()))
    }

    fn conflict_notice(&self, open: bool, t: &Theme) -> impl IntoElement + use<> {
        let (compare, reload, keep) = (self.path.clone(), self.path.clone(), self.path.clone());
        let actions = div()
            .flex()
            .items_center()
            .gap(px(space::S))
            .child(Button::new("conflict-compare").small().label(tr("Vergleichen")).on_click(move |_, window, cx| compare_note(compare.clone(), Some(window.window_handle()), cx)))
            .child(Button::new("conflict-reload").small().label(tr("Neu laden")).on_click(move |_, _, cx| act(cx, |l| l.load_note_disk(&reload))))
            .child(Button::new("conflict-keep").small().primary().label(tr("Meine behalten")).tooltip(tr("Die andere Version bleibt als Kopie neben der Notiz erhalten.")).on_click(move |_, _, cx| act(cx, |l| l.keep_note_mine(&keep))));
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

    fn find_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.find.open && self.find.bar.as_ref().is_some_and(|b| b.read(cx).has_focus(window, cx))
    }

    /// Enter follows the focused link, Space shows the preview, Escape ends.
    fn link_keys(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !reader_owns_keys(self.mode, self.live.is_some(), window.has_active_dialog(cx), self.find_has_focus(window, cx)) {
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
        let text = self.current_text(cx).to_string();
        self.save(cx);
        super::pdf::export(&self.path, Some(text), window, cx);
    }

    /// Bold or italic for the selection; where they are refused, a quiet hint says so once per session.
    fn emphasis(&mut self, token: &str, window: &mut Window, cx: &mut Context<Self>) {
        static HINTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let (s, r) = self.with_text(cx);
        let edit = edit_ops::wrap(&s, r.clone(), token);
        let unchanged = edit.range == r && s.get(r.clone()) == Some(edit.text.as_str());
        if !HINTED.load(std::sync::atomic::Ordering::Relaxed) && unchanged && edit_ops::emphasis_refused(&s, &r) && !HINTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            act(cx, |l| l.notify(tr("Fett und Kursiv gehen nicht innerhalb einer Linkadresse oder Formel."), "info", None, None));
        }
        self.apply(edit, window, cx)
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

    fn line_break(&self, field: &str, cx: &App) -> &'static str {
        if self.live.is_some() { edit_ops::eol(&self.current_text(cx)) } else { edit_ops::eol(field) }
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
        let mode_list: Vec<(Mode, String, &'static str)> = [(Mode::Read, tr("Lesen"), "eye"), (edit_mode, tr("Bearbeiten"), "pencil"), (Mode::Split, tr("Geteilte Ansicht"), "columns-2")].into_iter().collect();
        let split_off = self.editor_kind != prefs::NoteEditor::Code;
        let entries = self.toc.clone();
        let has_toc = !entries.is_empty();
        let segment = 2. * space::XXS + mode_list.len() as f32 * control::S;
        let toc_w = control::S;
        let pdf_w = bar_fit::button_width(&tr("PDF"), window);
        let more = control::S;
        let steps = [bar_fit::row(&[segment, toc_w, pdf_w, more], space::S), bar_fit::row(&[segment, toc_w, more], space::S), bar_fit::row(&[segment, more], space::S), bar_fit::row(&[control::S, more], space::S), more];
        let fold = room.map_or_else(|| actions_fold(&self.path, &steps, 2, window, cx), |room| bar_fit::fold_room(room, &steps, 2));
        let (pdf_out, toc_out, picker, modes_out) = (fold >= 1, fold >= 2, fold == 3, fold >= 4);
        let mut modes = div().flex().flex_none().rounded(px(radius::INNER)).bg(t.ink.opacity(0.06)).p(px(space::XXS));
        for (m, label, icon_name) in mode_list.iter().cloned() {
            let active = m == self.mode;
            let e = me.clone();
            let off = split_off && m == Mode::Split;
            let hint = if off { tr("Geteilte Ansicht gibt es nur im Quelltext") } else { keyed(&label, m) };
            let slot = div()
                .id(SharedString::from(format!("mode-{icon_name}")))
                .flex()
                .items_center()
                .justify_center()
                .w(px(control::S))
                .h(px(control::S - 2. * space::XXS))
                .rounded(px(radius::SMALL))
                .when(!off, |d| d.cursor_pointer().key_nav())
                .when(!active && !off, |d| d.soft(t.ink.opacity(0.06), radius::SMALL))
                .when(active, |d| d.bg(if t.dark { t.ink.opacity(0.16) } else { t.content }).shadow_sm())
                .child(icon(icon_name, iconsize::XS, if active { t.primary } else if off { t.secondary.opacity(0.4) } else { t.secondary }))
                .tooltip(tip(hint));
            modes = modes.child(if off { slot } else { slot.on_click(move |_, window, cx| e.update(cx, |n, cx| n.set_mode(m, window, cx))) });
        }
        let (e1, e2, path, p2, p3) = (me.clone(), me.clone(), self.path.clone(), self.path.clone(), self.path.clone());
        let current_mode = self.mode;
        let live_editor = self.editor_kind == prefs::NoteEditor::Live;
        let source_me = me.clone();
        let modes_me = me.clone();
        let find_me = me.clone();
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
        let toc = (!toc_out).then(|| {
            let toc_menu = toc_menu.clone();
            if has_toc {
                Button::new("note-toc").ghost().small().icon(menu_icon("list")).tooltip(tr("Inhalt")).dropdown_menu(move |menu, _, _| toc_menu(menu)).into_any_element()
            } else {
                Button::new("note-toc").ghost().small().icon(menu_icon("list")).tooltip(tr("Noch keine Überschriften")).disabled(true).into_any_element()
            }
        });
        let pdf_tip = tr("Als PDF neben der Notiz sichern (⇧⌘E)");
        let shown_mode = mode_list.iter().find(|(m, ..)| *m == current_mode || (*m == edit_mode && current_mode == Mode::Source)).cloned();
        let mode_picker = picker.then(|| {
            let (list, e) = (mode_list.clone(), me.clone());
            let (label, icon_name) = shown_mode.as_ref().or_else(|| mode_list.first()).map_or_else(|| (String::new(), "eye"), |(_, l, i)| (l.clone(), *i));
            Button::new("note-mode").ghost().small().icon(menu_icon(icon_name)).tooltip(keyed(&label, current_mode)).dropdown_menu(move |mut menu, _, _| {
                for (m, label, _) in list.iter().cloned() {
                    let e = e.clone();
                    menu = menu.item(PopupMenuItem::new(label).disabled(split_off && m == Mode::Split).checked(m == current_mode || (m == edit_mode && current_mode == Mode::Source)).on_click(move |_, window, cx| e.update(cx, |n, cx| n.set_mode(m, window, cx))));
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
                        menu = menu.item(PopupMenuItem::new(label).disabled(split_off && m == Mode::Split).checked(m == current_mode || (m == edit_mode && current_mode == Mode::Source)).on_click(move |_, window, cx| e.update(cx, |n, cx| n.set_mode(m, window, cx))));
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
                let finder = find_me.clone();
                menu = menu.item(PopupMenuItem::new(format!("{}{}", tr("In Notiz suchen"), ShortcutAction::FindInNote.hint())).icon(menu_icon("search")).on_click(move |_, window, cx| finder.update(cx, |n, cx| n.open_find(false, window, cx))));
                let replacer = find_me.clone();
                menu = menu.item(PopupMenuItem::new(tr("In Notiz ersetzen")).icon(menu_icon("replace")).on_click(move |_, window, cx| replacer.update(cx, |n, cx| n.open_find(true, window, cx))));
                if embedded {
                    menu = menu.item(PopupMenuItem::new(tr("In eigenem Fenster öffnen")).icon(menu_icon("app-window")).on_click(move |_, _, cx| {
                        super::note_window::open_exclusive(&a, cx);
                    }));
                }
                menu = menu
                    .item(PopupMenuItem::new(super::platform::reveal_label()).icon(menu_icon("folder")).on_click(move |_, _, _| library::reveal(&b)))
                    .item(PopupMenuItem::new(tr("Mit Standard-App öffnen")).icon(menu_icon("square-arrow-out-up-right")).on_click(move |_, _, _| library::open_path(&c)));
                if let Some(item) = super::convention_rename::menu_item(&d, cx) {
                    menu = menu.item(item);
                }
                let source_view = source_me.clone();
                let is_source = current_mode == Mode::Source;
                menu = menu.when(live_editor, |menu| menu.item(PopupMenuItem::new(format!("{}{}", tr("Quelltext"), ShortcutAction::ToggleLive.hint())).icon(menu_icon("code")).checked(is_source).on_click(move |_, window, cx| source_view.update(cx, |n, cx| n.set_mode(if is_source { Mode::Live } else { Mode::Source }, window, cx)))));
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
                    menu = menu.separator().item(PopupMenuItem::new(format!("{}{}", tr("Tab schließen"), ShortcutAction::CloseTab.hint())).icon(menu_icon("x")).on_click(move |_, _, cx| close_tab(&d, cx)));
                }
                menu
            }))
            .into_any_element()
    }

    fn reader(&mut self, lib_text: &Arc<str>, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let me = cx.entity();
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
            _ => match self.parsed_for_reader(lib_text, cx) {
                Some(p) => p,
                None => return self.loading(t),
            },
        };
        let stale = !self.is_parsed(lib_text);
        let width = self.reader_list.viewport_bounds().size.width;
        if self.row_estimate.1 != Some(width) && self.row_estimate.0 > 0. {
            let _ = self.reader_list.clone().with_uniform_item_height(px(self.row_estimate.0));
            self.row_estimate.1 = Some(width);
            window.request_animation_frame();
        }
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
            let nth = std::mem::take(&mut self.reveal_nth);
            let item = match (self.mode, self.link_focus.find.clone()) {
                (Mode::Live, _) | (_, None) => self.item_of_line(line),
                (_, Some(find)) => parsed.row_of_match(line, nth, &find.query, find.exact) + 1,
            };
            markdown_view::reveal_item_in_context(&self.reader_list, item);
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
        let scope = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (cx.entity_id(), &self.path).hash(&mut hasher);
            hasher.finish()
        };
        let live_item = self.live.as_ref().filter(|l| l.cell.is_none()).map(|l| l.index);
        let covered = self.live.as_ref().filter(|l| l.selecting).map(|l| (l.first_line, l.line_count));
        let live_cell = self.live.as_ref().and_then(|l| l.cell.map(|c| (l.index, c.row, c.column)));
        let hidden = if self.link_focus.folds.borrow().is_empty() { Vec::new() } else { parsed.hidden_blocks(&self.link_focus.folds) };
        let hidden_lines: Rc<Vec<bool>> = if !live_mode || !hidden.iter().any(|h| *h) { Rc::default() } else { Rc::new({
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
        }) };
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
        let room = if live_mode { self.reader_list.bounds_for_item(items - 2).map_or(px(0.), |b| self.reader_list.viewport_bounds().bottom() - b.bottom()) } else { px(0.) };
        let drop_me = me.clone();
        let gutter_me = me.clone();
        let pdf_drop = self.pdf_drop.filter(|_| cx.has_active_drag() && !NotePanels::held(cx));
        let pdf_me = me.clone();
        let (root_me, file_me, tab_me) = (me.clone(), me.clone(), me.clone());
        let (leave_pages, leave_file, leave_tab) = (me.clone(), me.clone(), me.clone());
        let total_lines = source_lines.len();
        let row = list(self.reader_list.clone(), move |i, window, cx| {
            let t = Theme::get(cx);
            if i == 0 {
                let notice_row = plugin.as_ref().map(|(name, path)| plugin_notice(name, path, &t));
                return content(div().pt(px(space::XL)).children(notice_row).into_any_element());
            }
            if i + 1 == items {
                let last_line = source_lines.len().saturating_sub(1);
                let footnotes = markdown_view::render_footnotes(&parsed, Some(footnote_list.clone()), &t).map(|notes| div().capture_any_mouse_down(|_, _, _| markdown_view::press_control()).child(notes));
                let footer = div().id("note-end").relative().min_h(px(NOTE_END_ROOM).max(room)).pb(px(space::XXL)).children(footnotes).when(live_mode, |d| d.cursor_text().on_mouse_up(MouseButton::Left, opener(last_line)));
                let footer = Self::pdf_targets(footer, &pdf_me, total_lines, total_lines, pdf_drop == Some(total_lines), t.accent);
                return content(footer.into_any_element());
            }
            if !live_mode {
                let Some(row) = parsed.rows().get(i - 1).copied() else { return div().into_any_element() };
                if hidden.get(row.block).copied().unwrap_or(false) {
                    return div().into_any_element();
                }
                let next = parsed.rows().get(i).map_or(total_lines, |r| r.line);
                let item = div().id(("read-item", i)).relative().pb(px(parsed.row_gap(i - 1))).child(markdown_view::render_row(&parsed, &row, base.as_deref(), (!stale).then(|| toggle.clone()), found.as_ref(), Some(&focus), &t));
                return content(Self::pdf_targets(item, &pdf_me, row.line, next, pdf_drop == Some(row.line), t.accent).into_any_element());
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
            let cut_note = |left: usize| {
                (left > 0).then(|| {
                    let open_me = me.clone();
                    div()
                        .id(("live-cut", i))
                        .min_h(px(control::S))
                        .flex()
                        .items_center()
                        .text_size(px(font::CAPTION))
                        .text_color(t.secondary)
                        .rounded(px(radius::SMALL))
                        .px(px(space::XS))
                        .cursor_pointer()
                        .key_nav()
                        .soft(t.ink.opacity(hover::FLOAT), radius::SMALL)
                        .capture_any_mouse_down(|_, _, _| markdown_view::press_control())
                        .tooltip(tip(tr("Klicken öffnet die Zeile im Quelltext.")))
                        .on_click(move |_, window, cx| open_me.update(cx, |n, cx| n.open_line(line, None, window, cx)))
                        .child(trf("Hier steht nur der Anfang dieser langen Zeile, %@ KB mehr. Lesen zeigt alles.", &[&crate::core::i18n::grouped(left.div_ceil(1024))]))
                })
            };
            let (body, lead, middle, fold, extra): (AnyElement, f32, Option<f32>, bool, Option<AnyElement>) = match segment.kind {
                SegmentKind::Blank => (div().h(px(note_size(font::BODY) * 1.5)).into_any_element(), 0., None, false, None),
                SegmentKind::Raw => {
                    let (shown, left) = live_text::drawn_part(source_lines.get(line).map_or("", |s| s.as_str()));
                    (div().child(div().text_size(px(note_size(font::CALLOUT))).line_height(relative(1.5)).text_color(t.tertiary).child(shown.to_string())).children(cut_note(left)).into_any_element(), 0., Some(raw_middle), false, None)
                }
                SegmentKind::Line { depth } => {
                    let written = source_lines.get(line).map_or("", |s| s.as_str());
                    let continues = source_lines.get(line + 1).is_some_and(|next| live_text::continues_paragraph(written, next));
                    let (shown, left) = live_text::drawn_part(live_text::drawn_line(written, continues));
                    let fragment = {
                        let mut known = fragments.borrow_mut();
                        if known.len() > 20_000 {
                            known.clear();
                        }
                        known.entry(shown.to_string()).or_insert_with(|| Rc::new(markdown_view::parse(&format!("{shown}\n\n{definitions}")))).clone()
                    };
                    let lead = depth as f32 * LIVE_INDENT;
                    if fragment.len() == 0 {
                        let (plain, plain_left) = live_text::drawn_part(written);
                        (div().child(div().text_size(px(note_size(font::CALLOUT))).line_height(relative(1.5)).text_color(t.tertiary).child(plain.to_string())).children(cut_note(plain_left)).into_any_element(), 0., Some(raw_middle), false, None)
                    } else {
                        let offset_toggle: markdown_view::OnToggle = {
                            let inner = toggle.clone();
                            Rc::new(move |_, cx: &mut App| inner(line, cx))
                        };
                        let draw = markdown_view::LiveDraw { item: Some(i - 1), line, footnotes: Some(parsed.footnote_order()), open_cell: None, count: fragment.line_blocks(), scope: 0 };
                        let drawn = markdown_view::render_live(&fragment, 0, base.as_deref(), Some(offset_toggle), None, Some(&focus), draw, &t);
                        let is_list = live_text::prefix_len(written) > 0 && !written.trim_start().starts_with('#') && !written.trim_start().starts_with('>');
                        (div().pl(px(lead)).when(is_list, |d| d.pb(px(space::XS))).child(drawn).children(cut_note(left)).into_any_element(), lead, fragment.first_line_middle(0), fragment.has_margin_control(0), None)
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
                    let drawn = markdown_view::render_live(&parsed, block, base.as_deref(), Some(toggle.clone()), None, Some(&focus), markdown_view::LiveDraw { item: Some(i - 1), open_cell, scope, ..Default::default() }, &t);
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
                    Some(middle) => gutter.top(px(middle - control::S / 2.)),
                    None => gutter.top_0().bottom_0(),
                }
                .into_any_element()
            });
            let item = div()
                .id(("live-item", i))
                .relative()
                .ml(px(-LIVE_ROOM))
                .pl(px(LIVE_ROOM))
                .group(group)
                .cursor_text()
                .on_mouse_up(MouseButton::Left, opener(line))
                .drag_over::<DraggedLines>(move |s, _, _, _| s.border_t_2().border_color(accent))
                .on_drop(move |d: &DraggedLines, window, cx| drop_target.update(cx, |n, cx| n.move_lines_to(d.line, d.count, line, window, cx)))
                .child(div().relative().child(body).children(handle).children(table_tools));
            content(Self::pdf_targets(item, &pdf_me, line, line + count, pdf_drop == Some(line), accent).into_any_element())
        });
        let accent = t.accent;
        let id = cx.entity_id();
        div()
            .relative()
            .size_full()
            .bg(t.content)
            .drag_over::<DraggedPages>(move |s, _, _, _| s.bg(accent.opacity(tint::DROP_ZONE)))
            .drag_over::<DraggedFile>(move |s, d, _, _| if is_pdf_file(&d.0) { s.bg(accent.opacity(tint::DROP_ZONE)) } else { s })
            .drag_over::<DraggedTab>(move |s, d, _, _| if is_pdf_file(&d.path) { s.bg(accent.opacity(tint::DROP_ZONE)) } else { s })
            .on_drag_move::<DraggedPages>(move |e, _, cx| leave_pages.update(cx, |n, cx| n.leave_reader_drop(e.event.position, e.bounds, cx)))
            .on_drag_move::<DraggedFile>(move |e, _, cx| leave_file.update(cx, |n, cx| n.leave_reader_drop(e.event.position, e.bounds, cx)))
            .on_drag_move::<DraggedTab>(move |e, _, cx| leave_tab.update(cx, |n, cx| n.leave_reader_drop(e.event.position, e.bounds, cx)))
            .on_drop(move |d: &DraggedPages, window, cx| root_me.update(cx, |n, cx| n.drop_pdf(&d.path, &d.pages, window, cx)))
            .on_drop(move |d: &DraggedFile, window, cx| {
                if is_pdf_file(&d.0) {
                    file_me.update(cx, |n, cx| n.drop_pdf(&d.0, &[], window, cx));
                }
            })
            .on_drop(move |d: &DraggedTab, window, cx| {
                if is_pdf_file(&d.path) {
                    tab_me.update(cx, |n, cx| n.drop_pdf(&d.path, &[], window, cx));
                }
            })
            .child(row.size_full())
            .vertical_scrollbar(&self.reader_list)
            .when(self.embedded, |d| {
                d.child(
                    canvas(
                        move |bounds, _, cx| {
                            cx.default_global::<NotePanels>().0.insert(id, bounds);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            })
            .into_any_element()
    }

    /// Lets a PDF, or pages of one, be dropped on a block of the reader: above the block's middle it lands before `line`, below it before `next`. `shown` draws the line where it lands.
    fn pdf_targets(item: Stateful<Div>, me: &Entity<Self>, line: usize, next: usize, shown: bool, accent: Hsla) -> Stateful<Div> {
        let (a, b, c, d, e, f) = (me.clone(), me.clone(), me.clone(), me.clone(), me.clone(), me.clone());
        item.on_drag_move::<DraggedPages>(move |ev, _, cx| a.update(cx, |n, cx| n.hover_item_drop(line, next, ev.event.position, ev.bounds, cx)))
            .on_drag_move::<DraggedFile>(move |ev, _, cx| {
                if is_pdf_file(&ev.drag(cx).0) {
                    b.update(cx, |n, cx| n.hover_item_drop(line, next, ev.event.position, ev.bounds, cx));
                }
            })
            .on_drag_move::<DraggedTab>(move |ev, _, cx| {
                if is_pdf_file(&ev.drag(cx).path) {
                    c.update(cx, |n, cx| n.hover_item_drop(line, next, ev.event.position, ev.bounds, cx));
                }
            })
            .on_drop(move |dp: &DraggedPages, window, cx| d.update(cx, |n, cx| n.drop_pdf(&dp.path, &dp.pages, window, cx)))
            .on_drop(move |df: &DraggedFile, window, cx| {
                if is_pdf_file(&df.0) {
                    e.update(cx, |n, cx| n.drop_pdf(&df.0, &[], window, cx));
                }
            })
            .on_drop(move |dt: &DraggedTab, window, cx| {
                if is_pdf_file(&dt.path) {
                    f.update(cx, |n, cx| n.drop_pdf(&dt.path, &[], window, cx));
                }
            })
            .when(shown, |d| d.child(div().absolute().left_0().right_0().top(px(-pages::DROP_LINE / 2.)).h(px(pages::DROP_LINE)).rounded(px(radius::FULL)).bg(accent)))
    }

    /// Follows a dragged PDF over a block of the reader: above the block's middle it lands before `line`, below it before `next`.
    fn hover_item_drop(&mut self, line: usize, next: usize, at: Point<Pixels>, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if !bounds.contains(&at) {
            return;
        }
        let target = if at.y > bounds.center().y { next } else { line };
        if self.pdf_drop != Some(target) || self.pdf_drop_bar.is_some() {
            self.pdf_drop = Some(target);
            self.pdf_drop_bar = None;
            cx.notify();
        }
    }

    /// A drag that left the reader takes its line along; `at` is the pointer, `bounds` the reader.
    fn leave_reader_drop(&mut self, at: Point<Pixels>, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if !bounds.contains(&at) && self.pdf_drop_bar.is_none() && self.pdf_drop.take().is_some() {
            cx.notify();
        }
    }

    /// Follows a dragged PDF over the writing field: the line snaps to the nearest start of a source line, wrapped rows included; over an open block of the live view it lands before or after the block.
    fn hover_editor_drop(&mut self, at: Point<Pixels>, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if !bounds.contains(&at) {
            if self.pdf_drop_bar.take().is_some() {
                self.pdf_drop = None;
                cx.notify();
            }
            return;
        }
        let (line, bar) = if let Some(live) = self.live.as_ref() {
            let after = at.y > bounds.center().y;
            (if after { live.first_line + live.line_count } else { live.first_line }, (0., if after { f32::from(bounds.size.height) } else { 0. }))
        } else {
            let editor = self.editor.read(cx);
            let (Some(text), Some(height)) = (editor.text_bounds(), editor.line_height()) else { return };
            let map = editor.display_map();
            let (rows, lines, height) = (map.display_row_count(), map.buffer_line_count(), f32::from(height).max(1.));
            let pos = (f32::from(at.y - text.top()) / height).clamp(0., rows as f32);
            let row = (pos.floor() as usize).min(rows);
            let line = if row >= rows { lines } else { map.display_row_to_buffer_line(row).min(lines) };
            let start = if line >= lines { rows } else { map.buffer_line_to_display_row(line) };
            let next = if line + 1 >= lines { rows } else { map.buffer_line_to_display_row(line + 1) };
            let (line, row) = if line >= lines || pos - start as f32 <= next as f32 - pos { (line, start) } else { (line + 1, next) };
            (line, (f32::from(text.left() - bounds.left()), f32::from(text.top() - bounds.top()) + row as f32 * height))
        };
        if self.pdf_drop != Some(line) || self.pdf_drop_bar != Some(bar) {
            self.pdf_drop = Some(line);
            self.pdf_drop_bar = Some(bar);
            cx.notify();
        }
    }

    /// Embeds a PDF, or the dragged pages of one, where the drag ended: each as a paragraph of its own, one undo step, with a short confirmation.
    fn drop_pdf(&mut self, pdf: &Path, pages: &[usize], window: &mut Window, cx: &mut Context<Self>) {
        let line = self.pdf_drop.take();
        self.pdf_drop_bar = None;
        let (missing, undecodable) = state::read(cx, |l| l.buffers.get(&self.path).map_or((true, false), |b| (b.missing, b.unavailable())));
        if missing || undecodable {
            cx.notify();
            return;
        }
        let vault = index::current();
        let in_vault = state::read(cx, |l| l.is_vault) && self.path.starts_with(&vault.root);
        let note = self.path.clone();
        let block = edit_ops::pdf_embeds(pages, |sub| if in_vault { vault.new_link(Some(&note), pdf, sub) } else { edit_ops::folder_link(&note, pdf, sub) });
        let name = pdf.file_name().unwrap_or_default().to_string_lossy().to_string();
        let message = match pages {
            [] => trf("%@ eingefügt", &[&name]),
            [page] => trf("Seite %lld eingefügt", &[&(page + 1)]),
            many => trf("%lld Seiten eingefügt", &[&many.len()]),
        };
        if !self.insert_paragraph_at(line, &block, message.clone(), window, cx) {
            cx.notify();
            return;
        }
        if matches!(self.mode, Mode::Source | Mode::Split) && self.live.is_none() {
            self.editor.update(cx, |s, cx| s.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
        let in_reader = self.mode == Mode::Read;
        act(cx, |l| {
            let undo = in_reader.then(|| l.undo_latest());
            l.notify(message, "file-text", None, undo);
        });
    }

    /// Puts a block before source line `line`, or at the end, as a paragraph of its own and one undo step: in the writing field as one of its steps, in the live view as one of the note's history, in the reader as one of the app's, named `label`. `false` when the note refused the change.
    fn insert_paragraph_at(&mut self, line: Option<usize>, block: &str, label: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let place = |text: &str| line.map_or(text.len(), |l| Self::line_offset(text, l));
        if matches!(self.mode, Mode::Source | Mode::Split) && self.live.is_none() {
            let text = self.editor.read(cx).value().to_string();
            self.apply(edit_ops::insert_paragraph(&text, place(&text), block), window, cx);
            return true;
        }
        if self.mode == Mode::Read {
            let path = self.path.clone();
            let kind = format!("pdf-embed@{}", line.unwrap_or(usize::MAX));
            return act(cx, |l| l.edit_note(&path, &kind, label, |text| edit_ops::apply(text, &edit_ops::insert_paragraph(text, place(text), block))));
        }
        self.finish_block(cx);
        let before = self.current_text(cx);
        let at = place(&before);
        let edit = edit_ops::insert_paragraph(&before, at, block);
        self.history.push((before.to_string(), before[..at].matches('\n').count()));
        self.future.clear();
        self.last_snapshot = None;
        self.put_text(edit_ops::apply(&before, &edit), cx);
        self.schedule_autosave(cx);
        cx.notify();
        true
    }

    /// What stands in for the reader until a long note is parsed; a word only when that takes a while.
    fn loading(&self, t: &Theme) -> AnyElement {
        div().id("note-loading").size_full().bg(t.content).flex().items_center().justify_center().gap(px(space::S)).text_size(px(font::CALLOUT)).text_color(t.secondary).when(self.parse_slow, |d| d.child(super::motion::spinner(iconsize::S, t.secondary)).child(caption(tr("Lädt…"), t))).into_any_element()
    }

    /// Tells the list which items changed, so everything else keeps its measured height.
    /// Items not drawn yet count with `estimate`, so the scrollbar knows the whole note from the start.
    fn adopt_items(&mut self, new: Vec<u64>, estimate: f32) {
        let old = std::mem::take(&mut self.item_hashes);
        let target = new.len() + 2;
        if self.reader_list.item_count() != old.len() + 2 {
            self.reader_list.reset(target);
            self.row_estimate = (estimate, None);
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
        let id = cx.entity_id();
        div()
            .w_full()
            .when(self.live.is_none(), |d| d.h_full())
            .id("note-editor")
            .relative()
            .key_context("NoteEditor")
            .on_drag_move::<DraggedPages>(cx.listener(|n, e: &DragMoveEvent<DraggedPages>, _, cx| n.hover_editor_drop(e.event.position, e.bounds, cx)))
            .on_drag_move::<DraggedFile>(cx.listener(|n, e: &DragMoveEvent<DraggedFile>, _, cx| {
                if is_pdf_file(&e.drag(cx).0) {
                    n.hover_editor_drop(e.event.position, e.bounds, cx);
                }
            }))
            .on_drag_move::<DraggedTab>(cx.listener(|n, e: &DragMoveEvent<DraggedTab>, _, cx| {
                if is_pdf_file(&e.drag(cx).path) {
                    n.hover_editor_drop(e.event.position, e.bounds, cx);
                }
            }))
            .on_drop(cx.listener(|n, d: &DraggedPages, window, cx| n.drop_pdf(&d.path, &d.pages, window, cx)))
            .on_drop(cx.listener(|n, d: &DraggedFile, window, cx| {
                if is_pdf_file(&d.0) {
                    n.drop_pdf(&d.0, &[], window, cx);
                }
            }))
            .on_drop(cx.listener(|n, d: &DraggedTab, window, cx| {
                if is_pdf_file(&d.path) {
                    n.drop_pdf(&d.path, &[], window, cx);
                }
            }))
            .when(self.embedded, |d| {
                d.child(
                    canvas(
                        move |bounds, _, cx| {
                            cx.default_global::<NotePanels>().0.insert(id, bounds);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            })
            .on_mouse_move(cx.listener(|n, e: &MouseMoveEvent, _, cx| n.spell_pointer(e.position, cx)))
            .on_hover(cx.listener(|n, on: &bool, _, cx| {
                if !*on {
                    spell_card::set_pointer_word(None, cx);
                    n.spell.pending = None;
                    if n.spell.hover.is_some() && !n.spell.card_hot && n.spell.hide.is_none() {
                        n.spell_hide_later(cx);
                    }
                }
            }))
            .on_scroll(cx.listener(|n, _: &ScrollEvent, _, cx| {
                if n.spell.hover.take().is_some() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|n, _: &AddWordUnderPointer, _, cx| {
                let word = spell_card::take_menu_word(cx);
                n.add_marked_word(word, cx)
            }))
            .on_action(cx.listener(|n, _: &AddWordAtCaret, _, cx| {
                let word = n.word_at_caret(cx);
                n.add_marked_word(word, cx)
            }))
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(accent.opacity(0.08)))
            .on_drop(cx.listener(|n, paths: &ExternalPaths, window, cx| {
                let paths = paths.paths().to_vec();
                n.drop_images(&paths, window, cx);
            }))
            .on_action(cx.listener(|n, _: &Bold, w, cx| n.emphasis("**", w, cx)))
            .on_action(cx.listener(|n, _: &Italic, w, cx| n.emphasis("*", w, cx)))
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
            .on_action(cx.listener(|n, _: &AskAiSelection, _, cx| {
                let (text, range) = n.with_text(cx);
                let chosen = text.get(range).unwrap_or_default().trim().to_string();
                if chosen.is_empty() {
                    super::ai::request(super::ai::Request::Chip(super::ai_panel::Chip::Note(n.path.clone())), cx);
                } else {
                    super::ai::request(super::ai::Request::Chip(super::ai_panel::Chip::Selection(chosen)), cx);
                }
            }))
            .on_action(cx.listener(|n, _: &CardsAiSelection, _, cx| {
                let (text, range) = n.with_text(cx);
                let chosen = text.get(range).unwrap_or_default().trim().to_string();
                let mut chips = vec![super::ai_panel::Chip::Note(n.path.clone())];
                if !chosen.is_empty() {
                    chips.push(super::ai_panel::Chip::Selection(chosen));
                }
                super::ai::start_flow(crate::core::ai::prompts::Flow::Cards, chips, cx);
            }))
            .on_action(cx.listener(|n, _: &ExplainAiSelection, _, cx| {
                let (text, range) = n.with_text(cx);
                let chosen = text.get(range).unwrap_or_default().trim().to_string();
                let mut chips = vec![super::ai_panel::Chip::Note(n.path.clone())];
                if !chosen.is_empty() {
                    chips.push(super::ai_panel::Chip::Selection(chosen));
                }
                super::ai::start_flow(crate::core::ai::prompts::Flow::Explain, chips, cx);
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
            .on_action(cx.listener(|n, _: &ToggleWrap, w, cx| n.toggle_wrap(w, cx)))
            .on_action(cx.listener(|n, _: &ToggleComment, w, cx| {
                let (s, r) = n.with_text(cx);
                if let Some(e) = edit_ops::toggle_comment(&s, r) {
                    n.apply(e, w, cx)
                }
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
                let (s, r) = n.with_text(cx);
                let r = if r.is_empty() { edit_ops::before_cr(&s, r.start)..edit_ops::before_cr(&s, r.start) } else { r };
                let nl = n.line_break(&s, cx);
                let end = r.start + nl.len();
                n.apply(Edit { range: r, text: nl.into(), select: end..end }, w, cx);
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
                let r = if r.is_empty() { edit_ops::before_cr(&s, r.start)..edit_ops::before_cr(&s, r.start) } else { r };
                let nl = n.line_break(&s, cx);
                if r.is_empty() {
                    let (full, line) = match n.live {
                        Some(live) => (n.current_text(cx), live.first_line + s[..r.start].matches('\n').count()),
                        None => (Arc::from(s.as_str()), s[..r.start].matches('\n').count()),
                    };
                    if let Some(mark) = edit_ops::closing_mark(&full, line).filter(|_| s[r.start..].split('\n').next().is_some_and(|rest| rest.trim().is_empty())) {
                        let inside = r.start + nl.len();
                        n.apply(Edit { range: r.clone(), text: format!("{nl}{nl}{mark}"), select: inside..inside }, w, cx);
                        return;
                    }
                }
                match (r.is_empty(), edit_ops::newline_in(&s, r.start, nl)) {
                    (true, Some(e)) => n.apply(e, w, cx),
                    _ => n.apply(Edit { range: r.clone(), text: nl.into(), select: r.start + nl.len()..r.start + nl.len() }, w, cx),
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
                if !n.target().read(cx).focus_handle(cx).is_focused(w) {
                    return;
                }
                if n.live.is_none() {
                    if n.history.is_empty() {
                        n.app_history_when_unchanged(true, w, cx);
                    }
                } else if n.history.is_empty() {
                    cx.stop_propagation();
                    n.app_history(true, cx);
                } else {
                    cx.stop_propagation();
                    n.text_undone = Some(std::time::Instant::now());
                    n.step_history(true, w, cx);
                }
            }))
            .capture_action(cx.listener(|n, _: &gpui_kit::component::input::Redo, w, cx| {
                if !n.target().read(cx).focus_handle(cx).is_focused(w) {
                    return;
                }
                if n.live.is_none() {
                    n.app_history_when_unchanged(false, w, cx);
                } else if n.future.is_empty() {
                    cx.stop_propagation();
                    n.app_history(false, cx);
                } else {
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
            .when_some(self.pdf_drop_bar.filter(|_| cx.has_active_drag() && !NotePanels::held(cx)), |d, (left, top)| {
                d.child(div().absolute().top_0().left_0().size_full().bg(accent.opacity(tint::DROP_ZONE)))
                    .child(div().absolute().left(px(left.max(space::M))).right(px(space::M)).top(px(top - pages::DROP_LINE / 2.)).h(px(pages::DROP_LINE)).rounded(px(radius::FULL)).bg(accent))
            })
            .into_any_element()
    }
}

impl Focusable for NoteView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Whether the note's own keys (typing opens a line, Enter follows a link) apply, which they never do while the find bar or a dialog has the keyboard.
fn reader_owns_keys(mode: Mode, line_open: bool, dialog: bool, find_focused: bool) -> bool {
    !matches!(mode, Mode::Source) && !line_open && !dialog && !find_focused
}

#[cfg(test)]
mod key_tests {
    use super::{Mode, reader_owns_keys};

    #[test]
    fn nothing_typed_in_the_find_bar_reaches_the_note_in_any_mode() {
        for mode in [Mode::Source, Mode::Live, Mode::Read, Mode::Split] {
            assert!(!reader_owns_keys(mode, false, false, true));
            assert!(!reader_owns_keys(mode, true, false, true));
        }
    }

    #[test]
    fn the_reader_keeps_its_keys_when_the_find_bar_is_closed() {
        assert!(reader_owns_keys(Mode::Live, false, false, false));
        assert!(reader_owns_keys(Mode::Read, false, false, false));
        assert!(!reader_owns_keys(Mode::Source, false, false, false));
        assert!(!reader_owns_keys(Mode::Live, true, false, false));
        assert!(!reader_owns_keys(Mode::Live, false, true, false));
    }
}

/// Estimated rows of soft-wrapped monospaced text in an editor column of `width`, gutter included.

/// The lines the assistant just changed in this file, waiting to be shown.
fn take_changed_request(path: &Path, cx: &mut App) -> Option<Vec<(usize, usize)>> {
    state::lib(cx).update(cx, |l, _| {
        let matches = l.changed_lines.as_ref().is_some_and(|(p, _)| dunce::canonicalize(p).is_ok_and(|p| p == path));
        if matches { l.changed_lines.take().map(|(_, lines)| lines) } else { None }
    })
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
        if !cx.has_active_drag() && self.pdf_drop.take().is_some() {
            self.pdf_drop_bar = None;
        }
        if self.save_problem.is_some() {
            self.retry_held = self.retry_focus.is_focused(window);
        } else if std::mem::take(&mut self.retry_held) && (self.retry_focus.is_focused(window) || window.focused(cx).is_none()) {
            self.focus_writing(window, cx);
        }
        let settings = prefs::get();
        if !settings.spell_check && self.spell.marks.is_some() {
            self.clear_spelling(cx);
        }
        if settings.spell_check && self.spell.words_seen.as_ref() != Some(&settings.spell_words) {
            self.spell.words_seen = Some(settings.spell_words.clone());
            self.spell_after(Duration::ZERO, cx);
        }
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
        if state::read(cx, |l| l.unsaved_shown.contains(&self.path)) {
            cx.defer_in(window, |n, window, cx| n.show_save_problem(window, cx));
        }
        let t = Theme::get(cx);
        let lib = state::lib(cx);
        let wheel = super::image_view::wheel_zoom(|factor, _, _, cx| scale_font(factor, cx));
        if !lib.read(cx).buffers.contains_key(&self.path) {
            self.follow_buffer(window, cx);
        }
        let (missing, snapshot, conflict) = lib.update(cx, |l, _| {
            let b = l.buffer(&self.path);
            (b.missing, b.snapshot(), b.conflict.is_some())
        });
        let text = snapshot.text.clone();
        let request = take_find_request(&self.path, cx);
        if let Some((term, nth)) = request {
            self.reveal(&term, nth, &text, window, cx);
        }
        if let Some(lines) = take_changed_request(&self.path, cx) {
            self.show_changes(&lines, &text, window, cx);
        }
        self.sync_find(&text, false, true, window, cx);
        self.center_find(cx);
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
            if self.split_hold.is_some() {
                window.request_animation_frame();
            }
        }
        let (undecodable, unreadable) = state::read(cx, |l| l.buffers.get(&self.path).map_or((false, false), |b| (b.undecodable, b.unreadable)));
        let words = if matches!(self.mode, Mode::Source | Mode::Split) { self.words_of(&snapshot, cx) } else { None };
        let caption = if matches!(self.mode, Mode::Source | Mode::Split) { self.plain_caption(text.len(), &t, cx) } else { None };
        let content: AnyElement = if unreadable {
            unavailable("file-lock-2", tr("Datei gerade nicht lesbar"), tr("Eine andere App hält die Datei gerade fest. Schrumbi ändert nichts daran und zeigt sie, sobald sie wieder lesbar ist."), &t).into_any_element()
        } else if undecodable {
            let original = self.path.clone();
            unavailable("file-warning", tr("Kodierung wird nicht unterstützt"), tr("Die Datei ist nicht in UTF-8 gespeichert. Sie bleibt unverändert. Eine UTF-8-Kopie daneben kannst du hier bearbeiten."), &t)
                .child(div().pt(px(space::M)).child(Button::new("utf8-copy").small().label(tr("UTF-8-Kopie öffnen")).on_click(move |_, _, cx| act(cx, |l| {
                    l.utf8_copy(&original);
                }))))
                .into_any_element()
        } else if missing {
            let closing = self.path.clone();
            unavailable("file-question", tr("Datei nicht gefunden"), trf("„%@“ wurde gelöscht oder außerhalb dieses Ordners verschoben.", &[&self.path.file_name().unwrap_or_default().to_string_lossy()]), &t).child(div().pt(px(space::M)).child(Button::new("missing-close").small().label(tr("Tab schließen")).on_click(move |_, _, cx| close_tab(&closing, cx)))).into_any_element()
        } else {
            match self.mode {
                Mode::Read | Mode::Live => self.reader(&text, &t, window, cx),
                Mode::Split => div().size_full().flex().child(div().flex_1().min_w_0().h_full().flex().flex_col().border_r_1().border_color(t.separator).child(div().flex_1().min_h_0().child(self.editor_view(cx))).child(word_count(words, caption, &t))).child(div().flex_1().min_w_0().h_full().child(self.reader(&text, &t, window, cx))).into_any_element(),
                Mode::Source => div().size_full().flex().flex_col().child(div().flex_1().min_h_0().child(self.editor_view(cx))).child(word_count(words, caption, &t)).into_any_element(),
            }
        };
        let top = if self.embedded {
            document_bar(&self.path, header, &t, window, cx)
        } else {
            div().relative().overflow_hidden().h(px(control::SUBBAR)).flex_none().flex().items_center().justify_center().border_b_1().border_color(t.separator).child(self.fit.measure_bar()).child(header)
        };
        let hover_card = (!self.embedded).then(|| super::link_card::overlay(cx)).flatten();
        let spell_card = self.spell.hover.as_ref().map(|h| {
            let (add, hot) = (cx.entity(), cx.entity());
            let word = h.word.clone();
            spell_card::card(
                h,
                move |_, cx| add.update(cx, |n, cx| n.add_marked_word(Some(word.clone()), cx)),
                move |on, _, cx| {
                    hot.update(cx, |n, cx| {
                        n.spell.card_hot = on;
                        if on {
                            n.spell.hide = None;
                        } else {
                            n.spell_hide_later(cx);
                        }
                    })
                },
                &t,
            )
        });
        let note_context = format!("NoteView{}{}", if !matches!(self.mode, Mode::Source) && self.live.is_none() { " WalkItems" } else { "" }, if self.find.open { " FindOpen" } else { "" });
        let view = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .children(hover_card)
            .children(spell_card)
            .track_focus(&self.focus)
            .key_context(note_context.as_str())
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
            .on_action(cx.listener(|n, _: &FindInNote, w, cx| n.open_find(false, w, cx)))
            .on_action(cx.listener(|n, _: &gpui_kit::component::input::Search, w, cx| n.open_find(false, w, cx)))
            .on_action(cx.listener(|n, _: &ReplaceInNote, w, cx| n.open_find(true, w, cx)))
            .on_action(cx.listener(|n, _: &gpui_kit::component::input::Replace, w, cx| n.open_find(true, w, cx)))
            .on_action(cx.listener(|n, _: &CloseFind, w, cx| n.close_find(w, cx)))
            .on_action(cx.listener(|n, _: &gpui_kit::component::input::Escape, w, cx| if n.find.open { n.close_find(w, cx) } else { cx.propagate() }))
            .on_action(cx.listener(|n, _: &super::canvas_view::NextItem, window, cx| n.step_link(false, window, cx)))
            .on_action(cx.listener(|n, _: &super::canvas_view::PreviousItem, window, cx| n.step_link(true, window, cx)))
            .when(!self.embedded, |d| {
                d.on_action(|_: &super::app::CloseTab, window, cx| super::note_window::request_close(window, cx))
                    .on_action(|_: &super::app::CloseWindow, window, cx| super::note_window::request_close(window, cx))
            })
            .child(top)
            .children(self.find.bar.clone().filter(|_| self.find.open))
            .child(self.conflict_notice(conflict, &t))
            .child(div().flex_1().min_h_0().child(content))
            .child(self.save_notice(window, cx, &t));
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
pub fn compare_note(path: PathBuf, window: Option<AnyWindowHandle>, cx: &mut App) {
    let Some((mine, theirs)) = state::read(cx, |l| l.buffers.get(&path).and_then(|b| b.conflict.clone().map(|c| (b.text.clone(), c)))) else { return };
    let Some(window) = window.or_else(|| cx.active_window()) else { return };
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
                d.entrance(DialogEntrance::FadeSlide).w(px(760.)).p_0().on_ok(super::sheets::pass_enter).child(
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

fn folder_gone(path: &Path) -> bool {
    path.parent().is_some_and(|d| !d.exists())
}

fn save_reason(path: &Path, error: &std::io::Error) -> String {
    if folder_gone(path) {
        return tr("Den Ordner dieser Notiz gibt es nicht mehr. Dein Text bleibt offen und wird gesichert, sobald der Ordner wieder da ist.");
    }
    crate::core::errors::explain_io(error).unwrap_or_else(|| error.to_string())
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
        Err(_) => false,
    }
}

const TOAST_NAME: usize = 24;

/// What the toast says about notes that stay open; `visible` points to the note's own notice, otherwise the toast offers trying again.
fn unsaved_message(names: &[String], visible: bool) -> String {
    match names {
        [one] if visible => trf("„%@“ ließ sich nicht sichern und bleibt offen. Erneut versuchen steht oben in der Notiz.", &[one]),
        [one] => trf("„%@“ ließ sich nicht sichern und bleibt offen.", &[one]),
        [_, _] | [_, _, _] => {
            let quoted: Vec<String> = names.iter().map(|n| format!("„{n}“")).collect();
            trf("%@ ließen sich nicht sichern und bleiben offen.", &[&quoted.join(", ")])
        }
        _ => trf("%@ Notizen ließen sich nicht sichern und bleiben offen.", &[&crate::core::i18n::grouped(names.len())]),
    }
}

fn announce_unsaved(l: &mut library::Library, failed: Vec<PathBuf>) {
    let names: Vec<String> = failed.iter().map(|p| crate::core::text::end_ellipsis(&l.note_title(p), TOAST_NAME)).collect();
    let visible = failed.len() == 1
        && matches!(l.route(), library::Route::Doc(_))
        && [l.active_document.as_deref(), l.split_document.as_deref()].into_iter().flatten().any(|d| l.buffer_key(d) == failed[0]);
    let message = unsaved_message(&names, visible);
    if visible {
        l.notify(message, "triangle-alert", None, None);
    } else {
        l.notify(message, "triangle-alert", Some(tr("Erneut versuchen")), Some(retry_unsaved(failed)));
    }
}

/// Saves the notes again, whether or not their tabs are shown; the ones that still fail are announced again.
fn retry_unsaved(paths: Vec<PathBuf>) -> library::Undo {
    Box::new(move |l| {
        let failed: Vec<PathBuf> = paths
            .into_iter()
            .filter(|p| {
                let saved = l.buffers.get_mut(p).is_none_or(|b| b.release(false).and_then(|_| b.save()).is_ok());
                if saved {
                    l.save_problems.retain(|s| s != p);
                    if !l.unsaved_shown.contains(p) {
                        l.unsaved_shown.push(p.clone());
                    }
                }
                !saved
            })
            .collect();
        if !failed.is_empty() {
            announce_unsaved(l, failed);
        }
    })
}

/// Saves the given notes and returns the ones that could not be saved. They stay open: their editors show why and one toast names them.
fn save_unsaved(paths: &[PathBuf], cx: &mut App) -> Vec<PathBuf> {
    let dirty: Vec<PathBuf> = state::read(cx, |l| {
        let mut keys: Vec<PathBuf> = Vec::new();
        for key in paths.iter().map(|p| l.buffer_key(p)) {
            if l.buffers.get(&key).is_some_and(|b| b.dirty) && !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys
    });
    let failed: Vec<PathBuf> = dirty.into_iter().filter(|p| !save_buffer(p, cx)).collect();
    if !failed.is_empty() {
        act(cx, |l| {
            let fresh: Vec<PathBuf> = failed.iter().filter(|p| !l.unsaved_shown.contains(p)).cloned().collect();
            l.unsaved_shown.extend(fresh);
            announce_unsaved(l, failed.clone());
        });
    }
    failed
}

/// Saves the given notes, then runs `done`. A failed save keeps everything open.
pub fn resolve_unsaved(paths: Vec<PathBuf>, cx: &mut App, done: impl FnOnce(&mut App) + 'static) {
    if save_unsaved(&paths, cx).is_empty() {
        done(cx);
    }
}

/// Saves the note behind a tab, then closes the tab; a note that cannot be saved stays open.
pub fn close_tab(path: &Path, cx: &mut App) {
    let target = path.to_path_buf();
    resolve_unsaved(vec![target.clone()], cx, move |cx| act(cx, |l| l.close_document(&target)));
}

fn close_other_tabs(path: &Path, cx: &mut App) {
    let keep = path.to_path_buf();
    let others: Vec<PathBuf> = state::read(cx, |l| l.open_documents.iter().filter(|d| **d != keep).cloned().collect());
    let failed = save_unsaved(&others, cx);
    if failed.is_empty() {
        act(cx, |l| l.close_other_documents(&keep));
        return;
    }
    act(cx, |l| {
        let closing: Vec<&PathBuf> = others.iter().filter(|p| !failed.contains(&l.buffer_key(p))).collect();
        for other in closing {
            l.close_document(other);
        }
        l.open_document(&keep);
    });
}

fn close_tabs_right(path: &Path, cx: &mut App) {
    let keep = path.to_path_buf();
    let right: Vec<PathBuf> = state::read(cx, |l| l.open_documents.iter().skip_while(|d| **d != keep).skip(1).filter(|d| l.split_document.as_ref() != Some(*d)).cloned().collect());
    let failed = save_unsaved(&right, cx);
    act(cx, |l| {
        if failed.is_empty() {
            l.close_documents_right(&keep);
            return;
        }
        let closing: Vec<&PathBuf> = right.iter().filter(|p| !failed.contains(&l.buffer_key(p))).collect();
        for tab in closing {
            l.close_document(tab);
        }
        l.open_document(&keep);
    });
}

pub fn close_all_tabs(cx: &mut App) {
    let all = state::read(cx, |l| l.open_documents.clone());
    let failed = save_unsaved(&all, cx);
    if failed.is_empty() {
        act(cx, |l| l.close_all_documents());
        return;
    }
    act(cx, |l| {
        let closing: Vec<&PathBuf> = all.iter().filter(|p| !failed.contains(&l.buffer_key(p))).collect();
        for tab in closing {
            l.close_document(tab);
        }
    });
}

fn word_count(words: Option<usize>, caption: Option<AnyElement>, t: &Theme) -> Div {
    let count = match words {
        Some(1) => div().child(tr("1 Wort")),
        Some(n) => div().child(trf("%lld Wörter", &[&crate::core::i18n::grouped(n)])),
        None => div().text_color(t.tertiary).child("…"),
    };
    div().flex_none().flex().items_center().justify_between().gap(px(space::M)).px(px(space::M)).py(px(space::XS)).border_t_1().border_color(t.separator).text_size(px(font::CAPTION)).text_color(t.secondary).children(caption).child(count.flex_none().ml_auto())
}

thread_local! {
    static HOVERED_TAB: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
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
        let (title, focused) = state::read(cx, |l| (l.note_title(current), l.beside_active));
        let close = current.to_path_buf();
        return div()
            .relative()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(space::S))
            .px(px(space::M))
            .flex_none().h(px(control::SUBBAR))
            .border_b_1()
            .border_color(if focused { t.accent.opacity(0.6) } else { t.separator })
            .child(fit.measure_bar())
            .child(div().flex_none().child(file_icon(current, iconsize::M)))
            .child(
                div()
                    .id("split-title")
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_middle()
                    .text_size(px(font::CALLOUT))
                    .tooltip(tip(title.clone()))
                    .context_menu(move |m, _, _| {
                        let shut = close.clone();
                        m.item(PopupMenuItem::new(format!("{}{}", tr("Tab schließen"), ShortcutAction::CloseTab.hint())).on_click(move |_, _, cx| close_tab(&shut, cx)))
                            .separator()
                            .item(PopupMenuItem::new(tr("Geteilte Ansicht schließen")).icon(menu_icon("columns-2")).on_click(|_, _, cx| act(cx, |l| l.set_split(None))))
                    })
                    .child(div().child(title)),
            )
            .child(div().flex_none().child(actions))
            .child(Button::new("split-close").ghost().small().icon(menu_icon("x")).tooltip(tr("Geteilte Ansicht schließen")).on_click(|_, _, cx| act(cx, |l| l.set_split(None))));
    }
    let (beside, focused, can_beside) = state::read(cx, |l| (l.split_document.clone(), l.split_document.is_some() && !l.beside_active, l.can_drop_document(current, library::DocDrop::Beside, None)));
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
    let hovered = HOVERED_TAB.with_borrow(Clone::clone);
    let mut tabs = div().flex().gap(px(space::XXS)).py(px(space::S));
    for (i, doc) in docs.iter().enumerate() {
        let active = doc.as_path() == current;
        let (start, end) = (left + offset, left + offset + widths[i]);
        left += widths[i] + space::XXS;
        if !active && strip.is_some_and(|strip| end < -strip || start > 2. * strip) {
            tabs = tabs.child(div().flex_none().w(px(widths[i])));
            continue;
        }
        let (open, menu, middle, target) = (doc.clone(), doc.clone(), doc.clone(), doc.clone());
        let accent = t.accent;
        let is_dirty = dirty[i];
        let hover = t.ink.opacity(0.04);
        let is_beside = beside.as_ref() == Some(doc);
        let (others, rightwards) = (docs.len() > 1, docs.len() > i + 1);
        let squeezed = widths[i] + 0.5 < natural[i] && !is_dirty;
        let pointed = hovered.as_ref() == Some(doc);
        let close_shown = active || is_dirty || pointed;
        let close_button = if close_shown {
            let close = doc.clone();
            div()
                .id(SharedString::from(format!("tab-close-{}", doc.display())))
                .flex_none()
                .size(px(iconsize::M))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .soft(t.ink.opacity(0.1), radius::FULL)
                .child(if is_dirty && !pointed { div().size(px(space::S)).rounded_full().bg(t.secondary).into_any_element() } else { icon("x", iconsize::XS, t.secondary).into_any_element() })
                .tooltip(tip(format!("{}{}", if is_dirty { tr("Ungesicherte Änderungen – Tab schließen") } else { tr("Tab schließen") }, ShortcutAction::CloseTab.hint())))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    close_tab(&close, cx);
                })
                .into_any_element()
        } else {
            div().flex_none().size(px(iconsize::M)).into_any_element()
        };
        let tab_path = doc.clone();
        let hint = move |window: &mut Window, cx: &mut App| {
            let rel = state::read(cx, |l| l.relative_path(&tab_path));
            tip(if is_beside { format!("{rel}
{}", tr("Rechts geöffnet")) } else { rel })(window, cx)
        };
        let hover_path = doc.clone();
        let under = t.content.blend(t.ink.opacity(if active { 0.1 } else { 0.05 }));
        tabs = tabs.child(
            div()
                .id(SharedString::from(format!("tab-{}", doc.display())))
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
                .when(is_beside, |d| d.bg(t.ink.opacity(0.04)).soft(t.ink.opacity(0.02), radius::INNER))
                .when(!active && !is_beside, |d| d.soft(hover, radius::INNER))
                .tooltip(hint)
                .on_hover(move |on, _, _| HOVERED_TAB.with_borrow_mut(|h| {
                    if *on {
                        *h = Some(hover_path.clone());
                    } else if h.as_ref() == Some(&hover_path) {
                        *h = None;
                    }
                }))
                .on_click(move |_, _, cx| act(cx, |l| l.open_document(&open)))
                .on_mouse_down(MouseButton::Middle, move |_, _, cx| close_tab(&middle, cx))
                .on_drag(DraggedTab { path: doc.clone(), title: titles[i].clone() }, |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(accent.opacity(0.15)))
                .on_drop(move |dragged: &DraggedTab, _, cx| {
                    let path = dragged.path.clone();
                    act(cx, |l| l.move_document(&path, &target));
                })
                .child(div().flex_none().child(if is_beside { icon("split-right", iconsize::M, t.secondary).into_any_element() } else { file_icon(doc, iconsize::M).into_any_element() }))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis_middle().text_size(px(font::CALLOUT)).text_color(if active || is_beside { t.primary } else { t.secondary }).child(titles[i].clone()))
                .children(match (squeezed, close_shown) {
                    (true, true) => Some(div().absolute().top_0().bottom_0().right_0().pl(px(space::XS)).pr(px(space::S)).rounded_r(px(radius::INNER)).flex().items_center().bg(under).child(close_button).into_any_element()),
                    (true, false) => None,
                    (false, _) => Some(close_button),
                })
                .context_menu(move |m, window, cx| {
                    let (a, b, c, d, beside, right, shown) = (menu.clone(), menu.clone(), menu.clone(), menu.clone(), menu.clone(), menu.clone(), menu.clone());
                    let ai_path = menu.clone();
                    let m = m.when(!active && !is_beside, |m| m.item(PopupMenuItem::new(tr("Daneben öffnen")).icon(menu_icon("columns-2")).on_click(move |_, _, cx| act(cx, |l| l.set_split(Some(beside.clone()))))))
                        .when(active && can_beside, |m| m.item(PopupMenuItem::new(tr("Daneben öffnen")).icon(menu_icon("columns-2")).on_click(move |_, _, cx| act(cx, |l| l.drop_document(&shown, library::DocDrop::Beside, None)))))
                        .item(PopupMenuItem::new(format!("{}{}", tr("Tab schließen"), ShortcutAction::CloseTab.hint())).on_click(move |_, _, cx| close_tab(&a, cx)))
                        .item(PopupMenuItem::new(tr("Andere Tabs schließen")).disabled(!others).on_click(move |_, _, cx| close_other_tabs(&b, cx)))
                        .item(PopupMenuItem::new(tr("Tabs rechts schließen")).disabled(!rightwards).on_click(move |_, _, cx| close_tabs_right(&right, cx)))
                        .item(PopupMenuItem::new(tr("Alle Tabs schließen")).on_click(|_, _, cx| close_all_tabs(cx)))
                        .separator()
                        .item(PopupMenuItem::new(super::platform::reveal_label()).on_click(move |_, _, _| library::reveal(&c)))
                        .item(PopupMenuItem::new(tr("Mit Standard-App öffnen")).on_click(move |_, _, _| library::open_path(&d)));
                    super::ai::file_submenu(m, &ai_path, window, cx)
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
        .border_color(if focused { t.accent.opacity(0.6) } else { t.separator })
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
    sheet: Option<Entity<super::sheet_view::SheetView>>,
    preview: Entity<FilePreview>,
    app: Option<String>,
    as_text: bool,
}

impl FileViewer {
    pub fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let kind = crate::core::office::kind(&path);
        let is_pdf = extension(&path.to_string_lossy()).eq_ignore_ascii_case("pdf");
        let pdf = match kind {
            _ if is_pdf => Some(cx.new(|cx| PdfView::new(&path, cx).take_focus_on_load())),
            Some(crate::core::office::Kind::Document | crate::core::office::Kind::Slides) => Some(cx.new(|cx| PdfView::office(&path, cx).take_focus_on_load())),
            _ => None,
        };
        let sheet = (kind == Some(crate::core::office::Kind::Sheet)).then(|| cx.new(|cx| super::sheet_view::SheetView::new(&path, cx).take_focus_on_load()));
        let preview = cx.new(|cx| {
            let mut p = FilePreview::new(false);
            if pdf.is_none() && sheet.is_none() {
                p.set_path(Some(path.clone()), cx);
            }
            p
        });
        let app = super::platform::default_app_name(&path);
        Self { path, pdf, sheet, preview, app, as_text: false }
    }
}

impl FileViewer {
    /// Gives the keyboard to the document, so its shortcuts work right after opening.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pdf) = self.pdf.clone() {
            pdf.update(cx, |v, cx| v.take_focus(window, cx));
        } else if let Some(sheet) = self.sheet.clone() {
            sheet.update(cx, |v, cx| v.take_focus(window, cx));
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
        } else if let Some(sheet) = self.sheet.clone()
            && let Some((term, nth)) = take_find_request(&self.path, cx)
        {
            sheet.update(cx, |v, cx| v.search_for(&term, nth, window, cx));
        }
        let (p1, p2, p3, p4) = (self.path.clone(), self.path.clone(), self.path.clone(), self.path.clone());
        let open_label = self.app.as_ref().map_or_else(|| tr("Mit Standard-App öffnen"), |a| trf("In %@ öffnen", &[a]));
        let table_toggle = crate::core::office::is_delimited(&self.path).then(|| {
            let as_text = self.as_text;
            Button::new("viewer-text").ghost().small().icon(menu_icon(if as_text { "table" } else { "file-text" })).tooltip(if as_text { tr("Als Tabelle zeigen") } else { tr("Als Text zeigen") }).on_click(cx.listener(move |v, _, _, cx| {
                v.as_text = !v.as_text;
                let raw = v.as_text;
                v.preview.update(cx, |p, cx| p.show_raw(raw, cx));
                cx.notify();
            }))
        });
        let more = Button::new("viewer-more").ghost().small().icon(menu_icon("ellipsis")).tooltip(tr("Mehr")).dropdown_menu(move |m, _, cx| {
            let (a, b, c, o, n) = (p1.clone(), p2.clone(), p3.clone(), p4.clone(), p4.clone());
            let named = super::convention_rename::menu_item(&n, cx);
            m.item(PopupMenuItem::new(open_label.clone()).icon(menu_icon("square-arrow-out-up-right")).on_click(move |_, _, _| library::open_path(&o)))
                .item(PopupMenuItem::new(super::platform::reveal_label()).icon(menu_icon("folder")).on_click(move |_, _, _| library::reveal(&a)))
                .item(PopupMenuItem::new(tr("Vorschau-Fenster (Leertaste)")).icon(menu_icon("eye")).on_click(move |_, _, cx| super::preview::show(&b, cx)))
                .when_some(named, |m, item| m.item(item))
                .separator()
                .item(PopupMenuItem::new(format!("{}{}", tr("Tab schließen"), ShortcutAction::CloseTab.hint())).icon(menu_icon("x")).on_click(move |_, _, cx| close_tab(&c, cx)))
        });
        let actions = div().flex().items_center().gap(px(space::XS)).children(table_toggle).child(more);
        let body: AnyElement = if !self.path.exists() {
            unavailable("file-question", tr("Datei nicht gefunden"), self.path.file_name().unwrap_or_default().to_string_lossy().to_string(), &t).into_any_element()
        } else if let Some(pdf) = &self.pdf {
            pdf.clone().into_any_element()
        } else if let Some(sheet) = &self.sheet {
            sheet.clone().into_any_element()
        } else {
            self.preview.clone().into_any_element()
        };
        div().size_full().flex().flex_col().child(document_bar(&self.path, actions.into_any_element(), &t, window, cx)).child(div().flex_1().min_h_0().child(body))
    }
}

/// Saves or asks about every open note, then quits.
pub fn prepare_to_quit(cx: &mut App) {
    if !state::notes_settled(cx) {
        return;
    }
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
    field.context_menu(|menu, _, cx| {
        let menu = match spell_card::menu_opened(cx) {
            Some(_) => menu.menu(tr("Zum Wörterbuch hinzufügen"), Box::new(AddWordUnderPointer)).separator(),
            None => menu,
        };
        let menu = menu.menu(tr("Ausschneiden"), Box::new(Cut))
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
            .menu(tr("Karte aus Auswahl…"), Box::new(CardFromSelection));
        if super::ai::active() {
            menu.separator().submenu(tr("KI"), gpui_kit::component::native_menu::NativeMenu::new().menu(tr("Frag die KI zu dieser Auswahl"), Box::new(AskAiSelection)).menu(tr("Karten aus Auswahl"), Box::new(CardsAiSelection)).menu(tr("Passage erklären"), Box::new(ExplainAiSelection)))
        } else {
            menu
        }
    })
}

#[cfg(test)]
mod window_tests {
    use super::{Mode, PARSE_IN_BACKGROUND_FROM, WRAP_MAX, WRAP_UNTIL, can_wrap, first_parse_in_background, has_long_line, wraps_note, parses_in_background, save_reason, spell_window, unsaved_message, wraps_lines};

    fn prose() -> String {
        (0..3000).map(|i| format!("Zeile {i} mit etwas Text darin\n")).collect()
    }

    #[test]
    fn a_chosen_wrap_beats_the_size_of_the_note() {
        assert!(wraps_lines(10, None));
        assert!(!wraps_lines(WRAP_UNTIL + 1, None));
        assert!(wraps_lines(WRAP_UNTIL + 1, Some(true)));
        assert!(!wraps_lines(WRAP_MAX + 1, Some(true)));
        assert!(!wraps_lines(10, Some(false)));
    }

    #[test]
    fn a_very_long_line_wraps_unless_the_user_turned_wrapping_off() {
        let long = format!("kurz\n{}\nkurz", "wort ".repeat(20_000));
        assert!(has_long_line(&long) && wraps_note(&long, None) && can_wrap(&long));
        assert!(!wraps_note(&long, Some(false)));
        let short = "wort ".repeat(20_000);
        assert!(!has_long_line("wort\n".repeat(50_000).as_str()) && has_long_line(&short));
        assert!(!wraps_note(&"wort\n".repeat(50_000), None));
    }

    #[test]
    fn a_spell_window_inside_a_long_fenced_block_stays_empty_and_bounded() {
        let code: String = (0..4000).map(|i| format!("let wert_{i} = {i};\n")).collect();
        let prose = prose();
        let text = format!("{prose}```rust\n{code}```\n{prose}");
        let window = spell_window(&text, prose.len() + code.len() / 2);
        assert!(window.is_empty(), "window {window:?} reaches into the block");
    }

    #[test]
    fn a_spell_window_starting_inside_a_fenced_block_moves_past_its_closing_fence() {
        let code: String = (0..2500).map(|i| format!("let wert_{i} = {i};\n")).collect();
        let prose = prose();
        let text = format!("{prose}```rust\n{code}```\n{prose}");
        let caret = prose.len() + code.len() + 10_000;
        let window = spell_window(&text, caret);
        assert!(text[window.start..].starts_with("Zeile"));
        assert!(window.start > prose.len() + code.len());
        assert!(window.len() <= 2 * super::SPELL_WINDOW + 100);
    }

    #[test]
    fn an_editor_that_does_not_wrap_is_as_tall_as_its_lines() {
        let text = format!("{}

{}
", "wort ".repeat(4000), "ä".repeat(900));
        assert_eq!(super::estimated_rows(&text, f32::INFINITY), 3.);
        assert!(super::estimated_rows(&text, 70.) > 60.);
        assert_eq!(super::estimated_rows("", 70.), 0.);
    }

    #[test]
    fn a_spell_window_after_a_closed_block_keeps_its_own_start() {
        let prose = prose();
        let head = "```\ncode\n```\n";
        let text = format!("{head}{prose}{prose}");
        let window = spell_window(&text, text.len() - 10);
        assert!(window.start > head.len());
        assert!(text[window.start..].starts_with("Zeile"));
    }

    #[test]
    fn only_long_notes_in_the_reader_modes_parse_in_the_background() {
        assert!(parses_in_background(Mode::Read, PARSE_IN_BACKGROUND_FROM));
        assert!(parses_in_background(Mode::Split, PARSE_IN_BACKGROUND_FROM + 1));
        assert!(!parses_in_background(Mode::Read, PARSE_IN_BACKGROUND_FROM - 1));
        assert!(!parses_in_background(Mode::Live, PARSE_IN_BACKGROUND_FROM * 10));
        assert!(first_parse_in_background(Mode::Live, PARSE_IN_BACKGROUND_FROM, true));
        assert!(!first_parse_in_background(Mode::Live, PARSE_IN_BACKGROUND_FROM, false));
        assert!(!first_parse_in_background(Mode::Live, PARSE_IN_BACKGROUND_FROM - 1, true));
        assert!(!first_parse_in_background(Mode::Source, PARSE_IN_BACKGROUND_FROM, true));
        assert!(!parses_in_background(Mode::Source, PARSE_IN_BACKGROUND_FROM * 10));
    }

    #[test]
    fn a_save_into_a_vanished_folder_says_the_text_stays_open() {
        let gone = std::env::temp_dir().join("schrumbi-gone-folder").join("sub").join("n.md");
        let message = save_reason(&gone, &std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(message.contains("bleibt offen") && !message.contains("os error"), "{message}");
        let here = std::env::temp_dir().join("n.md");
        let denied = save_reason(&here, &std::io::Error::from_raw_os_error(if cfg!(windows) { 5 } else { 13 }));
        assert!(denied.contains("Berechtigung") && !denied.contains("os error") && !denied.contains("denied"), "{denied}");
    }

    #[test]
    fn the_toast_for_notes_that_stay_open_names_one_and_counts_several() {
        assert_eq!(unsaved_message(&["Analysis".to_string()], true), "„Analysis“ ließ sich nicht sichern und bleibt offen. Erneut versuchen steht oben in der Notiz.");
        assert_eq!(unsaved_message(&["Analysis".to_string()], false), "„Analysis“ ließ sich nicht sichern und bleibt offen.");
        let names: Vec<String> = ["A", "B", "C", "D"].iter().map(|n| n.to_string()).collect();
        assert_eq!(unsaved_message(&names, false), "4 Notizen ließen sich nicht sichern und bleiben offen.");
        assert_eq!(unsaved_message(&names[..2], false), "„A“, „B“ ließen sich nicht sichern und bleiben offen.");
        assert!(unsaved_message(&names[..3], false).starts_with("„A“, „B“, „C“ ließen"));
    }

    #[test]
    fn the_toast_action_saves_a_note_whose_tab_is_not_shown() {
        use crate::core::library::Library;
        let dir = std::env::temp_dir().join(format!("schrumbi-toast-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let note = dir.join("n.md");
        std::fs::write(&note, "alt").unwrap();
        let mut lib = Library::default();
        lib.buffer(&note).edited("neu".into());
        lib.save_problems.push(note.clone());
        super::retry_unsaved(vec![note.clone()])(&mut lib);
        assert_eq!(std::fs::read_to_string(&note).unwrap(), "neu");
        assert!(lib.save_problems.is_empty() && lib.unsaved_shown.contains(&note));
        assert!(lib.toast.is_none());
        std::fs::remove_file(&note).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        lib.buffer(&note).edited("weg".into());
        super::retry_unsaved(vec![note.clone()])(&mut lib);
        let toast = lib.toast.as_ref().expect("a note that still fails is announced again");
        assert!(toast.undo.is_some() && toast.message.contains("bleibt offen"), "{}", toast.message);
    }
}
