# Implement ACP steering and grouped Herdr worker surfaces

This ExecPlan is a living document for the ACP transport, interactive steering,
and grouped Herdr terminal work. It follows the repository guidance in
`AGENTS.md`; the Rust runtime remains the public execution authority.

## Purpose / Big Picture

Users should be able to steer an active swarm task without relying on a
non-interactive CLI prompt. Providers that expose Agent Client Protocol (ACP)
will run through a supervised ACP session; providers without ACP support keep
the existing CLI batch path. The scheduler continues to own concurrency,
quotas, retries, artifacts, verification, and telemetry. On Windows, worker
surfaces selected for Herdr should appear as panes in one run workspace rather
than as unrelated native console windows. If Herdr is unavailable, the runtime
must fail closed to hidden execution instead of unexpectedly opening windows.

## Progress

- [x] (2026-08-05) Reconciled the checkout: `main` matched `origin/main`; preserved the two existing untracked directories.
- [x] (2026-08-05) Reviewed the ACP/Herdr design constraints and repository execution rules.
- [x] (2026-08-05) Added the additive ACP/terminal configuration contract and static validation.
- [x] (2026-08-05) Added a blocking ACP v1 stdio JSON-RPC client with initialization, sessions, updates, cancellation, and bounded startup.
- [x] (2026-08-05) Routed supported workers through ACP with pre-prompt CLI fallback while preserving scheduler ownership.
- [x] (2026-08-05) Exposed actual transport and ACP steering history through persisted task state and the existing UI.
- [x] (2026-08-05) Refactored Herdr surfaces to one workspace per run with split panes and hidden fallback.
- [x] (2026-08-05) Added focused ACP command/usage/parser tests and architecture/guide documentation.
- [x] (2026-08-05) Added a Windows PowerShell fake ACP peer test covering process launch, initialize, session/new, streamed update, prompt response, and shutdown; real providers remain uninvoked.
- [x] (2026-08-05 19:05) Ran the repository-required format, clippy, tests, release build, doctor, and example workflow checks successfully.
- [x] (2026-08-05 19:05) Recorded final outcomes and provider/Herdr live-validation limits below.
- [x] (2026-08-05) Grouped Herdr surfaces by task stage and persisted workspace/tab/pane identifiers.

## Surprises & Discoveries

- The repository already has a Herdr observer and per-worker pane path, plus a
  post-turn steering mechanism. The implementation can extend these paths
  instead of introducing a second UI runtime.
- ACP references are absent from the current Rust source, so protocol handling
  must be introduced without assuming an existing SDK dependency or provider
  session state.
- Herdr's current CLI supports pane splitting and returns the new pane at
  result.pane.pane_id; this permits a run-scoped workspace without taking UI
  focus.
- Herdr's workspace and tab creation responses expose `result.tab.tab_id` and
  `result.root_pane.pane_id`; these identifiers are sufficient to reuse a tab
  per stage without introducing a socket client.
- The current working tree contains unrelated untracked directories;
  implementation must not stage or modify them.

## Decision Log

- Decision: Keep transport selection in the Rust scheduler boundary and model
  ACP as an explicit transport alongside the existing CLI and HTTP adapters.
  Rationale: quotas, locks, retries, artifacts, and verification must remain
  deterministic and provider-independent.
- Decision: Implement the first ACP client over the documented stdio JSON-RPC
  shape with a narrow internal interface before adding an external SDK.
  Rationale: avoids an unverified async/dependency migration and keeps the
  process lifecycle controllable on Windows.
- Decision: Treat steering as cancel-then-prompt in the same ACP session.
  Rationale: ACP cancellation is portable, while injecting text into an active
  generation is not guaranteed by every provider.
- Decision: Herdr is a visual surface only. ACP processes are supervised by the
  runtime and panes display worker logs; Herdr failure uses hidden execution.
  Rationale: prevents terminal UI state from becoming scheduler state and avoids
  surprise native windows.
- Decision: Use one Herdr tab per exact task-stage key, rename the workspace's
  initial tab for the first stage, and create later tabs lazily.
  Rationale: avoids an unused bootstrap tab while keeping the hierarchy
  predictable as DAG waves reveal stages; pane labels remain bounded and safe
  for Herdr's metadata limits.

## Outcomes & Retrospective

Implemented and freshly validated: additive transport configuration, ACP v1
stdio process/session lifecycle, streamed log updates, cancel-then-continue
steering, transport telemetry, static ACP validation, run-scoped Herdr pane
creation grouped into stage tabs with descriptive labels, hidden Herdr
fallback, focused tests, and the existing mock workflow.

The Windows fake peer proves the client process protocol end to end without
calling a real provider. A live OpenCode/Kilo ACP session was not launched
because that would require provider authentication. Herdr was exercised with
an isolated mock run: the server created one workspace, one tab per stage,
descriptive pane labels, and persisted tab identifiers. Provider-specific ACP
support remains opt-in and should be certified on the user's machine with a
non-paid/local route before being treated as provider-specific acceptance
evidence.

The implementation stays on the requested branch and leaves unrelated
untracked directories untouched. No generated run artifacts or credentials are
part of the change.

## Context and Orientation

`rust/src/model.rs`, `config.rs`, and `review.rs` define plan/provider
configuration and static validation. `rust/src/adapter.rs` builds CLI and HTTP
provider invocations. `rust/src/runtime.rs` owns scheduling, subprocesses,
worker logs, steering, and Herdr panes. `rust/src/session.rs` stores reusable
provider session affinity. `rust/src/telemetry.rs` persists task and usage
state. `rust/src/ui_main.rs` observes runs, sends steering requests, and talks
to Herdr. The existing `worker_console_backend` and Herdr helper functions are
the starting point for grouped panes.

ACP means Agent Client Protocol: a line-oriented JSON-RPC protocol where the
runtime starts an agent process, creates or resumes a session, sends prompts,
and receives streamed session updates. A transport is the mechanism used to
execute one worker task; `cli_batch` is the existing one-shot process, while
`acp` is a persistent session capable of cancel-and-continue steering.

## Plan of Work

Additive configuration and validation now protect old plans. The ACP client is
implemented at the adapter/runtime boundary; streamed updates become worker
log records and steering uses cancel-then-continue in the same session. Herdr
uses one run-scoped workspace, creates panes as workers appear, and keeps
process supervision independent from pane lifetime. The provider-process
fake-peer certification runs under Windows tests; a real provider remains
intentionally uninvoked because local provider auth and paid API execution were
not requested.

## Concrete Steps

1. Inspect current model/config/review/runtime/session/UI contracts and add
   `execution.transport`, ACP limits, and terminal backend policy without
   breaking old JSON.
2. Add `rust/src/acp.rs` with bounded startup/read/cancel timeouts, JSON-RPC
   request IDs, initialization, session creation, prompt/update handling,
   cancellation, and supervised shutdown. Unit-test parsing and state
   transitions with an in-process fake message stream.
3. Add transport selection and provider capability mapping. `auto` uses ACP
   only when configured and supported; `cli_batch` remains the deterministic
   fallback. Never run paid or real provider commands during tests.
4. Integrate ACP worker execution and active-session steering into runtime
   state, session affinity, telemetry, and the UI command path.
5. Add a run-scoped Herdr manager and pane lifecycle. Reuse the existing viewer
   logs, hide helper processes, group panes by stage, label tabs/panes, and make
   Herdr-unavailable behavior explicit.
6. Add focused docs and tests, update this plan after each milestone, then run
   all repository-required checks and the example workflow. This step is now
   complete; the commands below are the reproducible acceptance record.

## Validation and Acceptance

From `C:\Proyectos\SWARMS`, run:

```powershell
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --release --manifest-path rust/Cargo.toml --all-features
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --run-id verify-agent --global-max-concurrency 3 --provider-cap mock=3
```

Acceptance requires: old plans review and run unchanged; ACP parser and
command-construction tests prove request/update/error behavior; an ACP-disabled
or unsupported route falls back to CLI without changing scheduler limits;
steering records the request and changes the active ACP session state; Herdr
mode uses one workspace per run and hidden fallback; no unrelated files are
staged. The Windows fake peer provides process-level protocol evidence while
avoiding real provider execution.

## Idempotence and Recovery

All configuration is additive and defaults to the current CLI/native behavior
unless the user explicitly selects ACP or Herdr. If an ACP child exits or
times out, close it, record the transport failure, and use the configured
fallback only before a session has produced a partial result; never duplicate a
completed task. A failed Herdr server must not stop the run and must not cause
native windows to appear. Reverting this branch removes the new transport and
terminal policy while leaving existing run artifacts untouched.

## Artifacts and Notes

The primary implementation artifacts are `rust/src/acp.rs`, the existing Rust
runtime/model/config/session/UI modules, focused tests, and this plan. Worker
logs and generated run reports remain runtime artifacts and must not be
committed.

## Interfaces and Dependencies

The ACP client must expose bounded `initialize`, `new_session`, `prompt`,
`cancel`, and `close` operations plus streamed update handling. Transport
selection must accept `auto`, `acp`, and `cli_batch`. The terminal policy must
accept `herdr`, `hidden`, and the existing native mode only when explicitly
requested. Dependencies should remain the existing Rust standard library and
already-approved crates unless a verified ACP SDK is demonstrably simpler and
compatible with the current toolchain.

## Plan revisions

- 2026-08-05: Created before implementation. Chose a narrow stdio client and
  run-scoped Herdr lifecycle to minimize dependency and architectural risk.
- 2026-08-05: Marked implementation milestones complete after cargo check and
  cargo test passed; retained fake-peer certification as the only remaining
  evidence item before final repository validation.
- 2026-08-05: Added lazy stage tabs and bounded descriptive pane labels using
  the official Herdr CLI surface; recorded tab identifiers in telemetry/UI.
