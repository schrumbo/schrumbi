use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion::{Soft, loading};
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

/// Where linked nodes stand in each diagram, filled while drawing.
static DIAGRAM_SPOTS: std::sync::LazyLock<std::sync::Mutex<HashMap<(String, bool), Vec<(String, f32, f32)>>>> = std::sync::LazyLock::new(Default::default);

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
    if let (Some(w), Some(h)) = (attribute("width"), attribute("height")) {
        return (w, h);
    }
    let start = svg.find("viewBox=\"").map(|at| at + 9);
    let view: Vec<f32> = start.and_then(|s| svg[s..].find('"').map(|e| &svg[s..s + e])).map(|v| v.split_whitespace().filter_map(|n| n.parse().ok()).collect()).unwrap_or_default();
    match view.as_slice() {
        [_, _, w, h] => (*w, *h),
        _ => (400., 240.),
    }
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
    scale: f32,
}

impl RenderOnce for Formula {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let color = if t.dark { "#EDEDED" } else { "#1F1F1F" };
        let source = self.source.clone();
        let painted = picture(format!("math:{source}"), t.dark, move || render::math_svg(&source, true, color), cx);
        let body: AnyElement = match painted {
            Painted::Done(image, w, h) => img(image).w(px(w * self.scale)).h(px(h * self.scale)).flex_none().into_any_element(),
            Painted::Busy => div().h(px(32.)).child(loading(iconsize::S, t.tertiary)).into_any_element(),
            Painted::Failed(e) => source_with_error(&self.source, &e, &t).into_any_element(),
        };
        div().id(("md-math", self.id)).w_full().overflow_x_scroll().flex().justify_center().py(px(space::XS)).child(body)
    }
}

fn source_with_error(source: &str, error: &str, t: &Theme) -> Div {
    div().w_full().flex().flex_col().gap(px(space::XS)).child(div().p(px(space::S)).rounded(px(radius::INNER)).bg(t.secondary_bg).font_family(Theme::mono_family()).text_size(px(font::CALLOUT)).child(source.to_string())).child(div().flex().items_center().gap(px(space::XS)).child(icon("triangle-alert", iconsize::XS, t.orange)).child(caption(error.to_string(), t)))
}

/// A formula for inside a sentence, once it is drawn; `None` while it is not.
pub fn inline_math(source: &str, scale: f32, cx: &mut App) -> Option<(Arc<Image>, f32, f32)> {
    let dark = Theme::get(cx).dark;
    let color = if dark { "#EDEDED" } else { "#1F1F1F" };
    let formula = source.to_string();
    match picture(format!("imath:{source}"), dark, move || render::math_svg(&formula, false, color), cx) {
        Painted::Done(image, w, h) => Some((image, w * 1.15 * scale, h * 1.15 * scale)),
        _ => None,
    }
}

/// Whether a formula for inside a sentence is still being drawn.
pub fn inline_math_pending(source: &str, cx: &App) -> bool {
    let slot = (format!("imath:{source}"), Theme::get(cx).dark);
    PICTURES.with(|c| !matches!(c.borrow().get(&slot), Some(Painted::Done(..) | Painted::Failed(_))))
}

pub fn math(source: &str, id: u64, _: &Theme) -> AnyElement {
    Formula { source: source.to_string(), id, scale: super::theme::note_scale() }.into_any_element()
}

#[derive(IntoElement)]
struct Diagram {
    code: String,
    id: u64,
    sample: bool,
}

impl RenderOnce for Diagram {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let (code, dark) = (self.code.clone(), t.dark);
        let links = render::mermaid_links(&self.code);
        let spot_links = links.clone();
        let spot_key = (self.code.clone(), dark);
        let painted = picture(format!("mermaid:{}:{code}", crate::core::i18n::is_english()), dark, move || render::mermaid_svg(&code, dark).map(|svg| {
            let (w, h) = svg_size(&svg);
            DIAGRAM_SPOTS.lock().unwrap().insert(spot_key, render::mermaid_link_spots(&svg, &spot_links));
            (svg, w, h)
        }), cx);
        let body: AnyElement = match painted {
            Painted::Done(image, w, h) => {
                let scale = (680. / w).min(1.);
                let shown = image.clone();
                let place = Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
                let measured = place.clone();
                let sample = self.sample;
                let spots = DIAGRAM_SPOTS.lock().unwrap().get(&(self.code.clone(), dark)).cloned().unwrap_or_default();
                div()
                    .id(("md-diagram-open", self.id))
                    .relative()
                    .cursor_pointer()
                    .child(img(image).w(px(w * scale)).h(px(h * scale)))
                    .child(canvas(|_, _, _| (), move |b, _, _, _| measured.set(b)).absolute().top_0().left_0().size_full())
                    .on_click(move |e, window, cx| {
                        let origin = place.get().origin;
                        let at = e.position();
                        let (x, y) = (f32::from(at.x - origin.x) / scale, f32::from(at.y - origin.y) / scale);
                        let near = spots.iter().map(|(name, sx, sy)| (name, (sx - x).powi(2) + (sy - y).powi(2))).filter(|(_, d)| *d < 70. * 70.).min_by(|a, b| a.1.total_cmp(&b.1));
                        match near {
                            Some((name, _)) if !sample => {
                                let destination = name.clone();
                                act(cx, |l| {
                                    l.open_link(&destination, None);
                                });
                            }
                            _ => open_large(shown.clone(), w, h, window, cx),
                        }
                    })
                    .into_any_element()
            }
            Painted::Busy => div().h(px(120.)).flex().items_center().child(loading(iconsize::M, t.tertiary)).into_any_element(),
            Painted::Failed(e) => source_with_error(&self.code, &e, &t).into_any_element(),
        };
        div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .child(body)
            .when(!links.is_empty(), |d| {
                d.child(div().w_full().pt(px(space::S)).flex().flex_wrap().gap(px(space::S)).children(links.into_iter().enumerate().map(|(i, name)| {
                    let (label, target) = (name.clone(), name);
                    if self.sample {
                        return div().text_color(t.accent_text).text_size(px(font::CALLOUT)).child(label).into_any_element();
                    }
                    link_button(("md-diagram-link", (self.id as usize) * 1000 + i), label, &t).text_size(px(font::CALLOUT)).on_click(move |_, _, cx| {
                        let destination = target.clone();
                        act(cx, |l| {
                            l.open_link(&destination, None);
                        });
                    }).into_any_element()
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
    let sample = super::markdown_view::is_sample_base(base);
    match lang.as_str() {
        "mermaid" => Some(Diagram { code: code.to_string(), id, sample }.into_any_element()),
        "math" | "latex" if false => None,
        "query" => Some(QueryBlock { query: code.trim().to_string(), id, sample }.into_any_element()),
        "base" => Some(BaseBlock { source: code.to_string(), this: base.map(|b| b.join(".")), id, sample }.into_any_element()),
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

/// The plugin a whole note belongs to, by its front matter or name.
pub fn note_plugin(path: &Path, text: &str) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let head = &text[..crate::core::vault::frontmatter::body_offset(text)];
    if name.ends_with(".excalidraw.md") || head.contains("excalidraw-plugin") {
        Some("Excalidraw")
    } else if head.contains("kanban-plugin") {
        Some("Kanban")
    } else {
        None
    }
}

#[derive(IntoElement)]
struct QueryBlock {
    query: String,
    id: u64,
    sample: bool,
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
                    let sample = self.sample;
                    div().id(("query-hit", (self.id as usize) * 1000 + i)).px(px(space::M)).py(px(space::XS)).flex().items_center().gap(px(space::S)).when(!sample, |d| d.cursor_pointer().soft(hover, 0.)).on_click(move |_, _, cx| if !sample { act(cx, |l| l.open_file(&open)) }).child(file_icon(&f, iconsize::S)).child(div().truncate().child(name))
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
    sample: bool,
}

type BaseTable = Result<(Vec<String>, Vec<bases::ResultRow>), String>;

thread_local! {
    static BASE_TABLES: RefCell<HashMap<(String, Option<PathBuf>, u64), Rc<BaseTable>>> = RefCell::new(HashMap::new());
    static FILE_TEXTS: RefCell<HashMap<PathBuf, (Option<std::time::SystemTime>, Rc<String>)>> = RefCell::new(HashMap::new());
}

/// A file's text, read again only when its time changed.
fn cached_text(path: &Path) -> Rc<String> {
    let time = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    FILE_TEXTS.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((known, text)) = c.get(path)
            && *known == time
        {
            return text.clone();
        }
        let text = Rc::new(std::fs::read_to_string(path).unwrap_or_default());
        c.insert(path.to_path_buf(), (time, text.clone()));
        text
    })
}

/// A table only changes with its source or the vault, so scrolling past it costs nothing.
fn base_table(source: &str, this: Option<&Path>, vault: &index::VaultIndex) -> Rc<BaseTable> {
    let key = (source.to_string(), this.map(PathBuf::from), vault.generation);
    if let Some(known) = BASE_TABLES.with(|c| c.borrow().get(&key).cloned()) {
        return known;
    }
    let table: BaseTable = bases::Base::parse(source).and_then(|base| {
        let view = base.views.first().cloned().unwrap_or_default();
        base.rows(&view, vault, this, chrono::Local::now()).map(|rows| (base.columns(&view).iter().map(|c| base.column_label(c)).collect(), rows))
    });
    let table = Rc::new(table);
    BASE_TABLES.with(|c| {
        let mut c = c.borrow_mut();
        c.retain(|k, _| k.2 == vault.generation);
        c.insert(key, table.clone());
    });
    table
}

impl RenderOnce for BaseBlock {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let vault = index::current();
        let table = base_table(&self.source, self.this.as_deref(), &vault);
        let body: AnyElement = match table.as_ref() {
            Err(e) => notice("triangle-alert", t.orange, tr("Diese Ansicht lässt sich nicht lesen."), Some(e.clone()), div(), &t).into_any_element(),
            Ok((columns, rows)) if rows.is_empty() => {
                let _ = columns;
                div().p(px(space::M)).child(caption(tr("Keine Notizen passen zu diesem Filter"), &t)).into_any_element()
            }
            Ok((columns, rows)) => {
                let head = div().flex().bg(t.secondary_bg).font_weight(FontWeight::SEMIBOLD).text_size(px(font::CALLOUT)).children(columns.iter().map(|c| div().flex_1().min_w_0().truncate().px(px(space::S)).py(px(space::XS)).child(c.clone())));
                let lines = rows.iter().take(100).enumerate().map(|(i, row)| {
                    let open = row.path.clone();
                    let hover = t.ink.opacity(0.05);
                    let sample = self.sample;
                    div().id(("base-row", (self.id as usize) * 1000 + i)).flex().border_t_1().border_color(t.separator).when(!sample, |d| d.cursor_pointer().soft(hover, 0.)).on_click(move |_, _, cx| if !sample { act(cx, |l| l.open_file(&open)) }).children(row.cells.iter().map(|(_, v)| div().flex_1().min_w_0().truncate().px(px(space::S)).py(px(space::XS)).text_size(px(font::CALLOUT)).child(v.display())))
                });
                div().id(("base-scroll", self.id)).max_h(px(400.)).overflow_y_scroll().child(head).children(lines).into_any_element()
            }
        };
        grouped(&t).child(body)
    }
}

/// `![[x.base]]`.
pub fn base_file(path: PathBuf, id: u64, t: &Theme) -> AnyElement {
    let source = cached_text(&path).to_string();
    let open = path.clone();
    let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let sample = super::help::is_sample(&path);
    let action = if sample {
        div().text_color(t.accent_text).text_size(px(font::CAPTION)).child(tr("Öffnen")).into_any_element()
    } else {
        link_button(("base-open", id), tr("Öffnen"), t).text_size(px(font::CAPTION)).on_click(move |_, _, cx| act(cx, |l| l.open_file(&open))).into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .gap(px(space::XS))
        .child(div().flex().justify_between().items_center().child(caption(name, t)).child(action))
        .child(BaseBlock { source, this: Some(path), id, sample })
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
    let doc = canvas::Canvas::parse(&cached_text(&path)).unwrap_or_default();
    let Some((x0, y0, x1, y1)) = doc.bounds() else { return div().p(px(space::M)).child(caption(tr("Leeres Canvas"), t)).into_any_element() };
    let (width, height) = ((x1 - x0).max(1.) as f32, (y1 - y0).max(1.) as f32);
    let scale = (640_f32 / width).min(240. / height).min(1.);
    let open = path.clone();
    let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let tint = t.ink.opacity(0.12);
    let border = t.ink.opacity(0.25);
    let sample = super::help::is_sample(&path);
    grouped(t)
        .child(
            div()
                .id(("canvas-embed", id))
                .when(!sample, |d| d.cursor_pointer().on_click(move |_, _, cx| act(cx, |l| l.open_file(&open))))
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

thread_local! {
    static TICKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn keep_ticking(cx: &mut App) {
    if TICKING.replace(true) {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            let playing = super::audio::any_playing();
            let _ = cx.update(|cx| cx.refresh_windows());
            if !playing {
                break;
            }
        }
        TICKING.set(false);
    })
    .detach();
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
        let total = total.or_else(|| super::audio::length(&self.path));
        if playing {
            keep_ticking(cx);
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
                    .soft(t.ink.opacity(0.06), radius::FULL)
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
