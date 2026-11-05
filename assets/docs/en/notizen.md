# Notes

Notes are Markdown files in your folder. Obsidian reads them the same way.

## New Note

1. {key:newFile} or {ui:Neue Notiz}. Schrumbi creates `Unbenannt.md`.
2. Type a name, {key:return}.

{key:escape} keeps the name `Unbenannt.md`. On a module page, {key:newFile} opens a window where you choose type and place.

## Files

| Key | Effect |
|---|---|
| {key:up} {key:down} | Choose a file, with preview |
| {key:right} {key:left} | Expand and collapse folders |
| {key:return} | Jump into the note |
| {key:renameNote} | Rename |
| {key:/} | Filter |

- {key:renameNote} also renames the open note, as does a click on its title at the top.
- A single click opens a preview, a double click keeps the tab.
- Dragging a file onto a folder moves it.
- Right-click → {ui:Abgabe zuordnen} makes the file count for a submission. The pill at the top of the note shows what it belongs to.
- {ui:In den Papierkorb} does not ask. {key:Undo} brings the file back.

At the top you switch between {ui:Dateien}, {ui:Suche}, {ui:Tags} and {ui:Aufgaben}.

## Office Files

Word, Excel and PowerPoint open in Schrumbi, read-only.

| File | View |
|---|---|
| `.docx` | Pages like a PDF, portrait and landscape per section: zoom, search, copy text |
| `.pptx` | One slide below the other, with slide numbers, zoom and search |
| `.xlsx` `.xlsm` `.xls` `.ods` | Spreadsheet with sheets at the bottom |

In a spreadsheet:

| Key | Effect |
|---|---|
| {key:up} {key:down} {key:left} {key:right} | Select a cell |
| {key:shift-down} | Extend the selection |
| {key:cmd-down} | To the end of the column |
| {key:cmd-c} | Copy the selection, with tabs between cells |
| {key:cmd-f} | Search this sheet, {key:return} for the next match |
| {key:cmd-pageup} {key:cmd-pagedown} | Previous and next sheet |
| {key:escape} | Cancel a drag and keep the old selection |

Edit in the default app: ⋯ → {ui:Mit Standard-App öffnen}. When it saves, Schrumbi shows the new version. Password-protected files open only in the default app. When one sheet can't be read, the other sheets stay open.

## Reading and Editing

| Mode | Shortcut |
|---|---|
| {ui:Lesen} / {ui:Bearbeiten} | {key:togglePreview} |
| {ui:Geteilte Ansicht}, code editor only | {key:splitPreview} |

You choose the editor in {key:OpenSettings} → {ui:Notizen}: {ui:Live-Ansicht: Formatierung beim Schreiben sichtbar} or {ui:Code-Editor: Quelltext mit Zeilennummern}. {key:toggleLive}, {ui:Live-Ansicht} in the {ui:Darstellung} menu or the {key:CommandPalette} switches at once.

## Find in a Note

| Shortcut | Effect |
|---|---|
| {key:findInNote} | Open search, selected text is filled in |
| {key:cmd-alt-f} | Open search with the replace row |
| {key:return} / {key:shift-return} | Next / previous match |
| {key:escape} | Close search |

The shortcut also works while the focus is in the file list or on the tabs. In PDFs and spreadsheets it opens their search, even when they were only last active. Without an open note, PDF or spreadsheet, {ui:In Notiz suchen} is greyed out in the {ui:Bearbeiten} menu, {ui:In Notiz ersetzen} without a note.

All matches are marked, the current one stronger. Case and umlauts do not matter until you choose {ui:Groß-/Kleinschreibung beachten}.

{ui:In Notiz ersetzen} or the arrow on the left of the search shows the replace row. {key:return} replaces one match, {ui:Alle ersetzen} all of them. One {key:Undo} takes it back. Reading mode has no replace.

## Spelling

Unknown words are underlined. Point at one, then {ui:Zum Wörterbuch hinzufügen}, also in the right-click menu. Without a mouse: {ui:Zum Wörterbuch hinzufügen} in the {key:CommandPalette} adds the word at the caret.

The words are listed in the settings under {ui:Eigenes Wörterbuch}; the cross removes one. All open notes pick it up at once.

## Long notes

From about 100 KB the source does not wrap lines. Press {key:toggleWrap}, or use {ui:Zeilenumbruch ein-/ausschalten} in the {key:CommandPalette}, to turn wrapping on or off for this note, up to about 120 KB; Schrumbi remembers it. Above that it tells you why it does not work. From about 200 KB the source also shows no highlighting, so typing stays fast. The bar below says {ui:Lange Notiz: ohne Zeilenumbruch und Hervorhebung}.

A single line over about 250 KB opens in the source instead of Live. The bar below says {ui:Diese Zeile ist zu lang für Live. Esc: zurück zu Live}; {key:escape} or a click on it goes back.

Live shows only the start of a line over about 32 KB; {ui:Lesen} shows all of it, and a click on the note under it opens the line in the source.

From about 80 KB spelling checks only the text around the caret. The bar below says {ui:Rechtschreibung nur um die Schreibmarke}.

## Saving

Schrumbi saves after a 2 second typing pause, adjustable under {ui:Notizen} → {ui:Sichern}. {key:Save} saves at once, closing always saves.

If another app changed the file, you see a notice. Nothing is lost until you choose:

| Button | Result |
|---|---|
| {ui:Vergleichen} | Both versions side by side |
| {ui:Neu laden} | The other app's version |
| {ui:Meine behalten} | Your version |

## Linking

- Type `[[` and choose a note.
- {key:toggleInspector} shows {ui:Rückverweise}, {ui:Ausgehend} and {ui:Erwähnungen ohne Link}.
- {key:showGraph} shows all notes as a graph.

## Tabs

| Shortcut | Effect |
|---|---|
| {key:closeTab} | Close tab |
| {key:reopenTab} | Reopen the last closed tab |
| {key:nextTab} / {key:previousTab} | Next / previous tab |

- A tab only closes when you close it: ×, middle click or {key:closeTab}.
- A click on a file opens it as a preview with a slanted title. The next file replaces it.
- Double-click, start writing or right-click → {ui:Tab behalten} to turn the preview into a kept tab.
- A link opens its target as a preview; the note with the link stays open.
- Drag a tab to change the order. {key:reopenTab} brings a tab back to its old place.

### Side by side

- Right-click a tab → {ui:Daneben öffnen}, or drag a tab or a file from the list into the right half. It says {ui:Rechts öffnen} there.
- The tab of the right note shows a window with a filled right half as its icon. Clicking it jumps into the right note.
- A coloured line under the bar shows which note you are in.
- Links in the right note open on the right.
- {key:closeTab} closes the note you are in. When nothing is left on the left, the right note moves over.
- × at the top right ({ui:Geteilte Ansicht schließen}) ends side by side; both tabs stay open.

## More

| Command | Effect |
|---|---|
| {ui:Tagesnotiz öffnen} {key:dailyNote} | Note for today, created if needed |
| {ui:Als PDF exportieren} {key:exportPDF} | PDF next to the note |
| {ui:Verlauf} | Compare and restore older versions, when sync is set up |
| {ui:Als Folien zeigen} | Parts split by `---` become slides |

In the PDF headings stay with their text; short tables, code blocks, callouts, quotes and list items are not split across two pages, long tables repeat their header row. Pictures appear as large as in the view, never wider than the text and never taller than about half a page; a caption stays with its picture.

If an earlier export already lies next to the note, Schrumbi asks before replacing it. {ui:Rückgängig} in the message afterwards brings the old PDF back.

- {ui:Nicht mehr fragen} replaces without asking from then on.
- Turn the question back on under {key:OpenSettings} → {ui:Notizen} → {ui:Vor dem Ersetzen eines früheren Exports fragen}.
- Another file with that name stays untouched, the PDF gets a number. The next export replaces that numbered PDF.
