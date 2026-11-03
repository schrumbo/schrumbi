# Semester & Module

Ein Semester hat Vorlesungswochen, ein Modul hat einen Ordner. Mehr braucht Schrumbi nicht.

## Semester

{key:GoGrid} zeigt das ganze Semester.

- Oben: Fortschritt und ein Band aus Wochen. Klick auf eine Vorlesungswoche springt hin.
- {ui:Module}: jedes Modul mit nächster Frist. Klick öffnet die Modulseite.
- {ui:Wochen}: {ui:Diese Woche} ist offen, ältere liegen unter {ui:Vergangene Wochen}.

## Modulseite

Klick auf ein Modul in der Seitenleiste öffnet seine Seite.

| Taste | Wirkung |
|---|---|
| {key:1} {key:2} {key:3} | Reiter {ui:Übersicht}, {ui:Abgaben}, {ui:Dateien} |
| {key:n} | {ui:Neue Notiz} im Modul |
| {key:f} | {ui:Ablegen} für die nächste offene Abgabe |
| {key:up} {key:down} | Durch die Einträge gehen |

## Modul anlegen

- In den {ui:Notizen} Rechtsklick auf einen Ordner: {ui:Als Modul verwenden}.
- Oder {key:OpenSettings} → {ui:Module}. Dort stehen {ui:Kürzel}, {ui:Name}, {ui:Semester} und {ui:Ordner}.

Schrumbi verschiebt dabei nichts.

## Wiederkehrende Abgaben

Ein Übungsblatt jede Woche? Eine Regel statt 14 Einträgen:

1. {key:OpenSettings} → {ui:Module} → Modul wählen.
2. {ui:Regel hinzufügen} und eine Vorlage wählen.
3. Titel, Tag und Uhrzeit anpassen.

Im Ordner der Regel stehen Platzhalter:

| Platzhalter | Wird zu |
|---|---|
| `{w}` | Vorlesungswoche: `01`, `02`, … |
| `{kw}` | Kalenderwoche: `44` |
| `{date}` | Datum als `yyMMdd` |

Aus `W{w}_KW{kw}` wird `W03_KW44`. Fristen in einer Pause fallen weg, Fristen an Feiertagen rutschen auf den nächsten Tag.

## Leistungen

{key:GoRecord} zeigt Punkte und Noten.

- Eine {ui:Note} setzt {ui:Bestanden} passend zum Notensystem.
- Klick auf einen Modulnamen öffnet die Teilleistungen, etwa Klausur 60 % und Projekt 40 %.
- Ab zwei Teilleistungen gibt es eine {ui:Zielnote}. Schrumbi zeigt, welchen Schnitt die offenen Teile noch brauchen.
