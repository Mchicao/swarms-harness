---
name: swarms
description: Use when an agent needs to run, review, configure, benchmark, or extend SWARMS, a quota-saving workflow harness for coding agents. Triggers include SWARMS, workflow_plan, planner critic runtime, provider caps, token-saving routing, and protecting premium model quota.
---

# SWARMS

> **Canonical orchestration skill:** `skills/multi-provider-agent-orchestration/`.
> Keep the installed Hermes copy synchronized with
> `python scripts/sync_multi_provider_skill.py` and verify drift with `--check`.
> This `swarms` skill remains a compact compatibility entrypoint for users of
> the repository's historical skill name.

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
- Treat OpenAI, LiteLLM, Anthropic, Claude, Codex, Gemini/Antigravity, OpenCode, Kilo, Aider, and paid APIs as user-configured quota-spending providers.
- Never interpret missing token telemetry as zero real usage.
- Spend intelligence on planning and critique; keep runtime orchestration deterministic.

## Role Split

- Planner: GLM 5.2 by default, premium OpenAI/Codex/Anthropic-style routes only when explicitly justified.
- Critic: GLM 5.2 first, premium routes only for high-risk/high-cost plans.
- Runtime: `scripts/swarm.py` and `scripts/workflow_runtime.py`, no model.
- Programmer workers: mock by default; cheap configured providers when requested.
- Verifier workers: local tests first; cheap model review second; premium escalation only by policy.

## Configuration Help

When a user asks to configure SWARMS:

1. Read `config/swarm_router.json`, `config/swarm_router.local.example.json`, `config/role_policy.json`, and `docs/CONFIG.md`.
2. Ask which provider families they want to use: OpenAI-compatible API, LiteLLM gateway, Anthropic, Gemini/Antigravity, GLM/OpenCode/Z.AI, Codex CLI, Kilo, Aider, local tests, or mock.
3. Keep secrets out of the repo. Use environment variables or ignored local config.
4. Create or update `config/swarm_router.local.json` only when the user approves.
5. Run `python scripts/swarm.py doctor` and the mock plan before routing real work.
6. Make provider caps explicit. Do not infer that a high subscription limit means unlimited spending.

## Singularity Help

Singularity is for autonomous loops: propose improvements, inspect issues, create tasks, run workers, perform QA, validate features, summarize state, then continue. It can run for long periods on a local machine.

Before helping with Singularity, confirm:

- maximum cycles or an explicit 24/7 intent;
- allowed providers;
- worker count and provider caps;
- stop condition, including `STOP_SINGULARITY`;
- expected verification commands;
- whether the user accepts high token usage.

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
