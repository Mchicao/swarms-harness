# ADR: ACP transport and run-scoped Herdr surfaces

## Status

Accepted and implemented incrementally in the Rust runtime.

## Context

SWARMS already schedules workers through deterministic Rust code, but a
one-shot CLI process cannot receive a reliable direction while it is
generating. ACP (Agent Client Protocol) provides a supervised JSON-RPC stdio
session with streamed updates and cancellation. Herdr is a terminal workspace
manager; it should display worker activity without becoming the scheduler or
owning provider processes.

## Decision

The runtime keeps ownership of quotas, provider caps, locks, retries, worker
logs, artifacts, verification, session affinity, and telemetry. The selected
transport is additive:

- cli_batch keeps the existing one-shot adapter behavior.
- acp starts a configured ACP v1 agent, creates or loads a session, streams
  updates into worker.log, and supports steering by canceling the current
  turn before sending a continuation prompt.
- auto uses ACP when the wrapper has a safe ACP command and falls back to
  cli_batch when startup/session setup fails before a prompt is sent.
- HTTP OpenAI-compatible and mock routes retain their existing paths.

ACP agents run with stdin/stdout pipes and hidden process creation on Windows.
Provider stderr is appended to the worker log. A failed ACP turn after a
prompt has started is not duplicated through CLI fallback because that could
repeat edits.

Herdr is the default observation surface. A run creates one Herdr workspace rooted at
the project; each task stage gets a labelled tab, and each worker gets a
descriptively labelled pane that tails its own worker.log. Herdr server and
helper commands are hidden. If Herdr is selected but unavailable, the default
policy is hidden execution; native consoles are only used when explicitly
selected or explicitly allowed as the unavailable fallback.

## Configuration

Plans may override the defaults or make them explicit with:

~~~
{
  "execution": {
    "transport": "auto",
    "fallback": "cli_batch",
    "acp": {
      "command": "opencode",
      "args": ["acp"],
      "protocol_version": 1,
      "startup_timeout_seconds": 10,
      "cancel_grace_seconds": 5
    }
  },
  "terminal": {
    "backend": "herdr",
    "on_unavailable": "hidden",
    "workspace_scope": "run"
  }
}
~~~

For OpenCode and Kilo, the ACP subcommand is selected automatically when no
command override is supplied. Codex and the current gemini/agy wrapper
require an explicit ACP command because their current SWARMS CLI mappings
do not prove that the same executable exposes ACP. ACP-only plans are rejected
statically when no launch command can be constructed.

## Consequences

Steering is portable and observable, but it is cancel-then-continue rather
than character injection into an active generation. A provider that does not
implement ACP remains steerable only at its existing post-turn/session
boundary. Herdr does not need to be installed for headless operation.

## Rejected alternatives

- Moving scheduling into Herdr: loses deterministic quotas, retries, and
  persisted audit state.
- Opening one native console per worker: creates unrelated windows and makes
  the UI surface own process lifecycle.
- Blind CLI replay after an ACP failure: can duplicate partial edits.
- Treating all wrappers as ACP-capable: hides provider-specific launch
  contracts and makes auto unsafe.
