# Read-only run state contract v1

The local UI may observe a SWARMS run, but it must never write coordinator
state, claim tasks, launch workers, or mutate plans. Python and Rust publish the
same read-only files under `.agent/swarm/runs/<run_id>/`:

- `workflow.json`: run identity, runtime, workspace, limits and heartbeat interval.
- `tasks/*.json`: current task/agent snapshot, written atomically.
- `events.jsonl`: append-only lifecycle stream.
- `claims/*.lock`: Python compatibility claim ownership; diagnostic only.
- `results/<task_id>/`: prompt, worker log, status and completion checkpoint.
- `report.json` or `report-rs.json`: terminal summary.

## Task snapshot

Consumers may rely on these fields:

```json
{
  "state_schema_version": 1,
  "task_id": "0001-build-api",
  "source_id": "build-api",
  "agent_id": "build-api",
  "parent_task_id": "architecture",
  "subagents": ["build-api-tests"],
  "provider_subagent_visibility": "not_reported",
  "provider_subagents": [],
  "stage": "Implementation",
  "role": "programmer",
  "depth": 1,
  "allow_subagent_spawning": false,
  "needs": ["architecture"],
  "route": "glm52",
  "provider": "opencode",
  "model": "zai-coding-plan/glm-5.2",
  "status": "in_progress",
  "attempts": 2,
  "heartbeat_unix_ms": 1784145600000,
  "error": null
}
```

`agent_id` is the stable plan identity. `parent_task_id` is optional and
references another task's `source_id`; `null` means a root agent. `subagents`
lists direct child `agent_id` values. These fields describe UI nesting only.
`needs` remains the execution DAG and must not be inferred from the visual
hierarchy.

`subagents` contains only children declared in the SWARMS plan. It must never
be populated by guessing what happens inside Codex, GLM, Gemini or another
provider. `provider_subagent_visibility` is `not_reported` when the adapter has
no child-agent event channel, `opaque` when the provider confirms internal
fan-out but hides identifiers, and `reported` only when machine-readable logs
provide explicit child IDs. In the latter case, adapters may append those IDs
to `provider_subagents`; the two child lists remain separate.

`depth` is the validated declared hierarchy depth. Public workflows currently
require `allow_subagent_spawning=false` and `spawn_budget=0`; child tasks are
materialized only by the coordinator from the reviewed plan.

The UI should treat a running task as stale only relative to
`workflow.json.heartbeat_interval_seconds`, and label it as stale rather than
changing its status. Unknown fields must be ignored for forward compatibility.

## Event stream

Each line in `events.jsonl` is independent JSON with `event`,
`time_unix_ms`, and optional `task_id`. The current lifecycle events are
`workflow_initialized`, `workflow_resumed`, `task_started`, `task_heartbeat`,
`task_finished`, and `workflow_finished`. Python may include additional fields
such as the ISO timestamp, model, provider, error or return code.

Readers should tail complete newline-terminated records and retry a snapshot
read if an atomic replacement races with the filesystem watcher. Opening task
details or child-agent panels belongs entirely to the UI process; it must not
signal or foreground worker processes.

The intended observer is a separate, feature-gated native Rust binary so the
coordinator remains lightweight when no UI is requested. This contract does
not require Node, a browser, WebView, HTTP server, UI framework, or new runtime
dependency; those choices stay outside the coordinator until the frontend
brief is approved.

## Resume semantics

`--resume` requires an existing `run_id`. A completed task is skipped only when
its Rust checkpoint matches the current task definition; unfinished, failed or
changed tasks are requeued. Python preserves completed task snapshots and
requeues every non-completed snapshot. `--force` and `--resume` are mutually
exclusive. Codex, OpenCode, and agy workers persist an exact session ID in
their task-local `status.json`. Python and Rust may continue it once while its
timestamp is at most 300 seconds old; they never select a global "last"
session. Corrupt, future-dated, or expired session state is ignored.

```powershell
# SWARMS-RESUME-001: Reanuda checkpoints sin borrar el run existente.
cargo run --release --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --run-id my-run --resume

# SWARMS-RESUME-002: Usa la misma semántica en el runtime Python de compatibilidad.
python scripts/swarm.py run --plan docs/workflow_plan_example.json --run-id my-run --resume
```
