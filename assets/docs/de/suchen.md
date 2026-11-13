# Suchen & Befehle

Drei Wege, alles zu finden.

| Was | Kürzel | Findet |
|---|---|---|
| **Befehlspalette** | {key:CommandPalette} | Jeden Befehl der App |
| {ui:Datei schnell öffnen} | {key:quickOpen} | Dateien nach Namen |
| {ui:In allen Dateien suchen} | {key:searchFiles} | Text in Notizen, PDFs und Namen |

## Befehlspalette

Tippen, mit {key:up} {key:down} wählen, {key:return} führt aus. Ohne Eingabe stehen deine letzten Befehle oben. Umlaute sind egal.

| Du tippst | Du bekommst |
|---|---|
| `synchro` | {ui:Jetzt synchronisieren} |
| `tages` | {ui:Tagesnotiz öffnen} |
| `pdf` | {ui:Als PDF exportieren}, wenn eine Notiz offen ist |
| `löschen` | {ui:Notiz löschen}, wenn eine Notiz offen ist |

Gibt es keinen Treffer, steht oben {ui:In allen Dateien suchen} mit deiner Eingabe, {key:return} startet die Suche.

## Datei schnell öffnen

Ohne Eingabe stehen offene Tabs und zuletzt geöffnete Dateien oben. Mit Eingabe steht der Name, der genau passt, ganz oben, dann Namen, die so beginnen.

| Du tippst | Ergebnis |
|---|---|
| `protokoll` | Dateien mit „protokoll“ im Namen oder Ordner |
| `#prüfung` | Notizen mit dem Tag `#prüfung` |
| `Skript#Kapitel 2` | Springt zur Überschrift in der Notiz |

Gibt es den Namen noch nicht, bietet Schrumbi an, die Notiz anzulegen.

## In allen Dateien suchen

Durchsucht Namen, Ordner, PDFs und Notizen. Passende Einträge stehen oben dazu. {key:up} {key:down} wählen einen Treffer, {key:return} öffnet ihn. Die Schreibmarke steht dann am Anfang der Fundstelle, die kurz aufleuchtet.

| Eingabe | Findet |
|---|---|
| `w3` oder `woche 3` | Dateien aus Vorlesungswoche 3 |
| `kw44` | Dateien aus der Vorlesungswoche in KW 44 |
| `INF1` | Dateien aus dem Modul mit diesem Kürzel |

Suchbefehle grenzen weiter ein. Mit ihnen werden Notizen durchsucht, PDFs nur nach Namen.

| Eingabe | Findet |
|---|---|
| `tag:#prüfung` | Notizen mit diesem Tag |
| `"genau so"` | Den Satz in genau dieser Form |
| `klausur -alt` | „klausur“, aber nicht „alt“ |
| `essay OR referat` | Eines von beiden |
| `path:Statistik` | Dateien, deren Pfad „Statistik“ enthält |
| `task-todo:lesen` | Offene Aufgaben mit „lesen“ |
| `[status:offen]` | Notizen mit dieser Eigenschaft |

Alle Befehle stehen im Menü {ui:Suchbefehle} neben dem Suchfeld.

## Kürzel und Hilfe

- {key:ShowShortcuts} zeigt alle Tastenkürzel.
- {key:ShowHelp} öffnet diese Anleitung.
