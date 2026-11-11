use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::core::ai::instance::alive;
use crate::core::drive::{self, on};

const KEPT_FRAMES: usize = 200_000;
const KEPT_SPOTS: usize = 20_000;
const AWAY: i32 = 12_000;

#[derive(Clone)]
struct Spot {
    id: SharedString,
    role: &'static str,
    label: Option<SharedString>,
    bounds: Bounds<Pixels>,
    visible: Option<Bounds<Pixels>>,
}

#[derive(Default)]
struct Drawn {
    pending: Vec<Spot>,
    shown: Vec<Spot>,
    last: Option<Instant>,
    forced: bool,
    title: Option<SharedString>,
}

#[derive(Default)]
struct Frames {
    first: u64,
    times: Vec<(u64, f32)>,
}

#[derive(Clone, Copy)]
struct Pointer {
    at: Point<Pixels>,
    held: Option<MouseButton>,
}

thread_local! {
    static WINDOWS: RefCell<HashMap<u64, Drawn>> = RefCell::new(HashMap::new());
    static FRAMES: RefCell<Frames> = RefCell::new(Frames::default());
    static POINTER: RefCell<HashMap<u64, Pointer>> = RefCell::new(HashMap::new());
}

/// Lets the driving agent find an element by id, role or label. Without an agent nothing is added.
pub trait Mark: Sized {
    fn mark(self, role: &'static str, label: impl Into<SharedString>) -> Self;
}

impl Mark for Stateful<Div> {
    fn mark(self, role: &'static str, label: impl Into<SharedString>) -> Self {
        super::coach::anchor(if on() { spot(self, role, Some(label.into())) } else { self })
    }
}

/// Like `Mark::mark`, for elements whose id says enough.
pub fn marked(element: Stateful<Div>, role: &'static str) -> Stateful<Div> {
    super::coach::anchor(if on() { spot(element, role, None) } else { element })
}

fn spot(mut element: Stateful<Div>, role: &'static str, label: Option<SharedString>) -> Stateful<Div> {
    let Some(id) = element.interactivity().element_id.clone() else { return element };
    let id = SharedString::from(id.to_string());
    element.child(
        canvas(
            move |bounds, window, _| {
                let visible = bounds.intersect(&window.content_mask().bounds);
                let visible = (visible.size.width > px(0.) && visible.size.height > px(0.)).then_some(visible);
                let key = window.window_handle().window_id().as_u64();
                WINDOWS.with_borrow_mut(|w| {
                    let drawn = w.entry(key).or_default();
                    if drawn.pending.len() < KEPT_SPOTS {
                        drawn.pending.push(Spot { id, role, label, bounds, visible });
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0(),
    )
}

/// Called after a window painted its frame, with the frame's CPU time.
pub fn frame(window: &Window, ms: Option<f32>, title: Option<&SharedString>, cx: &mut App) {
    if !on() {
        return;
    }
    let key = window.window_handle().window_id().as_u64();
    let (forced, first) = WINDOWS.with_borrow_mut(|w| {
        let drawn = w.entry(key).or_default();
        let first = drawn.last.is_none();
        let forced = std::mem::take(&mut drawn.forced);
        if forced {
            drawn.shown = std::mem::take(&mut drawn.pending);
        } else {
            drawn.pending.clear();
        }
        drawn.last = Some(Instant::now());
        if let Some(title) = title {
            drawn.title = Some(title.clone());
        }
        (forced, first)
    });
    if first && drive::away() {
        let handle = window.window_handle();
        cx.defer(move |cx| drop(handle.update(cx, |_, window, _| send_away(window))));
    }
    if let Some(ms) = ms.filter(|_| !forced) {
        FRAMES.with_borrow_mut(|f| {
            if f.times.len() == KEPT_FRAMES {
                f.times.drain(..KEPT_FRAMES / 2);
                f.first += (KEPT_FRAMES / 2) as u64;
            }
            f.times.push((key, ms));
        });
    }
}

/// Brings a window to the front, except while an agent drives the app: the owner keeps the focus.
pub fn activate_window(window: &Window) {
    let key = window.window_handle().window_id().as_u64();
    let title = WINDOWS.with_borrow(|w| w.get(&key).and_then(|d| d.title.clone())).map_or_else(|| window.window_title(), |t| t.to_string());
    if !drive::fenced("activate", title) {
        window.activate_window();
    }
}

/// Copies `text`, except while an agent drives the app: the owner's clipboard stays untouched.
pub fn copy(text: String, cx: &mut App) {
    if !drive::fenced("clipboard", &text) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

/// Makes the app frontmost, except while an agent drives it.
pub fn activate(cx: &mut App) {
    if !on() {
        cx.activate(true);
    }
}

struct Call {
    body: Value,
    reply: mpsc::Sender<Value>,
    deadline: Instant,
}

/// Opens the automation channel when `SCHRUMBI_DRIVE` is set: a local port and token in `drive.json`.
pub fn init(cx: &mut App) {
    let Some(dir) = drive::dir() else { return };
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("drive: no local port: {e}");
            return;
        }
    };
    let port = listener.local_addr().map_or(0, |a| a.port());
    let token = uuid::Uuid::new_v4().simple().to_string();
    let info = json!({ "port": port, "token": token, "pid": std::process::id() });
    let tmp = dir.join("drive.json.tmp");
    if std::fs::write(&tmp, info.to_string()).and_then(|()| std::fs::rename(&tmp, dir.join("drive.json"))).is_err() {
        eprintln!("drive: cannot write {}", dir.display());
        return;
    }
    let (tx, rx) = async_channel::unbounded::<Call>();
    let last = Arc::new(Mutex::new(Instant::now()));
    {
        let (tx, last) = (tx.clone(), last.clone());
        let _ = std::thread::Builder::new().name("drive".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                let (tx, last, token) = (tx.clone(), last.clone(), token.clone());
                let _ = std::thread::Builder::new().name("drive-client".into()).spawn(move || serve(stream, &token, &tx, &last));
            }
        });
    }
    let _ = std::thread::Builder::new().name("drive-watch".into()).spawn(move || watch(&tx, &last));
    std::mem::forget(cx.background_executor().prevent_app_nap("driven by an agent"));
    cx.spawn(async move |cx| {
        while let Ok(call) = rx.recv().await {
            if Instant::now() > call.deadline {
                continue;
            }
            cx.spawn(async move |cx| {
                let out = respond(&call.body, cx).await;
                let _ = call.reply.send(out);
            })
            .detach();
        }
    })
    .detach();
}

fn serve(stream: TcpStream, token: &str, tx: &async_channel::Sender<Call>, last: &Mutex<Instant>) {
    let Ok(mut writer) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let out = match serde_json::from_str::<Value>(&line) {
            Err(e) => json!({ "ok": false, "error": format!("bad request: {e}") }),
            Ok(body) if body["token"].as_str() != Some(token) => json!({ "ok": false, "error": "wrong token" }),
            Ok(body) => {
                *last.lock().unwrap() = Instant::now();
                if body["cmd"] == "ping" {
                    let ui = ask(tx, json!({ "cmd": "ui" }), Duration::from_secs(2)).is_some();
                    json!({ "ok": true, "pid": std::process::id(), "ui": ui })
                } else {
                    let wait = Duration::from_secs_f64(body["timeout"].as_f64().unwrap_or(30.).clamp(0., 3600.) + 5.);
                    ask(tx, body, wait).unwrap_or_else(|| json!({ "ok": false, "hung": true, "error": format!("the UI thread did not answer within {} s", wait.as_secs()) }))
                }
            }
        };
        if writeln!(writer, "{out}").and_then(|()| writer.flush()).is_err() {
            break;
        }
    }
}

fn ask(tx: &async_channel::Sender<Call>, body: Value, wait: Duration) -> Option<Value> {
    let (reply, answer) = mpsc::channel();
    let until = Instant::now() + wait;
    tx.send_blocking(Call { body, reply, deadline: until }).ok()?;
    let mut closed = false;
    loop {
        match answer.recv_timeout(Duration::from_millis(100)) {
            Ok(mut out) => {
                if closed {
                    out["menu"] = json!("a native menu opened and was closed; native menus cannot be driven");
                }
                return Some(out);
            }
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < until => {
                if native_menu::close() {
                    closed = true;
                    drive::note("native menu closed".into());
                }
            }
            Err(_) => return None,
        }
    }
}

#[cfg(target_os = "windows")]
mod native_menu {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_CANCELMODE};

    unsafe extern "system" fn collect(hwnd: HWND, list: LPARAM) -> BOOL {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == std::process::id() && unsafe { IsWindowVisible(hwnd) } != 0 {
            let mut class = [0u16; 16];
            let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
            let menu = String::from_utf16_lossy(&class[..n.max(0) as usize]) == "#32768";
            unsafe { (*(list as *mut Vec<(HWND, bool)>)).push((hwnd, menu)) };
        }
        TRUE
    }

    pub fn close() -> bool {
        let mut list: Vec<(HWND, bool)> = Vec::new();
        unsafe { EnumWindows(Some(collect), &mut list as *mut _ as LPARAM) };
        if !list.iter().any(|(_, menu)| *menu) {
            return false;
        }
        for (hwnd, _) in list {
            let mut result = 0;
            unsafe { SendMessageTimeoutW(hwnd, WM_CANCELMODE, 0, 0, SMTO_ABORTIFHUNG, 500, &mut result) };
        }
        true
    }
}

#[cfg(not(target_os = "windows"))]
mod native_menu {
    pub fn close() -> bool {
        false
    }
}

fn watch(tx: &async_channel::Sender<Call>, last: &Mutex<Instant>) {
    let owner = std::env::var("SCHRUMBI_DRIVE_OWNER").ok().and_then(|p| p.parse::<u32>().ok()).filter(|&p| p != 0);
    let ttl = Duration::from_secs(std::env::var("SCHRUMBI_DRIVE_TTL").ok().and_then(|s| s.parse().ok()).unwrap_or(1800));
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let reason = if owner.is_some_and(|p| !alive(p)) {
            "the agent that started it is gone"
        } else if last.lock().unwrap().elapsed() > ttl {
            "nobody drove it for too long"
        } else {
            continue;
        };
        eprintln!("drive: quitting, {reason}");
        let _ = ask(tx, json!({ "cmd": "quit" }), Duration::from_secs(10));
        std::thread::sleep(Duration::from_secs(10));
        std::process::exit(3);
    }
}

async fn respond(body: &Value, cx: &mut AsyncApp) -> Value {
    match run(body, cx).await {
        Ok(Value::Object(mut map)) => {
            map.insert("ok".into(), json!(true));
            Value::Object(map)
        }
        Ok(other) => json!({ "ok": true, "value": other }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

async fn run(b: &Value, cx: &mut AsyncApp) -> Result<Value, String> {
    let cmd = b["cmd"].as_str().unwrap_or_default();
    match cmd {
        "ui" => Ok(json!({})),
        "quit" => {
            cx.update(|cx| cx.quit());
            Ok(json!({}))
        }
        "windows" => Ok(json!({ "windows": cx.update(windows) })),
        "events" => Ok(json!({ "events": drive::take_events() })),
        "dialog" => {
            let paths: Option<Vec<PathBuf>> = b["paths"].as_array().map(|p| p.iter().filter_map(|p| p.as_str()).map(PathBuf::from).collect());
            drive::queue_answer(paths.filter(|p| !p.is_empty()));
            Ok(json!({}))
        }
        "state" => Ok(cx.update(state)),
        "actions" => Ok(json!({ "actions": cx.update(|cx| cx.all_action_names().iter().map(|a| a.to_string()).collect::<Vec<_>>()) })),
        "frames" => {
            let since = b["since"].as_u64().unwrap_or(u64::MAX);
            Ok(FRAMES.with_borrow(|f| {
                let next = f.first + f.times.len() as u64;
                let from = since.saturating_sub(f.first).min(f.times.len() as u64) as usize;
                json!({ "next": next, "frames": f.times[from..].iter().map(|(w, ms)| json!([w, ms])).collect::<Vec<_>>() })
            }))
        }
        "tree" => {
            let handles = redraw(b, cx).await?;
            let spots: Vec<Value> = handles.iter().flat_map(|h| shown(*h).into_iter().map(move |s| spot_json(*h, &s))).filter(|s| matches_filter(b, s)).collect();
            Ok(json!({ "spots": spots }))
        }
        "find" => {
            let (handle, spot) = find(b, cx).await?;
            Ok(spot_json(handle, &spot))
        }
        "wait" => wait(b, cx).await,
        "click" | "move" | "down" | "up" | "wheel" | "drop" | "drag" => pointer(cmd, b, cx).await,
        "key" => {
            let handle = one_window(b, cx)?;
            let names: Vec<String> = b["keys"].as_array().map(|k| k.iter().filter_map(|k| k.as_str()).map(key_name).collect()).unwrap_or_default();
            let os = cx.update(|cx| cx.platform_info().os);
            let mut keys = Vec::new();
            for k in names {
                let keystroke = Keystroke::parse_with_os(&k, os).map_err(|e| format!("{k}: {e}"))?;
                if !b["clipboard"].as_bool().unwrap_or(false) && touches_clipboard(&keystroke) {
                    return Err(format!("{k} would use the owner's clipboard; pass --clipboard if that is intended"));
                }
                keys.push((k, keystroke));
            }
            for (k, keystroke) in keys {
                let handled = press(handle, keystroke, cx)?;
                if !handled {
                    drive::note(format!("key {k} not handled"));
                }
                settle(handle, cx).await;
            }
            Ok(json!({}))
        }
        "type" => {
            let handle = one_window(b, cx)?;
            let text = b["text"].as_str().unwrap_or_default().to_string();
            if b["paste"].as_bool().unwrap_or(false) {
                let keystroke = Keystroke { modifiers: Modifiers::none(), key: String::new(), key_char: Some(text) };
                handle.update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx)).map_err(|e| e.to_string())?;
            } else {
                for c in text.chars() {
                    let keystroke = match c {
                        '\n' => Keystroke { modifiers: Modifiers::none(), key: "enter".into(), key_char: None },
                        ' ' => Keystroke { modifiers: Modifiers::none(), key: "space".into(), key_char: Some(" ".into()) },
                        '\t' => Keystroke { modifiers: Modifiers::none(), key: "tab".into(), key_char: None },
                        c if c.is_uppercase() => Keystroke { modifiers: Modifiers::shift(), key: c.to_lowercase().collect(), key_char: Some(c.to_string()) },
                        c => Keystroke { modifiers: Modifiers::none(), key: c.to_string(), key_char: Some(c.to_string()) },
                    };
                    press(handle, keystroke, cx)?;
                }
            }
            settle(handle, cx).await;
            Ok(json!({}))
        }
        "action" => {
            let handle = one_window(b, cx)?;
            let name = b["name"].as_str().unwrap_or_default().to_string();
            let name = if name.contains("::") {
                name
            } else {
                let full: Vec<String> = cx.update(|cx| cx.all_action_names().iter().filter(|a| a.rsplit("::").next() == Some(name.as_str())).map(|a| a.to_string()).collect());
                match full.as_slice() {
                    [one] => one.clone(),
                    [] => name,
                    many => return Err(format!("{name} is ambiguous: {}", many.join(", "))),
                }
            };
            if !b["clipboard"].as_bool().unwrap_or(false) && ["::Copy", "::Cut", "::Paste"].iter().any(|s| name.ends_with(s)) {
                return Err(format!("{name} would use the owner's clipboard; pass --clipboard if that is intended"));
            }
            handle
                .update(cx, |_, window, cx| {
                    let action = cx.build_action(&name, b.get("data").cloned()).map_err(|e| e.to_string())?;
                    window.dispatch_action(action, cx);
                    Ok::<(), String>(())
                })
                .map_err(|e| e.to_string())??;
            settle(handle, cx).await;
            Ok(json!({}))
        }
        "resize" => {
            let handle = one_window(b, cx)?;
            let (w, h) = (b["w"].as_f64().ok_or("w missing")? as f32, b["h"].as_f64().ok_or("h missing")? as f32);
            handle.update(cx, |_, window, _| window.resize(size(px(w), px(h)))).map_err(|e| e.to_string())?;
            settle(handle, cx).await;
            Ok(json!({}))
        }
        "capture" => {
            let handle = one_window(b, cx)?;
            settle(handle, cx).await;
            capture(handle, std::path::Path::new(b["path"].as_str().ok_or("path missing")?), b["full"].as_bool().unwrap_or(false), cx)
        }
        "theme" => {
            let choice: crate::core::prefs::ThemeChoice = serde_json::from_value(b["name"].clone()).map_err(|e| e.to_string())?;
            let handle = one_window(b, cx)?;
            handle.update(cx, |_, window, cx| super::theme::choose(choice, window, cx)).map_err(|e| e.to_string())?;
            settle(handle, cx).await;
            Ok(json!({}))
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

fn press(handle: AnyWindowHandle, keystroke: Keystroke, cx: &mut AsyncApp) -> Result<bool, String> {
    handle
        .update(cx, |_, window, cx| {
            let modifiers = keystroke.modifiers;
            let held = modifiers != Modifiers::none();
            if held {
                window.dispatch_event(PlatformInput::ModifiersChanged(ModifiersChangedEvent { modifiers, capslock: Capslock::default() }), cx);
            }
            let handled = window.dispatch_keystroke(keystroke.clone(), cx);
            window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke }), cx);
            if held {
                window.dispatch_event(PlatformInput::ModifiersChanged(ModifiersChangedEvent { modifiers: Modifiers::none(), capslock: Capslock::default() }), cx);
            }
            handled
        })
        .map_err(|e| e.to_string())
}

fn touches_clipboard(k: &Keystroke) -> bool {
    let key = k.key.to_lowercase();
    ((k.modifiers.platform || k.modifiers.control) && matches!(key.as_str(), "c" | "x" | "v" | "insert")) || (k.modifiers.shift && matches!(key.as_str(), "insert" | "delete"))
}

fn key_name(k: &str) -> String {
    let k = if k.len() > 1 && !k.ends_with('+') { k.replace('+', "-") } else { k.to_string() };
    k.split('-')
        .map(|part| match part {
            "cmd" | "mod" => "secondary",
            "esc" => "escape",
            "return" => "enter",
            "option" => "alt",
            "control" => "ctrl",
            p => p,
        })
        .collect::<Vec<_>>()
        .join("-")
}

fn windows(cx: &mut App) -> Vec<Value> {
    cx.windows()
        .into_iter()
        .filter_map(|handle| {
            handle
                .update(cx, |_, window, cx| {
                    let b = window.bounds();
                    let main = super::main_window::is_main(handle.window_id(), cx);
                    let title = WINDOWS.with_borrow(|w| w.get(&handle.window_id().as_u64()).and_then(|d| d.title.clone())).map_or_else(|| window.window_title(), |t| t.to_string());
                    let title = if title.is_empty() && main { "Schrumbi".to_string() } else { title };
                    let v = window.viewport_size();
                    json!({
                        "id": handle.window_id().as_u64(),
                        "title": title,
                        "main": main,
                        "active": window.is_window_active(),
                        "x": f32::from(b.origin.x), "y": f32::from(b.origin.y),
                        "w": f32::from(v.width), "h": f32::from(v.height),
                        "scale": window.scale_factor(),
                        "dark": super::theme::Theme::get(cx).dark,
                        "native": native(window),
                    })
                })
                .ok()
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn native(window: &Window) -> Value {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => json!(h.hwnd.get()),
        _ => Value::Null,
    }
}

#[cfg(target_os = "windows")]
fn send_away(window: &Window) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_EX_APPWINDOW,
        WS_EX_TOOLWINDOW,
    };
    let Some(hwnd) = native(window).as_i64() else { return };
    let hwnd = hwnd as isize as windows_sys::Win32::Foundation::HWND;
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return;
    }
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize));
        SetWindowPos(hwnd, std::ptr::null_mut(), rect.left - AWAY, rect.top, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

#[cfg(not(target_os = "windows"))]
fn send_away(_: &Window) {}

#[cfg(target_os = "macos")]
fn native(window: &Window) -> Value {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(RawWindowHandle::AppKit(h)) = HasWindowHandle::window_handle(window).map(|h| h.as_raw()) else { return Value::Null };
    let view = h.ns_view.as_ptr().cast::<AnyObject>();
    unsafe {
        let ns_window: *mut AnyObject = msg_send![view, window];
        if ns_window.is_null() {
            return Value::Null;
        }
        let number: isize = msg_send![ns_window, windowNumber];
        json!(number)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn native(window: &Window) -> Value {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xlib(h)) => json!(h.window),
        Ok(RawWindowHandle::Xcb(h)) => json!(h.window.get()),
        _ => Value::Null,
    }
}

fn state(cx: &mut App) -> Value {
    let main = cx.windows().into_iter().find(|h| super::main_window::is_main(h.window_id(), cx));
    let focus = main.and_then(|h| h.update(cx, |_, window, _| window.context_stack().iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(" > ")).ok());
    super::state::read(cx, |l| {
        json!({
            "route": format!("{:?}", l.route()),
            "workspace": l.root.as_ref().map(|p| p.display().to_string()),
            "error": l.last_error.clone(),
            "focus": focus,
        })
    })
}

fn pick_windows(b: &Value, cx: &mut AsyncApp) -> Result<Vec<AnyWindowHandle>, String> {
    let all = cx.update(|cx| {
        let mut all = cx.windows();
        all.sort_by_key(|h| !super::main_window::is_main(h.window_id(), cx));
        all
    });
    let Some(want) = b.get("window").filter(|w| !w.is_null()) else { return Ok(all) };
    let found: Vec<AnyWindowHandle> = if let Some(id) = want.as_u64() {
        all.into_iter().filter(|h| h.window_id().as_u64() == id).collect()
    } else {
        let want = want.as_str().unwrap_or_default().to_lowercase();
        cx.update(|cx| {
            all.into_iter()
                .filter(|h| if want == "main" { super::main_window::is_main(h.window_id(), cx) } else { title_of(*h, cx).to_lowercase().contains(&want) })
                .collect()
        })
    };
    if found.is_empty() { Err(format!("no window {want}")) } else { Ok(found) }
}

fn title_of(handle: AnyWindowHandle, cx: &mut App) -> String {
    WINDOWS.with_borrow(|w| w.get(&handle.window_id().as_u64()).and_then(|d| d.title.clone())).map_or_else(|| handle.update(cx, |_, w, _| w.window_title()).unwrap_or_default(), |t| t.to_string())
}

fn one_window(b: &Value, cx: &mut AsyncApp) -> Result<AnyWindowHandle, String> {
    pick_windows(b, cx)?.into_iter().next().ok_or_else(|| "no window".to_string())
}

fn shown(handle: AnyWindowHandle) -> Vec<Spot> {
    let key = handle.window_id().as_u64();
    let spots = WINDOWS.with_borrow(|w| w.get(&key).map(|d| d.shown.clone()).unwrap_or_default());
    let place = |s: &Spot| (s.id.clone(), [s.bounds.origin.x, s.bounds.origin.y, s.bounds.size.width, s.bounds.size.height].map(|v| (f32::from(v) * 2.).round() as i32));
    let mut at: HashMap<(SharedString, [i32; 4]), usize> = HashMap::new();
    let mut out: Vec<Spot> = Vec::new();
    for s in spots {
        match at.get(&place(&s)) {
            Some(&i) => {
                let kept = &mut out[i];
                if kept.label.is_none() {
                    kept.label = s.label;
                }
                if kept.role == "clickable" {
                    kept.role = s.role;
                }
            }
            None => {
                at.insert(place(&s), out.len());
                out.push(s);
            }
        }
    }
    out
}

fn rect(b: Bounds<Pixels>) -> Value {
    let r = |v: Pixels| (f32::from(v) * 10.).round() / 10.;
    json!([r(b.origin.x), r(b.origin.y), r(b.size.width), r(b.size.height)])
}

fn spot_json(handle: AnyWindowHandle, s: &Spot) -> Value {
    json!({
        "window": handle.window_id().as_u64(),
        "id": s.id,
        "role": s.role,
        "label": s.label,
        "bounds": rect(s.bounds),
        "visible": s.visible.map(rect),
    })
}

fn matches_filter(b: &Value, s: &Value) -> bool {
    b["target"].as_str().is_none_or(|t| matches(t, s))
}

fn matches(target: &str, s: &Value) -> bool {
    let (kind, query) = target.split_once([':', '~']).unwrap_or(("any", target));
    let fuzzy = target.as_bytes().get(kind.len()) == Some(&b'~') || kind == "any";
    let query = query.to_lowercase();
    let test = |field: &str| {
        let value = s[field].as_str().unwrap_or_default().to_lowercase();
        if fuzzy { value.contains(&query) } else { value == query }
    };
    match kind {
        "id" => test("id"),
        "text" | "label" => test("label"),
        "role" => test("role"),
        _ => test("id") || test("label"),
    }
}

async fn redraw(b: &Value, cx: &mut AsyncApp) -> Result<Vec<AnyWindowHandle>, String> {
    let named = b.get("window").is_some_and(|w| !w.is_null());
    let mut drawn = Vec::new();
    let mut stuck = None;
    for handle in pick_windows(b, cx)? {
        next_frame(handle, true, Duration::from_millis(600), cx).await;
        let key = handle.window_id().as_u64();
        if WINDOWS.with_borrow(|w| w.get(&key).is_some_and(|d| d.forced)) {
            stuck.get_or_insert(key);
        } else {
            drawn.push(handle);
        }
    }
    match stuck {
        Some(key) if named || drawn.is_empty() => Err(format!("window {key} did not draw within 0.6 s (hidden, minimized or busy), so its elements would be stale")),
        _ => Ok(drawn),
    }
}

async fn settle(handle: AnyWindowHandle, cx: &mut AsyncApp) {
    next_frame(handle, false, Duration::from_millis(150), cx).await;
}

async fn next_frame(handle: AnyWindowHandle, refresh: bool, limit: Duration, cx: &mut AsyncApp) {
    let (tx, rx) = async_channel::bounded::<()>(2);
    let late = tx.clone();
    let asked = handle
        .update(cx, move |_, window, _| {
            if refresh {
                WINDOWS.with_borrow_mut(|w| w.entry(handle.window_id().as_u64()).or_default().forced = true);
                window.refresh();
            }
            window.on_next_frame(move |window, _| window.on_next_frame(move |_, _| {
                let _ = tx.try_send(());
            }));
        })
        .is_ok();
    if !asked {
        return;
    }
    let timer = cx.background_executor().timer(limit);
    cx.background_executor()
        .spawn(async move {
            timer.await;
            let _ = late.try_send(());
        })
        .detach();
    let until = Instant::now() + limit;
    let mut got = rx.try_recv().is_ok();
    while !got && Instant::now() < until && pump(handle, cx) {
        cx.background_executor().timer(Duration::from_millis(35)).await;
        got = rx.try_recv().is_ok();
    }
    if !got {
        let _ = rx.recv().await;
    }
}

#[cfg(target_os = "macos")]
fn pump(handle: AnyWindowHandle, cx: &mut AsyncApp) -> bool {
    let view = handle.update(cx, |_, window, _| mac::view(window)).ok().flatten();
    view.is_some_and(|v| mac::covered(v) && mac::draw(v))
}

#[cfg(target_os = "windows")]
fn pump(handle: AnyWindowHandle, cx: &mut AsyncApp) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, SW_SHOWNOACTIVATE, ShowWindow};
    let Some(hwnd) = handle.update(cx, |_, window, _| native(window)).ok().and_then(|v| v.as_i64()) else { return false };
    let hwnd = hwnd as isize as windows_sys::Win32::Foundation::HWND;
    if unsafe { IsIconic(hwnd) } == 0 {
        return false;
    }
    drive::note(format!("restored minimized window {}", handle.window_id().as_u64()));
    unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    true
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn pump(_: AnyWindowHandle, _: &mut AsyncApp) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn capture(handle: AnyWindowHandle, path: &std::path::Path, full: bool, cx: &mut AsyncApp) -> Result<Value, String> {
    let number = handle.update(cx, |_, window, _| native(window)).map_err(|e| e.to_string())?.as_u64().ok_or("no native window")?;
    mac::capture(u32::try_from(number).map_err(|e| e.to_string())?, path, full)?;
    Ok(json!({}))
}

#[cfg(not(target_os = "macos"))]
fn capture(_: AnyWindowHandle, _: &std::path::Path, _: bool, _: &mut AsyncApp) -> Result<Value, String> {
    Err("the app captures itself only on macOS".into())
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::path::Path;

    use gpui_kit::Window;
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSString, NSURL};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        static CGRectNull: CGRect;
        fn CGWindowListCreateImage(bounds: CGRect, list: u32, window: u32, image: u32) -> *mut c_void;
        fn CGImageRelease(image: *mut c_void);
    }

    #[link(name = "ImageIO", kind = "framework")]
    unsafe extern "C" {
        fn CGImageDestinationCreateWithURL(url: *const c_void, kind: *const c_void, count: usize, options: *const c_void) -> *mut c_void;
        fn CGImageDestinationAddImage(destination: *mut c_void, image: *mut c_void, properties: *const c_void);
        fn CGImageDestinationFinalize(destination: *mut c_void) -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: *const c_void);
    }

    pub fn view(window: &Window) -> Option<usize> {
        let Ok(RawWindowHandle::AppKit(h)) = HasWindowHandle::window_handle(window).map(|h| h.as_raw()) else { return None };
        Some(h.ns_view.as_ptr() as usize)
    }

    pub fn covered(view: usize) -> bool {
        unsafe {
            let ns_window: *mut AnyObject = msg_send![view as *mut AnyObject, window];
            if ns_window.is_null() {
                return false;
            }
            let state: usize = msg_send![ns_window, occlusionState];
            state & 2 == 0
        }
    }

    pub fn draw(view: usize) -> bool {
        unsafe {
            let view = view as *mut AnyObject;
            let layer: *mut AnyObject = msg_send![view, layer];
            if layer.is_null() {
                return false;
            }
            let _: () = msg_send![view, displayLayer: layer];
        }
        true
    }

    pub fn capture(window: u32, path: &Path, full: bool) -> Result<(), String> {
        unsafe {
            let image = CGWindowListCreateImage(CGRectNull, 8, window, 1 | if full { 8 } else { 16 });
            if image.is_null() {
                return Err("the window could not be captured".into());
            }
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            let kind = NSString::from_str("public.png");
            let destination = CGImageDestinationCreateWithURL(Retained::as_ptr(&url).cast(), Retained::as_ptr(&kind).cast(), 1, std::ptr::null());
            if destination.is_null() {
                CGImageRelease(image);
                return Err(format!("cannot write {}", path.display()));
            }
            CGImageDestinationAddImage(destination, image, std::ptr::null());
            let written = CGImageDestinationFinalize(destination);
            CFRelease(destination);
            CGImageRelease(image);
            if written { Ok(()) } else { Err(format!("cannot write {}", path.display())) }
        }
    }
}

async fn find(b: &Value, cx: &mut AsyncApp) -> Result<(AnyWindowHandle, Spot), String> {
    lookup(b, cx).await?
}

async fn lookup(b: &Value, cx: &mut AsyncApp) -> Result<Result<(AnyWindowHandle, Spot), String>, String> {
    let target = b["target"].as_str().ok_or("target missing")?;
    let nth = b["nth"].as_u64().unwrap_or(0) as usize;
    let handles = redraw(b, cx).await?;
    let mut hits = Vec::new();
    let mut near = Vec::new();
    for handle in &handles {
        for s in shown(*handle) {
            let v = spot_json(*handle, &s);
            if matches(target, &v) {
                hits.push((*handle, s));
            } else if matches(&format!("~{}", target.split_once([':', '~']).map_or(target, |(_, q)| q)), &v) {
                near.push(s.label.clone().map_or(s.id.to_string(), |l| format!("{} ({l})", s.id)));
            }
        }
    }
    let query = target.split_once([':', '~']).map_or(target, |(_, q)| q).to_lowercase();
    hits.sort_by_key(|(_, s)| (s.visible.is_none(), s.label.as_ref().is_none_or(|l| l.to_lowercase() != query) && s.id.to_lowercase() != query));
    let count = hits.len();
    Ok(hits.into_iter().nth(nth).ok_or_else(|| {
        near.truncate(12);
        if count > 0 {
            format!("{target}: only {count} matches")
        } else if near.is_empty() {
            format!("{target}: no such element; app tree lists them, and lists hold only the rows on screen, so scroll first")
        } else {
            format!("{target}: no such element; similar: {}", near.join(", "))
        }
    }))
}

async fn wait(b: &Value, cx: &mut AsyncApp) -> Result<Value, String> {
    let started = Instant::now();
    let limit = Duration::from_secs_f64(b["timeout"].as_f64().unwrap_or(10.).clamp(0., 3600.));
    let quiet = Duration::from_millis(b["quiet"].as_u64().unwrap_or(300));
    let what = b["for"].as_str().unwrap_or("idle");
    let mut trouble = None;
    loop {
        let done = match what {
            "idle" => {
                let last = WINDOWS.with_borrow(|w| w.values().filter_map(|d| d.last).max());
                started.elapsed() >= quiet && last.is_none_or(|l| l.elapsed() >= quiet)
            }
            "window" => pick_windows(b, cx).is_ok(),
            _ => match lookup(b, cx).await {
                Ok(found) => found.is_ok() == (what != "gone"),
                Err(_) if what == "gone" && pick_windows(b, cx).is_err() => true,
                Err(e) => {
                    trouble = Some(e);
                    false
                }
            },
        };
        if done {
            return Ok(json!({ "waited": started.elapsed().as_millis() as u64 }));
        }
        if started.elapsed() > limit {
            let frames = FRAMES.with_borrow(|f| f.times.len());
            let why = trouble.map_or_else(String::new, |e| format!("; {e}"));
            return Err(format!("still not {what} after {:.1} s ({frames} frames so far){why}", limit.as_secs_f64()));
        }
        if what == "idle" {
            for handle in cx.update(|cx| cx.windows()) {
                pump(handle, cx);
            }
        }
        cx.background_executor().timer(Duration::from_millis(if what == "idle" { 50 } else { 150 })).await;
    }
}

fn modifiers(b: &Value, os: OperatingSystem) -> Modifiers {
    let mut m = Modifiers::none();
    for part in b["mods"].as_str().unwrap_or_default().split(['+', '-']) {
        match part {
            "shift" => m.shift = true,
            "alt" | "option" => m.alt = true,
            "ctrl" | "control" => m.control = true,
            "platform" | "super" | "win" => m.platform = true,
            "cmd" | "mod" | "secondary" => {
                if os == OperatingSystem::Mac {
                    m.platform = true;
                } else {
                    m.control = true;
                }
            }
            _ => {}
        }
    }
    m
}

async fn point_of(b: &Value, prefix: &str, cx: &mut AsyncApp) -> Result<(AnyWindowHandle, Point<Pixels>, Option<Spot>), String> {
    let target = b[format!("{prefix}target")].as_str();
    if let Some(target) = target {
        let mut q = b.clone();
        q["target"] = json!(target);
        let (handle, spot) = find(&q, cx).await?;
        let area = spot.visible.ok_or_else(|| format!("{target} is outside the visible part of its window; scroll first"))?;
        return Ok((handle, area.center(), Some(spot)));
    }
    let (x, y) = (b[format!("{prefix}x")].as_f64().ok_or("x/y or a target is needed")?, b[format!("{prefix}y")].as_f64().ok_or("y missing")?);
    Ok((one_window(b, cx)?, point(px(x as f32), px(y as f32)), None))
}

fn send(handle: AnyWindowHandle, event: PlatformInput, cx: &mut AsyncApp) -> Result<(), String> {
    handle.update(cx, |_, window, cx| drop(window.dispatch_event(event, cx))).map_err(|e| e.to_string())
}

fn tracked(handle: AnyWindowHandle) -> Option<Pointer> {
    POINTER.with_borrow(|p| p.get(&handle.window_id().as_u64()).copied())
}

fn track(handle: AnyWindowHandle, at: Point<Pixels>, held: Option<MouseButton>) {
    POINTER.with_borrow_mut(|p| p.insert(handle.window_id().as_u64(), Pointer { at, held }));
}

async fn glide(handle: AnyWindowHandle, from: Point<Pixels>, to: Point<Pixels>, steps: u64, held: Option<MouseButton>, modifiers: Modifiers, cx: &mut AsyncApp) -> Result<(), String> {
    for i in 1..=steps {
        let position = from + (to - from) * (i as f32 / steps as f32);
        send(handle, PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: held, modifiers }), cx)?;
        settle(handle, cx).await;
    }
    track(handle, to, held);
    Ok(())
}

async fn pointer(cmd: &str, b: &Value, cx: &mut AsyncApp) -> Result<Value, String> {
    let os = cx.update(|cx| cx.platform_info().os);
    let modifiers = modifiers(b, os);
    let button = match b["button"].as_str() {
        Some("right") => MouseButton::Right,
        Some("middle") => MouseButton::Middle,
        _ => MouseButton::Left,
    };
    let prefix = if cmd == "drag" { "from_" } else { "" };
    let (handle, at, spot) = point_of(b, prefix, cx).await?;
    let last = tracked(handle);
    let held = last.and_then(|p| p.held);
    let from = last.map_or(at, |p| p.at);
    match (cmd, held) {
        ("click" | "drag" | "drop" | "down", Some(h)) => return Err(format!("the {h:?} button is still held; app up releases it")),
        ("up", None) => return Err("no button is held; app down presses one".into()),
        _ => {}
    }
    let steps = b["steps"].as_u64().unwrap_or(if cmd == "drag" { 10 } else { 1 }).max(1);
    let moved = |position, pressed_button| PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button, modifiers });
    let down = |position, click_count| PlatformInput::MouseDown(MouseDownEvent { button, position, modifiers, click_count, first_mouse: false });
    let up = |button, position, click_count| PlatformInput::MouseUp(MouseUpEvent { button, position, modifiers, click_count });
    let changed = modifiers != Modifiers::none();
    if changed {
        send(handle, PlatformInput::ModifiersChanged(ModifiersChangedEvent { modifiers, capslock: Capslock::default() }), cx)?;
    }
    match cmd {
        "move" => glide(handle, from, at, steps, held, modifiers, cx).await?,
        "down" => {
            send(handle, moved(at, None), cx)?;
            settle(handle, cx).await;
            send(handle, down(at, 1), cx)?;
            track(handle, at, Some(button));
        }
        "up" => {
            glide(handle, from, at, steps, held, modifiers, cx).await?;
            send(handle, up(held.unwrap_or(button), at, 1), cx)?;
            track(handle, at, None);
        }
        "click" => {
            send(handle, moved(at, None), cx)?;
            settle(handle, cx).await;
            for n in 1..=b["count"].as_u64().unwrap_or(1).max(1) as usize {
                send(handle, down(at, n), cx)?;
                send(handle, up(button, at, n), cx)?;
            }
            track(handle, at, None);
        }
        "wheel" => {
            send(handle, moved(at, held), cx)?;
            track(handle, at, held);
            let (dx, dy) = (b["dx"].as_f64().unwrap_or(0.) as f32, b["dy"].as_f64().unwrap_or(0.) as f32);
            for _ in 0..steps {
                let delta = if b["pixels"].as_bool().unwrap_or(false) { ScrollDelta::Pixels(point(px(dx), px(dy))) } else { ScrollDelta::Lines(point(dx, dy)) };
                send(handle, PlatformInput::Scroll(ScrollEvent { position: at, delta, modifiers, touch_phase: TouchPhase::Moved, source: ScrollSource::default() }), cx)?;
                settle(handle, cx).await;
            }
        }
        "drop" => {
            let paths: Vec<PathBuf> = b["paths"].as_array().map(|p| p.iter().filter_map(|p| p.as_str()).map(PathBuf::from).collect()).unwrap_or_default();
            send(handle, PlatformInput::FileDrop(FileDropEvent::Entered { position: at, paths: ExternalPaths(paths.into()) }), cx)?;
            settle(handle, cx).await;
            send(handle, PlatformInput::FileDrop(FileDropEvent::Pending { position: at }), cx)?;
            settle(handle, cx).await;
            send(handle, PlatformInput::FileDrop(FileDropEvent::Submit { position: at }), cx)?;
            track(handle, at, None);
        }
        "drag" => {
            let (to_handle, to, _) = point_of(b, "to_", cx).await?;
            if to_handle != handle {
                return Err("drag only works inside one window".into());
            }
            send(handle, moved(at, None), cx)?;
            settle(handle, cx).await;
            send(handle, down(at, 1), cx)?;
            glide(handle, at, to, steps, Some(button), modifiers, cx).await?;
            send(handle, up(button, to, 1), cx)?;
            track(handle, to, None);
        }
        _ => {}
    }
    if changed {
        send(handle, PlatformInput::ModifiersChanged(ModifiersChangedEvent { modifiers: Modifiers::none(), capslock: Capslock::default() }), cx)?;
    }
    settle(handle, cx).await;
    let at = json!([f32::from(at.x), f32::from(at.y)]);
    let held = tracked(handle).and_then(|p| p.held).map(|h| format!("{h:?}").to_lowercase());
    Ok(json!({ "window": handle.window_id().as_u64(), "at": at, "spot": spot.map(|s| spot_json(handle, &s)), "held": held }))
}
