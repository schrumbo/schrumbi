# Sync

Sync sichert deinen Ordner mit Git und gleicht ihn mit anderen Geräten ab. Jeder Stand bleibt im {ui:Verlauf}.

## Einrichten

Du brauchst Git ab Version 2.38.

1. {key:CommandPalette} → {ui:Sync einrichten…}
2. {ui:Aufräumen}: Schrumbi schlägt eine `.gitignore` vor.
3. {ui:Verbinden}: {ui:Privat auf GitHub anlegen} oder eine eigene {ui:Adresse des Online-Speichers}.
4. {ui:Wann soll gesichert werden?} {ui:Automatisch} oder {ui:Manuell}.

Ohne Adresse bleibt alles lokal, der Verlauf funktioniert trotzdem.

## Im Alltag

| Modus | Was passiert |
|---|---|
| {ui:Automatisch} | Sichert nach 5 Minuten Ruhe (einstellbar), beim Öffnen und Beenden |
| {ui:Manuell} | Du sicherst mit {key:syncNow} |

Die Zeile unten in der Seitenleiste zeigt den Stand, ein Klick öffnet Details.

## Ohne Netz

Offline wird lokal gesichert und später hochgeladen. Du musst nichts tun.

Fehlen viele Dateien auf einmal, pausiert das automatische Sichern. War das gewollt, {ui:Trotzdem sichern}.

## Konflikte

Wurde dieselbe Datei auf zwei Geräten geändert, zeigt die Sync-Zeile in der Seitenleiste die Konflikte, und der automatische Sync wartet. Ein Klick darauf oder {ui:Konflikte lösen…} in {key:CommandPalette} öffnet die Auswahl. Nichts geht verloren.

| Wahl | Ergebnis |
|---|---|
| {ui:Beide behalten} | Deine Version behält den Namen, die andere liegt als Kopie daneben |
| {ui:Meine} | Deine Version bleibt |
| {ui:Andere} | Die Version vom anderen Gerät |
| {ui:Von Hand} | Beide bleiben, dann öffnet sich deine Version zum Zusammenführen. Nur bei Text |

## Was Schrumbi nie tut

Schrumbi löscht keinen Verlauf und schreibt keinen um. Zugangsdaten bleiben bei Git.

{ui:Ausschalten} in {key:OpenSettings} → {ui:Sync} beendet nur den Abgleich. Repository und Verlauf bleiben.
