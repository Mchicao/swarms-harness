# Publishing Checklist

This workspace may live inside a larger Git repository. Confirm the Git root before publishing:

```powershell
git rev-parse --show-toplevel
```

If the output is not the SWARMS directory itself, publish from a clean copy or initialize a dedicated repository inside the SWARMS folder.

## Recommended First Publication

```powershell
cd C:\Proyectos\SWARMS
git init
git add .
git status --short
git commit -m "Initial public alpha"
git branch -M main
git remote add origin https://github.com/Mchicao/swarms-harness.git
git push -u origin main
```

Before pushing, run:

```powershell
python -m ruff check .
python -m ruff format --check scripts\swarm.py scripts\plan_review.py scripts\workflow_runtime.py scripts\doctor.py scripts\mock_worker.py scripts\smart_router.py scripts\utils\token_telemetry.py tests
python -m py_compile scripts\swarm.py scripts\plan_review.py scripts\workflow_runtime.py scripts\doctor.py scripts\mock_worker.py
python -m pytest tests -q
python scripts\swarm.py doctor
python scripts\swarm.py run --plan docs\workflow_plan_example.json --force --run-id verify-publish --global-max-concurrency 3 --provider-cap mock=3
```

## Do Not Publish

- `.env`
- `config/*.local.json`
- `.agent/`
- `.cache/`
- `.swarm_worktrees/`
- generated prompts, logs, traces, reports, telemetry, and worktrees
- personal provider auth files

## Suggested Repository Metadata

- Description: `Experimental quota-saving workflow harness for coding agents.`
- Topics: `coding-agents`, `llm`, `workflow`, `orchestration`, `python`, `developer-tools`
- Website: `https://github.com/Mchicao`
