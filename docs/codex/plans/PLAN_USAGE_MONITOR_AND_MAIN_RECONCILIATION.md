# Clone, integrate, and reconcile the usage monitor

This ExecPlan is a living document for cloning the user's usage monitor,
configuring Windows startup, making SWARMS quota display independent from the
monitor process, and reconciling the current local work with the latest
GitHub PR changes on `main`.

## Purpose / Big Picture

The user wants the existing `ai-usage-monitor` application to run
automatically when Windows starts, while SWARMS remains able to display quota
data independently when the monitor is unavailable. The local SWARMS checkout
must then be reconciled with the other agent's PR after reviewing the exact
changes, preserving compatible local work and leaving local and GitHub `main`
aligned.

## Progress

- [x] (2026-08-06) Confirmed the SWARMS checkout is on local `main` with an uncommitted `AGENTS.md` edit and no additional local worktree.
- [x] (2026-08-06) Located recent GitHub PRs, including the merged consolidation PR #36 and the merged DataViz cleanup PR #37.
- [x] (2026-08-06) Cloned `ai-usage-monitor` at commit `6ca4e4d` into `C:\Proyectos\ai-usage-monitor` and created its private `.venv`.
- [x] (2026-08-06) Registered and started the `AI Usage Monitor` Scheduled Task at user logon; task result was `0` and the venv `pythonw.exe` process was observed.
- [x] (2026-08-06) Analyzed PR #36 commit-by-commit and merged PR #36 plus PR #37 into local `main` without conflicts.
- [x] (2026-08-06) Verified the existing SWARMS quota reader is independent of the monitor process and reads the sibling snapshot path when the quota guard is disabled.
- [x] (2026-08-06) Pushed the reconciled local `main` to GitHub and confirmed local and remote SHA `3fea20c051b295611e74740921355ebe3e6824ed`.

## Surprises & Discoveries

- The network sandbox initially blocked GitHub API access; approved network access is required for PR inspection and cloning.
- Local `main` is one commit ahead of `origin/main` because the legacy branch/worktree cleanup was committed locally but not pushed.
- PR #36 is the other agent's relevant change: it adds live Codex App Server, OpenCode Server, and Claude stream-json transports, bounded steering continuation, child cleanup, and matching tests. PR #37 only removes DataViz-specific workflow archives.
- The monitor's first real snapshot contained `codex:Codex`; AGY and Z.AI were absent because those providers did not return data in that run. The correct UI state for absent providers is unavailable, not zero.
- The monitor's documented launcher assumed Python 3.10, but this machine's available Python was Anaconda 3.12. A private `.venv` was created and the launcher was pointed at its `pythonw.exe`.

## Decision Log

- Decision: Keep the usage monitor outside the SWARMS Git tree.
  Rationale: It is a separate application and must not become an accidental nested repository or untracked project payload.
- Decision: Preserve local `main` and reconcile changes there rather than creating a new default feature branch.
  Rationale: The project policy explicitly says normal SWARMS operation does not create branches automatically, and the user requested local `main`.
- Decision: Do not treat a missing monitor process as zero quota.
  Rationale: SWARMS must show the last valid snapshot or an explicit unavailable state.

## Outcomes & Retrospective

The monitor is cloned at `C:\Proyectos\ai-usage-monitor` and starts through
Task Scheduler using its private Python environment. SWARMS local and GitHub
`main` both resolve to `3fea20c051b295611e74740921355ebe3e6824ed` and contain
the merged PR #36/#37 changes plus the local cleanup and plan commits.
Provider quota coverage remains dependent on each provider's authentication
and response; the monitor currently reports Codex data and explicit errors for
AGY and Z.AI on this machine.

## Context and Orientation

SWARMS is the Rust-native coordinator and observer UI. Its quota display is in
`rust/src/ui_main.rs` and its quota model/loading code is in the Rust quota
modules. The external application is `Mchicao/ai-usage-monitor`; its source,
snapshot format, and startup entry point must be inspected before changing
SWARMS.

GitHub PR reconciliation must compare the current local `main`, `origin/main`,
and the selected PR's merge commit and changed files. Local `AGENTS.md` edits
must be preserved.

## Plan of Work

First clone the monitor into an external local directory and inspect its
README, startup scripts, output files, and quota snapshot producer. Next
configure a Windows Startup shortcut or equivalent user-level startup entry
using the monitor's documented entry point, then verify it without exposing
credentials.

In parallel, fetch the current SWARMS remote state and inspect the recent PR
patch and commits. Categorize each change as already present locally,
compatible and required, conflicting, or unrelated. Apply only the compatible
delta, resolve conflicts on local `main`, and add tests for any quota contract
change.

Finally run the required Rust validation, verify the quota UI with and without
the monitor snapshot, commit the reconciled local `main`, push it to GitHub,
and confirm the local and remote SHAs match.

## Concrete Steps

1. Clone the monitor outside the SWARMS repository and record its commit.
2. Read its documented startup and quota output contract; identify whether it
   writes `quota_snapshot.json`, another file, or exposes a service.
3. Create a user-level Windows startup entry that launches the documented
   monitor command with a hidden window where appropriate.
4. Fetch `origin/main` and the candidate PR refs; inspect commits and diffs.
5. Reconcile local `AGENTS.md` and existing Rust changes with the PR without
   discarding user work. This produced a clean local merge of `origin/main`.
6. Keep the independent SWARMS quota loading with a clear stale/unavailable
   state and no invented percentages; validate it against the monitor snapshot.
7. Validate the monitor startup, quota snapshot reading, Rust tests, release
   build, doctor, and mock workflow.
8. Commit on local `main`, push `HEAD:main`, and verify SHA equality.

## Validation and Acceptance

- The monitor clone exists outside the SWARMS Git tree and reports its exact
  commit and startup command.
- Windows startup contains one user-level `AI Usage Monitor` entry and
  launching it does not require an interactive terminal.
- SWARMS displays the last valid quota snapshot when the monitor is not
  running, and displays an explicit unavailable/stale state when no snapshot
  exists.
- `cargo fmt --check`, Clippy, all-feature tests, release build, `doctor`, and
  the mock workflow pass from `C:\Proyectos\SWARMS`; the monitor's six unit
  tests and Python compilation also pass.
- Local `main` and GitHub `origin/main` resolve to the same commit after push.

## Idempotence and Recovery

The monitor clone can be refreshed with `git fetch` and reset only after its
local status is confirmed clean. The startup entry must be updated in place,
not duplicated. SWARMS quota loading must tolerate a missing or malformed
snapshot without deleting it. Git reconciliation must stop on conflicts and
preserve the pre-merge checkpoint.

## Artifacts and Notes

The authoritative plan remains this file. Do not commit monitor credentials,
startup user data, quota snapshots containing sensitive account information,
or generated `.agent` telemetry.

## Interfaces and Dependencies

- SWARMS Rust quota model and UI.
- `Mchicao/ai-usage-monitor` snapshot producer and startup entry point.
- Windows per-user Startup or Run entry.
- GitHub `main` and the selected PR merge commit.
- Rust toolchain and the commands required by `AGENTS.md`.
