# Workflow

Standing instruction from the owner: use the agents below without being asked. Do not wait for a request to spawn them.

- Any change beyond a few lines follows the `ship` skill. Questions, reading and one-line fixes do not.
- A Stop hook runs `.claude/bin/gate` and asks for missing reviews. Treat its message as a task. If a finding is a false positive or a review does not apply, say why in one line and stop again; it does not repeat for an unchanged tree.

| Work | Who | Model |
|---|---|---|
| Decisions, architecture, integration | main session | Opus |
| Finding code across many files | `Explore` | small default |
| Implementing a clear, bounded spec | `rust-implementer` | Sonnet |
| Correctness review of a diff | `rust-reviewer` | Opus |
| UI checklist on a diff | `ui-auditor` | Haiku |
| Using the real app, screenshots, hover | `ux-tester` | Sonnet |
| Frame times, idle cost, huge data | `perf-tester` | Sonnet |
| Bug still alive after two fix attempts | `root-cause` | Fable |

- Give each agent the exact scope and acceptance check, not the conversation. Ask for findings, not narration.
- A UI change is not done until it was used in the running app, and performance not until it was measured. Code reading and passing tests prove neither.
- Scripts before agents: anything `gate` or `app` can answer needs no model.
- Use the LSP tool (definitions, references) before grepping for symbols.
- Parallel independent features go through the `herd` skill, never through parallel edits in one tree.
- Whole-area polish passes use the `app-polish` skill, with `ux-tester` and `perf-tester` as its hands. It is the owner's standard for all UI work: Figma level, evidence for every claim.

## Two machines

The owner works on this repo on a Mac (`ssh mac`) and a Windows PC (`ssh windows`), each with its own clone: Mac `~/Developer/schrumbi-rs`, Windows `C:\Users\schrumbo\Documents\projects\schrumbi`. origin is the only link between them and only the owner pushes.

- Session start runs `.claude/bin/sync`: it fetches, fast-forwards, rebases unpushed local commits onto origin when the tree is clean, and reports what it left alone. Act on that report before editing.
- Run `.claude/bin/sync` again before every commit and before landing a herd branch.
- Never park work in `git stash`; commit it instead, unpushed commits can still be amended. Never pull with a merge commit, never force, never rewrite commits that are on origin.
- When stopping at a stable point, state how many commits are unpushed so the owner can push before switching machines.
- Untracked plans (`POLISH_PLAN.md`, `OBSIDIAN_ROADMAP.md`, `BUGS.md`) and claude-mem work state exist per machine. If one is missing or older here, copy it from the other machine, e.g. `scp mac:Developer/schrumbi-rs/POLISH_PLAN.md .` or `scp windows:Documents/projects/schrumbi/POLISH_PLAN.md .`
- `.claude/bin/app`, `ux-tester` and the gui tool are macOS-only. On Windows, UI and perf verification waits for the Mac; say so instead of skipping it silently.
