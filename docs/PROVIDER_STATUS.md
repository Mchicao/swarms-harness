# Provider Status

SWARMS separates route names from provider execution. A route can be documented before it is enabled in committed defaults.

## Current Public Defaults

| Route | Status | Default | Notes |
| --- | --- | --- | --- |
| `mock` | Supported | Enabled | Offline worker for tests, demos, and CI. |
| `local_tests` | Reserved | Disabled | Intended for deterministic shell/test verification tasks. |
| `glm52` | Experimental | Disabled | Intended low-cost programmer/planner route through local user configuration. |
| `gemini_flash` | Experimental | Disabled | Intended low-cost docs/review/test route through local user configuration. |
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

## Local Configuration

Copy the example file and edit the copy:

```powershell
Copy-Item config\swarm_router.local.example.json config\swarm_router.local.json
```

The local file is ignored by Git.
