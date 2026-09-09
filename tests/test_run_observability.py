"""Tests for the read-only run observability contract.

Covers the full nested tree, empty runs, partial/interrupted runs, secret/path
sanitization, and resilience against corrupt checkpoints. The fixtures fabricate
on-disk checkpoints directly (via the stdlib atomic JSON helper) so the observer
is exercised without any Python runtime dependency.
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
    write_json_atomic,
)

WORKFLOW_PLAN = "docs/workflow_plan_example.json"


def _task(
    task_id: str,
    index: int,
    source_id: str,
    stage: str,
    role: str,
    status: str,
    heartbeat_unix_ms: int | None,
    *,
    parent_task_id: str | None = None,
    subagents: list[str] | None = None,
    artifacts: list[str] | None = None,
    started_at: str = "2026-07-16T10:00:00+00:00",
    ended_at: str = "2026-07-16T10:01:00+00:00",
) -> dict:
    node: dict = {
        "state_schema_version": 1,
        "task_id": task_id,
        "source_id": source_id,
        "agent_id": source_id,
        "index": index,
        "stage": stage,
        "role": role,
        "status": status,
        "attempts": 1,
        "model": "mock-worker",
        "provider": "mock",
        "route": "mock",
        "provider_subagent_visibility": "not_reported",
        "provider_subagents": [],
        "needs": [],
        "artifacts": artifacts if artifacts is not None else [f"docs/{source_id}.md"],
        "error": None,
    }
    if parent_task_id is not None:
        node["parent_task_id"] = parent_task_id
    if subagents is not None:
        node["subagents"] = subagents
    if status == "completed":
        node["started_at"] = started_at
        node["ended_at"] = ended_at
    if heartbeat_unix_ms is not None:
        node["heartbeat_unix_ms"] = heartbeat_unix_ms
    return node


def _write_workflow(run_dir: Path, *, run_id: str, extra: dict | None = None) -> None:
    workflow = {
        "run_id": run_id,
        "state_schema_version": 1,
        "runtime": "python",
        "created_at": "2026-07-16T10:00:00+00:00",
        "workspace_root": str(run_dir.parents[1]),
        "heartbeat_interval_seconds": 120,
        "global_max_concurrency": 3,
        "provider_max_concurrency": {"mock": 3},
        "max_total_workers": 12,
        "task_count": 0,
        "tasks_file": "tasks",
        "workflow_plan": WORKFLOW_PLAN,
    }
    workflow.update(extra or {})
    write_json_atomic(run_dir / "workflow.json", workflow)


def _write_completed_run(tmp_path: Path) -> Path:
    """A fully completed run mirroring docs/workflow_plan_example.json."""
    run_dir = tmp_path / "runs" / "obs-completed"
    tasks = [
        # Discovery
        _task(
            "reshard_plan", 0, "reshard_plan", "Discovery", "planner", "completed",
            heartbeat_unix_ms=1_700_000_000_500, parent_task_id=None, subagents=["compress"],
        ),
        # Implementation
        _task(
            "0001-compress", 1, "compress", "Implementation", "programmer", "completed",
            heartbeat_unix_ms=1_700_000_001_000, parent_task_id="reshard_plan",
            artifacts=["bench_apps/reshard/compress.py"],
        ),
        _task(
            "0002-decompress", 2, "decompress", "Implementation", "programmer", "completed",
            heartbeat_unix_ms=1_700_000_001_500, parent_task_id="reshard_plan",
            artifacts=["bench_apps/reshard/decompress.py"],
        ),
        # Verification
        _task(
            "0003-tests", 3, "tests", "Verification", "verifier", "completed",
            heartbeat_unix_ms=1_700_000_002_000, parent_task_id=None,
            artifacts=["bench_tests/test_bench_reshard.py"],
        ),
    ]
    _write_workflow(
        run_dir,
        run_id="obs-completed",
        extra={"task_count": len(tasks), "workflow_plan": str(tmp_path / "nested-plan.json")},
    )
    for task in tasks:
        write_json_atomic(run_dir / "tasks" / f"{task['task_id']}.json", task)
        (run_dir / "results" / task["task_id"]).mkdir(parents=True, exist_ok=True)
        write_json_atomic(run_dir / "results" / task["task_id"] / "result.json", {"status": task["status"]})
    events = [{"event": "workflow_initialized"}, {"event": "task_finished"}, {"event": "workflow_finished"}]
    with (run_dir / "events.jsonl").open("w", encoding="utf-8") as handle:
        for event in events:
            handle.write(json.dumps(event) + "\n")
    write_json_atomic(run_dir / "report.json", {"status": "completed"})
    return run_dir


def _write_partial_run(tmp_path: Path) -> tuple[Path, dict, dict]:
    """A run left mid-flight: one completed, one in-progress with a live claim."""
    run_dir = tmp_path / "runs" / "obs-partial"
    t0 = _task("0001-a", 0, "a", "Discovery", "planner", "completed", heartbeat_unix_ms=1_700_000_000_000)
    t1 = _task("0002-b", 1, "b", "Discovery", "planner", "in_progress", heartbeat_unix_ms=1_700_000_001_000)
    _write_workflow(run_dir, run_id="obs-partial", extra={"task_count": 2})
    write_json_atomic(run_dir / "tasks" / "0001-a.json", t0)
    write_json_atomic(run_dir / "tasks" / "0002-b.json", t1)
    write_json_atomic(
        run_dir / "claims" / "0002-b.lock",
        {
            "task_id": "0002-b",
            "owner": "owner-x",
            "claimed_at": "2026-07-16T10:00:00+00:00",
            "heartbeat_at": "2026-07-16T10:05:00+00:00",
        },
    )
    return run_dir, t0, t1


def _write_empty_run_dir(tmp_path: Path) -> Path:
    run_dir = tmp_path / "runs" / "obs-empty"
    _write_workflow(
        run_dir,
        run_id="obs-empty",
        extra={"task_count": 0, "workspace_root": str(tmp_path)},
    )
    return run_dir


@pytest.fixture
def completed_run(tmp_path: Path) -> Path:
    return _write_completed_run(tmp_path)


@pytest.fixture
def partial_run(tmp_path: Path) -> Path:
    return _write_partial_run(tmp_path)[0]


@pytest.fixture
def empty_run_dir(tmp_path: Path) -> Path:
    return _write_empty_run_dir(tmp_path)


def test_contract_is_versioned_and_read_only(completed_run: Path):
    contract = RunObservability(completed_run).build_contract()

    assert contract["contract_schema_version"] == CONTRACT_SCHEMA_VERSION
    assert contract["read_only"] is True
    assert contract["run"]["run_id"] == "obs-completed"
    assert contract["run"]["runtime"] == "python"
    assert contract["run"]["status"] == "completed"


def test_contract_exposes_stages_tasks_and_nested_subagents(completed_run: Path):
    contract = RunObservability(completed_run).build_contract()

    # Three stages mirroring the example plan.
    assert [stage["name"] for stage in contract["stages"]] == [
        "Discovery",
        "Implementation",
        "Verification",
    ]
    discovery = contract["stages"][0]
    parent_task = discovery["tasks"][0]
    assert parent_task["source_id"] == "reshard_plan"
    assert parent_task["parent_task_id"] is None
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


def test_summary_counts_and_heartbeat_roll_up(completed_run: Path):
    contract = RunObservability(completed_run).build_contract()

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


def test_partial_run_reports_running_status_and_claims(partial_run: Path):
    contract = RunObservability(partial_run).build_contract()

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


def test_contract_tolerates_corrupt_task_and_claim_files(completed_run: Path):
    tasks_dir = completed_run / "tasks"
    poison = tasks_dir / "corrupt.json"
    poison.write_text("{not valid json", encoding="utf-8")
    claims_dir = completed_run / "claims"
    claims_dir.mkdir(parents=True, exist_ok=True)
    (claims_dir / "ghost.lock").write_text("trash", encoding="utf-8")
    (claims_dir / "corrupt.lock").write_text("trash", encoding="utf-8")

    contract = RunObservability(completed_run).build_contract()

    # Corrupt files are skipped, not fatal.
    assert len(contract["stages"]) == 3
    task_ids = {task["task_id"] for stage in contract["stages"] for task in stage["tasks"]}
    assert "corrupt" not in task_ids


def test_paths_are_relativized_and_absolute_structure_is_not_leaked(tmp_path):
    runs = tmp_path / "runs"
    run_dir = runs / "obs-paths"
    secret_workspace = tmp_path / "secret_workspace"
    _write_workflow(
        run_dir,
        run_id="obs-paths",
        extra={
            "workflow_plan": str(secret_workspace / "plan.json"),
            "workspace_root": str(secret_workspace),
        },
    )
    write_json_atomic(
        run_dir / "tasks" / "0001-a.json",
        {
            "task_id": "0001-a",
            "source_id": "a",
            "agent_id": "a",
            "index": 0,
            "stage": "Discovery",
            "status": "completed",
            "role": "planner",
            "route": "mock",
            "model": "mock-worker",
            "provider": "mock",
            "provider_subagent_visibility": "not_reported",
            "provider_subagents": [],
            "needs": [],
            "artifacts": [
                "docs/bench_notes/reshard_plan.md",
                str(secret_workspace / "build" / "out.txt"),
            ],
            "error": None,
        },
    )

    contract = RunObservability(run_dir, roots=(tmp_path,)).build_contract()

    plan_value = contract["run"]["workflow_plan"]
    assert plan_value is not None
    assert "secret_workspace" not in plan_value  # relativized, not absolute
    # Artifacts are relative repo paths; absolute foreign structure is not leaked.
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


def test_list_runs_indexes_directory(completed_run: Path, empty_run_dir: Path):
    runs = list_runs(completed_run.parent)
    by_id = {run["run_id"]: run for run in runs}
    assert by_id["obs-completed"]["task_count"] == 4
    assert by_id["obs-completed"]["has_report"] is True
    assert by_id["obs-empty"]["task_count"] == 0
    assert by_id["obs-empty"]["has_report"] is False


def test_list_runs_handles_missing_root(tmp_path):
    assert list_runs(tmp_path / "does-not-exist") == []


def test_contract_is_deterministic_for_completed_run(completed_run: Path):
    # observed_at changes per call, so compare the stable portions only.
    first = RunObservability(completed_run).build_contract()
    time.sleep(0.01)
    second = RunObservability(completed_run).build_contract()

    first["run"].pop("observed_at")
    second["run"].pop("observed_at")
    assert first == second


def test_default_runs_dir_is_under_agent_workspace():
    # Guards against the contract accidentally pointing outside .agent/swarm.
    assert DEFAULT_RUNS_DIR.name == "runs"
    assert ".agent" in DEFAULT_RUNS_DIR.parts


def test_iter_events_reads_jsonl_and_skips_corrupt_lines(completed_run: Path):
    events_path = completed_run / "events.jsonl"
    assert events_path.exists()
    with events_path.open("a", encoding="utf-8") as handle:
        handle.write("\n{not valid json\n")
    events = list(iter_events(completed_run))
    assert events, "should yield parsed event rows"
    assert all("event" in row for row in events)
    # Corrupt line was skipped, not fatal.
    assert all(isinstance(row, dict) for row in events)


def test_iter_events_handles_missing_file(tmp_path: Path):
    assert list(iter_events(tmp_path)) == []


def test_write_json_atomic_writes_json(tmp_path: Path):
    """Sanity-check the stdlib atomic JSON helper that fabricates fixtures."""
    path = tmp_path / "a" / "b.json"
    write_json_atomic(path, {"k": "v"})
    assert json.loads(path.read_text(encoding="utf-8")) == {"k": "v"}