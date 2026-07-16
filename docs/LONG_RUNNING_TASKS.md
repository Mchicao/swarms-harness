# Long-running tasks: checkpoints, leases, and resume

SWARMS tasks can run for minutes (model workers) or longer. This document
describes how the runtime keeps long-running work **safe to interrupt**,
**cheap to resume**, and **free of double-execution** without introducing a
new coordination dependency.

Python remains the control plane for claim lifecycle, checkpoint reuse, and
resume bookkeeping. Rust is the alternative low-overhead coordinator for the
same plan; it shares the run-directory layout and the resume contract but does
not add a separate coordination service.

## TL;DR for operators

```powershell
# Start a run (crashes or Ctrl-C are safe).
python scripts/swarm.py run --plan docs/workflow_plan_example.json `
  --run-id my-run --force --global-max-concurrency 3 --provider-cap mock=3

# Resume after a crash: completed tasks are skipped, crashed tasks are requeued.
python scripts/swarm.py run --plan docs/workflow_plan_example.json `
  --run-id my-run --resume

# Same contract via the Rust coordinator.
cargo run --release --manifest-path rust/Cargo.toml -- `
  run --plan docs/workflow_plan_example.json --run-id my-run --resume
```

`--force` and `--resume` are mutually exclusive. `--force` wipes the run
directory and starts over; `--resume` preserves completed work.

## Idempotent checkpoints

Every task has a **checkpoint key**: a stable FNV-1a hash of its full
definition (`task_id`, `source_id`, `stage`, `route`, `text`, `role`, `needs`,
`artifacts`, `tools_policy`, `provider`, `model`, `variant`, `wrapper`).

When a worker finishes, the runtime writes `checkpoint_key` into
`results/<task_id>/result.json` (Python) or `result-rs.json` (Rust).

On the next `run_task` call for the same task, the runtime checks whether a
completed `result.json` exists **whose checkpoint key still matches the current
definition**. If it does, the worker is **not re-invoked** — the stored result
is reused and a `task_checkpoint_hit` event is emitted.

This means:

| Scenario | Outcome |
|---|---|
| Worker finished, runtime crashed before saving `completed` state | `--resume` reuses the result; worker is **not** re-run. |
| Task definition unchanged, result on disk | Checkpoint hit; zero worker cost. |
| Task definition changed (edited plan, different model, etc.) | Checkpoint key mismatch; task is re-run. |
| `--force` | Run directory is wiped; no checkpoints survive. |

Checkpoints are **runtime-specific**: Python reads `result.json`, Rust reads
`result-rs.json`. Switching coordinators mid-run does not inherit the other
runtime's checkpoints — use `--force` for a clean start if you switch.

## Leases and heartbeats

Each task is guarded by a **file-based claim** (`claims/<task_id>.lock`) before
a worker is dispatched. The claim prevents two workers from executing the same
task concurrently within a run.

| Mechanism | What it does |
|---|---|
| `try_claim` | Atomically creates the lock (`O_CREAT \| O_EXCL`). Returns `false` if already owned. |
| `heartbeat` | A background thread updates `heartbeat_at` in the lock file every `SWARMS_HEARTBEAT_SECONDS` (default 30 s) and refreshes `tasks/<task_id>.json.heartbeat_unix_ms`. |
| `stale claim` | If `time.now - lock.mtime > SWARMS_CLAIM_STALE_SECONDS` (default 900 s), the claim is considered expired and a new `try_claim` reclaims it. |
| `release` | Removes the lock when the worker finishes (success or failure). |

### Tuning

```powershell
# Faster heartbeat for short tasks (more I/O, faster crash detection).
$env:SWARMS_HEARTBEAT_SECONDS = "10"

# Shorter stale window for aggressive claim recovery.
$env:SWARMS_CLAIM_STALE_SECONDS = "120"

# Or via the CLI flag (Python only).
python scripts/swarm.py run ... --claim-stale-seconds 120
```

Do not set `SWARMS_HEARTBEAT_SECONDS` higher than `SWARMS_CLAIM_STALE_SECONDS`,
or a healthy worker's claim could be stolen mid-flight.

## Resume after restart

`--resume` requires an existing `run_id`. It performs three actions in order:

1. **Sweep expired claims.** `ClaimStore.recover_expired()` removes every claim
   whose heartbeat has been silent past `stale_seconds`. The count is emitted
   in the `workflow_resumed` event as `claims_recovered`.

2. **Force-release orphaned claims.** For every non-completed task, the runtime
   calls `force_release(task_id)` — the previous owning process is dead, so its
   claims are all orphaned regardless of staleness. This is what makes resume
   immediate instead of waiting for `stale_seconds` to elapse.

3. **Requeue non-completed tasks.** Tasks that were `pending`, `queued`,
   `in_progress`, `failed`, or `blocked` become `pending` again. Their
   `started_at`, `ended_at`, and `error` are cleared; `attempts` is preserved.

Completed tasks are left untouched. When the scheduler reaches a requeued task,
`run_task` checks for a valid checkpoint (see above) before dispatching a
worker — so a task whose worker finished but whose state was never marked
`completed` is recovered for free.

### Event trail

Resume adds these events to `events.jsonl`:

```
workflow_resumed   { completed: N, claims_recovered: M }
task_requeued      { task_id: ..., attempts: K }
task_checkpoint_hit { task_id: ..., checkpoint_key: ... }   (if reused)
task_recovered      { task_id: ... }                         (Rust only)
```

## Expired-claim recovery (mid-run)

Even without a full restart, a worker that crashes (OOM, network death) leaves
its claim behind. Two recovery paths exist:

1. **Lazy.** When the scheduler dispatches the task again, `try_claim` checks
   the lock's mtime; if it exceeds `stale_seconds`, the lock is reclaimed
   in-place.

2. **Active sweep.** `ClaimStore.recover_expired()` iterates `claims/*.lock`
   and removes every expired one, returning the count. This is called
   automatically on `--resume`. It can also be invoked programmatically:

```python
from scripts.workflow_runtime import ClaimStore
recovered = ClaimStore(run_dir / "claims", stale_seconds=300).recover_expired()
```

## Crash safety checklist

Before treating a run as done, verify:

1. `report.json` (Python) or `report-rs.json` (Rust) exists and `status` is
   `completed`.
2. No `claims/*.lock` files remain (all claims were released).
3. Every task in `tasks/*.json` has `status: completed`.
4. `events.jsonl` ends with a `workflow_finished` line.

If any of these fail, use `--resume` to recover.

## Python vs Rust: who does what

| Concern | Python (`scripts/workflow_runtime.py`) | Rust (`rust/src/main.rs`) |
|---|---|---|
| Plan parsing & task DAG | Control plane | Control plane |
| Claim locks (file-based) | Yes — inter-worker | No — in-process threads |
| Heartbeat | Background thread per task | Poll loop per worker |
| Checkpoint key | `checkpoint_key()` (FNV-1a) | `checkpoint_key()` (FNV-1a) |
| Checkpoint reuse on resume | `load_completed_checkpoint` | `load_completed_checkpoint` |
| Expired-claim recovery | `recover_expired` + `force_release` | N/A (no cross-process claims) |
| Resume event | `workflow_resumed` + `task_checkpoint_hit` | `task_recovered` |

Rust is used **only** when its in-process thread model improves a coordination
boundary that has been measured as a bottleneck. It does not introduce a new
coordination service, claim store, or external dependency. For multi-process
or cross-machine coordination, Python's file-based claim store remains the
source of truth.

## File layout reference

```
.agent/swarm/runs/<run_id>/
  workflow.json          run identity, limits, heartbeat interval
  tasks/<task_id>.json   per-task snapshot (status, attempts, heartbeat)
  claims/<task_id>.lock  active lease (Python only)
  results/<task_id>/
    result.json          Python checkpoint + result (checkpoint_key, success)
    result-rs.json       Rust checkpoint + result
    worker.log           worker stdout+stderr
    prompt.txt           prompt sent to the worker
  events.jsonl           append-only lifecycle stream
  report.json            terminal summary (Python)
  report-rs.json         terminal summary (Rust)
```
