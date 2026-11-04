# Erste Schritte

Schrumbi plant dein Studium in einem normalen Ordner: Deadlines, Dateien, Notizen und Karten an einem Ort. Alles bleibt eine Datei, die du auch ohne Schrumbi öffnen kannst.

## Ordner wählen

1. {ui:Ordner wählen…} klicken oder einen Ordner ins Fenster ziehen.
2. Schrumbi legt dort `schrumbi.json` an. Sonst ändert sich nichts.
3. Die Einrichtung startet. {ui:Später einrichten} überspringt sie.

> [!tip] Später noch einmal
> {key:CommandPalette} öffnen und {ui:Einrichtung erneut starten} wählen.

## Einrichtung

Vier kurze Schritte, {key:return} geht weiter.

| Schritt | Du legst fest |
|---|---|
| {ui:Willkommen} | Studiengang, Namensmuster und ob Schrumbi Abgabeordner anlegt |
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

- Klick auf den offenen Bereich oder {key:ToggleSidebar} blendet die Beschriftungen ein und aus.
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
