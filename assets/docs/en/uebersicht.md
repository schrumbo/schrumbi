# Overview

What is due now, at a glance. {key:GoToday} opens it.

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
| {ui:Datei zuordnen…} | Use an existing file as the submission |
| {ui:Automatisch erkennen} | Remove the status you set by hand |

{key:Undo} reverts status changes.

## What Schrumbi Detects on Its Own

For submissions, Schrumbi looks in the folder. Files since the previous deadline count.

| In the folder | Status |
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
