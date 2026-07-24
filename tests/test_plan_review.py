import json
from pathlib import Path

from scripts.plan_review import review_plan
from scripts.workflow_runtime import WorkflowRuntime


def load_example_plan():
    return json.loads(Path("docs/workflow_plan_example.json").read_text(encoding="utf-8"))


def test_example_plan_passes_static_review():
    result = review_plan(load_example_plan())

    assert result["ok"]
    assert result["errors"] == 0
    assert result["task_count"] == 4
    assert set(result["routes"].values()) == {"mock"}


def test_static_review_blocks_premium_without_explicit_permission():
    plan = load_example_plan()
    plan["stages"][0]["tasks"][0]["route"] = "codex"

    result = review_plan(plan)

    assert not result["ok"]
    assert any(finding["code"] == "premium_route_blocked" for finding in result["findings"])


def test_static_review_detects_missing_dependencies():
    plan = load_example_plan()
    plan["stages"][1]["tasks"][0]["needs"] = ["does_not_exist"]

    result = review_plan(plan)

    assert not result["ok"]
    assert any(finding["code"] == "missing_dependency" for finding in result["findings"])


def test_static_review_detects_missing_parent_task():
    plan = load_example_plan()
    plan["stages"][1]["tasks"][0]["parent_task_id"] = "does_not_exist"

    result = review_plan(plan)

    assert not result["ok"]
    assert any(finding["code"] == "missing_parent_task" for finding in result["findings"])


def test_static_review_rejects_nested_artifact_traversal():
    plan = load_example_plan()
    plan["stages"][1]["tasks"][0]["artifacts"] = ["bench_apps/../../outside.py"]

    result = review_plan(plan)

    assert not result["ok"]
    assert any(finding["code"] == "unsafe_artifact_path" for finding in result["findings"])


def test_static_review_rejects_unknown_tools_policy():
    plan = load_example_plan()
    plan["stages"][0]["tasks"][0]["tools_policy"] = "yolo"

    result = review_plan(plan)

    assert not result["ok"]
    assert any(finding["code"] == "invalid_tools_policy" for finding in result["findings"])


def test_static_review_reports_malformed_types_without_crashing():
    plan = load_example_plan()
    plan["budget_policy"]["max_total_workers"] = "many"
    plan["stages"][0]["tasks"][0]["route"] = {"bad": "route"}
    plan["stages"][0]["tasks"][0]["artifacts"] = "docs/output.md"

    result = review_plan(plan)

    assert not result["ok"]
    codes = {finding["code"] for finding in result["findings"]}
    assert {"worker_budget", "invalid_route", "invalid_artifacts"} <= codes


def test_static_review_rejects_non_object_plan():
    result = review_plan([])

    assert not result["ok"]
    assert result["findings"][0]["code"] == "plan_type"


def test_runtime_accepts_external_workflow_plan(tmp_path):
    plan_path = tmp_path / "plan.json"
    plan = load_example_plan()
    plan_path.write_text(json.dumps(plan), encoding="utf-8")
    runtime = WorkflowRuntime(
        workflow_plan=plan_path,
        run_id="plan-run",
        run_root=tmp_path,
        global_max_concurrency=3,
        provider_max_concurrency={"mock": 3},
    )

    report = runtime.run(force=True)

    assert report["status"] == "completed"
    assert report["task_counts"] == {"completed": 4}
