# Abgaben & Termine

Aufgaben, Abgaben und Prüfungen sind Einträge mit Termin. Sie kommen auf drei Wegen in die App.

| Weg | Gut für |
|---|---|
| {ui:Neuer Eintrag} {key:newEntry} | Einzelne Termine |
| {ui:Kalender importieren (.ics)…} | Alles aus Moodle auf einmal |
| Regel im Modul | Ein Blatt jede Woche, siehe „Semester & Module“ |

## Neuer Eintrag

{key:newEntry} öffnet das Fenster von überall.

1. Art wählen: {ui:Aufgabe}, {ui:Abgabe} oder {ui:Prüfung}.
2. Titel tippen. Ein Datum am Ende wird erkannt.
3. {ui:Erinnern} nur ändern, wenn dieser Eintrag anders erinnern soll: wie in den Einstellungen, keine, 1 oder 3 Stunden oder 1 Tag vorher.
4. {ui:Hinzufügen}.

Ohne Uhrzeit gilt: Aufgabe 18:00, Abgabe 23:59, Prüfung 10:00.

| Du tippst | Termin |
|---|---|
| `Referat morgen 18 Uhr` | Morgen, 18:00 |
| `Essay bis Freitag um 12:30` | Nächster Freitag, 12:30 |
| `Lesen in 2 Wochen` | Heute in 14 Tagen |
| `Klausur am 12.11.` | 12. November |

Statt zu tippen gehen auch {ui:Heute}, {ui:Morgen}, {ui:Nächster Mo} und {ui:In 1 Woche}.

## Aus Moodle

1. In Moodle: Kalender → Exportieren → alle Termine → `.ics` speichern.
2. In Schrumbi: {key:CommandPalette} → {ui:Kalender importieren (.ics)…}

Termine landen beim passenden Modul, wenn Kürzel oder Name im Kurs stehen. Im Fenster wählst du sonst pro Kurs ein Modul oder {ui:Nicht importieren}.

## Termin verschieben

- Termin aus einer Regel: {ui:Termin verschieben…} im Rechtsklick-Menü des Eintrags. {ui:Ursprünglicher Termin} stellt das alte Datum wieder her.
- Eigene und importierte Einträge: Datum direkt in den Details unter {ui:Fällig} ändern.

## Kalender

- {ui:Kalender exportieren (.ics)…} schreibt die Termine des aktuellen Semesters und deinen Stundenplan in eine Datei.
- {key:OpenSettings} → {ui:Allgemein} → {ui:Kalenderdatei aktuell halten} pflegt `Schrumbi-Kalender.ics` in deinem Ordner. Deine Kalender-App kann sie abonnieren.
- Unter {ui:Kalender abonnieren} fügst du die Moodle-Adresse ein. Neue Fristen kommen dann von selbst.

### Apple Kalender (Mac)

{ui:Mit Apple Kalender abgleichen} trägt Abgaben und Stundenplan des laufenden Semesters in den Kalender Schrumbi ein. Über iCloud erscheinen sie auch auf dem iPhone.

| In Schrumbi | Im Kalender |
|---|---|
| Abgabe angelegt oder verschoben | Termin kommt dazu oder wandert mit |
| Abgabe erledigt | ✓ vor dem Titel, keine Erinnerung mehr |
| Abgabe gelöscht, Regel geändert | Termin verschwindet oder ändert sich |
| Vorlesungszeit endet | Stundenplan und vergangene Abgaben gehen, das nächste Semester kommt, offene Prüfungen bleiben |
| Abgleich ausgeschaltet | Schrumbis Termine gehen, der leere Kalender bleibt |

- Erinnerungen im Kalender folgen {ui:Erinnern}.
- Termine, die du selbst anlegst, auch im Kalender Schrumbi, und alle anderen Kalender bleiben unberührt.
- Beim ersten Einschalten fragt macOS nach dem Zugriff. Abgelehnt: {ui:Systemeinstellungen öffnen} führt zu Datenschutz → Kalender.
- Hast du die Kalenderdatei schon abonniert, entferne das Abo, sonst steht alles doppelt da.
