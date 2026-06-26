# Provider Status

SWARMS separates route names from provider execution. A route can exist in code, docs, telemetry, or local configuration while still staying disabled in committed defaults.

## Current Public Defaults

| Route | Status | Default | Notes |
| --- | --- | --- | --- |
| `mock` | Supported | Enabled | Offline worker for tests, demos, and CI. |
| `local_tests` | Reserved | Disabled | Intended for deterministic shell/test verification tasks. |
| `glm52` | Implemented route | Disabled | Low-cost programmer/planner route through OpenCode or Z.AI-style local setup. |
| `gemini_flash` | Implemented route | Disabled | Low-cost docs/review/test route through Antigravity CLI local setup. |
| `openai_compatible` | Configurable family | Disabled | Use for OpenAI-compatible APIs or gateways when a user adds a local route. |
| `litellm` | Configurable family | Disabled | Use for a local LiteLLM gateway when a user wants central routing. |
| `anthropic` | Configurable premium family | Disabled | Use for Claude/Opus-style planner, critic, or escalation routes. |
| `codex` | Reserved premium | Disabled | Must require explicit user approval and premium policy. |
| `claude` | Reserved premium | Disabled | Must require explicit user approval and premium policy. |
| `opus` | Reserved premium | Disabled | Planning/escalation only; not enabled by default. |

## Rules For Real Providers

- Never enable a real provider in committed config.
- Never require secrets for `doctor`, tests, or CI.
- Keep credentials in environment variables or ignored local config.
- Treat every real coding agent as code execution with file access.
- Record missing token usage honestly instead of reporting fake zero-cost runs.
- Keep premium routes blocked unless the plan and local config both allow them.

## Singularity Token Risk

Singularity can run repeated architect, worker, watcher, retry, QA, issue-reading, validation, and summary phases. With real providers enabled, a long run can consume far more tokens than a single coding-agent session. Start with `mock`, keep `MaxCycles` low, and set provider caps before using GLM, Gemini, OpenAI-compatible, LiteLLM, Codex, Anthropic, Claude, or Opus routes.

## Local Configuration

Copy the example file and edit the copy:

```powershell
Copy-Item config\swarm_router.local.example.json config\swarm_router.local.json
```

The local file is ignored by Git.
