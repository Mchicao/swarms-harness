import json
import subprocess
import sys

from scripts import swarm


def run_cli(*args):
    return subprocess.run([sys.executable, "scripts/swarm.py", *args], text=True, capture_output=True, timeout=120)


def test_swarm_cli_review_default_plan():
    result = run_cli("review", "--plan", "docs/workflow_plan_example.json")

    assert result.returncode == 0
    payload = json.loads(result.stdout)
    assert payload["ok"] is True
    assert payload["task_count"] == 4


def test_swarm_cli_preflight_json_lists_routes():
    result = run_cli("preflight", "--format", "json")

    assert result.returncode == 0
    payload = json.loads(result.stdout)
    assert payload["routes"]
    assert "detected_clis" in payload


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


def test_swarm_cli_passes_explicit_workspace_to_runtime(tmp_path):
    # SWARMS-005: El harness puede coordinar un repositorio vecino.
    workspace = tmp_path / "target-repo"
    args = swarm.build_parser().parse_args(
        [
            "dry-run",
            "--plan",
            "docs/workflow_plan_example.json",
            "--run-root",
            str(tmp_path / "runs"),
            "--workspace-root",
            str(workspace),
        ]
    )

    runtime = swarm.build_runtime(args)

    assert runtime.workspace_root == workspace.resolve()


def test_swarm_cli_passes_resume_to_runtime(tmp_path):
    args = swarm.build_parser().parse_args(
        [
            "dry-run",
            "--plan",
            "docs/workflow_plan_example.json",
            "--run-root",
            str(tmp_path / "runs"),
            "--run-id",
            "resume-me",
            "--resume",
        ]
    )

    assert args.resume is True


def test_swarm_cli_context_sync_is_opt_in_and_forwards_targets(monkeypatch, tmp_path):
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    captured = {}
    args = swarm.build_parser().parse_args(
        [
            "dry-run",
            "--workspace-root",
            str(workspace),
            "--sync-agent-context",
            "--context-sync-targets",
            "claudecode,codexcli",
        ]
    )
    monkeypatch.setattr(
        swarm,
        "sync_agent_context",
        lambda root, targets: captured.update(root=root, targets=targets) or {"success": True},
    )

    code, report = swarm.sync_context_or_stop(args)

    assert code == 0
    assert report == {"success": True}
    assert captured == {"root": workspace, "targets": ["claudecode", "codexcli"]}


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


def test_swarm_cli_blocks_unverified_real_route_before_dispatch(tmp_path, monkeypatch, capsys):
    plan = json.loads(open("docs/workflow_plan_example.json", encoding="utf-8").read())
    plan["stages"][0]["tasks"][0]["route"] = "glm52"
    plan_path = tmp_path / "unverified-route.json"
    plan_path.write_text(json.dumps(plan), encoding="utf-8")
    config = json.loads(open("config/swarm_router.json", encoding="utf-8").read())
    config["providers"]["glm52"]["enabled"] = True
    config_path = tmp_path / "router.json"
    config_path.write_text(json.dumps(config), encoding="utf-8")
    monkeypatch.setattr("scripts.agent_preflight.shutil.which", lambda command: command)
    monkeypatch.setattr("scripts.agent_preflight._auth_present", lambda _command: True)

    args = swarm.build_parser().parse_args(
        [
            "run",
            "--plan",
            str(plan_path),
            "--router-config",
            str(config_path),
            "--run-root",
            str(tmp_path / "runs"),
            "--run-id",
            "must-preflight",
            "--provider-cap",
            "glm52=1",
        ]
    )

    assert swarm.command_run(args) == 1
    payload = json.loads(capsys.readouterr().out)
    assert payload["findings"] == [{"code": "agent_unverified", "route": "glm52"}]
    assert not (tmp_path / "runs").exists()


def test_swarm_run_preflights_before_review(monkeypatch):
    calls = []
    args = swarm.build_parser().parse_args(["run"])

    monkeypatch.setattr(
        swarm,
        "enabled_routes_or_stop",
        lambda _args: calls.append("preflight") or 1,
    )
    monkeypatch.setattr(
        swarm,
        "review_or_stop",
        lambda _args: calls.append("review") or 0,
    )

    assert swarm.command_run(args) == 1
    assert calls == ["preflight"]
