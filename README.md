# SWARMS

Experimental quota-saving workflow harness for coding agents.

SWARMS is for developers who want parallel, agentic coding workflows without spending premium model quota on every implementation step. It keeps the expensive intelligence in planning, review, and escalation, while deterministic Python code handles task scheduling, provider caps, locks, state, reports, and offline verification.

The public default is safe after clone: it uses only the offline `mock` worker. Real providers such as GLM 5.2, Gemini Flash, Codex, Claude, or Opus are disabled unless you create local configuration and explicitly route work to them.

Spanish docs: [README.es.md](README.es.md)

## Status

SWARMS is alpha software. The offline workflow, static plan review, deterministic runtime, mock worker, tests, and CI path are the publishable MVP. Real provider adapters are intentionally marked experimental or reserved until they have stable local setup, telemetry, and safety checks.

## Why This Exists

SWARMS started around January-February 2026, when Ralph-style coding loops became popular. The original idea was to parallelize those loops using the cheaper models available to me as a student, especially Gemini through Antigravity, while reserving Opus for planning.

Today the same idea targets GLM 5.2 and Gemini Flash as routine programmer/verifier workers, with Codex or Opus reserved for planning, critique, security-sensitive work, or explicit escalation.

## Design Goals

- Save scarce or expensive model quota.
- Make planning and review explicit before execution.
- Run cheap or local workers under provider caps.
- Keep premium providers opt-in.
- Preserve deterministic state, task locks, reports, and telemetry.
- Offer a free offline demo path for CI and contributors.

## Quick Start

Requires Python 3.10+ and Git.

```powershell
python scripts/swarm.py doctor
python scripts/swarm.py review --plan docs/workflow_plan_example.json
python scripts/swarm.py dry-run --plan docs/workflow_plan_example.json --force
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Optional editable install:

```powershell
python -m pip install -e ".[dev,yaml]"
swarms doctor
swarms run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

## Public Architecture

```text
User goal
  -> planner writes a structured workflow plan
  -> static reviewer checks scope, dependencies, provider budget, and premium use
  -> deterministic runtime schedules tasks with provider caps and file locks
  -> workers execute narrow tasks
  -> verifier tasks and local commands check outcomes
  -> report.json records state, results, and token/cost fields
```

Core files:

- `scripts/swarm.py`: single public CLI.
- `scripts/plan_review.py`: static workflow-plan reviewer.
- `scripts/workflow_runtime.py`: deterministic runtime.
- `scripts/mock_worker.py`: offline worker for tests and demos.
- `scripts/doctor.py`: local health check.
- `config/role_policy.json`: planner, critic, programmer, and verifier policy.
- `config/swarm_router.json`: committed offline-safe router defaults.
- `docs/workflow_plan_example.json`: working offline plan.

## Provider Policy

The committed configuration enables only `mock`. Local provider configuration belongs in `config/swarm_router.local.json`, which is ignored by Git.

Default role intent:

- Planner: GLM 5.2 by default, Codex only when explicitly justified.
- Critic: GLM 5.2 first, Codex only for high-risk or high-cost plans.
- Programmer: GLM 5.2 or Gemini Flash when configured and requested.
- Verifier: local tests first, then cheap model review, then premium escalation only by policy.
- Claude, Codex, Opus, and other premium routes: disabled by default.

See `docs/PROVIDER_STATUS.md` and `docs/CONFIG.md`.

## Safety

Do not commit:

- `.env`
- `config/*.local.json`
- API keys, OAuth tokens, auth files, or private credentials
- `.agent/`
- worktrees
- generated prompts, logs, traces, reports, or telemetry

The default workflow does not call paid APIs or external model providers.

## Verification

```powershell
python -m py_compile scripts\swarm.py scripts\plan_review.py scripts\workflow_runtime.py scripts\doctor.py scripts\mock_worker.py
python -m pytest tests -q
python scripts/swarm.py doctor
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --run-id verify-readme --global-max-concurrency 3 --provider-cap mock=3
```

## Roadmap

See `docs/ROADMAP.md` for the planned path from offline MVP to real provider adapters, better telemetry, provider budgets, and safer multi-worktree execution.

## License

MIT. See `LICENSE`.
