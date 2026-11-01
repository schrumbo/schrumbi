use gpui_kit::component::WindowExt;
use std::path::{Path, PathBuf};

use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::markdown_view;
use super::quick::{Dismissed, QuickDown, QuickUp};
use super::state::{self, act};
use super::theme::{Theme, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::vault::{config::VaultConfig, index};

/// Whether a file belongs to the vault this workspace works with.
pub fn in_vault(path: &Path, cx: &App) -> bool {
    state::read(cx, |l| l.is_vault) && {
        let root = index::current().root.clone();
        !root.as_os_str().is_empty() && dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).starts_with(root)
    }
}

/// Opens a file in Obsidian, or says why that cannot work here.
pub fn open_in_obsidian(path: &Path, cx: &mut App) {
    use crate::core::vault::config::{self, Reach};
    let root = index::current().root.clone();
    match config::reach(&root, &config::known_vaults(), config::obsidian_installed()) {
        Reach::Known => cx.open_url(&config::open_uri(&dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))),
        Reach::UnknownVault => act(cx, |l| l.notify(tr("Obsidian kennt diesen Ordner noch nicht. Öffne ihn dort einmal mit „Ordner als Vault öffnen“."), "info", None, None)),
        Reach::NotInstalled => act(cx, |l| l.notify(tr("Obsidian ist auf diesem Gerät nicht installiert."), "info", None, None)),
    }
}

/// The button that hands a file over to Obsidian.
pub fn obsidian_button(id: &'static str, path: PathBuf) -> Button {
    Button::new(id).small().icon(menu_icon("external-link")).label("Obsidian").tooltip(tr("In Obsidian öffnen")).on_click(move |_, _, cx| open_in_obsidian(&path, cx))
}

/// What choosing a note in the picker does.
#[derive(Clone)]
pub enum Pick {
    InsertTemplate(PathBuf),
    MergeInto(PathBuf),
}

/// Searches notes for one purpose, with a preview of the selected one.
pub struct NotePicker {
    pick: Pick,
    query: Entity<InputState>,
    items: Vec<PathBuf>,
    index: usize,
    focused: bool,
    preview: Option<(PathBuf, String)>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<Dismissed> for NotePicker {}

fn templates(root: &Path) -> Vec<PathBuf> {
    let settings = VaultConfig::read(root);
    let folder = settings.templates.folder.trim_matches('/');
    if folder.is_empty() {
        return Vec::new();
    }
    let vault = index::current();
    let base = root.join(folder);
    let mut list: Vec<PathBuf> = vault.notes.keys().filter(|p| p.starts_with(&base) && crate::core::markdown::is_markdown(p)).cloned().collect();
    list.sort();
    list
}

impl NotePicker {
    pub fn new(pick: Pick, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Notiz suchen")));
        let subs = vec![cx.subscribe(&query, |this, _, e: &InputEvent, cx| match e {
            InputEvent::Change => {
                this.index = 0;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.choose(cx),
            _ => {}
        })];
        let vault = index::current();
        let items: Vec<PathBuf> = match &pick {
            Pick::InsertTemplate(_) => templates(&vault.root),
            Pick::MergeInto(source) => {
                let mut list: Vec<PathBuf> = vault.notes.keys().filter(|p| *p != source && crate::core::markdown::is_markdown(p)).cloned().collect();
                list.sort();
                list
            }
        };
        Self { pick, query, items, index: 0, focused: false, preview: None, _subs: subs }
    }

    fn shown(&self, cx: &App) -> Vec<PathBuf> {
        let filter = self.query.read(cx).value().to_lowercase();
        self.items.iter().filter(|p| filter.is_empty() || p.to_string_lossy().to_lowercase().contains(&filter)).take(200).cloned().collect()
    }

    fn choose(&mut self, cx: &mut Context<Self>) {
        let Some(chosen) = self.shown(cx).get(self.index).cloned() else { return };
        cx.emit(Dismissed);
        match self.pick.clone() {
            Pick::InsertTemplate(note) => act(cx, |l| l.insert_template(&note, &chosen)),
            Pick::MergeInto(source) => {
                act(cx, |l| l.merge_note(&source, &chosen));
                state::reindex(cx);
            }
        }
    }
}

impl Render for NotePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            self.focused = true;
            self.query.update(cx, |s, cx| s.focus(window, cx));
        }
        let t = Theme::get(cx);
        let items = self.shown(cx);
        self.index = self.index.min(items.len().saturating_sub(1));
        let selected_path = items.get(self.index).cloned();
        if let Some(path) = &selected_path
            && self.preview.as_ref().is_none_or(|(p, _)| p != path)
        {
            self.preview = Some((path.clone(), std::fs::read_to_string(path).unwrap_or_default()));
        }
        let me = cx.entity();
        let root = index::current().root.clone();
        let rows = items.iter().enumerate().map(|(i, path)| {
            let selected = i == self.index;
            let e = me.clone();
            let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let folder = path.parent().and_then(|p| p.strip_prefix(&root).ok()).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            div()
                .id(("pick", i))
                .px(px(space::M))
                .py(px(space::XS))
                .rounded(px(radius::INNER))
                .flex()
                .flex_col()
                .cursor_pointer()
                .key_nav()
                .when(selected, |d| d.bg(t.accent).text_color(white()))
                .child(div().truncate().child(name))
                .when(!folder.is_empty(), |d| d.child(div().truncate().text_size(px(font::CAPTION)).text_color(if selected { white().opacity(0.85) } else { t.secondary }).child(folder)))
                .on_click(move |_, _, cx| {
                    e.update(cx, |q, cx| {
                        q.index = i;
                        q.choose(cx);
                    })
                })
        });
        let empty_hint = match self.pick {
            Pick::InsertTemplate(_) => tr("Lege in Obsidian einen Vorlagenordner fest und speichere dort Notizen."),
            Pick::MergeInto(_) => tr("Es gibt keine andere Notiz."),
        };
        let preview_text = self.preview.as_ref().map_or(String::new(), |(_, s)| s.chars().take(2000).collect());
        div()
            .w(px(720.))
            .h(px(440.))
            .flex()
            .flex_col()
            .key_context("QuickOpen")
            .on_action(cx.listener(|q, _: &QuickDown, _, cx| {
                q.index += 1;
                cx.notify();
            }))
            .on_action(cx.listener(|q, _: &QuickUp, _, cx| {
                q.index = q.index.saturating_sub(1);
                cx.notify();
            }))
            .child(div().flex().items_center().gap(px(space::S)).p(px(space::L)).child(icon("search", iconsize::M, t.secondary)).child(div().flex_1().child(Input::new(&self.query).appearance(false))))
            .child(div().h(px(1.)).bg(t.separator))
            .child(if items.is_empty() {
                unavailable("file-question", tr("Keine Treffer"), empty_hint, &t).flex_1().into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(div().id("pick-list").w(px(280.)).flex_none().h_full().overflow_y_scrollbar().child(div().p(px(space::S)).flex().flex_col().gap(px(space::XXS)).children(rows)))
                    .child(div().id("pick-preview").flex_1().min_w_0().h_full().overflow_y_scrollbar().border_l_1().border_color(t.separator).child(div().p(px(space::M)).text_size(px(font::CALLOUT)).text_color(t.secondary).child(preview_text)))
                    .into_any_element()
            })
    }
}

/// Shows the note as slides over the window; `---` separates them.
pub struct Slides {
    parts: Vec<String>,
    base: Option<PathBuf>,
    at: usize,
    focus: FocusHandle,
    focused: bool,
}

impl Slides {
    pub fn new(path: &Path, cx: &mut Context<Self>) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        Self { parts: split_slides(&text), base: path.parent().map(PathBuf::from), at: 0, focus: cx.focus_handle(), focused: false }
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        self.at = self.at.saturating_add_signed(by).min(self.parts.len().saturating_sub(1));
        cx.notify();
    }
}

/// Splits a note at `---` lines outside code, without its front matter.
pub fn split_slides(text: &str) -> Vec<String> {
    let body = &text[crate::core::vault::frontmatter::body_offset(text)..];
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut fence = false;
    for line in body.split_inclusive('\n') {
        let bare = line.trim();
        if bare.starts_with("```") {
            fence = !fence;
        }
        if !fence && bare == "---" {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push_str(line);
        }
    }
    parts.push(current);
    parts.retain(|p| !p.trim().is_empty());
    if parts.is_empty() {
        parts.push(String::new());
    }
    parts
}

impl Render for Slides {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus.contains_focused(window, cx) && !window.has_active_dialog(cx) {
            self.focused = true;
            window.focus(&self.focus, cx);
        }
        let t = Theme::get(cx);
        let total = self.parts.len();
        let source = self.parts.get(self.at).cloned().unwrap_or_default();
        let note = markdown_view::parse(&source);
        let shown = markdown_view::render(&note, self.base.as_deref(), None, &t);
        div()
            .id("slides")
            .track_focus(&self.focus)
            .absolute()
            .inset_0()
            .bg(t.content)
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, _, cx| {
                match e.keystroke.key.as_str() {
                    "right" | "down" | "space" | "enter" => s.step(1, cx),
                    "left" | "up" | "backspace" => s.step(-1, cx),
                    "escape" => act(cx, |l| l.slides = None),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(div().id("slide-body").flex_1().min_h_0().overflow_y_scroll().flex().justify_center().child(div().w_full().max_w(px(860.)).px(px(space::XXL)).py(px(space::XXL)).text_size(px(font::TITLE2)).child(shown)))
            .child(div().flex_none().px(px(space::L)).py(px(space::S)).flex().justify_between().child(caption(tr("Esc beendet"), &t)).child(caption(trf("%@ / %@", &[&(self.at + 1).to_string(), &total.to_string()]), &t).font_features(tabular())))
    }
}

#[cfg(test)]
mod tests {
    use super::split_slides;

    #[test]
    fn slides_split_outside_code_and_skip_front_matter() {
        let text = "---\ntitle: x\n---\n# Eins\n---\nZwei\n```\n---\n```\n---\n\n---\nDrei";
        let parts = split_slides(text);
        assert_eq!(parts.len(), 3);
        assert!(parts[0].contains("# Eins"));
        assert!(parts[1].contains("```\n---\n```"));
        assert_eq!(parts[2], "Drei");
        assert_eq!(split_slides("").len(), 1);
    }
}
