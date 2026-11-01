use gpui_kit::{Action, KeyBinding};

use super::app::kb;
use super::documents::*;
use crate::core::shortcuts::ShortcutAction as S;

fn bind<A: Action>(a: S, action: A) -> Option<KeyBinding> {
    a.shortcut().map(|s| kb(s.keystroke(), action, Some("NoteEditor > Input")))
}

/// Note editor shortcuts from the user's settings, plus list-aware Return and Tab.
pub fn key_bindings() -> Vec<KeyBinding> {
    let ctx = Some("NoteEditor > Input");
    [
        bind(S::Bold, Bold),
        bind(S::Italic, Italic),
        bind(S::Link, InsertLink),
        bind(S::InlineCode, InlineCode),
        bind(S::ToggleTask, ToggleTask),
        bind(S::ToggleComment, ToggleComment),
        bind(S::MoveLineUp, MoveLineUp),
        bind(S::MoveLineDown, MoveLineDown),
        bind(S::CopyLineUp, CopyLineUp),
        bind(S::CopyLineDown, CopyLineDown),
        bind(S::DeleteLine, DeleteLine),
        bind(S::SelectLine, SelectLine),
        bind(S::Indent, IndentLines),
        bind(S::Outdent, OutdentLines),
        bind(S::ZoomIn, ZoomIn),
        bind(S::ZoomOut, ZoomOut),
        bind(S::ZoomReset, ZoomReset),
        Some(kb(super::app::cmd("alt-f"), gpui_kit::component::input::Replace, ctx)),
        Some(kb("enter".into(), ListNewline, ctx)),
        Some(kb("shift-enter".into(), PlainNewline, ctx)),
        Some(kb(super::app::cmd("shift-x"), Strike, ctx)),
        Some(kb(super::app::cmd("shift-h"), Mark, ctx)),
        Some(kb(super::app::cmd("shift-m"), InlineMath, ctx)),
        Some(kb(super::app::cmd("shift-."), Quote, ctx)),
        Some(kb(super::app::cmd("shift-8"), BulletList, ctx)),
        Some(kb(super::app::cmd("shift-7"), NumberedList, ctx)),
        Some(kb(super::app::cmd("alt-1"), Heading1, ctx)),
        Some(kb(super::app::cmd("alt-2"), Heading2, ctx)),
        Some(kb(super::app::cmd("alt-3"), Heading3, ctx)),
        Some(kb("tab".into(), ListTab, ctx)),
        Some(kb("shift-tab".into(), ListBacktab, ctx)),
    ]
    .into_iter()
    .flatten()
    .collect()
}
