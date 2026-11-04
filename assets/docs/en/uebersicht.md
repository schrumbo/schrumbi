# Overview

What is due now, at a glance. {key:goToday} opens it.

## Due Now

The big card at the top shows the most urgent item, in this order:

1. Overdue items
2. Done, but upload not confirmed
3. The next deadline, if it comes in less than 3 days
4. Files in the inbox

| Situation | Button |
|---|---|
| Submission due, file is ready | {ui:Hochladen} |
| Submission due, no file yet | {ui:Datei ablegen…} |
| Upload not confirmed | {ui:Als hochgeladen markieren} |
| Files in the inbox | {ui:Jetzt ablegen} |

If nothing is open, it says {ui:Nichts offen}. Next to it, {ui:Diese Woche} shows how much of this week is done, and {ui:Pünktlich in Folge} shows your streak of on-time submissions.

## Set Status

- Click the circle in front of the entry: {ui:Erledigt}, for submissions with upload {ui:Hochgeladen}.
- Another click hands the status back to automatic detection.
- Right-click the entry for the rest:

| Command | Effect |
|---|---|
| {ui:In Arbeit} | Started, but not finished |
| {ui:Entfällt} | No longer counts |
| {ui:Termin verschieben…} | New date for an entry from a rule |
| {ui:Neue Notiz} | New note that belongs to the submission |
| {ui:Datei zuordnen…} | Find an existing file and add it |
| {ui:Abgabeordner öffnen} | Show the folder in the file manager |
| {ui:Automatisch erkennen} | Remove the status you set by hand |

{key:Undo} reverts status changes.

## Which Files Count

A file counts for a submission when it belongs to it. Date and folder alone are not enough.

| How a file comes to belong | Where |
|---|---|
| {ui:Neue Notiz} | On the right, at the submission |
| Export a note as PDF | The PDF belongs to the same submissions |
| Drag or file a file onto the submission | Overview, submissions, inbox |
| {ui:Abgabe zuordnen} | Right-click a file in {ui:Notizen} |
| Pill at the top of the note | Shows the submission, a click changes it |

- New notes and filed files go into the module's {ui:Abgabeordner}, e.g. `Analysis/Abgaben`. You change the name under {key:OpenSettings} → {ui:Allgemein}.
- Renaming and moving in Schrumbi keep the link. If a file is missing because another app moved it, it shows on the right as {ui:Nicht mehr gefunden}.
- If the submissions folder holds something from before the deadline, Schrumbi asks {ui:Gehört das dazu?} on the right. Only {ui:Zuordnen} makes it count.
- If a rule has a {ui:Wochenordner}, everything in it counts without linking.

| Linked | Status |
|---|---|
| Nothing, date not reached | {ui:Offen} |
| Nothing, date passed | {ui:Überfällig}, for optional ones {ui:Entfällt} |
| Only drafts: `.md`, `.txt`, `.rtf` | {ui:In Arbeit} |
| A finished file | {ui:Erledigt} |
| Correctly named file, upload pending | {ui:Bereit zum Hochladen} |

- Schrumbi cannot see whether you uploaded. You confirm that with the circle.
- Exams count as done after their date.
- A status set by hand always beats the detected one.

## Further Down

- Collected: {ui:Überfällig} and {ui:Fertig, aber Upload nicht bestätigt}, with {ui:Alle als hochgeladen markieren}.
- {ui:Nächste 7 Tage}, {ui:Prüfungen} and {ui:Zuletzt geschrieben}.
- If cards are waiting, a hint with {ui:Lernen} appears.

## Add for a Day

Right-click a day under {ui:Nächste 7 Tage}, or click the + at its top right. For one-off things without a rule:

| Command | Default time |
|---|---|
| {ui:Abgabe eintragen…} | 23:59 |
| {ui:Aufgabe eintragen…} | 18:00 |
| {ui:Prüfung eintragen…} | 10:00 |

The date is already filled in. Type the title, {key:return}. Right-clicking an entry still shows its own commands.
