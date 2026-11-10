# KI

Schrumbi arbeitet mit Claude Code, Codex oder OpenCode zusammen, wenn eins davon installiert ist. Das Programm nutzt dein eigenes Konto dort; was du ihm zeigst, geht an diesen Anbieter.

## Ein und aus

Ist ein Programm installiert, ist die KI an. {ui:KI-Assistent verwenden} in {key:OpenSettings} → {ui:KI} schaltet sie aus, {ui:Knopf in der Werkzeugleiste} blendet den Knopf aus.

## Die Spalte

{key:toggleAi} oder der Knopf {ui:KI einblenden} öffnet rechts die Spalte {ui:KI}. Sie teilt sich den Platz mit den Details; {key:toggleInspector} holt die Details zurück.

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

Rechtsklick auf eine Datei, eine Abgabe oder markierten Text zeigt {ui:KI} mit passenden Aufträgen. Nach {ui:Sitzung beenden nach} ohne Nachricht endet das Programm; die nächste Nachricht startet es neu. Unterhaltungen liegen in den App-Daten, nicht in deinem Ordner.

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
