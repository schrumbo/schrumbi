# Semester & Module

Ein Semester hat Vorlesungswochen, ein Modul hat einen Ordner. Mehr braucht Schrumbi nicht.

## Semester

{key:goGrid} zeigt das ganze Semester.

- Oben: Fortschritt und ein Band aus Wochen. Klick auf eine Vorlesungswoche springt hin.
- {ui:Module}: jedes Modul mit nächster Frist. Klick öffnet die Modulseite.
- {ui:Wochen}: {ui:Diese Woche} ist offen, ältere liegen unter {ui:Vergangene Wochen}.

## Modulseite

Klick auf ein Modul in der Seitenleiste öffnet seine Seite. Links stehen alle {ui:Abgaben} mit ihren Dateien, {ui:Zuletzt geschrieben} und {ui:Dateien}, rechts Karten, Termine, Note und Links.

- Was noch offen ist, steht oben unter {ui:Noch offen}. Die graue Zahl neben {ui:Abgaben} zählt genau diese Einträge, wie in der Seitenleiste.
- Danach kommen die nächsten zwei Wochen. Spätere und erledigte Abgaben sind eingeklappt und zeigen die ersten fünf.
- Erledigtes gleitet aus der Liste, {ui:Erledigt} leuchtet kurz auf.
- {ui:Neuer Eintrag…} neben {ui:Abgaben} legt eine Abgabe an.
- {ui:Dateien} zeigt die zuletzt geänderten Dateien des Modulordners, die keine Notizen sind, z. B. PDFs. Ein Klick öffnet sie. Ohne solche Dateien fehlt der Abschnitt.
- {ui:Alle Dateien} zeigt den Modulordner im Dateibaum.
- Das Suchfeld oben durchsucht auf der Modulseite nur dieses Modul. Ein Klick auf das Kürzel daneben sucht überall.

| Taste | Wirkung |
|---|---|
| {key:n} | {ui:Neue Notiz} im Modul |
| {key:f} | {ui:Ablegen} für die nächste offene Abgabe |
| {key:up} {key:down} | Durch die Einträge gehen |
| {key:space} | Gewählten Eintrag erledigen, die Auswahl rückt zum nächsten |
| {key:escape} | Im Suchfeld: erst leeren, dann zurück zur Seite |

## Modul anlegen

- In den {ui:Notizen} Rechtsklick auf einen Ordner: {ui:Als Modul verwenden}.
- Oder {key:OpenSettings} → {ui:Module}. Dort stehen {ui:Kürzel}, {ui:Name}, {ui:Semester} und {ui:Ordner}.

Schrumbi verschiebt dabei nichts.

## Wiederkehrende Abgaben

Ein Übungsblatt jede Woche? Eine Regel statt 14 Einträgen:

1. {key:OpenSettings} → {ui:Module} → Modul wählen.
2. {ui:Regel hinzufügen} und eine Vorlage wählen.
3. Titel, Tag und Uhrzeit anpassen.

Dateien gehören zur Abgabe der Woche, sobald du sie ablegst, dafür eine Notiz anlegst oder sie zuordnest. Hat jede Woche einen eigenen Ordner, trägst du ihn unter {ui:Erweitert} → {ui:Wochenordner} ein. Was darin liegt, zählt dann ohne Zuordnen. Dort stehen Platzhalter:

| Platzhalter | Wird zu |
|---|---|
| `{w}` | Vorlesungswoche: `01`, `02`, … |
| `{kw}` | Kalenderwoche: `44` |
| `{date}` | Datum als `yyMMdd` |

Aus `W{w}_KW{kw}` wird `W03_KW44`. Fristen in einer Pause fallen weg, Fristen an Feiertagen rutschen auf den nächsten Tag.

## Leistungen

{key:goRecord} zeigt Punkte und Noten. {ui:Leistungen} erscheint in der Seitenleiste, sobald ein Modul Punkte oder eine Note hat; die Tastenkürzel und die Befehlspalette öffnen die Seite auch vorher.

- Eine {ui:Note} setzt {ui:Bestanden} passend zum Notensystem.
- Klick auf einen Modulnamen öffnet die Teilleistungen, etwa Klausur 60 % und Projekt 40 %.
- Ab zwei Teilleistungen gibt es eine {ui:Zielnote}. Schrumbi zeigt, welchen Schnitt die offenen Teile noch brauchen.
