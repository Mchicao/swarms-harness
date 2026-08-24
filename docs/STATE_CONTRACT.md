# Run state and control contract v1

Read-only observers can inspect a SWARMS run, while explicit steering clients may append user prompts;
consumers never write coordinator snapshots, claim tasks, launch workers, or mutate
plans. Python and Rust publish the same observed files under `.agent/swarm/runs/<run_id>/`:

- `workflow.json`: run identity, project, runtime, workspace, limits and heartbeat interval.
- `tasks/*.json`: current task/agent snapshot, written atomically.
- `events.jsonl`: append-only lifecycle stream.
- `claims/*.lock`: Python compatibility claim ownership; diagnostic only.
- `results/<task_id>/`: prompt, worker log, status and completion checkpoint.
- `report.json` or `report-rs.json`: terminal summary.
- `steering/<task_id>/inbox.jsonl`: user prompts claimed by the Rust runtime.
- `steering/<task_id>/history.jsonl`: applied/rejected/failed steering audit.

Every new Rust run writes `project_id` and `project_name` in `workflow.json`.
Plans may declare them with `"project": {"id": "stable-id", "name": "Display name"}`.
When omitted, the runtime derives a stable workspace project. Historical runs
without either field remain readable and appear under `Legacy runs`. Projects
are metadata only: run paths stay `.agent/swarm/runs/<run_id>` and resume never
moves an existing run between projects.

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
  "needs": ["architecture"],
  "route": "glm52",
  "provider": "opencode",
  "model": "zai-coding-plan/glm-5.2",
  "status": "in_progress",
  "attempts": 2,
  "heartbeat_unix_ms": 1784145600000,
  "worker_log_bytes": 1248,
  "last_progress_unix_ms": 1784145600000,
  "error": null
}
```

`agent_id` is the stable plan identity. `parent_task_id` is optional and
references another task's `source_id`; `null` means a root agent. `subagents`
lists direct child `agent_id` values. These fields describe presentation nesting only.
`needs` remains the execution DAG and must not be inferred from the visual
hierarchy.

`subagents` contains only children declared in the SWARMS plan. It must never
be populated by guessing what happens inside Codex, GLM, Gemini or another
provider. `provider_subagent_visibility` is `not_reported` when the adapter has
no child-agent event channel, `opaque` when the provider confirms internal
fan-out but hides identifiers, and `reported` only when machine-readable logs
provide explicit child IDs. In the latter case, adapters may append those IDs
to `provider_subagents`; the two child lists remain separate.

Observers may treat a running task as stale when `last_progress_unix_ms` is
older than `workflow.json.heartbeat_interval_seconds`; it falls back to the
coordinator heartbeat for historical runs. `stale` is visual only and never
cancels or changes the task status. Unknown fields must be ignored for forward
compatibility.

## Event stream

Each line in `events.jsonl` is independent JSON with `event`,
`time_unix_ms`, and optional `task_id`. The current lifecycle events are
`workflow_initialized`, `workflow_resumed`, `task_started`, `task_heartbeat`,
`tasks_heartbeat`,
`task_finished`, and `workflow_finished`. Python may include additional fields
such as the ISO timestamp, model, provider, error or return code.

Readers should tail complete newline-terminated records and retry a snapshot
read if an atomic replacement races with the filesystem watcher. Read-only consumers must not signal, foreground, or otherwise control worker
processes when presenting task details or child-agent data.

## Steering mailbox

Each inbox line is UTF-8 JSON with `id`, `created_at_epoch_ms`, `prompt` and
`source`. Task IDs use safe path-component rules and prompts contain 1–4000
characters. The runtime atomically renames the inbox before reading it. A
delivered instruction becomes a subsequent provider turn, never stdin
injection into the current CLI process. Unsupported or missing sessions are
recorded as `rejected` without falsifying delivery.

The state contract is UI-framework agnostic. Observers may be native, terminal,
web, or external tools without adding a dependency to the coordinator.

## Resume semantics

`--resume` requires an existing `run_id`. A completed task is skipped only when
its Rust checkpoint matches the current task definition; unfinished, failed or
changed tasks are requeued. Python preserves completed task snapshots and
requeues every non-completed snapshot. `--force` and `--resume` are mutually
exclusive. Automatic retries remain out of scope.

```powershell
# SWARMS-RESUME-001: Reanuda checkpoints sin borrar el run existente.
cargo run --release --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --run-id my-run --resume

# SWARMS-RESUME-002: Usa la misma semántica en el runtime Python de compatibilidad.
python scripts/swarm.py run --plan docs/workflow_plan_example.json --run-id my-run --resume
```
