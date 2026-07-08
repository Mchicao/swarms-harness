import json
import time
from pathlib import Path

from scripts.workflow_runtime import ClaimStore, WorkflowRuntime, WorkflowTask


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
    assert Path(command[1]).name == "gemini_worker.py"
    assert command[command.index("--model") + 1] == "Gemini 3.5 Flash (Low)"
    assert command[command.index("--tools-policy") + 1] == "none"


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
    assert Path(command[1]).name == "openai_compat_worker.py"
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
    assert Path(command[1]).name == "openai_compat_worker.py"
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
    assert Path(command[1]).name == "opencode_worker.py"
    assert command[command.index("--model") + 1] == "opencode/hy3-free"
    assert "--key-env" not in command
    assert "--base-url-env" not in command


def test_hermes_route_dispatches_to_hermes_worker(tmp_path):
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
    task = runtime.build_tasks_from_plan(plan_path)[0]
    command = runtime.worker_command(task, tmp_path / "prompt.txt", tmp_path / "status.json")

    assert task.provider == "hermes"
    assert task.wrapper == "hermes"
    assert Path(command[1]).name == "hermes_worker.py"
    # Hermes is a routing agent — no OpenAI-compat env-var injection.
    assert "--key-env" not in command
    assert "--base-url-env" not in command
    # tools-policy is still passed through the standard contract.
    assert command[command.index("--tools-policy") + 1] == "full"
    # The plain hermes route has empty model — no --provider injection needed.
    assert "--provider" not in command


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
    assert Path(command[1]).name == "hermes_worker.py"
    assert command[command.index("--model") + 1] == "tencent/hy3:free"
    # CRITICAL: --provider nous MUST be injected so Hermes uses the Nous Portal
    # free tier (tencent/hy3:free is $0 there), not its paid glm-5.2 default.
    assert "--provider" in command
    assert command[command.index("--provider") + 1] == "nous"
