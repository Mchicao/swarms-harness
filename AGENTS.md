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

SWARMS exists to let each user configure their own local agent workflow: which model plans, which model codes, which model reviews, which APIs or CLIs are available, and how much concurrency each provider gets. Spend intelligence on planning and review. Let deterministic scripts handle scheduling, locks, provider caps, execution state, verification, telemetry, and reports.

Priority order:

1. correctness;
2. scope control;
3. low token/quota spend;
4. reproducible local verification;
5. safe open-source defaults.

## Role Policy

- Planner: GLM 5.2 by default; Codex/OpenAI/Anthropic-style premium routes only when explicitly justified and configured.
- Critic: GLM 5.2 first; premium routes only for high-risk or high-cost plans.
- Runtime: `scripts/swarm.py` and `scripts/workflow_runtime.py`, no model.
- Programmer workers: mock by default; GLM 5.2/Gemini Flash/OpenAI-compatible/LiteLLM/Kilo/Aider routes only when configured and requested.
- Verifier workers: local tests first; cheap model review second; premium escalation only by policy.
- Claude: disabled by default.

## Local Provider Configuration

Agents should help users configure SWARMS without assuming secrets or subscriptions:

1. Inspect `config/swarm_router.json`, `config/swarm_router.local.example.json`, `config/role_policy.json`, and the user's requested provider policy.
2. Ask which APIs, CLIs, or subscription plans the user wants to spend.
3. Keep real credentials in environment variables or ignored local files.
4. Put local provider routing in `config/swarm_router.local.json`.
5. Run the offline flow before any real provider route.
6. Do not enable OpenAI, LiteLLM, Anthropic, Codex, Gemini, OpenCode, Kilo, Aider, or other paid/local CLIs unless the user explicitly asks.

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

## Singularity

Singularity is the autonomous loop for users who want SWARMS to keep proposing improvements, reading issues, creating tasks, running workers, doing QA, validating features, summarizing state, and starting the next cycle.

Use it only after confirming token budget and provider caps. With real providers and high cycle counts, Singularity can spend a large number of tokens. Start with `mock`, a low `-MaxCycles`, and a visible `STOP_SINGULARITY` escape hatch.

## Safety

Never run Claude Code, Codex, Gemini, OpenCode, or paid APIs unless the user explicitly asks and local config enables them. Never commit `.env`, `config/*.local.json`, auth files, telemetry traces, generated reports, `.agent/`, worktrees, or worker prompt/log/status artifacts.
