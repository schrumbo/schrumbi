use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::Disableable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::quick::Dismissed;
use super::state::{self, act};
use super::theme::{Theme, font, iconsize, space};
use super::widgets::*;
use crate::core::i18n::{tr, trf};
use crate::core::library::ConventionPlan;

const FORBIDDEN: &str = "\\/:*?\"<>|";

/// Whether a file can be renamed by its module's convention; never offered while no naming convention is switched on in the settings.
pub fn available(path: &Path, cx: &App) -> bool {
    state::read(cx, |l| l.config.as_ref().is_some_and(|c| c.naming_convention()) && l.can_name_by_convention(path))
}

/// Works out the convention name off the UI thread, then lets the user check and change it before renaming.
pub fn open(path: PathBuf, window: &mut Window, cx: &mut App) {
    let Some(plan) = state::read(cx, |l| l.convention_plan(&path)) else { return };
    let (file, reader) = (path.clone(), plan.clone());
    let task = cx.background_spawn(async move { reader.name(&file) });
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let suggested = task.await;
        let _ = handle.update(cx, move |_, window, cx| super::sheets::show_sheet(460., window, cx, move |window, cx| ConventionRename::new(path, plan, suggested, window, cx)));
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
    _sub: Subscription,
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
    fn new(path: PathBuf, plan: ConventionPlan, suggested: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (stem, ext) = split(&suggested);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(stem));
        let sub = cx.subscribe(&input, |this: &mut Self, _, _: &InputEvent, cx| {
            this.verdict = this.judge(cx);
            cx.notify();
        });
        let mut sheet = Self { path, plan, suggested, ext, input, verdict: Verdict::Empty, focused: false, _sub: sub };
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
        } else if stem.contains(|c| FORBIDDEN.contains(c)) || stem.ends_with('.') {
            Verdict::Forbidden
        } else if self.path.with_file_name(&name).exists() && !crate::core::library::same_file(&self.path, &self.path.with_file_name(&name)) {
            Verdict::Taken
        } else if self.plan.fits(&name) {
            Verdict::Fits
        } else {
            Verdict::Off
        }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.verdict, Verdict::Fits | Verdict::Off) {
            return;
        }
        let (path, name) = (self.path.clone(), self.name(cx));
        act(cx, |l| l.rename_keeping_heading(&path, &name));
        state::reindex(cx);
        cx.emit(Dismissed);
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
        let context = match &self.plan.deadline {
            Some(d) => trf("Muster von %@ · für %@", &[&self.plan.module, d]),
            None => trf("Muster von %@", &[&self.plan.module]),
        };
        let note = |name: &str, tint: Hsla, label: String| div().flex().items_center().gap(px(space::XS)).text_size(px(font::CAPTION)).text_color(tint).child(icon(name, iconsize::XS, tint)).child(div().min_w_0().child(label));
        let status = match verdict {
            Verdict::Fits => note("circle-check", t.green_text, tr("Passt zur Namenskonvention")),
            Verdict::Off => note("info", t.secondary, tr("Weicht von der Namenskonvention ab")),
            Verdict::Same => note("info", t.secondary, tr("Die Datei heißt schon so")),
            Verdict::Taken => note("triangle-alert", t.red, tr("Diesen Namen gibt es hier schon. Wähle einen anderen.")),
            Verdict::Forbidden => note("triangle-alert", t.red, trf("Diese Zeichen gehen nicht: %@", &[&"\\ / : * ? \" < > |".to_string()])),
            Verdict::Empty => note("triangle-alert", t.red, tr("Gib einen Namen ein.")),
        };
        let ready = matches!(verdict, Verdict::Fits | Verdict::Off);
        let field = Input::new(&self.input).when(!self.ext.is_empty(), |i| i.suffix(div().text_color(t.secondary).child(format!(".{}", self.ext))));
        div()
            .w_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|s, _: &Confirm, window, cx| if super::sheets::typing(window) { s.confirm(cx) } else { cx.propagate() }))
            .child(div().px(px(space::XL)).pt(px(space::L)).flex().flex_col().gap(px(space::XXS)).child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).child(tr("Nach Konvention benennen"))).child(caption(context, &t).truncate()))
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
                    .child(Button::new("convention-cancel").label(tr("Abbrechen")).on_click(cx.listener(|_, _, _, cx| cx.emit(Dismissed))))
                    .child(Button::new("convention-rename").primary().label(tr("Umbenennen")).disabled(!ready).on_click(cx.listener(|s, _, _, cx| s.confirm(cx)))),
            )
    }
}
