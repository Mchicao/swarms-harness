# ACP steering and grouped Herdr workers

## What this enables

SWARMS can keep a provider session alive through ACP and accept steering from
the existing UI mailbox while the agent is working. The runtime cancels the
active ACP turn, sends the new direction in the same session, and records the
request in the task history. Providers without ACP continue through the
existing CLI batch adapter.

When Herdr is selected on Windows, one run owns one Herdr workspace. SWARMS
creates one tab per task stage and places workers from that stage in split
panes. Tabs are labelled `Phase | <stage>` and panes are labelled with the
task id, role, provider, and model. Herdr only tails worker.log; Rust still
starts, supervises, retries, verifies, and records the provider process. The
workspace, tab, and pane identifiers are persisted in task telemetry so the
UI can show the exact Herdr surface used by a worker.

## Plan configuration

Add the following fields to a plan when interactive ACP and Herdr are desired:

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
provider processes remain hidden, while the Herdr server is launched hidden as
well.

The UI's Send steer control writes a bounded request to
steering/<task-id>/inbox.jsonl. An ACP worker drains it during streamed
updates or its short polling interval. The applied/rejected/failed result is
recorded in history.jsonl and surfaced in the task detail view.

## Safety and limitations

ACP cancellation is cooperative. The protocol does not guarantee that text
can be injected into a generation already in progress, so SWARMS uses
cancel-then-continue. If ACP fails before a prompt is sent, auto may use the
configured CLI fallback. Once a prompt has started, SWARMS does not replay the
task through CLI because the agent may already have modified files.

If Herdr is unavailable, the default is hidden execution. Use
on_unavailable: native only when opening a separate Windows console is
intended.
