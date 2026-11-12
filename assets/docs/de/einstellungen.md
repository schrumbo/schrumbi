# Einstellungen

{key:OpenSettings} öffnet die Einstellungen. Oben rechts suchst du in allen Seiten, {key:cmd-f} springt hinein.

| Seite | Hier stellst du ein |
|---|---|
| {ui:Allgemein} | Erscheinungsbild, eigene Designs, Sprache, Erinnerungen, Namensmuster, Seitenleiste |
| {ui:Semester} | Vorlesungszeit, Pausen, Feiertage |
| {ui:Module} | Kürzel, Ordner, Links, wiederkehrende Deadlines, Stundenplan |
| {ui:Notizen} | Editor, Schriftgröße, eigenes Wörterbuch, Sichern, PDF-Export, Obsidian |
| {ui:Karten} | Erinnerungsziel, neue Karten und Wiederholungen pro Tag |
| {ui:Sync} | Online-Speicher, Automatik, Ausschalten |
| {ui:Tastenkürzel} | Die meisten Kürzel ändern |

## Suchen

Die Suche kennt auch andere Wörter: `dark` findet das Erscheinungsbild, `git` den Sync. Sie findet auch Module, Semester und Kürzel.

| Taste | Wirkung |
|---|---|
| {key:up} {key:down} | Treffer wählen |
| {key:return} | Hinspringen, die Einstellung leuchtet kurz auf |
| {key:escape} | Suche leeren |

## Kürzel ändern

1. {ui:Tastenkürzel} öffnen und auf ein Kürzel oder seinen Namen klicken, oder mit Tab hinfahren und {key:return} drücken.
2. Neue Tasten drücken. Dabei sein muss eine Sondertaste, F-Tasten gehen allein. Fehlt die Sondertaste, steht ein Hinweis da. Nicht setzen lassen sich Kopieren, Einsetzen, Ausschneiden, Alles auswählen, Rückgängig, Wiederholen, Tab schließen, Beenden, {key:tab} und die Tasten der Textbearbeitung (Umschalt mit Pfeilen und {key:return}, Wort- und Zeilensprünge, Wörter löschen); der Grund steht an der Zeile. Alle anderen Tasten von Schrumbi lassen sich neu belegen.
3. {key:escape} bricht ab, {key:delete} entfernt das Kürzel.

Ein Klick ins Suchfeld oder woandershin beendet das Aufnehmen. Steht etwas im Suchfeld, leert {key:escape} es. Ist ein Kürzel doppelt belegt, steht {ui:Doppelt} und der andere Befehl an der Zeile. Beim Aufnehmen löst die Taste nichts aus, so lässt sich auch ein belegtes Kürzel setzen. {ui:Alle zurücksetzen} stellt die Standards wieder her. Nach jeder Änderung steht oben auf der Seite ein Hinweis, der den Befehl und seine Tasten nennt, mit {ui:Rückgängig}, der den letzten Stand zurückholt. Er bleibt beim Scrollen stehen und verschwindet nach wenigen Sekunden, mit dem Schließen-Knopf oder mit {key:escape}. Auf der Seite {ui:Tastenkürzel} filtert die Suche oben rechts die Liste der Kürzel.

## Folgesemester

Semester in der Liste wählen, dann das Kopier-Symbol unter der Liste: {ui:Folgesemester mit denselben Modulen und Regeln anlegen}. Es beginnt zwei Wochen nach dem letzten Semester.

## Eigene Designs

{ui:Design laden…} neben {ui:Erscheinungsbild} öffnet eine JSON-Datei. Schrumbi prüft sie, behält eine Kopie und wechselt sofort. {ui:Entfernen} unter {ui:Eigene Designs} löscht die Kopie, {ui:Rückgängig} bringt sie zurück. Hat ein geladenes Design den Namen eines vorhandenen, ersetzt es dieses, und {ui:Rückgängig} bringt das alte zurück. Fällt die Datei des gewählten Designs weg, gilt wieder das Erscheinungsbild von vorher. {key:CommandPalette} nennt jedes Design beim Namen und bietet {ui:Design laden…} an.

```json
{
  "schemaVersion": 1,
  "name": "Mein Design",
  "content": "#FDF6E3",
  "ink": "#073642",
  "accent": "#268BD2"
}
```

Alles andere ergänzt Schrumbi aus diesen drei Farben.

| Schlüssel | Bedeutung |
|---|---|
| `content`, `ink`, `accent` | Pflicht: Fläche, Text, Akzent |
| `dark` | `true` oder `false`; fehlt er, entscheidet die Helligkeit |
| `sidebar`, `secondaryBg`, `column`, `material` | Flächen |
| `primary`, `secondary`, `tertiary`, `separator`, `sidebarBorder` | Text und Linien, auch als `#RRGGBBAA` |
| `accentText`, `onAccent`, `selection` | Akzent als Text, Schrift auf dem Akzent, Auswahl |
| `green`, `blue`, `orange`, `red`, `gray`, `greenText`, `orangeText` | Zustände |
| `moduleColors` | Farben der Module: `blue`, `green`, `orange`, `purple`, `pink`, `teal`, `red`, `yellow`, `indigo`, `mint`, `brown`, `gray` |
| `syntax` | Farben von Code: `keyword`, `string`, `comment`, `number`, `type`, `function` |

Farben stehen als `#RRGGBB`. Bei einem Fehler nennt Schrumbi den Schlüssel, und nichts ändert sich; Text braucht 4,5 : 1 auf `content` und `sidebar`. Änderungen an der Kopie im Ordner `themes` gelten, sobald das Fenster wieder vorn ist. Fertige Designs liegen im Ordner `themes` des Quellcodes.

## Wo die Einstellungen liegen

| Was | Wo |
|---|---|
| Semester, Module, Regeln, Name, Namensmuster | `schrumbi.json` in deinem Ordner |
| Darstellung, Kürzel, Sprache | Bei Schrumbi auf diesem Gerät |
| Geladene Designs | Ordner `themes` bei Schrumbi auf diesem Gerät |

Deine Dateien gehen nur an deinen eigenen Sync-Speicher. Gelesen werden nur dein Ordner und ein verbundener Obsidian-Vault.
