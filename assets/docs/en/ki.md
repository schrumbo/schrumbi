# AI

Schrumbi works with Claude Code, Codex or OpenCode when one of them is installed. The program uses your own account there; what you show it goes to that provider.

## On and off

When a program is installed, the AI is on. {ui:KI-Assistent verwenden} in {key:OpenSettings} → {ui:KI} switches it off, {ui:Knopf in der Werkzeugleiste} under {ui:Für Fortgeschrittene} hides the button.

None installed yet? You need an account with Claude or ChatGPT.

1. Command search → {ui:KI-Assistent einrichten}
2. {ui:Claude Code installieren} opens the official guide.
3. Then {ui:Erneut suchen}. If the program is not signed in, it shows {ui:Anmeldung prüfen}.

The first start can take a minute; {ui:Abbrechen} stops it, {ui:Noch einmal} tries again.

## The column

{key:toggleAi} or the button {ui:KI einblenden} opens the column {ui:KI} on the right. It shares its place with the details: at the top you switch between {ui:Details} and {ui:KI}, {key:toggleInspector} jumps to the details. The same button again closes the column. Drag its left edge to make it wider or narrower; a double click resets the width. In a narrow window the column lies over the page; a click on the page or {key:escape} closes the AI, {key:escape} also the details.

1. Open a note or a PDF.
2. {key:toggleAi}
3. Click {ui:Karten aus dieser Seite} or write your own and press {key:return}.

Chips at the bottom show what the AI gets to see: open note, PDF page, selection, module. The × takes one out, {ui:Kontext} adds it back or another tab. The empty column offers up to three tasks; if sending does not work, your text stays in the input.

| Button | What happens |
|---|---|
| {ui:Folgen} | Changed files open and show the spot; off until you switch it on |
| {ui:Ansehen} | Before and after, one below the other |
| {ui:Rückgängig} | Takes back only this one change |
| {ui:Stopp} | Stops the request |
| {ui:Als Notiz speichern} | The conversation as a note in your folder |
| {ui:Frühere Unterhaltungen} | Open an old conversation and keep writing |
| {ui:Verwerfen} | In the comparison: take back only this spot, {ui:Übernehmen} writes the rest |

Right-clicking a file, a PDF page, a deadline or selected text shows {ui:KI} with fitting requests.

## Tasks

| Task | What happens |
|---|---|
| {ui:Karten erstellen} | Cards from a PDF page, pages, a selection or a note; they go as question :: answer into the note beside the PDF or into the module's {ui:Karteikarten} |
| {ui:Passage erklären} | Explains the passage and changes nothing; {ui:In Notiz einfügen} appends the explanation |
| {ui:Vorlesung zusammenfassen} | A new note in the module folder, named like the others there |
| {ui:Woche planen} | A plan from deadlines, due cards and the lecture week; tasks only after your yes |
| {ui:Inbox aufräumen} | Reads every file and shows a table: module, kind, date, deadline, new name; {ui:Ändern} changes a row right there |
| {ui:Abfragen} | Quizzes you, writes nothing and never rates a card |

In the filing window, {ui:Mit KI ausfüllen} fills the fields from the file. What the AI entered stays marked until you change it; nothing is filed before {ui:Ablegen}.

## Set up with AI

{ui:Mit KI einrichten} in the empty column or in the command search. If your folder already holds material, the AI reads it and derives modules, lecture period, naming pattern and weekly deadlines; otherwise it asks a few questions. Its proposal appears as a card: tick what fits, {ui:Alles übernehmen}, one {key:Undo} takes it all back. The AI only changes settings; renaming files it suggests at most in a list. After {ui:Sitzung beenden nach} without a message the program ends; the next message starts it again. Conversations stay in the app data, not in your folder. When you quit Schrumbi, the program ends too.

## In the terminal

1. {key:OpenSettings} → {ui:KI} → {ui:Für Fortgeschrittene} → {ui:Für die Kommandozeile}
2. {ui:Eintragen} next to your program.
3. Start it in your study folder, for example with {ui:Im Terminal öffnen}.

While Schrumbi has this folder open, it reads your notes, PDFs, deadlines and cards. Started in another folder, it only gets told where to start. Ask for example: Which submissions are due in the next 7 days?

The column's tasks are there as commands:

````example
/mcp__schrumbi__karten MA/VL3.pdf 12
/mcp__schrumbi__woche
/mcp__schrumbi__einrichten
````

## Changes

Before every change Schrumbi asks in the column, or at the bottom right of the window when the column is closed. {key:tab} moves between the buttons, {key:return} presses the chosen one, {key:escape} declines.

| Button | What happens |
|---|---|
| {ui:Erlauben} | This one change |
| {ui:In dieser Notiz heute erlauben} | The same change in this note without asking until midnight, only for the program that asked; for new notes {ui:In diesem Ordner heute erlauben} |
| {ui:Ablehnen} | Nothing is written |
| {ui:Ansehen} | The whole change, before you decide |

Long changes show the first lines, every line with a link and how many lines are left out. Filing, inbox, setup, status and deadlines always ask. Several changes of the same kind come as one card: {ui:Alle erlauben}, {ui:Alle ablehnen} or {ui:Einzeln ansehen}.

The changes of one request in the column are one step: {key:Undo} takes them back together. From the terminal every change is its own step. If you changed the note in the meantime, the AI writes nothing and reads it again.

## What the AI never does

It deletes nothing, renames nothing and never rates cards. Codex runs only in its read-only mode, OpenCode without its own file edits; both change things only through Schrumbi. It never overwrites unsaved text. Hidden files and folders whose name starts with a dot, and Schrumbi's own files, it neither reads nor changes. Instructions written in your notes or PDFs it does not follow; it tells you about them instead. It runs commands only when you switch on {ui:Befehle ausführen erlauben}.
