use gpui_kit::component::WindowExt;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::filing_form::{FilingEvent, FilingForm};
use super::quick::{Dismissed, NewNote, QuickAdd, QuickOpen};
use super::wizards::{ImportCalendar, Reschedule, Setup};
use crate::core::library::Library;

/// Opens the sheet the library asks for (filing, quick add, quick open, new note, setup, import, reschedule).
pub fn present(lib: &Entity<Library>, window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let pending = {
        let l = lib.read(cx);
        l.filing_request.is_some() || l.show_quick_add || l.show_quick_open || l.new_note.is_some() || l.import_path.is_some() || l.reschedule_id.is_some() || l.needs_setup
    };
    if pending {
        window.activate_window();
    }
    let request = lib.update(cx, |l, _| l.filing_request.take());
    if let Some(r) = request {
        let form = cx.new(|cx| FilingForm::new(r.paths, r.assignment_id, window, cx));
        let handle = window.window_handle();
        cx.subscribe(&form, move |_, _: &FilingEvent, cx| {
            let _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
        })
        .detach();
        window.open_dialog(cx, move |d, _, _| d.w(px(560.)).h(px(600.)).p_0().child(form.clone()));
        return;
    }
    let (quick_add, quick_open, new_note, module, import, reschedule, setup) = lib.update(cx, |l, _| {
        let module = if let crate::core::library::Route::Module(id) = l.route() { Some(id.clone()) } else { None };
        let setup = std::mem::take(&mut l.needs_setup);
        (std::mem::take(&mut l.show_quick_add), std::mem::take(&mut l.show_quick_open), l.new_note.take(), module, l.import_path.take(), l.reschedule_id.take(), setup)
    });
    if setup {
        let view = cx.new(|cx| Setup::new(window, cx));
        show(view, 600., window, cx);
    } else if let Some(p) = import {
        let view = cx.new(|cx| ImportCalendar::new(&p, cx));
        show(view, 640., window, cx);
    } else if let Some(id) = reschedule {
        let view = cx.new(|cx| Reschedule::new(id, window, cx));
        show(view, 400., window, cx);
    } else if let Some(r) = new_note {
        let view = cx.new(|cx| NewNote::new(r.module_id, r.folder, window, cx));
        show(view, 540., window, cx);
    } else if quick_add {
        let view = cx.new(|cx| QuickAdd::new(module, window, cx));
        show(view, 520., window, cx);
    } else if quick_open {
        let view = cx.new(|cx| QuickOpen::new(window, cx));
        show(view, 620., window, cx);
    }
}

/// Presents a sheet in a dialog that closes when the sheet emits `Dismissed`.
fn show<V: Render + EventEmitter<Dismissed>>(view: Entity<V>, width: f32, window: &mut Window, cx: &mut App) {
    let handle = window.window_handle();
    cx.subscribe(&view, move |_, _: &Dismissed, cx| {
        let _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
    })
    .detach();
    window.open_dialog(cx, move |d, _, _| d.w(px(width)).p_0().child(view.clone()));
}
