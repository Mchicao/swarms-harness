# AGENTS.md

This file is for coding agents working on SWARMS.

## Prime Directive

The Rust binary is the sole public runtime. Use it for all workflow operations:

```bash
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_example.json --force
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Python scripts are legacy benchmark/telemetry tools. No Rust code invokes Python.

## Branch and Worktree Policy

- Normal SWARMS operation: work on `main` or the current branch without creating branches automatically.
- Branches/worktrees: use them only when the user explicitly enables isolated or benchmark mode.

## Blocking Execution for Coordinators

- Treat each `swarms run` as one blocking tool call that returns control when the workflow finishes.
- While the process is active, do not poll processes, diffs, logs, reports, or artifacts, and do not emit intermediate validation updates.
- Wait for the tool call's final result. Inspect persisted state only after completion, timeout, error, or an explicit user request.
- Do not launch another execution in the same workspace while the previous one is still active.

## Goal

SWARMS exists to let each user configure their own local agent workflow: which model plans, which model codes, which model reviews, which APIs or CLIs are available, and how much concurrency each provider gets. Spend intelligence on planning and review. Let deterministic Rust handle scheduling, locks, provider caps, execution state, verification, telemetry, and reports.

Priority order:

1. correctness;
2. scope control;
3. low token/quota spend;
4. reproducible local verification;
5. safe open-source defaults.

## Role Policy

- Planner: GLM 5.2 by default; Codex/OpenAI/Anthropic-style premium routes only when explicitly justified and configured.
- Critic: GLM 5.2 first; premium routes only for high-risk/high-cost plans.
- Runtime: `rust/src/main.rs` schedules plan workflows without a model; all adapters are native Rust.
- Programmer workers: mock by default; GLM 5.2/Gemini Flash/OpenAI-compatible/Codex routes only when configured and requested.
- Verifier workers: local tests first; cheap model review second; premium escalation only by policy.
- Claude: disabled by default.

## Thinking Levels

Per-task `thinking` controls reasoning depth. Only verified adapter flags are used:

- Codex: `model_reasoning_effort` via `-c` (minimal/low/medium/high/ultra).
- OpenCode/Kilo: `--variant` (minimal/low/medium/high/max).
- Hermes/agy: not supported — review rejects non-default thinking.
- OpenAI-compat: only when route config declares `thinking_field`.

## Session Affinity

Tasks can reuse provider sessions for prompt caching. See `docs/RUST_RUNTIME.md`.

## Parallel Test-Time Scaling

A task may run several candidate rollouts instead of one shot. The policy is a
`scaling` block on the task (or plan-level default); absent = `single`, the
classic path, byte-identical to before.

Agent-facing rules (implementation: `rust/src/scaling.rs`):

- Modes: `single` | `best_of_n` | `adaptive_parallel` | `synthesize_n`.
  `adaptive_parallel` runs 1 rollout, expands by `candidates` more only if
  deterministic verification is ambiguous, then escalates once at most.
  `synthesize_n` runs N rollouts and has the synthesis route produce one new
  solution. Total rollouts are bounded by `max_rollouts` (default: mode
  budget); waves are clamped by the route cap and global cap.
- Candidate rollouts execute in isolated **git worktrees** seeded with the
  workspace dirty state. Scaling fails closed without a git workspace. Never
  run a scaled task on a non-repo workspace.
- Winner selection is deterministic-first: the task's `verify` commands run
  inside each candidate worktree; exactly one passer wins with no LLM call.
  Ties/all-fail go to `verifier_route` (JSON verdict, needs
  `confidence >= min_confidence`), then one quota-checked escalation
  (`select`/`review`/`synthesize` via `escalate_route`).
- The winner's diff is applied to the real workspace, then the standard
  artifact + verify gates run exactly once against the root. Session reuse is
  rejected on scaled tasks (candidates must be independent).
- Static review codes: `scaling_session_reuse`, `invalid_scaling_candidates`
  (2..=8), `invalid_scaling_budget`, `scaling_route_unknown`,
  `scaling_route_disabled`, `scaling_premium_blocked`,
  `scaling_requires_verification`, `scaling_requires_synth_route`.
- Observability: `TaskState.scaling` (rollouts, routes, models, per-rollout
  usage and verification, scores, winner, decision reason, verifier,
  escalation) plus `scaling_wave_started` / `scaling_rollout_finished` /
  `scaling_decision` events. Read these to answer "was scaling worth it".
- Reference plan: `docs/workflow_plan_scaling_example.json` (Gemini 3.7 Flash
  Medium route `gemini37_flash_medium`; base router keeps it disabled — it is
  enabled per-machine in `config/swarm_router.local.json`).

## Required Validation

Before claiming changes are complete:

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --release --manifest-path rust/Cargo.toml --all-features
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --run-id verify-agent --global-max-concurrency 3 --provider-cap mock=3
```

## Public Architecture

- `rust/src/main.rs`: CLI entry point.
- `rust/src/cli.rs`: argument parsing, run-id validation.
- `rust/src/config.rs`: router/plan loading, overlay merge.
- `rust/src/model.rs`: domain types (Plan, Task, Provider, ThinkingLevel, SessionConfig).
- `rust/src/review.rs`: static plan validation (DAG, routes, thinking, session, artifacts).
- `rust/src/runtime.rs`: scheduler (DAG waves, retries, observable progress, resume, verify, artifacts).
- `rust/src/scaling.rs`: parallel test-time scaling (candidate rollouts in git worktrees, verifier, escalation).
- `rust/src/adapter.rs`: native adapters (mock, CLI builders, OpenAI-compat HTTP, session/usage parsing).
- `rust/src/session.rs`: session affinity store.
- `rust/src/telemetry.rs`: usage normalisation, task state, report generation.
- `config/role_policy.json`: planner/critic/programmer/verifier policy.
- `docs/workflow_plan_example.json`: working plan example.
- `docs/workflow_plan_scaling_example.json`: parallel scaling plan example (Gemini 3.7 Flash Medium).

## Safety

Never run Claude Code, Codex, Gemini, OpenCode, or paid APIs unless the user explicitly asks and local config enables them. Never commit `.env`, `config/*.local.json`, auth files, telemetry traces, generated reports, `.agent/`, worktrees, or worker prompt/log/status artifacts.


## External Contribution

When an agent finds a reproducible defect, suggest a GitHub issue with the command, platform, expected result, and sanitized output. When it has a safe, scoped fix, create a branch and PR instead of pushing directly to the default branch. Never include secrets, local configuration, auth state, generated worker artifacts, or tokens in an issue, PR, command, screenshot, or commit. Ask the user's own agents to follow the same PR-or-issue workflow.
