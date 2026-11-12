use std::collections::HashMap;

use chrono::{Datelike, Days, NaiveDate, Utc};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Selectable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::{self, act};
use super::motion::{self, Soft};
use super::theme::{Theme, calendar as cal, control, font, radius, space, iconsize};
use super::widgets::{KeyNav, file_target, icon, menu_icon, section_title, status_menu, status_toggle, tabular, tip};
use crate::core::calendar::{self, CalendarView};
use crate::core::i18n::{tr, trf};
use crate::core::library::Library;
use crate::core::model::{Assignment, Kind, Module, Session, Status};
use crate::core::day;

/// The calendar of the overview: paging and the view switch above the week's columns or the month's grid.
#[derive(IntoElement)]
pub struct Board {
    narrow: bool,
}

/// The calendar for a page that is `narrow` or not; it reads the library itself so it can animate with the window.
pub fn render(narrow: bool) -> Board {
    Board { narrow }
}

impl RenderOnce for Board {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::get(cx);
        let (view, turn) = state::read(cx, |l| (l.calendar_view, l.calendar_turn));
        let segment = motion::follow("board-segment", if view == CalendarView::Month { 1. } else { 0. }, motion::FAST, window, cx);
        let travel = motion::timed(motion::FAST, cx);
        let lib = state::lib(cx);
        let l = lib.read(cx);
        let page = match view {
            CalendarView::Week => week(l, calendar::page_days(l.calendar_anchor, l.today()), self.narrow, &t),
            CalendarView::Month => month(l, l.calendar_anchor(), self.narrow, &t),
        };
        let slide = f32::from(turn.1) * cal::SLIDE;
        let page = if KEYED_TURN.get() == turn.0 {
            div().child(page).into_any_element()
        } else {
            div().child(page).with_animation(("board-turn", turn.0 as usize), Animation::new(travel).with_easing(ease_out_quint()), move |d, p| d.opacity(0.3 + 0.7 * p).ml(px(slide * (1. - p))).mr(px(-slide * (1. - p)))).into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(space::S))
            .child(header(l, view, segment, &t))
            .child(page)
    }
}

/// The calendar keys of the overview: ←/→ turn the page, T and Home go to today, W and M pick the view. Says whether `key` was one of them.
pub fn key(key: &str, cx: &mut App) -> bool {
    match key {
        "left" => act(cx, |l| l.turn_calendar(-1)),
        "right" => act(cx, |l| l.turn_calendar(1)),
        "t" | "home" => act(cx, Library::calendar_today),
        "w" => act(cx, |l| l.set_calendar_view(CalendarView::Week)),
        "m" => act(cx, |l| l.set_calendar_view(CalendarView::Month)),
        _ => return false,
    }
    KEYED_TURN.set(state::read(cx, |l| l.calendar_turn.0));
    true
}

thread_local! {
    static KEYED_TURN: std::cell::Cell<u64> = const { std::cell::Cell::new(u64::MAX) };
}

fn header(l: &Library, view: CalendarView, segment: f32, t: &Theme) -> Div {
    let today = l.today();
    let anchor = l.calendar_anchor;
    let at_today = calendar::at_home(view, anchor, today);
    let (title, caption, hint) = match view {
        CalendarView::Week => {
            let days = calendar::page_days(anchor, today);
            let (a, b) = (days[0], days[6]);
            let lectures = [a, b].map(|d| l.lecture_week_containing(d).map(|w| w.index));
            let caption = match lectures {
                [Some(x), Some(y)] if x != y => Some(trf("Vorlesungswochen %lld–%lld", &[&x, &y])),
                [Some(w), _] | [None, Some(w)] => Some(trf("Vorlesungswoche %lld", &[&w])),
                [None, None] => l.semester().and_then(|s| s.break_at(a).or_else(|| s.break_at(b))).map(|b| b.name.clone()),
            };
            (day::day_range(a, b, today), caption, day::calendar_weeks(a, b))
        }
        CalendarView::Month => {
            let grid = calendar::month_grid(l.calendar_anchor());
            let hint = day::calendar_weeks(grid[0][0], grid[calendar::ROWS - 1][6]);
            (day::month_year(calendar::named_month(view, anchor, today)), None, hint)
        }
    };
    let (back, forward) = match view {
        CalendarView::Week => (tr("Vorherige Woche"), tr("Nächste Woche")),
        CalendarView::Month => (tr("Vorheriger Monat"), tr("Nächster Monat")),
    };
    div()
        .flex()
        .items_center()
        .gap(px(space::M))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(space::XXS))
                .child(Button::new("board-prev").ghost().small().icon(menu_icon("chevron-left")).tooltip(format!("{back} (←)")).on_click(|_, _, cx| act(cx, |l| l.turn_calendar(-1))))
                .child(Button::new("board-next").ghost().small().icon(menu_icon("chevron-right")).tooltip(format!("{forward} (→)")).on_click(|_, _, cx| act(cx, |l| l.turn_calendar(1))))
                .child(Button::new("board-today").small().label(tr("Heute")).disabled(at_today).tooltip(format!("{} (T)", tr("Zu heute springen"))).on_click(|_, _, cx| act(cx, Library::calendar_today))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_baseline()
                .gap(px(space::S))
                .child(div().id("board-title").flex_none().tooltip(tip(hint)).child(section_title(title)))
                .children(caption.map(|c| div().min_w_0().truncate().text_size(px(font::CAPTION)).text_color(t.tertiary).child(c))),
        )
        .child(segmented(view, segment, t))
}

fn segmented(view: CalendarView, segment: f32, t: &Theme) -> Stateful<Div> {
    div()
        .id("board-view")
        .relative()
        .flex()
        .flex_none()
        .h(px(control::M))
        .p(px(space::XXS))
        .rounded(px(radius::CONTROL))
        .bg(t.ink.opacity(0.06))
        .child(div().absolute().top(px(space::XXS)).bottom(px(space::XXS)).left(px(space::XXS + segment * cal::SEGMENT)).w(px(cal::SEGMENT)).rounded(px(radius::INNER)).bg(t.content).shadow_sm())
        .child(segment_button("board-view-week", tr("Woche"), "W", view == CalendarView::Week, CalendarView::Week, t))
        .child(segment_button("board-view-month", tr("Monat"), "M", view == CalendarView::Month, CalendarView::Month, t))
}

fn segment_button(id: &'static str, label: String, key: &str, on: bool, view: CalendarView, t: &Theme) -> Stateful<Div> {
    let help = format!("{label} ({key})");
    div()
        .id(id)
        .w(px(cal::SEGMENT))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(radius::INNER))
        .soft(t.ink.opacity(0.05), radius::INNER)
        .cursor_pointer()
        .key_nav()
        .tooltip(tip(help))
        .on_click(move |_, _, cx| act(cx, |l| l.set_calendar_view(view)))
        .child(div().text_size(px(font::CAPTION)).font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM }).text_color(if on { t.primary } else { t.secondary }).child(label))
}

fn week(lib: &Library, days: [NaiveDate; 7], narrow: bool, t: &Theme) -> Div {
    if narrow {
        return div().flex().flex_col().gap(px(space::S)).children(days.iter().map(|d| agenda_row(lib, *d, t)));
    }
    div().flex().gap(px(space::S)).children(days.iter().map(|d| column(lib, *d, t)))
}

/// How many chips a month cell holds before the rest fold into "+N".
fn chip_capacity(row_height: f32) -> usize {
    let free = row_height - 2. * cal::CELL_PAD - cal::DAY - space::XXS;
    ((free + space::XXS) / (cal::CHIP + space::XXS)).floor().max(1.) as usize
}

fn month(lib: &Library, anchor: NaiveDate, narrow: bool, t: &Theme) -> Div {
    let grid = calendar::month_grid(anchor);
    let weeks = calendar::week_numbers(&grid);
    let (first, last) = (grid[0][0], grid[calendar::ROWS - 1][6]);
    let mut by_day: HashMap<NaiveDate, Vec<&Assignment>> = HashMap::new();
    for (d, a) in lib.assignments_between(first, last) {
        by_day.entry(d).or_default().push(a);
    }
    let line = t.separator.opacity(0.7);
    let row_height = if narrow { cal::ROW_NARROW } else { cal::ROW };
    let head = div()
        .flex()
        .py(px(space::XS))
        .child(div().w(px(cal::KW)).flex_none().text_size(px(font::CAPTION2)).font_weight(FontWeight::SEMIBOLD).text_color(t.tertiary).text_center().child(tr("KW")))
        .children(grid[0].iter().map(|d| div().flex_1().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(if day::is_weekend(*d) { t.tertiary } else { t.secondary }).text_center().child(day::weekday_short(*d).to_uppercase())));
    let rows = grid.iter().zip(weeks).map(|(row, kw)| {
        div()
            .flex()
            .h(px(row_height))
            .border_t_1()
            .border_color(line)
            .child(div().w(px(cal::KW)).flex_none().pt(px(cal::CELL_PAD + space::XXS)).text_size(px(font::CAPTION2)).text_color(t.tertiary).text_center().font_features(tabular()).child(kw.to_string()))
            .children(row.iter().map(|d| cell(lib, *d, anchor.month(), by_day.get(d).map_or(&[][..], Vec::as_slice), first, row_height, narrow, line, t)))
    });
    div().flex().flex_col().rounded(px(radius::CARD)).border_1().border_color(line).bg(t.content).overflow_hidden().child(head).children(rows)
}

#[allow(clippy::too_many_arguments)]
fn cell(lib: &Library, d: NaiveDate, month: u32, items: &[&Assignment], first: NaiveDate, row_height: f32, narrow: bool, line: Hsla, t: &Theme) -> AnyElement {
    let today = lib.today();
    let is_today = d == today;
    let in_month = d.month() == month;
    let semester = lib.semester();
    let brk = semester.and_then(|s| s.break_at(d));
    let free = semester.is_some_and(|s| !s.contains(d) || brk.is_some());
    let weekend = day::is_weekend(d);
    let shade = if weekend { 0.02 } else { 0. } + if free { 0.03 } else { 0. };
    let mut bg = t.content;
    if shade > 0. {
        bg = bg.blend(t.ink.opacity(shade));
    }
    if is_today {
        bg = bg.blend(t.accent.opacity(0.06));
    }
    let break_name = brk.filter(|b| d == first || !b.contains(d - Days::new(1))).map(|b| b.name.clone());
    let ce = d.num_days_from_ce() as u32 as usize;
    let accent = t.accent;
    let capacity = chip_capacity(row_height);
    let total = items.len();
    let shown = if narrow {
        0
    } else if total > capacity {
        capacity - 1
    } else {
        total
    };
    let hidden = total - shown;
    let used = if narrow { usize::from(total > 0) } else { shown + usize::from(hidden > 0) };
    let free_height = row_height - 2. * cal::CELL_PAD - cal::DAY - (used as f32 + 1.) * space::XXS - used as f32 * cal::CHIP;
    let number = div()
        .id(("board-cell-day", ce))
        .flex_none()
        .size(px(cal::DAY))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(is_today, |n| n.bg(accent))
        .soft(if is_today { t.on_accent.opacity(0.2) } else { t.ink.opacity(0.08) }, radius::FULL)
        .cursor_pointer()
        .key_nav()
        .tooltip(tip(tr("Woche öffnen")))
        .on_click(move |_, _, cx| act(cx, |l| l.show_week_of(d)))
        .child(div().text_size(px(font::CAPTION)).font_weight(if is_today { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM }).text_color(if is_today { t.on_accent } else if in_month { t.primary } else { t.tertiary }).font_features(tabular()).child(d.day().to_string()));
    div()
        .id(("board-cell", ce))
        .group(day_group(d))
        .drag_over::<DraggedCard>(move |s, _, _, _| s.bg(accent.opacity(0.1)))
        .on_drop(move |card: &DraggedCard, _, cx| move_to_day(&card.id, d, cx))
        .context_menu(move |menu, _, _| day_menu(menu, d))
        .flex_1()
        .min_w_0()
        .h_full()
        .relative()
        .flex()
        .flex_col()
        .p(px(cal::CELL_PAD))
        .gap(px(space::XXS))
        .border_l_1()
        .border_color(line)
        .bg(bg)
        .when(!in_month, |c| c.opacity(0.45))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::XS))
                .child(number)
                .when_some(break_name, |h, name| h.child(div().min_w_0().flex().items_center().gap(px(space::XXS)).text_size(px(font::CAPTION2)).text_color(t.secondary).child(icon("sun", iconsize::XS, t.secondary)).child(div().truncate().child(name)))),
        )
        .when(narrow && total > 0, |c| c.child(dots(lib, d, items, t)))
        .children(items.iter().take(shown).map(|a| chip(lib, a, t)))
        .when(!narrow && hidden > 0, |c| c.child(more_button(d, hidden)))
        .when(free_height >= cal::DOT, |c| c.child(DayArea(empty_area(d, free_height, t)).dropdown_menu(move |menu, _, _| day_menu(menu, d))))
        .into_any_element()
}

/// The empty part of a day cell; a click opens the day menu, a double click the week.
fn empty_area(d: NaiveDate, height: f32, t: &Theme) -> Stateful<Div> {
    div()
        .id(("board-day-space", d.num_days_from_ce() as u32 as usize))
        .w_full()
        .h(px(height))
        .rounded(px(radius::INNER))
        .soft(t.ink.opacity(0.04), radius::INNER)
        .cursor_pointer()
        .key_nav()
        .on_click(move |e, _, cx| {
            if e.click_count() == 2 {
                act(cx, |l| l.show_week_of(d));
            }
        })
}

/// A plain div lent the kit's trigger trait, so the day menu opens on a left click as it does on a button.
struct DayArea(Stateful<Div>);

impl Selectable for DayArea {
    fn selected(self, _: bool) -> Self {
        self
    }

    fn is_selected(&self) -> bool {
        false
    }
}

impl Styled for DayArea {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl InteractiveElement for DayArea {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.0.interactivity()
    }
}

impl IntoElement for DayArea {
    type Element = Stateful<Div>;

    fn into_element(self) -> Self::Element {
        self.0
    }
}

impl DropdownMenu for DayArea {}

/// The whole day as cards, for the days of the month grid that show fewer chips than they have.
fn more_list(d: NaiveDate, cx: &App) -> AnyElement {
    let t = Theme::get(cx);
    let lib = state::lib(cx);
    let l = lib.read(cx);
    let items = l.assignments_on(d);
    div()
        .id("board-more-list")
        .w(px(cal::MORE))
        .max_h(px(cal::MORE_HEIGHT))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap(px(space::S))
        .child(div().text_size(px(font::CALLOUT)).font_weight(FontWeight::SEMIBOLD).child(day::day_header(d, l.today())))
        .children(items.iter().map(|a| deadline_card(l, a, &t)))
        .into_any_element()
}

fn more_button(d: NaiveDate, hidden: usize) -> impl IntoElement {
    let ce = d.num_days_from_ce() as u32 as usize;
    Popover::new(("board-more", ce))
        .anchor(Anchor::TopLeft)
        .trigger_style(StyleRefinement::default().w_full())
        .trigger(Button::new(("board-more-trigger", ce)).ghost().xsmall().w_full().h(px(cal::CHIP)).label(trf("+%@ weitere", &[&hidden])))
        .content(move |_, _, cx| more_list(d, cx))
}

/// One dot per entry and their count, standing in for chips when the window is narrow.
fn dots(lib: &Library, d: NaiveDate, items: &[&Assignment], t: &Theme) -> impl IntoElement {
    let ce = d.num_days_from_ce() as u32 as usize;
    let marks: Vec<Hsla> = items.iter().take(5).map(|a| t.palette(lib.module(a).map_or("blue", |m| m.color.as_str())).opacity(if lib.is_done(a) { 0.35 } else { 1. })).collect();
    let count = items.len();
    Popover::new(("board-dots", ce))
        .anchor(Anchor::TopLeft)
        .trigger_style(StyleRefinement::default().w_full())
        .trigger(
            Button::new(("board-dots-trigger", ce))
                .ghost()
                .xsmall()
                .w_full()
                .h(px(cal::CHIP))
                .tooltip(trf("%@ weitere", &[&count]))
                .child(div().flex().items_center().gap(px(space::XXS)).children(marks.into_iter().map(|c| div().size(px(cal::DOT)).rounded_full().bg(c))).child(div().text_size(px(font::CAPTION2)).text_color(t.secondary).font_features(tabular()).child(count.to_string()))),
        )
        .content(move |_, _, cx| more_list(d, cx))
}

/// A deadline as one line of the month grid: module colour at the edge, the title, a cap for exams.
fn chip(lib: &Library, a: &Assignment, t: &Theme) -> AnyElement {
    let e = lib.evaluation(a);
    let module = lib.module(a);
    let tint = t.palette(module.map_or("blue", |m| m.color.as_str()));
    let closed = e.status.is_closed();
    let selected = lib.is_selected(&a.id);
    let attention = lib.needs_action(a);
    let exam = a.kind == Kind::Exam;
    let fill = if exam && !closed { tint.opacity(0.22) } else { tint.opacity(if closed { 0.06 } else { 0.12 }) };
    let hover = tint.opacity(if closed { 0.12 } else { 0.3 });
    let edge = if attention {
        if e.status == Status::Ready { t.blue } else { t.orange }
    } else {
        tint.opacity(if closed { 0.35 } else { 1. })
    };
    let (select_id, menu_id) = (a.id.clone(), a.id.clone());
    let help = format!("{} · {}", a.title, day::due(a.due, lib.now));
    let title = div().flex_1().min_w_0().truncate().text_size(px(font::CAPTION)).font_weight(if exam { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM }).text_color(if closed { t.secondary } else { t.primary }).child(a.title.clone());
    let chip = div()
        .id(SharedString::from(format!("mchip-{}", a.id)))
        .relative()
        .flex_none()
        .w_full()
        .h(px(cal::CHIP))
        .pl(px(space::M))
        .pr(px(space::XS))
        .flex()
        .items_center()
        .gap(px(space::XS))
        .rounded(px(radius::INNER))
        .bg(if selected { hover } else { fill })
        .soft(hover, radius::INNER)
        .cursor_pointer()
        .key_nav()
        .when(selected, |c| c.border(px(1.5)).border_color(t.accent))
        .tooltip(tip(help))
        .on_click(move |_, _, cx| act(cx, |l| l.select(Some(select_id.clone()))))
        .when(!closed, |c| c.on_drag(DraggedCard { id: a.id.clone(), title: a.title.clone(), tint }, |d, _, _, cx| cx.new(|_| d.clone())))
        .child(div().absolute().top(px(space::XS)).bottom(px(space::XS)).left(px(space::XS)).w(px(cal::EDGE)).rounded_full().bg(edge))
        .when(exam, |c| c.child(icon("graduation-cap", iconsize::XS, t.readable(tint))))
        .child(if e.status == Status::Skipped { title.line_through() } else { title })
        .context_menu(move |menu, window, cx| super::ai::deadline_submenu(status_menu(menu, &menu_id, cx), &menu_id, window, cx));
    motion::flip(format!("mchip-{}", a.id), chip).into_any_element()
}

/// A deadline card on its way to another day.
#[derive(Clone)]
pub struct DraggedCard {
    id: String,
    title: String,
    tint: Hsla,
}

impl Render for DraggedCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        div().flex().items_center().gap(px(space::S)).px(px(space::M)).py(px(space::S)).rounded(px(radius::CONTROL)).bg(t.material).text_color(t.primary).shadow_md().child(div().size(px(space::S)).rounded_full().bg(self.tint)).child(self.title.clone())
    }
}

/// Moves a deadline to `date` and keeps its time of day.
fn move_to_day(id: &str, date: NaiveDate, cx: &mut App) {
    let id = id.to_string();
    act(cx, |l| {
        let Some(a) = l.assignment(&id).cloned() else { return };
        let Some(due) = calendar::same_time_on(a.due, date) else { return };
        if due.date_naive() == a.due.date_naive() {
            return;
        }
        if a.is_manual() {
            if let Some(mut manual) = l.manual_for(&a).cloned() {
                manual.due = due.with_timezone(&Utc);
                l.update_manual(manual);
            }
        } else {
            l.move_occurrence(&id, Some(due.with_timezone(&Utc)));
        }
    });
}

/// Makes a day accept dragged deadlines.
fn drop_target(day: Div, date: NaiveDate, t: &Theme) -> Div {
    let accent = t.accent;
    day.drag_over::<DraggedCard>(move |s, _, _, _| s.border_color(accent).bg(accent.opacity(0.08))).on_drop(move |card: &DraggedCard, _, cx| move_to_day(&card.id, date, cx))
}

/// Right-click on a day: add a one-off task, submission or exam due that day.
fn day_menu(menu: PopupMenu, date: NaiveDate) -> PopupMenu {
    let add = |label: String, kind: Kind| {
        PopupMenuItem::new(label).icon(menu_icon(kind.icon())).on_click(move |_, _, cx| {
            act(cx, |l| {
                l.quick_add_kind = Some(kind);
                l.quick_add_day = Some(date);
                l.show_quick_add = true;
            })
        })
    };
    menu.label(day::long(date))
        .item(add(tr("Abgabe eintragen…"), Kind::Submission))
        .item(add(tr("Aufgabe eintragen…"), Kind::Task))
        .item(add(tr("Prüfung eintragen…"), Kind::Exam))
}

fn day_group(date: NaiveDate) -> SharedString {
    SharedString::from(format!("board-day-{}", date.num_days_from_ce()))
}

/// The day menu behind a quiet "+" that comes forward while the pointer is over the day, so right-click is not the only way in.
fn add_button(date: NaiveDate) -> Div {
    div()
        .opacity(0.45)
        .group_hover(day_group(date), |s| s.opacity(1.))
        .child(Button::new(("board-day-add", date.num_days_from_ce() as u32 as usize)).ghost().small().icon(menu_icon("plus")).tooltip(tr("Für diesen Tag eintragen")).dropdown_menu(move |menu, _, _| day_menu(menu, date)))
}

fn day_label(d: NaiveDate, today: NaiveDate) -> String {
    if d == today { tr("Heute").to_uppercase() } else { day::weekday_short(d).to_uppercase() }
}

fn column(lib: &Library, d: NaiveDate, t: &Theme) -> AnyElement {
    let today = lib.today();
    let is_today = d == today;
    let weekend = day::is_weekend(d);
    let items = lib.assignments_on(d);
    let sessions = lib.sessions_on(d);
    let semester = lib.semester();
    let break_name = semester.and_then(|s| s.break_at(d)).map(|b| b.name.clone());
    let free = semester.is_some_and(|s| !s.contains(d) || break_name.is_some());
    let shade = if weekend { 0.02 } else { 0. } + if free { 0.03 } else { 0. };
    let open = items.iter().filter(|a| !lib.is_done(a)).count();
    let ink = t.ink;

    let header = div()
        .flex()
        .items_baseline()
        .gap(px(space::S))
        .pb(px(space::XXS))
        .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(if is_today { t.accent_text } else { t.secondary }).child(day_label(d, today)))
        .child(div().text_size(px(font::TITLE2)).font_weight(FontWeight::SEMIBOLD).text_color(if is_today { t.accent_text } else { t.primary }).child(d.day().to_string()))
        .child(div().flex_1())
        .when(open > 0, |h| h.child(div().text_size(px(font::CAPTION2)).font_weight(FontWeight::BOLD).text_color(t.secondary).child(open.to_string())))
        .child(add_button(d));

    drop_target(div(), d, t)
        .group(day_group(d))
        .id(("board-day", d.num_days_from_ce() as u32 as usize))
        .context_menu(move |menu, _, _| day_menu(menu, d))
        .flex_1()
        .min_w_0()
        .min_h(px(cal::COLUMN))
        .p(px(space::S))
        .flex()
        .flex_col()
        .gap(px(space::S))
        .rounded(px(radius::CARD))
        .bg(if is_today {
            t.content.blend(t.accent.opacity(0.12))
        } else if shade > 0. {
            t.content.blend(ink.opacity(shade))
        } else {
            t.column
        })
        .border(px(if is_today { 1.5 } else { 1. }))
        .border_color(if is_today { t.accent.opacity(0.7) } else { ink.opacity(0.06) })
        .when(d < today, |c| c.opacity(0.6))
        .child(header)
        .when_some(break_name.clone(), |c, name| {
            c.child(div().flex().items_center().gap(px(space::XS)).py(px(space::XXS)).text_size(px(font::CAPTION)).text_color(t.secondary).child(icon("sun", iconsize::XS, t.secondary)).child(name))
        })
        .children(sessions.iter().map(|(m, s)| session_chip(m, s, t)))
        .children(items.iter().map(|a| deadline_card(lib, a, t)))
        .when(items.is_empty() && sessions.is_empty() && break_name.is_none(), |c| {
            c.child(div().w_full().flex().justify_center().py(px(space::M)).text_size(px(font::CAPTION)).text_color(t.tertiary).child(if weekend { tr("Wochenende") } else { tr("Frei") }))
        })
        .into_any_element()
}

fn agenda_row(lib: &Library, d: NaiveDate, t: &Theme) -> AnyElement {
    let today = lib.today();
    let is_today = d == today;
    let items = lib.assignments_on(d);
    let sessions = lib.sessions_on(d);
    drop_target(div(), d, t)
        .group(day_group(d))
        .id(("board-agenda-day", d.num_days_from_ce() as u32 as usize))
        .context_menu(move |menu, _, _| day_menu(menu, d))
        .flex()
        .items_start()
        .gap(px(space::M))
        .p(px(space::M))
        .rounded(px(radius::CARD))
        .bg(if is_today { t.content.blend(t.accent.opacity(0.07)) } else { t.secondary_bg })
        .border(px(if is_today { 1.5 } else { 1. }))
        .border_color(if is_today { t.accent.opacity(0.7) } else { t.ink.opacity(0.06) })
        .when(d < today, |c| c.opacity(0.6))
        .child(
            div()
                .w(px(52.))
                .flex_none()
                .flex()
                .flex_col()
                .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(if is_today { t.accent_text } else { t.secondary }).child(day_label(d, today)))
                .child(div().text_size(px(font::TITLE3)).font_weight(FontWeight::SEMIBOLD).text_color(if is_today { t.accent_text } else { t.primary }).child(d.day().to_string())),
        )
        .child(if items.is_empty() && sessions.is_empty() {
            div().pt(px(space::S)).text_size(px(font::CALLOUT)).text_color(t.tertiary).child(if day::is_weekend(d) { tr("Wochenende") } else { tr("Frei") }).into_any_element()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap(px(space::S))
                .children(sessions.iter().map(|(m, s)| div().w(px(200.)).child(session_chip(m, s, t))))
                .children(items.iter().map(|a| div().w(px(200.)).child(deadline_card(lib, a, t))))
                .into_any_element()
        })
        .child(div().flex_1())
        .child(add_button(d))
        .into_any_element()
}

fn session_chip(m: &Module, s: &Session, t: &Theme) -> Stateful<Div> {
    let title = if s.title.is_empty() { m.code.clone() } else { s.title.clone() };
    let place = if s.location.is_empty() { String::new() } else { format!(" · {}", s.location) };
    div()
        .id(SharedString::from(format!("session-{}", s.id)))
        .flex()
        .gap(px(space::S))
        .py(px(space::XXS))
        .tooltip(tip(format!("{} · {} {}–{}", m.code, s.title, s.start, s.end)))
        .child(div().w(px(cal::EDGE)).flex_none().rounded_full().bg(t.palette(&m.color).opacity(0.6)))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_size(px(font::CAPTION)).font_weight(FontWeight::MEDIUM).truncate().child(title))
                .child(div().text_size(px(font::CAPTION2)).text_color(t.secondary).truncate().child(format!("{}–{}{place}", s.start, s.end))),
        )
}

/// The module's colour at a card's left edge, inset so the card's corners stay free.
pub fn edge_bar(color: Hsla) -> Div {
    div().absolute().top(px(space::S)).bottom(px(space::S)).left(px(space::XS)).w(px(cal::EDGE)).rounded_full().bg(color)
}

pub fn deadline_card(lib: &Library, a: &Assignment, t: &Theme) -> AnyElement {
    let e = lib.evaluation(a);
    let module = lib.module(a);
    let tint = t.palette(module.map_or("blue", |m| m.color.as_str()));
    let closed = e.status.is_closed();
    let selected = lib.is_selected(&a.id);
    let attention = lib.needs_action(a);
    let border = if selected {
        Some(t.accent)
    } else if attention {
        Some(if e.status == Status::Ready { t.blue.opacity(0.7) } else { t.secondary.opacity(0.6) })
    } else if a.optional && !closed {
        Some(t.secondary.opacity(0.5))
    } else {
        None
    };
    let base = t.column;
    let fill = base.blend(tint.opacity(if closed { 0.05 } else { 0.12 }));
    let hover_fill = base.blend(tint.opacity(if closed { 0.05 } else { 0.2 }));
    let (select_id, menu_id) = (a.id.clone(), a.id.clone());
    let tracks = a.tracks_files();
    let help = format!("{} · {}", a.title, day::due(a.due, lib.now));

    let mut card = div()
        .id(SharedString::from(a.id.clone()))
        .relative()
        .w_full()
        .py(px(space::S))
        .pl(px(space::M))
        .pr(px(space::S))
        .flex()
        .flex_col()
        .gap(px(space::XS))
        .rounded(px(radius::CONTROL))
        .bg(if selected { hover_fill } else { fill })
        .soft(hover_fill, radius::CONTROL)
        .cursor_pointer().key_nav()
        .tooltip(tip(help))
        .on_click(move |_, _, cx| act(cx, |l| l.select(Some(select_id.clone()))))
        .when(!closed, |c| c.on_drag(DraggedCard { id: a.id.clone(), title: a.title.clone(), tint }, |d, _, _, cx| cx.new(|_| d.clone())))
        .child(edge_bar(tint.opacity(if closed { 0.35 } else { 1. })))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(space::XS))
                .text_size(px(font::CAPTION))
                .child(status_toggle(a, lib, iconsize::M, t))
                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(t.readable(tint)).child(module.map_or(String::new(), |m| m.code.clone())))
                .child(div().text_color(t.secondary).child(day::hm(a.due))),
        )
        .child({
            let title = div().text_size(px(font::BODY)).font_weight(FontWeight::MEDIUM).line_clamp(3).text_color(if closed { t.secondary } else { t.primary }).child(a.title.clone());
            if e.status == Status::Skipped { title.line_through() } else { title }
        })
        .when(attention, |c| {
            let (label, color) = if e.status == Status::Ready { (tr("Upload bestätigen"), t.blue) } else { (tr("Noch offen"), t.secondary) };
            c.child(div().text_size(px(font::CAPTION2)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(label))
        });
    if let Some(color) = border {
        card = card.border(px(if selected || attention { 1.5 } else { 1. })).border_color(color);
        if a.optional && !closed && !selected {
            card = card.border_dashed();
        }
    }
    if tracks {
        card = file_target(card, a, t);
    }
    super::motion::flip(format!("card-{}", a.id), card.context_menu(move |menu, window, cx| super::ai::deadline_submenu(status_menu(menu, &menu_id, cx), &menu_id, window, cx))).into_any_element()
}
