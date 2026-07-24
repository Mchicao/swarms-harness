# Repository Governance

Status: Proposed

Tracks: R-018, R-019, R-021

## Default branch policy

After the reviewed baseline PR is merged, configure `main` with a branch ruleset or branch protection rule that:

- requires a pull request before merge;
- requires at least one approving review when more than one maintainer is available;
- dismisses stale approvals after code changes;
- requires conversation resolution;
- requires branches to be up to date before merge when GitHub can enforce it reliably;
- requires all supported CI and security checks;
- blocks force pushes and branch deletion;
- restricts bypass to documented emergency maintainers;
- records emergency bypass rationale in the merge/PR discussion.

## Required checks

Use stable job names and require:

- `Rust (ubuntu-latest)`;
- `Rust (windows-latest)`;
- `Rust (macos-latest)`;
- `Python legacy tests (3.10)` while Python remains supported;
- `Python legacy tests (3.11)` while Python remains supported;
- `Python legacy tests (3.12)` while Python remains supported;
- `Cargo advisory audit` when dependency or security workflow files change.

When an MSRV is declared, add a required `Rust MSRV` job. When Python support is removed, remove the Python required checks in the same change that removes the support metadata.

## Rust MSRV policy

Do not guess an MSRV. Determine it by:

1. identifying the minimum supported versions of direct dependencies;
2. running the full non-UI and all-feature test/build gates on the candidate compiler;
3. documenting any platform-specific compiler constraints;
4. setting `package.rust-version` only after the candidate passes;
5. testing that version in CI and using stable/latest CI separately.

Any dependency update that raises MSRV must be called out in its PR and release notes.

## Supply-chain policy

- Dependabot covers Cargo, pip, and GitHub Actions.
- Rust locked dependencies are audited on dependency changes and weekly.
- Add license/source policy (`cargo deny` or equivalent) after an explicit allow/deny policy is committed.
- Pin third-party GitHub Actions to immutable commit SHAs. Keep the release/tag in a comment so automated updates remain understandable.
- Never use an unreviewed action that can write repository contents or access secrets.
- Workflows default to `contents: read`; additional permissions are job-scoped and justified.
- Provider credentials are never available to pull-request CI.

## Merge policy

- Draft PRs may have failing or incomplete checks but must identify what is incomplete.
- A PR marked ready for review must contain real validation results or clearly identify checks delegated to GitHub Actions.
- Runtime behavior changes require tests.
- State/router/compiler contract changes require versioning and fixture updates.
- Security-sensitive capability or process-launch changes require explicit threat-model notes.
- Large refactors must be separated from behavior changes when practical.

## Emergency changes

An emergency direct change or bypass must record:

- incident or failure being mitigated;
- exact files and behavior changed;
- checks skipped and why;
- rollback plan;
- follow-up PR restoring normal validation.

## Manual repository setting

GitHub repository feature settings and branch rules are not represented fully by source files. A repository administrator must apply this policy in **Settings → Rules → Rulesets** (or **Branches** for classic protection). The policy is considered implemented only after the active rule is verified against `main`.
