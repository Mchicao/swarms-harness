from __future__ import annotations

import json
from pathlib import Path

import pytest

from scripts.workflow_ir import WorkflowCompileError, compile_plan, validate_plan_limits


def dynamic_plan() -> dict:
    """WORKFLOW-IR-TEST-001: Return a compact plan exercising every IR step."""
    return {
        "schema_version": 2,
        "goal": "Audit and summarize files",
        "review_policy": {"premium_allowed": False},
        "budget_policy": {
            "max_total_workers": 12,
            "max_depth": 2,
            "max_children_per_agent": 4,
            "max_rounds": 3,
            "global_max_concurrency": 3,
            "provider_concurrency": {"mock": 3},
        },
        "workflow": {
            "steps": [
                {"id": "discover", "type": "agent", "route": "mock", "role": "planner", "task": "Discover"},
                {
                    "id": "audit",
                    "type": "map",
                    "items": ["a.py", "b.py"],
                    "item_name": "file",
                    "route": "mock",
                    "role": "programmer",
                    "task": "Audit {file}",
                    "needs": ["discover"],
                    "parent": "discover",
                },
                {"id": "merge", "type": "reduce", "from": ["audit"], "route": "mock", "task": "Merge"},
                {"id": "check", "type": "verify", "route": "mock", "task": "Verify", "needs": ["merge"]},
                {"id": "optional", "type": "condition", "when": False, "step": {"type": "agent", "task": "Skip"}},
                {
                    "id": "polish",
                    "type": "loop",
                    "max_rounds": 2,
                    "step": {"type": "agent", "route": "mock", "task": "Polish round {round}"},
                    "needs": ["check"],
                },
            ]
        },
    }


def test_compile_plan_expands_dynamic_steps_deterministically():
    first = compile_plan(dynamic_plan())
    second = compile_plan(dynamic_plan())

    assert first == second
    assert compile_plan(first) == first
    tasks = [task for stage in first["stages"] for task in stage["tasks"]]
    assert [task["id"] for task in tasks] == [
        "discover",
        "audit-000-a.py",
        "audit-001-b.py",
        "merge",
        "check",
        "polish-round-001",
        "polish-round-002",
    ]
    assert tasks[3]["needs"] == ["audit-000-a.py", "audit-001-b.py"]
    assert tasks[4]["role"] == "verifier"
    assert tasks[-1]["needs"] == ["polish-round-001"]


@pytest.mark.parametrize(
    ("field", "value", "match"),
    [
        ("max_total_workers", 3, "max_total_workers"),
        ("max_children_per_agent", 1, "max_children_per_agent"),
        ("max_depth", 0, "max_depth"),
        ("max_rounds", 1, "max_rounds"),
    ],
)
def test_compile_plan_enforces_hard_recursion_limits(field, value, match):
    plan = dynamic_plan()
    plan["budget_policy"][field] = value

    with pytest.raises(WorkflowCompileError, match=match):
        compile_plan(plan)


def test_version_one_plan_is_preserved():
    plan = json.loads(Path("docs/workflow_plan_example.json").read_text(encoding="utf-8"))

    assert compile_plan(plan) == plan


def test_worker_cannot_enable_opaque_recursive_spawning():
    plan = dynamic_plan()
    plan["workflow"]["steps"][0]["allow_subagent_spawning"] = True

    with pytest.raises(WorkflowCompileError, match="machine-locked to 0"):
        compile_plan(plan)


def test_flat_plan_rejects_parent_and_dependency_cycles():
    plan = {
        "schema_version": 1,
        "budget_policy": {"max_total_workers": 12, "max_depth": 2},
        "stages": [
            {
                "tasks": [
                    {"id": "a", "parent_task_id": "b", "needs": ["b"]},
                    {"id": "b", "parent_task_id": "a", "needs": ["a"]},
                ]
            }
        ],
    }

    with pytest.raises(WorkflowCompileError, match="Parent cycle"):
        validate_plan_limits(plan)


def test_flat_plan_rejects_positive_spawn_budget_and_worker_override():
    plan = {
        "schema_version": 1,
        "budget_policy": {"spawn_budget": 1},
        "stages": [{"tasks": [{"id": "root", "allow_subagent_spawning": True}]}],
    }

    with pytest.raises(WorkflowCompileError, match="spawn_budget"):
        validate_plan_limits(plan)
