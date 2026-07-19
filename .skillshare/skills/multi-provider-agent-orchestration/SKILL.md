---
name: multi-provider-agent-orchestration
description: Delegate engineering work and plans across Codex CLI, OpenCode, Gemini, GLM and similar agents. Use when splitting a goal among agents, selecting providers by task shape, defining safe handoffs, coordinating parallel work, or integrating independently produced changes.
---

# Multi-provider agent orchestration

Use this skill to design delegation. It is runtime-agnostic: it does not own a
specific scheduler, router format, UI or provider credentials. For operations
inside this repository, use `$swarms` after the delegation contract is ready.

## Make a delegation contract

For every delegated task, state:

- objective and acceptance criteria;
- repository, base branch/worktree and allowed writable paths;
- provider, exact model and requested reasoning level;
- dependencies and inputs from earlier tasks;
- required artifacts and deterministic verification commands;
- handoff format: changed files, commands run, outcome, remaining risks.

Give one coherent ownership area to each writer. Use separate worktrees for
parallel edits; if that is not available, assign disjoint paths or serialize
with dependencies. Never claim isolation merely because workers have separate
logs or sessions.

## Select and prove providers

Assign by task shape, availability and quota rather than provider prestige:

- Fast/cheap agents: inventory, coverage review, documentation and narrow UI
  checks.
- Deep reasoning agents: difficult diagnosis, multi-file implementation and
  security-sensitive logic.
- Premium agents: architecture critique, integration review and exceptional
  risk; use only when authorized and quota is available.
- Local tools: tests, builds, linters, migrations and acceptance gates.

Before delegation, verify the local CLI and its exact flags with `--version`
and `--help`. For Codex, inspect `codex debug models`, forward the exact model
to `codex exec`, set `model_reasoning_effort` only to a locally supported value,
and capture JSONL/session evidence. Do not silently substitute a provider if a
requested route is blocked; record the blocker and continue independent work.

## Keep long work observable

Bound the task scope, not the worker with a blind deadline. Use log/output
activity, checkpoints and explicit status to identify a stuck agent. When work
is silent, inspect evidence, steer or stop it deliberately; never report that
elapsed time alone proves a model failure.

## Integrate through gates

Establish the common base and inspect ancestry before combining branches. Review
each result against its contract, then run targeted checks followed by the full
build/lint/test gates. Inspect conflict markers, `git diff --check` and working
tree state before accepting a change.

Report which provider/model actually ran each task, completed/failed/blocked
state, real validation output and unresolved risks. Keep credentials out of
commands, logs, plans and commits.
