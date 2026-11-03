# Filing & Inbox

Drop a file on Schrumbi, and it lands in the right folder with the right name.

## File a File

1. Drag a file onto an entry, a module or the {ui:Inbox}.
2. Schrumbi suggests target and name.
3. {key:return} files it.

Filed onto an entry, the file goes into the module's {ui:Abgabeordner} and belongs to the submission. Dragged from {ui:Notizen} onto an entry, it stays where it is and only comes to belong to it.

| Key | Effect |
|---|---|
| {key:return} | File it |
| {key:e} | Open all fields, then {key:CommitFiling} files it |
| {key:1} {key:2} {key:3} | Take another suggestion |
| {key:escape} | Close fields, otherwise cancel |

Under {ui:Was ist das?} you switch between {ui:Meine Abgabe}, {ui:Aufgabenstellung} and {ui:Unterlagen}.

> [!note] Move or copy
> Files from your study folder are moved. Files from outside are copied, the original stays. Files dragged into the {ui:Inbox} are always copied.

## Naming Convention

With {ui:Nach einem Muster benennen}, submissions get uniform names.

| Placeholder | Example |
|---|---|
| `{code}` | `P1` |
| `{yy}` / `{yyyy}` | `26` / `2026` |
| `{title}` | `Stakeholder-Radar` |
| `{author}` | `Muster-M_Mueller-C` |
| `{date}` | `261028` |
| `{w}` / `{kw}` | `W03` / `KW44` |
| `{date:yyyy-MM-dd}` | `2026-10-28` |
| `{matrikel}` | Your student ID |

The pattern `{code}_{yy}_{title}_{author}_{date}` becomes:

```
P1_26_Stakeholder-Radar_Muster-M_Mueller-C_261028.pdf
```

A task description is then named `P1_26_Infografik_Aufgabe_261028.pdf`.

Whatever you create or rename keeps the name you type. A file gets the pattern only when you ask for it:

1. {ui:Mehr} or right-click the file → {ui:Nach Konvention benennen…}, or {key:CommandPalette} and the same command
2. Check the suggestion and change it if needed. Below it you see whether it follows the pattern.
3. {key:return} or {ui:Umbenennen}. {key:escape} cancels.

The suggestion takes a note's heading as title and keeps date and co-authors from the old name. A note with a pattern name no longer changes its name when you change the heading.

## Inbox

The inbox is the folder `_Inbox` in your study folder, you change the name under {ui:Inbox-Ordner}. What lies there waits for a place. {key:GoInbox} opens it.

| Key | Effect |
|---|---|
| {key:up} {key:down} | Choose a file |
| {key:space} | Preview |
| {key:return} | File it |
| {key:delete} | To the trash |

{ui:Dateien hinzufügen…} brings in files from elsewhere.
