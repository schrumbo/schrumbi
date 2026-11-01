---
name: ship
description: Default path for every code change in Schrumbi beyond a few lines, from spec to commit, with agents for implementation, review and real-app testing. Use for features, bug fixes, refactors and UI work, without being asked.
---

# Ship

1. **Spec.** State the outcome, the acceptance check and every state of the user's workflow it touches: entry, hover, press, focus, loading, empty, error, undo, exit. Design it here; this is where quality is decided. For UI, read `~/.claude/skills/app-polish/references/feel.md` and `flows.md` and consult `ui-ux-pro-max:ui-ux-pro-max`; shorten the workflow before polishing it. Ask only about removals and remakes.
2. **Locate.** Use LSP and `Explore`. Do not read whole large files.
3. **Build.** Delegate to `rust-implementer` when the spec is clear and the change is mechanical or isolated. Do it yourself when it needs judgement across modules. Bugs start with a failing test.
4. **Gate.** `.claude/bin/gate` until green. Add `--deep` when `Cargo.toml` changed.
5. **Review.** In parallel, in one message:
   - always `rust-reviewer`
   - if `src/ui` changed: `ui-auditor` and `ux-tester`
   - if rendering, lists, search, scanning or parsing changed: `perf-tester`
6. **Fix.** Verify each finding before acting on it. Fix the real ones, run the gate again, then re-review only what changed substantially. At most two rounds; after that report what is open. Two failed attempts on one bug: `root-cause`.
7. **Commit.** At a stable point: stage by name, search the diff for personal data, terse message, no co-author. Never push.
8. **Report.** What works now, with evidence. Anything open. What only a person can judge, as short manual test steps.

Changes under `src/core` with high risk of data loss: also run `cargo mutants --in-diff <(git diff HEAD) -f 'src/core/**' --test-tool nextest` and add tests for survivors. It rebuilds from scratch, so only then.
