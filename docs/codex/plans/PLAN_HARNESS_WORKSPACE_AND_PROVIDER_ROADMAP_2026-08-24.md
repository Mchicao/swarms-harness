# ExecPlan — workspace boundary, skill quality and provider roadmap

## Package `SWARMS-HARNESS-018`

- **Objective:** prevent a coordinator launched from the SWARMS repository from
  silently giving workers the wrong target repository or persisting run inputs
  outside the selected workspace.
- **Scope:** Rust CLI path resolution, run-state placement, focused tests,
  operator skills, and public provider-roadmap documentation. No real provider
  execution and no edits to dirty legacy Python worker files.
- **Observed failures:** a real run started without `--workspace-root` and
  workers inspected SWARMS instead of DataVIZ; a later run kept dependency logs
  under SWARMS while reviewers were sandboxed to DataVIZ.
- **Implementation:** fail closed when an executable plan is outside the current
  directory and no workspace is explicit; persist dry-run/run/singularity state
  under `<workspace-root>/.agent/swarm/runs`; document that dependent tasks use
  injected `needs` output or workspace-owned artifacts rather than external
  worker-log paths.
- **Future providers:** record OpenCode 2.0 and pi-agent as roadmap targets only,
  with no claim of current adapter or protocol compatibility.
- **Files:** `rust/src/main.rs`, `rust/src/cli.rs`, `rust/src/tests.rs`,
  `skills/swarms/SKILL.md`, `skills/multi-provider-agent-orchestration/SKILL.md`,
  `README.md`, `README.es.md`, `docs/PROVIDER_STATUS.md`, `docs/CONFIG.md`.
- **Execution ownership:** the coordinator edits and integrates directly; the
  SWARMS runtime is not used to execute this package. One bounded OpenCode
  worker using `venice/stealth-ox-alpha` may edit only `rust/src/tests.rs`.
  Its patch must receive a separate adversarial review with
  `venice/z-ai-glm-5-3` before integration. HY3 and silent model substitution
  are prohibited.
- **Validation:** format, warnings-denied Clippy, all-feature tests, release
  build, doctor and one offline mock run from a separate launcher directory
  against an explicit workspace.
- **Evidence:** CLI output plus a workflow snapshot whose `run_dir` and
  `workspace_root` both resolve under the target workspace.
- **Risk:** changing run location can make an old run appear missing if a user
  resumes from the launcher repository. For runs created by the OLD binary the
  state lives under the launcher even with `--workspace-root`, so no flag
  combination resumes them; the documented one-time recovery (see
  `docs/CONFIG.md`) is to move `<launcher>/.agent/swarm/runs/<id>` into the
  target workspace manually. Runs created by the new binary resume from the
  same workspace used by the original run.
- **Close when:** wrong-root execution fails before workers start, explicit-root
  execution stores state in the target, skills pass the repository style guide,
  and roadmap wording clearly separates future intent from implemented support.

## Result

- [x] Completed 2026-08-24. Fail-closed gate verified E2E: external plan without
  `--workspace-root` exits with the required-flag error; the explicit-workspace
  mock run `explicit-workspace-proof` finished `completed` with report and task
  evidence under the target workspace
  (`TableauToPBIP_V2/output/validation/swarms_workspace_boundary_mock_20260824/.agent/swarm/runs/explicit-workspace-proof/`).
- Adversarial GLM review (initial verdict NO PASS: 2xP1, 2xP2, 3xP3) addressed:
  workspace resolution now returns non-verbatim Windows paths (P1-1); the
  git-tracked `.skillshare/skills` copies were synced with the edited skills so
  agents load the new versions (P1-2); the one-time legacy-run migration is
  documented in `docs/CONFIG.md` and this plan's recovery note was corrected
  (P2-1); tests now route through `resolve_workspace_root`, assert no `\?\`
  leak, and cover the default launcher mapping and the `review` exemption
  (P2-2); the gated-command set lives in the shared `is_workspace_gated`
  predicate (P3-1).
- Validation: `cargo fmt`, 186 tests green (150 unit + 18 + 18).
