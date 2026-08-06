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
- [ ] Clone and inspect `ai-usage-monitor`.
- [ ] Configure and verify Windows startup for the monitor.
- [ ] Analyze the selected PR commit-by-commit and reconcile it with local SWARMS changes.
- [ ] Make SWARMS quota reading independent from the monitor process.
- [ ] Run full validation, commit compatible changes on local `main`, and push the final `main` to GitHub.

## Surprises & Discoveries

- The network sandbox initially blocked GitHub API access; approved network access is required for PR inspection and cloning.
- Local `main` is one commit ahead of `origin/main` because the legacy branch/worktree cleanup was committed locally but not pushed.
- The exact PR still needs to be determined from the recent merged PRs; #36 is titled `feat: consolidate local steering advances`, while #37 is a DataViz cleanup.

## Decision Log

- Decision: Keep the usage monitor outside the SWARMS Git tree.
  Rationale: It is a separate application and must not become an accidental nested repository or untracked project payload.
- Decision: Preserve local `main` and reconcile changes there rather than creating a new default feature branch.
  Rationale: The project policy explicitly says normal SWARMS operation does not create branches automatically, and the user requested local `main`.
- Decision: Do not treat a missing monitor process as zero quota.
  Rationale: SWARMS must show the last valid snapshot or an explicit unavailable state.

## Outcomes & Retrospective

Pending implementation. This section will record the final startup command,
quota data path, selected PR commits, merge result, and any remaining provider
quota limitations.

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
   discarding user work.
6. Implement independent SWARMS quota loading with a clear stale/unavailable
   state and no invented percentages.
7. Validate the monitor startup, quota snapshot reading, Rust tests, release
   build, doctor, and mock workflow.
8. Commit on local `main`, push `HEAD:main`, and verify SHA equality.

## Validation and Acceptance

- The monitor clone exists outside the SWARMS Git tree and reports its exact
  commit and startup command.
- Windows startup contains one user-level entry for the monitor and launching
  it does not require an interactive terminal.
- SWARMS displays the last valid quota snapshot when the monitor is not
  running, and displays an explicit unavailable/stale state when no snapshot
  exists.
- `cargo fmt --check`, Clippy, all-feature tests, release build, `doctor`, and
  the mock workflow pass from `C:\Proyectos\SWARMS`.
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
