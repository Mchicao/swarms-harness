# SWARMS Run Observability Contract

A **read-only, versioned** view of a SWARMS run, derived purely from existing
on-disk checkpoints. It is intended for UIs, dashboards, summaries, and agents
that need to inspect run/agent/task state **without** touching the runtime or
spending tokens.

- **Read-only.** The observer never writes and never spawns workers.
- **No web dependencies.** Standard library only (`json`, `pathlib`, `re`,
  `datetime`). No HTTP server, framework, or DB.
- **Derived, not authored.** Every field comes from `workflow.json`,
  `tasks/*.json`, `results/*/result.json`, `claims/*.lock`, `events.jsonl`, or
  `report.json`. There is no second source of truth.
- **Sanitized.** Absolute paths are relativized to the workspace/repo root
  (unknown paths collapse to a basename); error strings are length-capped and
  scrubbed of bearer tokens, API keys, and secret-like env values.

## Schema version

`contract_schema_version` is `1`. Bump it on any breaking shape change;
additive fields may stay on the same version.

## Source

| Field | Origin |
| --- | --- |
| `run` | `workflow.json` + derived `status` |
| `stages[].tasks[]` | `tasks/*.json` (one node per task) |
| `task.agent` | task fields + matching `claims/<task_id>.lock` |
| `task.subagents[]` | plan-derived `subagents` resolved via `agent_id` |
| `summary.result_count` | count of `results/*/result.json` |
| `summary.last_heartbeat_unix_ms` | max `heartbeat_unix_ms` across tasks |
| `summary.report_status` | `report.json` if present |

## Run status derivation

1. If `report.json` exists, use its `status` (`planned`, `completed`, `failed`).
2. No task checkpoints → `empty`.
3. All tasks `completed` → `completed`.
4. Any task `in_progress`/`queued` → `running`.
5. Any task `failed` (none running) → `failed`.
6. Otherwise → `partial` (interrupted/blocked mid-flight).

## Shape

```jsonc
{
  "contract_schema_version": 1,
  "read_only": true,
  "run": {
    "run_id": "obs-completed",
    "runtime": "python",
    "state_schema_version": 1,
    "created_at": "2026-07-16T10:00:00+00:00",
    "status": "completed",
    "workspace_root": "<relativized or null>",
    "tasks_file": "<relativized or null>",
    "workflow_plan": "<relativized or null>",
    "global_max_concurrency": 3,
    "provider_max_concurrency": { "mock": 3 },
    "task_count": 5,
    "observed_at": "2026-07-16T10:01:00+00:00"
  },
  "summary": {
    "stage_count": 3,
    "task_status_counts": { "completed": 5 },
    "has_real_provider": false,
    "last_heartbeat_unix_ms": 1752662460123,
    "result_count": 5,
    "report_status": "completed"
  },
  "stages": [
    {
      "name": "Discovery",
      "status_counts": { "completed": 1 },
      "tasks": [
        {
          "task_id": "0000-reshard-plan",
          "index": 0,
          "role": "planner",
          "source_id": "reshard_plan",
          "status": "completed",
          "attempts": 1,
          "model": "mock-worker",
          "provider": "mock",
          "route": "mock",
          "wrapper": "mock",
          "agent": {
            "agent_id": "reshard_plan",
            "owner": null,
            "claimed_at": null,
            "heartbeat_at": null
          },
          "subagents": [
            { "agent_id": "compress", "task_id": "0001-compress", "status": "completed", "model": "mock-worker" }
          ],
          "provider_subagents": [],
          "provider_subagent_visibility": "not_reported",
          "timestamps": {
            "started_at": "2026-07-16T10:00:01+00:00",
            "ended_at": "2026-07-16T10:00:02+00:00",
            "heartbeat_unix_ms": 1752662402000
          },
          "needs": [],
          "artifacts": ["docs/bench_notes/reshard_plan.md"],
          "error": null
        }
      ]
    }
  ]
}
```

## Nested subagents

A task whose `source_id`/`agent_id` is referenced as a `parent_task_id` by
other tasks exposes those children in `subagents[]`. Each entry is resolved to
its task's `status` and `model` when available, otherwise marked `unknown`.
Provider-reported subagents are surfaced verbatim in `provider_subagents` with
`provider_subagent_visibility` describing whether the provider reported them.

## Usage

```python
from scripts.run_observability import RunObservability, list_runs

contract = RunObservability.from_run("my-run").build_contract()
for run in list_runs():
    print(run["run_id"], run["task_count"])
```

CLI (human / piping into other tools):

```powershell
python -m scripts.run_observability --list
python -m scripts.run_observability --run-id my-run --run-root .agent/swarm/runs
```

## Resilience

- Corrupt `tasks/*.json` or `claims/*.lock` files are skipped, never fatal.
- Missing `workflow.json` degrades gracefully (fields become `null`/defaults).
- A run directory with only `workflow.json` reports `status: "empty"`.
- An interrupted run (no `report.json`, mixed task states) reports `partial`
  or `running` depending on whether any task is still `in_progress`.

## Security

The contract is safe to log or ship to a UI because:

- Absolute paths under the workspace/repo root are made relative; foreign
  absolute paths are reduced to their basename.
- Errors are capped at 1000 chars and scrubbed of `Bearer ...`, `sk-...`,
  `api_key=...`, and `<NAME>_API_KEY=...` patterns.
- No credentials, env values, or worker logs are embedded in the contract.
