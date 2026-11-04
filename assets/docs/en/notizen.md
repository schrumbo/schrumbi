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
| {key:f2} | Rename |
| {key:/} | Filter |

- A single click opens a preview, a double click keeps the tab.
- Dragging a file onto a folder moves it.
- Right-click → {ui:Abgabe zuordnen} makes the file count for a submission. The pill at the top of the note shows what it belongs to.
- {ui:In den Papierkorb} does not ask. {key:Undo} brings the file back.

At the top you switch between {ui:Dateien}, {ui:Suche}, {ui:Tags} and {ui:Aufgaben}.

## Office Files

Word, Excel and PowerPoint open in Schrumbi, read-only.

| File | View |
|---|---|
| `.docx` | Pages like a PDF: zoom, search, copy text |
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

You choose the editor in {key:OpenSettings} → {ui:Notizen}: {ui:Live: Text bleibt gezeichnet} or {ui:Code-Editor: Quelltext mit Zeilennummern}.

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
- {key:ShowGraph} shows all notes as a graph.

## Tabs

| Shortcut | Effect |
|---|---|
| {key:closeTab} | Close tab |
| {key:ReopenTab} | Reopen the last closed tab |
| {key:nextTab} / {key:previousTab} | Next / previous tab |

Right-click another tab → {ui:Daneben öffnen} shows it next to the current note.

## More

| Command | Effect |
|---|---|
| {ui:Tagesnotiz öffnen} {key:dailyNote} | Note for today, created if needed |
| {ui:Als PDF exportieren} {key:exportPDF} | PDF next to the note |
| {ui:Verlauf} | Compare and restore older versions, when sync is set up |
| {ui:Als Folien zeigen} | Parts split by `---` become slides |

If an earlier export already lies next to the note, Schrumbi asks before replacing it. {ui:Rückgängig} in the message afterwards brings the old PDF back.

- {ui:Nicht mehr fragen} replaces without asking from then on.
- Turn the question back on under {key:OpenSettings} → {ui:Notizen} → {ui:Vor dem Ersetzen eines früheren Exports fragen}.
- Another file with that name stays untouched, the PDF gets a number. The next export replaces that numbered PDF.
