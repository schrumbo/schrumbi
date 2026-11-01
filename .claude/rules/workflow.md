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
