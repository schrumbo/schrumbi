# Settings

{key:OpenSettings} opens the settings. At the top right you search all pages, {key:cmd-f} jumps into it.

| Page | Here you set |
|---|---|
| {ui:Allgemein} | Appearance, language, naming pattern, sidebar, reminders |
| {ui:Semester} | Lecture period, breaks, holidays |
| {ui:Module} | Codes, folders, links, recurring deadlines, timetable |
| {ui:Notizen} | Editor, font size, personal dictionary, saving, PDF export, Obsidian |
| {ui:Karten} | Retention target, new cards and reviews per day |
| {ui:Sync} | Online storage, automatic mode, switching off |
| {ui:Tastenkürzel} | Change most shortcuts |

## Search

Search knows other words too: `dark` finds the appearance, `git` the sync. It also finds modules, semesters and codes.

| Key | Effect |
|---|---|
| {key:up} {key:down} | Choose a hit |
| {key:return} | Jump there, the setting lights up briefly |
| {key:escape} | Clear search |

## Change Shortcuts

1. Open {ui:Tastenkürzel} and click a shortcut or its name, or Tab to it and press {key:return}.
2. Press the new keys. A modifier key must be included, for arrows, Tab and Return Shift is enough, F keys work alone. Without a modifier a hint appears. Shortcuts that Schrumbi or the system use for fixed purposes (copy, paste, close tab) cannot be set; the reason appears on the row.
3. {key:escape} cancels, {key:delete} removes the shortcut.

A click in the search field or elsewhere ends recording. With no hit, {key:escape} in the field clears the search. If a shortcut is used twice, {ui:Doppelt} appears next to it, and its hint names the other command. While recording, the key triggers nothing, so a key already in use can be set too. {ui:Alle zurücksetzen} restores the defaults, {ui:Rückgängig} in the notice next to the search field brings your shortcuts back; it disappears after a few seconds.

## Next Semester

Choose a semester in the list, then the copy icon under the list: {ui:Folgesemester mit denselben Modulen und Regeln anlegen}. It starts two weeks after the last semester.

## Where Settings Live

| What | Where |
|---|---|
| Semesters, modules, rules, name, naming pattern | `schrumbi.json` in your folder |
| Appearance, shortcuts, language | With Schrumbi on this device |

Your files go only to your own sync storage. Only your folder and a connected Obsidian vault are read.
