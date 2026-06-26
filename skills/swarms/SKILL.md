---
name: swarms
description: Use when an agent needs to run, review, configure, benchmark, or extend SWARMS, a quota-saving workflow harness for coding agents. Triggers include SWARMS, workflow_plan, planner critic runtime, provider caps, token-saving routing, and protecting premium model quota.
---

# SWARMS

Use exactly one public flow:

```powershell
python scripts/swarm.py doctor
python scripts/swarm.py review --plan docs/workflow_plan_example.json
python scripts/swarm.py dry-run --plan docs/workflow_plan_example.json --force
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Do not call legacy runners directly unless the user explicitly asks for legacy compatibility.

## Principles

- Preserve the offline `mock` default.
- Do not run real providers unless the user explicitly asks.
- Treat Claude, Codex, Gemini/Antigravity, OpenCode, and paid APIs as quota-spending providers.
- Never interpret missing token telemetry as zero real usage.
- Spend intelligence on planning and critique; keep runtime orchestration deterministic.

## Role Split

- Planner: GLM 5.2 by default, Codex only when explicitly justified.
- Critic: GLM 5.2 first, Codex only for high-risk/high-cost plans.
- Runtime: `scripts/swarm.py` and `scripts/workflow_runtime.py`, no model.
- Programmer workers: mock by default; cheap configured providers when requested.
- Verifier workers: local tests first; cheap model review second; premium escalation only by policy.

## Repository Detection

In a SWARMS repo, expect:

- `scripts/swarm.py`
- `scripts/plan_review.py`
- `scripts/workflow_runtime.py`
- `scripts/mock_worker.py`
- `config/role_policy.json`
- `docs/workflow_plan_example.json`

## Required Validation

Before claiming SWARMS changes are complete:

```powershell
python -m py_compile scripts\swarm.py scripts\plan_review.py scripts\workflow_runtime.py scripts\doctor.py scripts\mock_worker.py
python -m pytest tests -q
python scripts/swarm.py doctor
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --run-id verify-skill --global-max-concurrency 3 --provider-cap mock=3
```

## Safety

Never commit:

- `.env`
- `config/*.local.json`
- auth files or OAuth tokens
- telemetry traces
- generated reports
- `.agent/`
- worktrees
- worker prompt/log/status artifacts

If a user asks for real providers, confirm the exact provider and plan first, then keep premium routes disabled unless explicitly enabled.
