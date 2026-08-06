[CmdletBinding()]
param(
    [string]$Repo = "bolmer/swarms-harness",
    [string]$WorkDir = "$PWD\swarms-harness-pr-repair",
    [switch]$MergeWhenGreen
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Require-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' is not installed or not on PATH."
    }
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory=$true)][string]$FilePath,
        [Parameter(Mandatory=$true)][string[]]$Arguments,
        [string]$WorkingDirectory = $PWD
    )
    Write-Host ">> $FilePath $($Arguments -join ' ')" -ForegroundColor Cyan
    Push-Location $WorkingDirectory
    try {
        & $FilePath @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "Command failed with exit code $LASTEXITCODE: $FilePath $($Arguments -join ' ')"
        }
    }
    finally {
        Pop-Location
    }
}

function Get-Pr([int]$Number) {
    $json = gh pr view $Number --repo $Repo --json number,title,url,state,isDraft,baseRefName,headRefName,headRefOid,mergeStateStatus,statusCheckRollup
    if ($LASTEXITCODE -ne 0) { throw "Unable to read PR #$Number." }
    return $json | ConvertFrom-Json
}

function Set-PrMetadata([int]$Number, [string]$Title, [string]$Body) {
    $bodyFile = Join-Path $env:TEMP "swarms-pr-$Number-body.md"
    Set-Content -Path $bodyFile -Value $Body -Encoding utf8
    Invoke-Checked gh @("pr","edit","$Number","--repo",$Repo,"--title",$Title,"--body-file",$bodyFile)
}

function Set-ReadyWhenGreen([int]$Number) {
    $pr = Get-Pr $Number
    $failed = @($pr.statusCheckRollup | Where-Object {
        $_.conclusion -in @("FAILURE","TIMED_OUT","CANCELLED","ACTION_REQUIRED","STARTUP_FAILURE")
    })
    $pending = @($pr.statusCheckRollup | Where-Object {
        -not $_.conclusion -or $_.status -in @("QUEUED","IN_PROGRESS","PENDING","WAITING")
    })

    if ($failed.Count -gt 0 -or $pending.Count -gt 0) {
        Write-Warning "PR #$Number is staying draft: $($failed.Count) failed and $($pending.Count) pending checks."
        return $false
    }

    if ($pr.isDraft) {
        Invoke-Checked gh @("pr","ready","$Number","--repo",$Repo)
    }
    return $true
}

function Merge-IfAllowed([int]$Number) {
    if (-not $MergeWhenGreen) { return }
    $pr = Get-Pr $Number
    if ($pr.isDraft) {
        Write-Warning "PR #$Number is still draft; not merging."
        return
    }
    if ($pr.mergeStateStatus -notin @("CLEAN","HAS_HOOKS","UNSTABLE")) {
        Write-Warning "PR #$Number merge state is $($pr.mergeStateStatus); not merging."
        return
    }
    Invoke-Checked gh @("pr","merge","$Number","--repo",$Repo,"--merge","--match-head-commit",$pr.headRefOid)
}

Require-Command git
Require-Command gh
Invoke-Checked gh @("auth","status")

if (-not (Test-Path (Join-Path $WorkDir ".git"))) {
    Invoke-Checked git @("clone","https://github.com/$Repo.git",$WorkDir)
}
Invoke-Checked git @("fetch","--prune","origin") $WorkDir

$Titles = @{
  1 = "chore(repo): sync the reviewed Rust runtime baseline into main"
  2 = "docs(review): add an issue-ready remediation backlog for R-001 through R-021"
  3 = "chore(runtime,ci): make Rust the canonical CLI and align packaging, versions, and CI"
  4 = "docs(rfc): define process supervision, cancellation, and capability enforcement"
  5 = "docs(contract): add state and event contract v2 with strict schemas and golden fixtures"
  6 = "docs(rfc): define continuous scheduling and a deterministic workflow compiler"
  7 = "docs(architecture): plan runtime decomposition and retirement of the duplicate Python runtime"
  8 = "ci(security): add Rust advisory scanning and define repository merge governance"
}

$Bodies = @{
  1 = @'
## Type
Repository synchronization / prerequisite change

## What this PR changes
- Advances `main` to the reviewed native Rust runtime and observer UI baseline.
- Adds the runtime, workflow compiler, provider adapters, steering, telemetry, UI, expanded documentation, and existing validation workflows.
- Establishes the common ancestor required by PRs #2–#8.

## Why this is needed
The canonical repository's `main` branch was behind the commit used for the code-quality review. Without this synchronization, every remediation PR includes the entire runtime implementation in its diff and cannot be reviewed independently.

## User-visible impact
- Makes the reviewed Rust implementation part of the default branch.
- Does not intentionally change behavior beyond the existing reviewed commits.

## Validation
Before merge, this PR must pass:
- Rust formatting
- Clippy with warnings denied
- Rust tests on supported operating systems
- Release build
- Doctor, review, dry-run, and mock E2E checks
- Legacy Python lint/tests while those tools remain supported

## Merge order
Merge this PR first. PRs #2–#8 must then be retargeted to `main`.

## Risk
Large baseline synchronization. Review commit ancestry and CI results carefully before merging.
'@
  2 = @'
## Type
Documentation / engineering backlog

## What this PR changes
- Records every distinct repository-review finding as R-001 through R-021.
- Assigns priority, impact, scope, and objective acceptance criteria.
- Preserves the positive safety and CI controls that future fixes must not regress.

## Why this is needed
Repository Issues were disabled when the review was converted into work items. This document provides durable, one-to-one tracking until each entry can be migrated to a GitHub issue.

## User-visible impact
None. This PR changes documentation only.

## Validation
- Every original review finding has exactly one backlog entry.
- Each entry has measurable completion criteria.
- No finding is represented as fixed merely because it is documented.

## Not included
This PR does not implement runtime fixes.
'@
  3 = @'
## Type
Packaging change / CI improvement / compatibility cleanup

## What this PR changes
- Removes the installed Python `swarms` console entry point so the package no longer advertises a second public runtime.
- Documents the native Rust binary as the supported workflow coordinator.
- Aligns project version metadata and canonical repository URLs.
- Expands legacy Python CI to the declared Python 3.10, 3.11, and 3.12 range.
- Adds Dependabot coverage for Cargo, Python, and GitHub Actions dependencies.

## Bugs and inconsistencies fixed
- Fixes the contradiction between Rust-only runtime documentation and the Python package entry point.
- Fixes mismatched project versions and stale `Mchicao/...` repository links.
- Fixes CI coverage that declared Python 3.10–3.12 support but tested only Python 3.11.

## User-visible impact
Existing direct invocations such as `python scripts/swarm.py` remain possible during the retirement period, but new package installations no longer expose that legacy script as the primary product CLI.

## Validation
- Rust CI remains green.
- Python tests pass on 3.10, 3.11, and 3.12.
- Package metadata contains no public Python `swarms` entry point.
- Dependabot configuration parses successfully.

## Review findings addressed
- R-001, partial
- R-015
- R-018, Python portion
- R-019, dependency-update portion
'@
  4 = @'
## Type
Architecture RFC

## What this PR changes
- Defines a cross-platform process-supervision abstraction for provider and verification processes.
- Specifies deadlines, cancellation, process-tree termination, bounded output, and stable terminal reasons.
- Defines typed runtime errors and explicit retryability.
- Replaces implicit shell/tool permissions with structured verification commands and typed capability policies.
- Defines how provider adapters should be split and how production code must be separated from fixtures.

## Bugs and risks targeted
- Hung workers without enforceable deadlines.
- Child processes surviving coordinator cancellation.
- Unbounded logs or captured output.
- Arbitrary shell verification and dangerous tool-policy mappings.
- Stringly typed failures that cannot be classified safely for retry.
- Oversized adapter modules containing production and mock code together.

## User-visible impact
None yet. This PR establishes the implementation contract; it does not claim that supervision is implemented.

## Acceptance gate for follow-up implementation
The runtime must pass timeout, cancellation, process-tree, output-bound, capability-denial, and retry-classification fault tests before the findings are closed.

## Review findings covered
- R-004
- R-005
- R-008
- R-014
'@
  5 = @'
## Type
Data-contract change / compatibility foundation

## What this PR changes
- Defines a versioned durable contract for task snapshots and event streams.
- Adds strict JSON Schemas for task state and events.
- Adds a golden succeeded-task fixture for producer-to-UI compatibility tests.
- Defines typed usage completeness, persistence failures, corruption quarantine, and durable steering identifiers.

## Bugs and risks targeted
- Runtime state that does not match `docs/STATE_CONTRACT.md`.
- Missing or inconsistent event timestamps and fields.
- Corrupt session/state JSON silently becoming empty state.
- Steering identifiers that can collide.
- Token usage represented by ambiguous strings such as `"missing"`.
- Producer and UI changes drifting without a shared compatibility test.

## User-visible impact
None until producers and readers implement v2. The current contract remains active during migration.

## Validation
- Schemas must validate the golden fixtures.
- Rust producers and UI readers must both pass the same fixtures before v2 becomes active.
- Corrupt and unknown-version state must fail explicitly, not silently default.

## Review findings covered
- R-003
- R-007
- R-011
- R-012
- R-020
'@
  6 = @'
## Type
Scheduler/compiler RFC

## What this PR changes
- Defines an event-driven scheduler that starts newly ready tasks whenever capacity becomes available.
- Requires exact canonical dependency identifiers and removes suffix-based dependency matching.
- Defines strict router parsing and transitive alias/fallback cycle validation.
- Replaces route-name heuristics with explicit typed cost policy.
- Replaces brace-sensitive interpolation with `${name}` variables and explicit escaping.
- Defines deterministic compiled-plan artifacts, source maps, and hashes.

## Bugs and risks targeted
- Wave scheduling leaving worker capacity idle.
- Ambiguous dependency references selecting the wrong task.
- Alias/fallback cycles and one-level-only resolution.
- Premium-cost decisions inferred from route names.
- JSON or source code braces breaking workflow-template expansion.
- Conditional expansion silently dropping dependency edges.
- Router fields being documented but ignored by Serde.

## User-visible impact
None yet. This PR defines the behavior required from follow-up implementation.

## Review findings covered
- R-002
- R-006
- R-009
- R-010
'@
  7 = @'
## Type
Maintainability and migration plan

## What this PR changes
- Defines cohesive module boundaries for scheduler, executor, supervisor, persistence, checkpoints, prompts, verification, artifacts, and retry logic.
- Defines provider-adapter module boundaries and fixture separation.
- Classifies duplicate Python runtime/review components for migration, compatibility shims, or removal.
- Defines Ruff cleanup, lockfile policy, and version ownership.

## Bugs and maintenance problems targeted
- `runtime.rs` and `adapter.rs` carrying too many responsibilities.
- Mock implementations embedded in production adapter source.
- Python and Rust runtimes drifting in behavior, state, caps, and safety.
- Broad Ruff exclusions hiding quality problems.
- Ignored `uv.lock` despite continued Python support.
- Conflicting release/version semantics.

## User-visible impact
None yet. This is a staged migration plan and does not claim the modules have already been split.

## Review findings covered
- R-001, remaining retirement work
- R-013
- R-014
- R-016
- R-017
'@
  8 = @'
## Type
Security CI / repository governance

## What this PR changes
- Adds scheduled and dependency-change-triggered RustSec advisory scanning with `cargo audit`.
- Defines the required default-branch checks and evidence-based MSRV policy.
- Documents dependency/supply-chain controls, merge rules, and emergency bypass handling.
- Expands the PR template with contract, compatibility, capability, shell-execution, validation, and residual-risk checks.

## Bugs and risks targeted
- Vulnerable Rust dependencies reaching `main` without an automated advisory gate.
- No documented required-check or branch-protection policy.
- Pull requests omitting compatibility, security, or validation impact.
- Deprecated GitHub Action runtimes.

## User-visible impact
No runtime behavior change. Merges become more strictly gated by security and validation results.

## Validation
- `cargo audit` must complete successfully.
- Any RustSec advisory must be fixed by updating the dependency, or explicitly documented with a narrow temporary exception and owner.
- GitHub Actions must use supported Node runtimes.
- The repository administrator must apply the documented branch ruleset after job names stabilize.

## Review findings covered
- R-018, Rust/MSRV policy portion
- R-019, advisory automation portion
- R-021
'@
}

# Always improve PR metadata first.
foreach ($number in 1..8) {
    Set-PrMetadata $number $Titles[$number] $Bodies[$number]
}

# Repair PR #1 and the shared CI foundation.
$baseline = "agent/sync-reviewed-baseline"
Invoke-Checked git @("switch","$baseline") $WorkDir
Invoke-Checked git @("fetch","--prune","origin") $WorkDir
Invoke-Checked git @("reset","--hard","origin/$baseline") $WorkDir

# Integrate current main without rewriting published history.
& git -C $WorkDir merge --no-edit origin/main
if ($LASTEXITCODE -ne 0) {
    Write-Error @"
PR #1 has a real merge conflict. Resolve the files listed by:
  git -C "$WorkDir" status
Then run:
  git -C "$WorkDir" add <resolved-files>
  git -C "$WorkDir" commit
  git -C "$WorkDir" push origin "$baseline"
Re-run this script afterward.
"@
    exit 2
}

$ciPath = Join-Path $WorkDir ".github\workflows\ci.yml"
if (-not (Test-Path $ciPath)) { throw "Missing $ciPath" }
$ci = Get-Content $ciPath -Raw

# Avoid duplicate feature-branch runs while retaining PR and main validation.
$oldTrigger = "on:`n  push:`n  pull_request:"
$newTrigger = @"
on:
  pull_request:
    branches:
      - main
  push:
    branches:
      - main
  workflow_dispatch:
"@
if ($ci.Contains($oldTrigger)) {
    $ci = $ci.Replace($oldTrigger, $newTrigger.TrimEnd())
}

# Move first-party Actions to Node 24-compatible releases.
$ci = $ci.Replace("actions/checkout@v4", "actions/checkout@v6")
$ci = $ci.Replace("actions/setup-python@v5", "actions/setup-python@v6")
Set-Content -Path $ciPath -Value $ci -Encoding utf8

$changes = git -C $WorkDir status --porcelain
if ($changes) {
    Invoke-Checked git @("add",".github/workflows/ci.yml") $WorkDir
    Invoke-Checked git @("commit","-m","ci: remove duplicate branch runs and update action runtimes") $WorkDir
    Invoke-Checked git @("push","origin",$baseline) $WorkDir
}

# Run the most relevant local validation when toolchains are present.
if (Get-Command cargo -ErrorAction SilentlyContinue) {
    Invoke-Checked cargo @("fmt","--manifest-path","rust/Cargo.toml","--","--check") $WorkDir
    Invoke-Checked cargo @("clippy","--manifest-path","rust/Cargo.toml","--all-targets","--all-features","--","-D","warnings") $WorkDir
    Invoke-Checked cargo @("test","--manifest-path","rust/Cargo.toml","--all-features") $WorkDir
} else {
    Write-Warning "cargo is unavailable; Rust validation will rely on GitHub Actions."
}

# PR #1 can only be ready/merged after its checks are green.
if (Set-ReadyWhenGreen 1) {
    Merge-IfAllowed 1
}

$pr1 = Get-Pr 1
if ($pr1.state -ne "MERGED") {
    Write-Warning "PR #1 is not merged. Stop here; PRs #2–#8 must not be retargeted yet."
    Write-Host "Run: gh pr checks 1 --repo $Repo --watch"
    exit 3
}

# Retarget and rebase every independent remediation branch onto the merged baseline.
$branches = @{
    2 = "agent/review-remediation-backlog"
    3 = "agent/public-runtime-and-ci-hygiene"
    4 = "agent/runtime-supervision-contract"
    5 = "agent/state-contract-v2"
    6 = "agent/scheduler-compiler-contract"
    7 = "agent/maintainability-and-retirement-plan"
    8 = "agent/security-and-repository-governance"
}

Invoke-Checked git @("fetch","--prune","origin") $WorkDir

foreach ($number in 2..8) {
    $branch = $branches[$number]
    Invoke-Checked gh @("pr","edit","$number","--repo",$Repo,"--base","main")
    Invoke-Checked git @("switch","-C",$branch,"origin/$branch") $WorkDir

    & git -C $WorkDir rebase origin/main
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Rebase conflict in PR #$number ($branch). Resolve it, run 'git rebase --continue', push with --force-with-lease, and rerun."
        exit 10 + $number
    }
    Invoke-Checked git @("push","--force-with-lease","origin",$branch) $WorkDir

    # Update deprecated action runtime in the security workflow.
    if ($number -eq 8) {
        $securityPath = Join-Path $WorkDir ".github\workflows\security.yml"
        if (Test-Path $securityPath) {
            $security = Get-Content $securityPath -Raw
            $updated = $security.Replace("actions/checkout@v4", "actions/checkout@v6")
            if ($updated -ne $security) {
                Set-Content -Path $securityPath -Value $updated -Encoding utf8
                Invoke-Checked git @("add",".github/workflows/security.yml") $WorkDir
                Invoke-Checked git @("commit","-m","ci(security): use Node 24-compatible checkout action") $WorkDir
                Invoke-Checked git @("push","origin",$branch) $WorkDir
            }
        }

        if (Get-Command cargo -ErrorAction SilentlyContinue) {
            if (-not (Get-Command cargo-audit -ErrorAction SilentlyContinue)) {
                Invoke-Checked cargo @("install","cargo-audit","--locked") $WorkDir
            }
            & cargo audit --manifest-path (Join-Path $WorkDir "rust\Cargo.toml")
            if ($LASTEXITCODE -ne 0) {
                Write-Warning "PR #8 remains draft because cargo audit found an advisory. Fix the named crate/version; do not suppress it generically."
                continue
            }
        }
    }

    if (Set-ReadyWhenGreen $number) {
        Merge-IfAllowed $number
    }
}

Write-Host "Repair pass complete." -ForegroundColor Green
Write-Host "Recommended merge order: #1 -> #3 -> #2 -> #4 -> #5 -> #6 -> #7 -> #8"
Write-Host "No PR was merged unless -MergeWhenGreen was supplied and all checks were complete."
