use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::help_example::{self, COPIED_FOR};
use super::markdown_view::{self, Folds, ParsedNote};
use super::motion::{self, Soft};
use super::theme::{Theme, control, font, iconsize, radius, space};
use super::widgets::*;
use crate::core::docs::{self, Doc, Group, Hit, PAGES, Piece};
use crate::core::i18n::{self, tr, trf};
use crate::core::prefs::Shortcut;
use crate::core::shortcuts::ShortcutAction;

const SIDEBAR: f32 = 232.;
const MEASURE: f32 = 680.;
const STEP: f32 = 48.;

struct LastPage(usize);

impl Global for LastPage {}

/// Opens the guide, on `slug` if given, else where it was left; an open guide comes to the front.
pub fn open(slug: Option<&str>, cx: &mut App) {
    let page = slug.and_then(|s| PAGES.iter().position(|p| p.slug == s));
    for w in cx.windows() {
        if let Some(h) = w.downcast::<gpui_kit::base::Root>()
            && h.update(cx, |root, window, cx| {
                let Ok(view) = root.view().clone().downcast::<Help>() else { return false };
                window.activate_window();
                if let Some(p) = page {
                    view.update(cx, |h, cx| h.go(p, None, cx));
                }
                true
            })
            .unwrap_or(false)
        {
            return;
        }
    }
    let page = page.or_else(|| cx.try_global::<LastPage>().map(|l| l.0)).unwrap_or(0);
    write_samples();
    let bounds = Bounds::centered(None, size(px(980.), px(700.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(720.), px(480.))),
        titlebar: Some(TitlebarOptions { title: Some(tr("Schrumbi-Anleitung").into()), appears_transparent: true, traffic_light_position: Some(point(px(14.), px(14.))) }),
        ..Default::default()
    };
    let options = super::chrome::window_options(options, tr("Schrumbi-Anleitung"));
    let _ = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Help::new(page, window, cx)));
}

/// Where the examples find the notes and images they link and embed; never inside the user's folder.
pub fn sample_dir() -> PathBuf {
    std::env::temp_dir().join("schrumbi-guide")
}

/// Whether a note lives in the guide's sample folder, whose links must do nothing.
pub fn is_sample(base: &Path) -> bool {
    base.starts_with(sample_dir())
}

/// How long a section found by search stays lit.
const FLASH: std::time::Duration = std::time::Duration::from_millis(1400);

const SAMPLE_NOTE: &str = "# Beispiel\n\nEine Notiz, auf die die Beispiele verlinken.\n\n## Zweiter Teil\n\nDieser Abschnitt erscheint dort, wo er eingebettet wird.\n";

const SAMPLE_NOTE_EN: &str = "# Example\n\nA note the examples link to.\n\n## Second part\n\nThis section appears wherever it is embedded.\n";

const SAMPLE_IMAGE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200" viewBox="0 0 320 200"><rect width="320" height="200" rx="16" fill="#0A84FF" fill-opacity="0.12"/><rect x="40" y="120" width="40" height="50" rx="6" fill="#0A84FF"/><rect x="100" y="90" width="40" height="80" rx="6" fill="#0A84FF"/><rect x="160" y="60" width="40" height="110" rx="6" fill="#0A84FF"/><rect x="220" y="30" width="40" height="140" rx="6" fill="#0A84FF"/></svg>"##;

fn write_samples() {
    let dir = sample_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    for (name, text) in [("Beispiel.md", SAMPLE_NOTE), ("Beispiel", SAMPLE_NOTE), ("Example.md", SAMPLE_NOTE_EN), ("Example", SAMPLE_NOTE_EN), ("Diagramm.svg", SAMPLE_IMAGE), ("Chart.svg", SAMPLE_IMAGE)] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
            let _ = std::fs::write(path, text);
        }
    }
}

fn binding_shortcut(binding: &KeyBinding) -> Option<Shortcut> {
    let [stroke] = binding.keystrokes() else { return None };
    let m = stroke.modifiers();
    let key = match stroke.key() {
        "enter" => "return",
        "backspace" => "delete",
        k => k,
    };
    Some(Shortcut { key: key.to_string(), command: m.platform, shift: m.shift, option: m.alt, control: m.control })
}

/// The keys for a `{key:…}` name: a configurable shortcut id, a fixed app action, or a plain key like `return`.
fn key_text(name: &str, cx: &App) -> Option<String> {
    if let Some(a) = ShortcutAction::ALL.iter().find(|a| a.id() == name) {
        return a.shortcut().map(|s| s.display());
    }
    if name.chars().next().is_some_and(char::is_uppercase) {
        let suffix = format!("::{name}");
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        return keymap.bindings().filter(|b| b.action().name().starts_with("schrumbi") && b.action().name().ends_with(&suffix)).find_map(binding_shortcut).map(|s| s.display());
    }
    docs::plain_key(name)
}

enum Item {
    Head,
    Text { note: Rc<ParsedNote>, section: usize },
    Example { index: usize, title: String, source: String, note: Rc<ParsedNote>, folds: Folds },
    Foot,
}

/// The guide window: pages in a sidebar, search across all of them, and the page in the reader.
pub struct Help {
    focus: FocusHandle,
    search: Entity<InputState>,
    docs: Vec<Doc>,
    page: usize,
    items: Vec<Item>,
    starts: Vec<usize>,
    list: ListState,
    hits: Vec<Hit>,
    hit: usize,
    sources: HashSet<(usize, usize)>,
    toggled: Option<(usize, usize)>,
    copied: Option<(usize, usize)>,
    copy_task: Option<Task<()>>,
    flash: Option<(usize, usize)>,
    back: Vec<usize>,
    forward: Vec<usize>,
    _subs: Vec<Subscription>,
}

impl Help {
    fn new(page: usize, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("In der Anleitung suchen")));
        let subs = vec![cx.subscribe_in(&search, window, |h, _, e: &InputEvent, window, cx| {
            match e {
                InputEvent::Change => {
                    h.hit = 0;
                    h.hits = docs::search(&h.docs, h.search.read(cx).value().trim());
                }
                InputEvent::PressEnter { .. } => h.open_hit(h.hit, window, cx),
                _ => {}
            }
            cx.notify();
        })];
        let lang = i18n::lang();
        let docs = PAGES.iter().map(|p| docs::split(&docs::expand(p.source(lang), &|name| key_text(name, cx)))).collect();
        let mut help = Self {
            focus: cx.focus_handle(),
            search,
            docs,
            page,
            items: Vec::new(),
            starts: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(800.)),
            hits: Vec::new(),
            hit: 0,
            sources: HashSet::new(),
            toggled: None,
            copied: None,
            copy_task: None,
            flash: None,
            back: Vec::new(),
            forward: Vec::new(),
            _subs: subs,
        };
        help.load();
        help
    }

    fn load(&mut self) {
        let doc = &self.docs[self.page];
        let mut items = vec![Item::Head];
        let mut starts = Vec::new();
        let mut example = 0;
        for (s, section) in doc.sections.iter().enumerate() {
            starts.push(items.len());
            for piece in &section.pieces {
                items.push(match piece {
                    Piece::Text(text) => Item::Text { note: Rc::new(markdown_view::parse(text)), section: s },
                    Piece::Example { title, source } => {
                        example += 1;
                        Item::Example { index: example - 1, title: title.clone(), source: source.clone(), note: Rc::new(markdown_view::parse(&sample_links(source))), folds: Folds::default() }
                    }
                });
            }
        }
        items.push(Item::Foot);
        self.list.reset(items.len());
        self.items = items;
        self.starts = starts;
    }

    fn go(&mut self, page: usize, section: Option<usize>, cx: &mut Context<Self>) {
        if page != self.page {
            self.back.push(self.page);
            self.forward.clear();
            self.show(page, cx);
        }
        self.reveal(section, cx);
    }

    fn show(&mut self, page: usize, cx: &mut Context<Self>) {
        self.page = page;
        self.toggled = None;
        self.flash = None;
        cx.set_global(LastPage(page));
        self.load();
        cx.notify();
    }

    fn reveal(&mut self, section: Option<usize>, cx: &mut Context<Self>) {
        let item = section.filter(|s| *s > 0).and_then(|s| self.starts.get(s).copied()).unwrap_or(0);
        self.list.scroll_to(ListOffset { item_ix: item, offset_in_item: px(0.) });
        self.flash = section.filter(|s| *s > 0).map(|s| (self.page, s));
        cx.notify();
    }

    fn history(&mut self, forward: bool, cx: &mut Context<Self>) {
        let (from, to) = if forward { (&mut self.forward, &mut self.back) } else { (&mut self.back, &mut self.forward) };
        let Some(page) = from.pop() else { return };
        to.push(self.page);
        self.show(page, cx);
        self.reveal(None, cx);
    }

    fn step_page(&mut self, by: isize, cx: &mut Context<Self>) {
        let next = self.page as isize + by;
        if (0..PAGES.len() as isize).contains(&next) {
            self.go(next as usize, None, cx);
        }
    }

    fn searching(&self, cx: &App) -> bool {
        !self.search.read(cx).value().trim().is_empty()
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.hits.clear();
        self.hit = 0;
    }

    fn open_hit(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.hits.get(index).cloned() else { return };
        self.clear_search(window, cx);
        window.focus(&self.focus, cx);
        self.go(hit.page, Some(hit.section), cx);
    }

    fn toggle_source(&mut self, item: usize, index: usize, show: bool, cx: &mut Context<Self>) {
        let key = (self.page, index);
        if show == self.sources.contains(&key) {
            return;
        }
        if show {
            self.sources.insert(key);
        } else {
            self.sources.remove(&key);
        }
        self.toggled = Some(key);
        self.list.remeasure_items(item..item + 1);
        cx.notify();
    }

    fn copy(&mut self, index: usize, source: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(source));
        self.copied = Some((self.page, index));
        self.copy_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |h, cx| {
                h.copied = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let secondary = k.modifiers.secondary(cx);
        let typing = window.context_stack().iter().any(|c| c.contains("Input"));
        let handled = match k.key.as_str() {
            "f" if secondary => {
                self.search.update(cx, |s, cx| s.focus(window, cx));
                true
            }
            "escape" if self.searching(cx) => {
                self.clear_search(window, cx);
                true
            }
            "escape" if typing => {
                window.focus(&self.focus, cx);
                true
            }
            "escape" => {
                window.remove_window();
                true
            }
            "down" if self.searching(cx) => {
                self.hit = (self.hit + 1).min(self.hits.len().saturating_sub(1));
                true
            }
            "up" if self.searching(cx) => {
                self.hit = self.hit.saturating_sub(1);
                true
            }
            "[" if secondary => {
                window.focus(&self.focus, cx);
                self.history(false, cx);
                true
            }
            "]" if secondary => {
                window.focus(&self.focus, cx);
                self.history(true, cx);
                true
            }
            "up" if k.modifiers.alt && !typing => {
                window.focus(&self.focus, cx);
                self.step_page(-1, cx);
                true
            }
            "down" if k.modifiers.alt && !typing => {
                window.focus(&self.focus, cx);
                self.step_page(1, cx);
                true
            }
            _ if typing || k.modifiers.platform || k.modifiers.control || k.modifiers.alt => false,
            "/" => {
                self.search.update(cx, |s, cx| s.focus(window, cx));
                true
            }
            "down" => self.scroll(px(STEP)),
            "up" => self.scroll(px(-STEP)),
            "pagedown" | "space" if !k.modifiers.shift => self.scroll(self.page_height()),
            "pageup" | "space" => self.scroll(-self.page_height()),
            "home" => {
                self.list.scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) });
                true
            }
            "end" => {
                self.list.scroll_to_end();
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn page_height(&self) -> Pixels {
        (self.list.viewport_bounds().size.height - px(STEP)).max(px(STEP))
    }

    fn scroll(&self, by: Pixels) -> bool {
        self.list.scroll_by(by);
        true
    }

    fn sidebar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut groups = div().flex().flex_col().gap(px(space::L));
        let mut current: Option<Group> = None;
        let mut block = div();
        for (i, page) in PAGES.iter().enumerate() {
            if current != Some(page.group) {
                if current.is_some() {
                    groups = groups.child(block);
                }
                current = Some(page.group);
                block = div().flex().flex_col().gap(px(space::XXS)).child(div().px(px(space::S)).pb(px(space::XS)).text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.tertiary).child(page.group.label()));
            }
            let selected = i == self.page && !self.searching(cx);
            block = block.child(
                div()
                    .id(("help-page", i))
                    .h(px(control::M))
                    .px(px(space::S))
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .rounded(px(radius::INNER))
                    .cursor_pointer()
                    .key_nav()
                    .soft(t.ink.opacity(0.05), radius::INNER)
                    .when(selected, |d| d.bg(t.ink.opacity(0.08)).font_weight(FontWeight::MEDIUM))
                    .child(icon(page.icon, iconsize::S, if selected { t.accent } else { t.secondary }))
                    .child(div().flex_1().min_w_0().truncate().text_color(if selected { t.primary } else { t.secondary }).child(self.docs[i].title.clone()))
                    .on_click(cx.listener(move |h, _, window, cx| {
                        h.clear_search(window, cx);
                        h.go(i, None, cx);
                    })),
            );
        }
        groups = groups.child(block);
        div()
            .w(px(SIDEBAR))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.sidebar)
            .border_r_1()
            .border_color(t.separator)
            .child(div().px(px(space::M)).pt(px(if super::chrome::CUSTOM { space::M } else { 44. })).pb(px(space::M)).child(Input::new(&self.search).small().cleanable(true).prefix(icon("search", iconsize::XS, t.secondary))))
            .child(div().id("help-pages").flex_1().min_h_0().overflow_y_scrollbar().px(px(space::S)).pb(px(space::L)).child(groups))
    }

    fn results(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.hits.is_empty() {
            let query = self.search.read(cx).value().trim().to_string();
            return unavailable("search", tr("Keine Treffer"), trf("Nichts zu „%@“. Versuch ein anderes Wort, zum Beispiel Karten, Sync oder Tabelle.", &[&query]), t).into_any_element();
        }
        let selected = self.hit.min(self.hits.len() - 1);
        let mut rows = div().w_full().max_w(px(MEASURE)).flex().flex_col().gap(px(space::XXS));
        rows = rows.child(div().px(px(space::M)).pb(px(space::S)).child(caption(if self.hits.len() == 1 { tr("1 Treffer") } else { trf("%lld Treffer", &[&self.hits.len()]) }, t)));
        for (i, hit) in self.hits.iter().enumerate() {
            let page = &PAGES[hit.page];
            let title = self.docs[hit.page].title.clone();
            let place = match &hit.heading {
                Some(h) => format!("{title} › {h}"),
                None => title,
            };
            let marks = hit.marks.iter().map(|r| (r.clone(), HighlightStyle { color: Some(t.primary), font_weight: Some(FontWeight::SEMIBOLD), background_color: Some(t.accent.opacity(0.16)), ..Default::default() })).collect::<Vec<_>>();
            rows = rows.child(
                div()
                    .id(("help-hit", i))
                    .w_full()
                    .flex()
                    .gap(px(space::M))
                    .px(px(space::M))
                    .py(px(space::S + space::XXS))
                    .rounded(px(radius::CONTROL))
                    .cursor_pointer()
                    .key_nav()
                    .soft(t.ink.opacity(0.05), radius::CONTROL)
                    .when(i == selected, |d| d.bg(t.ink.opacity(0.08)))
                    .child(div().pt(px(space::XXS)).child(icon(page.icon, iconsize::S, if i == selected { t.accent } else { t.secondary })))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(space::XXS))
                            .child(div().truncate().font_weight(FontWeight::SEMIBOLD).child(place))
                            .child(div().text_size(px(font::CALLOUT)).line_height(relative(1.45)).text_color(t.secondary).child(StyledText::new(hit.snippet.clone()).with_highlights(marks))),
                    )
                    .when(i == selected, |d| d.child(div().pt(px(space::XXS)).child(kbd(Shortcut { key: "return".into(), command: false, shift: false, option: false, control: false }.display(), t))))
                    .on_click(cx.listener(move |h, _, window, cx| h.open_hit(i, window, cx))),
            );
        }
        div().id("help-results").size_full().overflow_y_scrollbar().pt(px(if super::chrome::CUSTOM { space::XL } else { 44. })).px(px(space::XL)).pb(px(space::XL)).flex().justify_center().child(rows).into_any_element()
    }

    fn head(&self, t: &Theme) -> AnyElement {
        let page = &PAGES[self.page];
        div()
            .pt(px(if super::chrome::CUSTOM { space::XL } else { 52. }))
            .pb(px(space::L))
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.accent_text).child(page.group.label()))
            .child(div().text_size(px(font::LARGE_TITLE)).font_weight(FontWeight::BOLD).line_height(relative(1.15)).child(self.docs[self.page].title.clone()))
            .into_any_element()
    }

    fn foot(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let card = |id: &'static str, to: usize, label: String, forward: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(space::M))
                .when(forward, Styled::flex_row_reverse)
                .p(px(space::M))
                .rounded(px(radius::CARD))
                .border_1()
                .border_color(t.separator)
                .cursor_pointer()
                .key_nav()
                .soft(t.ink.opacity(0.04), radius::CARD)
                .child(icon(if forward { "chevron-right" } else { "chevron-left" }, iconsize::M, t.secondary))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XXS)).when(forward, Styled::items_end).child(caption(label, t)).child(div().truncate().font_weight(FontWeight::SEMIBOLD).child(self.docs[to].title.clone())))
                .on_click(cx.listener(move |h, _, _, cx| h.go(to, None, cx)))
        };
        let mut row = div().flex().gap(px(space::M));
        row = match self.page.checked_sub(1) {
            Some(p) => row.child(card("help-prev", p, tr("Vorherige Seite"), false, cx)),
            None => row.child(div().flex_1()),
        };
        row = match (self.page + 1 < PAGES.len()).then_some(self.page + 1) {
            Some(p) => row.child(card("help-next", p, tr("Nächste Seite"), true, cx)),
            None => row.child(div().flex_1()),
        };
        div().pt(px(space::XL)).pb(px(space::XXL)).child(div().pt(px(space::XL)).border_t_1().border_color(t.separator).child(row)).into_any_element()
    }

    fn item(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::get(cx);
        let base = sample_dir();
        let body = match &self.items[ix] {
            Item::Head => self.head(&t),
            Item::Foot => self.foot(&t, cx),
            Item::Text { note, section } => {
                let lit = self.flash == Some((self.page, *section)) && self.starts.get(*section) == Some(&ix);
                let starts_section = *section > 0 && self.starts.get(*section) == Some(&ix);
                let shown = div().pb(px(space::M)).when(starts_section, |d| d.pt(px(space::L))).child(markdown_view::render(note, Some(&base), None, &t));
                if lit {
                    let tint = t.accent;
                    div()
                        .id(("help-flash", ix))
                        .mx(px(-space::M))
                        .px(px(space::M))
                        .rounded(px(radius::CONTROL))
                        .child(shown)
                        .with_animation(("help-flash", ix), Animation::new(motion::timed(FLASH, cx)).with_easing(ease_in_out), move |d, p| d.bg(tint.opacity(0.12 * (1. - p))))
                        .into_any_element()
                } else {
                    shown.into_any_element()
                }
            }
            Item::Example { index, title, source, note, folds } => {
                let key = (self.page, *index);
                let (index, entity) = (*index, cx.entity().downgrade());
                let copy_entity = entity.clone();
                let copy_source = source.clone();
                let on_show: help_example::OnShow = Rc::new(move |show, _, cx| {
                    let _ = entity.update(cx, |h, cx| h.toggle_source(ix, index, show, cx));
                });
                let on_copy = Rc::new(move |_: &mut Window, cx: &mut App| {
                    let source = copy_source.clone();
                    let _ = copy_entity.update(cx, |h, cx| h.copy(index, source, cx));
                });
                let id = self.page * 1000 + index;
                div()
                    .pb(px(space::L))
                    .child(help_example::example(id, title.clone(), source.clone(), note.clone(), folds.clone(), Some(&base), self.sources.contains(&key), self.copied == Some(key), self.toggled == Some(key), on_show, on_copy))
                    .into_any_element()
            }
        };
        div().w_full().flex().justify_center().px(px(space::XXL)).child(div().w_full().max_w(px(MEASURE)).child(body)).into_any_element()
    }
}

/// The example as the reader draws it: links to the sample note point at its file, so they show as existing.
fn sample_links(source: &str) -> String {
    let mut out = source.to_string();
    for name in ["Beispiel", "Example"] {
        out = out.replace(&format!("![[{name}#"), &format!("![[{name}.md#")).replace(&format!("![[{name}]]"), &format!("![[{name}.md]]"));
    }
    out
}

impl Render for Help {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        let t = Theme::get(cx);
        let body: AnyElement = if self.searching(cx) {
            self.results(&t, cx)
        } else {
            div().size_full().child(list(self.list.clone(), cx.processor(|h, ix, _, cx| h.item(ix, cx))).size_full()).vertical_scrollbar(&self.list).into_any_element()
        };
        let content = div()
            .id("help")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(t.content)
            .font_family(Theme::font_family())
            .text_size(px(font::BODY))
            .line_height(relative(1.22))
            .text_color(t.primary)
            .capture_key_down(cx.listener(Self::on_key))
            .child(self.sidebar(&t, cx))
            .child(div().flex_1().min_w_0().h_full().child(body));
        super::chrome::frame(tr("Schrumbi-Anleitung"), content, &t)
    }
}

#[cfg(test)]
mod tests {
    use crate::core::docs::{self, PAGES, Piece};
    use crate::core::i18n::Lang;
    use crate::core::shortcuts::ShortcutAction;

    #[test]
    fn every_key_placeholder_names_a_real_key() {
        let app = include_str!("app.rs");
        let mut unknown = Vec::new();
        for page in PAGES {
            for (kind, name) in docs::placeholders(page.source(Lang::De)) {
                let known = ShortcutAction::ALL.iter().any(|a| a.id() == name) || docs::plain_key(&name).is_some() || name.chars().next().is_some_and(char::is_uppercase) && regex::Regex::new(&format!(r"[\s(,:]{name}[\s),]")).unwrap().is_match(app);
                if kind == "key" && !known {
                    unknown.push(format!("{}: {name}", page.slug));
                }
            }
        }
        assert!(unknown.is_empty(), "keys the app does not bind: {unknown:?}");
    }

    #[test]
    fn prose_has_no_links() {
        let link = regex::Regex::new(r"\[\[|\]\(|https?://").unwrap();
        let code = regex::Regex::new(r"`[^`\n]*`").unwrap();
        for page in PAGES {
            for lang in [Lang::De, Lang::En] {
                for section in docs::split(page.source(lang)).sections {
                    for piece in section.pieces {
                        if let Piece::Text(text) = piece {
                            let bare = code.replace_all(&text, "");
                            assert!(!link.is_match(&bare), "{} ({lang:?}): links in prose open or create notes in the user's folder: {text}", page.slug);
                        }
                    }
                }
            }
        }
    }
}
