use std::time::Duration;

use gpui_kit::App;

use super::state::{self, act};
use crate::core::library::Route;

/// Debug builds only: with `SCHRUMBI_SMOKE` set, opens every page and window once and quits.
/// CI runs this on all platforms, so a crash at startup or while rendering fails the build.
pub fn run(cx: &mut App) {
    if !cfg!(debug_assertions) || std::env::var_os("SCHRUMBI_SMOKE").is_none() {
        return;
    }
    let root = std::env::temp_dir().join(format!("schrumbi-smoke-{}", std::process::id()));
    let note = root.join("M1/Notiz.md");
    std::fs::create_dir_all(root.join("M1/W1/Abgabe")).expect("smoke folder");
    std::fs::write(&note, "# Notiz\n\n- [ ] Aufgabe\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n").expect("smoke note");
    std::fs::write(root.join("M1/W1/Abgabe/Text.txt"), "Text").expect("smoke file");
    let today = chrono::Local::now().date_naive();
    let config = serde_json::json!({
        "version": 2, "name": "Smoke", "author": "Muster-M", "inbox": "_Inbox",
        "semesters": [{ "id": "s", "name": "WS", "start": (today - chrono::Days::new(30)).to_string(), "end": (today + chrono::Days::new(60)).to_string(), "breaks": [], "holidays": [] }],
        "modules": [{ "id": "m1", "code": "M1", "name": "Modul", "semester": "s", "folder": "M1", "color": "blue", "naming": "{code}_{yy}_{title}_{author}_{date}",
            "rules": [{ "id": "r", "title": "Abgabe", "short": "A", "weekday": 3, "time": "18:00", "interval": 1, "upload": true, "folder": "W{w}/Abgabe", "filePattern": "*" }] }],
    });
    std::fs::write(root.join("schrumbi.json"), config.to_string()).expect("smoke config");
    state::open_workspace(&root, cx);
    cx.spawn(async move |cx| {
        let pause = Duration::from_millis(500);
        cx.background_executor().timer(pause).await;
        for route in [Route::Today, Route::Grid, Route::Record, Route::Inbox, Route::Search, Route::Cards, Route::Module("m1".into())] {
            cx.update(|cx| act(cx, |l| l.set_route(route)));
            cx.background_executor().timer(pause).await;
        }
        let cards_have_keys = |cx: &mut App| {
            cx.windows().into_iter().find(|h| super::main_window::is_main(h.window_id(), cx)).and_then(|h| h.update(cx, |_, window, _| window.context_stack().iter().any(|c| c.contains("CardsPage"))).ok()).unwrap_or(false)
        };
        cx.update(|cx| act(cx, |l| l.set_route(Route::Cards)));
        cx.background_executor().timer(pause).await;
        let entered = cx.update(cards_have_keys);
        cx.update(|cx| act(cx, |l| l.set_route(Route::Notes)));
        cx.background_executor().timer(pause).await;
        let left = cx.update(cards_have_keys);
        if !entered || left {
            println!("smoke failed: the cards page {}", if left { "kept the keyboard after another page opened" } else { "never took the keyboard" });
            cx.update(|cx| cx.quit());
            return;
        }
        let steps: Vec<Box<dyn FnOnce(&mut App)>> = {
            let (a, b, c) = (note.clone(), note.clone(), note.clone());
            vec![
                Box::new(move |cx| act(cx, |l| l.open_document(&a))),
                Box::new(|cx| {
                    crate::core::prefs::update(|p| {
                        p.ai.custom_command = "schrumbi-smoke-no-agent".into();
                        p.ai.agent = "custom".into();
                    });
                    super::ai::set_enabled(true, cx);
                    super::ai::open_column(cx);
                }),
                Box::new(super::ai::toggle_details),
                Box::new(super::ai::open_column),
                Box::new(super::ai::close_panel),
                Box::new(super::settings::open),
                Box::new(super::settings::open_about),
                Box::new(move |cx| super::note_window::open(&b, cx)),
                Box::new(move |cx| super::preview::show(&c, cx)),
                Box::new(|cx| {
                    if let Some(id) = state::read(cx, |l| l.assignments.first().map(|a| a.id.clone())) {
                        super::upload::begin(&id, cx);
                    }
                }),
                Box::new(super::main_window::show),
            ]
        };
        for step in steps {
            cx.update(step);
            cx.background_executor().timer(pause).await;
        }
        println!("smoke ok");
        let _ = std::fs::remove_dir_all(&root);
        cx.update(|cx| cx.quit());
    })
    .detach();
}
