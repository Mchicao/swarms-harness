# Magnitude Architecture Lessons For SWARMS

Reviewed: 2026-06-19

Sources:

- https://magnitude.dev/
- https://github.com/magnitudedev/browser-agent
- https://docs.magnitude.run/advanced/roles
- https://docs.magnitude.run/advanced/memory
- https://docs.magnitude.run/reference/llm-providers
- https://docs.magnitude.run/reference/browser-agent

## Useful Decisions To Adopt

### 1. Cost Savings As Product Positioning

Magnitude positions itself around "frontier coding without frontier prices" and claims cost savings through open models and smart routing. SWARMS should keep the same level of clarity: the primary product objective is saving scarce or expensive model quota without losing task quality.

### 2. Role-Based Model Routing

Magnitude separates responsibilities such as leader, scout, architect, engineer, critic, and also documents role-specific LLM assignment for browser-agent operations like `act`, `extract`, and `query`.

SWARMS already has role tags and provider routing. We should formalize roles beyond current tags:

- `leader`: task decomposition and merge strategy.
- `scout`: repo discovery and context collection.
- `architect`: design and interfaces.
- `engineer`: implementation.
- `qa`: tests and verification.
- `critic`: review and risk assessment.

The router should map these roles to provider capabilities and scarcity, not just generic tags.

### 3. Prompt Caching And Sliding Memory

Magnitude documents prompt caching and constrained retained memory to reduce cost on longer tasks. SWARMS should implement the same principle for coding work:

- keep stable system/task contract in cache-friendly prompt sections;
- avoid sending whole repo context repeatedly;
- store worker handoff artifacts in deterministic files;
- pass only dependency outputs that are needed by the next stage;
- summarize prior worker state before launching later workers.

### 4. Provider Interface Schema

Magnitude exposes provider configuration as typed provider options. SWARMS should move toward declarative provider adapters:

```json
{
  "wrapper": "command",
  "command": "opencode run -m {model} --format json --dangerously-skip-permissions {prompt}",
  "usage_parser": "opencode_jsonl",
  "supports_noninteractive": true,
  "supports_token_usage": true,
  "max_concurrency": 2,
  "quota_window": "5h"
}
```

This would let users add providers/plans without editing scheduler logic.

### 5. Benchmark Methodology

Magnitude describes a benchmark with multiple real-world tasks, repeated trials, score, and cost by token usage. SWARMS should evolve from single micro-task checks to:

- multiple task families;
- repeated trials per variant;
- score plus wall time plus measured/missing token usage;
- explicit failure categories;
- separate cheap/offline CI benchmark from real paid-provider benchmark.

### 6. Controllable Abstraction Levels

Magnitude distinguishes high-level natural-language actions from lower-level controllable steps. SWARMS should expose both:

- high-level: "run swarm on this feature";
- low-level: "route this taskfile with this provider matrix and budget guard";
- deterministic: "mock_swarm doctor/CI".

### 7. Avoid Default Premium Provider Bias

Magnitude browser-agent docs mention defaulting to Claude when an Anthropic key is available. SWARMS should intentionally avoid this pattern. Presence of an API key must not imply permission to spend quota. Real providers must require local config and/or explicit route.

## Near-Term SWARMS Backlog From This Review

1. Add a declarative provider adapter schema.
2. Add explicit role taxonomy: leader, scout, architect, engineer, qa, critic.
3. Add budget guard before worker launch.
4. Add prompt-cache-friendly prompt templates.
5. Add benchmark repeats and score/cost aggregation.
6. Add a Python-native scheduler roadmap for macOS/Linux without PowerShell.
