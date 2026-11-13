# Overview

What is due now, at a glance. {key:goToday} opens it.

## Due Now

The big card at the top ({ui:Jetzt}) shows the most urgent item, in this order:

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

If nothing is open, it says {ui:Nichts offen}. Next to it, {ui:Diese Woche} shows how much of this week is done. {ui:Pünktlich in Folge} appears once you have a streak of on-time submissions.

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

- One {ui:Offen} row: everything else still open. A click unfolds the entries, {ui:Alle erledigt} marks them all at once.
- The calendar, {ui:Prüfungen} and {ui:Zuletzt geschrieben}.
- If cards are waiting, a hint with {ui:Lernen} appears.

## Calendar

The week in columns or the month as a grid. {ui:Woche} and {ui:Monat} at the top right switch, ‹ and › page, {ui:Heute} jumps back. The choice is remembered.

The week starts today and shows the next seven days. From there ‹ and › page in weeks from Monday to Sunday: › to the week from next Monday, ‹ to the current week with the days already past.

| Key | Effect |
|---|---|
| {key:left} and {key:right} | One week or one month back and forward |
| {key:t} or {key:home} | Back to today |
| {key:w} and {key:m} | Week or month |

- In the month, every entry is a chip in its module colour, exams with a cap. If not all fit into a day, the last line opens the whole list.
- Lectures and tutorials appear below as a thin line with their time while the day still has room.
- Clicking the day number opens that week. Chips can be dragged to another day.
- Weekends and lecture-free time from the semester are tinted lightly, in week and month.
- The top shows the date range, beside it in small type the lecture week or the break. The calendar week is in the tooltip over the range.

## Add for a Day

Right-click a day in the calendar, the + at the top right of a day in the week, or click free space in the month. For one-off things without a rule:

| Command | Default time |
|---|---|
| {ui:Abgabe eintragen…} | 23:59 |
| {ui:Aufgabe eintragen…} | 18:00 |
| {ui:Prüfung eintragen…} | 10:00 |

The date is already filled in. Type the title, {key:return}. Right-clicking an entry still shows its own commands.
