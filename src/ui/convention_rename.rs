use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::Disableable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::quick::Dismissed;
use super::state::{self, act};
use super::theme::{Theme, font, iconsize, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::library::{ConventionOffer, ConventionPlan};
use crate::core::text::end_ellipsis;

const NAME_IN_MENU: usize = 40;
const NAMED_ENTRIES: usize = 5;

/// Whether a file can be renamed by its module's convention right now; never offered while no naming convention is switched on in the settings.
pub fn available(path: &Path, cx: &App) -> bool {
    state::read(cx, |l| l.convention_offer(path)) == ConventionOffer::Ready
}

/// The working menu entry for renaming by convention, else none.
pub fn menu_item(path: &Path, cx: &App) -> Option<PopupMenuItem> {
    match state::read(cx, |l| l.convention_offer(path)) {
        ConventionOffer::Ready => menu_item_or_hint(path, cx),
        ConventionOffer::Hidden | ConventionOffer::NeedsDeadline => None,
    }
}

/// The working entry for one file, named after it, for menus that list one entry per misnamed file; none unless the file is ready to rename.
pub fn named_menu_item(path: &Path, cx: &App) -> Option<PopupMenuItem> {
    if state::read(cx, |l| l.convention_offer(path)) != ConventionOffer::Ready {
        return None;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let path = path.to_path_buf();
    Some(PopupMenuItem::new(trf("„%@“ nach Muster benennen…", &[&end_ellipsis(&name, NAME_IN_MENU)])).icon(menu_icon("wand-sparkles")).on_click(move |_, window, cx| open(path.clone(), window, cx)))
}

/// Entries for the misnamed files of one deadline: one named entry each for up to `NAMED_ENTRIES`, else a single entry that starts with the first.
pub fn named_menu_items(files: &[PathBuf], cx: &App) -> Vec<PopupMenuItem> {
    let ready: Vec<&PathBuf> = files.iter().filter(|f| state::read(cx, |l| l.convention_offer(f)) == ConventionOffer::Ready).collect();
    if ready.len() <= NAMED_ENTRIES {
        return ready.into_iter().filter_map(|f| named_menu_item(f, cx)).collect();
    }
    let all: Vec<PathBuf> = ready.into_iter().cloned().collect();
    vec![PopupMenuItem::new(trf("Nach Muster benennen… (%@ Dateien)", &[&all.len().to_string()])).icon(menu_icon("wand-sparkles")).on_click(move |_, window, cx| open_all(all.clone(), window, cx))]
}

/// Like `menu_item`, but for a file in a submission folder that belongs to no deadline yet the entry is disabled with its reason; for menus that also offer to assign a deadline.
pub fn menu_item_or_hint(path: &Path, cx: &App) -> Option<PopupMenuItem> {
    match state::read(cx, |l| l.convention_offer(path)) {
        ConventionOffer::Hidden => None,
        ConventionOffer::NeedsDeadline => Some(PopupMenuItem::new(tr("Nach Muster benennen – erst Abgabe zuordnen")).icon(menu_icon("wand-sparkles")).disabled(true)),
        ConventionOffer::Ready => {
            let path = path.to_path_buf();
            Some(PopupMenuItem::new(tr("Nach Muster benennen…")).icon(menu_icon("wand-sparkles")).on_click(move |_, window, cx| open(path.clone(), window, cx)))
        }
    }
}

/// Works out the convention name off the UI thread, then lets the user check and change it before renaming.
pub fn open(path: PathBuf, window: &mut Window, cx: &mut App) {
    start(path, Vec::new(), window, cx);
}

/// Like `open` for the first file; after each rename or skip the sheet goes on with the next file that is still ready.
pub fn open_all(mut files: Vec<PathBuf>, window: &mut Window, cx: &mut App) {
    if !files.is_empty() {
        let first = files.remove(0);
        start(first, files, window, cx);
    }
}

fn start(path: PathBuf, rest: Vec<PathBuf>, window: &mut Window, cx: &mut App) {
    let Some(plan) = state::read(cx, |l| l.convention_plan(&path)) else { return };
    let (file, reader) = (path.clone(), plan.clone());
    let task = cx.background_spawn(async move { reader.name(&file) });
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let suggested = task.await;
        let _ = handle.update(cx, move |_, window, cx| super::sheets::show_sheet(460., window, cx, move |window, cx| ConventionRename::new(path, plan, suggested, rest, window, cx)));
    })
    .detach();
}

#[derive(PartialEq)]
enum Verdict {
    Empty,
    Same,
    Forbidden,
    Taken,
    Fits,
    Off,
}

struct ConventionRename {
    path: PathBuf,
    plan: ConventionPlan,
    suggested: String,
    ext: String,
    input: Entity<InputState>,
    verdict: Verdict,
    focused: bool,
    rest: Vec<PathBuf>,
    index: usize,
    total: usize,
    renamed: usize,
    error: Option<String>,
    busy: bool,
    _sub: Subscription,
    _release: Subscription,
}

impl EventEmitter<Dismissed> for ConventionRename {}

fn split(name: &str) -> (String, String) {
    let ext = crate::core::naming::extension(name);
    if ext.is_empty() {
        return (name.to_string(), String::new());
    }
    (crate::core::naming::stem(name).to_string(), ext.to_string())
}

impl ConventionRename {
    fn new(path: PathBuf, plan: ConventionPlan, suggested: String, rest: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (stem, ext) = split(&suggested);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(stem));
        let sub = cx.subscribe(&input, |this: &mut Self, _, e: &InputEvent, cx| {
            this.verdict = this.judge(cx);
            if matches!(e, InputEvent::Change) {
                this.error = None;
            }
            cx.notify();
        });
        let release = cx.on_release(|this: &mut Self, cx| this.finish(cx));
        let total = rest.len() + 1;
        let mut sheet = Self { path, plan, suggested, ext, input, verdict: Verdict::Empty, focused: false, rest, index: 1, total, renamed: 0, error: None, busy: false, _sub: sub, _release: release };
        sheet.verdict = sheet.judge(cx);
        sheet
    }

    fn name(&self, cx: &App) -> String {
        let stem = self.input.read(cx).value().trim().to_string();
        if self.ext.is_empty() { stem } else { format!("{stem}.{}", self.ext) }
    }

    fn current(&self) -> String {
        self.path.file_name().unwrap_or_default().to_string_lossy().to_string()
    }

    fn judge(&self, cx: &App) -> Verdict {
        let stem = self.input.read(cx).value().trim().to_string();
        let name = self.name(cx);
        let current = self.current();
        if stem.is_empty() {
            Verdict::Empty
        } else if name == current {
            Verdict::Same
        } else if super::rename_sheet::forbidden_message(&stem).is_some() {
            Verdict::Forbidden
        } else if self.path.with_file_name(&name).exists() && !crate::core::library::same_file(&self.path, &self.path.with_file_name(&name)) {
            Verdict::Taken
        } else if self.plan.fits(&name) {
            Verdict::Fits
        } else {
            Verdict::Off
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !matches!(self.verdict, Verdict::Fits | Verdict::Off) {
            return;
        }
        let (path, name) = (self.path.clone(), self.name(cx));
        match act(cx, |l| l.rename_in_series(&path, &name)) {
            Ok(()) => {
                self.renamed += 1;
                state::reindex(cx);
                self.error = None;
                self.next(window, cx);
            }
            Err(e) => {
                self.error = Some(e);
                cx.notify();
            }
        }
    }

    fn finish(&mut self, cx: &mut App) {
        let renamed = std::mem::take(&mut self.renamed);
        if renamed > 0 {
            act(cx, |l| l.finish_series(renamed));
        }
    }

    fn skip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.busy {
            self.next(window, cx);
        }
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while !self.rest.is_empty() {
            let path = self.rest.remove(0);
            let Some(plan) = state::read(cx, |l| (l.convention_offer(&path) == ConventionOffer::Ready).then(|| l.convention_plan(&path)).flatten()) else {
                self.total -= 1;
                continue;
            };
            self.busy = true;
            cx.notify();
            let (reader, file) = (plan.clone(), path.clone());
            let task = cx.background_spawn(async move { reader.name(&file) });
            cx.spawn_in(window, async move |this, cx| {
                let suggested = task.await;
                let _ = this.update_in(cx, |s, window, cx| s.show(path, plan, suggested, window, cx));
            })
            .detach();
            return;
        }
        self.finish(cx);
        cx.emit(Dismissed);
    }

    fn show(&mut self, path: PathBuf, plan: ConventionPlan, suggested: String, window: &mut Window, cx: &mut Context<Self>) {
        let (stem, ext) = split(&suggested);
        (self.path, self.plan, self.suggested, self.ext, self.busy) = (path, plan, suggested, ext, false);
        self.index += 1;
        self.input.update(cx, |s, cx| {
            s.set_value(stem.clone(), window, cx);
            s.focus(window, cx);
            s.set_selected_range(0..stem.len(), cx);
        });
        self.verdict = self.judge(cx);
        cx.notify();
    }

    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (stem, _) = split(&self.suggested);
        self.input.update(cx, |s, cx| {
            s.set_value(stem.clone(), window, cx);
            s.focus(window, cx);
            s.set_selected_range(0..stem.len(), cx);
        });
        self.verdict = self.judge(cx);
        cx.notify();
    }
}

impl Render for ConventionRename {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !std::mem::replace(&mut self.focused, true) {
            self.input.update(cx, |s, cx| {
                let end = s.value().len();
                s.focus(window, cx);
                s.set_selected_range(0..end, cx);
            });
        }
        let t = Theme::get(cx);
        let verdict = &self.verdict;
        let edited = self.name(cx) != self.suggested;
        let context = trf("Muster von %@ · für %@", &[&self.plan.module, &self.plan.deadline]);
        let note = |name: &str, tint: Hsla, label: String| div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(tint).child(icon(name, iconsize::XS, tint)).child(div().min_w_0().child(label));
        let status = match verdict {
            _ if self.error.is_some() => note("triangle-alert", t.red, self.error.clone().unwrap_or_default()),
            Verdict::Fits => note("circle-check", t.green_text, tr("Passt zum Namensmuster")),
            Verdict::Off => note("info", t.secondary, tr("Weicht vom Namensmuster ab")),
            Verdict::Same => note("info", t.secondary, tr("Die Datei heißt schon so")),
            Verdict::Taken => note("triangle-alert", t.red, tr("Diesen Namen gibt es hier schon. Wähle einen anderen.")),
            Verdict::Forbidden => note("triangle-alert", t.red, super::rename_sheet::forbidden_message(self.input.read(cx).value().trim()).unwrap_or_default()),
            Verdict::Empty => note("info", t.secondary, tr("Gib einen Namen ein.")),
        };
        let ready = matches!(verdict, Verdict::Fits | Verdict::Off);
        let field = Input::new(&self.input).when(!self.ext.is_empty(), |i| i.suffix(div().text_color(t.secondary).child(format!(".{}", self.ext))));
        div()
            .w_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|s, _: &Confirm, window, cx| if super::sheets::typing(window) { s.confirm(window, cx) } else { cx.propagate() }))
            .child(div().px(px(space::XL)).pt(px(space::L)).flex().flex_col().gap(px(space::XXS)).child(div().flex().items_baseline().gap(px(space::S)).child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(tr("Nach Muster benennen"))).when(self.total > 1, |d| d.child(caption(trf("%lld von %lld", &[&self.index, &self.total]), &t)))).child(caption(context, &t).truncate()))
            .child(
                div()
                    .px(px(space::XL))
                    .py(px(space::L))
                    .flex()
                    .flex_col()
                    .gap(px(space::S))
                    .child(field)
                    .child(status)
                    .child(caption(trf("Jetzt: %@", &[&self.current()]), &t).truncate()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S))
                    .p(px(space::L))
                    .border_t_1()
                    .border_color(t.separator)
                    .when(edited, |d| d.child(Button::new("convention-restore").ghost().icon(super::widgets::menu_icon("rotate-ccw")).label(tr("Zurück zum Vorschlag")).on_click(cx.listener(|s, _, window, cx| s.restore(window, cx)))))
                    .child(div().flex_1())
                    .child(Button::new("convention-cancel").label(tr("Abbrechen")).on_click(cx.listener(|s, _, _, cx| {
                        s.finish(cx);
                        cx.emit(Dismissed);
                    })))
                    .when(!self.rest.is_empty(), |d| d.child(Button::new("convention-skip").label(tr("Überspringen")).disabled(self.busy).on_click(cx.listener(|s, _, window, cx| s.skip(window, cx)))))
                    .child(Button::new("convention-rename").primary().label(if self.rest.is_empty() { tr("Umbenennen") } else { tr("Umbenennen & weiter") }).loading(self.busy).disabled(!ready || self.busy).on_click(cx.listener(|s, _, window, cx| s.confirm(window, cx)))),
            )
    }
}
