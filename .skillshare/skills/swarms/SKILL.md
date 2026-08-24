---
name: swarms
description: "Trigger: SWARMS plan, run, resume, observe, steer, provider route, or Rust runtime. Operate the native coordinator with bounded workspace ownership."
license: MIT
metadata:
  author: SWARMS
  version: "2.0"
---

## Activation Contract

Use for SWARMS workflow contracts and operations. Read `AGENTS.md` only when
editing the Rust runtime/UI; workbook operation uses this skill and references.

## Hard Rules

- Use the Rust binary; Python is legacy benchmark/telemetry code.
- Pass the exact target repo with `--workspace-root` whenever the plan is not
  inside the launcher repo. Run state belongs to that target workspace.
- Use `needs` for dependencies. Dependents receive completed readable output;
  never tell them to read another repo's `worker.log`. Put reusable inputs in
  declared workspace-owned artifacts.
- Use mock unless real configured providers are explicitly authorized.
- Treat `run` as blocking. Do not poll its files while it is active.
- Never commit credentials, local routers, `.agent/`, prompts, logs or reports.
- OpenCode 2.0 and pi-agent are roadmap targets, not supported routes today.

## Decision Gates

| Situation | Action |
| --- | --- |
| Plan outside current repo | Require `--workspace-root <target>` |
| Parallel writers | Separate worktrees or disjoint writable paths |
| Prior run interrupted | Use `--resume`; never combine with `--force` |
| Route unavailable | Report blocker; do not silently substitute |
| Runtime/UI code changes | Load `AGENTS.md` and run every Rust gate |

## Execution Steps

1. Define goal, bounded tasks, routes, `needs`, artifacts, tools policy and
   deterministic `verify` commands.
2. Run `doctor`, `review`, then `dry-run` with the same explicit workspace and
   router intended for execution.
3. Run one coordinator with explicit global/provider caps.
4. After it exits, inspect terminal report, task states, readable logs and
   required artifacts. Distinguish requested from effective routes.
5. For UI observation, launch `swarms-ui` separately against the target run.

## Output Contract

Return target workspace, run id/path, requested/effective routes, task states,
validation evidence, fallback/blocking and remaining risk.

## References

- `../../AGENTS.md` — runtime development contract.
- `../../docs/CONFIG.md` — router and workspace configuration.
- `../../docs/STATE_CONTRACT.md` — persisted run contract.
- `../../docs/PROVIDER_STATUS.md` — implemented versus future providers.
