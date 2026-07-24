"""Tests for the read-only run observability contract.

Covers the full nested tree, empty runs, partial/interrupted runs, secret/path
sanitization, and resilience against corrupt checkpoints.
"""

from __future__ import annotations

import json
import time
from pathlib import Path

import pytest

from scripts.run_observability import (
    CONTRACT_SCHEMA_VERSION,
    DEFAULT_RUNS_DIR,
    RunObservability,
    iter_events,
    list_runs,
    sanitize_error,
    sanitize_path,
)
from scripts.workflow_runtime import WorkflowRuntime, write_json_atomic


def _write_claim(run_dir: Path, task_id: str, owner: str, heartbeat_at: str) -> None:
    claims = run_dir / "claims"
    claims.mkdir(parents=True, exist_ok=True)
    write_json_atomic(
        claims / f"{task_id}.lock",
        {
            "task_id": task_id,
            "owner": owner,
            "claimed_at": "2026-07-16T10:00:00+00:00",
            "heartbeat_at": heartbeat_at,
        },
    )


@pytest.fixture
def nested_plan(tmp_path: Path) -> Path:
    """Example plan with a parent task and nested subagents."""
    plan = json.loads(Path("docs/workflow_plan_example.json").read_text(encoding="utf-8"))
    plan["stages"][1]["tasks"][0]["parent_task_id"] = "reshard_plan"
    plan_path = tmp_path / "nested-plan.json"
    plan_path.write_text(json.dumps(plan), encoding="utf-8")
    return plan_path


@pytest.fixture
def completed_run(tmp_path: Path, nested_plan: Path) -> WorkflowRuntime:
    """A fully completed mock run with events, results, and a report."""
    runtime = WorkflowRuntime(
        workflow_plan=nested_plan,
        run_id="obs-completed",
        run_root=tmp_path / "runs",
        global_max_concurrency=3,
        provider_max_concurrency={"mock": 3},
    )
    runtime.run(force=True)
    return runtime


@pytest.fixture
def partial_run(tmp_path: Path, nested_plan: Path) -> WorkflowRuntime:
    """A run initialized but left mid-flight: some completed, some pending."""
    runtime = WorkflowRuntime(
        workflow_plan=nested_plan,
        run_id="obs-partial",
        run_root=tmp_path / "runs",
    )
    tasks = runtime.initialize(force=True)
    tasks[0].status = "completed"
    tasks[0].heartbeat_unix_ms = 1_700_000_000_000
    tasks[1].status = "in_progress"
    tasks[1].heartbeat_unix_ms = 1_700_000_001_000
    runtime.save_task(tasks[0])
    runtime.save_task(tasks[1])
    _write_claim(
        runtime.run_dir,
        tasks[1].task_id,
        "owner-x",
        "2026-07-16T10:05:00+00:00",
    )
    return runtime


@pytest.fixture
def empty_run_dir(tmp_path: Path) -> Path:
    """A run directory with only workflow.json and no task checkpoints."""
    run_dir = tmp_path / "runs" / "obs-empty"
    run_dir.mkdir(parents=True)
    write_json_atomic(
        run_dir / "workflow.json",
        {
            "run_id": "obs-empty",
            "state_schema_version": 1,
            "runtime": "python",
            "created_at": "2026-07-16T10:00:00+00:00",
            "workspace_root": str(tmp_path),
            "task_count": 0,
        },
    )
    return run_dir


def test_contract_is_versioned_and_read_only(completed_run: WorkflowRuntime):
    contract = RunObservability(completed_run.run_dir).build_contract()

    assert contract["contract_schema_version"] == CONTRACT_SCHEMA_VERSION
    assert contract["read_only"] is True
    assert contract["run"]["run_id"] == "obs-completed"
    assert contract["run"]["runtime"] == "python"
    assert contract["run"]["status"] == "completed"


def test_contract_exposes_stages_tasks_and_nested_subagents(completed_run, nested_plan):
    contract = RunObservability(completed_run.run_dir).build_contract()

    # Three stages from the example plan.
    assert [stage["name"] for stage in contract["stages"]] == [
        "Discovery",
        "Implementation",
        "Verification",
    ]
    discovery = contract["stages"][0]
    parent_task = discovery["tasks"][0]
    assert parent_task["source_id"] == "reshard_plan"
    assert parent_task["status"] == "completed"

    # The owning agent block carries model/route and resolves subagents.
    assert parent_task["agent"]["agent_id"] == "reshard_plan"
    assert parent_task["model"] == "mock-worker"
    assert parent_task["route"] == "mock"

    subagents = parent_task["subagents"]
    assert subagents, "parent should list nested subagents"
    compress = next(s for s in subagents if s["agent_id"] == "compress")
    assert compress["status"] == "completed"
    assert compress["model"] == "mock-worker"

    # Timestamps and last heartbeat are present for every finished task.
    for stage in contract["stages"]:
        for task in stage["tasks"]:
            assert task["timestamps"]["started_at"]
            assert task["timestamps"]["ended_at"]
            assert task["timestamps"]["heartbeat_unix_ms"]


def test_summary_counts_and_heartbeat_roll_up(completed_run: WorkflowRuntime):
    contract = RunObservability(completed_run.run_dir).build_contract()

    assert contract["summary"]["task_status_counts"] == {"completed": 4}
    assert contract["summary"]["stage_count"] == 3
    assert contract["summary"]["result_count"] == 4
    assert contract["summary"]["last_heartbeat_unix_ms"]
    assert contract["summary"]["has_real_provider"] is False
    assert contract["summary"]["report_status"] == "completed"


def test_empty_run_contract_has_no_tasks(empty_run_dir: Path):
    contract = RunObservability(empty_run_dir).build_contract()

    assert contract["run"]["status"] == "empty"
    assert contract["stages"] == []
    assert contract["summary"]["task_status_counts"] == {}
    assert contract["summary"]["last_heartbeat_unix_ms"] is None


def test_partial_run_reports_running_status_and_claims(partial_run: WorkflowRuntime):
    contract = RunObservability(partial_run.run_dir).build_contract()

    assert contract["run"]["status"] == "running"
    counts = contract["summary"]["task_status_counts"]
    assert counts["completed"] == 1
    assert counts["in_progress"] == 1

    in_progress = next(
        task for stage in contract["stages"] for task in stage["tasks"] if task["status"] == "in_progress"
    )
    # The live claim owner and heartbeat surface on the owning agent.
    assert in_progress["agent"]["owner"] == "owner-x"
    assert in_progress["agent"]["heartbeat_at"] == "2026-07-16T10:05:00+00:00"
    assert in_progress["timestamps"]["heartbeat_unix_ms"] == 1_700_000_001_000
    assert contract["summary"]["last_heartbeat_unix_ms"] == 1_700_000_001_000


def test_contract_tolerates_corrupt_task_and_claim_files(completed_run: WorkflowRuntime):
    tasks_dir = completed_run.tasks_dir
    poison = tasks_dir / "corrupt.json"
    poison.write_text("{not valid json", encoding="utf-8")
    bad_claim = completed_run.run_dir / "claims" / "ghost.lock"
    bad_claim.write_text("trash", encoding="utf-8")

    contract = RunObservability(completed_run.run_dir).build_contract()

    # Corrupt files are skipped, not fatal.
    assert len(contract["stages"]) == 3
    task_ids = {task["task_id"] for stage in contract["stages"] for task in stage["tasks"]}
    assert "corrupt" not in task_ids


def test_paths_are_relativized_and_absolute_structure_is_not_leaked(tmp_path, nested_plan):
    runtime = WorkflowRuntime(
        workflow_plan=nested_plan,
        run_id="obs-paths",
        run_root=tmp_path / "runs",
        workspace_root=tmp_path / "secret_workspace",
    )
    runtime.initialize(force=True)

    contract = RunObservability(runtime.run_dir, roots=(tmp_path,)).build_contract()

    plan_value = contract["run"]["workflow_plan"]
    assert plan_value is not None
    assert "secret_workspace" not in plan_value  # relative, not absolute
    # Artifacts are relative repo paths.
    for stage in contract["stages"]:
        for task in stage["tasks"]:
            for artifact in task["artifacts"]:
                assert artifact is None or not Path(artifact).is_absolute()


def test_sanitize_error_redacts_secrets_and_caps_length():
    long = "x" * 5000
    capped = sanitize_error(long)
    assert len(capped) <= 1100
    assert capped.endswith("[truncated]")
    assert sanitize_error(None) is None

    bearer = sanitize_error("Authorization: Bearer sk-abcdEFGH12345 token")
    assert "sk-abcdEFGH12345" not in bearer
    assert "***" in bearer

    apikey = sanitize_error("failure api_key=supersecret_value tail")
    assert "supersecret_value" not in apikey
    assert "***" in apikey

    env = sanitize_error("env OPENAI_API_KEY=sk-live-12345 end")
    assert "sk-live-12345" not in env
    assert "***" in env


def test_sanitize_path_collapses_unknown_absolute_to_basename(tmp_path):
    foreign = tmp_path / "foreign" / "deep" / "secret.bin"
    result = sanitize_path(foreign, roots=(tmp_path / "different",))
    assert result == "secret.bin"
    assert sanitize_path(None, roots=()) is None
    assert sanitize_path("docs/x.md", roots=()) == "docs/x.md"


def test_unsafe_run_id_is_rejected_for_observation(tmp_path):
    with pytest.raises(ValueError, match="Unsafe run_id"):
        RunObservability.from_run("../escape", run_root=tmp_path)
    # Path separators are rejected by the safe-character guard before escaping.
    with pytest.raises(ValueError, match="Unsafe run_id"):
        RunObservability.from_run("a/b", run_root=tmp_path)


def test_list_runs_indexes_directory(completed_run: WorkflowRuntime, empty_run_dir):
    runs = list_runs(completed_run.run_dir.parent)
    by_id = {run["run_id"]: run for run in runs}
    assert by_id["obs-completed"]["task_count"] == 4
    assert by_id["obs-completed"]["has_report"] is True
    assert by_id["obs-empty"]["task_count"] == 0
    assert by_id["obs-empty"]["has_report"] is False


def test_list_runs_handles_missing_root(tmp_path):
    assert list_runs(tmp_path / "does-not-exist") == []


def test_contract_is_deterministic_for_completed_run(completed_run: WorkflowRuntime):
    # observed_at changes per call, so compare the stable portions only.
    first = RunObservability(completed_run.run_dir).build_contract()
    time.sleep(0.01)
    second = RunObservability(completed_run.run_dir).build_contract()

    first["run"].pop("observed_at")
    second["run"].pop("observed_at")
    assert first == second


def test_default_runs_dir_is_under_agent_workspace():
    # Guards against the contract accidentally pointing outside .agent/swarm.
    assert DEFAULT_RUNS_DIR.name == "runs"
    assert ".agent" in DEFAULT_RUNS_DIR.parts


def test_iter_events_reads_jsonl_and_skips_corrupt_lines(completed_run: WorkflowRuntime):
    events_path = completed_run.run_dir / "events.jsonl"
    assert events_path.exists()
    with events_path.open("a", encoding="utf-8") as handle:
        handle.write("\n{not valid json\n")
    events = list(iter_events(completed_run.run_dir))
    assert events, "should yield parsed event rows"
    assert all("event" in row for row in events)
    # Corrupt line was skipped, not fatal.
    assert all(isinstance(row, dict) for row in events)


def test_iter_events_handles_missing_file(tmp_path: Path):
    assert list(iter_events(tmp_path)) == []
