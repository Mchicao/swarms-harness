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
        "--provider-cap",
        "mock=3",
        "--force",
    )

    assert result.returncode == 0
    payload = json.loads(result.stdout)
    assert payload["status"] == "completed"
    assert payload["task_counts"] == {"completed": 4}
    assert all(item["success"] for item in payload["results"])
