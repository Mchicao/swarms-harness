# Repository Review Remediation Backlog

This backlog records every distinct finding from the July 2026 architecture, code-quality, reliability, safety, testing, and maintainability review. Each item is intended to become a focused pull request or an explicitly documented design decision.

## P0 — correctness and production safety

### R-001: Establish one public runtime

**Finding:** Python remains the packaged public CLI while project documentation describes Rust as the exclusive runtime. The two implementations differ in scheduling, state, routing, timeout, resume, and review semantics.

**Acceptance criteria:**
- Rust is the only supported workflow runtime and public CLI.
- Python is reduced to launch/compatibility tooling or clearly isolated as legacy.
- Package metadata, installation instructions, examples, contributing guidance, and runtime docs agree.
- CI exercises the supported entry point end to end.

### R-002: Reject silently ignored router configuration

**Finding:** Router configuration documents fields such as preferences, variants, quality/cost/scarcity/health metadata, but Rust does not deserialize or implement several of them. Unknown fields are silently ignored.

**Acceptance criteria:**
- Configuration structs reject unknown fields unless a compatibility exception is intentional.
- Every documented field has implemented behavior and tests, or is removed/deprecated from docs and examples.
- Alias and fallback resolution is deterministic and cycle-safe.

### R-003: Implement the documented state contract exactly

**Finding:** `STATE_CONTRACT.md`, Rust task snapshots, event records, Python state, and UI reader expectations disagree. Promised hierarchy, dependency, retry, timestamp, and usage fields are missing or differently typed.

**Acceptance criteria:**
- Introduce versioned contract types shared by producers and readers.
- Rust state/events match the documented schema.
- Golden fixtures validate producer-to-UI compatibility.
- Schema changes require explicit versioning and migration guidance.

### R-004: Add process supervision, cancellation, and bounded execution

**Finding:** General workers and verification commands have no reliable timeout/cancellation/process-tree supervision. A hung wrapper or child process can block the coordinator indefinitely.

**Acceptance criteria:**
- Every worker and verification process has configurable deadlines and cancellation.
- Process trees are terminated cross-platform.
- Output is bounded/spooled safely.
- Terminal reason distinguishes timeout, cancellation, signal, protocol failure, and exit failure.
- Tests cover hung and child-spawning fake CLIs.

### R-005: Treat plans as executable code

**Finding:** Verification commands run through `sh -c`/`cmd /C`, and permissive adapter modes can enable unrestricted writes. Prompt-level workspace instructions are not an isolation boundary.

**Acceptance criteria:**
- Trust boundaries are documented prominently.
- Verification supports structured executable/argument forms; shell mode is explicit and opt-in.
- Capability policies replace stringly typed tool modes.
- Dangerous adapter flags require explicit authorization.
- Workspace/path constraints are enforced where technically possible and never represented as sandboxing when they are not.

## P1 — runtime reliability

### R-006: Replace wave scheduling with continuous scheduling

**Finding:** Rust launches a selected wave and waits for the entire wave before recalculating readiness, leaving capacity idle when task durations differ. Python already uses first-completed scheduling.

**Acceptance criteria:**
- New tasks start whenever dependencies and provider/global capacity permit.
- Provider caps remain enforced under concurrent completions.
- Deterministic tests cover heterogeneous task durations and dependency release.

### R-007: Stop swallowing persistence and audit errors

**Finding:** Event appends, prompt/directory writes, session persistence, steering audit writes, and malformed state reads are frequently ignored or defaulted.

**Acceptance criteria:**
- Critical writes fail the affected operation with actionable context.
- Non-critical telemetry failures are surfaced and counted.
- Corrupt state/session files are quarantined rather than silently treated as empty.
- Durable writes use collision-safe temporary files, flush/sync where required, and atomic replacement.

### R-008: Introduce typed domain errors and retryability

**Finding:** Retry logic treats most adapter failures as generic and retryable, obscuring configuration, authorization, protocol, quota, timeout, and permanent failures.

**Acceptance criteria:**
- Define typed error categories and stable terminal reasons.
- Retry policy is category-aware and respects provider retry hints such as `Retry-After`.
- Reports preserve root cause and retry history.
- Tests cover retryable and non-retryable failures.

### R-009: Tighten plan identity, dependency, alias, and route semantics

**Finding:** Dependency lookup can match slugified suffixes ambiguously; aliases resolve one level; fallback graphs are not fully validated; premium/cost classification relies on route-name substrings.

**Acceptance criteria:**
- Dependencies use exact canonical task IDs.
- Alias/fallback graphs resolve transitively with cycle detection and bounded depth.
- Cost/premium class is explicit typed metadata.
- Review output is deterministically sorted.

### R-010: Redesign workflow template interpolation

**Finding:** The workflow compiler treats remaining braces as unknown variables, making JSON, Rust, and other brace-heavy content difficult or impossible to embed safely. Condition expansion can also remove dependency edges.

**Acceptance criteria:**
- Use an unambiguous syntax such as `${name}` with escaping.
- Preserve dependency semantics when conditional steps produce no output.
- Emit a compiled-plan artifact and source mapping for diagnostics.
- Apply default tool policy consistently across schema versions.
- Add property/fuzz tests for interpolation and graph compilation.

### R-011: Make steering durable and collision-safe

**Finding:** Steering message IDs can collide within the same millisecond/process; rename-and-parse behavior can strand malformed batches; concurrent append/claim semantics are under-specified.

**Acceptance criteria:**
- Use collision-resistant IDs.
- Malformed entries are isolated without losing valid messages.
- Concurrent writers and claimers have deterministic, tested behavior.
- Audit persistence failures are visible.

### R-012: Make telemetry typed and completeness-aware

**Finding:** Usage values are strings including `"missing"`; aggregation can report partial numeric totals without clearly indicating incomplete coverage; hash-map output order may vary.

**Acceptance criteria:**
- Usage fields use typed optional numeric values.
- Reports include completeness, source, and task coverage counts.
- Partial totals cannot be mistaken for complete totals.
- Serialized output ordering is deterministic where humans or golden tests consume it.

## P2 — maintainability and architecture

### R-013: Split the runtime into cohesive modules

**Finding:** `runtime.rs` combines scheduling, process execution, persistence, checkpoints, prompts, verification, artifacts, retries, and reporting.

**Acceptance criteria:**
- Separate scheduler, executor/supervisor, persistence, checkpoint, prompt, verification, artifact, and retry responsibilities.
- Preserve behavior with focused unit and integration tests.
- Avoid circular dependencies and expose narrow interfaces.

### R-014: Split adapters and isolate production from fixtures

**Finding:** Provider adapters, protocol parsing, process launch, URL validation, and hard-coded mock benchmark behavior coexist in a large production module.

**Acceptance criteria:**
- Provider implementations and shared process/HTTP supervision are separated.
- Mock/benchmark fixtures move out of production adapter code.
- Protocol parsers have fixture-based tests.
- OpenAI-compatible HTTP calls have explicit timeout, cancellation, and fake-server tests.

### R-015: Align project versions and release semantics

**Finding:** Python and Rust package versions differ, and runtime ownership/release meaning is unclear.

**Acceptance criteria:**
- Define one release/version policy for the supported product.
- Package metadata and docs use consistent versions or explicitly documented independent versioning.
- Release automation validates version consistency.

### R-016: Reduce Ruff exclusions and legacy Python surface

**Finding:** Many scripts are excluded from Ruff, which permits quality drift in operational tooling.

**Acceptance criteria:**
- Reduce exclusions to generated/vendor/intentional legacy files only.
- Add typing/lint coverage incrementally for retained Python tools.
- Clearly mark removal timelines for obsolete runtime modules.

### R-017: Commit the Python lockfile when Python tooling remains supported

**Finding:** `uv.lock` is ignored, weakening reproducibility for retained Python tooling and CI.

**Acceptance criteria:**
- Stop ignoring `uv.lock` if Python remains part of the supported toolchain.
- Generate and commit the lockfile.
- CI installs from the lockfile and detects drift.

## P2 — tests, CI, dependency and release hygiene

### R-018: Expand Python and Rust version coverage

**Finding:** Python declares support for 3.10–3.12 but CI only runs 3.11; Rust does not declare/test an MSRV.

**Acceptance criteria:**
- Test Python 3.10, 3.11, and 3.12 or narrow declared support.
- Declare and test Rust MSRV.
- Keep platform matrix coverage for supported operating systems.

### R-019: Add dependency and supply-chain automation

**Finding:** No committed Dependabot/Renovate configuration was found; CI lacks `cargo audit`/`cargo deny`; third-party Actions are tag-pinned rather than SHA-pinned.

**Acceptance criteria:**
- Add dependency update automation.
- Add Rust advisory/license/source checks.
- Pin GitHub Actions to immutable SHAs with update comments.
- Document vulnerability response and update cadence.

### R-020: Add contract, corruption, concurrency, fault-injection, and fuzz tests

**Finding:** Existing tests are strong but do not fully cover state-contract fixtures, producer/UI compatibility, corrupt persistence, concurrent steering, cancellation, process trees, symlink artifacts, HTTP retry behavior, or parser/compiler fuzzing.

**Acceptance criteria:**
- Add golden contract fixtures and producer-to-reader tests.
- Add corrupt-state/session and interrupted-write tests.
- Add steering concurrency tests.
- Add fake hung/child-spawning CLI tests.
- Add fake HTTP server tests including `Retry-After`.
- Add symlink/path-containment tests.
- Add fuzz/property tests for workflow interpolation, paths, and provider event parsers.

### R-021: Add branch protection and required checks

**Finding:** Repository policy does not ensure that the existing CI gates must pass before merge.

**Acceptance criteria:**
- Require the supported CI jobs on the default branch.
- Require review and prevent force-push/deletion according to project policy.
- Document any admin bypass policy.

## Positive controls to preserve

The remediation work must preserve these strengths identified by the review:

- deterministic pre-execution plan review;
- fail-closed quota freshness/minimum checks;
- strict run-ID and force/resume CLI validation;
- artifact canonical-path containment checks;
- safe instruction/skill/MCP discovery without retaining secret values;
- cross-platform Rust CI with formatting, Clippy, tests, release build, doctor, review, dry-run, and mock end-to-end checks;
- offline/mock-first provider testing and the security reporting policy.
