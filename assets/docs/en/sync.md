# Sync

Sync backs up your folder with Git and syncs it with other devices. Every version stays in the {ui:Verlauf}.

## Set Up

You need Git version 2.38 or newer.

1. {key:CommandPalette} → {ui:Sync einrichten…}
2. {ui:Aufräumen}: Schrumbi leaves out temporary files, {ui:Details} shows which.
3. {ui:Verbinden}: {ui:Privat auf GitHub anlegen} or your own {ui:Adresse des Online-Speichers}. Without an address, go on with {ui:Ohne Online-Speicher weiter}.
4. {ui:Sicherung}: {ui:Automatisch} or {ui:Manuell}.

Without an address, everything stays local, the history still works. The line in the sidebar then says {ui:Lokal gesichert}.

{key:escape} or the X at the top right closes like {ui:Später einrichten}.

## App Data

Tasks, cards and settings (`schrumbi.json`, `.schrumbi-state.json`, `.schrumbi-cards.json`) travel along. If they were changed on two devices, Schrumbi merges them by itself.

{ui:App-Daten mitnehmen} at the bottom of the sync window turns that off. The files stay on disk, on the other device too.

## Day to Day

| Mode | What happens |
|---|---|
| {ui:Automatisch} | Backs up after 5 minutes of rest (adjustable), on open and on quit |
| {ui:Manuell} | You back up with {key:syncNow}, then a message says how many files were saved, or {ui:Nichts Neues} |

The line at the bottom of the sidebar shows the state, a click opens details.

## Offline

Offline, it backs up locally and uploads later. You do not need to do anything.

If many files are gone at once, saving pauses, also when started by hand. If that was on purpose, {ui:Trotzdem sichern}.

## Conflicts

If the same file was changed on two devices, the sync line in the sidebar shows the conflicts, and automatic sync waits. A click on it or {ui:Konflikte lösen…} in {key:CommandPalette} opens the choice. Nothing is lost.

| Choice | Result |
|---|---|
| {ui:Beide behalten} | Your version keeps the name, the other lies next to it as a copy |
| {ui:Meine} | Your version stays |
| {ui:Andere} | The version from the other device |
| {ui:Von Hand} | Both stay, then your version opens for merging. Text only |

## What Schrumbi Never Does

Schrumbi deletes no history and rewrites none. Credentials stay with Git.

{ui:Ausschalten} in {key:OpenSettings} → {ui:Sync} only ends the syncing. Repository and history stay.
