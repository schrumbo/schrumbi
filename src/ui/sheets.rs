use gpui_kit::base::actions::Cancel;
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::filing_form::{FilingEvent, FilingForm};
use super::palette::{Palette, ShortcutSheet};
use super::quick::{Dismissed, NewNote, QuickAdd, QuickOpen};
use super::wizards::{ImportCalendar, Reschedule, Setup};
use crate::core::i18n::tr;
use crate::core::library::Library;

/// Opens the sheet the library asks for (filing, quick add, quick open, new note, setup, import, reschedule).
pub fn present(lib: &Entity<Library>, window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let pending = {
        let l = lib.read(cx);
        l.show_save_layout || l.note_pick.is_some() || l.show_sync_setup || l.show_conflicts || l.filing_request.is_some() || l.show_quick_add || l.show_quick_open || l.link_pick.is_some() || l.show_palette || l.show_shortcuts || l.new_note.is_some() || l.import_path.is_some() || l.reschedule_id.is_some() || l.needs_setup
    };
    if pending {
        super::drive::activate_window(window);
        if close_popovers(window, cx) {
            let lib = lib.clone();
            window.on_next_frame(move |_, cx| lib.update(cx, |_, cx| cx.notify()));
            return;
        }
    }
    if lib.update(cx, |l, _| std::mem::take(&mut l.show_save_layout)) {
        let view = cx.new(|cx| super::quick::SaveLayout::new(window, cx));
        show(view, 420., window, cx);
        return;
    }
    let (sync_setup, conflicts) = lib.update(cx, |l, _| (std::mem::take(&mut l.show_sync_setup), std::mem::take(&mut l.show_conflicts)));
    if sync_setup {
        let view = cx.new(|cx| super::sync_panel::SyncSetup::new(window, cx));
        show_with(view, 760., false, false, window, cx);
        return;
    }
    if conflicts {
        let view = cx.new(|cx| super::sync_panel::ConflictSheet::new(window, cx));
        show(view, 760., window, cx);
        return;
    }
    let request = lib.update(cx, |l, _| l.filing_request.take());
    if let Some(r) = request {
        let form = cx.new(|cx| FilingForm::new(r.paths, r.assignment_id, true, window, cx));
        let handle = window.window_handle();
        cx.subscribe(&form, move |_, _: &FilingEvent, cx| {
            let _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
            super::state::return_focus_to_page(cx);
        })
        .detach();
        window.open_dialog(cx, move |d, _, _| d.entrance(DialogEntrance::FadeSlide).w(px(560.)).h(px(600.)).p_0().on_ok(pass_enter).child(form.clone()));
        return;
    }
    if let Some((merge, note)) = lib.update(cx, |l, _| l.note_pick.take()) {
        let pick = if merge { super::vault_tools::Pick::MergeInto(note) } else { super::vault_tools::Pick::InsertTemplate(note) };
        let view = cx.new(|cx| super::vault_tools::NotePicker::new(pick, window, cx));
        show(view, 720., window, cx);
        return;
    }
    if let Some(id) = lib.update(cx, |l, _| l.link_pick.take()) {
        let view = cx.new(|cx| QuickOpen::linking(id, window, cx));
        show_picker(view, 620., window, cx);
        return;
    }
    let (palette, shortcuts) = lib.update(cx, |l, _| (std::mem::take(&mut l.show_palette), std::mem::take(&mut l.show_shortcuts)));
    if palette {
        let view = cx.new(|cx| Palette::new(window, cx));
        show_picker(view, 620., window, cx);
        return;
    }
    if shortcuts {
        let view = cx.new(|_| ShortcutSheet);
        show(view, 760., window, cx);
        return;
    }
    let (quick_add, quick_open, new_note, module, import, reschedule, setup, only) = lib.update(cx, |l, _| {
        let module = if let crate::core::library::Route::Module(id) = l.route() { Some(id.clone()) } else { None };
        let setup = std::mem::take(&mut l.needs_setup);
        ((std::mem::take(&mut l.show_quick_add), l.quick_add_kind.take(), l.quick_add_day.take()), std::mem::take(&mut l.show_quick_open), l.new_note.take(), module, l.import_path.take(), l.reschedule_id.take(), setup, l.setup_only.take())
    });
    if setup {
        let view = cx.new(|cx| {
            let setup = Setup::new(window, cx);
            match only {
                Some(step) => setup.only(step),
                None => setup,
            }
        });
        let handle = window.window_handle();
        cx.subscribe(&view, move |_, _: &Dismissed, cx| {
            let _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
            super::state::return_focus_to_page(cx);
        })
        .detach();
        window.open_dialog(cx, move |d, window, _| {
            let leaving = view.clone();
            let room = f32::from(window.viewport_size().width) - 2. * super::theme::space::L;
            d.entrance(DialogEntrance::FadeSlide).w(px(760f32.min(room).max(0.))).p_0().overlay_closable(false).close_button(false).on_ok(pass_enter).on_cancel(move |_, _, cx| {
                leaving.update(cx, |s, cx| s.leave(cx));
                false
            }).child(view.clone())
        });
    } else if let Some(p) = import {
        let view = cx.new(|cx| ImportCalendar::new(&p, cx));
        show(view, 640., window, cx);
    } else if let Some(id) = reschedule {
        crate::core::prefs::mark_used("reschedule");
        let view = cx.new(|cx| Reschedule::new(id, window, cx));
        show(view, 400., window, cx);
    } else if let Some(r) = new_note {
        let view = cx.new(|cx| NewNote::new(r.module_id, r.folder, window, cx));
        show(view, 540., window, cx);
    } else if let (true, kind, day) = quick_add {
        if day.is_some() {
            crate::core::prefs::mark_used("day_entry");
        }
        let view = cx.new(|cx| QuickAdd::new(module, kind, day, window, cx));
        show_with(view, 520., false, false, window, cx);
    } else if quick_open {
        let view = cx.new(|cx| QuickOpen::new(window, cx));
        show_picker(view, 620., window, cx);
    }
}

/// Asks before a change that cannot simply be taken back. Focus starts on Abbrechen and Enter only clicks the focused button, so Enter alone never runs `ok`.
pub fn ask(window: &mut Window, cx: &mut App, title: String, message: String, ok_text: String, danger: bool, ok: impl Fn(&mut App) + 'static) {
    let ok = std::rc::Rc::new(ok);
    let cancel = cx.focus_handle();
    let first = cancel.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let ok = ok.clone();
        let confirm = Button::new("alert-ok").label(ok_text.clone()).on_click(move |_, window, cx| {
            window.close_dialog(cx);
            ok(cx);
        });
        alert.title(title.clone()).description(message.clone()).on_ok(pass_enter).footer(
            gpui_kit::component::dialog::DialogFooter::new()
                .child(Button::new("alert-cancel").label(tr("Abbrechen")).track_focus(&cancel).on_click(|_, window, cx| window.close_dialog(cx)))
                .child(if danger { confirm.danger() } else { confirm.primary() }),
        )
    });
    first.focus(window, cx);
}

/// Lets Enter reach the focused field, button or link instead of closing the dialog around it.
pub fn pass_enter(_: &ClickEvent, _: &mut Window, cx: &mut App) -> bool {
    cx.propagate();
    false
}

/// Whether a text field has focus, so Enter there belongs to the sheet's main action rather than to a focused button.
pub fn typing(window: &Window) -> bool {
    window.context_stack().iter().any(|c| c.contains("Input"))
}

/// Closes a menu or popover that has the keyboard, so a sheet opened by a shortcut does not stack on top of it.
pub fn close_popovers(window: &mut Window, cx: &mut App) -> bool {
    let open = window.context_stack().iter().any(|c| c.contains("PopupMenu"));
    if open {
        window.dispatch_action(Box::new(Cancel), cx);
    }
    open
}

/// Presents a sheet in a dialog that closes when the sheet emits `Dismissed`.
fn show<V: Render + EventEmitter<Dismissed>>(view: Entity<V>, width: f32, window: &mut Window, cx: &mut App) {
    show_with(view, width, true, false, window, cx);
}

/// Presents a search sheet without a close button, below the title bar and never wider than the window.
fn show_picker<V: Render + EventEmitter<Dismissed>>(view: Entity<V>, width: f32, window: &mut Window, cx: &mut App) {
    show_with(view, width, false, true, window, cx);
}

fn show_with<V: Render + EventEmitter<Dismissed>>(view: Entity<V>, width: f32, close_button: bool, below_bar: bool, window: &mut Window, cx: &mut App) {
    let handle = window.window_handle();
    cx.subscribe(&view, move |_, _: &Dismissed, cx| {
        let _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
        super::state::return_focus_to_page(cx);
    })
    .detach();
    window.open_dialog(cx, move |d, window, _| {
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        let d = d.entrance(DialogEntrance::FadeSlide).w(px(width)).p_0().close_button(close_button).on_ok(pass_enter).child(view.clone());
        if below_bar { d.w(px(width.min(w - 2. * super::theme::space::L).max(0.))).margin_top(px(super::quick::dialog_top(h))) } else { d }
    });
}

/// Opens a sheet built on the spot, for pages with their own forms.
pub fn show_sheet<V: Render + EventEmitter<Dismissed>>(width: f32, window: &mut Window, cx: &mut App, build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static) {
    if close_popovers(window, cx) {
        window.on_next_frame(move |window, cx| {
            let view = cx.new(|cx| build(window, cx));
            show(view, width, window, cx);
        });
        return;
    }
    let view = cx.new(|cx| build(window, cx));
    show(view, width, window, cx);
}
