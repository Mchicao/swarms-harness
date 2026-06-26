# Planning Architecture

SWARMS separates intelligence-heavy planning from deterministic orchestration.

## Roles

1. **Planner / Architect**
   - Creates `workflow_plan.json`.
   - Uses the strongest model that is justified by the expected worker spend.
   - Defines stages, dependencies, artifacts, verification, budgets, and provider routes.

2. **Critic / Plan Reviewer**
   - Reviews the plan before workers run.
   - Looks for ambiguity, missing dependencies, missing verification, risky scope, file collisions, and unnecessary premium usage.
   - Can be a cheap model first, with Codex escalation only when the policy allows it.

3. **Runtime / Orchestrator**
   - Deterministic code, not a model.
   - Runs `plan_review.py`, compiles the plan into runtime tasks, applies provider caps, claims tasks with locks, executes workers, and writes reports.

4. **Programmer Workers**
   - Implement focused tasks from the approved plan.
   - Default to cheaper providers such as GLM 5.2 or Gemini Flash when configured.

5. **Verifier Workers**
   - Run local tests first.
   - Review outputs from other workers only when deterministic checks are insufficient.
   - Premium verifier routes are opt-in.

## Flow

```text
User goal
  -> Planner model creates workflow_plan.json
  -> Static review with scripts/plan_review.py
  -> Optional critic model feedback
  -> Planner revises the plan
  -> Runtime dry-run and budget check
  -> Runtime launches programmer/verifier workers
  -> Runtime writes report.json
```

## Commands

Review a plan without spending quota:

```powershell
python scripts/swarm.py review --plan docs/workflow_plan_example.json
```

Dry-run a reviewed plan:

```powershell
python scripts/swarm.py dry-run --plan docs/workflow_plan_example.json --force
```

Run the mock plan:

```powershell
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

## Policy

The planner can be smart and expensive when the downstream run is expected to be expensive. The runtime should stay deterministic. Premium providers are disabled by default and must be explicitly allowed by plan policy and local provider config.
