---
name: herd
description: Runs independent features in parallel as separate Claude sessions in their own git worktrees, visible in the Herdr sidebar. Use when work splits into tasks touching different files, or when asked to work on several things at once. Not for reviews or small tasks; those use subagents.
---

# Herd

Each helper is a full session with its own context, so it costs far more than a subagent. Use it only for tasks that run long and do not overlap. A first build in a new worktree takes minutes; `sccache` shares dependencies between worktrees.

```sh
.claude/bin/herd new <name> "<exact task and acceptance check>"
.claude/bin/herd new <name> --model opus "<task>"
.claude/bin/herd ls
.claude/bin/herd rm <name>
```

Worktree: `.claude/worktrees/<name>`, branch `herd/<name>`. Default model Sonnet. Helpers follow `ship`, commit there and report back into this session as `<hc from=...>`.

1. Split the work so files do not overlap. Give each helper the task, the scope and the acceptance check.
2. Keep working. Wait with `hc wait --children any`, never sleep.
3. Steer with `hc send <name> "..."`. Read with `hc read --lines 100 <name>`.
4. On a report, read the actual diff: `git diff main...herd/<name>`. Do not infer success from the report.
5. Land with `git merge --no-ff herd/<name>`, run `.claude/bin/gate --base HEAD~1`, then `herd rm <name>`.
