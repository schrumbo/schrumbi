# Semesters & Modules

A semester has lecture weeks, a module has a folder. Schrumbi needs nothing more.

## Semester

{key:GoGrid} shows the whole semester.

- Top: progress and a band of weeks. Clicking a lecture week jumps there.
- {ui:Module}: every module with its next deadline. Clicking opens the module page.
- {ui:Wochen}: {ui:Diese Woche} is open, older ones are under {ui:Vergangene Wochen}.

## Module Page

Clicking a module in the sidebar opens its page.

| Key | Effect |
|---|---|
| {key:1} {key:2} {key:3} | Tabs {ui:Übersicht}, {ui:Abgaben}, {ui:Dateien} |
| {key:n} | {ui:Neue Notiz} in the module |
| {key:f} | {ui:Ablegen} for the next open submission |
| {key:up} {key:down} | Move through the entries |

## Create a Module

- In {ui:Notizen}, right-click a folder: {ui:Als Modul verwenden}.
- Or {key:OpenSettings} → {ui:Module}. There you find {ui:Kürzel}, {ui:Name}, {ui:Semester} and {ui:Ordner}.

Schrumbi moves nothing in the process.

## Recurring Submissions

A worksheet every week? One rule instead of 14 entries:

1. {key:OpenSettings} → {ui:Module} → choose a module.
2. {ui:Regel hinzufügen} and choose a template.
3. Adjust title, day and time.

The rule's folder can use placeholders:

| Placeholder | Becomes |
|---|---|
| `{w}` | Lecture week: `01`, `02`, … |
| `{kw}` | Calendar week: `44` |
| `{date}` | Date as `yyMMdd` |

`W{w}_KW{kw}` becomes `W03_KW44`. Deadlines during a break are dropped, deadlines on holidays move to the next day.

## Grades

{key:GoRecord} shows points and grades.

- A {ui:Note} sets {ui:Bestanden} to match the grading system.
- Clicking a module name opens its partial results, such as exam 60 % and project 40 %.
- From two partial results on, there is a {ui:Zielnote}. Schrumbi shows which average the open parts still need.
