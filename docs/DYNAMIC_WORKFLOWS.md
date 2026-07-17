# Dynamic Workflow Runtime

SWARMS supports an UltraCode-style runtime without copying Claude Code's cost profile.

The runtime keeps orchestration state on disk and lets the harness execute the plan deterministically. The model can propose or edit a workflow, but the runtime owns dependency resolution, task claiming, concurrency limits, summaries, telemetry, and final reporting. Interrupted runs resume from completed task checkpoints and supported provider sessions may be resumed once within five minutes.

Schema version 2 adds a small declarative Workflow IR. `agent`, literal `map`,
`reduce`, verifier-agent `verify`, boolean `condition`, and fixed-round `loop`
steps compile into stable version-1 tasks before dispatch. The compiler does
not execute generated JavaScript or accept result-derived task injection. That
keeps checkpoints deterministic and closes the recursive-spawn failure mode.

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

The safe default is 12 logical workers with bounded live concurrency. A user
may explicitly raise the flat worker ceiling, while depth, direct children,
rounds, provider concurrency, and total workers remain hard plan limits:

```json
{
  "max_total_workers": 1000,
  "max_depth": 2,
  "max_children_per_agent": 4,
  "max_rounds": 4,
  "spawn_budget": 0,
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
python scripts/swarm.py dry-run --plan docs/workflow_plan_example.json --force
```

Review and expand the bounded Workflow IR through the public Rust coordinator:

```powershell
# SWARMS-DYNAMIC-001: Compila el IR declarativo antes del dispatch Rust.
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_dynamic_example.json
```

Every public Python and Rust worker prompt carries a policy before task and
dependency content that forbids spawning, delegation, and recursive agent
trees. `spawn_budget` must remain zero and
`allow_subagent_spawning=true` is rejected because provider-internal fan-out
cannot yet be counted reliably.

## Coordinated agent context

`--sync-agent-context` is opt-in. It previews and then synchronizes configured
Skillshare skills, followed by project-scoped Rulesync generation for rules,
AGENTS files, subagents, skills, and MCP:

```powershell
# SWARMS-CONTEXT-003: Sincroniza contexto sólo cuando se solicita explícitamente.
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_dynamic_example.json --sync-agent-context --context-sync-targets claude,codex,opencode,agy
```

The canonical source is `.rulesync/`. MCP credentials must use environment
references such as `${GITHUB_TOKEN}`; literal secrets and credential-bearing
URLs are rejected. Rulesync output is limited to the selected workspace.
Skillshare uses its configured target set because its CLI does not provide a
per-target filter.

Run the deterministic mock workflow:

```powershell
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Resume the same run without redoing completed tasks:

```powershell
# SWARMS-RESUME-003: reusa checkpoints de tareas terminadas.
python scripts/swarm.py run --plan docs/workflow_plan_example.json --run-id my-run --resume --provider-cap mock=3
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
