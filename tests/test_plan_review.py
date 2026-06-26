import json

from scripts.plan_review import review_plan
from scripts.workflow_runtime import WorkflowRuntime


def load_example_plan():
    return json.loads(open("docs/workflow_plan_example.json", encoding="utf-8").read())


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
