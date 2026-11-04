# Getting Started

Schrumbi plans your studies in a normal folder: deadlines, files, notes and cards in one place. Everything stays a file you can open without Schrumbi.

## Choose a Folder

1. Click {ui:Ordner wählen…} or drag a folder into the window.
2. Schrumbi creates `schrumbi.json` there. Nothing else changes.
3. Setup starts. {ui:Später einrichten} skips it.

> [!tip] Again later
> Open {key:CommandPalette} and choose {ui:Einrichtung erneut starten}.

## Setup

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

- Clicking the open area or {key:ToggleSidebar} shows and hides the labels.
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
