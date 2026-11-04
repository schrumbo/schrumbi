# Übersicht

Was jetzt dran ist, auf einen Blick. {key:goToday} öffnet sie.

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
| {ui:Neue Notiz} | Neue Notiz, die zur Abgabe gehört |
| {ui:Datei zuordnen…} | Eine vorhandene Datei suchen und dazunehmen |
| {ui:Abgabeordner öffnen} | Den Ordner im Dateimanager zeigen |
| {ui:Automatisch erkennen} | Den von Hand gesetzten Status aufheben |

{key:Undo} nimmt Statusänderungen zurück.

## Welche Dateien zählen

Eine Datei zählt für eine Abgabe, wenn sie dazugehört. Datum und Ordner allein reichen nicht.

| So gehört eine Datei dazu | Wo |
|---|---|
| {ui:Neue Notiz} | Rechts bei der Abgabe |
| Notiz als PDF exportieren | Das PDF gehört zu denselben Abgaben |
| Datei auf die Abgabe ziehen oder ablegen | Übersicht, Abgaben, Inbox |
| {ui:Abgabe zuordnen} | Rechtsklick auf eine Datei in den {ui:Notizen} |
| Pille oben in der Notiz | Zeigt die Abgabe, ein Klick wechselt sie |

- Neue Notizen und abgelegte Dateien landen im {ui:Abgabeordner} des Moduls, z. B. `Analysis/Abgaben`. Den Namen änderst du unter {key:OpenSettings} → {ui:Allgemein}.
- Umbenennen und Verschieben in Schrumbi behalten die Zuordnung. Fehlt eine Datei, weil eine andere App sie verschoben hat, steht sie rechts als {ui:Nicht mehr gefunden}.
- Liegt im Abgabeordner etwas aus der Zeit vor der Frist, fragt Schrumbi rechts {ui:Gehört das dazu?}. Erst {ui:Zuordnen} zählt.
- Hat eine Regel einen {ui:Wochenordner}, zählt alles darin ohne Zuordnen.

| Zugeordnet | Status |
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

## Für einen Tag eintragen

Rechtsklick auf einen Tag unter {ui:Nächste 7 Tage}, oder auf das + rechts oben im Tag. Für Einmaliges ohne Regel:

| Befehl | Standardzeit |
|---|---|
| {ui:Abgabe eintragen…} | 23:59 |
| {ui:Aufgabe eintragen…} | 18:00 |
| {ui:Prüfung eintragen…} | 10:00 |

Das Datum steht schon drin. Titel tippen, {key:return}. Rechtsklick auf einen Eintrag zeigt weiter seine eigenen Befehle.
