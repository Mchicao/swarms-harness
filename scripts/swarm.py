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
import subprocess
import sys
from pathlib import Path
from typing import Any

try:
    from .plan_review import DEFAULT_ROLE_POLICY, load_json, review_plan
    from .workflow_runtime import DEFAULT_RUNS_DIR, WorkflowRuntime, parse_provider_caps
except ImportError:  # pragma: no cover - direct script execution path.
    from plan_review import DEFAULT_ROLE_POLICY, load_json, review_plan
    from workflow_runtime import DEFAULT_RUNS_DIR, WorkflowRuntime, parse_provider_caps

PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PLAN = PROJECT_ROOT / "docs" / "workflow_plan_example.json"


def print_json(data: Any) -> None:
    print(json.dumps(data, indent=2, sort_keys=True))


def command_review(args: argparse.Namespace) -> int:
    plan = load_json(args.plan)
    role_policy = load_json(args.role_policy) if args.role_policy.exists() else {}
    result = review_plan(plan, role_policy)
    print_json(result)
    return 0 if result["ok"] else 1


def build_runtime(args: argparse.Namespace) -> WorkflowRuntime:
    return WorkflowRuntime(
        workflow_plan=args.plan,
        run_id=args.run_id,
        max_total_workers=args.max_total_workers,
        global_max_concurrency=args.global_max_concurrency,
        provider_max_concurrency=parse_provider_caps(args.provider_cap),
        run_root=args.run_root,
    )


def review_or_stop(args: argparse.Namespace) -> int:
    plan = load_json(args.plan)
    role_policy = load_json(args.role_policy) if args.role_policy.exists() else {}
    result = review_plan(plan, role_policy)
    if not result["ok"]:
        print_json(result)
        return 1
    return 0


def command_dry_run(args: argparse.Namespace) -> int:
    review_code = review_or_stop(args)
    if review_code != 0:
        return review_code
    report = build_runtime(args).run(dry_run=True, force=args.force)
    print_json(report)
    return 0 if report["status"] == "planned" else 1


def command_run(args: argparse.Namespace) -> int:
    review_code = review_or_stop(args)
    if review_code != 0:
        return review_code
    report = build_runtime(args).run(dry_run=False, force=args.force)
    print_json(report)
    return 0 if report["status"] == "completed" else 1


def command_doctor(args: argparse.Namespace) -> int:
    result = subprocess.run([sys.executable, "scripts/doctor.py"], cwd=PROJECT_ROOT)
    return result.returncode


def add_runtime_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--plan", type=Path, default=DEFAULT_PLAN, help="Structured workflow plan JSON")
    parser.add_argument("--role-policy", type=Path, default=DEFAULT_ROLE_POLICY)
    parser.add_argument("--run-id")
    parser.add_argument("--run-root", type=Path, default=DEFAULT_RUNS_DIR)
    parser.add_argument("--force", action="store_true", help="Overwrite an existing run directory with the same run id")
    parser.add_argument("--max-total-workers", type=int, default=1000)
    parser.add_argument("--global-max-concurrency", type=int, default=8)
    parser.add_argument(
        "--provider-cap", action="append", default=[], help="Provider cap as provider=count, e.g. mock=3"
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

    doctor = subparsers.add_parser("doctor", help="Run offline health checks")
    doctor.set_defaults(func=command_doctor)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
