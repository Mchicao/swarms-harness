# SWARMS Native Runtime UI

Status: native Rust UI for Windows, Linux, and macOS. The optional `swarms-ui`
binary is built with the `ui-egui` feature.

The UI is an observer. It reads `.agent/swarm/runs/<run-id>/` and never claims
tasks, starts providers, or changes a workflow. Its only writes are explicit
user actions: an agent steering message and one of the feature-specific
Rulesync actions in the **Sync** view.

## Run

```powershell
cargo run --release --manifest-path rust/Cargo.toml --bin swarms-ui --features ui-egui -- --run-id <run-id>
```

Options:

- `--run-root <path>`: run root; defaults to `.agent/swarm/runs`.
- `--run-id <id>`: select a concrete run.
- `--ready-file <path>`: atomically writes a JSON ready signal after the window opens.
- `--bench-duration <seconds>`: exits after a controlled measurement interval.

## Navigation and visual language

The centered **Code**, **Swarms**, and **Sync** controls are distinct navigation
buttons. The native window and the product header use the Marraqueta toast mark
and a restrained toasted-bread palette (`#EBDFC2`, `#9C6620`, `#A8351A`,
`#5E7A24`). Herd uses the matching custom theme when its service is restarted.

- **Code** is a direct Herd terminal workspace reader. It refreshes the visible
  workspaces and the selected pane output every two seconds while open. `Open in
  Herd` is explicit and only focuses the selected workspace; it never starts,
  stops, or steers an agent.
- **Swarms** presents the project → run → stage → task hierarchy, a compact DAG,
  activity stream, quotas, task details, verification evidence, and a read-only
  worker log. Active tasks have a real-progress stale indication; stale is an
  observation only and never terminates a worker.
- **Sync** displays the global Rulesync rules used to generate agent
  configuration, not a duplicate Skillshare inventory. It has independent
  **Sync Skills**, **Sync MCP**, and **Sync AGENTS.md** actions. The source root
  is `SWARMS_RULESYNC_ROOT`, or the SWARMS workspace by default. Its source is
  `.rulesync/skills`, `.rulesync/mcp.json` or `.rulesync/mcp.jsonc`, and
  `.rulesync/rules`. No source tree is invented or initialized by the UI:
  actions remain disabled until a real Rulesync source is present.

  The repository owns the default source under `.rulesync/`. Its global
  generation configuration is `rulesync.jsonc`, with `delete: false`; syncing
  only writes Rulesync-managed artifacts and never deletes an unrelated global
  skill or agent configuration. The view therefore shows the source artifacts
  that are actually applied by Rulesync, rather than the separately managed
  Skillshare catalog.

## Herd worker terminals

Set `SWARMS_TERMINAL_BACKEND=herdr` to attach a real worker's read-only log view
to Herd. The coordinator remains the owner of the worker process. Herd panes
are closed when their worker finishes, so completed tasks do not leave empty
PowerShell consoles. `SWARMS_WORKER_CONSOLES=hidden` suppresses the legacy
Windows console viewer.

The task details view records the Herd session, workspace, and pane. A visible
workspace contains the current prompt and readable OpenCode JSONL events rather
than raw unreadable protocol text. The Code view remains useful even when no
SWARMS run is selected.

## Runtime observations

- Eframe uses the Glow renderer; WGPU, WebView, persistence, and raster assets
  are not included.
- Metadata polling is one second during an active run and five seconds while
  idle. The Herd Code view refreshes independently every two seconds.
- JSON is re-read only when the signature of workflow, tasks, claims, results,
  events, or reports changes.
- The run index refreshes every ten seconds, task rows are virtualized, the
  event buffer retains at most 500 entries, and a selected log is capped at its
  newest 256 KiB.
- Errors are truncated and token-like strings are sanitized before display.
- The UI reports observed quota snapshots only. It never opens OAuth, reads
  tokens, or invents availability.

## Steering

For active Codex, OpenCode, Kilo, or mock tasks, **Steer agent** queues up to
4,000 characters. CLI workers are turn-based: the request is consumed after
the current turn, resumed through the provider session when available, then
applied before artifact verification. History is persisted under
`steering/<task-id>/history.jsonl` as `applied`, `rejected`, or `failed`.

## Validate

```powershell
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --release --manifest-path rust/Cargo.toml --all-features
cargo tree --manifest-path rust/Cargo.toml --features ui-egui
```

The runtime includes automatic tests for read-only observation, limits,
steering persistence, shared-resource discovery, Herd workspace parsing, retry
telemetry, and the visible worker-console formatter. CI validates all features
on Windows, Linux, and macOS.

## Deliberate limits

- The UI does not start, stop, or resume workflows.
- It does not inject text into a provider turn already generating tokens.
- Hermes, Agy, OpenAI-compatible providers, and provider-internal subagents are
  not presented as steerable without a resumable session.
- It does not keep complete logs in memory or add a WebView/WGPU frontend.
