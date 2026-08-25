# SWARMS

![SWARMS workflow cover](images/swarms-cover.png)

> **Local-First Multi-Agent Orchestration for Coding Swarms.**  
> Spend intelligence on planning and review. Let deterministic Rust coordinate fast, zero-cost, and open-weight models in parallel.

[![Website](https://img.shields.io/badge/Website-swarms--orchestrator.vercel.app-gold)](https://swarms-orchestrator.vercel.app/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![SwDD](https://img.shields.io/badge/Spec--Driven-SwDD-blueviolet)](https://github.com/Mchicao/swarm-driven-development)
[![Español](https://img.shields.io/badge/Docs-Espa%C3%B1ol-orange)](README.es.md)

SWARMS is a local-first agent orchestrator that lets you decide **which model plans, which model codes, which model reviews, and how much concurrency each provider gets**. It runs completely offline out of the box with simulated mocks, and connects to real CLI/API routes only when you configure them.

---

## The Proof: Cheap Models, Premium Results

![Terminal-Bench 2.1 scaling results](images/benchmark-terminal-bench.png)

**Scaling self-verification and parallel rollouts makes open-weight and budget models significantly more capable than frontier proprietary models at a fraction of the cost.**

### 1. Terminal-Bench 2.1 (DeepSeek V4 Flash)
- **79% → 88% Accuracy**: Sampling 5 candidate solutions with DeepSeek V4 Flash and ranking them with LLM-as-a-Verifier lifts benchmark performance from 79% to 88%.
- **11× Cheaper than Claude Fable 5**: Outperforms Claude Fable 5 at the same accuracy level while costing **11× less** (~$0.50/task vs ~$5.50/task).
- **4× Cheaper than Codex GPT-5.6 Sol**.

### 2. DeepSWE Benchmark (Together AI Research · Zain Hasan)
- **Single-shot**: GLM (69.0%) vs Claude Fable 5 (69.7%).
- **2 Candidates (Pass@2)**: GLM reaches **81.1%** (passing Fable 5's 77.1%).
- **4 Candidates (Pass@4)**: GLM hits **87.6%** (dominating Fable 5's 84.1%) at ~10× lower cost.

> Sources: [Together AI DeepSWE Research by Zain Hasan (@zainhas)](https://x.com/zainhas/status/2091297526347677701) and the [LLM-as-a-Verifier self-verification study](https://github.com/llm-as-a-verifier/llm-as-a-verifier#self-verification-terminal-bench-21).

---

## Verified Zero-Cost & Budget Routes (Tested August 2026)

Take advantage of promotional and zero-cost agent routes with zero credit card lock-in:

| Route | Model | Source | Cost / Quota |
|---|---|---|---|
| `ox_alpha_free` | `opencode/x-preview-f-free` — Ox Alpha Free | OpenCode Zen | **$0 (Unlimited)** |
| `ox_alpha_hermes` | `stealth/ox-alpha` — Ox Alpha Promo | Nous Portal via Hermes Agent | **$0 ("Quadrillion tokens/day")** |
| `gemini37_flash_medium` | Gemini 3.7 Flash (Medium) | Antigravity CLI | **$0 (Verified)** |
| `muse_spark_free` | `opencode/muse-spark-1.2-contributor-free` (1M ctx) | OpenCode Zen | **$0 (Free)** |
| `deepseek_v4_flash` | DeepSeek V4 Flash | OpenRouter / DeepSeek API | **~$0.05 / task** |
| `glm_53` | GLM 5.3 (`zai-coding-plan/glm-5.3`) | Z.AI Coding Plan via OpenCode | High-IQ Plan/Code |

Run 4 parallel zero-cost workers instantly:
```bash
cargo run --manifest-path rust/Cargo.toml -- run --force \
  --plan my_plan.json --global-max-concurrency 4 --provider-cap ox_alpha_free=4
```

---

## Core Capabilities & Features

### 🚀 Parallel Test-Time Scaling
Run N candidate solutions simultaneously in parallel across isolated Git worktrees.
1. **Objective First**: Automated tests (`pytest`, `cargo test`, linters) run per candidate. If exactly one candidate passes, it wins with **zero extra LLM calls**.
2. **LLM-as-a-Verifier**: On ties, a fast verifier model scores candidates.
3. **Escalation**: Ambiguous cases escalate to a synthesis or review route within strictly bounded token budgets.

### 🛡️ Anti-Slop Architecture & Role Specialization
- **Smart Planner**: Spend high-intelligence models (Claude Fable, GPT-5.6, GLM) strictly on formulating DAG workflow plans.
- **Static Critic**: Validates DAG dependencies, cycles, routes, and budget constraints *before* any execution begins.
- **Budget Programmer Workers**: Offload heavy coding sub-tasks to ultra-fast, zero-cost workers (Ox Alpha, DeepSeek V4 Flash, Gemini Flash).
- **Deterministic Verifier**: Grade code with objective compiler checks, unit tests, and SHA256 integrity hashes.

### 🔒 Zero Workspace Contamination
Every programmer worker operates inside a detached, temporary Git worktree. Changes are cryptographically verified with SHA256 pre/post signatures before being merged to the primary workspace.

### ⏱️ Runaway & Silent-Hang Protection
Active log watchers monitor worker stdout and file activity. If a task goes silent or hangs, SWARMS triggers immediate warnings and enforces timeouts, preventing zombie processes from burning your API credits.

### 🌐 Swarm-Driven Development (SwDD)
Integrate with [SwDD](https://github.com/Mchicao/swarm-driven-development) to connect OpenSpec specifications, SWARMS execution, Gentle-AI orchestration, and Engram memory behind a unified workflow:
$$	ext{Specification} \longrightarrow 	ext{Swarm Execution} \longrightarrow 	ext{Receipt-Backed Delivery}$$

---

## Quick Start in 30 Seconds

SWARMS is built in 100% native, self-contained Rust:

```bash
# 1. Check local environment and tools
cargo run --manifest-path rust/Cargo.toml -- doctor

# 2. Statically validate a workflow plan
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json

# 3. Dry-run without side effects
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_example.json --force

# 4. Execute with provider concurrency caps
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

---

## Supported Ecosystem & Integrations

- **CLIs & Agents**: Claude Code, Codex CLI, OpenCode, Kilo Code, Hermes Agent, Antigravity CLI.
- **APIs & Protocols**: OpenAI-compatible HTTP, LiteLLM gateways, OpenRouter, Z.AI, Nous Portal.
- **Offline / CI**: Self-contained `mock` provider for offline testing, demos, and CI/CD pipelines.
- **Observability & Telemetry**: Full token normalization, cache reads/writes, reasoning effort tracking, and JSON reports in `.agent/swarm/runs/<run_id>/`.

---

## Deep-Dive Technical Documentation

For developers seeking low-level runtime internals, schema specifications, and adapter implementation guides:

- [Rust Runtime Architecture](docs/RUST_RUNTIME.md) — Schedulers, locks, thinking levels, session affinity.
- [Parallel Test-Time Scaling Guide](docs/workflow_plan_scaling_example.json) — Scaled execution example with candidate rollouts.
- [Workflow State Contract](docs/STATE_CONTRACT.md) — Run-state JSON contracts and event schemas.
- [Provider & Route Configuration](docs/CONFIG.md) — Local overlays and provider limits.
- [Agent Standards & Prime Directives](AGENTS.md) — Guidelines for autonomous coding agents.

---

## License

MIT License. See [LICENSE](LICENSE) for details.
