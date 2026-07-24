#!/usr/bin/env python3
"""Static review for SWARMS workflow plans.

The cheap deterministic reviewer runs before any model worker. It catches plan
shape problems, unsafe scope, missing verification, dependency mistakes, and
unapproved premium usage.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any

try:
    from .paths import PROJECT_ROOT
except ImportError:  # pragma: no cover - direct script execution path.
    from paths import PROJECT_ROOT

DEFAULT_ROLE_POLICY = PROJECT_ROOT / "config" / "role_policy.json"
PREMIUM_ROUTES = {"codex", "claude", "opus", "gpt55", "gpt-5.5"}
ALLOWED_BENCHMARK_PREFIXES = ("bench_apps/", "bench_tests/", "docs/bench_notes/")
VALID_ROLES = {"planner", "critic", "programmer", "verifier", "docs", "backend", "qa", "debug", "general"}
VALID_TOOLS_POLICIES = {"none", "full"}


@dataclass
class Finding:
    severity: str
    code: str
    message: str
    task_id: str | None = None

    def to_dict(self) -> dict[str, Any]:
        data = {"severity": self.severity, "code": self.code, "message": self.message}
        if self.task_id:
            data["task_id"] = self.task_id
        return data


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def iter_tasks(plan: dict[str, Any]) -> list[tuple[str, dict[str, Any]]]:
    tasks: list[tuple[str, dict[str, Any]]] = []
    stages = plan.get("stages", [])
    if not isinstance(stages, list):
        return tasks
    for stage in stages:
        if not isinstance(stage, dict):
            continue
        stage_name = str(stage.get("name", "Unnamed"))
        stage_tasks = stage.get("tasks", [])
        if isinstance(stage_tasks, list):
            tasks.extend((stage_name, task) for task in stage_tasks if isinstance(task, dict))
    return tasks


def is_premium_route(route: str | None) -> bool:
    if not route:
        return False
    lowered = route.lower()
    return lowered in PREMIUM_ROUTES or any(marker in lowered for marker in PREMIUM_ROUTES)


def is_safe_repo_path(value: Any) -> bool:
    """Return whether *value* is a normalized path below the repository root."""
    if not isinstance(value, str) or not value.strip():
        return False
    normalized = value.replace("\\", "/")
    path = PurePosixPath(normalized)
    return not path.is_absolute() and ":" not in normalized and ".." not in path.parts


def review_plan(plan: Any, role_policy: dict[str, Any] | None = None) -> dict[str, Any]:
    findings: list[Finding] = []
    if not isinstance(plan, dict):
        finding = Finding("error", "plan_type", "Workflow plan must be a JSON object")
        return {
            "ok": False,
            "errors": 1,
            "warnings": 0,
            "findings": [finding.to_dict()],
            "task_count": 0,
            "routes": {},
        }
    role_policy = role_policy or {}
    review_policy = plan.get("review_policy", {})
    budget_policy = plan.get("budget_policy", {})
    premium_allowed = bool(review_policy.get("premium_allowed"))
    tasks = iter_tasks(plan)

    if plan.get("schema_version") != 1:
        findings.append(Finding("error", "schema_version", "workflow_plan schema_version must be 1"))
    if not plan.get("goal"):
        findings.append(Finding("error", "missing_goal", "Plan must include a goal"))
    if not tasks:
        findings.append(Finding("error", "no_tasks", "Plan must include at least one task"))

    try:
        max_total_workers = int(budget_policy.get("max_total_workers", len(tasks) or 0))
    except (TypeError, ValueError):
        max_total_workers = 0
        findings.append(Finding("error", "worker_budget", "max_total_workers must be an integer"))
    if len(tasks) > max_total_workers:
        findings.append(
            Finding(
                "error", "worker_budget", f"Plan has {len(tasks)} tasks above max_total_workers={max_total_workers}"
            )
        )

    provider_concurrency = budget_policy.get("provider_concurrency", {})
    if not isinstance(provider_concurrency, dict):
        provider_concurrency = {}
        findings.append(Finding("error", "provider_capacity", "provider_concurrency must be an object"))
    task_ids: set[str] = set()
    task_routes: dict[str, str] = {}
    for stage_name, task in tasks:
        task_id = task.get("id")
        if not isinstance(task_id, str) or not task_id.strip():
            findings.append(Finding("error", "missing_task_id", f"Task in stage {stage_name!r} is missing id"))
            continue
        if task_id in task_ids:
            findings.append(Finding("error", "duplicate_task_id", f"Duplicate task id {task_id!r}", task_id))
        task_ids.add(task_id)
        role = task.get("role", "general")
        route = task.get("route", "mock")
        if not isinstance(route, str) or not route:
            findings.append(Finding("error", "invalid_route", f"Invalid route {route!r}", task_id))
            route = ""
        task_routes[task_id] = route

        if not isinstance(role, str) or role not in VALID_ROLES:
            findings.append(Finding("error", "invalid_role", f"Invalid role {role!r}", task_id))
        tools_policy = task.get("tools_policy", "none")
        if not isinstance(tools_policy, str) or tools_policy not in VALID_TOOLS_POLICIES:
            findings.append(Finding("error", "invalid_tools_policy", f"Invalid tools_policy {tools_policy!r}", task_id))
        if is_premium_route(route) and not premium_allowed:
            findings.append(
                Finding(
                    "error",
                    "premium_route_blocked",
                    f"Premium route {route!r} requires explicit premium_allowed=true",
                    task_id,
                )
            )
        if provider_concurrency:
            try:
                route_capacity = int(provider_concurrency.get(route, 0))
            except (TypeError, ValueError):
                route_capacity = 0
            if route_capacity <= 0:
                findings.append(
                    Finding("error", "provider_capacity", f"Route {route!r} has zero provider_concurrency", task_id)
                )
        task_text = task.get("task")
        if not isinstance(task_text, str) or not task_text.strip():
            findings.append(Finding("error", "missing_task_text", "Task must include task text", task_id))
        artifacts = task.get("artifacts", [])
        if not isinstance(artifacts, list):
            artifacts = []
            findings.append(Finding("error", "invalid_artifacts", "Task artifacts must be a list", task_id))
        if not artifacts and role in {"programmer", "verifier", "backend", "qa"}:
            findings.append(
                Finding(
                    "warning",
                    "missing_artifacts",
                    "Implementation/review task should declare expected artifacts",
                    task_id,
                )
            )
        for artifact in artifacts:
            normalized = str(artifact).replace("\\", "/")
            if not is_safe_repo_path(artifact):
                findings.append(
                    Finding("error", "unsafe_artifact_path", f"Artifact path is not repo-relative: {artifact}", task_id)
                )
            if plan.get("benchmark_family") or any(str(a).startswith(ALLOWED_BENCHMARK_PREFIXES) for a in artifacts):
                if not normalized.startswith(ALLOWED_BENCHMARK_PREFIXES):
                    findings.append(
                        Finding(
                            "error", "benchmark_scope", f"Benchmark artifact outside allowed paths: {artifact}", task_id
                        )
                    )
        if role == "verifier" and not task.get("verify"):
            findings.append(
                Finding(
                    "warning",
                    "missing_verification",
                    "Verifier task should include at least one deterministic verify command",
                    task_id,
                )
            )

    for _, task in tasks:
        task_id = task.get("id")
        parent_task_id = task.get("parent_task_id", task.get("parent_id"))
        if parent_task_id is not None and (not isinstance(parent_task_id, str) or parent_task_id not in task_ids):
            findings.append(
                Finding(
                    "error",
                    "missing_parent_task",
                    f"Parent task {parent_task_id!r} does not match any task id",
                    task_id,
                )
            )
        elif parent_task_id == task_id:
            findings.append(Finding("error", "self_parent_task", "Task cannot be its own parent", task_id))
        needs = task.get("needs", [])
        if not isinstance(needs, list):
            findings.append(Finding("error", "invalid_dependencies", "Task needs must be a list", task_id))
            continue
        for dep in needs:
            if not isinstance(dep, str) or dep not in task_ids:
                findings.append(
                    Finding("error", "missing_dependency", f"Dependency {dep!r} does not match any task id", task_id)
                )

    if len(tasks) > int(role_policy.get("review_policy", {}).get("require_critic_review_when_task_count_gt", 10_000)):
        if not review_policy.get("critic_review_required"):
            findings.append(Finding("warning", "critic_review_recommended", "Large plan should require critic review"))

    errors = [finding for finding in findings if finding.severity == "error"]
    warnings = [finding for finding in findings if finding.severity == "warning"]
    return {
        "ok": not errors,
        "errors": len(errors),
        "warnings": len(warnings),
        "findings": [finding.to_dict() for finding in findings],
        "task_count": len(tasks),
        "routes": task_routes,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Static review a SWARMS workflow plan.")
    parser.add_argument("plan", type=Path)
    parser.add_argument("--role-policy", type=Path, default=DEFAULT_ROLE_POLICY)
    args = parser.parse_args()
    plan = load_json(args.plan)
    role_policy = load_json(args.role_policy) if args.role_policy.exists() else {}
    result = review_plan(plan, role_policy)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
