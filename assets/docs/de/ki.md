# KI

Schrumbi arbeitet mit Claude Code, Codex oder OpenCode zusammen, wenn eins davon installiert ist. Das Programm nutzt dein eigenes Konto dort; was du ihm zeigst, geht an diesen Anbieter.

## Ein und aus

Ist ein Programm installiert, ist die KI an. {ui:KI-Assistent verwenden} in {key:OpenSettings} → {ui:KI} schaltet sie aus, {ui:Knopf in der Werkzeugleiste} blendet den Knopf aus.

## Die Spalte

{key:toggleAi} oder der Knopf {ui:KI einblenden} öffnet rechts die Spalte {ui:KI}. Sie teilt sich den Platz mit den Details: oben wechselst du zwischen {ui:Details} und {ui:KI}, {key:toggleInspector} springt zu den Details. Derselbe Knopf noch einmal schließt die Spalte. Ihren linken Rand ziehst du breiter oder schmaler, ein Doppelklick setzt die Breite zurück.

1. Öffne eine Notiz oder ein PDF.
2. {key:toggleAi}
3. Klick auf {ui:Karten aus dieser Seite} oder schreib selbst und drück {key:return}.

Unten stehen Kärtchen mit dem, was die KI mitbekommt: offene Notiz, PDF-Seite, Auswahl, Modul. Das × nimmt eins raus, {ui:Kontext} holt es oder einen anderen Tab dazu.

| Knopf | Was passiert |
|---|---|
| {ui:Folgen} | Geänderte Dateien öffnen sich und zeigen die Stelle |
| {ui:Ansehen} | Vorher und nachher nebeneinander |
| {ui:Rückgängig} | Nur diese eine Änderung zurück |
| {ui:Stopp} | Bricht den Auftrag ab |
| {ui:Als Notiz speichern} | Die Unterhaltung als Notiz in deinem Ordner |
| {ui:Frühere Unterhaltungen} | Eine alte Unterhaltung öffnen und weiterschreiben |
| {ui:Verwerfen} | Im Vergleich: nur diese Änderung zurück |

Rechtsklick auf eine Datei, eine PDF-Seite, eine Abgabe oder markierten Text zeigt {ui:KI} mit passenden Aufträgen.

## Aufträge

| Auftrag | Was passiert |
|---|---|
| {ui:Karten erstellen} | Karten aus einer PDF-Seite, Seiten, Auswahl oder Notiz; sie landen als Frage :: Antwort in der Notiz zum PDF oder in {ui:Karteikarten} des Moduls |
| {ui:Passage erklären} | Erklärt die Stelle, ändert nichts; {ui:In Notiz einfügen} hängt die Erklärung an |
| {ui:Vorlesung zusammenfassen} | Eine neue Notiz im Modulordner, benannt wie die anderen dort |
| {ui:Woche planen} | Plan aus Fristen, fälligen Karten und Vorlesungswoche; Aufgaben erst nach deinem Ja |
| {ui:Inbox aufräumen} | Liest jede Datei und zeigt eine Tabelle: Modul, Art, Datum, Frist, neuer Name |
| {ui:Abfragen} | Fragt dich ab, schreibt nichts und bewertet keine Karte |

Im Ablegen-Fenster füllt {ui:Mit KI ausfüllen} die Felder aus der Datei aus. Was die KI eingetragen hat, ist markiert, bis du es änderst; abgelegt wird erst mit {ui:Ablegen}.

## Mit KI einrichten

{ui:Mit KI einrichten} in der leeren Spalte oder in der Befehlssuche. Liegen schon Unterlagen im Ordner, liest die KI sie und leitet Module, Vorlesungszeit, Namensmuster und wöchentliche Abgaben ab; sonst fragt sie kurz nach. Ihr Vorschlag erscheint als Karte: Haken setzen, {ui:Alles übernehmen}, ein {key:Undo} nimmt alles zurück. Die KI ändert nur Einstellungen; Dateien umzubenennen schlägt sie höchstens in einer Liste vor. Nach {ui:Sitzung beenden nach} ohne Nachricht endet das Programm; die nächste Nachricht startet es neu. Unterhaltungen liegen in den App-Daten, nicht in deinem Ordner.

## Im Terminal

1. {key:OpenSettings} → {ui:KI} → {ui:Für die Kommandozeile}
2. {ui:Eintragen} neben deinem Programm.
3. Starte es im Terminal, etwa über {ui:Im Terminal öffnen}.

Solange Schrumbi läuft, liest es deine Notizen, PDFs, Fristen und Karten. Frag zum Beispiel: Welche Abgaben habe ich in den nächsten 7 Tagen?

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
| {ui:Immer heute} | Diese Art Änderung bis Mitternacht ohne Nachfrage |
| {ui:Ablehnen} | Nichts wird geschrieben |

Die Änderungen eines Auftrags sind ein Schritt: {key:Undo} nimmt sie zusammen zurück. Hast du die Notiz inzwischen geändert, schreibt die KI nichts und liest sie neu.

## Was die KI nie tut

Sie löscht nichts, benennt nichts um und bewertet keine Karten. Codex läuft nur in seinem Nur-Lesen-Modus, OpenCode ohne eigene Dateiänderungen; beide ändern nur über Schrumbi. Ungesicherten Text überschreibt sie nicht. Befehle startet sie nur, wenn du {ui:Befehle ausführen erlauben} einschaltest.
