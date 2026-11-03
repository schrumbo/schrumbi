# Ablegen & Inbox

Ziehe eine Datei auf Schrumbi, und sie landet im richtigen Ordner mit dem richtigen Namen.

## Datei ablegen

1. Datei auf einen Eintrag, ein Modul oder die {ui:Inbox} ziehen.
2. Schrumbi schlägt Ziel und Namen vor.
3. {key:return} legt ab.

| Taste | Wirkung |
|---|---|
| {key:return} | Ablegen |
| {key:e} | Alle Felder öffnen, dann legt {key:CommitFiling} ab |
| {key:1} {key:2} {key:3} | Einen anderen Vorschlag nehmen |
| {key:escape} | Felder schließen, sonst abbrechen |

Unter {ui:Was ist das?} stellst du um zwischen {ui:Meine Abgabe}, {ui:Aufgabenstellung} und {ui:Unterlagen}.

> [!note] Verschieben oder kopieren
> Dateien aus deinem Studienordner werden verschoben. Dateien von außerhalb werden kopiert, das Original bleibt. In die {ui:Inbox} gezogene Dateien werden immer kopiert.

## Namenskonvention

Mit {ui:Nach einem Muster benennen} bekommen Abgaben einheitliche Namen.

| Platzhalter | Beispiel |
|---|---|
| `{code}` | `P1` |
| `{yy}` / `{yyyy}` | `26` / `2026` |
| `{title}` | `Stakeholder-Radar` |
| `{author}` | `Muster-M_Mueller-C` |
| `{date}` | `261028` |
| `{w}` / `{kw}` | `W03` / `KW44` |
| `{date:yyyy-MM-dd}` | `2026-10-28` |
| `{matrikel}` | Deine Matrikelnummer |

Aus dem Muster `{code}_{yy}_{title}_{author}_{date}` wird:

```
P1_26_Stakeholder-Radar_Muster-M_Mueller-C_261028.pdf
```

Eine Aufgabenstellung heißt dann `P1_26_Infografik_Aufgabe_261028.pdf`.

## Inbox

Die Inbox ist der Ordner `_Inbox` in deinem Studienordner, den Namen änderst du unter {ui:Inbox-Ordner}. Was dort liegt, wartet auf einen Platz. {key:GoInbox} öffnet sie.

| Taste | Wirkung |
|---|---|
| {key:up} {key:down} | Datei wählen |
| {key:space} | Vorschau |
| {key:return} | Ablegen |
| {key:delete} | In den Papierkorb |

{ui:Dateien hinzufügen…} holt Dateien von woanders dazu.
