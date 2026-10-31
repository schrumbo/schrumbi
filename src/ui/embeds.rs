use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::loading;
use super::state::act;
use super::theme::{Theme, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::vault::{bases, canvas, index, query, render};

/// A finished picture from text: formula or diagram.
#[derive(Clone)]
enum Painted {
    Busy,
    Done(Arc<Image>, f32, f32),
    Failed(String),
}

thread_local! {
    static PICTURES: RefCell<HashMap<(String, bool), Painted>> = RefCell::new(HashMap::new());
}

/// How an SVG's stated size reads in points.
fn svg_size(svg: &str) -> (f32, f32) {
    let attribute = |name: &str| -> Option<f32> {
        let start = svg.find(&format!("{name}=\""))? + name.len() + 2;
        let end = svg[start..].find('"')? + start;
        svg[start..end].trim_end_matches("pt").trim_end_matches("px").parse().ok()
    };
    (attribute("width").unwrap_or(400.), attribute("height").unwrap_or(240.))
}

/// The cached picture, started in the background on first sight.
fn picture(key: String, dark: bool, make: impl FnOnce() -> Result<(String, f32, f32), String> + Send + 'static, cx: &mut App) -> Painted {
    let slot = (key, dark);
    if let Some(p) = PICTURES.with(|c| c.borrow().get(&slot).cloned()) {
        return p;
    }
    PICTURES.with(|c| c.borrow_mut().insert(slot.clone(), Painted::Busy));
    let task = cx.background_spawn(async move { make() });
    cx.spawn(async move |cx| {
        let result = task.await;
        let painted = match result {
            Ok((svg, w, h)) => Painted::Done(Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes())), w, h),
            Err(e) => Painted::Failed(e),
        };
        PICTURES.with(|c| c.borrow_mut().insert(slot, painted));
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
    Painted::Busy
}

#[derive(IntoElement)]
struct Formula {
    source: String,
    id: u64,
}

impl RenderOnce for Formula {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let color = if t.dark { "#EDEDED" } else { "#1F1F1F" };
        let source = self.source.clone();
        let painted = picture(format!("math:{source}"), t.dark, move || render::math_svg(&source, true, color), cx);
        let body: AnyElement = match painted {
            Painted::Done(image, w, h) => img(image).w(px(w)).h(px(h)).flex_none().into_any_element(),
            Painted::Busy => div().h(px(32.)).child(loading(iconsize::S, t.tertiary)).into_any_element(),
            Painted::Failed(e) => source_with_error(&self.source, &e, &t).into_any_element(),
        };
        div().id(("md-math", self.id)).w_full().overflow_x_scroll().flex().justify_center().py(px(space::XS)).child(body)
    }
}

fn source_with_error(source: &str, error: &str, t: &Theme) -> Div {
    div().w_full().flex().flex_col().gap(px(space::XS)).child(div().p(px(space::S)).rounded(px(radius::INNER)).bg(t.secondary_bg).font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(source.to_string())).child(div().flex().items_center().gap(px(space::XS)).child(icon("triangle-alert", iconsize::XS, t.orange)).child(caption(error.to_string(), t)))
}

pub fn math(source: &str, id: u64, _: &Theme) -> AnyElement {
    Formula { source: source.to_string(), id }.into_any_element()
}

#[derive(IntoElement)]
struct Diagram {
    code: String,
    id: u64,
}

impl RenderOnce for Diagram {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let (code, dark) = (self.code.clone(), t.dark);
        let painted = picture(format!("mermaid:{code}"), dark, move || render::mermaid_svg(&code, dark).map(|svg| {
            let (w, h) = svg_size(&svg);
            (svg, w, h)
        }), cx);
        let links = render::mermaid_links(&self.code);
        let body: AnyElement = match painted {
            Painted::Done(image, w, h) => {
                let scale = (680. / w).min(1.);
                let shown = image.clone();
                div()
                    .id(("md-diagram-open", self.id))
                    .cursor_pointer()
                    .child(img(image).w(px(w * scale)).h(px(h * scale)))
                    .on_click(move |_, window, cx| open_large(shown.clone(), w, h, window, cx))
                    .into_any_element()
            }
            Painted::Busy => div().h(px(120.)).flex().items_center().child(loading(iconsize::M, t.tertiary)).into_any_element(),
            Painted::Failed(e) => source_with_error(&self.code, &e, &t).into_any_element(),
        };
        grouped(&t)
            .p(px(space::M))
            .items_center()
            .child(body)
            .when(!links.is_empty(), |d| {
                d.child(div().w_full().pt(px(space::S)).flex().flex_wrap().gap(px(space::S)).children(links.into_iter().enumerate().map(|(i, name)| {
                    let (label, target) = (name.clone(), name);
                    link_button(("md-diagram-link", (self.id as usize) * 1000 + i), label, &t).text_size(px(font::CALLOUT)).on_click(move |_, _, cx| {
                        let destination = target.clone();
                        act(cx, |l| {
                            l.open_link(&destination, None);
                        });
                    })
                })))
            })
    }
}

fn open_large(image: Arc<Image>, w: f32, h: f32, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt;
    window.open_dialog(cx, move |d, _, _| {
        let image = image.clone();
        d.w(px(900.)).p_0().child(div().id("diagram-large").max_h(px(560.)).overflow_scroll().p(px(space::L)).flex().justify_center().child(img(image).w(px(w * 1.4)).h(px(h * 1.4)).flex_none()))
    });
}

/// Code blocks that are really something else. `None` means ordinary code.
pub fn special_block(lang: Option<&str>, code: &str, base: Option<&Path>, id: u64, t: &Theme) -> Option<AnyElement> {
    let lang = lang?.to_lowercase();
    match lang.as_str() {
        "mermaid" => Some(Diagram { code: code.to_string(), id }.into_any_element()),
        "math" | "latex" if false => None,
        "query" => Some(QueryBlock { query: code.trim().to_string(), id }.into_any_element()),
        "base" => Some(BaseBlock { source: code.to_string(), this: base.map(|b| b.join(".")), id }.into_any_element()),
        "dataview" | "dataviewjs" | "tasks" | "excalidraw" | "kanban" | "tracker" | "chart" | "ad-note" => Some(plugin_block(&lang, code, id, t)),
        _ => None,
    }
}

fn plugin_block(plugin: &str, code: &str, id: u64, t: &Theme) -> AnyElement {
    let name = plugin.to_string();
    let _ = id;
    div()
        .flex()
        .flex_col()
        .gap(px(space::XS))
        .child(div().p(px(space::M)).rounded(px(radius::CONTROL)).bg(t.secondary_bg).font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(code.to_string()))
        .child(caption(trf("Wird vom Obsidian-Plugin %@ dargestellt.", &[&name]), t))
        .into_any_element()
}

#[derive(IntoElement)]
struct QueryBlock {
    query: String,
    id: u64,
}

thread_local! {
    static QUERIES: RefCell<HashMap<(String, u64), Result<Vec<PathBuf>, String>>> = RefCell::new(HashMap::new());
}

impl RenderOnce for QueryBlock {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let vault = index::current();
        let key = (self.query.clone(), vault.generation);
        let known = QUERIES.with(|q| q.borrow().get(&key).cloned());
        let result = known.unwrap_or_else(|| {
            let parsed = query::parse(&self.query);
            let (slot, worker) = (key.clone(), vault.clone());
            QUERIES.with(|q| q.borrow_mut().insert(slot.clone(), Ok(Vec::new())));
            let task = cx.background_spawn(async move { parsed.map(|q| query::search(&q, &worker, query::Sort::NameAsc)) });
            cx.spawn(async move |cx| {
                let found = task.await;
                QUERIES.with(|q| q.borrow_mut().insert(slot, found));
                cx.update(|cx| cx.refresh_windows());
            })
            .detach();
            Ok(Vec::new())
        });
        let head = div().px(px(space::M)).py(px(space::XS)).flex().items_center().gap(px(space::XS)).bg(t.secondary_bg).text_size(px(font::CAPTION)).text_color(t.secondary).child(icon("search", iconsize::XS, t.tertiary)).child(div().truncate().child(self.query.clone()));
        let body: AnyElement = match result {
            Err(e) => div().p(px(space::S)).child(caption(e, &t)).into_any_element(),
            Ok(files) if files.is_empty() => div().p(px(space::M)).child(caption(tr("Keine Treffer"), &t)).into_any_element(),
            Ok(files) => div()
                .flex()
                .flex_col()
                .py(px(space::XS))
                .children(files.into_iter().take(50).enumerate().map(|(i, f)| {
                    let open = f.clone();
                    let name = f.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    let hover = t.ink.opacity(0.05);
                    div().id(("query-hit", (self.id as usize) * 1000 + i)).px(px(space::M)).py(px(space::XS)).flex().items_center().gap(px(space::S)).cursor_pointer().hover(move |s| s.bg(hover)).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open))).child(file_icon(&f, iconsize::S)).child(div().truncate().child(name))
                }))
                .into_any_element(),
        };
        grouped(&t).child(head).child(body)
    }
}

#[derive(IntoElement)]
struct BaseBlock {
    source: String,
    this: Option<PathBuf>,
    id: u64,
}

impl RenderOnce for BaseBlock {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let vault = index::current();
        let table = bases::Base::parse(&self.source).map_err(|e| e).and_then(|base| {
            let view = base.views.first().cloned().unwrap_or_default();
            base.rows(&view, &vault, self.this.as_deref(), chrono::Local::now()).map(|rows| (base.columns(&view).iter().map(|c| base.column_label(c)).collect::<Vec<_>>(), rows))
        });
        let body: AnyElement = match table {
            Err(e) => notice("triangle-alert", t.orange, tr("Diese Ansicht lässt sich nicht lesen."), Some(e), div(), &t).into_any_element(),
            Ok((columns, rows)) if rows.is_empty() => {
                let _ = columns;
                div().p(px(space::M)).child(caption(tr("Keine Notizen passen zu diesem Filter"), &t)).into_any_element()
            }
            Ok((columns, rows)) => {
                let head = div().flex().bg(t.secondary_bg).font_weight(FontWeight::SEMIBOLD).text_size(px(font::CALLOUT)).children(columns.iter().map(|c| div().flex_1().min_w_0().truncate().px(px(space::S)).py(px(space::XS)).child(c.clone())));
                let lines = rows.into_iter().take(100).enumerate().map(|(i, row)| {
                    let open = row.path.clone();
                    let hover = t.ink.opacity(0.05);
                    div().id(("base-row", (self.id as usize) * 1000 + i)).flex().border_t_1().border_color(t.separator).cursor_pointer().hover(move |s| s.bg(hover)).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open))).children(row.cells.iter().map(|(_, v)| div().flex_1().min_w_0().truncate().px(px(space::S)).py(px(space::XS)).text_size(px(font::CALLOUT)).child(v.display())))
                });
                div().id(("base-scroll", self.id)).max_h(px(400.)).overflow_y_scroll().child(head).children(lines).into_any_element()
            }
        };
        grouped(&t).child(body)
    }
}

/// `![[x.base]]`.
pub fn base_file(path: PathBuf, id: u64, t: &Theme) -> AnyElement {
    let source = std::fs::read_to_string(&path).unwrap_or_default();
    let open = path.clone();
    let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    div()
        .flex()
        .flex_col()
        .gap(px(space::XS))
        .child(div().flex().justify_between().items_center().child(caption(name, t)).child(link_button(("base-open", id), tr("Öffnen"), t).text_size(px(font::CAPTION)).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open)))))
        .child(BaseBlock { source, this: Some(path), id })
        .into_any_element()
}

/// A page of a PDF as an image, loaded in the background and shared by everything that shows it.
pub fn pdf_page(path: &std::path::Path, page: usize, cx: &mut App) -> Option<Arc<Image>> {
    let key = (path.to_path_buf(), page);
    let known = PAGES.with(|p| p.borrow().get(&key).cloned());
    if known.is_none() {
        PAGES.with(|p| p.borrow_mut().insert(key.clone(), None));
        let path = path.to_path_buf();
        let task = cx.background_spawn(async move { crate::core::pdf_render::page_png(&path, page.saturating_sub(1), 2.) });
        cx.spawn(async move |cx| {
            let png = task.await;
            PAGES.with(|p| p.borrow_mut().insert(key, png.map(|b| Arc::new(Image::from_bytes(ImageFormat::Png, b)))));
            cx.update(|cx| cx.refresh_windows());
        })
        .detach();
    }
    known.flatten()
}

/// `![[x.canvas]]`: the cards as shapes, as Obsidian does; a click opens it.
pub fn canvas_file(path: PathBuf, id: u64, t: &Theme) -> AnyElement {
    let doc = std::fs::read_to_string(&path).ok().and_then(|s| canvas::Canvas::parse(&s)).unwrap_or_default();
    let Some((x0, y0, x1, y1)) = doc.bounds() else { return div().p(px(space::M)).child(caption(tr("Leeres Canvas"), t)).into_any_element() };
    let (width, height) = ((x1 - x0).max(1.) as f32, (y1 - y0).max(1.) as f32);
    let scale = (640_f32 / width).min(240. / height).min(1.);
    let open = path.clone();
    let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let tint = t.ink.opacity(0.12);
    let border = t.ink.opacity(0.25);
    grouped(t)
        .child(
            div()
                .id(("canvas-embed", id))
                .cursor_pointer()
                .on_click(move |_, _, cx| act(cx, |l| l.open_file(&open)))
                .child(div().px(px(space::M)).py(px(space::XS)).bg(t.secondary_bg).text_size(px(font::CAPTION)).text_color(t.secondary).child(name))
                .child(div().p(px(space::M)).flex().justify_center().child(div().relative().w(px(width * scale)).h(px(height * scale)).children(doc.nodes.iter().map(|n| {
                    div().absolute().left(px((n.x - x0) as f32 * scale)).top(px((n.y - y0) as f32 * scale)).w(px(n.width as f32 * scale)).h(px(n.height as f32 * scale)).rounded(px(radius::SMALL)).bg(tint).border_1().border_color(border)
                })))),
        )
        .into_any_element()
}

/// `![[a.pdf#page=3&height=400]]`: shows the page; a click opens the document there.
pub fn pdf(path: PathBuf, query: &str, id: u64, t: &Theme) -> AnyElement {
    let setting = |name: &str| query.split('&').find_map(|p| p.strip_prefix(&format!("{name}=")).and_then(|v| v.parse::<usize>().ok()));
    let page = setting("page").unwrap_or(1).max(1);
    let height = setting("height").unwrap_or(400) as f32;
    PdfEmbed { path, page, height, id, _dark: t.dark }.into_any_element()
}

#[derive(IntoElement)]
struct PdfEmbed {
    path: PathBuf,
    page: usize,
    height: f32,
    id: u64,
    _dark: bool,
}

thread_local! {
    static PAGES: RefCell<HashMap<(PathBuf, usize), Option<Arc<Image>>>> = RefCell::new(HashMap::new());
}

impl RenderOnce for PdfEmbed {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let key = (self.path.clone(), self.page);
        let known = PAGES.with(|p| p.borrow().get(&key).cloned());
        if known.is_none() {
            PAGES.with(|p| p.borrow_mut().insert(key.clone(), None));
            let (path, page) = (self.path.clone(), self.page);
            let task = cx.background_spawn(async move { crate::core::pdf_render::page_png(&path, page - 1, 2.) });
            cx.spawn(async move |cx| {
                let png = task.await;
                PAGES.with(|p| p.borrow_mut().insert(key, png.map(|b| Arc::new(Image::from_bytes(ImageFormat::Png, b)))));
                cx.update(|cx| cx.refresh_windows());
            })
            .detach();
        }
        let name = self.path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let open = self.path.clone();
        let page = self.page;
        let image = known.flatten();
        grouped(&t)
            .id(("pdf-embed", self.id))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                let at = open.clone();
                let position = crate::core::prefs::PdfPosition { page: page - 1, fraction: 0., scale: None, fit_page: false, used: chrono::Local::now().timestamp() };
                crate::core::prefs::update(|p| {
                    p.pdf_positions.insert(at.to_string_lossy().to_string(), position);
                });
                act(cx, |l| l.open_file(&at));
            })
            .child(div().px(px(space::M)).py(px(space::XS)).flex().items_center().gap(px(space::S)).bg(t.secondary_bg).text_size(px(font::CAPTION)).text_color(t.secondary).child(file_icon(&self.path, iconsize::S)).child(div().flex_1().truncate().child(name)).child(div().font_features(tabular()).child(trf("Seite %@", &[&page.to_string()]))))
            .child(div().h(px(self.height)).flex().items_center().justify_center().bg(t.secondary_bg).child(match image {
                Some(i) => img(i).h_full().object_fit(ObjectFit::Contain).into_any_element(),
                None => loading(iconsize::M, t.tertiary).into_any_element(),
            }))
    }
}

/// One line for an audio file: play, pause, position.
pub fn audio(path: PathBuf, id: u64, _: &Theme) -> AnyElement {
    AudioLine { path, id }.into_any_element()
}

#[derive(IntoElement)]
struct AudioLine {
    path: PathBuf,
    id: u64,
}

impl RenderOnce for AudioLine {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let playing = super::audio::is_playing(&self.path);
        let (position, total) = super::audio::progress(&self.path);
        if playing {
            window.request_animation_frame();
        }
        let name = self.path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let toggle = self.path.clone();
        let time = |d: Duration| format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60);
        let fraction = total.map_or(0., |t| if t.as_secs_f32() > 0. { position.as_secs_f32() / t.as_secs_f32() } else { 0. });
        let seek = self.path.clone();
        let bar = window.use_keyed_state(("audio-bar", self.id), cx, |_, _| std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()))).read(cx).clone();
        let measured = bar.clone();
        div()
            .px(px(space::M))
            .py(px(space::S))
            .flex()
            .items_center()
            .gap(px(space::M))
            .rounded(px(radius::CONTROL))
            .border_1()
            .border_color(t.separator)
            .child(
                div()
                    .id(("audio-toggle", self.id))
                    .size(px(28.))
                    .flex_none()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(t.ink.opacity(0.08))
                    .cursor_pointer()
                    .key_nav()
                    .child(icon(if playing { "pause" } else { "play" }, iconsize::S, t.primary))
                    .tooltip(tip(if playing { tr("Pause") } else { tr("Abspielen") }))
                    .on_click(move |_, _, cx| {
                        let error = super::audio::toggle(&toggle).err();
                        if let Some(e) = error {
                            act(cx, |l| l.notify(trf("Lässt sich nicht abspielen: %@", &[&e]), "triangle-alert", None, None));
                        }
                        cx.refresh_windows();
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .child(div().truncate().child(name))
                    .child(div().id(("audio-seek", self.id)).relative().cursor_pointer().on_click(move |e, _, cx| {
                        let b = bar.get();
                        let width = f32::from(b.size.width);
                        if width > 0. {
                            super::audio::seek(&seek, f32::from(e.position().x - b.origin.x) / width);
                            cx.refresh_windows();
                        }
                    }).child(progress_bar(fraction, t.accent, &t)).child(canvas(move |b, _, _| measured.set(b), |_, _, _, _| {}).absolute().size_full())),
            )
            .child(caption(format!("{} / {}", time(position), total.map_or("–:––".to_string(), time)), &t).flex_none().font_features(tabular()))
    }
}
