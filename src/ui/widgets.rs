use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

use gpui_kit::component::Icon;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::browser::DraggedFile;
use super::motion::{self, Soft};
use super::state::{self, act};
use super::theme::{Theme, font, radius, space, iconsize};
use crate::core::i18n::{tr, trf};
use crate::core::library::{self, Library};
use crate::core::model::{Assignment, Kind, Status};
use crate::core::naming::extension;

/// Digits of equal width, for numbers that change or stand in columns.
pub fn tabular() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".to_string(), 1)]))
}

/// Makes a clickable element reachable with Tab; Enter and Space then click it.
pub trait KeyNav {
    fn key_nav(self) -> Self;
}

impl KeyNav for Stateful<Div> {
    fn key_nav(self) -> Self {
        let ring = rgb(0x0A84FF);
        self.tab_index(0).focus_visible(move |s| s.bg(Hsla::from(ring).opacity(0.28)))
    }
}

/// A Lucide icon from `assets/icons`, tinted like an SF Symbol.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg().path(SharedString::from(format!("icons/{name}.svg"))).size(px(size)).flex_none().text_color(color)
}

/// File-type icon from the Material Icon Theme, as in VS Code.
pub fn file_icon(path: &Path, size: f32) -> Img {
    let name = if path.is_dir() {
        "folder-base"
    } else {
        match extension(&path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).to_lowercase().as_str() {
            "md" | "markdown" | "mdown" => "markdown",
            "pdf" => "pdf",
            "doc" | "docx" | "pages" | "odt" | "rtf" | "rtfd" => "word",
            "ppt" | "pptx" | "key" | "odp" => "powerpoint",
            "xls" | "xlsx" | "numbers" | "ods" | "csv" | "tsv" => "table",
            "png" | "jpg" | "jpeg" | "heic" | "gif" | "webp" | "tiff" | "bmp" => "image",
            "svg" => "svg",
            "mp4" | "mov" | "m4v" | "avi" => "video",
            "mp3" | "m4a" | "wav" | "aiff" => "audio",
            "zip" | "rar" | "7z" | "tar" | "gz" => "zip",
            "json" => "json",
            "html" | "htm" => "html",
            "css" | "scss" => "css",
            "js" | "jsx" | "mjs" => "javascript",
            "ts" | "tsx" => "typescript",
            "swift" => "swift",
            "java" => "java",
            "py" => "python",
            "c" | "h" => "c",
            "cpp" | "hpp" | "cc" => "cpp",
            "yml" | "yaml" => "yaml",
            "tex" | "bib" => "tex",
            "xml" | "plist" => "xml",
            "fig" => "figma",
            "sketch" => "sketch",
            "ttf" | "otf" | "woff" | "woff2" => "font",
            _ => "document",
        }
    };
    img(SharedString::from(format!("fileicons/{name}.svg"))).size(px(size)).flex_none()
}

/// Icon for menus and component buttons.
pub fn menu_icon(name: &str) -> Icon {
    Icon::empty().path(SharedString::from(format!("icons/{name}.svg")))
}

/// How long after a right click tooltips stay away, so none covers the context menu that just opened.
const TOOLTIP_PAUSE: std::time::Duration = std::time::Duration::from_secs(3);

struct LastRightClick(std::time::Instant);

impl Global for LastRightClick {}

struct NoTooltip;

impl Render for NoTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Notes a right click for `tip`; call it from the window's root.
pub fn pause_tooltips(cx: &mut App) {
    cx.set_global(LastRightClick(std::time::Instant::now()));
}

/// Tooltip builder for `.tooltip(...)`.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |window, cx| {
        if cx.try_global::<LastRightClick>().is_some_and(|c| c.0.elapsed() < TOOLTIP_PAUSE) {
            return cx.new(|_| NoTooltip).into();
        }
        gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
    }
}

pub fn text(s: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div().text_size(px(size)).text_color(color).child(s.into())
}

pub fn caption(s: impl Into<SharedString>, t: &Theme) -> Div {
    text(s, font::CAPTION, t.secondary)
}

/// Module dot plus code.
pub fn module_tag(code: &str, color: Hsla) -> Div {
    div().flex().items_center().gap(px(space::XS)).child(div().size(px(space::S)).rounded_full().bg(color)).child(code.to_string())
}

/// Capsule with the module code on a tinted background.
pub fn module_chip(code: &str, color: Hsla) -> Div {
    div().px(px(space::S)).py(px(space::XXS)).rounded_full().bg(color.opacity(0.18)).text_color(color).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).child(code.to_string())
}

/// Left half of a disc, inset like the Swift `Circle().trim(0, 0.5)` in the status glyph.
fn half_disc(color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let c = bounds.center();
            let r = bounds.size.width / 2. - px(3.);
            let mut b = PathBuilder::fill();
            b.move_to(point(c.x, c.y - r));
            for i in 0..=32 {
                let a = -TAU / 4. - TAU / 2. * i as f32 / 32.;
                b.line_to(point(c.x + r * a.cos(), c.y + r * a.sin()));
            }
            b.close();
            if let Ok(p) = b.build() {
                window.paint_path(p, color);
            }
        },
    )
    .absolute()
    .size_full()
}

/// The Things-style completion glyph.
pub fn status_glyph(status: Status, kind: Kind, optional: bool, tint: Hsla, size: f32, t: &Theme) -> AnyElement {
    let base = div().size(px(size)).flex_none().rounded_full().flex().items_center().justify_center();
    let mark = |name: &str, color: Hsla, scale: f32| icon(name, size * scale, color);
    match status {
        Status::Uploaded | Status::Done => base.bg(t.green).child(mark("check", white(), 0.62)).into_any_element(),
        Status::Skipped => base.bg(t.gray.opacity(0.35)).child(mark("minus", white(), 0.62)).into_any_element(),
        Status::Ready => base.border(px(1.6)).border_color(t.blue).child(mark("check", t.blue, 0.58)).into_any_element(),
        Status::InProgress => base.border(px(1.6)).border_color(t.orange).relative().child(half_disc(t.orange)).into_any_element(),
        Status::Overdue => base.border(px(1.8)).border_color(t.red).into_any_element(),
        Status::Open if kind == Kind::Exam => div().size(px(size)).flex().items_center().justify_center().child(mark("graduation-cap", tint, 0.85)).into_any_element(),
        Status::Open => {
            let ring = base.border(px(1.5)).border_color(tint.opacity(0.8));
            if optional { ring.border_dashed() } else { ring }.into_any_element()
        }
    }
}

fn status_help(a: &Assignment, status: Status) -> String {
    if status.is_closed() {
        crate::core::i18n::trf("%@ – klicken zum Zurücksetzen", &[&status.label()])
    } else if a.upload {
        tr("Als hochgeladen markieren")
    } else {
        tr("Als erledigt markieren")
    }
}

/// Clickable completion control that toggles done and stops the click from selecting the row.
pub fn status_toggle(a: &Assignment, lib: &Library, size: f32, t: &Theme) -> Stateful<Div> {
    let status = lib.status(a);
    let tint = t.palette(lib.module(a).map_or("blue", |m| m.color.as_str()));
    let id = a.id.clone();
    let help = status_help(a, status);
    div()
        .id(SharedString::from(format!("toggle-{}", a.id)))
        .flex_none()
        .relative()
        .rounded_full()
        .cursor_pointer().key_nav()
        .child(status_glyph(status, a.kind, a.optional, tint, size, t))
        .child(motion::done_pulse("done", status.is_closed() && status != Status::Skipped, size))
        .tooltip(tip(help))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            act(cx, |l| l.toggle_done(&id));
        })
}

/// The status actions offered in context menus and the inspector's "…" menu.
pub fn status_menu(menu: PopupMenu, id: &str, cx: &App) -> PopupMenu {
    let lib = state::lib(cx);
    let l = lib.read(cx);
    let Some(a) = l.assignment(id).cloned() else { return menu };
    let e = l.evaluation(&a);
    let mut menu = menu;
    let item = |label: String, icon_name: &'static str, f: Box<dyn Fn(&mut App)>| PopupMenuItem::new(label).icon(menu_icon(icon_name)).on_click(move |_, _, cx| f(cx));
    let set = |s: Option<Status>| {
        let id = id.to_string();
        Box::new(move |cx: &mut App| act(cx, |l| l.set_status(s, &id, true))) as Box<dyn Fn(&mut App)>
    };
    if a.upload && !e.status.is_closed() {
        let id2 = id.to_string();
        menu = menu.item(item(tr("Hochladen…"), "file-up", Box::new(move |cx| super::upload::begin(&id2, cx)))).separator();
    }
    menu = if a.upload { menu.item(item(tr("Hochgeladen"), "circle-check", set(Some(Status::Uploaded)))) } else { menu.item(item(tr("Erledigt"), "circle-check", set(Some(Status::Done)))) };
    if a.kind == Kind::Submission {
        menu = menu.item(item(tr("In Arbeit"), "circle-dot", set(Some(Status::InProgress))));
    }
    menu = menu.item(item(tr("Entfällt"), "circle-minus", set(Some(Status::Skipped))));
    if e.is_override {
        menu = menu.separator().item(item(tr("Automatisch erkennen"), "undo-2", set(None)));
    }
    menu = menu.separator();
    if !a.is_manual() {
        let id2 = id.to_string();
        menu = menu.item(item(tr("Termin verschieben…"), "calendar-clock", Box::new(move |cx| act(cx, |l| l.reschedule_id = Some(id2.clone())))));
    }
    if a.tracks_files() {
        let id2 = id.to_string();
        let folder = a.folder.clone();
        menu = menu
            .item(item(tr("Vorhandene Datei zuordnen…"), "link", Box::new(move |cx| super::platform::pick_link_files(&id2, cx))))
            .item(item(tr("Ordner öffnen"), "folder", Box::new(move |_| library::open_path(&folder))));
    }
    menu
}

/// Common file actions for context menus.
pub fn file_menu(menu: PopupMenu, path: PathBuf) -> PopupMenu {
    let (p1, p2, p3) = (path.clone(), path.clone(), path);
    menu.item(PopupMenuItem::new(tr("Öffnen")).on_click(move |_, _, cx| act(cx, |l| l.open_file(&p1))))
        .item(PopupMenuItem::new(tr("Mit Standard-App öffnen")).on_click(move |_, _, _| library::open_path(&p2)))
        .item(PopupMenuItem::new(super::platform::reveal_label()).on_click(move |_, _, _| library::reveal(&p3)))
}

/// Rounded card with a caption header, as the Swift `Tile`.
pub fn tile(id: impl Into<SharedString>, title: impl Into<SharedString>, icon_name: &str, tint: Hsla, clickable: bool, t: &Theme) -> Stateful<Div> {
    let ink = t.ink;
    let bg = t.secondary_bg;
    let card = div()
        .id(id.into())
        .group("tile")
        .flex_1()
        .min_w_0()
        .min_h(px(112.))
        .p(px(space::L))
        .flex()
        .flex_col()
        .gap(px(space::S))
        .rounded(px(radius::CARD))
        .bg(bg)
        .border_1()
        .border_color(ink.opacity(0.1))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::XS))
                .text_size(px(font::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.secondary)
                .child(icon(icon_name, iconsize::XS, tint))
                .child(title.into())
                .when(clickable, |d| d.child(div().flex_1()).child(div().invisible().group_hover("tile", |s| s.visible()).child(icon("chevron-right", iconsize::XS, t.tertiary)))),
        );
    if clickable { card.cursor_pointer().key_nav().soft(ink.opacity(0.04), radius::CARD) } else { card }
}

/// Thin capsule progress bar.
pub fn progress_bar(value: f32, color: Hsla, t: &Theme) -> Div {
    div().h(px(6.)).w_full().rounded_full().bg(t.ink.opacity(0.1)).child(div().h_full().rounded_full().bg(color).w(relative(value.clamp(0., 1.))))
}

fn circle_path(center: Point<Pixels>, r: Pixels, from: f32, to: f32, width: f32) -> Option<gpui_kit::Path<Pixels>> {
    let steps = ((to - from).abs() / TAU * 96.).ceil().max(2.) as usize;
    let mut b = PathBuilder::stroke(px(width));
    for i in 0..=steps {
        let a = from + (to - from) * i as f32 / steps as f32;
        let p = point(center.x + r * a.cos(), center.y + r * a.sin());
        if i == 0 { b.move_to(p) } else { b.line_to(p) }
    }
    b.build().ok()
}

/// Circular progress with the percentage in the middle, as the Swift `ProgressRing`.
pub fn progress_ring(value: f32, size: f32, t: &Theme) -> Div {
    let (track, accent, value) = (t.ink.opacity(0.12), t.accent, value.clamp(0., 1.));
    div()
        .size(px(size))
        .flex_none()
        .relative()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let c = bounds.center();
                    let r = bounds.size.width / 2. - px(3.);
                    if let Some(p) = circle_path(c, r, 0., TAU, 6.) {
                        window.paint_path(p, track);
                    }
                    if value > 0. {
                        let start = -TAU / 4.;
                        let end = start + TAU * value;
                        if let Some(p) = circle_path(c, r, start, end, 6.) {
                            window.paint_path(p, accent);
                        }
                        for a in [start, end] {
                            if let Some(p) = circle_path(point(c.x + r * a.cos(), c.y + r * a.sin()), px(1.5), 0., TAU, 3.) {
                                window.paint_path(p, accent);
                            }
                        }
                    }
                },
            )
            .absolute()
            .size_full(),
        )
        .child(
            div()
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(font::CAPTION2))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{}%", (value * 100.).round() as i32)),
        )
}

/// Container with the secondary background, a hairline border and clipped rounded corners.
pub fn grouped(t: &Theme) -> Div {
    div().flex().flex_col().rounded(px(radius::CARD)).bg(t.secondary_bg).border_1().border_color(t.separator.opacity(0.6)).overflow_hidden()
}

pub fn divider(t: &Theme) -> Div {
    div().h(px(1.)).w_full().bg(t.separator)
}

/// Tinted banner with a hairline border, used for hints and attention groups.
pub fn banner(tint: Hsla) -> Div {
    div().p(px(space::L)).rounded(px(radius::CARD)).bg(tint.opacity(0.08)).border_1().border_color(tint.opacity(0.25))
}

/// Section heading in title3 weight.
pub fn section_title(s: impl Into<SharedString>) -> Div {
    div().text_size(px(font::TITLE3)).font_weight(FontWeight::SEMIBOLD).child(s.into())
}

/// Empty-state placeholder like SwiftUI's `ContentUnavailableView`.
pub fn unavailable(icon_name: &str, title: impl Into<SharedString>, description: impl Into<SharedString>, t: &Theme) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(space::S))
        .p(px(space::XL))
        .child(icon(icon_name, iconsize::HERO, t.secondary))
        .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::BOLD).child(title.into()))
        .child(div().max_w(px(320.)).text_center().text_color(t.secondary).child(description.into()))
}

/// The label of a plain row button with hover feedback.
pub fn row_button(id: impl Into<ElementId>, t: &Theme) -> Stateful<Div> {
    let hover = t.ink.opacity(0.06);
    div().id(id).w_full().cursor_pointer().key_nav().soft(hover, 0.)
}

/// Compact text button styled like `.borderless` / `.link` buttons.
pub fn link_button(id: impl Into<ElementId>, label: impl Into<SharedString>, t: &Theme) -> Stateful<Div> {
    let accent = t.accent;
    div().id(id).text_color(accent).cursor_pointer().key_nav().hover(move |s| s.text_color(accent.opacity(0.8))).child(label.into())
}

/// Keyboard-shortcut chip.
pub fn kbd(s: impl Into<SharedString>, t: &Theme) -> Div {
    div().px(px(space::XS)).py(px(space::XXS)).rounded(px(radius::SMALL)).bg(t.ink.opacity(0.08)).text_size(px(font::CAPTION)).text_color(t.secondary).child(s.into())
}

/// Remaining time as the Swift `DueLabel` shows it.
pub fn due_label(lib: &Library, a: &Assignment, t: &Theme) -> Div {
    let e = lib.evaluation(a);
    let soon = (a.due - lib.now).num_seconds() < 86400 && a.due > lib.now;
    let (text, color) = if e.status.is_closed() {
        (e.status.label(), t.secondary)
    } else if e.status == Status::Ready && lib.needs_action(a) {
        (tr("Upload bestätigen"), t.blue)
    } else if lib.needs_action(a) {
        (crate::core::day::countdown(a.due, lib.now), t.red)
    } else {
        (crate::core::day::compact(a.due, lib.now), if soon { t.orange } else { t.secondary })
    };
    div().text_size(px(font::CALLOUT)).text_color(color).child(text)
}

/// One assignment as a list row: status, title, module, week, date and file hints.
pub fn assignment_row(lib: &Library, a: &Assignment, show_module: bool, show_date: bool, show_week: bool, t: &Theme) -> impl IntoElement + use<> {
    let e = lib.evaluation(a);
    let module = lib.module(a);
    let tint = t.palette(module.map_or("blue", |m| m.color.as_str()));
    let (menu_id, drop_id, sel) = (a.id.clone(), a.id.clone(), a.id.clone());
    let title = div().font_weight(FontWeight::MEDIUM).text_color(if e.status.is_closed() { t.secondary } else { t.primary }).child(a.title.clone());
    let mut meta = div().flex().items_center().gap(px(space::S)).text_size(px(font::CAPTION)).text_color(t.secondary);
    if show_module {
        meta = meta.child(module_tag(module.map_or("?", |m| m.code.as_str()), tint));
    }
    if let (true, Some(w)) = (show_week, a.week) {
        meta = meta.child(format!("W{w:02}"));
    }
    meta = meta.child(if show_date { crate::core::day::due(a.due, lib.now) } else { crate::core::day::hm(a.due) });
    if !e.files.is_empty() {
        meta = meta.child(div().flex().items_center().gap(px(space::XXS)).child(icon("file", iconsize::XS, t.secondary)).child(e.files.len().to_string()));
    } else if !e.briefs.is_empty() && !e.status.is_closed() {
        meta = meta.child(div().id(SharedString::from(format!("brief-{}", a.id))).flex().items_center().gap(px(space::XXS)).child(icon("clipboard-list", iconsize::XS, t.secondary)).child(tr("Aufgabe da")).tooltip(tip(tr("Aufgabenstellung liegt im Ordner, Abgabe fehlt noch"))));
    }
    if !e.misnamed.is_empty() {
        meta = meta.child(div().id(SharedString::from(format!("mis-{}", a.id))).child(icon("triangle-alert", iconsize::XS, t.orange)).tooltip(tip(tr("Dateiname entspricht nicht der Konvention"))));
    }
    let tracks = a.tracks_files();
    div()
        .id(SharedString::from(format!("row-{}", a.id)))
        .relative()
        .flex()
        .items_center()
        .gap(px(space::M))
        .py(px(space::XS))
        .cursor_pointer().key_nav()
        .on_click(move |_, _, cx| act(cx, |l| l.select(Some(sel.clone()))))
        .child(status_toggle(a, lib, iconsize::M, t))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(space::XS))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .when(a.kind != Kind::Submission, |d| d.child(icon(a.kind.icon(), iconsize::XS, t.secondary)))
                        .child(if e.status == Status::Skipped { title.line_through() } else { title })
                        .when(a.optional && !e.status.is_closed(), |d| d.child(div().px(px(space::XS)).py(px(space::XXS)).rounded_full().bg(t.ink.opacity(0.08)).text_size(px(font::CAPTION2)).text_color(t.secondary).child(tr("optional")))),
                )
                .child(meta),
        )
        .child(due_label(lib, a, t))
        .when(tracks, |c| file_target(c, a, t))
        .context_menu(move |menu, _, cx| status_menu(menu, &menu_id, cx))
}

/// Lets an assignment take files from outside and from the file list; call it after the last child, the element must be relative.
pub fn file_target(element: Stateful<Div>, a: &Assignment, t: &Theme) -> Stateful<Div> {
    let group = SharedString::from(format!("drop-{}", a.id));
    let (outside, inside) = (a.id.clone(), a.id.clone());
    element
        .group(group.clone())
        .on_drop(move |p: &ExternalPaths, _, cx| {
            let paths = p.paths().to_vec();
            act(cx, |l| l.request_filing(paths, Some(outside.clone())));
        })
        .on_drop(move |f: &DraggedFile, _, cx| {
            let path = f.0.clone();
            act(cx, |l| l.request_filing(vec![path], Some(inside.clone())));
        })
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right_0()
                .invisible()
                .hover(|s| s)
                .group_drag_over::<ExternalPaths>(group.clone(), |s| s.visible())
                .group_drag_over::<DraggedFile>(group, |s| s.visible())
                .flex()
                .items_center()
                .justify_center()
                .px(px(space::S))
                .rounded(px(radius::CONTROL))
                .bg(t.accent)
                .text_color(white())
                .text_size(px(font::CALLOUT))
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().truncate().child(trf("In %@ ablegen", &[&a.title]))),
        )
}
