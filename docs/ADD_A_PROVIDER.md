# Add A Provider

The public SWARMS flow routes work from a structured plan through `scripts/swarm.py`.

Provider integration has two layers:

1. **Policy**: `config/role_policy.json` and each plan's `budget_policy.provider_concurrency`.
2. **Adapter**: runtime code that maps a route name to a provider/model/wrapper.

Current public route:

- `mock` -> offline `scripts/mock_worker.py`

Reserved route names:

- `glm52`
- `gemini_flash`
- `codex`
- `claude`
- `local_tests`

Premium routes must remain disabled by default. A plan that requests `codex`, `claude`, `opus`, or `gpt-5.5` fails static review unless `review_policy.premium_allowed` is explicitly true and local provider config allows it.

When adding a real provider adapter:

- keep credentials in local ignored config or environment variables;
- record missing token telemetry honestly;
- add tests that use `mock` or a fake adapter, not the real provider;
- update `docs/workflow_plan_example.json` only if the example remains free/offline.

## Herramientas interactivas

Un proveedor sólo puede usarse por el scheduler si acepta una tarea no
interactiva y devuelve una salida capturable. ZCode es una aplicación desktop,
no un CLI headless público: el alias `zcode` debe resolver a `glm52`, que usa
OpenCode con el plan de Z.AI. Freebuff actualmente sólo documenta UI
interactiva, `login`, `--continue` y `--cwd`; mantenlo deshabilitado hasta que
publique un modo de prompt/batch estructurado.
