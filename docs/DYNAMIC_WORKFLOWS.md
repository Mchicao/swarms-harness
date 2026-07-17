# Dynamic Workflow Runtime

SWARMS supports an UltraCode-style runtime without copying Claude Code's cost profile.

The runtime keeps orchestration state on disk and lets the harness execute the plan deterministically. The model can propose or edit a workflow, but the runtime owns dependency resolution, task claiming, concurrency limits, summaries, telemetry, and final reporting. Interrupted runs can be resumed from completed task checkpoints. Codex, OpenCode, and Kilo sessions emitted before a failed CLI exit are resumed once within a bounded five-minute recovery window (configurable with `SWARMS_SESSION_RESUME_WINDOW_SECONDS`).

Schema v2 plans are compiled natively by Rust into the ordinary deterministic DAG before review or execution. Supported bounded steps are `agent`, `map`, `reduce`, `verify`, `condition`, and `loop`. Map items and conditions are literal, loops are expanded statically, and recursive runtime spawning remains locked off with `spawn_budget: 0`.

## GPT-5.6 Ultra-Style Runtime

OpenAI describes GPT-5.6 `ultra` as a mode that uses subagents for complex work. SWARMS is the local-first version of that pattern: the user owns the plan, routing, provider caps, verification metadata, and token budget. Deterministic `verify` commands remain metadata today; callers must run them separately.

This makes SWARMS useful when a user wants Ultra-style fan-out but needs:

- local repo state and reports;
- provider choice across configured OpenAI-compatible APIs, GLM, Gemini, Codex CLI, Hermes, and offline mock workers;
- explicit premium permissions;
- caps per provider;
- Singularity loops for ongoing QA, issue triage, and improvement proposals.

## Why This Exists

Large fan-out should not mean that the orchestrator model carries every worker log in its context. SWARMS stores intermediate state under `.agent/swarm/runs/<run_id>/`:

- `workflow.json` - immutable run metadata and limits.
- `tasks/*.json` - task state, dependencies, role, provider, attempts.
- `claims/*.lock` - atomic task claims with stale-claim recovery.
- `results/<task_id>/` - worker prompts, logs, and result JSON.
- `events.jsonl` - append-only lifecycle events.
- `report.json` - final summary.

The stable read-only contract for a local observer UI, including nested
`parent_task_id`, `agent_id`, `subagents`, heartbeat and event fields, is documented in
`docs/STATE_CONTRACT.md`.

## Scale Model

The target is many logical workers with bounded live concurrency:

```json
{
  "max_total_workers": 1000,
  "global_max_concurrency": 8,
  "provider_concurrency": {
    "mock": 64,
    "glm52": 6,
    "gemini_flash": 3,
    "codex": 1,
    "claude": 0
  }
}
```

This is intentionally different from launching 100 expensive CLIs at once. The workflow may contain hundreds of tasks, but only the provider pools with available budget are allowed to run concurrently.

## Singularity Loop

Singularity is the long-running mode for users who want local agents working while they are away:

- read issues and local notes;
- propose improvements;
- create or update workflow plans;
- launch worker pools;
- run QA and validation;
- summarize state for the next cycle.

This mode makes sense for users with large token budgets, local models, or provider plans they are willing to spend. It should still use provider caps and stop conditions. A 24/7 loop without caps can burn through API credit or subscription quota quickly.

## Commands

Plan without running workers:

```powershell
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_dynamic_example.json --force
```

Run the deterministic mock workflow:

```powershell
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_dynamic_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Resume the same run without redoing completed tasks:

```powershell
# SWARMS-RESUME-003: reusa checkpoints de tareas terminadas.
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_dynamic_example.json --run-id my-run --resume --provider-cap mock=3
```

The default provider is `mock`, so these commands do not spend model quota.

## What To Copy From Claude Code UltraCode

- Move orchestration into code, not the chat context.
- Keep intermediate results in runtime state.
- Cap concurrent agents while allowing many total agents per run.
- Expose progress, stop/restart, and final reports.
- Make workflows reviewable before execution.

## What SWARMS Adds

- Provider pools centered on saving expensive quota.
- Premium providers disabled by default.
- Context packs and scope guards before worker launch.
- Token/cost telemetry as first-class benchmark output.
