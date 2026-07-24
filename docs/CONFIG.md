# Configuration

SWARMS has two configuration layers:

1. `config/swarm_router.json` is the committed safe default. It routes every task to the offline mock provider.
2. `config/swarm_router.local.json` is your private local config. It is gitignored and may enable real providers.

To enable real providers:

```powershell
Copy-Item config\swarm_router.local.example.json config\swarm_router.local.json
```

Edit only the local file.

Las rutas OpenCode pueden fijar una variante de razonamiento en el proveedor:

```jsonc
{
  // SWARMS-CONFIG-001: Solicita el esfuerzo high sin cambiar el model id.
  "providers": {
    "glm52": {"enabled": true, "variant": "high"}
  }
}
```

Para ejecutar workers con herramientas sobre otro repositorio, usa
`--workspace-root`. El router y el código del harness permanecen en SWARMS;
los agentes leen las reglas y escriben los artefactos en esa raíz objetivo.

## Token-Saving Defaults

The router scores providers with:

- quality: expected task capability;
- relative cost: API or plan quota cost;
- scarcity: how strongly to protect that plan;
- role match: deterministic role preferences;
- health: optional `swarm_limits.yaml` status.

For saving expensive quota, keep scarce models disabled or route them only with explicit directives.

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
