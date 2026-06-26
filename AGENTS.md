# AGENTS.md

This file is for coding agents working on SWARMS.

## Prime Directive

Use exactly one public flow:

```powershell
python scripts/swarm.py doctor
python scripts/swarm.py review --plan docs/workflow_plan_example.json
python scripts/swarm.py dry-run --plan docs/workflow_plan_example.json --force
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Do not invoke legacy runners directly unless the user explicitly asks for legacy compatibility work.

## Goal

SWARMS exists to save scarce or expensive model quota. Spend intelligence on planning and review. Let deterministic scripts handle scheduling, locks, provider caps, execution state, verification, telemetry, and reports.

Priority order:

1. correctness;
2. scope control;
3. low token/quota spend;
4. reproducible local verification;
5. safe open-source defaults.

## Role Policy

- Planner: GLM 5.2 by default; Codex only when explicitly justified.
- Critic: GLM 5.2 first; Codex only for high-risk or high-cost plans.
- Runtime: `scripts/swarm.py` and `scripts/workflow_runtime.py`, no model.
- Programmer workers: mock by default; GLM 5.2/Gemini Flash only when configured and requested.
- Verifier workers: local tests first; cheap model review second; premium escalation only by policy.
- Claude: disabled by default.

## Required Validation

Before claiming changes are complete:

```powershell
python -m py_compile scripts\swarm.py scripts\plan_review.py scripts\workflow_runtime.py scripts\doctor.py scripts\mock_worker.py
python -m pytest tests -q
python scripts/swarm.py doctor
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --run-id verify-agent --global-max-concurrency 3 --provider-cap mock=3
```

## Public Architecture

- `scripts/swarm.py`: single public CLI.
- `scripts/plan_review.py`: static workflow-plan reviewer.
- `scripts/workflow_runtime.py`: deterministic runtime with task state, locks, provider caps, events, results, and reports.
- `scripts/mock_worker.py`: offline provider for CI/demos.
- `config/role_policy.json`: planner/critic/programmer/verifier policy.
- `docs/workflow_plan_example.json`: working plan example.

Legacy scripts such as `scripts/parallel_swarm.ps1` and `scripts/run_agentic_swarm_benchmark.py` are internal compatibility assets, not the public workflow.

## Safety

Never run Claude Code, Codex, Gemini, OpenCode, or paid APIs unless the user explicitly asks and local config enables them. Never commit `.env`, `config/*.local.json`, auth files, telemetry traces, generated reports, `.agent/`, worktrees, or worker prompt/log/status artifacts.
