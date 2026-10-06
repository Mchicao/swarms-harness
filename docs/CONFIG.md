# Configuration

SWARMS has two configuration layers:

1. `config/swarm_router.json` is the committed safe default. It routes every task to the offline mock provider.
2. `config/swarm_router.local.json` is your private local config. It is gitignored and may enable real providers.

To enable real providers:

```powershell
Copy-Item config\swarm_router.local.example.json config\swarm_router.local.json
```

Edit only the local file.

Router configuration is validated after the local overlay is merged. Unknown
root, quota-policy, preference, and provider fields fail closed with the full
field path. This is intentional: a typo such as `providers.glm52.modle` must
not look like accepted configuration.

Some fields remain in the committed router files only for compatibility with
older configuration and documentation: top-level `preferences` and provider
metadata such as `variant`, `health_key`, `metric_key`, `relative_cost`,
`quality`, `scarcity`, `strengths`, and `weaknesses`. The current Rust runtime
does **not** use those fields for route selection or reasoning configuration.
They are explicitly allowlisted so they are not confused with arbitrary
unknown configuration, but new behavior must not be inferred from them.

For reasoning depth, use the plan/task `thinking` setting. OpenCode translates
that setting to its verified `--variant` CLI surface, while OpenCode V2 encodes
the variant in the model string as documented in `AGENTS.md` and
`docs/RUST_RUNTIME.md`.

Para ejecutar workers con herramientas sobre otro repositorio, usa
`--workspace-root`. El router y el código del harness permanecen en SWARMS;
los agentes leen las reglas y escriben los artefactos en esa raíz objetivo.
El runtime persiste el run bajo
`<workspace-root>/.agent/swarm/runs/<run-id>`. Si un comando ejecutable apunta
a un plan fuera del directorio de lanzamiento y omite `--workspace-root`, el
CLI falla antes de iniciar workers en vez de asumir el repositorio equivocado.

Migración one-time para runs anteriores al cambio de ubicación: los runs
lanzados con el binario viejo guardaban su estado bajo el directorio del
launcher (`<launcher>/.agent/swarm/runs/<id>`), incluso con
`--workspace-root`. Esos runs no se pueden resumir con el binario nuevo
ninguna combinación de flags; para recuperarlos, mueve el estado una vez:

```
move <launcher>\.agent\swarm\runs\<id> <workspace>\.agent\swarm\runs
```

y resume desde ese workspace con `--workspace-root <workspace>`.

Las tareas dependientes reciben automáticamente la salida legible de sus
`needs`. No codifiques rutas hacia `worker.log` de otro repositorio en el
prompt del reviewer; si necesita un archivo completo, decláralo como artefacto
dentro del workspace objetivo.

## Token-Saving Defaults

The current Rust runtime uses explicit route configuration rather than a
quality/cost/scarcity scoring function. To protect expensive quota:

- keep scarce providers disabled unless they are intentionally enabled locally;
- use `quota_policy` and provider `quota_key` for fail-closed quota checks;
- use typed `cost_class` plus plan `review_policy.premium_allowed` for premium routes;
- configure provider `fallback_routes` and the router `fallback_route` deliberately;
- select cheap routes explicitly in plans for routine work.

Legacy `preferences`, `quality`, `relative_cost`, `scarcity`, and health/metric
metadata are descriptive compatibility fields only in the current Rust runtime.

Example:

```markdown
- [ ] [backend] Implement routine parser
- [ ] [codex] [[route:codex]] Fix critical security-sensitive race condition
```

## Real Provider Notes

SWARMS is local-first. The repo does not ship provider credentials. Each user decides which plans, APIs, CLIs, and model routes to enable in local config.

OpenAI-compatible APIs can be added as local routes when the user provides the base URL, model name, and key through environment variables or ignored config.

LiteLLM can sit in front of multiple providers. In that setup, SWARMS should route to the local LiteLLM endpoint and let LiteLLM own provider credentials.

Anthropic-style routes should be treated as premium planner, critic, or escalation routes. They should remain blocked unless the plan and local config both allow them.

`opencode` can expose token usage in JSON output, so SWARMS can price and compare it.

`agy`/Antigravity can consume Google AI Pro quota. In current headless mode, token counts are not reliably exposed, so SWARMS records those events as `missing_usage_events`.

Codex and Claude-style premium agents should stay disabled unless the user explicitly opts in.

OpenCode 2.0 and pi-agent integrate through dedicated adapters. They are not
aliases for the current OpenCode adapter; enable them only after validating
their CLI/API, session, steering, telemetry and sandbox behavior locally.
