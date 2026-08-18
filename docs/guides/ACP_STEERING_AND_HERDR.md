# ACP steering and grouped Herdr workers

## What this enables

SWARMS can keep a provider session alive and accept steering from the existing
UI mailbox while the agent is working. Generic ACP remains available, while
optional live transports cover Codex App Server, OpenCode Server, and Claude's
streaming CLI. Providers without a live transport continue through the
existing CLI batch adapter.

When Herdr is selected on Windows, one run owns one Herdr workspace. SWARMS
creates one tab per task stage and places workers from that stage in split
panes. Tabs are labelled `Phase | <stage>` and panes are labelled with the
task id, role, provider, and model. Herdr only tails worker.log; Rust still
starts, supervises, retries, verifies, and records the provider process. The
workspace, tab, and pane identifiers are persisted in task telemetry so the
UI can show the exact Herdr surface used by a worker.

## Plan configuration

Interactive ACP-or-CLI transport and the run-scoped Herdr surface are now the
SWARMS defaults. Add the following fields when a plan should make those
defaults explicit or override them per run:

~~~
{
  "execution": {
    "transport": "auto",
    "fallback": "cli_batch",
    "acp": {
      "command": "opencode",
      "args": ["acp"]
    }
  },
  "terminal": {
    "backend": "herdr",
    "on_unavailable": "hidden",
    "workspace_scope": "run"
  }
}
~~~

The timeout and protocol fields are optional and default to ACP v1, a
10-second startup timeout, and a 5-second cancel grace period. auto only
selects ACP for wrappers with a known launch command; acp makes the absence of
that command a review error. Set transport to cli_batch to preserve legacy
behavior for a particular plan.

## Operating commands

Review and run through the Rust binary:

~~~
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
~~~

For Herdr, ensure herdr.exe is available on PATH or set SWARMS_HERDR_BIN.
SWARMS_HERDR_SESSION can select the Herdr session; it defaults to swarms. ACP
provider processes remain hidden. SWARMS launches the Herdr server hidden and
opens one visible Windows Terminal client titled `Herdr | <session>`; if
Windows Terminal is unavailable, it falls back to a new PowerShell console.
Headless environments keep the server-only behavior and retain the pane ids in
telemetry.

The UI's Send steer control writes a bounded request to
steering/<task-id>/inbox.jsonl. The selected delivery mode is persisted with
the request:

- `immediate` asks the live provider to steer at its earliest safe boundary.
- `enqueue` lets the current turn finish and sends a continuation afterwards.
- `cancel_and_restart` interrupts the active turn and restarts from the saved
  task prompt plus the new direction.

The runtime records `accepted` when a provider acknowledges an immediate or
cancel request, and records `applied` only after queued work actually runs.
Other terminal results are `rejected` or `failed`.

## Optional live transports

Plan configuration remains portable. The generic ACP path is selected by
default when the wrapper exposes a safe launcher; provider-specific live
transports remain opt-in environment switches and only apply while
`execution.transport` is `auto`.

| Provider | Enable | Provider mechanism |
| --- | --- | --- |
| Codex | `SWARMS_CODEX_APP_SERVER=1` | `codex app-server --stdio` |
| OpenCode | `SWARMS_OPENCODE_SERVER=1` | OpenCode HTTP server and event stream |
| Claude | `SWARMS_CLAUDE_STREAMING=1` | `claude --input-format stream-json` |

Codex uses `turn/steer` for immediate delivery and `turn/interrupt` before a
cancel-and-restart. OpenCode sends asynchronous session prompts and aborts the
active session for restart. Claude keeps its stream-json process alive and
sends another input turn. All three preserve their provider session identifier.

AGY remains on its stable CLI adapter. A local SDK bridge prototype was not
retained because it did not match the currently published SDK interface and
could not be validated as a reliable transport.

## Safety and limitations

ACP cancellation is cooperative. No transport guarantees that text is injected
into a generation already in progress; `immediate` therefore means the
earliest provider-safe boundary, not arbitrary mid-token injection. If ACP
fails before a prompt is sent, auto may use the configured CLI fallback. Once
a prompt has started, SWARMS does not replay the task through CLI because the
agent may already have modified files.

If Herdr is unavailable, the default is hidden execution. Use
on_unavailable: native only when opening a separate Windows console is
intended.
