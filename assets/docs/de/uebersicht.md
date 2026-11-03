# Übersicht

Was jetzt dran ist, auf einen Blick. {key:GoToday} öffnet sie.

## Jetzt dran

Die große Karte oben zeigt das Dringendste, in dieser Reihenfolge:

1. Überfälliges
2. Fertig, aber Upload nicht bestätigt
3. Die nächste Frist, wenn sie in weniger als 3 Tagen kommt
4. Dateien in der Inbox

| Lage | Knopf |
|---|---|
| Abgabe fällig, Datei liegt bereit | {ui:Hochladen} |
| Abgabe fällig, noch keine Datei | {ui:Datei ablegen…} |
| Upload nicht bestätigt | {ui:Als hochgeladen markieren} |
| Dateien in der Inbox | {ui:Jetzt ablegen} |

Ist nichts offen, steht dort {ui:Nichts offen}. Daneben zeigt {ui:Diese Woche}, wie viel von dieser Woche erledigt ist, und {ui:Pünktlich in Folge} deine Serie pünktlicher Abgaben.

## Status setzen

- Kreis vor dem Eintrag klicken: {ui:Erledigt}, bei Abgaben mit Upload {ui:Hochgeladen}.
- Noch ein Klick gibt den Status an die automatische Erkennung zurück.
- Rechtsklick auf den Eintrag zeigt den Rest:

| Befehl | Wirkung |
|---|---|
| {ui:In Arbeit} | Angefangen, aber nicht fertig |
| {ui:Entfällt} | Zählt nicht mehr mit |
| {ui:Termin verschieben…} | Neuer Termin für einen Eintrag aus einer Regel |
| {ui:Vorhandene Datei zuordnen…} | Eine Datei als Abgabe nehmen |
| {ui:Automatisch erkennen} | Den von Hand gesetzten Status aufheben |

{key:Undo} nimmt Statusänderungen zurück.

## Was Schrumbi selbst erkennt

Bei Abgaben schaut Schrumbi in den Ordner. Es zählen Dateien seit der vorigen Frist.

| Im Ordner | Status |
|---|---|
| Nichts, Termin noch nicht da | {ui:Offen} |
| Nichts, Termin vorbei | {ui:Überfällig}, bei optionalen {ui:Entfällt} |
| Nur Entwürfe: `.md`, `.txt`, `.rtf` | {ui:In Arbeit} |
| Eine fertige Datei | {ui:Erledigt} |
| Richtig benannte Datei, Upload steht aus | {ui:Bereit zum Hochladen} |

- Ob du hochgeladen hast, kann Schrumbi nicht sehen. Das bestätigst du mit dem Kreis.
- Prüfungen gelten nach ihrem Termin als erledigt.
- Ein Status von Hand gilt immer vor dem erkannten.

## Weiter unten

- Gesammelt: {ui:Überfällig} und {ui:Fertig, aber Upload nicht bestätigt}, mit {ui:Alle als hochgeladen markieren}.
- {ui:Nächste 7 Tage}, {ui:Prüfungen} und {ui:Zuletzt geschrieben}.
- Warten Karten, erscheint ein Hinweis mit {ui:Lernen}.
