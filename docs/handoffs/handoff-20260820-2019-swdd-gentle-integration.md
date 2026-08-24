# Finish SwDD, SWARMS, and Gentle-AI integration

Complete SwDD v1 without weakening the authority boundaries defined in `C:\Proyectos\swarm-driven-development\docs\spec\`. Installation and offline execution now work on this Windows machine, but full RDD, capability negotiation, isolated integration, memory publication, and delivery remain incomplete.

## Start here

1. Read `C:\Proyectos\SWARMS\AGENTS.md` and `C:\Proyectos\swarm-driven-development\AGENTS.md`.
2. Inspect both dirty worktrees before editing. Preserve every existing change; do not reset, checkout, or overwrite concurrent scaling work.
3. Stabilize the SWARMS worktree enough to compile, then validate the Herdr patch already present in `rust/src/runtime.rs`.
4. Run the complete SwDD validation after reconciling its installer changes.
5. Implement the remaining lifecycle in the priority order below.

## Current machine state

| Component | Observed state |
| --- | --- |
| SwDD | `0.1.0`, installed in Cargo bin |
| SWARMS | `swarms-rs` installed from commit `05261185d11693dfda3852061ad7262e872f8e13` |
| Gentle-AI | `2.4.0`, five agents configured |
| Engram | `1.20.0`; duplicate binaries currently exist on PATH |
| OpenSpec | `1.10.0` |
| Go | `1.26.7` |
| GGA | `2.10.1` |
| Agent Runtimes | OpenCode, Codex, Claude Code, Gemini CLI, and Kilo detected |
| RDD | Globally `off`, decided by default |

Gentle-AI native installation completed with 232 checks passing. A later `gentle-ai doctor` reported 11 passed and one warning because Engram resolves from both `%LOCALAPPDATA%\engram\bin` and `%USERPROFILE%\go\bin`. Resolve the duplicate only after determining which installation owns current agent MCP paths.

The pre-install configuration backup is at `%LOCALAPPDATA%\SwDD\backups\20260820-125715`. Do not print or commit its contents.

## Work completed

### SwDD repository

Repository: `C:\Proyectos\swarm-driven-development`

- Added `scripts/install.ps1` as the local Windows bootstrap.
- `swdd install --apply --yes` installs pinned prerequisites and components.
- Installer pins Gentle-AI `v2.4.0`, Engram `v1.20.0`, OpenSpec `1.10.0`, and the SWARMS revision above.
- Installer now delegates full detected-agent configuration to `gentle-ai install --agents ... --scope global`.
- Bootstrap refreshes persistent Windows PATH and propagates all external command failures.
- `swdd` skills exist for all five detected runtimes.
- OpenSpec initialization is idempotent and receives the detected tool list.
- `examples/runbook.mock.json` now uses the deterministic SWARMS mock artifact and a Windows-native verification command.
- Installed-machine E2E passed: `init` twice, Runbook validation, SWARMS mock execution, artifact verification, and final SwDD status `deliverable`.

Important files:

- `C:\Proyectos\swarm-driven-development\src\installer.rs`
- `C:\Proyectos\swarm-driven-development\scripts\install.ps1`
- `C:\Proyectos\swarm-driven-development\examples\runbook.mock.json`
- `C:\Proyectos\swarm-driven-development\docs\implementation\status.md`

The SwDD worktree has roughly 408 additions and 92 deletions across nine tracked files, plus `scripts/`. Nothing is committed.

### Herdr behavior in SWARMS

User requirement: every worker in one run must share one Herdr workspace, stale completed-run workspaces must not accumulate, and later runs must reuse an existing visible Herdr client instead of opening another console.

The live dirty worktree contains a targeted patch in `rust/src/runtime.rs`:

- Herdr workspace registry keys always derive from `run_dir`, never `work_dir`.
- A run label is always `SWARMS | <run-id>`.
- All stages become tabs and all workers become panes in that run workspace.
- Completed runs close their unique Herdr workspace.
- Visible client launch no longer uses PowerShell `-NoExit`.
- A read-only Win32 process probe detects and reuses an existing client across coordinator processes.
- Regression tests cover run-scoped keys and absence of `-NoExit`.

Evidence from a clean baseline copy under `.cache/herdr-fix-check`:

- Four Herdr-focused tests passed.
- Clippy with warnings denied passed.
- A four-worker mock run completed with every task reporting `terminal_workspace_id: w1Q`.
- Herdr emitted one `workspace close` and the completed workspace disappeared.
- A second run reused the existing Herdr client PID rather than launching another.
- A serial full library run passed all 158 tests.
- Forty-one stale `SWARMS | ...` workspaces were closed; `manual-swarms` was preserved.
- The pre-cleanup workspace inventory is ignored at `.cache/herdr-workspaces-before-cleanup.json`.

## Dirty worktree warning

`C:\Proyectos\SWARMS` contains substantial concurrent scaling changes that predate the Herdr fix:

- Modified: `AGENTS.md`, `README.md`, router/docs, `rust/src/lib.rs`, `model.rs`, `review.rs`, `runtime.rs`, `telemetry.rs`, and `tests.rs`.
- Untracked: `rust/src/scaling.rs`, `docs/workflow_plan_scaling_example.json`, and generated `output/`.
- The live worktree previously failed compilation in `rust/src/scaling.rs` around lines 638-640 because `want` was borrowed and `.min()`/`.max()` received values; several `outcome_of` calls also passed `policy` instead of `&policy`.

Do not assume those compile failures belong to the Herdr patch. Confirm ownership and intent before modifying scaling code. The clean `HEAD + Herdr patch` copy is the evidence that the Herdr change itself is sound.

## Remaining release-critical work

### 1. Reconcile and validate both worktrees

- Make the live SWARMS scaling work compile without reverting concurrent changes.
- Run every command required by `C:\Proyectos\SWARMS\AGENTS.md`.
- Re-run the Herdr grouping/reuse integration on the live binary.
- Run SwDD format, Clippy, tests, release build, doctor, bootstrap idempotence, and installed E2E.
- Update `docs/implementation/status.md` only with fresh evidence.

### 2. Complete capability negotiation

SwDD currently detects executable names on PATH and reports every runtime as `capabilities unverified`. Implement versioned Capability Profiles that prove skill loading, MCP, delegation, session reuse, and required transports. Prefer consuming a public structured Gentle-AI inventory if available; do not infer capabilities from executable presence.

Primary seams:

- `C:\Proyectos\swarm-driven-development\src\doctor.rs`
- `C:\Proyectos\swarm-driven-development\src\installer.rs`
- `C:\Proyectos\swarm-driven-development\docs\spec\acceptance.md`

### 3. Complete isolated execution and integration

`runtime::develop` still rejects `Isolation::Worktrees`. Implement one worktree/ephemeral branch per writer, a distinct integrator, conflict enforcement inside the approved envelope, deterministic verification, and final history choices. Never degrade to parallel writers in one shared worktree.

Primary seam: `C:\Proyectos\swarm-driven-development\src\runtime.rs`.

### 4. Complete Gentle-AI RDD

SwDD currently reads native RDD status/next transition but does not drive the full negotiated reviewer collection, bounded correction, validation, finalization, receipt issuance, and delivery-gate validation. Gentle-AI must remain the only authority. Do not fabricate receipts, candidate hashes, transitions, runtime identities, or gate results.

RDD is globally off. Enabling it changes behavior across repositories and still requires explicit user confirmation.

Primary seams:

- `C:\Proyectos\swarm-driven-development\src\integrations.rs`
- `C:\Proyectos\swarm-driven-development\src\runtime.rs`
- Gentle-AI public review integration contract and `v2.4.0` CLI help/docs

### 5. Complete Project Memory publication

Implement deterministic binding from decoded Engram semantic content to frozen compressed chunks, secret-scan both forms, freeze a separate candidate, and run a separate Gentle-AI RDD lifecycle. Keep publication disabled if upstream contracts cannot prove the binding.

### 6. Complete adaptive Runbooks and delivery

- Add typed Worker change requests and native envelope validation for Conductor revisions.
- Implement separate immediate consent for commit, push, PR, and release.
- Offer squash, preserved commits, or uncommitted application.
- Revalidate exact candidate, receipt, remote, and base immediately before every delivery action.
- Finish remote checkpoint deletion and cleanup semantics.

### 7. Complete release certification

Run the full matrix in `C:\Proyectos\swarm-driven-development\docs\spec\acceptance.md` on disposable clean and upgrade Windows profiles. Include cross-machine checkpoint resume, version incompatibility, pre-existing agent config, full RDD, Engram Git sync, and secret-scanner evidence. No paid API may run without explicit authorization and a hard budget.

## Verification commands

SWARMS:

```powershell
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --release --manifest-path rust/Cargo.toml --all-features
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --run-id verify-agent --global-max-concurrency 3 --provider-cap mock=3
```

SwDD:

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release --all-features
swdd doctor
gentle-ai doctor
gentle-ai review mode status --cwd .
engram doctor
```

## Suggested skills

- `swarms`: operate and validate the native coordinator.
- `diagnosing-bugs`: preserve a tight repro for scaling, Herdr, and installer failures.
- `ponytail`: keep fixes minimal and rooted in existing public contracts.
- `verifying-completion`: require fresh evidence before status claims.
- `aikido-safe-chain`: protect any npm/package-manager operation.
- `domain-modeling`: update terms only if lifecycle concepts change.
- `cognitive-doc-design`: keep normative docs reviewable.
- `work-unit-commits`: split the final changes into reviewable commits once the user requests commits.

## Guardrails

- Never run paid providers without explicit authorization and a hard budget.
- Never push, merge, rebase, reset, or overwrite concurrent work without current-turn permission.
- Never commit local configuration, credentials, backups, `.agent/`, `.cache/`, `output/`, worker logs, or reports.
- Do not enable RDD globally without explicit user confirmation.
- Preserve the `manual-swarms` Herdr workspace.
- Treat clean-copy validation as evidence for the Herdr patch only, not for the dirty scaling worktree.
