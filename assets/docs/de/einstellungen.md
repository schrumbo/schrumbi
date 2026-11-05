# Einstellungen

{key:OpenSettings} öffnet die Einstellungen. Oben rechts suchst du in allen Seiten, {key:cmd-f} springt hinein.

| Seite | Hier stellst du ein |
|---|---|
| {ui:Allgemein} | Erscheinungsbild, Sprache, Namensmuster, Seitenleiste, Erinnerungen |
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
2. Neue Tasten drücken. Dabei sein muss eine Sondertaste, F-Tasten gehen allein. Fehlt die Sondertaste, steht ein Hinweis da. Nicht setzen lassen sich Kopieren, Einsetzen, Ausschneiden, Alles auswählen, Widerrufen, Wiederholen, Tab schließen, Beenden, {key:tab} und die Tasten der Textbearbeitung (Umschalt mit Pfeilen und {key:return}, Wort- und Zeilensprünge, Wörter löschen); der Grund steht an der Zeile. Alle anderen Tasten von Schrumbi lassen sich neu belegen.
3. {key:escape} bricht ab, {key:delete} entfernt das Kürzel.

Ein Klick ins Suchfeld oder woandershin beendet das Aufnehmen. Steht etwas im Suchfeld, leert {key:escape} es. Ist ein Kürzel doppelt belegt, steht {ui:Doppelt} und der andere Befehl an der Zeile. Beim Aufnehmen löst die Taste nichts aus, so lässt sich auch ein belegtes Kürzel setzen. {ui:Alle zurücksetzen} stellt die Standards wieder her. Nach jeder Änderung steht unter dem Suchfeld ein Hinweis, der den Befehl und seine Tasten nennt, mit {ui:Rückgängig}, der den letzten Stand zurückholt. Er bleibt beim Scrollen stehen und verschwindet nach wenigen Sekunden. In {ui:Tastenkürzel} springt {key:cmd-f} ins Filterfeld.

## Folgesemester

Semester in der Liste wählen, dann das Kopier-Symbol unter der Liste: {ui:Folgesemester mit denselben Modulen und Regeln anlegen}. Es beginnt zwei Wochen nach dem letzten Semester.

## Wo die Einstellungen liegen

| Was | Wo |
|---|---|
| Semester, Module, Regeln, Name, Namensmuster | `schrumbi.json` in deinem Ordner |
| Darstellung, Kürzel, Sprache | Bei Schrumbi auf diesem Gerät |

Deine Dateien gehen nur an deinen eigenen Sync-Speicher. Gelesen werden nur dein Ordner und ein verbundener Obsidian-Vault.
