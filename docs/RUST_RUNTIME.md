# Rust runtime

`rust/` contains the self-contained, cross-platform SWARMS coordinator. It runs
on Windows, macOS, and Linux with no Python dependency. All adapter logic —
mock, Codex, OpenCode, Kilo, Hermes, agy, and OpenAI-compatible HTTP — is
implemented natively in Rust.

## Quick start

```bash
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_example.json --force
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

For CI or offline work, use `--offline` after Cargo has built dependencies once.

## Projects

A workflow plan can group every dry-run and execution under a stable project:

```json
"project": {"id": "billing-api", "name": "Billing API"}
```

The runtime copies `project_id` and `project_name` into `workflow.json`. When
the plan omits `project`, it derives one from `workspace_root`. Project metadata
does not affect scheduling, quotas, run paths, or checkpoint identity.

## Architecture

| Module | Responsibility |
|---|---|
| `main.rs` | CLI entry point, doctor, dispatch |
| `cli.rs` | Argument parsing, run-id validation |
| `config.rs` | Router/plan loading, overlay merge, task building |
| `model.rs` | Domain types: Plan, Task, Provider, ThinkingLevel, SessionConfig |
| `quota.rs` | Read-only external quota snapshot and freshness/threshold guard |
| `review.rs` | Static plan validation: DAG, routes, thinking, session, artifacts |
| `runtime.rs` | Scheduler: DAG waves, retries, progreso observable, resume, verify, artifacts |

## Long-running workers

El runtime Rust no aplica deadlines ni mata workers por tiempo. Los campos
históricos `default_timeout_seconds` y `timeout_seconds` se aceptan para leer
planes anteriores, pero no se ejecutan. Cada heartbeat observa el tamaño y la
modificación de `worker.log`; la UI marca una tarea `stale` si no hay progreso,
sin cambiar su estado ni cancelar el proceso.

En Windows, los workers reales abren por defecto una consola de sólo lectura
que sigue `worker.log` mientras el coordinador continúa en segundo plano. Usa
`$env:SWARMS_WORKER_CONSOLES = "hidden"` para suprimir esas ventanas.

### Herd terminal backend

Set `$env:SWARMS_TERMINAL_BACKEND = "herdr"` before `swarms-rs run` to show
real-worker logs in persistent Herd panes instead of separate PowerShell
windows. SWARMS still owns worker processes, retries, quotas and raw logs;
Herd is the observable terminal surface. The task snapshot records its Herd
session and pane ID, which `swarms-ui` displays. If Herd is unavailable, the
runtime falls back to the native Windows console. Set `SWARMS_HERDR_BIN` or
`SWARMS_HERDR_SESSION` only when the default executable/session must change.
| `adapter.rs` | Native adapters: mock, CLI command builders, OpenAI-compat HTTP, session/usage parsing |
| `session.rs` | Session affinity store: persist, validate, reuse, lock |
| `telemetry.rs` | Usage normalisation, task state, report generation |

## Thinking levels

Per-task `thinking` controls reasoning depth. Only verified adapter flags are used:

| Level | Codex | OpenCode/Kilo | Hermes | agy | OpenAI-compat |
|---|---|---|---|---|---|
| `auto` | (default) | (default) | n/a | n/a | n/a |
| `minimal` | `model_reasoning_effort=minimal` | `--variant minimal` | not supported | not supported | via `thinking_field` |
| `low` | `model_reasoning_effort=low` | `--variant low` | not supported | not supported | via `thinking_field` |
| `medium` | `model_reasoning_effort=medium` | `--variant medium` | not supported | not supported | via `thinking_field` |
| `high` | `model_reasoning_effort=high` | `--variant high` | not supported | not supported | via `thinking_field` |
| `max` | `model_reasoning_effort=ultra` | `--variant max` | not supported | not supported | via `thinking_field` |

Review fails with `thinking_not_supported` if a non-default level is set on an
adapter without a verified flag.

## Session affinity

Tasks can reuse provider sessions to leverage prompt caching:

```json
{
  "id": "investigate",
  "route": "codex",
  "session": { "mode": "new", "key": "auth-workstream" }
}
```

```json
{
  "id": "implement",
  "route": "codex",
  "needs": ["investigate"],
  "session": { "mode": "reuse", "key": "auth-workstream", "on_missing": "new" }
}
```

- `mode: disabled` (default) — no session reuse.
- `mode: new` — start a new session, capture the ID from adapter output.
- `mode: reuse` — resume a prior session by key. Validated: route, model,
  adapter, and workspace must match. `on_missing: new` or `fail`.

Session reuse is only supported for adapters that expose structured session IDs
in their output: Codex (`thread_id` in JSONL), OpenCode/Kilo (`sessionID` in
JSON events). Hermes and agy do not expose reliably parseable session IDs in
headless mode; review rejects `mode: reuse` for those adapters.

Same-key tasks are serialised by the scheduler to prevent concurrent
continuation of a single conversation.

## Quota-aware routing

The optional quota guard reads the atomic snapshot written by
`ai-usage-monitor` before scheduling a run:

```json
{
  "quota_policy": {
    "enabled": true,
    "snapshot_path": "../ai-usage-monitor/quota_snapshot.json",
    "min_remaining_percent": 10,
    "max_age_seconds": 600,
    "on_unknown": "block"
  },
  "providers": {
    "codex": {
      "quota_key": "codex:Codex",
      "fallback_routes": ["glm52", "gemini_flash"]
    }
  }
}
```

Snapshot contract: `generated_at_epoch` plus `quotas.<key>.windows`, where
each window value is remaining percent. Only known windows are emitted; a
missing weekly limit is not treated as zero. `on_unknown: block` prevents use
when a managed key is absent or the snapshot is stale; `allow` preserves the
route. Codex accounts use separate keys (`codex:Codex` and `codex:Hermes`);
the standard `codex` and `hermes` routes respectively consume those keys.

Fallbacks are tried in order and also spill across routes whose concurrency
cap is already full. Reports preserve the plan's `route` and record the
selected `effective_route`. Stages with `parallel: false` run one task per wave.

## Prompt caching

Prompts use a byte-stable prefix (`PROMPT_PREFIX`) before dynamic content
(role, task, artifacts, dependency outputs). This maximises provider cache hits
even when session reuse is not active. The prefix gives every worker a compact
`Ponytail/full` policy: prefer the smallest correct solution and existing or
native tools without weakening validation, security, or error handling. The
runtime does not embed the full skill text, avoiding a repeated token cost. If
the workspace has a local `.codegraph/` index and the selected adapter exposes
tools, the same prefix asks the worker to prefer CodeGraph for exploration and
impact analysis; tool-less adapters fall back normally.

## Resume

A run can be resumed only with `--resume` and an existing run id. Completed
tasks are preserved only when their checkpoint hash still matches the complete
task definition; changed, interrupted, failed, or blocked tasks are requeued.
Without `--resume` or `--force`, an existing run id is rejected.

## Token telemetry

- Mock: `0` for all fields.
- OpenAI-compat: parsed from `usage` in HTTP response (`prompt_tokens`,
  `cached_tokens`, `cache_creation_input_tokens`, `completion_tokens`,
  `reasoning_tokens`).
- CLI adapters (Codex/OpenCode/Kilo): parsed from JSONL output
  (`part.tokens` or `usage` events). If no structured usage is found, fields
  are `"missing"` — never fabricated zeros.
- Hermes/agy: `"missing"` (no structured usage in headless mode).

## Python compatibility

Python scripts (`scripts/swarm.py`, `scripts/workflow_runtime.py`, etc.) remain
as legacy benchmark and telemetry tools. No Rust code invokes Python. The
public runtime path is exclusively Rust.

## Native observer UI

The optional `swarms-ui` binary reads the Rust runtime state contract without
starting workers or changing workflow state:

```powershell
cargo run --release --manifest-path rust/Cargo.toml --bin swarms-ui --features ui-egui -- --run-id <run-id>
```

It uses Glow without WGPU, caches the flattened tree, caps events at 500 and the
selected log at 256 KiB with virtualized lines, and polls filesystem metadata at 1 Hz while active or
0.2 Hz while idle. See `docs/SWARM_UI.md`.
