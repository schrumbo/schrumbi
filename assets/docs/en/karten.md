# Cards

Flashcards are text in your notes. Schrumbi finds them and schedules the reviews. {key:goCards} opens them.

## Write Cards

````example One card
What is a median? :: The middle value of a sorted list.
````

````example In both directions
Dog ::: Hund
````

````example Over several lines
Which three measures of location are there?
?
Mean, median and mode.
````

````example Cloze
The {{c1::variance}} is the square of the {{c2::standard deviation}}.
````

- `::` needs a space on both sides. `std::vector` stays text.
- With several lines, the answer runs until the next blank line or heading.
- `??` instead of `?` also studies that card in both directions.
- In notes with `#cards` or `#flashcards`, `{{gap}}` and `==highlighted==` also become gaps.
- `cards: false` in the properties excludes a note. Code and formula blocks never make a card.

Without typing: select text and {ui:Karte aus Auswahl…}, or {ui:Neue Karte} with {key:n}. From anywhere, {key:newCard} opens a new card.

While there are no cards, the page shows only the two simplest ways to write one; {ui:Mehr Schreibweisen} unfolds the rest.

## Study

{key:space} starts. What is due today gets studied.

| Key | Effect |
|---|---|
| {key:space} | {ui:Antwort zeigen} |
| {key:1} | {ui:Nochmal} |
| {key:2} | {ui:Schwer} |
| {key:3} or {key:space} | {ui:Gut} |
| {key:4} | {ui:Einfach} |
| {key:e} | Edit card |
| {key:o} | Show in the note |
| {key:cmd-z} | Undo last answer |
| {key:Redo} | Give an undone answer again |
| {key:escape} | Quit |

Scheduling uses FSRS-6, like Anki. A higher target means more reviews: above 95 % costs a lot of time for little gain. A new day starts at 4 am.

## Before the Exam

In the deck menu, {ui:Alles wiederholen}: every unpaused card once, in random order. There is only {ui:Nochmal} and {ui:Gut}, the scheduling stays unchanged.

## From Anki

1. {ui:Aus Anki importieren…} or {key:i}.
2. Choose `.apkg`, `.colpkg`, `.txt`, `.tsv` or `.csv`.
3. {ui:Lernfortschritt übernehmen} keeps due dates and stability.

Each deck becomes a note in the {ui:Zielordner}. Cards that are already there are skipped. If two cards have the same question, both arrive, the second with a number.

- Cards never learned stay new.
- {key:Undo} takes back the whole import, including cards added to an existing note.

## Where Things Are

| File | Contents |
|---|---|
| your notes | The cards themselves |
| `Karteikarten.md` in the deck's folder | New cards without a note of their own |
| `.schrumbi-cards.json` | The study progress |

Settings for this: {key:OpenSettings} → {ui:Karten}.
