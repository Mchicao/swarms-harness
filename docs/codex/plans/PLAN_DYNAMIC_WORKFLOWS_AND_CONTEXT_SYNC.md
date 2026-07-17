# Add bounded dynamic workflows and coordinated agent context

This ExecPlan is a living document. The sections `Progress`, `Surprises &
Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to
date as work proceeds.

## Purpose / Big Picture

SWARMS will let either the user or a planner describe orchestration while the
runtime enforces non-negotiable limits. A workflow may fan work out, repeat a
bounded step, verify results, and reduce many results into one task. Every
worker receives an explicit instruction not to create an excessive recursive
tree. An optional CLI flag synchronizes skills with Skillshare and generates
shared rules, AGENTS files, subagents, and MCP configuration with Rulesync
before dispatch.

The observable outcome is a dry-run that expands a schema-version-2 workflow
into deterministic tasks without exceeding its agent budget, plus a context
sync preflight that reports exactly which noninteractive tools and targets it
used.

## Progress

- [x] (2026-07-17 02:05Z) Reconciled the current PR branch, runtime, plan
  reviewer, installed Skillshare 0.20.21, and Rulesync 8.16.0.
- [x] (2026-07-17 02:52Z) Added red tests for workflow expansion, recursion limits, prompt policy,
  and context-sync command construction.
- [x] (2026-07-17 02:52Z) Implemented the bounded Workflow IR compiler and integrated the Python runtime.
- [x] (2026-07-17 02:52Z) Mirrored hard recursion limits, prompt policy, and Python-backed IR expansion in the Rust coordinator.
- [x] (2026-07-17 02:52Z) Added opt-in coordinated-context preview/apply flow and a credential-free canonical Rulesync source.
- [ ] Add an explicit Rust CI job, confirm it passes, and leave draft PR #4 updated at the verified head.

## Surprises & Discoveries

- Observation: the current Python and Rust runtimes materialize the task list
  once before scheduling; neither currently accepts runtime task insertion.
- Observation: Rulesync already supports canonical rules, MCP, subagents, and
  skills for `agentsmd`, `claudecode`, `codexcli`, `geminicli`, and `opencode`.
- Observation: Skillshare is globally configured for Claude, Codex, Gemini,
  OpenCode, Antigravity, and a universal target, but has no configured agents.
- Observation: Skillshare has no per-target sync filter. The flag's target list
  scopes Rulesync, while Skillshare uses its configured target set.
- Observation: the local MSVC Rust toolchain cannot link because `link.exe` is
  absent. Formatting passes; compilation must be confirmed by GitHub CI.

## Decision Log

- Decision: implement a small declarative Workflow IR rather than execute
  arbitrary JavaScript.
  Rationale: it provides fan-out, bounded repetition, reduction, conditions,
  and verification without introducing a code-execution trust boundary.
  Date/Author: 2026-07-17 / Codex.
- Decision: keep user budgets as hard caps and treat planner scale requests as
  advice only.
  Rationale: a model must never override `max_total_workers`,
  `max_children_per_agent`, or `max_depth`.
  Date/Author: 2026-07-17 / Codex.
- Decision: use Skillshare for skills and Rulesync as the canonical generator
  for rules, AGENTS.md, subagents, and MCP.
  Rationale: both are installed and Rulesync already translates MCP formats;
  adding another synchronization dependency would duplicate functionality.
  Date/Author: 2026-07-17 / Codex.

## Outcomes & Retrospective

The schema-version-2 example expands to seven deterministic tasks and completed
an offline Python mock run. The version-1 example also completed. Ruff,
formatting, and all 159 Python tests pass. Rulesync and Skillshare previews are
clean. Rust formatting passes, while local compilation is externally blocked
by the missing Windows linker; GitHub CI remains the required independent Rust
verification before closing the plan. The existing GitHub jobs passed on
`10c80e2`, but inspection showed they covered Python only. An explicit Rust
format/test/clippy/mock job is therefore required before closure.

## Context and Orientation

`scripts/plan_review.py` validates plans before execution.
`scripts/workflow_runtime.py` materializes Python compatibility tasks and
schedules them. `rust/src/main.rs` is the public coordinator. `scripts/swarm.py`
owns the user-facing CLI. A Workflow IR is a declarative list of steps that the
runtime compiles into ordinary tasks before dispatch; this keeps the existing
checkpoint and scheduling machinery intact.

## Plan of Work

Add schema version 2 without breaking schema version 1. Version 2 may contain
normal stages and a `workflow.steps` list. Step kinds compile as follows:
`agent` creates one task; `map` creates one task per literal item; `reduce`
creates a task depending on every task produced by named input steps; `verify`
is an agent with verifier role; `condition` includes or skips a nested step
using an explicit boolean; and `loop` repeats a nested step up to
`max_rounds`. Expansion stops before it can violate any hard cap.

Extend every task with `depth` and enforce parent depth, child count, total
workers, and a per-task `allow_subagent_spawning` switch. Add a standard prompt
paragraph stating that workers must not create subagents unless explicitly
allowed and must remain within the declared remaining budget.

Add `--sync-agent-context` and `--context-sync-targets`. The operation first
runs Skillshare in structured noninteractive mode, then requires a project
`.rulesync` source and invokes Rulesync for rules, MCP, subagents, and skills.
It records commands, versions, targets, and hashes but never environment values
or generated credential material.

## Concrete Steps

From `C:\Proyectos\SWARMS`, add focused tests and run them once red. Implement
the compiler, policy checks, prompt propagation, and context sync adapter. Run
`uv run ruff check .`, `uv run ruff format --check .`, and
`uv run pytest tests -q --basetemp .cache/pytest-dynamic-workflows`. From the
Rust directory run `cargo fmt --check`, followed by check/test/clippy if the
Windows linker is available. Finish with Python and Rust offline mock runs.

## Validation and Acceptance

A version-2 fixture must expand deterministically and expose map, reduce,
verify, condition, and loop tasks in dry-run output. Tests must show rejection
at depth, child, total-agent, and loop limits. Every generated prompt must carry
the anti-recursion policy. Context sync must be off by default, reject missing
canonical sources, construct only allowlisted commands, and never serialize
environment values. Existing version-1 plans and checkpoints must still pass.

## Idempotence and Recovery

Workflow expansion is deterministic, so resuming recompiles the same task IDs
and checkpoint keys. Context synchronization is explicit and idempotent;
Skillshare and Rulesync merge or regenerate their own targets. A failed sync
stops before provider dispatch. `--force` still clears only the selected run;
it does not delete synchronization sources or user configuration.

## Artifacts and Notes

- `docs/codex/plans/PLAN_DYNAMIC_WORKFLOWS_AND_CONTEXT_SYNC.md`
- `docs/workflow_plan_dynamic_example.json`
- `scripts/workflow_ir.py`
- `scripts/context_sync.py`
- `scripts/plan_review.py`
- `scripts/workflow_runtime.py`
- `scripts/swarm.py`
- `rust/src/main.rs`

## Interfaces and Dependencies

The implementation uses only Python and Rust standard libraries plus the
already installed `skillshare` and `rulesync` executables. The public Python
interface adds `compile_plan(plan)`, `WorkflowLimits`, and
`sync_agent_context(workspace, targets)`. The CLI adds
`--sync-agent-context` and `--context-sync-targets`. No credential values may
cross these interfaces.

## Plan Revision Notes

- 2026-07-17: initial plan created after reconciling Ultracode-style dynamic
  workflows with the current deterministic SWARMS architecture.
