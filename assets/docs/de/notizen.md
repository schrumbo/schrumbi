# Notizen

Notizen sind Markdown-Dateien in deinem Ordner. Obsidian liest sie genauso.

## Neue Notiz

1. {key:newFile} oder {ui:Neue Notiz}. Schrumbi legt `Unbenannt.md` an.
2. Namen tippen, {key:return}.

{key:escape} lässt den Namen `Unbenannt.md`. Auf einer Modulseite öffnet {key:newFile} ein Fenster, in dem du Art und Ort wählst.

## Dateien

| Taste | Wirkung |
|---|---|
| {key:up} {key:down} | Datei wählen, mit Vorschau |
| {key:right} {key:left} | Ordner auf- und zuklappen |
| {key:return} | In die Notiz springen |
| {key:f2} | Umbenennen |
| {key:/} | Filtern |

- Einfacher Klick öffnet eine Vorschau, Doppelklick behält den Tab.
- Datei auf einen Ordner ziehen verschiebt sie.
- Rechtsklick → {ui:Abgabe zuordnen} zählt die Datei für eine Abgabe. Die Pille oben in der Notiz zeigt, wozu sie gehört.
- {ui:In den Papierkorb} fragt nicht nach. {key:Undo} holt die Datei zurück.

Oben wechselst du zwischen {ui:Dateien}, {ui:Suche}, {ui:Tags} und {ui:Aufgaben}.

## Office-Dateien

Word, Excel und PowerPoint öffnen in Schrumbi, nur zum Lesen.

| Datei | Ansicht |
|---|---|
| `.docx` | Seiten wie im PDF: Zoom, Suche, Text kopieren |
| `.pptx` | Eine Folie unter der anderen, mit Foliennummer, Zoom und Suche |
| `.xlsx` `.xlsm` `.xls` `.ods` | Tabelle mit Blättern unten |

In einer Tabelle:

| Taste | Wirkung |
|---|---|
| {key:up} {key:down} {key:left} {key:right} | Zelle wählen |
| {key:shift-down} | Auswahl erweitern |
| {key:cmd-down} | Zum Ende der Spalte |
| {key:cmd-c} | Auswahl kopieren, mit Tabs zwischen den Zellen |
| {key:cmd-f} | In diesem Blatt suchen, {key:return} zum nächsten Treffer |
| {key:cmd-pageup} {key:cmd-pagedown} | Voriges und nächstes Blatt |
| {key:escape} | Ziehen abbrechen, die alte Auswahl bleibt |

Bearbeiten geht in der Standard-App: ⋯ → {ui:Mit Standard-App öffnen}. Sichert sie, zeigt Schrumbi die neue Version. Mit Passwort geschützte Dateien öffnet nur die Standard-App. Lässt sich ein Blatt nicht lesen, bleiben die anderen Blätter offen.

## Lesen und Bearbeiten

| Modus | Kürzel |
|---|---|
| {ui:Lesen} / {ui:Bearbeiten} | {key:togglePreview} |
| {ui:Geteilte Ansicht}, nur im Code-Editor | {key:splitPreview} |

Welcher Editor, legst du in {key:OpenSettings} → {ui:Notizen} fest: {ui:Live: Text bleibt gezeichnet} oder {ui:Code-Editor: Quelltext mit Zeilennummern}.

## Sichern

Schrumbi sichert nach 2 Sekunden Tipp-Pause, einstellbar unter {ui:Notizen} → {ui:Sichern}. {key:Save} sichert sofort, Schließen sichert immer.

Hat eine andere App die Datei geändert, siehst du einen Hinweis. Nichts geht verloren, bis du wählst:

| Knopf | Ergebnis |
|---|---|
| {ui:Vergleichen} | Beide Versionen nebeneinander |
| {ui:Neu laden} | Die Version der anderen App |
| {ui:Meine behalten} | Deine Version |

## Verknüpfen

- `[[` tippen und eine Notiz wählen.
- {key:toggleInspector} zeigt {ui:Rückverweise}, {ui:Ausgehend} und {ui:Erwähnungen ohne Link}.
- {key:ShowGraph} zeigt alle Notizen als Graph.

## Tabs

| Kürzel | Wirkung |
|---|---|
| {key:closeTab} | Tab schließen |
| {key:ReopenTab} | Zuletzt geschlossenen Tab öffnen |
| {key:nextTab} / {key:previousTab} | Nächster / vorheriger Tab |

- Ein Tab geht nur zu, wenn du ihn schließt: ×, Mittelklick oder {key:closeTab}.
- Ein Klick auf eine Datei öffnet sie als Vorschau mit schrägem Titel. Die nächste Datei ersetzt sie.
- Doppelklick, Schreiben oder Rechtsklick → {ui:Tab behalten} macht aus der Vorschau einen festen Tab.
- Ein Link öffnet sein Ziel als Vorschau, die Notiz mit dem Link bleibt offen.
- Tab ziehen ändert die Reihenfolge. {key:ReopenTab} bringt den Tab an seinen alten Platz zurück.

### Nebeneinander

- Rechtsklick auf einen Tab → {ui:Daneben öffnen}, oder den Tab in die rechte Hälfte ziehen.
- Der Tab der rechten Notiz zeigt als Symbol ein Fenster mit gefüllter rechter Hälfte. Ein Klick darauf springt in die rechte Notiz.
- Eine farbige Linie unter der Leiste zeigt, in welcher Notiz du gerade bist.
- Links in der rechten Notiz öffnen rechts.
- {key:closeTab} schließt die Notiz, in der du gerade bist. Ist links nichts mehr offen, rückt die rechte Notiz nach links.
- × rechts oben ({ui:Geteilte Ansicht schließen}) beendet das Nebeneinander, beide Tabs bleiben offen.

## Mehr

| Befehl | Wirkung |
|---|---|
| {ui:Tagesnotiz öffnen} {key:dailyNote} | Notiz für heute, wird bei Bedarf angelegt |
| {ui:Als PDF exportieren} {key:exportPDF} | PDF neben die Notiz |
| {ui:Verlauf} | Ältere Stände vergleichen und wiederherstellen, wenn Sync eingerichtet ist |
| {ui:Als Folien zeigen} | Teile mit `---` werden Folien |

Liegt neben der Notiz schon ein früherer Export, fragt Schrumbi vor dem Ersetzen. {ui:Rückgängig} in der Meldung danach holt das alte PDF zurück.

- {ui:Nicht mehr fragen} ersetzt künftig ohne Frage.
- Die Frage schaltest du wieder ein unter {key:OpenSettings} → {ui:Notizen} → {ui:Vor dem Ersetzen eines früheren Exports fragen}.
- Eine andere Datei mit diesem Namen bleibt unangetastet, das PDF bekommt eine Nummer. Der nächste Export ersetzt dieses nummerierte PDF.
