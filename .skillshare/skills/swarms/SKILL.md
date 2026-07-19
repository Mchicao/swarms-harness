---
name: swarms
description: Operate and extend the native Rust SWARMS runtime and observer UI. Use when creating a SWARMS workflow contract, configuring routes and quotas, reviewing or running a plan, resuming a run, observing worker progress, or troubleshooting SWARMS in this repository.
---

# SWARMS

Read `AGENTS.md` first. It is the project contract; the Rust binary is the
only public workflow runtime. Python scripts are legacy telemetry and
benchmark tools, not the default execution path.

## Create a workflow contract

Create a plan JSON with a goal, optional project identity, budget policy and
ordered stages. Every task must declare an id, role, route, bounded scope,
dependencies, allowed artifacts, tools policy and deterministic verification.

- Use `needs` for execution dependencies; do not infer dependencies from UI
  nesting or task names.
- Make concurrent writers use separate worktrees or disjoint writable paths.
  SWARMS isolates its own prompts, logs and results, not a target repo worktree
  per task.
- Keep provider concurrency explicit and within the plan/router limits.
- Route names must exist in `config/swarm_router.json` plus ignored local
  overrides. Never place credentials in a plan, report or commit.
- Treat historical timeout fields as read compatibility only. They do not stop
  Rust workers. Split broad work by ownership and observe real progress instead.

For Codex tasks, set the router model and task `thinking`; the adapter invokes
`codex exec` and persists JSONL/session evidence. Before a new real-provider
route, verify the local CLI with `codex debug models` and `codex exec --help`.

## Run safely

Use this lifecycle from the repository root:

```powershell
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- review --plan <plan.json>
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan <plan.json> --force
cargo run --manifest-path rust/Cargo.toml -- run --plan <plan.json> --force --run-id <run-id> --global-max-concurrency <n> --provider-cap mock=<n>
```

Use `mock` unless the user explicitly authorizes real configured providers.
Start exactly one coordinator run for a workspace. Treat `run` as blocking; do
not poll its artifacts from the coordinator while it is active.

Use `--resume` with an existing run id after interruption. It preserves only
matching Rust checkpoints. Never combine `--resume` and `--force`.

## Observe without cancelling

Launch the native observer separately when the user needs a dashboard:

```powershell
cargo run --release --manifest-path rust/Cargo.toml --bin swarms-ui --features ui-egui -- --run-id <run-id>
```

The UI is read-only except for explicit steering. It never starts workers or
mutates plan/state snapshots. A task is `stale` when `worker.log` stops growing
or changing; stale is an investigation signal, never an automatic kill.

On Windows, real CLI workers open a visible PowerShell console that tails their
`worker.log` while the worker and coordinator remain in the background. Set
`SWARMS_WORKER_CONSOLES=hidden` only when those windows are unwanted. Mock
workers intentionally do not open consoles.

## Complete with evidence

After the run ends, inspect the terminal report, task states, worker logs and
required artifacts. Report the requested route and the effective route
separately, record fallback/blocking honestly, and never equate missing token
telemetry with zero usage.

After Rust changes, run every validation command in `AGENTS.md`, including
format, Clippy, all-feature tests, release build, doctor and one mock workflow.
Do not commit local router files, auth, `.agent/` runs, prompts, logs or reports.
