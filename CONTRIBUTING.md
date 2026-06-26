# Contributing

Thanks for considering a contribution. SWARMS is alpha software, so small, well-scoped changes are preferred.

## Ground Rules

- Keep the default path offline and free.
- Do not add secrets, tokens, local auth files, generated traces, or provider logs.
- Do not enable paid providers in committed config.
- Use `scripts/swarm.py` as the public entrypoint.
- Keep docs honest about experimental behavior.

## Local Setup

```powershell
python -m pip install -e ".[dev,yaml]"
python scripts/swarm.py doctor
python -m pytest tests -q
```

## Required Checks

Run these before opening a pull request:

```powershell
python -m py_compile scripts\swarm.py scripts\plan_review.py scripts\workflow_runtime.py scripts\doctor.py scripts\mock_worker.py
python -m pytest tests -q
python scripts/swarm.py doctor
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --run-id verify-contrib --global-max-concurrency 3 --provider-cap mock=3
```

## Provider Changes

Provider adapters should include tests that use fake or mock providers. Do not add tests that require paid credentials in CI.
