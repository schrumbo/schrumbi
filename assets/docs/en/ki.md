# AI

Schrumbi works with Claude Code, Codex or OpenCode when one of them is installed. The program uses your own account there; what you show it goes to that provider.

## On and off

When a program is installed, the AI is on. {ui:KI-Assistent verwenden} in {key:OpenSettings} → {ui:KI} switches it off, {ui:Knopf in der Werkzeugleiste} hides the button.

## The column

{key:toggleAi} or the button {ui:KI einblenden} opens the column {ui:KI} on the right. It shares its place with Details; {key:toggleInspector} brings Details back.

1. Open a note or a PDF.
2. {key:toggleAi}
3. Click {ui:Karten aus dieser Seite} or write your own and press {key:return}.

Chips at the bottom show what the AI gets to see: open note, PDF page, selection, module. The × takes one out, {ui:Kontext} adds it back or another tab.

| Button | What happens |
|---|---|
| {ui:Folgen} | Changed files open and show the spot |
| {ui:Ansehen} | Before and after side by side |
| {ui:Rückgängig} | Takes back only this one change |
| {ui:Stopp} | Stops the request |
| {ui:Als Notiz speichern} | The conversation as a note in your folder |

Right-clicking a file, a deadline or selected text shows {ui:KI} with fitting requests. After {ui:Sitzung beenden nach} without a message the program ends; the next message starts it again. Conversations stay in the app data, not in your folder.

## In the terminal

1. {key:OpenSettings} → {ui:KI} → {ui:Für die Kommandozeile}
2. {ui:Eintragen} next to your program.
3. Start it in the terminal, for example with {ui:Im Terminal öffnen}.

While Schrumbi runs, it reads your notes, PDFs, deadlines and cards. Ask for example: Which submissions are due in the next 7 days?

## Changes

Before every change Schrumbi asks in the column, or at the bottom right of the window when the column is closed.

| Button | What happens |
|---|---|
| {ui:Erlauben} | This one change |
| {ui:Immer heute} | This kind of change without asking until midnight |
| {ui:Ablehnen} | Nothing is written |

The changes of one request are one step: {key:Undo} takes them back together. If you changed the note in the meantime, the AI writes nothing and reads it again.

## What the AI never does

It deletes nothing, renames nothing and never rates cards. It never overwrites unsaved text. It runs commands only when you switch on {ui:Befehle ausführen erlauben}.
