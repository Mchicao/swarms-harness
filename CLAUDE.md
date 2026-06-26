# CLAUDE.md

This file is for Claude Code and Claude-compatible agents working on SWARMS.

## Prime Directive

SWARMS exists to save expensive quota. Do not spend Claude, Codex, Gemini, OpenCode, or paid API calls for routine work.

Use the single public flow:

```powershell
python scripts/swarm.py doctor
python scripts/swarm.py review --plan docs/workflow_plan_example.json
python scripts/swarm.py dry-run --plan docs/workflow_plan_example.json --force
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Do not call legacy runners directly unless the user explicitly asks.

## Architecture

- Planning and critique are model work.
- Runtime orchestration is deterministic Python.
- Premium providers are opt-in and blocked by default.
- `scripts/swarm.py` is the only public entrypoint.
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
