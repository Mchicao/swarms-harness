import json
import time
from pathlib import Path

import pytest

from scripts import workflow_runtime
from scripts.workflow_runtime import ClaimStore, WorkflowRuntime, WorkflowTask, parse_provider_caps


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


def test_new_workflow_initializes_without_force(tmp_path):
    runtime = WorkflowRuntime(run_id="fresh", run_root=tmp_path, global_max_concurrency=2)

    report = runtime.run("micro-reshard-roundtrip", dry_run=True)

    assert report["status"] == "planned"
    assert report["task_counts"] == {"pending": 5}


def test_force_reinitialization_removes_stale_task_files(tmp_path):
    runtime = WorkflowRuntime(run_id="reused", run_root=tmp_path, global_max_concurrency=2)
    runtime.run("micro-reshard-roundtrip", dry_run=True)
    stale = runtime.tasks_dir / "stale.json"
    stale.write_text('{"stale": true}', encoding="utf-8")

    runtime.run("micro-reshard-roundtrip", dry_run=True, force=True)

    assert not stale.exists()
    assert len(runtime.load_tasks()) == 5


@pytest.mark.parametrize("run_id", ["..", ".", "../victim", "..\\victim", "nested/run"])
def test_run_id_cannot_escape_run_root(tmp_path, run_id):
    sentinel = tmp_path / "victim" / "keep.txt"
    sentinel.parent.mkdir()
    sentinel.write_text("keep", encoding="utf-8")

    with pytest.raises(ValueError, match="Unsafe run_id"):
        WorkflowRuntime(run_id=run_id, run_root=tmp_path)

    assert sentinel.read_text(encoding="utf-8") == "keep"


def test_provider_cap_is_reserved_within_ready_wave(tmp_path):
    runtime = WorkflowRuntime(
        run_id="caps",
        run_root=tmp_path,
        global_max_concurrency=4,
        provider_max_concurrency={"glm52": 1},
    )
    tasks = [
        WorkflowTask(
            task_id=f"000{index}-task-{index}",
            source_id=f"task-{index}",
            stage="Work",
            index=index,
            text="[programmer] Work",
            role="programmer",
            route="glm52",
            provider="opencode",
        )
        for index in range(3)
    ]

    assert len(runtime.ready_tasks(tasks)) == 1


def test_antigravity_concurrency_is_clamped_until_responses_can_be_correlated(tmp_path):
    runtime = WorkflowRuntime(
        run_id="agy-cap",
        run_root=tmp_path,
        provider_max_concurrency={"gemini_flash": 3},
    )

    assert runtime.provider_max_concurrency["gemini_flash"] == 1


def test_negative_provider_cap_is_rejected():
    with pytest.raises(ValueError, match="cannot be negative"):
        parse_provider_caps(["mock=-1"])


def test_plan_dependencies_match_source_id_exactly(tmp_path):
    runtime = WorkflowRuntime(run_id="exact-deps", run_root=tmp_path)
    auth = WorkflowTask(
        task_id="0000-auth",
        source_id="auth",
        stage="Work",
        index=0,
        text="[programmer] Auth",
        role="programmer",
    )
    authz = WorkflowTask(
        task_id="0001-authz",
        source_id="authz",
        stage="Work",
        index=1,
        text="[programmer] Authz",
        role="programmer",
        status="completed",
    )
    consumer = WorkflowTask(
        task_id="0002-consumer",
        source_id="consumer",
        stage="Work",
        index=2,
        text="[qa] Verify",
        role="qa",
        needs=["auth"],
    )

    assert not runtime.dependency_satisfied(consumer, [auth, authz, consumer])
    auth.status = "completed"
    assert runtime.dependency_satisfied(consumer, [auth, authz, consumer])


def test_interrupted_tasks_become_blocked_instead_of_spinning(tmp_path):
    runtime = WorkflowRuntime(run_id="resume", run_root=tmp_path)
    tasks = runtime.initialize("micro-reshard-roundtrip", force=True)
    tasks[0].status = "queued"
    runtime.save_task(tasks[0])

    resumed = WorkflowRuntime(run_id="resume", run_root=tmp_path)
    report = resumed.run("micro-reshard-roundtrip", dry_run=True)

    assert report["task_counts"]["blocked"] == 1


def test_future_exception_is_persisted_as_failed_result(tmp_path, monkeypatch):
    runtime = WorkflowRuntime(
        run_id="future-error",
        run_root=tmp_path,
        global_max_concurrency=1,
        provider_max_concurrency={"mock": 1},
    )

    def fail_task(*_args, **_kwargs):
        raise RuntimeError("worker crashed")

    monkeypatch.setattr(runtime, "run_task", fail_task)

    report = runtime.run("micro-reshard-roundtrip", force=True)

    assert report["status"] == "failed"
    assert report["results"][0]["returncode"] == 70
    assert "worker crashed" in report["results"][0]["error"]


def test_runtime_refuses_disabled_route_before_worker_dispatch(tmp_path):
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Review",
                        "tasks": [
                            {
                                "id": "review",
                                "role": "critic",
                                "route": "glm52",
                                "task": "Review only.",
                                "needs": [],
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    runtime = WorkflowRuntime(
        workflow_plan=plan_path,
        run_id="disabled",
        run_root=tmp_path / "runs",
        router_config=Path("config/swarm_router.json"),
        provider_max_concurrency={"glm52": 1},
    )

    with pytest.raises(ValueError, match="Routes are disabled"):
        runtime.run(force=True)

    assert not runtime.results_dir.exists()


def test_runtime_uses_router_config_as_route_source(tmp_path):
    config = json.loads(Path("config/swarm_router.json").read_text(encoding="utf-8"))
    config["providers"]["glm52"]["model"] = "custom/glm"
    config_path = tmp_path / "router.json"
    config_path.write_text(json.dumps(config), encoding="utf-8")
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Review",
                        "tasks": [
                            {
                                "id": "review",
                                "role": "critic",
                                "route": "glm52",
                                "task": "Review only.",
                                "needs": [],
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )

    runtime = WorkflowRuntime(
        workflow_plan=plan_path, run_id="configured", run_root=tmp_path, router_config=config_path
    )

    assert runtime.build_tasks_from_plan(plan_path)[0].model == "custom/glm"


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


def test_gemini_route_dispatches_to_agy_low_worker(tmp_path):
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Plan",
                        "tasks": [
                            {
                                "id": "plan",
                                "role": "planner",
                                "route": "gemini_flash",
                                "task": "Analyze a hard problem.",
                                "artifacts": [],
                                "needs": [],
                                "tools_policy": "none",
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    runtime = WorkflowRuntime(workflow_plan=plan_path, run_id="agy", run_root=tmp_path)
    task = runtime.build_tasks_from_plan(plan_path)[0]
    command = runtime.worker_command(task, tmp_path / "prompt.txt", tmp_path / "status.json")

    assert task.model == "Gemini 3.5 Flash (Low)"
    assert command[1:3] == ["-m", "scripts.gemini_worker"]
    assert command[command.index("--model") + 1] == "Gemini 3.5 Flash (Low)"
    assert command[command.index("--tools-policy") + 1] == "none"


def test_glm52_route_uses_installed_opencode_model_identifier(tmp_path):
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Review",
                        "tasks": [
                            {
                                "id": "review",
                                "role": "critic",
                                "route": "glm52",
                                "task": "Review a bounded change.",
                                "artifacts": [],
                                "needs": [],
                                "tools_policy": "none",
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    runtime = WorkflowRuntime(workflow_plan=plan_path, run_id="glm52", run_root=tmp_path)
    task = runtime.build_tasks_from_plan(plan_path)[0]
    command = runtime.worker_command(task, tmp_path / "prompt.txt", tmp_path / "status.json")

    assert task.provider == "opencode"
    assert task.model == "zai-coding-plan/glm-5.2"
    assert command[1:3] == ["-m", "scripts.opencode_worker"]
    assert command[command.index("--model") + 1] == "zai-coding-plan/glm-5.2"


def test_dependency_outputs_are_included_in_downstream_prompt(tmp_path):
    runtime = WorkflowRuntime(run_id="context", run_root=tmp_path)
    dependency = WorkflowTask(
        task_id="0000-analysis",
        stage="Analysis",
        index=0,
        text="[planner] Analyze",
        role="planner",
        status="completed",
    )
    downstream = WorkflowTask(
        task_id="0001-review",
        stage="Review",
        index=1,
        text="[critic] Review",
        role="critic",
        needs=["analysis"],
    )
    log_dir = runtime.results_dir / dependency.task_id
    log_dir.mkdir(parents=True)
    (log_dir / "worker.log").write_text("DEPENDENCY_RESULT_42", encoding="utf-8")

    prompt = runtime.write_prompt(downstream, tmp_path, [dependency, downstream])

    assert "DEPENDENCY_RESULT_42" in prompt.read_text(encoding="utf-8")


def test_real_provider_usage_is_marked_missing_not_free(tmp_path):
    runtime = WorkflowRuntime(run_id="usage", run_root=tmp_path)
    task = WorkflowTask(
        task_id="0000-real",
        stage="Plan",
        index=0,
        text="[planner] Analyze",
        role="planner",
        provider="antigravity_cli",
        model="Gemini 3.5 Flash (Low)",
        wrapper="gemini",
        status="completed",
    )

    usage = runtime.report([task], status="completed")["token_usage"]

    assert usage["known_cost_usd"] is None
    assert usage["usage_source"] == "missing"


def test_hy3_openrouter_route_dispatches_to_openai_compat_worker(tmp_path):
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Implement",
                        "tasks": [
                            {
                                "id": "impl",
                                "role": "programmer",
                                "route": "hy3_openrouter",
                                "task": "Add a helper function.",
                                "artifacts": [],
                                "needs": [],
                                "tools_policy": "none",
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    runtime = WorkflowRuntime(workflow_plan=plan_path, run_id="hy3", run_root=tmp_path)
    task = runtime.build_tasks_from_plan(plan_path)[0]
    command = runtime.worker_command(task, tmp_path / "prompt.txt", tmp_path / "status.json")

    assert task.provider == "openrouter"
    assert task.model == "tencent/hy3:free"
    assert command[1:3] == ["-m", "scripts.openai_compat_worker"]
    assert command[command.index("--model") + 1] == "tencent/hy3:free"
    # The runtime must inject the provider's key env var so the worker reads
    # the right secret without hardcoding it anywhere in the repo.
    assert command[command.index("--key-env") + 1] == "OPENROUTER_API_KEY"
    assert command[command.index("--base-url-env") + 1] == "OPENROUTER_BASE_URL"


def test_hy3_novita_route_uses_novita_key_env(tmp_path):
    runtime = WorkflowRuntime(run_root=tmp_path)
    task = WorkflowTask(
        task_id="0000-novita",
        stage="Implement",
        index=0,
        text="[programmer] Build it",
        role="programmer",
        provider="novita",
        model="tencent/hy3",
        wrapper="openai_compat",
    )
    command = runtime.worker_command(task, tmp_path / "prompt.txt", tmp_path / "status.json")
    assert command[command.index("--key-env") + 1] == "NOVITA_API_KEY"
    assert command[command.index("--base-url-env") + 1] == "NOVITA_BASE_URL"


def test_hy3_gitlawb_route_dispatches_to_openai_compat_worker(tmp_path):
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Implement",
                        "tasks": [
                            {
                                "id": "impl",
                                "role": "programmer",
                                "route": "hy3_gitlawb",
                                "task": "Add a helper function.",
                                "artifacts": [],
                                "needs": [],
                                "tools_policy": "none",
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    runtime = WorkflowRuntime(workflow_plan=plan_path, run_id="gitlawb", run_root=tmp_path)
    task = runtime.build_tasks_from_plan(plan_path)[0]
    command = runtime.worker_command(task, tmp_path / "prompt.txt", tmp_path / "status.json")

    assert task.provider == "gitlawb"
    assert task.model == "tencent/hy3"
    assert command[1:3] == ["-m", "scripts.openai_compat_worker"]
    assert command[command.index("--key-env") + 1] == "GITLAWB_API_KEY"
    assert command[command.index("--base-url-env") + 1] == "GITLAWB_BASE_URL"


def test_hy3_opencode_route_reuses_existing_opencode_worker(tmp_path):
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Implement",
                        "tasks": [
                            {
                                "id": "impl",
                                "role": "programmer",
                                "route": "hy3_opencode",
                                "task": "Add a helper function.",
                                "artifacts": [],
                                "needs": [],
                                "tools_policy": "none",
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    runtime = WorkflowRuntime(workflow_plan=plan_path, run_id="oc-hy3", run_root=tmp_path)
    task = runtime.build_tasks_from_plan(plan_path)[0]
    command = runtime.worker_command(task, tmp_path / "prompt.txt", tmp_path / "status.json")

    assert task.provider == "opencode"
    assert task.model == "opencode/hy3-free"
    # Reuses the existing opencode worker (auth handled by OpenCode's store,
    # no env-var-key injection needed).
    assert command[1:3] == ["-m", "scripts.opencode_worker"]
    assert command[command.index("--model") + 1] == "opencode/hy3-free"
    assert "--key-env" not in command
    assert "--base-url-env" not in command


def test_hy3_kilo_route_uses_kilo_cli_worker(tmp_path):
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Implement",
                        "tasks": [
                            {
                                "id": "impl",
                                "role": "programmer",
                                "route": "hy3_kilo",
                                "task": "Add a helper function.",
                                "artifacts": [],
                                "needs": [],
                                "tools_policy": "none",
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    runtime = WorkflowRuntime(workflow_plan=plan_path, run_id="kilo", run_root=tmp_path)
    task = runtime.build_tasks_from_plan(plan_path)[0]
    command = runtime.worker_command(task, tmp_path / "prompt.txt", tmp_path / "status.json")

    assert task.provider == "kilo_cli"
    assert task.model == "kilo/tencent/hy3:free"
    assert command[1:3] == ["-m", "scripts.kilo_worker"]


def test_gitlawb_route_uses_its_gateway_default(tmp_path):
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Review",
                        "tasks": [
                            {
                                "id": "review",
                                "route": "hy3_gitlawb",
                                "task": "Return OK.",
                                "artifacts": [],
                                "needs": [],
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    runtime = WorkflowRuntime(workflow_plan=plan_path, run_id="gitlawb", run_root=tmp_path)
    task = runtime.build_tasks_from_plan(plan_path)[0]
    command = runtime.worker_command(task, tmp_path / "prompt.txt", tmp_path / "status.json")

    assert command[-2:] == ["--base-url", "https://opengateway.gitlawb.com/v1"]


def test_write_json_atomic_retries_transient_windows_file_locks(tmp_path, monkeypatch):
    target = tmp_path / "task.json"
    original_replace = Path.replace
    attempts = 0

    def flaky_replace(self, destination):
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            raise PermissionError("locked")
        return original_replace(self, destination)

    monkeypatch.setattr(Path, "replace", flaky_replace)
    monkeypatch.setattr(workflow_runtime.time, "sleep", lambda _seconds: None)

    workflow_runtime.write_json_atomic(target, {"status": "completed"})

    assert attempts == 2
    assert json.loads(target.read_text(encoding="utf-8")) == {"status": "completed"}


def test_hermes_route_requires_an_explicit_model(tmp_path):
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Implement",
                        "tasks": [
                            {
                                "id": "impl",
                                "role": "programmer",
                                "route": "hermes",
                                "task": "Refactor the auth module and add tests.",
                                "artifacts": [],
                                "needs": [],
                                "tools_policy": "full",
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    runtime = WorkflowRuntime(workflow_plan=plan_path, run_id="hermes", run_root=tmp_path)

    with pytest.raises(ValueError, match="pin an explicit model"):
        runtime.build_tasks_from_plan(plan_path)


def test_hy3_hermes_route_forces_free_model_and_nous_provider(tmp_path):
    """Safety-critical: hy3_hermes must inject --provider nous so Hermes
    never falls back to its paid glm-5.2 Z.AI default. This is the route users
    pick when they want $0 cost guaranteed."""
    plan_path = tmp_path / "plan.json"
    plan_path.write_text(
        json.dumps(
            {
                "stages": [
                    {
                        "name": "Implement",
                        "tasks": [
                            {
                                "id": "impl",
                                "role": "programmer",
                                "route": "hy3_hermes",
                                "task": "Add a helper function.",
                                "artifacts": [],
                                "needs": [],
                                "tools_policy": "none",
                            }
                        ],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    runtime = WorkflowRuntime(workflow_plan=plan_path, run_id="hy3hermes", run_root=tmp_path)
    task = runtime.build_tasks_from_plan(plan_path)[0]
    command = runtime.worker_command(task, tmp_path / "prompt.txt", tmp_path / "status.json")

    assert task.model == "tencent/hy3:free"
    assert command[1:3] == ["-m", "scripts.hermes_worker"]
    assert command[command.index("--model") + 1] == "tencent/hy3:free"
    # CRITICAL: --provider nous MUST be injected so Hermes uses the Nous Portal
    # free tier (tencent/hy3:free is $0 there), not its paid glm-5.2 default.
    assert "--provider" in command
    assert command[command.index("--provider") + 1] == "nous"
