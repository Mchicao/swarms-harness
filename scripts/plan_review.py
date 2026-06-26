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
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ROLE_POLICY = PROJECT_ROOT / "config" / "role_policy.json"
PREMIUM_ROUTES = {"codex", "claude", "opus", "gpt55", "gpt-5.5"}
ALLOWED_BENCHMARK_PREFIXES = ("bench_apps/", "bench_tests/", "docs/bench_notes/")
VALID_ROLES = {"planner", "critic", "programmer", "verifier", "docs", "backend", "qa", "debug", "general"}


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
    for stage in plan.get("stages", []):
        stage_name = stage.get("name", "Unnamed")
        for task in stage.get("tasks", []):
            tasks.append((stage_name, task))
    return tasks


def is_premium_route(route: str | None) -> bool:
    if not route:
        return False
    lowered = route.lower()
    return lowered in PREMIUM_ROUTES or any(marker in lowered for marker in PREMIUM_ROUTES)


def review_plan(plan: dict[str, Any], role_policy: dict[str, Any] | None = None) -> dict[str, Any]:
    findings: list[Finding] = []
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

    max_total_workers = int(budget_policy.get("max_total_workers", len(tasks) or 0))
    if len(tasks) > max_total_workers:
        findings.append(
            Finding(
                "error", "worker_budget", f"Plan has {len(tasks)} tasks above max_total_workers={max_total_workers}"
            )
        )

    provider_concurrency = budget_policy.get("provider_concurrency", {})
    task_ids: set[str] = set()
    task_routes: dict[str, str] = {}
    for stage_name, task in tasks:
        task_id = task.get("id")
        if not task_id:
            findings.append(Finding("error", "missing_task_id", f"Task in stage {stage_name!r} is missing id"))
            continue
        if task_id in task_ids:
            findings.append(Finding("error", "duplicate_task_id", f"Duplicate task id {task_id!r}", task_id))
        task_ids.add(task_id)
        role = task.get("role", "general")
        route = task.get("route", "mock")
        task_routes[task_id] = route

        if role not in VALID_ROLES:
            findings.append(Finding("error", "invalid_role", f"Invalid role {role!r}", task_id))
        if is_premium_route(route) and not premium_allowed:
            findings.append(
                Finding(
                    "error",
                    "premium_route_blocked",
                    f"Premium route {route!r} requires explicit premium_allowed=true",
                    task_id,
                )
            )
        if provider_concurrency and int(provider_concurrency.get(route, 0)) <= 0:
            findings.append(
                Finding("error", "provider_capacity", f"Route {route!r} has zero provider_concurrency", task_id)
            )
        if not task.get("task"):
            findings.append(Finding("error", "missing_task_text", "Task must include task text", task_id))
        artifacts = task.get("artifacts", [])
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
            if normalized.startswith("../") or normalized.startswith("/") or ":" in normalized:
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
        for dep in task.get("needs", []):
            if dep not in task_ids:
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
