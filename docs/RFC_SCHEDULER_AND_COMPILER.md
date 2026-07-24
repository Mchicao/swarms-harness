# RFC: Continuous Scheduler and Deterministic Workflow Compiler

Status: Draft

Tracks: R-002, R-006, R-009, R-010

## Goals

- Keep all available global/provider capacity busy when runnable work exists.
- Make task identity and dependency resolution exact and unambiguous.
- Make router aliases/fallbacks deterministic and cycle-safe.
- Replace brace-sensitive interpolation with an explicit escaped syntax.
- Produce inspectable compiled-plan artifacts and source diagnostics.
- Reject configuration that the runtime does not understand.

## Continuous scheduler

The scheduler maintains these sets:

- `pending`: tasks not yet terminal;
- `ready`: dependency-complete tasks ordered by deterministic priority;
- `running`: active attempts with provider and global permits;
- `terminal`: succeeded, failed, blocked, or cancelled tasks.

It reacts to task completion, cancellation, steering, and quota/provider availability events. It must not wait for unrelated tasks in a previously selected wave.

Pseudo-loop:

```text
reconcile dependency outcomes
move newly runnable tasks into ready queue
start ready tasks while global and provider permits are available
if no tasks can start and running is non-empty, wait for first completion/event
if no tasks can start and running is empty, finish or report a blocked graph
repeat
```

Deterministic ready ordering:

1. stage index;
2. task source order;
3. canonical task ID.

Completion timing must not change which task wins a permit when multiple tasks become ready in the same reconciliation cycle.

## Capacity invariants

At all times:

- running task count does not exceed global cap;
- running tasks for a concrete effective route do not exceed its provider cap;
- permits are acquired before process launch and released exactly once;
- route fallback obtains a permit for the final effective route, not the requested alias;
- a task cannot have two active attempts;
- terminal dependency failure blocks dependents according to explicit policy.

## Canonical task identity

- `source_id` must be unique after workflow compilation.
- `task_id` is generated once from stable compiler order and source ID.
- `needs` resolves only exact `source_id` values during compilation and is rewritten to canonical `task_id` values in the compiled plan.
- Slug suffix matching is removed.
- Duplicate or missing dependencies are compile errors with source locations.

## Router contract

Router parsing is fail-closed:

- typed configuration structs use `deny_unknown_fields` except explicitly versioned extension objects;
- documented fields must be implemented or removed;
- `_schema`, `_doc`, and version metadata are explicitly modeled;
- aliases and fallbacks resolve transitively;
- cycles produce a configuration error showing the cycle path;
- maximum resolution depth is bounded defensively;
- every resolved target must exist;
- fallback chains are validated before a run;
- premium/cost policy uses typed metadata, never route-name substring matching.

The deterministic initial implementation does not score providers automatically. A task uses its explicit route, with validated ordered fallbacks only when policy permits. Any future scoring router requires a separate versioned decision policy and reproducibility tests.

## Template syntax

Use `${name}` for interpolation.

Escaping:

- `$${name}` emits the literal `${name}`;
- ordinary `{` and `}` characters are always literal;
- unknown `${name}` is a compile error;
- values are stringified only according to a declared variable type;
- no recursive interpolation unless explicitly specified and depth-bounded.

This permits JSON, Rust, shell, and template content without false unknown-variable errors.

## Conditional graph semantics

Every compiler step has an explicit output contract.

- A skipped condition emits a synthetic terminal `skipped` compiler node or rewrites downstream dependencies according to the declared branch join policy.
- Dependencies may never disappear implicitly because a condition produced zero tasks.
- Loop and map expansions preserve stable source paths and instance indices.
- Reduce steps depend on the exact expanded task IDs they consume.

## Compiled-plan artifact

`review`, `dry-run`, and `run` persist a canonical compiled plan before execution:

```json
{
  "schema": "swarms.compiled-plan",
  "schema_version": 1,
  "source_plan_sha256": "...",
  "router_sha256": "...",
  "tasks": [],
  "source_map": {}
}
```

The artifact includes:

- canonical task IDs and exact dependencies;
- requested and initially resolved routes;
- effective defaults for thinking, sessions, attempts, deadlines, tools, and verification;
- source locations/paths for generated tasks;
- compiler warnings and compatibility conversions;
- deterministic serialization and hash.

Resume validates this artifact/hash rather than reinterpreting changed source plans silently.

## Configuration compatibility

- Existing schema-v1 plans receive a compatibility conversion into the canonical compiled representation.
- Existing brace interpolation is accepted only behind a temporary compatibility flag and emits a deprecation warning.
- Existing free-form tool strings are parsed into a typed capability policy or rejected.
- Router metadata currently unused by Rust is either modeled as informational-only with explicit documentation or removed from committed examples.

## Required tests

Scheduler:

- heterogeneous durations keep all capacity occupied;
- tasks becoming ready after one completion start before unrelated running tasks finish;
- global/provider caps under simultaneous completion;
- fallback route permit accounting;
- deterministic launch ordering;
- dependency failure and cancellation propagation.

Compiler/router:

- exact dependency resolution and duplicate IDs;
- alias/fallback chains and cycles;
- unknown configuration fields;
- JSON/Rust content containing braces;
- `${name}` substitution and `$${name}` escaping;
- skipped conditional branch joins;
- loop/map/reduce stable IDs;
- deterministic compiled artifact hashes;
- property tests for arbitrary strings and bounded workflow graphs.

## Migration sequence

1. Add compiled-plan type and exact dependency validation.
2. Add `${name}` interpolation and compatibility warnings.
3. Add strict router structs and graph validation.
4. Replace wave scheduler with a permit/event-driven loop.
5. Rewrite state snapshots to store canonical dependencies and compiled-plan identity.
6. Remove suffix dependency matching and legacy interpolation.

## Acceptance gate

The finding is closed only when runtime execution consumes the persisted compiled plan, the scheduler is event-driven/continuous, and all committed router fields have enforced semantics.
