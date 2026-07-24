# RFC: Runtime Supervision and Capability Enforcement

Status: Draft

Tracks: R-004, R-005, R-008, R-014

## Decision summary

SWARMS must supervise every external process through one cross-platform abstraction. Plans are executable input, not passive data. Process launch, deadlines, cancellation, output limits, retry classification, and dangerous capabilities must therefore be explicit and testable.

## Non-goals

- Pretending prompt instructions provide filesystem isolation.
- Automatically enabling operating-system sandboxing that is unavailable on every supported platform.
- Retrying configuration, authentication, policy, or deterministic verification failures.
- Passing provider prompts or credentials through telemetry.

## Process supervisor

Introduce a `ProcessSupervisor` used by provider adapters and verification commands.

```rust
pub struct ProcessSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env_allowlist: Vec<OsString>,
    pub stdin: ProcessInput,
    pub deadline: Option<Duration>,
    pub idle_deadline: Option<Duration>,
    pub output_limit_bytes: u64,
    pub tree_policy: ProcessTreePolicy,
}

pub enum ProcessInput {
    None,
    Bytes(Vec<u8>),
    File(PathBuf),
}

pub enum ProcessTreePolicy {
    TerminateOnDrop,
    DetachedExplicitly,
}
```

Prompts should use stdin or a protected temporary file rather than command-line arguments. This prevents prompt contents from appearing in process listings and avoids platform argument-length limits.

## Terminal outcomes

Every process returns one stable terminal reason:

```rust
pub enum ProcessTerminalReason {
    Exited { code: i32 },
    Signaled { signal: String },
    TimedOut,
    IdleTimedOut,
    Cancelled,
    OutputLimitExceeded,
    SpawnFailed,
    ProtocolFailed,
    TreeTerminationFailed,
}
```

The coordinator report must retain the terminal reason, elapsed duration, bounded stdout/stderr locations, and whether all descendants were confirmed terminated.

## Cross-platform process trees

- Windows: create workers in a Job Object configured to terminate contained processes when the job closes.
- Unix: create an isolated process group/session and signal the group, escalating from graceful termination to kill after a bounded grace period.
- Never claim successful cancellation until the process tree is reaped or a tree-termination failure is recorded.

## Deadlines and cancellation

- Restore `default_timeout_seconds` and task `timeout_seconds` as real execution deadlines.
- Add an independent verification deadline.
- Cancellation is cooperative at coordinator level and forceful at the process boundary.
- Steering must not reset the execution deadline unless the plan explicitly permits deadline extension and records it as an event.
- UI `stale` remains an observability signal; it is not itself a cancellation policy.

## Structured verification

Add a versioned structured form while retaining shell strings only as an explicit compatibility mode:

```json
{
  "verify": [
    {
      "program": "cargo",
      "args": ["test", "--manifest-path", "rust/Cargo.toml"],
      "cwd": ".",
      "timeout_seconds": 900
    }
  ]
}
```

Shell verification requires `allow_shell: true` at plan or task level and must be displayed prominently by `review` and `dry-run`.

## Capability policy

Replace free-form `tools_policy` strings with a tagged enum:

```rust
pub enum CapabilityPolicy {
    NoTools,
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}
```

Provider adapters map capabilities to verified flags. `FullAccess` must require both:

1. explicit plan authorization; and
2. an explicit CLI acknowledgement for a live run.

The review output must show the requested policy, effective adapter flags, workspace root, shell permission, network assumption, and whether any real sandbox is active.

## Error taxonomy and retry policy

```rust
pub enum RuntimeErrorKind {
    Configuration,
    Authorization,
    Policy,
    Quota,
    RateLimited { retry_after: Option<Duration> },
    NetworkTransient,
    ProviderUnavailable,
    Protocol,
    Timeout,
    Cancelled,
    Verification,
    Artifact,
    Persistence,
    Internal,
}
```

Retryable by default:

- rate-limited responses, respecting `Retry-After`;
- transient network failures;
- explicitly classified provider-unavailable failures;
- narrow, provider-specific lock contention.

Not retryable by default:

- invalid configuration;
- missing authentication;
- policy rejection;
- malformed provider protocol after a terminal event;
- deterministic verification failure;
- artifact containment violation;
- cancellation.

Every retry event records category, attempt, delay, provider hint, and previous terminal reason.

## Adapter decomposition

Split adapter responsibilities into:

- `adapters/mod.rs`: common interface and capability mapping;
- `adapters/codex.rs`, `opencode.rs`, `kilo.rs`, `hermes.rs`, `openai_compat.rs`;
- `process_supervisor.rs`: process lifecycle and bounded logs;
- `provider_protocol.rs`: typed event parsing;
- `retry.rs`: taxonomy and retry decisions;
- test fixtures outside production adapter modules.

## Migration sequence

1. Add typed terminal reasons and error categories without behavior changes.
2. Route mock and one CLI adapter through `ProcessSupervisor`.
3. Add cancellation and deadline tests.
4. Migrate remaining CLI adapters.
5. Migrate verification commands and add structured verification.
6. Introduce typed capability policy with compatibility parsing for old plans.
7. Remove direct `Command` process management from adapters/runtime.

## Required tests

- fake CLI that never exits;
- fake CLI that spawns a child and exits;
- fake CLI that ignores graceful termination;
- stdout/stderr output-limit enforcement;
- cancellation during startup, streaming, retry delay, and verification;
- prompt absent from process arguments and telemetry;
- Windows Job Object and Unix process-group behavior;
- `Retry-After` parsing with a fake HTTP server;
- policy rejection for shell and full-access execution;
- compatibility parsing for legacy `tools_policy` and string verification.

## Acceptance gate

This RFC is complete only when no provider or verification path launches a child process outside the supervisor and all terminal reports use the typed outcome/error model.
