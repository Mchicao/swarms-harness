# Roadmap

SWARMS is a public release of a personal workflow that has been in use since January-February 2026. The current public goal is to keep the offline path reproducible while making real-provider routes explicit and local-only.

## Phase 0: Public Release

- Keep `mock` as the only enabled provider in committed config.
- Maintain the single public CLI in `scripts/swarm.py`.
- Keep CI fully offline.
- Document provider status and limitations honestly.
- Make plans reviewable before runtime execution.

## Phase 1: Provider Adapter Hardening

- Tighten adapter contracts for GLM 5.2, Gemini Flash, Codex, and local shell verification.
- Require explicit local configuration before any real provider call.
- Keep premium routes blocked unless `premium_allowed=true` and local config enables them.
- Record token usage as `api_reported`, `cli_reported`, `estimated`, or `missing`.

## Phase 2: Safer Parallel Coding

- Add worktree setup and cleanup as first-class runtime operations.
- Prevent writes outside declared artifact paths where practical.
- Improve conflict detection before merging worker outputs.
- Add richer reports for blocked, failed, and retried tasks.

## Phase 3: Budget Enforcement

- Enforce per-run provider caps, token caps, and cost caps.
- Add per-role budget rules for planner, critic, programmer, and verifier.
- Add provider health checks that can disable routes automatically.

## Phase 4: Better Planning UX

- Provide plan templates for common coding workflows.
- Add plan linting messages that are easier for humans to fix.
- Add bilingual examples for English and Spanish users.

## Non-Goals For Now

- Replacing Claude Code, Codex, or other full coding agents.
- Calling paid APIs by default.
- Hiding provider cost or missing telemetry.
- Pretending real-provider cost is predictable without telemetry.
