use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::{self, act};
use super::theme::{Theme, font};
use super::widgets::*;
use crate::core::i18n::tr;
use crate::core::library;
use crate::core::markdown;
use crate::core::naming::extension;
use crate::core::pdf_render::PdfDoc;

const IMAGES: [&str; 9] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "svg", "heic"];
const TEXTS: [&str; 14] = ["txt", "csv", "json", "xml", "yml", "yaml", "css", "js", "ts", "swift", "java", "py", "c", "html"];

enum Content {
    Loading,
    Markdown(Arc<super::markdown_view::ParsedNote>),
    Text(String),
    Image(PathBuf),
    Pdf(Vec<(Arc<Image>, f32)>),
    Other,
}

/// Read-only preview of one file: Markdown, text, images and PDF pages; anything else offers its default app.
pub struct FilePreview {
    path: Option<PathBuf>,
    content: Content,
    load: Option<Task<()>>,
    /// Shows the header with title, path and actions.
    pub header: bool,
}

fn ext(path: &Path) -> String {
    extension(&path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).to_lowercase()
}

impl FilePreview {
    pub fn new(header: bool) -> Self {
        Self { path: None, content: Content::Other, load: None, header }
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    /// Shows `path`, loading it off the main thread.
    pub fn set_path(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.path == path {
            return;
        }
        self.path = path.clone();
        let Some(path) = path else {
            self.content = Content::Other;
            cx.notify();
            return;
        };
        let e = ext(&path);
        if IMAGES.contains(&e.as_str()) {
            self.content = Content::Image(path);
            cx.notify();
            return;
        }
        self.content = Content::Loading;
        cx.notify();
        let task = cx.background_spawn(async move {
            if markdown::is_markdown(&path) {
                Content::Markdown(Arc::new(super::markdown_view::parse(&std::fs::read_to_string(&path).unwrap_or_default())))
            } else if TEXTS.contains(&e.as_str()) {
                Content::Text(std::fs::read_to_string(&path).map(|s| s.chars().take(200_000).collect()).unwrap_or_default())
            } else if e == "pdf" {
                match PdfDoc::open(&path) {
                    Some(doc) => Content::Pdf((0..doc.page_count().min(60)).filter_map(|i| Some((Arc::new(Image::from_bytes(ImageFormat::Png, doc.render_png(i, 2.)?)), doc.page_size(i).map_or(0.707, |(w, h)| w / h.max(1.))))).collect()),
                    None => Content::Other,
                }
            } else {
                Content::Other
            }
        });
        self.load = Some(cx.spawn(async move |this, cx| {
            let content = task.await;
            let _ = this.update(cx, |p, cx| {
                p.content = content;
                cx.notify();
            });
        }));
    }
}

impl Render for FilePreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let Some(path) = self.path.clone() else {
            return unavailable("eye", tr("Keine Datei ausgewählt"), tr("Die Vorschau zeigt die ausgewählte Datei. ↩ öffnet sie als Tab."), &t).into_any_element();
        };
        let (title, folder) = state::read(cx, |l| (l.note_title(&path), path.parent().map(|p| l.relative_path(p).replace('/', " › ")).unwrap_or_default()));
        let (reveal, open, default_app) = (path.clone(), path.clone(), path.clone());
        let body: AnyElement = match &self.content {
            Content::Loading => div().size_full().flex().items_center().justify_center().child(icon("loader", 18., t.secondary)).into_any_element(),
            Content::Markdown(text) => div()
                .id("preview-md")
                .size_full()
                .overflow_y_scroll()
                .child(div().w_full().flex().justify_center().child(div().w_full().max_w(px(680.)).p(px(24.)).child(super::markdown_view::render(text, self.path.as_deref().and_then(Path::parent), None, &t))))
                .into_any_element(),
            Content::Text(text) => div().id("preview-text").size_full().overflow_y_scroll().p(px(24.)).font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(text.clone()).into_any_element(),
            Content::Image(p) => div().size_full().p(px(16.)).flex().items_center().justify_center().child(img(p.clone()).max_w_full().max_h_full().object_fit(ObjectFit::Contain)).into_any_element(),
            Content::Pdf(pages) => div()
                .id("preview-pdf")
                .size_full()
                .overflow_y_scroll()
                .bg(t.ink.opacity(0.04))
                .child(div().flex().flex_col().items_center().gap(px(12.)).p(px(16.)).children(pages.iter().map(|(p, ratio)| {
                    let mut page = div().w_full().max_w(px(820.)).flex_none().shadow_md();
                    page.style().aspect_ratio = Some(*ratio);
                    page.child(img(p.clone()).size_full())
                })))
                .into_any_element(),
            Content::Other => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .child(file_icon(&path, 64.))
                .child(div().text_color(t.secondary).child(path.file_name().unwrap_or_default().to_string_lossy().to_string()))
                .child(Button::new("open-default").label(tr("Mit Standard-App öffnen")).on_click(move |_, _, _| library::open_path(&default_app)))
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.content)
            .when(self.header, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(14.))
                        .py(px(9.))
                        .border_b_1()
                        .border_color(t.separator)
                        .child(file_icon(&path, 20.))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(1.)).child(div().font_weight(FontWeight::BOLD).truncate().child(title)).child(div().text_size(px(font::CAPTION)).text_color(t.secondary).truncate().child(folder)))
                        .child(Button::new("preview-reveal").ghost().small().icon(menu_icon("folder")).tooltip(super::platform::reveal_label()).on_click(move |_, _, _| library::reveal(&reveal)))
                        .child(Button::new("preview-open").primary().small().label(tr("Öffnen")).tooltip(tr("Als Tab öffnen (↩ in der Suche, Doppelklick in der Liste)")).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open)))),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}

/// Opens a floating preview window for `path`, like Quick Look (Space in lists).
pub fn show(path: &Path, cx: &mut App) {
    show_in(vec![path.to_path_buf()], 0, cx);
}

/// Opens the preview on one of `files`; the arrow keys move through them.
pub fn show_in(files: Vec<PathBuf>, index: usize, cx: &mut App) {
    let Some(path) = files.get(index).cloned() else { return };
    let title = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let bounds = Bounds::centered(None, size(px(820.), px(900.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some(title.clone().into()), appears_transparent: false, traffic_light_position: None }),
        kind: WindowKind::Floating,
        ..Default::default()
    };
    let options = super::chrome::window_options(options, title);
    let _ = gpui_kit::open_window(options, cx, move |_, cx| {
        let preview = cx.new(|cx| {
            let mut p = FilePreview::new(true);
            p.set_path(Some(path), cx);
            p
        });
        cx.new(|cx| PreviewWindow { files, index, preview, focus: cx.focus_handle(), focused: false })
    });
}

/// The floating preview: the file preview below the window's title bar.
struct PreviewWindow {
    files: Vec<PathBuf>,
    index: usize,
    preview: Entity<FilePreview>,
    focus: FocusHandle,
    focused: bool,
}

impl PreviewWindow {
    fn title(&self) -> String {
        self.files[self.index].file_name().unwrap_or_default().to_string_lossy().to_string()
    }

    fn step(&mut self, delta: i64, window: &mut Window, cx: &mut Context<Self>) {
        let next = (self.index as i64 + delta).clamp(0, self.files.len() as i64 - 1) as usize;
        if next != self.index {
            self.index = next;
            let path = self.files[next].clone();
            self.preview.update(cx, |p, cx| p.set_path(Some(path), cx));
            window.set_window_title(&self.title());
            cx.notify();
        }
    }
}

impl Render for PreviewWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !std::mem::replace(&mut self.focused, true) {
            window.focus(&self.focus, cx);
        }
        let t = Theme::get(cx);
        let content = div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|p, e: &KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
                "escape" | "space" => window.remove_window(),
                "left" | "up" => p.step(-1, window, cx),
                "right" | "down" => p.step(1, window, cx),
                _ => {}
            }))
            .child(self.preview.clone());
        super::chrome::frame(self.title(), content, &t)
    }
}
