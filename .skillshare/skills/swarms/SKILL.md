---
name: swarms
description: "Operate the native Rust SWARMS runtime for durable multi-agent workflows: plan, review, dry-run, run, resume, provider caps, persisted state, observation, retries and scaling."
license: MIT
metadata:
  author: SWARMS
  version: "3.0"
---

# SWARMS runtime

## Activation Contract

Use this skill only when work should execute through the SWARMS Rust runtime. Typical reasons are a
multi-stage DAG, durable/resumable execution, provider/global concurrency caps, persisted run state,
observer/telemetry, retries, or parallel test-time scaling.

If the goal is simply to ask Codex, Gemini, Claude, OpenCode, GLM or another agent to do one or a few
bounded tasks directly, use `$multi-provider-agent-orchestration` instead. Do not introduce the
runtime when direct delegation is sufficient.

Read `AGENTS.md` before editing the SWARMS runtime itself.

## Hard Rules

- The Rust binary is the sole public SWARMS runtime. Python runtime scripts are legacy or retired.
- Pass the exact target repo with `--workspace-root` whenever the workflow plan is outside the target
  workspace. Run state belongs to that target workspace.
- Use `needs` for dependencies. Dependents receive completed readable outputs; never tell a worker
  to read another process's private log. Put reusable inputs in declared workspace-owned artifacts.
- Use mock unless real configured providers were explicitly authorized.
- Treat `run` as one blocking coordinator call. Do not poll its files while it is active.
- Run only one coordinator for the same workspace/run identity at a time.
- Never commit credentials, local routers, `.agent/`, prompts, logs, reports, worker state, or
  generated worktrees.
- Do not silently substitute a blocked or unavailable provider.

## When Runtime Is Worth Using

| Need | Use SWARMS runtime? |
| --- | --- |
| Ask one agent for a review or implementation | No - direct delegation skill |
| A few independent direct agent calls you can supervise yourself | No - direct delegation skill |
| DAG with explicit dependencies | Yes |
| Crash-safe resume / persisted task state | Yes |
| Global and per-provider concurrency caps | Yes |
| Durable evidence, telemetry, observer or reports | Yes |
| Runtime-managed retries / provider routing | Yes |
| Best-of-N, adaptive parallel or synthesis scaling | Yes |

## Workflow Contract

Define a plan JSON with a goal and bounded tasks. Each task should declare the fields required by the
current schema, including its id, role/route, scope, dependency edges, artifacts, tools policy and
deterministic verification. Preserve repository invariants and make write ownership explicit.

Use provider routes only as configured by the router. Route names are not credentials and do not
imply that a provider is locally enabled.

## Execution Lifecycle

Use the same router and workspace for review, dry-run and execution.

```powershell
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- review --plan <plan.json> --workspace-root <target>
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan <plan.json> --workspace-root <target> --force
cargo run --manifest-path rust/Cargo.toml -- run --plan <plan.json> --workspace-root <target> --force --global-max-concurrency <n> --provider-cap <route>=<n>
```

If the plan already lives in the target workspace and the CLI derives the same root correctly, the
explicit workspace flag may be unnecessary; prefer being explicit when operating another repo.

For an interrupted prior run, use the runtime's resume path. Never combine mutually exclusive
resume/force semantics. Verify the current CLI help before relying on a flag that may have changed.

## After the Blocking Run Returns

Inspect the terminal report, persisted task states, declared artifacts and readable logs. Confirm:

- requested versus effective route/model;
- completed, failed and blocked task states;
- deterministic verification results;
- retries/fallbacks and quota/cap behavior;
- final target-workspace diff and unresolved risk.

Do not report a workflow as successful merely because a worker said it succeeded.

## Runtime Development

When changing SWARMS itself, follow `AGENTS.md` and run the required Rust gates for the affected
scope. In particular, preserve scheduler locks, provider-cap semantics, persisted state contracts,
workspace boundaries and fail-closed validation.

## References

- `../../AGENTS.md` - runtime development contract.
- `../../docs/CONFIG.md` - router and workspace configuration.
- `../../docs/STATE_CONTRACT.md` - persisted run contract.
- `../../docs/PROVIDER_STATUS.md` - implemented versus locally enabled providers.
- `../multi-provider-agent-orchestration/SKILL.md` - direct delegation without SWARMS runtime.