"""Compile a small, bounded workflow IR into ordinary SWARMS stages."""

from __future__ import annotations

import copy
import json
import re
import sys
from dataclasses import dataclass
from typing import Any


class WorkflowCompileError(ValueError):
    """Raised when workflow expansion is invalid or exceeds a hard limit."""


@dataclass(frozen=True)
class WorkflowLimits:
    """Hard limits that a generated workflow cannot override."""

    max_total_workers: int = 12
    max_depth: int = 2
    max_children_per_agent: int = 4
    max_rounds: int = 4

    @classmethod
    def from_plan(cls, plan: dict[str, Any]) -> WorkflowLimits:
        budget = plan.get("budget_policy", {})
        try:
            limits = cls(
                max_total_workers=int(budget.get("max_total_workers", 12)),
                max_depth=int(budget.get("max_depth", 2)),
                max_children_per_agent=int(budget.get("max_children_per_agent", 4)),
                max_rounds=int(budget.get("max_rounds", 4)),
            )
        except (TypeError, ValueError) as exc:
            raise WorkflowCompileError(f"Workflow limits must be integers: {exc}") from exc
        if min(limits.max_total_workers, limits.max_children_per_agent, limits.max_rounds) < 1:
            raise WorkflowCompileError("Worker, child, and round limits must be positive")
        if limits.max_depth < 0:
            raise WorkflowCompileError("max_depth cannot be negative")
        return limits


def validate_plan_limits(plan: dict[str, Any]) -> None:
    """SWARMS-WORKFLOW-002: Enforce non-recursive limits on every plan version."""
    limits = WorkflowLimits.from_plan(plan)
    budget = plan.get("budget_policy", {})
    try:
        spawn_budget = int(budget.get("spawn_budget", 0))
    except (TypeError, ValueError) as exc:
        raise WorkflowCompileError("spawn_budget must be an integer") from exc
    if spawn_budget != 0:
        raise WorkflowCompileError("spawn_budget must remain 0 until runtime-controlled task insertion is available")

    tasks = [
        task
        for stage in plan.get("stages", [])
        if isinstance(stage, dict)
        for task in stage.get("tasks", [])
        if isinstance(task, dict)
    ]
    if len(tasks) > limits.max_total_workers:
        raise WorkflowCompileError(f"Plan exceeds max_total_workers={limits.max_total_workers}")
    if any(task.get("allow_subagent_spawning") for task in tasks):
        raise WorkflowCompileError("allow_subagent_spawning is machine-locked to false")

    by_id = {str(task.get("id")): task for task in tasks if task.get("id")}
    child_counts: dict[str, int] = {}
    parent_of: dict[str, str] = {}
    needs_of: dict[str, list[str]] = {}
    for task_id, task in by_id.items():
        parent = task.get("parent_task_id") or task.get("parent_id")
        if parent:
            parent = str(parent)
            if parent not in by_id:
                raise WorkflowCompileError(f"Invalid parent chain for {task_id!r}")
            parent_of[task_id] = parent
            child_counts[parent] = child_counts.get(parent, 0) + 1
            if child_counts[parent] > limits.max_children_per_agent:
                raise WorkflowCompileError(
                    f"Parent {parent!r} exceeds max_children_per_agent={limits.max_children_per_agent}"
                )
        needs = task.get("needs", [])
        if isinstance(needs, list):
            needs_of[task_id] = [str(need) for need in needs if str(need) in by_id]

    def visit_parent(task_id: str, trail: set[str]) -> int:
        if task_id in trail:
            raise WorkflowCompileError(f"Parent cycle includes {task_id!r}")
        parent = parent_of.get(task_id)
        if not parent:
            return 0
        depth = 1 + visit_parent(parent, {*trail, task_id})
        if depth > limits.max_depth:
            raise WorkflowCompileError(f"Task {task_id!r} exceeds max_depth={limits.max_depth}")
        return depth

    def visit_needs(task_id: str, trail: set[str], done: set[str]) -> None:
        if task_id in trail:
            raise WorkflowCompileError(f"Dependency cycle includes {task_id!r}")
        if task_id in done:
            return
        for dependency in needs_of.get(task_id, []):
            visit_needs(dependency, {*trail, task_id}, done)
        done.add(task_id)

    completed: set[str] = set()
    for task_id in by_id:
        visit_parent(task_id, set())
        visit_needs(task_id, set(), completed)


def _slug(value: object) -> str:
    clean = re.sub(r"[^A-Za-z0-9_.-]+", "-", str(value).strip()).strip("-")
    return clean[:48] or "item"


def _format(value: Any, variables: dict[str, Any]) -> Any:
    if isinstance(value, str):
        try:
            return value.format_map(variables)
        except KeyError as exc:
            raise WorkflowCompileError(f"Unknown workflow template variable: {exc.args[0]}") from exc
    if isinstance(value, list):
        return [_format(item, variables) for item in value]
    return value


def compile_plan(plan: dict[str, Any]) -> dict[str, Any]:
    """Expand schema-version-2 workflow steps into deterministic stage tasks."""
    if not isinstance(plan, dict) or plan.get("schema_version") != 2:
        return plan
    if plan.get("workflow_compiled") is True and "workflow" not in plan:
        validate_plan_limits(plan)
        return plan
    workflow = plan.get("workflow")
    if not isinstance(workflow, dict) or not isinstance(workflow.get("steps"), list):
        raise WorkflowCompileError("schema_version 2 requires workflow.steps")

    limits = WorkflowLimits.from_plan(plan)
    validate_plan_limits({**plan, "stages": plan.get("stages", [])})
    compiled = copy.deepcopy(plan)
    compiled.pop("workflow", None)
    compiled["workflow_compiled"] = True
    stages = compiled.setdefault("stages", [])
    if not isinstance(stages, list):
        raise WorkflowCompileError("stages must be a list")
    generated: list[dict[str, Any]] = []
    outputs: dict[str, list[str]] = {}

    def resolve_refs(refs: Any) -> list[str]:
        if refs is None:
            return []
        if not isinstance(refs, list):
            raise WorkflowCompileError("needs/from must be a list")
        resolved: list[str] = []
        for ref in refs:
            resolved.extend(outputs.get(str(ref), [str(ref)]))
        return list(dict.fromkeys(resolved))

    def add_task(step: dict[str, Any], task_id: str, variables: dict[str, Any], needs: list[str]) -> str:
        if step.get("allow_subagent_spawning"):
            raise WorkflowCompileError(
                f"Task {task_id!r} cannot enable subagent spawning while spawn_budget is machine-locked to 0"
            )
        task = {
            "id": task_id,
            "role": step.get("role", "general"),
            "route": step.get("route", "mock"),
            "task": _format(step.get("task", ""), variables),
            "artifacts": _format(step.get("artifacts", []), variables),
            "needs": needs,
            "verify": _format(step.get("verify", []), variables),
            "tools_policy": step.get("tools_policy", "none"),
            "allow_subagent_spawning": bool(step.get("allow_subagent_spawning", False)),
        }
        parent = step.get("parent") or step.get("parent_task_id")
        if parent:
            parents = outputs.get(str(parent), [str(parent)])
            if len(parents) != 1:
                raise WorkflowCompileError(f"Parent step {parent!r} must produce exactly one task")
            task["parent_task_id"] = parents[0]
        generated.append(task)
        if len(generated) > limits.max_total_workers:
            raise WorkflowCompileError(f"Expanded workflow exceeds max_total_workers={limits.max_total_workers}")
        return task_id

    def expand(
        step: dict[str, Any], inherited_id: str | None = None, inherited_needs: list[str] | None = None
    ) -> list[str]:
        if not isinstance(step, dict):
            raise WorkflowCompileError("Every workflow step must be an object")
        step_type = str(step.get("type", "agent"))
        step_id = str(step.get("id") or inherited_id or "").strip()
        if not step_id:
            raise WorkflowCompileError("Every workflow step requires id")
        needs = resolve_refs(step.get("needs", inherited_needs or []))

        if step_type in {"agent", "verify", "reduce"}:
            normalized = dict(step)
            if step_type == "verify":
                normalized["role"] = "verifier"
            if step_type == "reduce":
                needs = resolve_refs(step.get("from", [])) + needs
                needs = list(dict.fromkeys(needs))
            return [add_task(normalized, step_id, {}, needs)]
        if step_type == "map":
            items = step.get("items")
            if not isinstance(items, list):
                raise WorkflowCompileError(f"map step {step_id!r} requires literal items")
            item_name = str(step.get("item_name", "item"))
            return [
                add_task(step, f"{step_id}-{index:03d}-{_slug(item)}", {item_name: item, "index": index}, needs)
                for index, item in enumerate(items)
            ]
        if step_type == "condition":
            if not isinstance(step.get("when"), bool):
                raise WorkflowCompileError(f"condition step {step_id!r} requires a boolean when")
            if not step["when"]:
                return []
            nested = dict(step.get("step") or {})
            nested.setdefault("id", step_id)
            return expand(nested, step_id, needs)
        if step_type == "loop":
            rounds = int(step.get("max_rounds", 1))
            if rounds < 1 or rounds > limits.max_rounds:
                raise WorkflowCompileError(f"loop step {step_id!r} exceeds max_rounds={limits.max_rounds}")
            nested = dict(step.get("step") or {})
            produced: list[str] = []
            round_needs = needs
            for round_number in range(1, rounds + 1):
                round_id = f"{step_id}-round-{round_number:03d}"
                normalized = {
                    **nested,
                    "id": round_id,
                    "task": _format(nested.get("task", ""), {"round": round_number}),
                    "needs": round_needs,
                }
                round_ids = expand(normalized, round_id, round_needs)
                produced.extend(round_ids)
                round_needs = round_ids
            return produced
        raise WorkflowCompileError(f"Unsupported workflow step type: {step_type}")

    for step in workflow["steps"]:
        step_id = str(step.get("id", "")) if isinstance(step, dict) else ""
        if step_id in outputs:
            raise WorkflowCompileError(f"Duplicate workflow step id: {step_id}")
        outputs[step_id] = expand(step)

    existing = [task for stage in stages if isinstance(stage, dict) for task in stage.get("tasks", [])]
    if len(existing) + len(generated) > limits.max_total_workers:
        raise WorkflowCompileError(f"Plan exceeds max_total_workers={limits.max_total_workers}")

    by_id = {str(task.get("id")): task for task in [*existing, *generated] if isinstance(task, dict)}
    child_counts: dict[str, int] = {}

    def depth(task: dict[str, Any], seen: set[str] | None = None) -> int:
        parent = task.get("parent_task_id") or task.get("parent_id")
        if not parent:
            return 0
        seen = set() if seen is None else seen
        if str(parent) in seen or str(parent) not in by_id:
            raise WorkflowCompileError(f"Invalid parent chain for {task.get('id')!r}")
        seen.add(str(parent))
        return 1 + depth(by_id[str(parent)], seen)

    for task in [*existing, *generated]:
        parent = task.get("parent_task_id") or task.get("parent_id")
        task_depth = depth(task)
        task["depth"] = task_depth
        if task_depth > limits.max_depth:
            raise WorkflowCompileError(f"Task {task.get('id')!r} exceeds max_depth={limits.max_depth}")
        if parent:
            child_counts[str(parent)] = child_counts.get(str(parent), 0) + 1
            if child_counts[str(parent)] > limits.max_children_per_agent:
                raise WorkflowCompileError(
                    f"Parent {parent!r} exceeds max_children_per_agent={limits.max_children_per_agent}"
                )

    if generated:
        stages.append({"name": "Dynamic Workflow", "parallel": True, "tasks": generated})
    validate_plan_limits(compiled)
    return compiled


def main(argv: list[str] | None = None) -> int:
    """WORKFLOW-IR-CLI-001: Print a compiled plan for the Rust coordinator."""
    args = list(sys.argv[1:] if argv is None else argv)
    if len(args) != 1:
        print("usage: python -m scripts.workflow_ir PLAN.json", file=sys.stderr)
        return 2
    try:
        source = json.loads(open(args[0], encoding="utf-8").read())
        print(json.dumps(compile_plan(source), sort_keys=True))
        return 0
    except (OSError, json.JSONDecodeError, WorkflowCompileError) as exc:
        print(f"workflow compile failed: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
