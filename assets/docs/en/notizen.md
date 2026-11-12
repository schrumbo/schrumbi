# Notes

Notes are Markdown files in your folder. Obsidian reads them the same way.

## New Note

1. {key:newFile} or {ui:Neue Notiz}. Schrumbi creates `Unbenannt.md`.
2. Type a name, {key:return}.

{key:escape} keeps the name `Unbenannt.md`. On a module page, {key:newFile} opens a window where you choose type and place.

## Files

| Key | Effect |
|---|---|
| {key:up} {key:down} | Choose a file |
| {key:right} {key:left} | Expand and collapse folders |
| {key:return} | Open and jump into the note |
| {key:renameNote} | Rename |
| {key:/} | Filter |

- {key:renameNote} also renames the open note, as does a click on its title at the top.
- A click opens the file in its own tab. If it is already open, its tab comes to the front.
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
| `.csv` `.tsv` | One spreadsheet; {ui:Als Text zeigen} in the file menu shows the text, the file stays unchanged |

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

Audio (`.mp3` `.m4a` `.wav` `.aiff`) opens with a play button and the length.

Edit in the default app: ⋯ → {ui:Mit Standard-App öffnen}. When it saves, Schrumbi shows the new version. Password-protected files open only in the default app. When one sheet can't be read, the other sheets stay open.

## PDFs

Highlight and move pages, in the file itself and only for PDFs.

| Shortcut | Effect |
|---|---|
| Select text, {key:markText} | Highlight in the color you chose last |
| Color in the bar | Colors the highlight, an existing one too |
| Click a highlight, {key:delete} | Remove. {ui:Markierung löschen} works with the mouse too |
| {key:Undo} | Takes back highlighting and removing |

Highlighting the same passage again recolors it instead of doubling it.

### Drawing

| Shortcut | Effect |
|---|---|
| {key:penTool} | Pick up or put down the {ui:Stift} |
| {key:eraserTool} | Pick up or put down the {ui:Radierer} |
| Drag on the page | Draw; with the eraser, touching a stroke deletes it |
| {key:escape} | Put the tool down |

- With the pen in hand, color and width ({ui:Dünn} or {ui:Dick}) appear in a strip at the bottom of the page.
- Every stroke and every sweep of the eraser is one step for {key:Undo}, with no message.
- While the PDF loads or saves, and when the folder cannot be written, the tools are grey. The tooltip says why.
- The eraser deletes only drawings, never highlights or notes.
- Click a drawing, {key:delete} removes it.
- A stroke that leaves the page ends at the edge. Thin strokes and dots are at least two pixels wide.
- Colors, width and {ui:Auswahl} are also in the command palette, the search word draw finds them. You set a shortcut for them in Settings, they ship without one.

### Notes

| Shortcut | Effect |
|---|---|
| {key:noteTool}, click on the page | {ui:Notiz setzen}: a note at that spot |
| {key:noteTool}, then {key:return} | A note in the middle of the visible page |
| Select text, {key:noteOnSelection} or {ui:Notiz hinzufügen} in the bar | Highlight and note in one step |
| Click a highlight, {ui:Notiz hinzufügen} in the bar | A note on the highlight |
| Click a note | Edit |
| {ui:Speichern} or {key:cmd-return} | Save. A click outside saves too |
| {key:escape} | Discard |

- An empty note places nothing. Emptying an existing note deletes it, the highlight stays. {ui:Notiz löschen} only exists for stored notes.
- With a new pin, a pale pin shows the spot while the note is open. Under the field it says how to save or discard.
- A note that is still open is saved: when you close the PDF, before a page change, when you switch folders and when you quit.
- A note on text shows as a small symbol left of the first line, never on the text.
- Everything is also listed in the {ui:Marker} tab, a click jumps there. {key:up} and {key:down} change the row, {key:return} jumps, {key:delete} removes. Without a mouse: {ui:Markierungen anzeigen} in the palette or {key:markList}.

### Export with marks

| Shortcut | Effect |
|---|---|
| {key:exportMarkedPdf} | {ui:Mit Markierungen exportieren} |

- Writes a copy next to the PDF. Highlights, notes and drawings are real annotations that other programs show too.
- The original PDF stays as it is. {key:Undo} removes the copy again.
- Without marks, in a read-only folder and while the PDF loads or saves, the button is grey. The tooltip says why.
- A message names the copy, {ui:Öffnen} shows it.

### Pages

On the left in {ui:Seiten}, the page list. {key:pageList} shows and hides it. A click selects and jumps, Ctrl plus click (Mac: ⌘) adds, Shift plus click selects a range. With nothing selected, every command acts on the page you see.

| Command | Shortcut |
|---|---|
| {ui:Seiten links drehen} | {key:rotatePagesLeft} |
| {ui:Seiten rechts drehen} | {key:rotatePagesRight} |
| {ui:Seiten nach oben verschieben} | {key:movePagesUp} |
| {ui:Seiten nach unten verschieben} | {key:movePagesDown} |
| {ui:Seiten löschen} | {key:deletePages} or {key:delete} in the page list |
| {ui:Seiten aus PDF einfügen …} | {key:insertPdf} |

- The commands name the page or say how many pages they act on. The bar under the list counts from two selected pages on, the cross clears the selection from one. Deleting all pages says that one stays.
- {key:rotatePagesLeft}, {key:rotatePagesRight} and {key:insertPdf} only work in the page list, the other shortcuts and the command palette everywhere.
- In the page list: {key:up} and {key:down} change the page, {key:home} and {key:end} go to the first and last, {key:pageup} and {key:pagedown} page through, {key:shift-up}, {key:shift-down}, {key:shift-home}, {key:shift-end}, {key:shift-pageup} and {key:shift-pagedown} extend the selection, {key:cmd-a} selects all, {key:shift-f10} opens the menu.
- Drag pages, the line shows where they land. {key:escape} cancels the drag.
- Drag PDFs from the file manager onto the page list to insert them together at the line, as one step for {key:Undo} with one message. While a PDF is being saved, more wait. {key:escape} stops the rest, those already in stay.
- Drag a page from the page list into a note to embed it there, as in the example below; several selected pages become one paragraph each. The line shows where the paragraph lands, {key:Undo} in the note takes it back. The PDF itself, dragged as a tab or from the file list, embeds the whole file; dropped at the right edge it opens beside.
- Drag a page onto the page list of another PDF to insert a copy there at the line, as one step for {key:Undo}. The source stays as it is.
- After a delete, the page at the deleted spot is selected, so the keyboard can carry on.
- For Word and PowerPoint files, a note under the list says why pages cannot be changed there.
- Right-clicking a page shows the same commands.
- A notice goes away with {key:escape}.
- The file really changes. {key:Undo} and {key:Redo} restore it byte for byte, and the highlights stay with their text.
- The last page cannot be deleted. Files that are not PDFs are skipped.

````example Link a page
![[Skript.pdf#page=3]]
````

## Reading and Editing

| Mode | Shortcut |
|---|---|
| {ui:Lesen} / {ui:Bearbeiten} | {key:togglePreview} |
| {ui:Geteilte Ansicht}, source only | {key:splitPreview} |

You choose the editor in {key:OpenSettings} → {ui:Notizen}: {ui:Live-Ansicht: Formatierung beim Schreiben sichtbar} or {ui:Quelltext: Text mit Zeilennummern}. {key:toggleLive}, {ui:Live-Ansicht} in the {ui:Darstellung} menu or the {key:CommandPalette} switches at once.

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

From about 20 KB the source does not wrap lines. Press {key:toggleWrap}, or use {ui:Zeilenumbruch ein-/ausschalten} in the {key:CommandPalette}, to turn wrapping on or off for this note, up to about 35 KB; Schrumbi remembers it. Above that it tells you why it does not work. From about 200 KB the source also shows no highlighting, so typing stays fast. The bar below says {ui:Lange Notiz: ohne Zeilenumbruch und Hervorhebung}.

A single line over about 250 KB opens in the source instead of the Live view. The bar below says {ui:Diese Zeile ist zu lang für die Live-Ansicht. Esc: zurück zur Live-Ansicht}; {key:escape} or a click on it goes back.

The Live view shows only the start of a line over about 32 KB; {ui:Lesen} shows all of it, and a click on the note under it opens the line in the source.

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
- Every file gets its own tab. An open file is never opened twice: its tab comes to the front.
- A link opens its target in a new tab; the note with the link stays open.
- Right-click a tab: {ui:Andere Tabs schließen}, {ui:Tabs rechts schließen}, {ui:Alle Tabs schließen}.
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
