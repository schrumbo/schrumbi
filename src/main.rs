#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod core;
mod ui;

use std::borrow::Cow;

use gpui_kit::{App, AssetSource, SharedString};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
#[exclude = "i18n/*"]
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if let Some(f) = Self::get(path) {
            return Ok(Some(f.data));
        }
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut out: Vec<SharedString> = Self::iter().filter(|p| p.starts_with(path)).map(SharedString::from).collect();
        out.extend(gpui_kit::assets::AllAssets.list(path)?);
        Ok(out)
    }
}

fn main() {
    core::i18n::init(&core::prefs::get().language);
    let workspace = core::workspace::resolve();
    let files: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(std::path::PathBuf::from).filter(|p| core::markdown::is_markdown(p) && p.is_file()).collect();
    let (tx, rx) = async_channel::unbounded::<Vec<String>>();
    let app = gpui_kit::application().with_assets(Assets);
    app.on_open_urls(move |urls| {
        let _ = tx.try_send(urls);
    });
    app.on_reopen(|cx| ui::main_window::show(cx));
    app.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        ui::theme::apply(false, None, cx);
        ui::state::init(cx);
        ui::app::init(cx);
        if let Some(path) = workspace {
            ui::state::open_workspace(&path, cx);
        }
        ui::main_window::open(cx);
        ui::tray::init(cx);
        for f in &files {
            ui::state::act(cx, |l| l.open_document(f));
        }
        cx.spawn(async move |cx| {
            while let Ok(urls) = rx.recv().await {
                let _ = cx.update(|cx| {
                    for u in urls {
                        if let Some(p) = url::Url::parse(&u).ok().and_then(|u| u.to_file_path().ok()).filter(|p| core::markdown::is_markdown(p)) {
                            ui::state::act(cx, |l| l.open_document(&p));
                        }
                    }
                });
            }
        })
        .detach();
        cx.activate(true);
    });
}
