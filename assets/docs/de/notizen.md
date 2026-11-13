# Notizen

Notizen sind Markdown-Dateien in deinem Ordner. Obsidian liest sie genauso.

## Neue Notiz

1. {key:newFile} oder {ui:Neue Notiz}. Schrumbi legt `Unbenannt.md` an.
2. Namen tippen, {key:return}.

Ist in Obsidian ein fester Ordner für neue Notizen eingestellt, landet sie dort. {key:escape} lässt den Namen `Unbenannt.md`. Auf einer Modulseite öffnet {key:newFile} ein Fenster, in dem du Art und Ort wählst.

{ui:Neue Leinwand} und {ui:Neue Tabelle (Base)} stehen im Menü {ui:Mehr} über der Dateiliste und in der Befehlspalette. Beide Dateien öffnet auch Obsidian.

## Dateien

| Taste | Wirkung |
|---|---|
| {key:up} {key:down} | Datei wählen |
| {key:right} {key:left} | Ordner auf- und zuklappen |
| {key:return} | Öffnen und in die Notiz springen |
| {key:renameNote} | Umbenennen |
| {key:/} | Filtern |

- {key:renameNote} benennt auch die offene Notiz um, ebenso ein Klick auf ihren Titel oben.
- Ein Klick öffnet die Datei in einem eigenen Tab. Ist sie schon offen, kommt ihr Tab nach vorn.
- Datei auf einen Ordner ziehen verschiebt sie.
- {key:escape} leert zuerst das Filterfeld, ein zweites {key:escape} verlässt es.
- Unter 800 Pixel Fensterbreite klappt die Dateiliste zu, damit die Notiz Platz hat. Der Knopf links in der Leiste holt sie zurück, bis du die App schließt.
- Fehlt der Studienordner beim Sichern, sagt die Meldung das: er wurde verschoben oder gelöscht. {ui:Ordner neu anlegen} legt ihn an derselben Stelle neu an, {ui:Anderen Ordner wählen} öffnet einen anderen. Fehlt nur der Ordner einer offenen Notiz, legt {ui:Ordner neu anlegen} in ihrer Meldung ihn wieder an und sichert den Text.
- Rechtsklick → {ui:Abgabe zuordnen} zählt die Datei für eine Abgabe. Die Pille oben in der Notiz zeigt, wozu sie gehört.
- {ui:In den Papierkorb} fragt nicht nach. {key:Undo} holt die Datei zurück.
- Sehr große Dateien liegen danach nur im Papierkorb des Systems und kommen von dort zurück. Hat das Laufwerk keinen Papierkorb, bleiben sie, wo sie sind.
- Eine Notiz, deren letzte Änderungen nicht gesichert werden konnten, bleibt liegen.
- Umbenennen und Verschieben passt Links in anderen Notizen an, auch in einen umbenannten Ordner hinein.
- Eine Notiz, die nicht in UTF-8 gespeichert ist, bleibt unverändert. {ui:UTF-8-Kopie öffnen} legt eine bearbeitbare Kopie daneben.

Oben wechselst du zwischen {ui:Dateien}, {ui:Suche}, {ui:Tags} und {ui:Aufgaben}.

## Office-Dateien

Word, Excel und PowerPoint öffnen in Schrumbi, nur zum Lesen.

| Datei | Ansicht |
|---|---|
| `.docx` | Seiten wie im PDF, Hoch- und Querformat je Abschnitt: Zoom, Suche, Text kopieren |
| `.pptx` | Eine Folie unter der anderen, mit Foliennummer, Zoom und Suche |
| `.xlsx` `.xlsm` `.xls` `.ods` | Tabelle mit Blättern unten |
| `.csv` `.tsv` | Eine Tabelle; {ui:Als Text zeigen} im Menü der Datei zeigt den Text, die Datei bleibt unverändert |

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

Audio (`.mp3` `.m4a` `.wav` `.aiff`) öffnet mit Abspielknopf und Dauer.

Bearbeiten geht in der Standard-App: ⋯ → {ui:Mit Standard-App öffnen}. Sichert sie, zeigt Schrumbi die neue Version. Mit Passwort geschützte Dateien öffnet nur die Standard-App. Lässt sich ein Blatt nicht lesen, bleiben die anderen Blätter offen.

## PDFs

Markieren und Seiten verschieben, in der Datei selbst und nur bei PDFs.

| Kürzel | Wirkung |
|---|---|
| Text wählen, {key:markText} | Markieren in der zuletzt gewählten Farbe |
| Farbe in der Leiste | Färbt die Markierung, auch eine vorhandene |
| Markierung anklicken, {key:delete} | Entfernen. {ui:Markierung löschen} geht auch mit der Maus |
| {key:Undo} | Nimmt Markieren und Entfernen zurück |

Dieselbe Stelle noch einmal markieren färbt sie um, statt sie zu doppeln.

### Zeichnen

| Kürzel | Wirkung |
|---|---|
| {key:penTool} | {ui:Stift} nehmen oder weglegen |
| {key:eraserTool} | {ui:Radierer} nehmen oder weglegen |
| Ziehen auf der Seite | Zeichnen, mit dem Radierer einen Strich berühren löscht ihn |
| {key:escape} | Werkzeug weglegen |

- Mit dem Stift in der Hand erscheinen Farbe und Dicke ({ui:Dünn} oder {ui:Dick}) in einer Leiste unten auf der Seite.
- Jeder Strich und jeder Zug mit dem Radierer ist ein Schritt für {key:Undo}, ohne Meldung dazu.
- Während die PDF lädt oder speichert und wenn der Ordner nicht beschreibbar ist, sind die Werkzeuge grau. Der Tooltip nennt den Grund.
- Der Radierer löscht nur Zeichnungen, nie Markierungen oder Notizen.
- Eine Zeichnung anklicken, {key:delete} entfernt sie.
- Ein Strich, der die Seite verlässt, endet am Rand. Dünne Striche und Punkte sind mindestens zwei Pixel breit.
- Farben, Dicke und {ui:Auswahl} stehen auch in der Befehlspalette, das Suchwort zeichnen findet sie. Ein Kürzel dafür legst du in den Einstellungen fest, ab Werk haben sie keins.

### Notizen

| Kürzel | Wirkung |
|---|---|
| {key:noteTool}, auf die Seite klicken | {ui:Notiz setzen}: Notiz an dieser Stelle |
| {key:noteTool}, dann {key:return} | Notiz in der Mitte der sichtbaren Seite |
| Text wählen, {key:noteOnSelection} oder {ui:Notiz hinzufügen} in der Leiste | Markierung und Notiz in einem Schritt |
| Markierung anklicken, {ui:Notiz hinzufügen} in der Leiste | Notiz an der Markierung |
| Notiz anklicken | Bearbeiten |
| {ui:Speichern} oder {key:cmd-return} | Speichern. Ein Klick daneben speichert auch |
| {key:escape} | Verwerfen |

- Eine leere Notiz setzt nichts. Leerst du eine vorhandene Notiz, ist sie gelöscht, die Markierung bleibt. {ui:Notiz löschen} gibt es nur bei gespeicherten Notizen.
- Beim neuen Pin zeigt ein blasser Pin die Stelle, solange die Notiz offen ist. Unter dem Feld steht, wie du speicherst oder verwirfst.
- Eine Notiz, die noch offen ist, wird gespeichert: beim Schließen der PDF, vor einer Seitenänderung, beim Ordnerwechsel und beim Beenden.
- Eine Notiz an Text steht als kleines Symbol links neben der ersten Zeile, nie auf dem Text.
- Alles steht auch im Reiter {ui:Marker}, ein Klick springt dorthin. {key:up} und {key:down} wechseln die Zeile, {key:return} springt, {key:delete} entfernt. Ohne Maus: {ui:Markierungen anzeigen} im Befehlsfeld oder {key:markList}.

### Mit Markierungen exportieren

| Kürzel | Wirkung |
|---|---|
| {key:exportMarkedPdf} | {ui:Mit Markierungen exportieren} |

- Schreibt eine Kopie neben die PDF. Markierungen, Notizen und Zeichnungen sind echte Anmerkungen, die auch andere Programme zeigen.
- Die Original-PDF bleibt, wie sie ist. {key:Undo} entfernt die Kopie wieder.
- Ohne Markierungen, bei schreibgeschütztem Ordner und während die PDF lädt oder speichert ist der Knopf grau. Der Tooltip nennt den Grund.
- Eine Meldung nennt die Kopie, {ui:Öffnen} zeigt sie.

### Seiten

Links in {ui:Seiten}, der Seitenliste. {key:pageList} blendet sie ein und aus. Ein Klick wählt und springt, Strg plus Klick (Mac: ⌘) fügt hinzu, Umschalt plus Klick wählt einen Bereich. Ohne Auswahl wirkt jeder Befehl auf die Seite, die du siehst.

| Befehl | Kürzel |
|---|---|
| {ui:Seiten links drehen} | {key:rotatePagesLeft} |
| {ui:Seiten rechts drehen} | {key:rotatePagesRight} |
| {ui:Seiten nach oben verschieben} | {key:movePagesUp} |
| {ui:Seiten nach unten verschieben} | {key:movePagesDown} |
| {ui:Seiten löschen} | {key:deletePages} oder {key:delete} in der Seitenliste |
| {ui:Seiten aus PDF einfügen …} | {key:insertPdf} |

- Die Befehle nennen die Seite oder wie viele Seiten sie wirken. Die Leiste unter der Liste zählt ab zwei gewählten Seiten mit, das Kreuz hebt schon ab einer die Auswahl auf. Das Löschen aller Seiten sagt, dass eine bleibt.
- {key:rotatePagesLeft}, {key:rotatePagesRight} und {key:insertPdf} wirken nur in der Seitenliste, die anderen Kürzel und die Befehlspalette überall.
- In der Seitenliste: {key:up} und {key:down} wechseln die Seite, {key:home} und {key:end} gehen zur ersten und letzten, {key:pageup} und {key:pagedown} blättern, {key:shift-up}, {key:shift-down}, {key:shift-home}, {key:shift-end}, {key:shift-pageup} und {key:shift-pagedown} erweitern die Auswahl, {key:cmd-a} wählt alle, {key:shift-f10} öffnet das Menü.
- Seiten ziehen, die Linie zeigt, wohin sie kommen. {key:escape} bricht das Ziehen ab.
- PDFs aus dem Dateimanager auf die Seitenliste ziehen fügt sie zusammen an der Linie ein, als ein Schritt für {key:Undo} mit einer Meldung. Während eine PDF gespeichert wird, warten weitere. {key:escape} hält die übrigen an, schon eingefügte bleiben.
- Eine Seite aus der Seitenliste in eine Notiz ziehen bettet sie dort ein, wie im Beispiel unten; mehrere gewählte Seiten ergeben je einen Absatz. Die Linie zeigt, wo der Absatz landet, {key:Undo} in der Notiz nimmt ihn zurück. Die PDF selbst, als Tab oder aus der Dateiliste gezogen, bettet die ganze Datei ein; am rechten Rand abgelegt öffnet sie sich daneben.
- Eine Seite auf die Seitenliste einer anderen PDF ziehen fügt dort eine Kopie an der Linie ein, als ein Schritt für {key:Undo}. Die Quelle bleibt, wie sie ist.
- Nach dem Löschen ist die Seite an der Stelle der gelöschten gewählt, so geht es mit der Tastatur weiter.
- Bei Word- und PowerPoint-Dateien steht unter der Liste, warum sich Seiten dort nicht ändern lassen.
- Rechtsklick auf eine Seite zeigt dieselben Befehle.
- Ein Hinweis verschwindet mit {key:escape}.
- Die Datei ändert sich wirklich. {key:Undo} und {key:Redo} stellen sie Byte für Byte wieder her, die Markierungen bleiben an ihrem Text.
- Die letzte Seite lässt sich nicht löschen. Andere Dateien als PDFs werden übersprungen.

````example Seite verlinken
![[Skript.pdf#page=3]]
````

## Lesen und Bearbeiten

| Modus | Kürzel |
|---|---|
| {ui:Lesen} / {ui:Bearbeiten} | {key:togglePreview} |
| {ui:Geteilte Ansicht}, nur im Quelltext | {key:splitPreview} |

Notizen öffnen in der Live-Ansicht, in einer Spalte gut lesbarer Breite. Welcher Editor, legst du in {key:OpenSettings} → {ui:Notizen} fest: {ui:Live-Ansicht: Formatierung beim Schreiben sichtbar} oder {ui:Quelltext: Text mit Zeilennummern}. {key:toggleLive}, {ui:Live-Ansicht} im Menü {ui:Darstellung} oder die {key:CommandPalette} wechselt sofort. {ui:Text in voller Breite} im Menü {ui:Mehr} der Notiz nutzt die ganze Fläche.

Beim Wechsel bleibt der Cursor an seiner Stelle im Text, und {key:Undo} nimmt auch Änderungen vor dem Wechsel zurück. Die Knöpfe bleiben an ihrem Platz; die {ui:Geteilte Ansicht} ist im Live-Editor ausgegraut.

`/` am Zeilenanfang öffnet eine Liste zum Einfügen: Überschriften, Listen, Aufgaben, Tabelle, {ui:Formelblock}, {ui:Hinweisbox}, {ui:Notiz einbetten} und mehr. Tippen filtert, was mit den Buchstaben beginnt, steht oben. Rechts steht das Kürzel, wo es eins gibt. {key:return} oder {key:tab} fügt ein. Ohne `/`: Rechtsklick → {ui:Hinzufügen}, oder {ui:Tabelle einfügen} und Co. in der {key:CommandPalette}. Ein eigenes Kürzel legst du in {key:OpenSettings} → {ui:Tastenkürzel} fest.

## In der Notiz suchen

| Kürzel | Wirkung |
|---|---|
| {key:findInNote} | Suche öffnen, markierter Text wird eingesetzt |
| {key:cmd-alt-f} | Suche mit Ersetzen-Zeile öffnen |
| {key:return} / {key:shift-return} | Nächster / vorheriger Treffer |
| {key:escape} | Suche schließen |

Das Kürzel gilt auch, wenn der Fokus in der Dateiliste oder bei den Tabs liegt. Bei PDFs und Tabellen öffnet es deren Suche, auch wenn sie nur zuletzt aktiv waren. Ohne offene Notiz, PDF oder Tabelle ist {ui:In Notiz suchen} im Menü {ui:Bearbeiten} ausgegraut, {ui:In Notiz ersetzen} ohne Notiz.

Alle Treffer sind markiert, der aktuelle kräftiger. Groß- und Kleinschreibung sowie Umlaute sind egal, bis du {ui:Groß-/Kleinschreibung beachten} wählst.

{ui:In Notiz ersetzen} oder der Pfeil links in der Suche zeigt die Ersetzen-Zeile. {key:return} ersetzt einen Treffer, {ui:Alle ersetzen} alle. Ein {key:Undo} nimmt es zurück. Im Lesen-Modus gibt es kein Ersetzen.

## Rechtschreibung

Nur auf dem Mac; unter Windows und Linux fehlen die Einstellung und der Befehl, bis die Prüfung dort angebunden ist. Unbekannte Wörter sind unterstrichen. Zeige auf eins, dann {ui:Zum Wörterbuch hinzufügen}, auch im Rechtsklick-Menü. Ohne Maus: {ui:Zum Wörterbuch hinzufügen} in der {key:CommandPalette} nimmt das Wort an der Schreibmarke auf.

Die Wörter stehen in den Einstellungen unter {ui:Eigenes Wörterbuch}; das Kreuz nimmt eins wieder heraus. Alle offenen Notizen übernehmen das sofort.

## Lange Notizen

Ab etwa 20 KB bricht der Quelltext Zeilen nicht um. {key:toggleWrap} oder {ui:Zeilenumbruch ein-/ausschalten} in der {key:CommandPalette} schaltet den Umbruch für diese Notiz um, bis etwa 35 KB; Schrumbi merkt sich das. Darüber erklärt Schrumbi, warum es nicht geht. Ab etwa 200 KB zeigt der Quelltext auch keine Hervorhebung, damit das Tippen schnell bleibt. Das steht unten als {ui:Lange Notiz: ohne Zeilenumbruch und Hervorhebung}.

Eine einzelne Zeile über etwa 250 KB öffnet Schrumbi im Quelltext statt in der Live-Ansicht. Unten steht {ui:Diese Zeile ist zu lang für die Live-Ansicht. Esc: zurück zur Live-Ansicht}; {key:escape} oder ein Klick darauf führt zurück.

Die Live-Ansicht zeigt eine Zeile über etwa 32 KB nur mit ihrem Anfang; {ui:Lesen} zeigt sie ganz, und ein Klick auf den Hinweis darunter öffnet die Zeile im Quelltext.

Auf dem Mac prüft die Rechtschreibung ab etwa 80 KB nur den Text um die Schreibmarke. Das steht unten als {ui:Rechtschreibung nur um die Schreibmarke}.

## Sichern

Schrumbi sichert nach 2 Sekunden Tipp-Pause, einstellbar unter {ui:Notizen} → {ui:Sichern}. {key:Save} sichert sofort, Schließen sichert immer.

Hat eine andere App die Datei geändert, siehst du einen Hinweis. Nichts geht verloren, bis du wählst:

| Knopf | Ergebnis |
|---|---|
| {ui:Vergleichen} | Beide Versionen nebeneinander |
| {ui:Neu laden} | Die Version der anderen App |
| {ui:Meine behalten} | Deine Version |

## Verknüpfen

- `[[` tippen und eine Notiz wählen. {key:escape} gleich danach nimmt die leeren Klammern wieder weg.
- {key:toggleInspector} zeigt {ui:Rückverweise}, {ui:Ausgehend} und {ui:Erwähnungen ohne Link}.
- {key:showGraph} zeigt alle Notizen als Graph.

## Tabs

| Kürzel | Wirkung |
|---|---|
| {key:closeTab} | Tab schließen |
| {key:reopenTab} | Zuletzt geschlossenen Tab öffnen |
| {key:nextTab} / {key:previousTab} | Nächster / vorheriger Tab |

- Ein Tab geht nur zu, wenn du ihn schließt: ×, Mittelklick oder {key:closeTab}.
- Jede Datei bekommt ihren eigenen Tab. Eine offene Datei wird nie doppelt geöffnet: ihr Tab kommt nach vorn.
- Ein Link öffnet sein Ziel in einem neuen Tab, die Notiz mit dem Link bleibt offen.
- Rechtsklick auf einen Tab: {ui:Andere Tabs schließen}, {ui:Tabs rechts schließen}, {ui:Alle Tabs schließen}. Rechts im Menü steht jeweils das Kürzel.
- Tab ziehen ändert die Reihenfolge. {key:reopenTab} bringt den Tab an seinen alten Platz zurück.

### Nebeneinander

- Rechtsklick auf einen Tab → {ui:Daneben öffnen}, oder einen Tab oder eine Datei aus der Liste in die rechte Hälfte ziehen. Dort steht {ui:Rechts öffnen}.
- {key:openBeside} öffnet die Notiz, in der du bist, rechts daneben, {key:switchSide} wechselt zwischen den Seiten.
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

Im PDF bleiben Überschriften bei ihrem Text; kurze Tabellen, Codeblöcke, Hinweise, Zitate und Listenpunkte werden nicht über zwei Seiten geteilt, lange Tabellen wiederholen ihre Kopfzeile. Bilder erscheinen so groß wie in der Ansicht, nie breiter als der Text und nie höher als etwa eine halbe Seite; eine Bildunterschrift bleibt beim Bild.

Liegt neben der Notiz schon ein früherer Export, fragt Schrumbi vor dem Ersetzen. {ui:Rückgängig} in der Meldung danach holt das alte PDF zurück.

- {ui:Nicht mehr fragen} ersetzt künftig ohne Frage.
- Die Frage schaltest du wieder ein unter {key:OpenSettings} → {ui:Notizen} → {ui:Vor dem Ersetzen eines früheren Exports fragen}.
- Eine andere Datei mit diesem Namen bleibt unangetastet, das PDF bekommt eine Nummer. Der nächste Export ersetzt dieses nummerierte PDF.
