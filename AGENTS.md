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

## Ejecución bloqueante para agentes coordinadores

- Trata cada `swarms run` como una única tool call bloqueante que devuelve control cuando el workflow termina.
- Mientras el proceso siga activo, no hagas polling de procesos, diffs, logs, reportes ni artefactos y no emitas validaciones intermedias.
- Espera el resultado final de la llamada. Inspecciona estado persistido sólo después de finalización, timeout, error o petición explícita del usuario.
- No lances otra ejecución sobre el mismo workspace mientras la anterior siga activa.

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
- `rust/src/runtime.rs`: scheduler (DAG waves, retries, timeout, resume, verify, artifacts).
- `rust/src/adapter.rs`: native adapters (mock, CLI builders, OpenAI-compat HTTP, session/usage parsing).
- `rust/src/session.rs`: session affinity store.
- `rust/src/telemetry.rs`: usage normalisation, task state, report generation.
- `rust/src/ui_main.rs`: low-overhead egui/Glow observability UI.
- `rust/src/ui_bin.rs`: native `swarms-ui` binary entry point.
- `config/role_policy.json`: planner/critic/programmer/verifier policy.
- `docs/workflow_plan_example.json`: working plan example.

## Safety

Never run Claude Code, Codex, Gemini, OpenCode, or paid APIs unless the user explicitly asks and local config enables them. Never commit `.env`, `config/*.local.json`, auth files, telemetry traces, generated reports, `.agent/`, worktrees, or worker prompt/log/status artifacts.

## External Contribution

When an agent finds a reproducible defect, suggest a GitHub issue with the command, platform, expected result, and sanitized output. When it has a safe, scoped fix, create a branch and PR instead of pushing directly to the default branch. Never include secrets, local configuration, auth state, generated worker artifacts, or tokens in an issue, PR, command, screenshot, or commit. Ask the user's own agents to follow the same PR-or-issue workflow.
