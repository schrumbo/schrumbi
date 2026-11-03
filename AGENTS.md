# Schrumbi

Rust/GPUI desktop study planner working on the user's plain folders. Losing user data is the worst possible bug. Feel target: Figma.

## Commands

- Check everything: `.claude/bin/gate` (clippy and UI lint on changed lines, then tests)
- Tests: `cargo nextest run`, or `cargo test`
- Run, drive, screenshot and measure the real app: `.claude/bin/app help` (scratch data, never the real folder)
- Walk every page and window once: `.github/smoke.sh`

## Rules

- Never run against `~/UXD` or the real settings. Manual runs use a scratch folder and `SCHRUMBI_HOME`. Never touch `/Applications/Schrumbi.app` or instances you did not start.
- Performance is a feature: frame p95 under 8.3 ms and max under 16.7 ms; at 20000 notes median within budget and no frame over 100 ms; no frames and near 0% CPU when idle. Measure with `app perf`; judge only the `perf` profile.
- UI work is verified by using the running app, not by reading code.
- Do not run `cargo fmt` or `rustfmt`: the repo is not rustfmt-formatted and it would rewrite thousands of hunks. Match the surrounding style.
- No code comments. Doc comments only on public items, in the existing terse style.
- UI text is German source passed through `tr()`/`trf()`; add the English string to `assets/i18n/en.json` by editing only the new keys, never by rewriting the file. A test fails on missing keys.
- Cross-platform crates never go into `target.cfg` sections. Linux and Windows are only verified by CI.
- Never `git push`. No co-author lines. Before committing, search the diff for personal data; use the placeholders `Muster-M`, `Mueller-C`, `Beispiel`.
- Other sessions may be working in this tree. Leave changes you did not make alone and stage files by name.
- The owner also works on this repo from a second machine (Mac and Windows). Run `.claude/bin/sync` before work and before each commit; never stash, merge-pull or force. Details in `.claude/rules/workflow.md`.

## Plans

`POLISH_PLAN.md` and `OBSIDIAN_ROADMAP.md` are local and untracked. When continuing that work, read the file and start at the first open item.
