import json
import subprocess
import sys


def run_cli(*args):
    return subprocess.run([sys.executable, "scripts/swarm.py", *args], text=True, capture_output=True, timeout=120)


def test_swarm_cli_review_default_plan():
    result = run_cli("review", "--plan", "docs/workflow_plan_example.json")

    assert result.returncode == 0
    payload = json.loads(result.stdout)
    assert payload["ok"] is True
    assert payload["task_count"] == 4


def test_swarm_cli_dry_run_default_plan(tmp_path):
    result = run_cli(
        "dry-run",
        "--plan",
        "docs/workflow_plan_example.json",
        "--run-root",
        str(tmp_path),
        "--run-id",
        "cli-dry",
        "--force",
    )

    assert result.returncode == 0
    payload = json.loads(result.stdout)
    assert payload["status"] == "planned"


def test_swarm_cli_run_default_plan(tmp_path):
    result = run_cli(
        "run",
        "--plan",
        "docs/workflow_plan_example.json",
        "--run-root",
        str(tmp_path),
        "--run-id",
        "cli-run",
        "--global-max-concurrency",
        "3",
        "--force",
    )

    assert result.returncode == 0
    payload = json.loads(result.stdout)
    assert payload["status"] == "completed"
    assert payload["task_counts"] == {"completed": 4}
    assert all(item["success"] for item in payload["results"])


def test_swarm_cli_blocks_disabled_real_route_before_dispatch(tmp_path):
    plan = json.loads(open("docs/workflow_plan_example.json", encoding="utf-8").read())
    plan["stages"][0]["tasks"][0]["route"] = "glm52"
    plan_path = tmp_path / "disabled-route.json"
    plan_path.write_text(json.dumps(plan), encoding="utf-8")

    result = run_cli(
        "run",
        "--plan",
        str(plan_path),
        "--router-config",
        "config/swarm_router.json",
        "--run-root",
        str(tmp_path / "runs"),
        "--run-id",
        "must-not-dispatch",
        "--provider-cap",
        "glm52=1",
    )

    assert result.returncode == 1
    payload = json.loads(result.stdout)
    assert payload["findings"] == [{"code": "route_disabled", "route": "glm52", "task_id": "reshard_plan"}]
    assert not (tmp_path / "runs").exists()
