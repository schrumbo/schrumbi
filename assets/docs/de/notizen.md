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

Rechtsklick auf einen anderen Tab → {ui:Daneben öffnen} zeigt ihn neben der aktuellen Notiz.

## Mehr

| Befehl | Wirkung |
|---|---|
| {ui:Tagesnotiz öffnen} {key:dailyNote} | Notiz für heute, wird bei Bedarf angelegt |
| {ui:Als PDF exportieren} {key:exportPDF} | PDF neben die Notiz |
| {ui:Verlauf} | Ältere Stände vergleichen und wiederherstellen, wenn Sync eingerichtet ist |
| {ui:Als Folien zeigen} | Teile mit `---` werden Folien |
