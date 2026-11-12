# Semesters & Modules

A semester has lecture weeks, a module has a folder. Schrumbi needs nothing more.

## Semester

{key:goGrid} shows the whole semester.

- Top: progress and a band of weeks. Clicking a lecture week jumps there.
- {ui:Module}: every module with its next deadline. Clicking opens the module page.
- {ui:Wochen}: {ui:Diese Woche} is open, older ones are under {ui:Vergangene Wochen}.

## Module Page

Clicking a module in the sidebar opens its page. On the left are all {ui:Abgaben} with their files and {ui:Zuletzt geschrieben}, on the right cards, sessions, grade and links.

- What is still open comes first, under {ui:Noch offen}. The grey number next to {ui:Abgaben} counts exactly these entries, like the sidebar.
- Then the next two weeks. Later and finished submissions are collapsed and show the first five.
- Finished entries slide out of the list, {ui:Erledigt} lights up briefly.
- {ui:Neuer Eintrag…} next to {ui:Abgaben} adds a submission.
- {ui:Alle Dateien} shows the module folder in the file tree.
- On the module page the search field at the top searches only this module. A click on the code next to it searches everywhere.

| Key | Effect |
|---|---|
| {key:n} | {ui:Neue Notiz} in the module |
| {key:f} | {ui:Ablegen} for the next open submission |
| {key:up} {key:down} | Move through the entries |
| {key:space} | Finish the selected entry, the selection moves to the next |
| {key:escape} | In the search field: clear first, then back to the page |

## Create a Module

- In {ui:Notizen}, right-click a folder: {ui:Als Modul verwenden}.
- Or {key:OpenSettings} → {ui:Module}. There you find {ui:Kürzel}, {ui:Name}, {ui:Semester} and {ui:Ordner}.

Schrumbi moves nothing in the process.

## Recurring Submissions

A worksheet every week? One rule instead of 14 entries:

1. {key:OpenSettings} → {ui:Module} → choose a module.
2. {ui:Regel hinzufügen} and choose a template.
3. Adjust title, day and time.

Files belong to the week's submission once you file them, create a note for it or link them. If every week has its own folder, enter it under {ui:Erweitert} → {ui:Wochenordner}. What lies in it then counts without linking. It can use placeholders:

| Placeholder | Becomes |
|---|---|
| `{w}` | Lecture week: `01`, `02`, … |
| `{kw}` | Calendar week: `44` |
| `{date}` | Date as `yyMMdd` |

`W{w}_KW{kw}` becomes `W03_KW44`. Deadlines during a break are dropped, deadlines on holidays move to the next day.

## Grades

{key:goRecord} shows points and grades.

- A {ui:Note} sets {ui:Bestanden} to match the grading system.
- Clicking a module name opens its partial results, such as exam 60 % and project 40 %.
- From two partial results on, there is a {ui:Zielnote}. Schrumbi shows which average the open parts still need.
