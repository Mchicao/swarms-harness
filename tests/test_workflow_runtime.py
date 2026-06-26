import json
import time

from scripts.workflow_runtime import ClaimStore, WorkflowRuntime


def test_workflow_plan_is_deterministic(tmp_path):
    runtime_a = WorkflowRuntime(run_id="plan-a", run_root=tmp_path, global_max_concurrency=4)
    runtime_b = WorkflowRuntime(run_id="plan-b", run_root=tmp_path, global_max_concurrency=4)

    tasks_a = [task.to_dict() for task in runtime_a.build_tasks("micro-reshard-roundtrip")]
    tasks_b = [task.to_dict() for task in runtime_b.build_tasks("micro-reshard-roundtrip")]

    assert tasks_a == tasks_b
    assert len(tasks_a) == 5
    assert tasks_a[0]["needs"] == []
    assert any("reshard_plan" in dep for dep in tasks_a[1]["needs"])


def test_claim_store_prevents_double_claim_and_recovers_stale_claim(tmp_path):
    claims = ClaimStore(tmp_path, stale_seconds=1)

    assert claims.try_claim("task-1", "worker-a")
    assert not claims.try_claim("task-1", "worker-b")

    stale_path = claims.claim_path("task-1")
    old_time = time.time() - 5
    import os

    os.utime(stale_path, (old_time, old_time))
    assert claims.try_claim("task-1", "worker-b")
    claims.release("task-1", "worker-b")
    assert claims.try_claim("task-1", "worker-c")


def test_workflow_dry_run_writes_planned_report(tmp_path):
    runtime = WorkflowRuntime(run_id="dry", run_root=tmp_path, global_max_concurrency=2)

    report = runtime.run("micro-reshard-roundtrip", dry_run=True, force=True)

    assert report["status"] == "planned"
    assert report["task_counts"] == {"pending": 5}
    saved = json.loads((tmp_path / "dry" / "report.json").read_text(encoding="utf-8"))
    assert saved["status"] == "planned"


def test_mock_workflow_executes_dependency_waves(tmp_path):
    runtime = WorkflowRuntime(
        run_id="mock-run",
        run_root=tmp_path,
        global_max_concurrency=3,
        provider_max_concurrency={"mock": 3},
    )

    report = runtime.run("micro-reshard-roundtrip", force=True)

    assert report["status"] == "completed"
    assert report["task_counts"] == {"completed": 5}
    assert report["token_usage"]["known_cost_usd"] == 0.0
    assert (tmp_path / "mock-run" / "events.jsonl").exists()
