# Maintainability and Python Retirement Plan

Status: Draft

Tracks: R-001, R-013, R-014, R-016, R-017

## Objective

Reduce SWARMS to one production runtime with cohesive Rust modules and a deliberately small, reproducible Python support surface.

## Runtime module boundaries

The current runtime module owns too many responsibilities. Extract in behavior-preserving stages:

```text
rust/src/
  runtime/
    mod.rs             orchestration facade
    scheduler.rs       readiness, permits, completion events
    executor.rs        attempt lifecycle and task outcomes
    supervisor.rs      child process lifecycle/cancellation/output
    persistence.rs     snapshots, events, reports, atomic writes
    checkpoint.rs      plan identity and resume validation
    prompt.rs          prompt construction and redaction
    verification.rs    structured verification execution
    artifacts.rs       containment and artifact outcomes
    retry.rs           typed errors and retry policy
  adapters/
    mod.rs
    codex.rs
    opencode.rs
    kilo.rs
    hermes.rs
    openai_compat.rs
    mock.rs
```

Rules:

- modules communicate through typed domain objects, not shared mutable filesystem conventions;
- process launch exists only in the supervisor;
- persistence exists only behind a durable store interface;
- scheduler tests use fake executors and a deterministic clock;
- provider fixtures live under tests/fixtures, not production modules;
- the public runtime facade remains small enough to reason about lifecycle invariants.

## Extraction sequence

1. Extract artifact and verification helpers with existing tests unchanged.
2. Extract retry classification and terminal outcome types.
3. Extract persistence and checkpoint interfaces; add fault injection.
4. Extract process supervision and migrate mock plus one adapter.
5. Extract scheduler into a deterministic event loop.
6. Split provider adapters and protocol parsers.
7. Remove obsolete compatibility functions only after callers and fixtures migrate.

Each extraction PR must avoid feature changes unless the feature is required to define a clean boundary. Behavior changes receive separate tests and release notes.

## Python classification

Every Python file is assigned one disposition:

- **retain:** useful support tool with no duplicate Rust ownership;
- **migrate:** behavior belongs in Rust and has an explicit replacement milestone;
- **compatibility:** temporary launcher/reader for an older interface;
- **remove:** obsolete or unsafe duplication;
- **fixture:** test-only code moved under tests.

Initial classification target:

- `scripts/swarm.py`: compatibility/removal after Rust installation path is stable;
- `scripts/workflow_runtime.py`: migrate/remove because it is a duplicate scheduler/runtime;
- `scripts/plan_review.py`: migrate/remove because Rust owns plan review;
- provider worker scripts: retain only where an external CLI genuinely requires a Python wrapper, otherwise migrate into typed Rust adapters;
- benchmark, telemetry, migration, and synchronization utilities: retain when they are independent tools with owners and tests;
- mock worker logic: move to test fixtures unless required by the shipped offline demo.

## Public entry-point policy

- The product CLI is the Rust binary.
- Python packaging must not install a competing `swarms` command.
- Compatibility launchers must print a deprecation warning and delegate without reimplementing scheduling or plan semantics.
- Documentation examples must not use a legacy Python runtime.
- Removal follows one documented compatibility window and release note.

## Ruff and typing plan

1. Generate a report for every current Ruff exclusion with failure count and disposition.
2. Remove exclusions for files classified `retain` or `compatibility` after focused cleanup.
3. Move fixtures to test directories with appropriate per-file ignores rather than global exclusion.
4. Delete exclusions together with removed scripts.
5. Add a CI check preventing new broad exclusions without an explanatory comment.
6. Add type checking to retained operational modules after lint coverage is complete.

Quality gates for retained Python:

- Ruff check and formatting;
- Python 3.10–3.12 tests or narrowed support metadata;
- deterministic offline tests;
- no secrets or provider credentials;
- explicit CLI exit codes and error messages.

## Lockfile policy

If any Python tooling remains supported:

- `uv.lock` is committed;
- CI uses `uv sync --frozen` or an equivalently locked installation path;
- dependency updates are automated and reviewed;
- lockfile drift fails CI;
- Python support metadata and lockfile Python constraints agree.

If Python is fully removed from the supported toolchain:

- remove Python package metadata and its CI job;
- retain isolated migration scripts only when they have self-contained dependency instructions;
- do not commit an unused lockfile merely for historical code.

The current repository must choose one of these outcomes before R-017 is closed.

## Version and release ownership

- `rust/Cargo.toml` owns the product version.
- Retained Python support packaging either follows the exact product version or uses an explicitly independent support-tool package name/version.
- Release CI validates the selected relationship.
- Migration/removal notices identify the first release without the old Python entry point.

## Required tests

- Rust public CLI installation/smoke test on Windows, Linux, and macOS;
- compatibility launcher delegates arguments and exit codes exactly;
- no Python scheduler is invoked by product docs or package entry points;
- retained Python files are covered by Ruff and tests;
- module extraction preserves existing runtime golden behavior;
- dependency direction test or architectural lint prevents adapters/persistence from reaching scheduler internals.

## Completion criteria

- one public runtime;
- no duplicate Python scheduler, plan reviewer, or state producer;
- `runtime.rs` and `adapter.rs` replaced by cohesive modules with narrow interfaces;
- production code contains no hard-coded benchmark fixtures;
- every retained Python file is linted, tested, owned, and reproducibly installed;
- version and lockfile policy enforced by CI.
