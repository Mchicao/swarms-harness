#!/usr/bin/env python3
"""Single public CLI for SWARMS.

This is the only flow humans and agents should use directly:

1. review a structured plan;
2. dry-run the deterministic runtime;
3. run the approved plan with provider caps.

Legacy scripts remain available as internal adapters, but this CLI is the
stable entrypoint for the open-source package.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

try:
    from .agent_preflight import discover_agents, format_text, route_findings
    from .context_sync import DEFAULT_TARGETS, ContextSyncError, sync_agent_context
    from .doctor import main as doctor_main
    from .paths import PROJECT_ROOT
    from .plan_review import DEFAULT_ROLE_POLICY, load_json, review_plan
    from .smart_router import load_config
    from .workflow_ir import compile_plan
    from .workflow_runtime import DEFAULT_RUNS_DIR, WORKER_SCRIPTS, WorkflowRuntime, parse_provider_caps
except ImportError:  # pragma: no cover - direct script execution path.
    from agent_preflight import discover_agents, format_text, route_findings
    from context_sync import DEFAULT_TARGETS, ContextSyncError, sync_agent_context
    from doctor import main as doctor_main
    from paths import PROJECT_ROOT
    from plan_review import DEFAULT_ROLE_POLICY, load_json, review_plan
    from smart_router import load_config
    from workflow_ir import compile_plan
    from workflow_runtime import DEFAULT_RUNS_DIR, WORKER_SCRIPTS, WorkflowRuntime, parse_provider_caps

DEFAULT_PLAN = PROJECT_ROOT / "docs" / "workflow_plan_example.json"


def print_json(data: Any) -> None:
    print(json.dumps(data, indent=2, sort_keys=True))


def plan_routes(plan: dict[str, Any]) -> set[str]:
    plan = compile_plan(plan)
    return {str(task.get("route", "mock")) for stage in plan.get("stages", []) for task in stage.get("tasks", [])}


def sync_context_or_stop(args: argparse.Namespace) -> tuple[int, dict[str, Any] | None]:
    """SWARMS-CONTEXT-001: Run explicit noninteractive context synchronization."""
    if not args.sync_agent_context:
        return 0, None
    targets = [target.strip() for target in args.context_sync_targets.split(",") if target.strip()]
    try:
        return 0, sync_agent_context(args.workspace_root, targets)
    except ContextSyncError as exc:
        print_json({"ok": False, "errors": 1, "findings": [{"code": "context_sync_failed", "message": str(exc)}]})
        return 1, None


def command_preflight(args: argparse.Namespace) -> int:
    report = discover_agents(args.router_config)
    if args.format == "json":
        print_json(report)
    else:
        print(format_text(report))
    return 0


def command_review(args: argparse.Namespace) -> int:
    plan = load_json(args.plan)
    role_policy = load_json(args.role_policy) if args.role_policy.exists() else {}
    result = review_plan(plan, role_policy)
    print_json(result)
    return 0 if result["ok"] else 1


def build_runtime(args: argparse.Namespace) -> WorkflowRuntime:
    plan = load_json(args.plan)
    budget = plan.get("budget_policy", {})
    plan_caps = {
        str(route): int(count) for route, count in budget.get("provider_concurrency", {}).items() if int(count) > 0
    }
    caps = {**plan_caps, **parse_provider_caps(args.provider_cap)}
    return WorkflowRuntime(
        workflow_plan=args.plan,
        run_id=args.run_id,
        max_total_workers=min(args.max_total_workers, int(budget.get("max_total_workers", args.max_total_workers))),
        global_max_concurrency=min(
            args.global_max_concurrency,
            int(budget.get("global_max_concurrency", args.global_max_concurrency)),
        ),
        provider_max_concurrency=caps,
        run_root=args.run_root,
        router_config=args.router_config,
        workspace_root=args.workspace_root,
    )


def review_or_stop(args: argparse.Namespace) -> int:
    plan = load_json(args.plan)
    role_policy = load_json(args.role_policy) if args.role_policy.exists() else {}
    result = review_plan(plan, role_policy)
    if not result["ok"]:
        print_json(result)
        return 1
    return 0


def enabled_routes_or_stop(args: argparse.Namespace) -> int:
    """Refuse real execution unless every route is enabled and supported."""
    plan = compile_plan(load_json(args.plan))
    preflight = discover_agents(args.router_config)
    findings = route_findings(preflight, plan_routes(plan))
    if findings and not args.allow_unverified_agents:
        print_json({"ok": False, "errors": len(findings), "findings": findings, "preflight": preflight})
        return 1
    providers = load_config(args.router_config).get("providers", {})
    findings = []
    for stage in plan.get("stages", []):
        for task in stage.get("tasks", []):
            route = task.get("route", "mock")
            provider = providers.get(route)
            if not provider:
                findings.append({"code": "unknown_route", "route": route, "task_id": task.get("id")})
            elif not provider.get("enabled", False):
                findings.append({"code": "route_disabled", "route": route, "task_id": task.get("id")})
            elif provider.get("wrapper") not in WORKER_SCRIPTS:
                findings.append({"code": "unsupported_wrapper", "route": route, "task_id": task.get("id")})
    if findings:
        print_json({"ok": False, "errors": len(findings), "findings": findings, "preflight": preflight})
        return 1
    return 0


def command_dry_run(args: argparse.Namespace) -> int:
    review_code = review_or_stop(args)
    if review_code != 0:
        return review_code
    sync_code, sync_report = sync_context_or_stop(args)
    if sync_code != 0:
        return sync_code
    report = build_runtime(args).run(dry_run=True, force=args.force, resume=args.resume)
    if sync_report:
        report["context_sync"] = sync_report
    print_json(report)
    return 0 if report["status"] == "planned" else 1


def command_run(args: argparse.Namespace) -> int:
    # SWARMS-PREFLIGHT-004: descubre agentes antes de revisar o crear cualquier run.
    route_code = enabled_routes_or_stop(args)
    if route_code != 0:
        return route_code
    review_code = review_or_stop(args)
    if review_code != 0:
        return review_code
    sync_code, sync_report = sync_context_or_stop(args)
    if sync_code != 0:
        return sync_code
    report = build_runtime(args).run(dry_run=False, force=args.force, resume=args.resume)
    if sync_report:
        report["context_sync"] = sync_report
    print_json(report)
    return 0 if report["status"] == "completed" else 1


def command_doctor(args: argparse.Namespace) -> int:
    del args
    return doctor_main()


def add_runtime_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--plan", type=Path, default=DEFAULT_PLAN, help="Structured workflow plan JSON")
    parser.add_argument("--role-policy", type=Path, default=DEFAULT_ROLE_POLICY)
    parser.add_argument(
        "--router-config",
        type=Path,
        help="Router config used to authorize real routes (defaults to local config when present)",
    )
    parser.add_argument("--run-id")
    parser.add_argument("--run-root", type=Path, default=DEFAULT_RUNS_DIR)
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path.cwd(),
        help="Repository where full-tools workers read and write",
    )
    restart = parser.add_mutually_exclusive_group()
    restart.add_argument(
        "--force", action="store_true", help="Overwrite an existing run directory with the same run id"
    )
    restart.add_argument(
        "--resume", action="store_true", help="Reuse completed task checkpoints from an existing run id"
    )
    parser.add_argument("--max-total-workers", type=int, default=1000)
    parser.add_argument("--global-max-concurrency", type=int, default=8)
    parser.add_argument("--provider-cap", action="append", default=[], help="Route cap as route=count, e.g. mock=3")
    parser.add_argument(
        "--allow-unverified-agents",
        action="store_true",
        help="Explicitly bypass preflight for real routes after an external probe.",
    )
    parser.add_argument(
        "--sync-agent-context",
        action="store_true",
        help="Synchronize skills, rules, AGENTS files, subagents, and MCP before dispatch",
    )
    parser.add_argument(
        "--context-sync-targets",
        default=",".join(DEFAULT_TARGETS),
        help="Comma-separated Rulesync targets used with --sync-agent-context",
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="SWARMS quota-saving workflow CLI")
    subparsers = parser.add_subparsers(dest="command", required=True)

    review = subparsers.add_parser("review", help="Static-review a workflow plan")
    review.add_argument("--plan", type=Path, default=DEFAULT_PLAN)
    review.add_argument("--role-policy", type=Path, default=DEFAULT_ROLE_POLICY)
    review.set_defaults(func=command_review)

    dry_run = subparsers.add_parser("dry-run", help="Review and plan a workflow without running workers")
    add_runtime_args(dry_run)
    dry_run.set_defaults(func=command_dry_run)

    run = subparsers.add_parser("run", help="Review and run a workflow")
    add_runtime_args(run)
    run.set_defaults(func=command_run)

    preflight = subparsers.add_parser("preflight", help="Inventory local agent CLIs and configured routes")
    preflight.add_argument("--router-config", type=Path)
    preflight.add_argument("--format", choices=("text", "json"), default="text")
    preflight.set_defaults(func=command_preflight)

    doctor = subparsers.add_parser("doctor", help="Run offline health checks")
    doctor.set_defaults(func=command_doctor)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
