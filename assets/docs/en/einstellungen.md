# Settings

{key:OpenSettings} opens the settings. At the top right you search all pages, {key:cmd-f} jumps into it.

| Page | Here you set |
|---|---|
| {ui:Allgemein} | Appearance, custom themes, language, reminders, naming pattern, sidebar |
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
2. Press the new keys. A modifier key must be included, F keys work alone. Without a modifier a hint appears. Copy, paste, cut, select all, undo, redo, close tab, quit, {key:tab} and the text editing keys (Shift with arrows and {key:return}, word and line jumps, deleting words) cannot be set; the reason appears on the row. All other Schrumbi keys can be reassigned.
3. {key:escape} cancels, {key:delete} removes the shortcut.

A click in the search field or elsewhere ends recording. If something is in the search field, {key:escape} clears it. If a shortcut is used twice, {ui:Doppelt} and the other command appear on the row. While recording, the key triggers nothing, so a key already in use can be set too. {ui:Alle zurücksetzen} restores the defaults. After every change a notice naming the command and its keys appears at the top of the page, with {ui:Rückgängig}, which brings the previous state back. It stays in place while you scroll and disappears after a few seconds, with its close button or with {key:escape}. On the {ui:Tastenkürzel} page the search at the top right filters the list of shortcuts.

## Next Semester

Choose a semester in the list, then the copy icon under the list: {ui:Folgesemester mit denselben Modulen und Regeln anlegen}. It starts two weeks after the last semester.

## Custom Themes

{ui:Design laden…} beside {ui:Erscheinungsbild} opens a JSON file. Schrumbi checks it, keeps a copy and switches at once. {ui:Entfernen} under {ui:Eigene Designs} deletes the copy, {ui:Rückgängig} brings it back. If a loaded theme has the name of an existing one, it replaces it, and {ui:Rückgängig} brings the old one back. If the file of the chosen theme goes away, the appearance from before applies again. {key:CommandPalette} lists every theme by name and offers {ui:Design laden…}.

```json
{
  "schemaVersion": 1,
  "name": "My Theme",
  "content": "#FDF6E3",
  "ink": "#073642",
  "accent": "#268BD2"
}
```

Schrumbi fills in everything else from these three colours.

| Key | Meaning |
|---|---|
| `content`, `ink`, `accent` | Required: surface, text, accent |
| `dark` | `true` or `false`; if missing, the brightness decides |
| `sidebar`, `secondaryBg`, `column`, `material` | Surfaces |
| `primary`, `secondary`, `tertiary`, `separator`, `sidebarBorder` | Text and lines, also as `#RRGGBBAA` |
| `accentText`, `onAccent`, `selection` | Accent as text, text on the accent, selection |
| `green`, `blue`, `orange`, `red`, `gray`, `greenText`, `orangeText` | States |
| `moduleColors` | Module colours: `blue`, `green`, `orange`, `purple`, `pink`, `teal`, `red`, `yellow`, `indigo`, `mint`, `brown`, `gray` |
| `syntax` | Code colours: `keyword`, `string`, `comment`, `number`, `type`, `function` |

Colours are written as `#RRGGBB`. On a mistake Schrumbi names the key and nothing changes; text needs 4.5 : 1 on `content` and `sidebar`. Edits to the copy in the `themes` folder apply when the window is in front again. Ready-made themes are in the `themes` folder of the source code.

## Where Settings Live

| What | Where |
|---|---|
| Semesters, modules, rules, name, naming pattern | `schrumbi.json` in your folder |
| Appearance, shortcuts, language | With Schrumbi on this device |
| Loaded themes | Folder `themes` with Schrumbi on this device |

Your files go only to your own sync storage. Only your folder and a connected Obsidian vault are read.
