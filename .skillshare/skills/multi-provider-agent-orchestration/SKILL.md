---
name: multi-provider-agent-orchestration
description: "Directly delegate bounded work to Codex, Gemini, Claude, OpenCode, GLM or other agents without using the SWARMS runtime. Use native subagents/tools first, otherwise call the provider CLI directly."
license: MIT
metadata:
  author: SWARMS
  version: "3.0"
---

# Direct agent delegation

## Activation Contract

Use this skill when the goal is to ask one or more other agents to do bounded work **directly**.
This skill is deliberately runtime-independent: do not create a SWARMS plan, do not call the
SWARMS Rust binary, and do not use SWARMS run state merely to delegate a task.

Use `$swarms` instead when the user needs a durable workflow with DAG dependencies, resume,
provider/global concurrency caps, persisted state, observer/telemetry, retries, or test-time scaling.

## Delegation Order

1. If the current host exposes a native subagent/delegation tool, use that tool directly.
2. Otherwise, if the requested agent has a local CLI, invoke that CLI directly from the target
   workspace in non-interactive/headless mode.
3. If neither route exists, report the unavailable agent. Never silently replace it with another
   provider.

Do not route a direct delegation through `cargo run ... swarms`, a workflow JSON, or a SWARMS
provider route.

## Task Contract

Every delegated task must state, in the prompt itself:

- objective and concrete acceptance criteria;
- exact repository/workspace and relevant files;
- read-only versus write permission;
- constraints and invariants that must not change;
- expected deliverable (analysis, patch, files, test evidence, etc.);
- deterministic validation to run when applicable;
- whether commit/push/external writes are forbidden or explicitly authorized.

For concurrent writers, use disjoint writable paths or isolated worktrees. A read-only reviewer can
share the main worktree. Do not let multiple write-capable agents race over the same files.

## Direct CLI Patterns

Always check the installed CLI's current `--version` and relevant `--help` before relying on flags.
Run the command with the target repo as its working directory (or use the CLI's explicit directory
flag when available).

### Codex CLI

Read-only:

```powershell
codex exec -C <workspace> -s read-only -m <model> "<task contract>"
```

Workspace write:

```powershell
codex exec -C <workspace> -s workspace-write -m <model> "<task contract>"
```

Do not use `danger-full-access` or bypass approvals unless the user explicitly authorized that risk.

### Gemini CLI

Run from the target workspace. Read-only:

```powershell
gemini -p "<task contract>" -m <model> --approval-mode plan -o json
```

For edits, use the least-permissive mode that can complete the task. `auto_edit` may approve edits;
`yolo` is not a default and requires explicit authorization.

### Claude Code

Run from the target workspace. Read-only:

```powershell
claude -p "<task contract>" --model <model> --permission-mode plan --output-format json
```

For edits, prefer normal/manual permissions or `acceptEdits` only when the user authorized writing.
Never default to `bypassPermissions`.

### OpenCode / Kilo

```powershell
opencode run --dir <workspace> -m <provider/model> --variant <level> "<task contract>"
kilo run --dir <workspace> -m <provider/model> --variant <level> "<task contract>"
```

Do not add `--auto` or dangerous permission bypass flags unless autonomous writes were explicitly
authorized.

### Antigravity / AGY

Run from the target workspace. For a read-only planning/review task:

```powershell
agy --new-project --model <model> --mode plan --sandbox --print "<task contract>"
```

Verify the installed AGY help before changing effort, timeout, or permission flags.

## Parallel Delegation

Parallelize only independent tasks. Record which agent owns which scope before dispatch. A useful
split is implementation, independent review, and focused research/testing, but do not create agents
just to increase agent count.

When direct calls return, inspect their actual output and workspace diff. Agent claims are evidence
to verify, not proof. Run the smallest deterministic checks that establish correctness.

## Handoff Contract

A direct agent should return:

- provider/model actually used if known;
- task status: completed, failed, or blocked;
- files or artifacts changed;
- validation commands and results;
- assumptions, unresolved risks, and any action still requiring approval.

Do not depend on another agent's private logs or hidden context. Put required predecessor conclusions
in the next task prompt or a workspace-owned artifact.

## References

- `../swarms/SKILL.md` - use the SWARMS runtime when durable orchestration is actually required.
- `../../docs/SKILL_INSTALL.md` - distinction and installation of the two public skills.