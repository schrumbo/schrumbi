# Search & Commands

Three ways to find anything.

| What | Shortcut | Finds |
|---|---|---|
| **Command palette** | {key:CommandPalette} | Every command in the app |
| {ui:Datei schnell öffnen} | {key:quickOpen} | Files by name |
| {ui:In allen Dateien suchen} | {key:searchFiles} | Text in notes, PDFs and names |

## Command Palette

Type, choose with {key:up} {key:down}, {key:return} runs it. With no input, your latest commands are on top. Umlauts do not matter.

| You type | You get |
|---|---|
| `synchro` | {ui:Jetzt synchronisieren} |
| `tages` | {ui:Tagesnotiz öffnen} |
| `pdf` | {ui:Als PDF exportieren}, when a note is open |
| `delete` | {ui:Notiz löschen}, when a note is open |

With no match, {ui:In allen Dateien suchen} with your input is on top, {key:return} starts the search.

## Quick Open

With no input, open tabs and recently opened files are on top. With input, the name that matches exactly is first, then names that start with it.

| You type | Result |
|---|---|
| `protocol` | Files with "protocol" in the name or folder |
| `#exam` | Notes with the tag `#exam` |
| `Script#Chapter 2` | Jumps to the heading in the note |

If the name does not exist yet, Schrumbi offers to create the note.

## Search All Files

Searches names, folders, PDFs and notes. Matching entries are listed on top as well. {key:up} {key:down} choose a match, {key:return} opens it. The caret then sits at the start of the match, which lights up briefly.

| Input | Finds |
|---|---|
| `w3` or `woche 3` | Files from lecture week 3 |
| `kw44` | Files from the lecture week in calendar week 44 |
| `INF1` | Files from the module with this code |

Search operators narrow it further. With them, notes are searched, PDFs by name only.

| Input | Finds |
|---|---|
| `tag:#exam` | Notes with this tag |
| `"exactly this"` | The phrase in exactly this form |
| `exam -old` | "exam", but not "old" |
| `essay OR talk` | Either one |
| `path:Statistics` | Files whose path contains "Statistics" |
| `task-todo:read` | Open tasks with "read" |
| `[status:open]` | Notes with this property |

All operators are in the {ui:Suchbefehle} menu next to the search field.

## Shortcuts and Help

- {key:ShowShortcuts} shows all shortcuts.
- {key:ShowHelp} opens this guide.
