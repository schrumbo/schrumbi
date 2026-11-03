# Submissions & Deadlines

Tasks, submissions and exams are entries with a date. They get into the app in three ways.

| Way | Good for |
|---|---|
| {ui:Neuer Eintrag} {key:newEntry} | Single dates |
| {ui:Kalender importieren (.ics)…} | Everything from Moodle at once |
| Rule in the module | A worksheet every week, see "Semesters & Modules" |

## New Entry

{key:newEntry} opens the window from anywhere.

1. Choose a type: {ui:Aufgabe}, {ui:Abgabe} or {ui:Prüfung}.
2. Type a title. A date at the end is recognized.
3. {ui:Hinzufügen}.

Without a time: task 18:00, submission 23:59, exam 10:00.

| You type | Date |
|---|---|
| `Presentation tomorrow 18:00` | Tomorrow, 18:00 |
| `Essay on Friday at 12:30` | Next Friday, 12:30 |
| `Reading in 2 weeks` | In 14 days from today |
| `Exam on 12.11.` | 12 November |

Instead of typing, {ui:Heute}, {ui:Morgen}, {ui:Nächster Mo} and {ui:In 1 Woche} work too.

## From Moodle

1. In Moodle: Calendar → Export → all events → save the `.ics`.
2. In Schrumbi: {key:CommandPalette} → {ui:Kalender importieren (.ics)…}.

Events land in the matching module if its code or name appears in the course. Otherwise you choose a module per course in the window, or {ui:Nicht importieren}.

## Move a Date

- Date from a rule: right-click the entry → {ui:Termin verschieben…}. {ui:Ursprünglicher Termin} restores the old date.
- Your own and imported entries: change the date directly in the details under {ui:Fällig}.

## Calendar

- {ui:Kalender exportieren (.ics)…} writes the current semester's dates and your timetable to a file.
- {key:OpenSettings} → {ui:Allgemein} → {ui:Kalenderdatei aktuell halten} maintains `Schrumbi-Kalender.ics` in your folder. Your calendar app can subscribe to it.
- Under {ui:Kalender abonnieren} you paste the Moodle address. New deadlines then arrive by themselves.

## Clean Up

{key:GoChecks} collects what is wrong. The area only appears when there is something.

| Card | When | Button |
|---|---|---|
| {ui:Dateinamen} | A submission does not match the naming convention | {ui:Alle umbenennen} |
| {ui:Upload bestätigen} | File is ready, deadline passed, upload not confirmed | {ui:Alle bestätigen} |
| {ui:Überfällig} | Date passed, no file or not ticked off | {ui:Abgegeben} or {ui:Entfällt} |
| {ui:Ordnerstruktur} | An expected folder is missing | {ui:Alle anlegen} |
