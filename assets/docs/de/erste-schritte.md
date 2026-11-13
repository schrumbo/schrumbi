# Erste Schritte

Schrumbi plant dein Studium in einem normalen Ordner: Abgaben, Dateien, Notizen und Karten an einem Ort. Alles bleibt eine Datei, die du auch ohne Schrumbi öffnen kannst.

## Ordner wählen

1. {ui:Ordner wählen…} klicken oder einen Ordner ins Fenster ziehen.
2. Schrumbi legt dort `schrumbi.json` an. Sonst ändert sich nichts.
3. Die Übersicht ist sofort da. Ordner, die nach Modulen aussehen, sind schon Module, und die Vorlesungszeit ist geschätzt.

Gefragt wird erst, wenn etwas gebraucht wird: dein Kürzel beim ersten Ablegen einer Abgabe, deren Name es braucht, die Moodle-Adresse, wenn du Abgaben holen willst.

## Einrichten

Oben in der {ui:Übersicht} steht eine Liste, die sich von selbst abhakt.

| Punkt | Fertig, sobald |
|---|---|
| {ui:Module} | mindestens ein Modul da ist |
| {ui:Vorlesungszeit} | du die Daten geändert oder {ui:Stimmt so} geklickt hast |
| {ui:Moodle-Kalender} | eine Kalenderadresse eingetragen ist |
| {ui:Dateinamen} | Dateien nach Muster benannt werden und dein Kürzel feststeht |
| {ui:Notizen schreiben} | du {ui:Formatiert} oder {ui:Text mit Zeilennummern} gewählt hast |
| {ui:Karten} | die erste Karte da ist |
| {ui:Sichern & Abgleichen} | das Abgleichen eingerichtet ist |

- Groß steht nur der nächste Schritt, die übrigen liegen eingeklappt darunter.
- {ui:Überspringen} zählt als erledigt. {ui:Doch einrichten} holt den Punkt zurück. {ui:Alle überspringen} erledigt den Rest auf einmal.
- Die Zeile mit {ui:Einrichten} klappt die Liste zu, das X blendet sie aus.
- Fertig oder ausgeblendet gilt für alle Geräte mit diesem Ordner. Was nur ein Gerät betrifft, etwa der Kalender, kommt dort einzeln und einmal.
- {key:OpenSettings} → {ui:Allgemein} → {ui:Einrichten in der Übersicht} zeigt sie wieder, übersprungene Punkte auch.
- Sind alle Punkte erledigt, sagt die Liste {ui:Alles eingerichtet} und verschwindet.

> [!tip] Alles auf einmal
> {ui:Geführt einrichten…} unten in der Liste oder in {key:CommandPalette} geht Semester, Module und Abgaben nacheinander durch.

## Tipp des Tages

Ab dem zweiten Start erscheint einmal am Tag unten rechts ein Tipp, nur zu Dingen, die du noch nicht benutzt hast.

- {ui:Zeig's mir} öffnet die Stelle und hebt sie hervor. Ein Klick oder {key:escape} schließt das.
- {ui:Nächster Tipp} zeigt einen anderen.
- {ui:Keine Tipps mehr} schaltet sie ab, {key:OpenSettings} → {ui:Allgemein} → {ui:Tipp des Tages} wieder ein.

## Geführt einrichten

Ein paar kurze Schritte, {key:return} geht weiter. {key:escape} oder das X oben rechts schließt wie {ui:Später einrichten}.

| Schritt | Du legst fest |
|---|---|
| {ui:Willkommen} | Ob Schrumbi Abgabeordner anlegt |
| {ui:Semester} | Name, Vorlesungsbeginn, Vorlesungsende |
| {ui:Module} | Welche Ordner Module sind, mit Kürzel und Name |
| {ui:Abgaben} | Moodle-Kalender und eine Abgabe jede Woche, beides freiwillig |

Pausen und Feiertage ergänzt du später in {key:OpenSettings} → {ui:Semester}.

Bei {ui:Module} sind passende Ordner schon angehakt: alles direkt in einem Semesterordner wie `WS25`, und Ordner mit Kurskürzel wie `INF1`. Gibt es beides nicht, sind alle Ordner der obersten Ebene angehakt.

## Die Bereiche

| Bereich | Kürzel | Zeigt |
|---|---|---|
| {ui:Übersicht} | {key:goToday} | Was jetzt dran ist |
| {ui:Semester} | {key:goGrid} | Alle Wochen und Module |
| {ui:Leistungen} | {key:goRecord} | Punkte und Noten |
| {ui:Inbox} | {key:goInbox} | Dateien, die noch einen Platz brauchen |
| {ui:Notizen} | {key:goNotes} | Notizen, Suche, Tags, Aufgaben |
| {ui:Karten} | {key:goCards} | Karteikarten zum Lernen |

- Die Seitenleiste zeigt Symbole mit Namen. {ui:Seitenleiste einklappen} unten links macht sie schmal, dann nennt der Zeiger Bereich und Kürzel. Unter 820 px Fensterbreite bleibt sie schmal.
- Rechtsklick auf einen Bereich außer {ui:Übersicht}: {ui:Aus der Seitenleiste ausblenden}.

## Was im Ordner liegt

| Datei | Inhalt |
|---|---|
| `schrumbi.json` | Semester, Module, Regeln, Noten |
| `.schrumbi-state.json` | Status, den du von Hand gesetzt hast |
| `.schrumbi-cards.json` | Lernstand deiner Karten |
| `.schrumbi-backup/` | Die letzten 12 Sicherungen von `schrumbi.json` und Status |

- Ist der Status beschädigt, holt Schrumbi die letzte Sicherung von selbst zurück.
- Ist `schrumbi.json` beschädigt, zeigt das Fenster {ui:Letzte Sicherung wiederherstellen}.
- Die beschädigte Datei bleibt als `.defekt` daneben liegen.
