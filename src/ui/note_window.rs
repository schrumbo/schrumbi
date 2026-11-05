use std::path::Path;

use gpui_kit::base::Root;
use gpui_kit::*;

use super::documents::{self, NoteView};
use super::state::{self, act};

fn note_path(window: &mut Window, cx: &mut App) -> Option<std::path::PathBuf> {
    let root = window.root::<Root>().flatten()?;
    let view = root.read(cx).view().clone();
    view.downcast::<NoteView>().ok().map(|n| n.read(cx).path().to_path_buf())
}

fn forget(path: &Path, cx: &mut App) {
    let path = path.to_path_buf();
    act(cx, |l| {
        l.note_windows.remove(&path);
        if !l.open_documents.contains(&path) {
            l.buffers.remove(&path);
        }
    });
}

/// Saves or asks about the note, then closes its window.
pub fn request_close(window: &mut Window, cx: &mut App) {
    let Some(path) = note_path(window, cx) else {
        window.remove_window();
        return;
    };
    let handle = window.window_handle();
    documents::resolve_unsaved(vec![path.clone()], cx, move |cx| {
        forget(&path, cx);
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        });
    });
}

/// Opens a note in its own window, or focuses the window already showing it.
pub fn open(path: &Path, cx: &mut App) {
    let path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    for w in cx.windows() {
        if let Some(h) = w.downcast::<Root>()
            && h.update(cx, |root, window, cx| {
                let found = root.view().clone().downcast::<NoteView>().ok().is_some_and(|n| n.read(cx).path() == path);
                if found {
                    super::drive::activate_window(window);
                }
                found
            })
            .unwrap_or(false)
        {
            return;
        }
    }
    let title = state::read(cx, |l| l.note_title(&path));
    let title_for_bar = title.clone();
    let target = path.clone();
    state::lib(cx).update(cx, |l, _| {
        l.note_windows.insert(path.clone());
    });
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(760.), px(640.)), cx))),
        window_min_size: Some(size(px(420.), px(320.))),
        ..Default::default()
    };
    let options = super::chrome::window_options(options, title_for_bar);
    let _ = super::chrome::open_window(options, cx, |window, cx| {
        let id = window.window_handle().window_id();
        window.on_window_should_close(cx, |window, cx| {
            request_close(window, cx);
            false
        });
        cx.new(|cx| {
            cx.observe_window_activation(window, move |_, window, cx| super::app::note_window_front(id, window.is_window_active(), cx)).detach();
            NoteView::new(target, false, window, cx)
        })
    });
}
