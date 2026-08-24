---
name: multi-provider-agent-orchestration
description: "Trigger: delegate across Codex, OpenCode, Gemini, GLM or other agents. Design bounded provider handoffs and evidence-first integration."
license: MIT
metadata:
  author: SWARMS
  version: "2.0"
---

## Activation Contract

Use to design delegation independently of a scheduler. Use `$swarms` only when
the approved contract will execute through this repository's runtime.

## Hard Rules

- Give every task an objective, acceptance criteria, exact repo/workspace,
  writable paths, provider/model, dependencies, artifacts and verification.
- Isolate concurrent writers with worktrees or disjoint paths.
- Verify each CLI's current `--version` and `--help`; never infer flags from a
  different major version.
- Never silently substitute a blocked provider or expose credentials.
- Pass predecessor conclusions through structured handoff/artifacts, not raw
  external logs that the reviewer cannot access.
- OpenCode 2.0 and pi-agent are future integration targets only until their
  protocols, sessions, steering, usage and safety boundaries are validated.

## Decision Gates

| Task shape | Route intent |
| --- | --- |
| Inventory/docs/narrow tests | Fast low-cost agent or local tool |
| Multi-file diagnosis/implementation | Deep reasoning agent |
| Security/architecture/integration | Independent critic; premium only if authorized |
| Deterministic acceptance | Local test/build/lint before model judgment |

## Execution Steps

1. Record a common base and non-overlapping ownership.
2. Verify provider availability, exact model and quota before dispatch.
3. Keep scopes bounded; use activity/checkpoints, not blind timeouts, to judge
   progress.
4. Review each handoff against its contract, then integrate and run targeted
   followed by full gates.
5. Inspect ancestry, conflicts, `git diff --check` and final worktree state.

## Output Contract

Return provider/model actually used, ownership, completed/failed/blocked state,
changed artifacts, commands/results, fallbacks and unresolved risk.

## References

- `../swarms/SKILL.md` — native execution lifecycle.
- `../../docs/PROVIDER_STATUS.md` — provider support boundary.
- `../../docs/ADD_A_PROVIDER.md` — adapter requirements.
