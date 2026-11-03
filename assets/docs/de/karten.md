# Karten

Karteikarten stehen als Text in deinen Notizen. Schrumbi findet sie und plant die Wiederholungen. {key:GoCards} öffnet sie.

## Karten schreiben

````example Eine Karte
Was ist ein Median? :: Der mittlere Wert einer sortierten Liste.
````

````example In beide Richtungen
Hund ::: dog
````

````example Über mehrere Zeilen
Welche drei Lageparameter gibt es?
?
Mittelwert, Median und Modus.
````

````example Lückentext
Die {{c1::Varianz}} ist das Quadrat der {{c2::Standardabweichung}}.
````

- `::` braucht ein Leerzeichen auf beiden Seiten. `std::vector` bleibt Text.
- Bei mehreren Zeilen reicht die Antwort bis zur nächsten Leerzeile oder Überschrift.
- `??` statt `?` lernt auch diese Karte in beide Richtungen.
- In Notizen mit `#karten` oder `#flashcards` werden auch `{{Lücke}}` und `==Markiertes==` zu Lücken.
- `cards: false` in den Eigenschaften nimmt eine Notiz aus. Code und Formelblöcke ergeben nie eine Karte.

Ohne Tippen: Text markieren und {ui:Karte aus Auswahl…}, oder {ui:Neue Karte} mit {key:n}.

## Lernen

{key:space} startet. Gelernt wird, was heute fällig ist.

| Taste | Wirkung |
|---|---|
| {key:space} | {ui:Antwort zeigen} |
| {key:1} | {ui:Nochmal} |
| {key:2} | {ui:Schwer} |
| {key:3} oder {key:space} | {ui:Gut} |
| {key:4} | {ui:Einfach} |
| {key:e} | Karte bearbeiten |
| {key:o} | In der Notiz zeigen |
| {key:cmd-z} | Letzte Antwort zurück |
| {key:escape} | Beenden |

Geplant wird mit FSRS-6, wie in Anki. Ein neuer Tag beginnt um 4 Uhr.

## Vor der Prüfung

Im Stapelmenü {ui:Alles wiederholen}: jede nicht pausierte Karte einmal, in zufälliger Reihenfolge. Es gibt nur {ui:Nochmal} und {ui:Gut}, die Planung bleibt unverändert.

## Aus Anki

1. {ui:Aus Anki importieren…} oder {key:i}.
2. `.apkg`, `.colpkg`, `.txt`, `.tsv` oder `.csv` wählen.
3. {ui:Lernfortschritt übernehmen} behält Termine und Stabilität.

Je Stapel entsteht eine Notiz im {ui:Zielordner}. Doppelte Karten werden übersprungen.

## Wo was liegt

| Datei | Inhalt |
|---|---|
| deine Notizen | Die Karten selbst |
| `Karteikarten.md` im Ordner des Stapels | Neue Karten ohne eigene Notiz |
| `.schrumbi-cards.json` | Der Lernstand |

Einstellungen dazu: {key:OpenSettings} → {ui:Karten}.
