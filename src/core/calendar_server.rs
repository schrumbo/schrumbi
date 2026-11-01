//! Serves the calendar file to calendar apps on this computer, which on Windows and Linux often subscribe only to addresses.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static SERVED: LazyLock<Mutex<Option<(u16, PathBuf)>>> = LazyLock::new(Default::default);
const FIRST_PORT: u16 = 47821;

fn answer(mut stream: TcpStream, token: &str, file: &PathBuf) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut request = [0u8; 2048];
    let read = stream.read(&mut request).unwrap_or(0);
    let head = String::from_utf8_lossy(&request[..read]);
    let wanted = format!("GET /{token}/Schrumbi-Kalender.ics ");
    let local = head.lines().any(|l| {
        let l = l.to_lowercase();
        l.starts_with("host: 127.0.0.1") || l.starts_with("host: localhost")
    });
    let body = (head.starts_with(&wanted) && local).then(|| std::fs::read(file).ok()).flatten();
    let reply = match body {
        Some(body) => [format!("HTTP/1.1 200 OK\r\nContent-Type: text/calendar; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes(), body].concat(),
        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
    };
    let _ = stream.write_all(&reply);
}

/// Serves this file from now on and returns the address a calendar app can subscribe to, while the app runs.
pub fn serve(file: PathBuf) -> Option<String> {
    let mut token = super::prefs::get().calendar_token;
    if token.is_empty() {
        token = super::model::new_id().replace('-', "");
        let saved = token.clone();
        super::prefs::update(|p| p.calendar_token = saved);
    }
    let mut served = SERVED.lock().unwrap();
    if let Some((port, known)) = served.as_mut() {
        known.clone_from(&file);
        return Some(format!("http://127.0.0.1:{port}/{token}/Schrumbi-Kalender.ics"));
    }
    let (listener, port) = (FIRST_PORT..FIRST_PORT + 20).find_map(|port| TcpListener::bind(("127.0.0.1", port)).ok().map(|l| (l, port)))?;
    *served = Some((port, file));
    let serving_token = token.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let file = SERVED.lock().unwrap().as_ref().map(|s| s.1.clone());
            if let Some(file) = file {
                answer(stream, &serving_token, &file);
            }
        }
    });
    Some(format!("http://127.0.0.1:{port}/{token}/Schrumbi-Kalender.ics"))
}

/// The subscription address, once serving started.
pub fn address() -> Option<String> {
    let token = super::prefs::get().calendar_token;
    SERVED.lock().unwrap().as_ref().map(|(port, _)| format!("http://127.0.0.1:{port}/{token}/Schrumbi-Kalender.ics"))
}
