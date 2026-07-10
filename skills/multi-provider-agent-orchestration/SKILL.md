---
name: multi-provider-agent-orchestration
description: Orchestrate heterogeneous coding agents across GLM, Gemini, Codex and similar providers with deterministic plans, bounded tasks, isolated workspaces, timeout budgets, evidence and integration gates.
version: 1.1.0
author: Hermes Agent
license: MIT
platforms: [windows, linux, macos]
metadata:
  hermes:
    tags: [multi-agent, orchestration, swarms, glm, gemini, codex, worktrees, verification]
    related_skills: [opencode, codex, zai-coding-plan, plan, test-driven-development]
---

# Multi-Provider Agent Orchestration

Use this skill when one engineering goal should be split across different model/provider strengths, especially repository-wide audits, migrations, feature programs, or parallel fixes.

## Source of Truth and Synchronization

This repository directory is the canonical source. Synchronize the installed Hermes skill with:

```powershell
python scripts/sync_multi_provider_skill.py
python scripts/sync_multi_provider_skill.py --check
```

The first command copies this complete tree to the active user's Hermes skills directory. The second exits non-zero if the installed copy has drifted. Update the repository copy first, validate it, then synchronize; do not maintain two independent versions.

## Core Principle

The model proposes and implements bounded work. A deterministic coordinator owns:

- task dependencies and provider caps;
- isolated worktrees/workdirs;
- prompt, status, log and result artifacts;
- retries and timeout budgets;
- integration order;
- local tests and final acceptance.

Never let several agents edit the same working tree concurrently. If the runtime does not actually create isolated worktrees, parallel tasks must have disjoint writable file sets or run sequentially.

## Provider Assignment

Assign work by task shape, not prestige:

- **Fast/cheap model (for example Gemini Flash):** narrow frontend review, documentation, repetitive tests, summaries, clearly specified UI changes.
- **Slow/deep model (for example GLM 5.2):** backend rules, security, multi-file reasoning, difficult debugging. Split broad audits into bounded domains.
- **Scarce premium model (for example Codex/GPT high reasoning):** architecture critic, integration review, security-sensitive edge cases and final code review.
- **Local deterministic tools:** builds, tests, linters, schema validation and smoke checks.

If a requested provider is quota-blocked, record the blocker and continue independent streams. Do not silently substitute a different model while claiming the requested one ran.

## Required Workflow

1. **Checkpoint and inspect**
   - Verify a clean git state or create a dedicated integration branch.
   - Read repository instructions and specification.
   - Record baseline build/test/typecheck results.
   - Completion: branch, working-tree state and baseline commands are recorded.

2. **Merge or establish the common base before auditing gaps**
   - Fetch every source branch with authenticated credentials kept out of URLs and logs.
   - Check ancestry and merge base; do not assume one branch is based on another because its tree looks similar.
   - Resolve conflicts semantically, then build before committing the merge.
   - Completion: merge commit exists and conflict-marker/build checks pass.

3. **Create a structured workflow plan**
   - Each task has: id, role, route/model, exact scope, dependencies, allowed artifacts, tools policy and verification.
   - Run static review and dry-run before real providers.
   - Keep provider concurrency explicit.
   - Completion: review is `ok`, dry-run is `planned`, and every task has non-zero provider capacity.

4. **Bound tasks by model speed**
   - Avoid prompts such as “audit the whole repo” for slow workers.
   - Prefer one domain per task: authorization/tenant isolation, state machine, SLA/time, frontend contracts, migrations, or reporting.
   - Give exact paths and a strict output budget.
   - Completion: each worker can finish within its inner timeout without relying on unspecified context.

5. **Probe local CLI contracts**
   - Run `<tool> --version` and the relevant subcommand `--help`.
   - Approval flags and config keys can change between releases; select only flags shown by local help.
   - For Codex, forward the requested `--model`; do not treat it as informational.
   - Codex reasoning uses the locally documented config key (commonly `model_reasoning_effort`), verified from local config/help rather than guessed.
   - Completion: effective provider/model/effort appear in real runtime output or a bounded smoke check.

6. **Budget nested timeouts correctly**
   - Inner provider timeout < outer worker timeout < orchestrator/job timeout.
   - Fast bounded tasks: about 180–300 s.
   - Slow deep tasks: 600 s or more.
   - A timeout should trigger scope reduction or a larger verified budget, not an unsupported claim that the provider is broken.
   - Completion: all three timeout layers are explicit and monotonically increasing.

7. **Integrate through gates**
   - Review each result against the specification before accepting it.
   - Run targeted tests, then full build/lint/typecheck.
   - Inspect `git diff --check`, conflict markers and working tree state.
   - Commit one logical integration at a time.
   - Completion: every modified file is accounted for and deterministic gates have real outputs.

8. **Report evidence**
   - State which model actually completed each task.
   - Include failed/timed-out tasks separately.
   - Provide concrete branch/commit paths and real command outcomes.
   - Completion: report distinguishes completed, failed, blocked and skipped work without inferred success.

## Secure Git Authentication

Keep tokens in an ignored environment file or secret store. For a one-shot private fetch, prefer a temporary `GIT_ASKPASS` helper that reads the token from an environment variable. Disable unrelated credential helpers for that invocation. Never embed tokens in remote URLs, command output, commits or plans.

## Common Pitfalls

- Treating tree similarity as proof of git ancestry.
- Running a gap audit before merging the requested branches.
- Giving a slow reasoning model one repository-wide task with a 300-second timeout.
- Using the orchestrator repository as `cwd` when the worker needs the target repository.
- Claiming isolated worktrees when the runtime only isolates logs/result directories.
- Hardcoding a headless approval flag from another CLI version.
- Letting agent-supplied “success” replace local verification.
- Re-invoking a timer `start` endpoint as a heartbeat; heartbeat requires an idempotent endpoint or lease update.
- Accepting fast-model recommendations without validating API semantics.

## Verification Checklist

- [ ] Common base and branch ancestry verified
- [ ] Integration branch/checkpoint created
- [ ] Baseline captured
- [ ] Static plan review passed
- [ ] Dry-run passed
- [ ] Provider/model/effort confirmed from real runtime output
- [ ] Independent tasks use isolated workspaces or provably disjoint writable scopes
- [ ] Every worker has a bounded scope and timeout budget
- [ ] Results reviewed for spec compliance
- [ ] Targeted and full validations pass
- [ ] Installed skill passes `python scripts/sync_multi_provider_skill.py --check`
- [ ] Final report distinguishes completed, failed and skipped work

## References

- `references/transafeticket-swarms-lessons.md` — concrete merge, authentication, worker timeout and audit lessons from a multi-provider run.
