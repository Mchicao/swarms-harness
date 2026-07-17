# CLAUDE.md

This file is for Claude Code and Claude-compatible agents working on SWARMS.

## Prime Directive

SWARMS exists to save expensive quota. Do not spend Claude, Codex, Gemini, OpenCode, or paid API calls for routine work.

Use the Rust public flow:

```powershell
cargo run --release --manifest-path rust/Cargo.toml -- doctor
cargo run --release --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json
cargo run --release --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_example.json --force
cargo run --release --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Do not call legacy runners directly unless the user explicitly asks.

## Architecture

- Planning and critique are model work.
- Runtime orchestration is deterministic Rust; Python remains the compatibility layer and provider bridge.
- Premium providers are opt-in and blocked by default.
- `rust/src/main.rs` is the public entrypoint.
- `scripts/plan_review.py` reviews structured plans.
- `scripts/workflow_runtime.py` executes reviewed plans with locks and provider caps.

## Required Validation

```powershell
python -m py_compile scripts\swarm.py scripts\plan_review.py scripts\workflow_runtime.py scripts\doctor.py scripts\mock_worker.py
python -m pytest tests -q
python scripts/swarm.py doctor
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --run-id verify-claude --global-max-concurrency 3 --provider-cap mock=3
```

## Safety

Never commit `.env`, `config/*.local.json`, auth files, OAuth tokens, telemetry traces, generated reports, `.agent/`, worktrees, or worker prompt/log/status artifacts.

Do not spawn subagents or recursively delegate work. The coordinator owns the
reviewed task graph and all worker budgets. If a task appears to require more
delegation, report the blocker instead of creating another agent tree.
