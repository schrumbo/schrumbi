use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::text::TextView;
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
    Markdown(String),
    Text(String),
    Image(PathBuf),
    Pdf(Vec<Arc<Image>>),
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
                Content::Markdown(std::fs::read_to_string(&path).unwrap_or_default())
            } else if TEXTS.contains(&e.as_str()) {
                Content::Text(std::fs::read_to_string(&path).map(|s| s.chars().take(200_000).collect()).unwrap_or_default())
            } else if e == "pdf" {
                match PdfDoc::open(&path) {
                    Some(doc) => Content::Pdf((0..doc.page_count().min(60)).filter_map(|i| doc.render_png(i, 2.)).map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png))).collect()),
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
                .child(div().w_full().flex().justify_center().child(div().w_full().max_w(px(680.)).p(px(24.)).child(TextView::markdown("preview-markdown", text.clone()).selectable(true))))
                .into_any_element(),
            Content::Text(text) => div().id("preview-text").size_full().overflow_y_scroll().p(px(24.)).font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(text.clone()).into_any_element(),
            Content::Image(p) => div().size_full().p(px(16.)).flex().items_center().justify_center().child(img(p.clone()).max_w_full().max_h_full().object_fit(ObjectFit::Contain)).into_any_element(),
            Content::Pdf(pages) => div()
                .id("preview-pdf")
                .size_full()
                .overflow_y_scroll()
                .bg(t.ink.opacity(0.04))
                .child(div().flex().flex_col().items_center().gap(px(12.)).p(px(16.)).children(pages.iter().map(|p| img(p.clone()).w_full().max_w(px(820.)).shadow_md())))
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
    let path = path.to_path_buf();
    let bounds = Bounds::centered(None, size(px(820.), px(900.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some(path.file_name().unwrap_or_default().to_string_lossy().to_string().into()), appears_transparent: false, traffic_light_position: None }),
        kind: WindowKind::Floating,
        ..Default::default()
    };
    let _ = gpui_kit::open_window(options, cx, move |_, cx| {
        cx.new(|cx| {
            let mut p = FilePreview::new(true);
            p.set_path(Some(path), cx);
            p
        })
    });
}
