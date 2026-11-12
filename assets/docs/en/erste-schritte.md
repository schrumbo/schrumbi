# Getting Started

Schrumbi plans your studies in a normal folder: deadlines, files, notes and cards in one place. Everything stays a file you can open without Schrumbi.

## Choose a Folder

1. Click {ui:Ordner wählen…} or drag a folder into the window.
2. Schrumbi creates `schrumbi.json` there. Nothing else changes.
3. The overview is there at once. Folders that look like modules already are modules, and the lecture period is estimated.

You are only asked when something is needed: your name tag the first time you file a submission whose name uses it, the Moodle address when you want to fetch submissions.

## Setup

At the top of the {ui:Übersicht} sits a list that ticks itself off.

| Item | Done as soon as |
|---|---|
| {ui:Module} | there is at least one module |
| {ui:Vorlesungszeit} | you changed the dates or clicked {ui:Stimmt so} |
| {ui:Moodle-Kalender} | a calendar address is entered |
| {ui:Dateinamen} | files are named by pattern and your name tag is set |
| {ui:Notizen schreiben} | you chose {ui:Formatiert} or {ui:Text mit Zeilennummern} |
| {ui:Karten} | the first card exists |
| {ui:Sichern & Abgleichen} | syncing is set up |

- Only the next step is shown large, the others wait folded below it.
- {ui:Überspringen} counts as done. {ui:Doch einrichten} brings the item back. {ui:Alle überspringen} settles the rest at once.
- The {ui:Einrichten} row collapses the list, the X hides it.
- Finished or hidden counts for every device with this folder. What only concerns one device, like the calendar, comes up there one at a time, once.
- {key:OpenSettings} → {ui:Allgemein} → {ui:Einrichten in der Übersicht} shows it again, skipped items too.
- Once every item is done, the list says {ui:Alles eingerichtet} and goes away.

> [!tip] All at once
> {ui:Geführt einrichten…} at the bottom of the list or in {key:CommandPalette} walks through semester, modules and submissions in order.

## Tip of the Day

From the second start on, a tip shows once a day at the bottom right, only about things you have not used yet.

- {ui:Zeig's mir} opens the place and highlights it. A click or {key:escape} closes that.
- {ui:Nächster Tipp} shows another one.
- {ui:Keine Tipps mehr} turns them off, {key:OpenSettings} → {ui:Allgemein} → {ui:Tipp des Tages} back on.

## Guided Setup

Four short steps, {key:return} moves on.

| Step | You set |
|---|---|
| {ui:Willkommen} | Degree program, naming pattern and whether Schrumbi creates submission folders |
| {ui:Semester} | Name, lecture start, lecture end |
| {ui:Module} | Which folders are modules, with code and name |
| {ui:Abgaben} | Moodle calendar and a weekly submission, both optional |

You add breaks and holidays later in {key:OpenSettings} → {ui:Semester}.

In {ui:Module}, matching folders are already ticked: everything directly inside a semester folder like `WS25`, and folders with a course code like `INF1`. If there are neither, all top-level folders are ticked.

## The Areas

| Area | Shortcut | Shows |
|---|---|---|
| {ui:Übersicht} | {key:goToday} | What is due now |
| {ui:Semester} | {key:goGrid} | All weeks and modules |
| {ui:Leistungen} | {key:goRecord} | Points and grades |
| {ui:Inbox} | {key:goInbox} | Files that still need a place |
| {ui:Notizen} | {key:goNotes} | Notes, search, tags, tasks |
| {ui:Karten} | {key:goCards} | Flashcards to study |

- The sidebar shows icons with names. {ui:Seitenleiste einklappen} at the bottom left makes it narrow, then hovering names the area and its shortcut. Below 820 px window width it stays narrow.
- Right-click an area except {ui:Übersicht}: {ui:Aus der Seitenleiste ausblenden}.

## What Is in the Folder

| File | Contents |
|---|---|
| `schrumbi.json` | Semesters, modules, rules, grades |
| `.schrumbi-state.json` | Status you set by hand |
| `.schrumbi-cards.json` | Study progress of your cards |
| `.schrumbi-backup/` | The last 12 backups of `schrumbi.json` and status |

- If the status is damaged, Schrumbi restores the last backup by itself.
- If `schrumbi.json` is damaged, the window shows {ui:Letzte Sicherung wiederherstellen}.
- The damaged file stays next to it as `.defekt`.
