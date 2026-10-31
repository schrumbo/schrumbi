use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::{self, act};
use super::motion::{self, Soft};
use super::theme::{Theme, font, radius, space, iconsize};
use super::widgets::*;
use crate::core::day;
use crate::core::filing::FileRole;
use crate::core::i18n::{Lang, lang, tr, trf};
use crate::core::library;
use crate::core::model::Status;
use crate::core::prefs;

/// Whether files can be dragged out of the app on this platform.
const CAN_DRAG_OUT: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// Starts the upload flow: opens the module's upload link and the floating upload panel.
pub fn begin(id: &str, cx: &mut App) {
    act(cx, |l| l.begin_upload(id));
    open(cx);
}

fn open(cx: &mut App) {
    for w in cx.windows() {
        if let Some(h) = w.downcast::<gpui_kit::base::Root>()
            && h.update(cx, |root, window, _| {
                let found = root.view().clone().downcast::<UploadPanel>().is_ok();
                if found {
                    window.activate_window();
                }
                found
            })
            .unwrap_or(false)
        {
            return;
        }
    }
    let panel = size(px(340.), px(420.));
    let saved = prefs::get().upload_window.map(|[x, y]| Bounds::new(point(px(x), px(y)), panel)).filter(|b| cx.displays().iter().any(|d| d.bounds().intersects(b)));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(saved.unwrap_or_else(|| Bounds::centered(None, panel, cx)))),
        window_min_size: Some(size(px(340.), px(220.))),
        titlebar: Some(TitlebarOptions { title: Some(tr("Hochladen").into()), ..Default::default() }),
        kind: WindowKind::Floating,
        ..Default::default()
    };
    let options = super::chrome::window_options(options, tr("Hochladen"));
    let _ = gpui_kit::open_window(options, cx, |window, cx| {
        keep_visible_when_inactive(window);
        cx.new(|cx| UploadPanel::new(window, cx))
    });
}

/// The panel must stay above the browser while the user drags files into it.
#[cfg(target_os = "macos")]
fn keep_visible_when_inactive(window: &Window) {
    use objc2::runtime::{AnyObject, Bool};
    use objc2::msg_send;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(RawWindowHandle::AppKit(handle)) = HasWindowHandle::window_handle(window).map(|h| h.as_raw()) else { return };
    let view = handle.ns_view.as_ptr() as *mut AnyObject;
    unsafe {
        let ns_window: *mut AnyObject = msg_send![view, window];
        if !ns_window.is_null() {
            let _: () = msg_send![ns_window, setHidesOnDeactivate: Bool::NO];
        }
    }
}

/// Windows has no floating level for this window kind, so the panel is marked topmost.
#[cfg(windows)]
fn keep_visible_when_inactive(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos};
    let Ok(RawWindowHandle::Win32(handle)) = HasWindowHandle::window_handle(window).map(|h| h.as_raw()) else { return };
    unsafe {
        SetWindowPos(handle.hwnd.get() as _, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn keep_visible_when_inactive(_: &Window) {}

fn size_label(path: &Path) -> String {
    let bytes = std::fs::metadata(path).map_or(0, |m| m.len()) as f64;
    let label = match bytes {
        b if b < 1e3 => format!("{b} B"),
        b if b < 1e6 => format!("{:.0} KB", b / 1e3),
        b if b < 1e9 => format!("{:.1} MB", b / 1e6),
        b => format!("{:.1} GB", b / 1e9),
    };
    if lang() == Lang::De { label.replace('.', ",") } else { label }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn start_native_drag(paths: Vec<PathBuf>, preview: Option<Vec<u8>>, window: &Window) {
    let item = drag::DragItem::Files(paths);
    let image = drag::Image::Raw(preview.unwrap_or_else(|| include_bytes!("../../assets/brand/tray.png").to_vec()));
    let _ = drag::start_drag(window, item, image, |_, _| {}, drag::Options::default());
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn start_native_drag(_: Vec<PathBuf>, _: Option<Vec<u8>>, _: &Window) {}

/// Floating helper while uploading: the assignment's files ready to drag into the browser.
pub struct UploadPanel {
    focus: FocusHandle,
    focused: bool,
    notes_open: bool,
    pressed: Option<(Point<Pixels>, Vec<PathBuf>)>,
    previews: HashMap<PathBuf, Option<Vec<u8>>>,
    save_position: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl UploadPanel {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let lib = state::lib(cx);
        let subs = vec![
            cx.observe(&lib, |_, _, cx| cx.notify()),
            cx.observe_window_bounds(window, |this, window, cx| {
                let origin = window.bounds().origin;
                let saved = [f32::from(origin.x), f32::from(origin.y)];
                this.save_position = Some(cx.spawn(async move |_, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
                    prefs::update(|p| p.upload_window = Some(saved));
                }));
            }),
        ];
        Self { focus: cx.focus_handle(), focused: false, notes_open: false, pressed: None, previews: HashMap::new(), save_position: None, _subs: subs }
    }

    /// Renders small first-page images of PDFs in the background, to show while dragging.
    fn load_previews(&mut self, files: &[PathBuf], cx: &mut Context<Self>) {
        for file in files {
            if self.previews.contains_key(file) || !file.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
                continue;
            }
            self.previews.insert(file.clone(), None);
            let path = file.clone();
            let task = cx.background_spawn(async move { crate::core::pdf_render::PdfDoc::open(&path).and_then(|d| d.render_png(0, 0.2)) });
            let file = file.clone();
            cx.spawn(async move |this, cx| {
                let png = task.await;
                let _ = this.update(cx, |p, _| {
                    p.previews.insert(file, png);
                });
            })
            .detach();
        }
    }

    fn start_drag(&mut self, window: &Window) {
        if let Some((_, files)) = self.pressed.take() {
            let preview = files.first().and_then(|f| self.previews.get(f).cloned().flatten());
            start_native_drag(files, preview, window);
        }
    }

    fn drag_all(&self, files: Vec<PathBuf>, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let count = files.len();
        div()
            .id("upload-all")
            .flex()
            .items_center()
            .justify_center()
            .gap(px(space::S))
            .p(px(space::S))
            .rounded(px(radius::CONTROL))
            .border_1()
            .border_dashed()
            .border_color(t.separator)
            .soft(t.ink.opacity(0.1), radius::CONTROL)
            .cursor_grab()
            .text_size(px(font::CALLOUT))
            .on_mouse_down(MouseButton::Left, cx.listener(move |p, e: &MouseDownEvent, _, _| p.pressed = Some((e.position, files.clone()))))
            .on_mouse_move(cx.listener(|p, e: &MouseMoveEvent, window, _| p.follow_drag(e, window)))
            .child(icon("hand", iconsize::S, t.secondary))
            .child(trf("Alle %lld Dateien ziehen", &[&count]))
    }

    fn follow_drag(&mut self, e: &MouseMoveEvent, window: &Window) {
        if e.pressed_button != Some(MouseButton::Left) {
            self.pressed = None;
            return;
        }
        let moved = self.pressed.as_ref().is_some_and(|(start, _)| {
            let (dx, dy) = (f32::from(e.position.x - start.x), f32::from(e.position.y - start.y));
            dx * dx + dy * dy > 16.
        });
        if moved {
            self.start_drag(window);
        }
    }

    fn mark_uploaded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = state::read(cx, |l| l.upload_assignment().filter(|a| l.evaluation(a).status != Status::Uploaded).map(|a| a.id.clone())) else { return };
        act(cx, |l| {
            l.set_status(Some(Status::Uploaded), &id, true);
            l.upload_assignment_id = None;
        });
        window.remove_window();
    }

    fn row(&self, path: &Path, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let hover = t.ink.opacity(0.1);
        let (press, open_path, menu) = (path.to_path_buf(), path.to_path_buf(), path.to_path_buf());
        div()
            .id(SharedString::from(format!("upload-{}", path.display())))
            .flex()
            .items_center()
            .gap(px(space::S))
            .p(px(space::S))
            .rounded(px(radius::CONTROL))
            .bg(t.ink.opacity(0.05))
            .soft(hover, radius::CONTROL)
            .when(CAN_DRAG_OUT, |d| d.cursor_grab())
            .tooltip(tip(if CAN_DRAG_OUT { tr("Ziehen zum Hochladen, Doppelklick öffnet") } else { tr("Doppelklick öffnet") }))
            .on_mouse_down(MouseButton::Left, cx.listener(move |p, e: &MouseDownEvent, _, _| p.pressed = Some((e.position, vec![press.clone()]))))
            .on_mouse_move(cx.listener(|p, e: &MouseMoveEvent, window, _| p.follow_drag(e, window)))
            .on_click(move |e, _, _| {
                if e.click_count() == 2 {
                    library::open_path(&open_path);
                }
            })
            .context_menu(move |m, _, _| {
                let (a, b, c) = (menu.clone(), menu.clone(), menu.clone());
                m.item(PopupMenuItem::new(tr("Vorschau")).on_click(move |_, _, cx| super::preview::show(&a, cx)))
                    .item(PopupMenuItem::new(super::platform::reveal_label()).on_click(move |_, _, _| library::reveal(&b)))
                    .item(PopupMenuItem::new(tr("Pfad kopieren")).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(c.to_string_lossy().to_string()))))
            })
            .child(file_icon(path, iconsize::L))
            .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().child(path.file_name().unwrap_or_default().to_string_lossy().to_string())).child(caption(size_label(path), t)))
            .when(CAN_DRAG_OUT, |d| d.child(icon("hand", iconsize::S, t.secondary.opacity(0.6))))
    }
}

impl Render for UploadPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !std::mem::replace(&mut self.focused, true) {
            window.focus(&self.focus, cx);
        }
        let t = Theme::get(cx);
        let root = div()
            .size_full()
            .track_focus(&self.focus)
            .bg(t.content)
            .text_size(px(font::BODY))
            .text_color(t.primary)
            .on_key_down(cx.listener(|p, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "enter" {
                    p.mark_uploaded(window, cx);
                } else if e.keystroke.key == "escape" {
                    window.remove_window();
                }
            }));
        struct Data {
            id: String,
            title: String,
            code: Option<(String, Hsla)>,
            link: Option<(String, url::Url)>,
            countdown: String,
            urgent: bool,
            files: Vec<PathBuf>,
            folder: PathBuf,
            uploaded: bool,
        }
        let data = state::read(cx, |l| {
            l.upload_assignment().map(|a| {
                let e = l.evaluation(a);
                let module = l.module(a);
                Data {
                    id: a.id.clone(),
                    title: a.title.clone(),
                    code: module.map(|m| (m.code.clone(), t.palette(&m.color))),
                    link: module.and_then(|m| m.upload_link()).and_then(|k| k.url().map(|u| (if k.title.is_empty() { tr("Link") } else { k.title.clone() }, u))),
                    countdown: day::countdown(a.due, l.now),
                    urgent: l.needs_action(a),
                    uploaded: e.status == Status::Uploaded,
                    folder: a.folder.clone(),
                    files: e.files,
                }
            })
        });
        let Some(data) = data else {
            return super::chrome::frame(tr("Hochladen"), root.child(unavailable("file-up", tr("Kein Upload aktiv"), tr("Starte einen Upload über „Hochladen“ bei einer Abgabe."), &t)), &t);
        };
        self.load_previews(&data.files, cx);
        let (finals, drafts): (Vec<PathBuf>, Vec<PathBuf>) = data.files.iter().cloned().partition(|f| !FileRole::is_draft(f));
        let both = !finals.is_empty() && !drafts.is_empty();
        let main = if finals.is_empty() { drafts.clone() } else { finals };
        let mut body = div().flex().flex_col().gap(px(space::M)).p(px(space::L)).child(
            div()
                .flex()
                .flex_col()
                .gap(px(space::XS))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .when_some(data.code.clone(), |d, (code, color)| d.child(module_chip(&code, color)))
                        .child(text(data.countdown.clone(), font::CAPTION, if data.urgent { t.palette("red") } else { t.secondary })),
                )
                .child(div().font_weight(FontWeight::SEMIBOLD).child(data.title.clone())),
        );
        if data.files.is_empty() {
            let id = data.id.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(space::S))
                    .py(px(space::M))
                    .child(icon("file-plus", iconsize::L, t.secondary))
                    .child(div().text_color(t.secondary).child(tr("Noch keine Datei abgelegt.")))
                    .child(Button::new("upload-pick").small().label(tr("Datei ablegen…")).on_click(move |_, _, cx| super::platform::pick_files_for_filing(Some(id.clone()), cx))),
            );
        } else {
            let hint = if CAN_DRAG_OUT { tr("In das Upload-Feld im Browser ziehen:") } else { tr("Im Dateimanager zeigen und von dort in den Browser ziehen:") };
            body = body.child(caption(hint, &t)).child(div().flex().flex_col().gap(px(space::S)).children(main.iter().map(|f| self.row(f, &t, cx)).collect::<Vec<_>>()).when(CAN_DRAG_OUT && main.len() > 1, |d| d.child(self.drag_all(main.clone(), &t, cx))));
            if both {
                let open = self.notes_open;
                let me = cx.entity();
                body = body.child(
                    div()
                        .id("upload-notes")
                        .flex()
                        .items_center()
                        .gap(px(space::XS))
                        .cursor_pointer().key_nav()
                        .on_click(move |_, _, cx| me.update(cx, |p, cx| {
                            p.notes_open = !p.notes_open;
                            cx.notify();
                        }))
                        .child(motion::chevron("chevron", open, t.secondary))
                        .child(caption(trf("Notizen (%lld)", &[&drafts.len()]), &t)),
                );
                body = body.child(motion::reveal("upload-notes-list", open, div().flex().flex_col().gap(px(space::S)).children(drafts.iter().map(|f| self.row(f, &t, cx)).collect::<Vec<_>>())));
            }
        }
        let first = data.files.first().cloned();
        let folder = data.folder.clone();
        let link = data.link.clone();
        let done = cx.entity();
        let footer = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .px(px(space::L))
            .py(px(space::M))
            .border_t_1()
            .border_color(t.separator)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .when_some(link, |d, (title, url)| {
                        d.child(Button::new("upload-link").ghost().small().icon(menu_icon("globe")).label(trf("%@ öffnen", &[&title])).on_click(move |_, _, _| {
                            let _ = opener::open(url.as_str());
                        }))
                    })
                    .child(Button::new("upload-reveal").ghost().small().icon(menu_icon("folder")).label(super::platform::reveal_label()).on_click(move |_, _, _| {
                        match &first {
                            Some(f) => library::reveal(f),
                            None => {
                                let _ = std::fs::create_dir_all(&folder);
                                library::open_path(&folder);
                            }
                        }
                    })),
            )
            .child(Button::new("upload-done").primary().w_full().icon(menu_icon("check")).label(tr("Hochgeladen")).disabled(data.uploaded).on_click(move |_, window, cx| done.update(cx, |p, cx| p.mark_uploaded(window, cx))));
        super::chrome::frame(tr("Hochladen"), root.flex().flex_col().child(div().flex_1().min_h_0().child(div().id("upload-scroll").size_full().overflow_y_scrollbar().child(body))).child(footer), &t)
    }
}
