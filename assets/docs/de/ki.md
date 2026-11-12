# KI

Schrumbi arbeitet mit Claude Code, Codex oder OpenCode zusammen, wenn eins davon installiert ist. Das Programm nutzt dein eigenes Konto dort; was du ihm zeigst, geht an diesen Anbieter.

## Ein und aus

Ist ein Programm installiert, ist die KI an. {ui:KI-Assistent verwenden} in {key:OpenSettings} → {ui:KI} schaltet sie aus, {ui:Knopf in der Werkzeugleiste} unter {ui:Für Fortgeschrittene} blendet den Knopf aus.

Noch keins installiert? Du brauchst ein Konto bei Claude oder ChatGPT.

1. Befehlssuche → {ui:KI-Assistent einrichten}
2. {ui:Claude Code installieren} öffnet die offizielle Anleitung.
3. Danach {ui:Erneut suchen}. Ist das Programm nicht angemeldet, steht dort {ui:Anmeldung prüfen}.

Der erste Start kann eine Minute dauern; {ui:Abbrechen} hält ihn an, {ui:Noch einmal} versucht es neu.

## Die Spalte

{key:toggleAi} oder der Knopf {ui:KI einblenden} öffnet rechts die Spalte {ui:KI}. Sie teilt sich den Platz mit den Details: oben wechselst du zwischen {ui:Details} und {ui:KI}, {key:toggleInspector} springt zu den Details. Derselbe Knopf noch einmal schließt die Spalte. Ihren linken Rand ziehst du breiter oder schmaler, ein Doppelklick setzt die Breite zurück. In einem schmalen Fenster liegt die Spalte über der Seite; ein Klick auf die Seite oder {key:escape} schließt die KI, {key:escape} auch die Details.

1. Öffne eine Notiz oder ein PDF.
2. {key:toggleAi}
3. Klick auf {ui:Karten aus dieser Seite} oder schreib selbst und drück {key:return}.

Unten stehen Kärtchen mit dem, was die KI mitbekommt: offene Notiz, PDF-Seite, Auswahl, Modul. Das × nimmt eins raus, {ui:Kontext} holt es oder einen anderen Tab dazu.

| Knopf | Was passiert |
|---|---|
| {ui:Folgen} | Geänderte Dateien öffnen sich und zeigen die Stelle; aus, bis du es einschaltest |
| {ui:Ansehen} | Vorher und nachher untereinander |
| {ui:Rückgängig} | Nur diese eine Änderung zurück |
| {ui:Stopp} | Bricht den Auftrag ab |
| {ui:Als Notiz speichern} | Die Unterhaltung als Notiz in deinem Ordner |
| {ui:Frühere Unterhaltungen} | Eine alte Unterhaltung öffnen und weiterschreiben |
| {ui:Verwerfen} | Im Vergleich: nur diese Stelle zurück, {ui:Übernehmen} schreibt den Rest |

Rechtsklick auf eine Datei, eine PDF-Seite, eine Abgabe oder markierten Text zeigt {ui:KI} mit passenden Aufträgen.

## Aufträge

| Auftrag | Was passiert |
|---|---|
| {ui:Karten erstellen} | Karten aus einer PDF-Seite, Seiten, Auswahl oder Notiz; sie landen als Frage :: Antwort in der Notiz zum PDF oder in {ui:Karteikarten} des Moduls |
| {ui:Passage erklären} | Erklärt die Stelle, ändert nichts; {ui:In Notiz einfügen} hängt die Erklärung an |
| {ui:Vorlesung zusammenfassen} | Eine neue Notiz im Modulordner, benannt wie die anderen dort |
| {ui:Woche planen} | Plan aus Fristen, fälligen Karten und Vorlesungswoche; Aufgaben erst nach deinem Ja |
| {ui:Inbox aufräumen} | Liest jede Datei und zeigt eine Tabelle: Modul, Art, Datum, Frist, neuer Name; {ui:Ändern} ändert eine Zeile direkt dort |
| {ui:Abfragen} | Fragt dich ab, schreibt nichts und bewertet keine Karte |

Im Ablegen-Fenster füllt {ui:Mit KI ausfüllen} die Felder aus der Datei aus. Was die KI eingetragen hat, ist markiert, bis du es änderst; abgelegt wird erst mit {ui:Ablegen}.

## Mit KI einrichten

{ui:Mit KI einrichten} in der leeren Spalte oder in der Befehlssuche. Liegen schon Unterlagen im Ordner, liest die KI sie und leitet Module, Vorlesungszeit, Namensmuster und wöchentliche Abgaben ab; sonst fragt sie kurz nach. Ihr Vorschlag erscheint als Karte: Haken setzen, {ui:Alles übernehmen}, ein {key:Undo} nimmt alles zurück. Die KI ändert nur Einstellungen; Dateien umzubenennen schlägt sie höchstens in einer Liste vor. Nach {ui:Sitzung beenden nach} ohne Nachricht endet das Programm; die nächste Nachricht startet es neu. Unterhaltungen liegen in den App-Daten, nicht in deinem Ordner. Beendest du Schrumbi, endet auch das Programm.

## Im Terminal

1. {key:OpenSettings} → {ui:KI} → {ui:Für Fortgeschrittene} → {ui:Für die Kommandozeile}
2. {ui:Eintragen} neben deinem Programm.
3. Starte es in deinem Studienordner, etwa über {ui:Im Terminal öffnen}.

Solange Schrumbi diesen Ordner offen hat, liest es deine Notizen, PDFs, Fristen und Karten. In einem anderen Ordner gestartet, bekommt es nur die Antwort, wo es starten soll. Frag zum Beispiel: Welche Abgaben habe ich in den nächsten 7 Tagen?

Die Abläufe der Spalte gibt es dort als Befehle:

````example
/mcp__schrumbi__karten MA/VL3.pdf 12
/mcp__schrumbi__woche
/mcp__schrumbi__einrichten
````

## Änderungen

Vor jeder Änderung fragt Schrumbi in der Spalte, bei geschlossener Spalte unten rechts im Fenster.

| Knopf | Was passiert |
|---|---|
| {ui:Erlauben} | Diese eine Änderung |
| {ui:In dieser Notiz heute erlauben} | Dieselbe Art Änderung in dieser Notiz bis Mitternacht ohne Nachfrage; bei neuen Notizen {ui:In diesem Ordner heute erlauben} |
| {ui:Ablehnen} | Nichts wird geschrieben |
| {ui:Ansehen} | Die ganze Änderung, bevor du entscheidest |

Lange Änderungen zeigen die ersten Zeilen, jede Zeile mit einem Link und wie viele Zeilen fehlen. Ablegen, Inbox, Einrichten, Status und Fristen fragen immer. Mehrere gleiche Änderungen kommen als eine Karte: {ui:Alle erlauben}, {ui:Alle ablehnen} oder {ui:Einzeln ansehen}.

Die Änderungen eines Auftrags in der Spalte sind ein Schritt: {key:Undo} nimmt sie zusammen zurück. Aus dem Terminal ist jede Änderung ein eigener Schritt. Hast du die Notiz inzwischen geändert, schreibt die KI nichts und liest sie neu.

## Was die KI nie tut

Sie löscht nichts, benennt nichts um und bewertet keine Karten. Codex läuft nur in seinem Nur-Lesen-Modus, OpenCode ohne eigene Dateiänderungen; beide ändern nur über Schrumbi. Ungesicherten Text überschreibt sie nicht. Versteckte Dateien und Ordner, deren Name mit einem Punkt beginnt, und Schrumbis eigene Dateien liest und ändert sie nicht. Anweisungen, die in deinen Notizen oder PDFs stehen, befolgt sie nicht, sondern sagt dir davon. Befehle startet sie nur, wenn du {ui:Befehle ausführen erlauben} einschaltest.
