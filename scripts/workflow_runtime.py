#!/usr/bin/env python3
"""Deterministic dynamic-workflow runtime for SWARMS.

This is the scalable orchestration layer: it keeps the plan, task state,
claims, worker summaries, and telemetry on disk so the model orchestrator does
not need to hold intermediate worker noise in its context.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import threading
import time
import uuid
from concurrent.futures import FIRST_COMPLETED, Future, ThreadPoolExecutor, wait
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_TASKS_FILE = PROJECT_ROOT / "docs" / "agentic_swarm_micro_tasks.json"
DEFAULT_RUNS_DIR = PROJECT_ROOT / ".agent" / "swarm" / "runs"
ROUTE_PROVIDER_MAP = {
    "mock": ("mock", "mock-worker", "mock"),
    "glm52": ("opencode", "glm-5.2", "opencode"),
    "gemini_flash": ("antigravity_cli", "gemini-3.5-flash", "gemini"),
    "codex": ("codex_cli", "gpt-5.5-codex", "codex"),
    "local_tests": ("local", "local-tests", "shell"),
}

ROLE_RE = re.compile(r"^\[(?P<role>[A-Za-z0-9_-]+)\]\s*(?P<body>.*)$")
NEEDS_RE = re.compile(r"@needs\((?P<deps>[^)]+)\)")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def slugify(value: str) -> str:
    clean = re.sub(r"[^A-Za-z0-9_.-]+", "-", value.strip()).strip("-")
    return clean[:80] or "task"


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json_atomic(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(f"{path.name}.{uuid.uuid4().hex}.tmp")
    tmp.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    tmp.replace(path)


def parse_role(task_text: str) -> tuple[str, str]:
    match = ROLE_RE.match(task_text.strip())
    if not match:
        return "general", task_text.strip()
    return match.group("role").lower(), match.group("body").strip()


def parse_needs(task_text: str) -> list[str]:
    match = NEEDS_RE.search(task_text)
    if not match:
        return []
    return [dep.strip() for dep in match.group("deps").split(",") if dep.strip()]


def task_matches_dependency(task: WorkflowTask, dependency: str) -> bool:
    if dependency == task.task_id:
        return True
    haystack = f"{task.task_id} {task.text} {' '.join(task.artifacts)}".lower()
    return dependency.lower() in haystack


@dataclass
class ProviderPool:
    provider: str
    model: str
    wrapper: str
    max_concurrency: int


@dataclass
class WorkflowTask:
    task_id: str
    stage: str
    index: int
    text: str
    role: str
    needs: list[str] = field(default_factory=list)
    provider: str = "mock"
    model: str = "mock-worker"
    wrapper: str = "mock"
    status: str = "pending"
    attempts: int = 0
    artifacts: list[str] = field(default_factory=list)
    error: str | None = None
    started_at: str | None = None
    ended_at: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "task_id": self.task_id,
            "stage": self.stage,
            "index": self.index,
            "text": self.text,
            "role": self.role,
            "needs": self.needs,
            "provider": self.provider,
            "model": self.model,
            "wrapper": self.wrapper,
            "status": self.status,
            "attempts": self.attempts,
            "artifacts": self.artifacts,
            "error": self.error,
            "started_at": self.started_at,
            "ended_at": self.ended_at,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> WorkflowTask:
        return cls(**data)


class ClaimStore:
    """Small file-lock claim store.

    Atomic create is enough for local worker coordination and portable across
    Windows/macOS/Linux. A heartbeat makes stale claims recoverable.
    """

    def __init__(self, claims_dir: Path, stale_seconds: int = 900):
        self.claims_dir = claims_dir
        self.stale_seconds = stale_seconds
        self.claims_dir.mkdir(parents=True, exist_ok=True)

    def claim_path(self, task_id: str) -> Path:
        return self.claims_dir / f"{task_id}.lock"

    def try_claim(self, task_id: str, owner: str) -> bool:
        path = self.claim_path(task_id)
        if path.exists() and time.time() - path.stat().st_mtime > self.stale_seconds:
            path.unlink(missing_ok=True)
        payload = {"task_id": task_id, "owner": owner, "claimed_at": utc_now(), "heartbeat_at": utc_now()}
        try:
            fd = os.open(str(path), os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            return False
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, indent=2)
        return True

    def heartbeat(self, task_id: str, owner: str) -> None:
        path = self.claim_path(task_id)
        if not path.exists():
            return
        data = read_json(path)
        if data.get("owner") != owner:
            return
        data["heartbeat_at"] = utc_now()
        write_json_atomic(path, data)

    def release(self, task_id: str, owner: str) -> None:
        path = self.claim_path(task_id)
        if not path.exists():
            return
        try:
            data = read_json(path)
            if data.get("owner") == owner:
                path.unlink(missing_ok=True)
        except json.JSONDecodeError:
            path.unlink(missing_ok=True)


class WorkflowRuntime:
    def __init__(
        self,
        tasks_file: Path = DEFAULT_TASKS_FILE,
        workflow_plan: Path | None = None,
        run_id: str | None = None,
        max_total_workers: int = 1000,
        global_max_concurrency: int = 16,
        provider_max_concurrency: dict[str, int] | None = None,
        run_root: Path = DEFAULT_RUNS_DIR,
    ):
        self.tasks_file = tasks_file
        self.workflow_plan = workflow_plan
        self.run_id = run_id or str(uuid.uuid4())
        self.run_dir = run_root / self.run_id
        self.tasks_dir = self.run_dir / "tasks"
        self.results_dir = self.run_dir / "results"
        self.claim_store = ClaimStore(self.run_dir / "claims")
        self.max_total_workers = max_total_workers
        self.global_max_concurrency = global_max_concurrency
        self.provider_max_concurrency = provider_max_concurrency or {"mock": 64}
        self.provider_active: dict[str, int] = {provider: 0 for provider in self.provider_max_concurrency}
        self.events: list[dict[str, Any]] = []
        self.state_lock = threading.RLock()

    def event(self, event_type: str, **payload: Any) -> None:
        item = {"time": utc_now(), "event": event_type, **payload}
        self.events.append(item)
        self.run_dir.mkdir(parents=True, exist_ok=True)
        with (self.run_dir / "events.jsonl").open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(item, sort_keys=True) + "\n")

    def build_tasks(self, instance_id: str | None = None) -> list[WorkflowTask]:
        if self.workflow_plan:
            return self.build_tasks_from_plan(self.workflow_plan)
        benchmarks = read_json(self.tasks_file)
        if instance_id:
            benchmarks = [task for task in benchmarks if task["instance_id"] == instance_id]
        if not benchmarks:
            raise ValueError(f"No benchmark task matched {instance_id!r}")
        benchmark = benchmarks[0]
        tasks: list[WorkflowTask] = []
        index = 0
        for stage in benchmark["stages"]:
            for raw_text in stage["tasks"]:
                role, _ = parse_role(raw_text)
                task_id = f"{index:04d}-{slugify(stage['name'])}-{slugify(raw_text)}"
                tasks.append(
                    WorkflowTask(
                        task_id=task_id,
                        stage=stage["name"],
                        index=index,
                        text=raw_text,
                        role=role,
                        needs=parse_needs(raw_text),
                        artifacts=self._expected_artifacts(raw_text),
                    )
                )
                index += 1
        if len(tasks) > self.max_total_workers:
            raise ValueError(f"Workflow has {len(tasks)} tasks, above max_total_workers={self.max_total_workers}")
        return tasks

    def build_tasks_from_plan(self, plan_path: Path) -> list[WorkflowTask]:
        plan = read_json(plan_path)
        tasks: list[WorkflowTask] = []
        index = 0
        for stage in plan.get("stages", []):
            for spec in stage.get("tasks", []):
                route = spec.get("route", "mock")
                provider, model, wrapper = ROUTE_PROVIDER_MAP.get(route, (route, route, route))
                task_text = f"[{spec.get('role', 'general')}] {spec.get('task', '')}"
                tasks.append(
                    WorkflowTask(
                        task_id=f"{index:04d}-{slugify(spec.get('id', task_text))}",
                        stage=stage.get("name", "Unnamed"),
                        index=index,
                        text=task_text,
                        role=spec.get("role", "general"),
                        needs=list(spec.get("needs", [])),
                        provider=provider,
                        model=model,
                        wrapper=wrapper,
                        artifacts=list(spec.get("artifacts", [])),
                    )
                )
                index += 1
        if len(tasks) > self.max_total_workers:
            raise ValueError(f"Workflow has {len(tasks)} tasks, above max_total_workers={self.max_total_workers}")
        return tasks

    def _expected_artifacts(self, text: str) -> list[str]:
        artifacts = []
        for token in re.findall(r"[\w./-]+\.(?:py|md|json|toml|yaml|yml)", text):
            if token.startswith(("bench_apps/", "bench_tests/", "docs/bench_notes/")):
                artifacts.append(token)
        return artifacts

    def initialize(self, instance_id: str | None = None, force: bool = False) -> list[WorkflowTask]:
        if self.run_dir.exists() and not force:
            return self.load_tasks()
        self.run_dir.mkdir(parents=True, exist_ok=True)
        tasks = self.build_tasks(instance_id)
        for task in tasks:
            self.save_task(task)
        write_json_atomic(
            self.run_dir / "workflow.json",
            {
                "run_id": self.run_id,
                "created_at": utc_now(),
                "tasks_file": str(self.tasks_file),
                "workflow_plan": str(self.workflow_plan) if self.workflow_plan else None,
                "global_max_concurrency": self.global_max_concurrency,
                "provider_max_concurrency": self.provider_max_concurrency,
                "max_total_workers": self.max_total_workers,
                "task_count": len(tasks),
            },
        )
        self.event("workflow_initialized", task_count=len(tasks))
        return tasks

    def save_task(self, task: WorkflowTask) -> None:
        with self.state_lock:
            write_json_atomic(self.tasks_dir / f"{task.task_id}.json", task.to_dict())

    def load_tasks(self) -> list[WorkflowTask]:
        with self.state_lock:
            return [WorkflowTask.from_dict(read_json(path)) for path in sorted(self.tasks_dir.glob("*.json"))]

    def dependency_satisfied(self, task: WorkflowTask, tasks: list[WorkflowTask]) -> bool:
        for dep in task.needs:
            matches = [
                candidate
                for candidate in tasks
                if candidate.task_id != task.task_id and task_matches_dependency(candidate, dep)
            ]
            if not matches or not any(candidate.status == "completed" for candidate in matches):
                return False
        return True

    def ready_tasks(self, tasks: list[WorkflowTask]) -> list[WorkflowTask]:
        ready = [
            task
            for task in tasks
            if task.status == "pending"
            and self.dependency_satisfied(task, tasks)
            and self.provider_active.get(task.provider, 0) < self.provider_max_concurrency.get(task.provider, 0)
        ]
        return sorted(ready, key=lambda task: (task.index, task.task_id))

    def write_prompt(self, task: WorkflowTask, work_dir: Path) -> Path:
        prompt = work_dir / f"{task.task_id}.prompt.txt"
        prompt.write_text(
            "\n".join(
                [
                    "You are a SWARMS worker with a narrow task.",
                    f"Role: {task.role}",
                    f"Task: {task.text}",
                    f"Allowed artifacts: {', '.join(task.artifacts) or '(task-defined)'}",
                    "Return only the required artifact changes and keep output concise.",
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        return prompt

    def run_mock_task(self, task: WorkflowTask) -> dict[str, Any]:
        owner = f"mock-{task.task_id}-{uuid.uuid4().hex[:8]}"
        if not self.claim_store.try_claim(task.task_id, owner):
            return {"success": False, "error": "task already claimed"}
        task.status = "in_progress"
        task.attempts += 1
        task.started_at = utc_now()
        self.save_task(task)
        self.event("task_started", task_id=task.task_id, provider=task.provider, model=task.model)
        work_dir = self.results_dir / task.task_id
        work_dir.mkdir(parents=True, exist_ok=True)
        prompt = self.write_prompt(task, work_dir)
        output_log = work_dir / "worker.log"
        command = [sys.executable, str(PROJECT_ROOT / "scripts" / "mock_worker.py"), "--prompt", str(prompt)]
        started = time.time()
        proc = subprocess.run(command, cwd=work_dir, text=True, capture_output=True, timeout=120)
        output_log.write_text((proc.stdout or "") + (proc.stderr or ""), encoding="utf-8")
        success = proc.returncode == 0
        task.status = "completed" if success else "failed"
        task.error = None if success else output_log.read_text(encoding="utf-8", errors="replace")[-2000:]
        task.ended_at = utc_now()
        self.save_task(task)
        self.claim_store.release(task.task_id, owner)
        result = {
            "task_id": task.task_id,
            "success": success,
            "returncode": proc.returncode,
            "duration_seconds": round(time.time() - started, 3),
            "provider": task.provider,
            "model": task.model,
            "output_log": str(output_log),
        }
        write_json_atomic(work_dir / "result.json", result)
        self.event("task_finished", **result)
        return result

    def run(self, instance_id: str | None = None, dry_run: bool = False, force: bool = False) -> dict[str, Any]:
        self.initialize(instance_id, force=force)
        if dry_run:
            tasks = self.load_tasks()
            report = self.report(tasks, status="planned")
            write_json_atomic(self.run_dir / "report.json", report)
            return report

        active: dict[Future[dict[str, Any]], WorkflowTask] = {}
        results: list[dict[str, Any]] = []
        with ThreadPoolExecutor(max_workers=self.global_max_concurrency) as executor:
            while True:
                tasks = self.load_tasks()
                if all(task.status in {"completed", "failed", "blocked"} for task in tasks) and not active:
                    break

                launched = False
                capacity = self.global_max_concurrency - len(active)
                for task in self.ready_tasks(tasks)[:capacity]:
                    task.status = "queued"
                    self.save_task(task)
                    self.provider_active[task.provider] = self.provider_active.get(task.provider, 0) + 1
                    active[executor.submit(self.run_mock_task, task)] = task
                    launched = True

                if not active and not launched:
                    for task in tasks:
                        if task.status == "pending":
                            task.status = "blocked"
                            task.error = "dependencies were not satisfied or provider pool has zero capacity"
                            self.save_task(task)
                            self.event("task_blocked", task_id=task.task_id, error=task.error)
                    continue

                done, _ = wait(active.keys(), timeout=0.2, return_when=FIRST_COMPLETED)
                for future in done:
                    task = active.pop(future)
                    self.provider_active[task.provider] = max(0, self.provider_active.get(task.provider, 0) - 1)
                    results.append(future.result())

        tasks = self.load_tasks()
        status = "completed" if all(task.status == "completed" for task in tasks) else "failed"
        report = self.report(tasks, status=status, results=results)
        write_json_atomic(self.run_dir / "report.json", report)
        self.event("workflow_finished", status=status)
        return report

    def report(
        self, tasks: list[WorkflowTask], status: str, results: list[dict[str, Any]] | None = None
    ) -> dict[str, Any]:
        counts: dict[str, int] = {}
        for task in tasks:
            counts[task.status] = counts.get(task.status, 0) + 1
        return {
            "run_id": self.run_id,
            "status": status,
            "run_dir": str(self.run_dir),
            "task_counts": counts,
            "global_max_concurrency": self.global_max_concurrency,
            "provider_max_concurrency": self.provider_max_concurrency,
            "tasks": [task.to_dict() for task in tasks],
            "results": results or [],
            "token_usage": {
                "input": 0,
                "cached": 0,
                "cache_write": 0,
                "output": 0,
                "reasoning": 0,
                "known_cost_usd": 0.0,
            },
        }


def parse_provider_caps(values: list[str]) -> dict[str, int]:
    caps: dict[str, int] = {"mock": 64}
    for value in values:
        provider, _, raw_count = value.partition("=")
        if not provider or not raw_count:
            raise ValueError(f"Provider cap must be provider=count, got {value!r}")
        caps[provider] = int(raw_count)
    return caps


def main() -> int:
    parser = argparse.ArgumentParser(description="Run a deterministic SWARMS dynamic workflow.")
    parser.add_argument("--tasks-file", type=Path, default=DEFAULT_TASKS_FILE)
    parser.add_argument("--workflow-plan", type=Path)
    parser.add_argument("--instance-id", default="micro-reshard-roundtrip")
    parser.add_argument("--run-id")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--max-total-workers", type=int, default=1000)
    parser.add_argument("--global-max-concurrency", type=int, default=16)
    parser.add_argument("--provider-cap", action="append", default=[], help="Provider cap as provider=count")
    args = parser.parse_args()

    runtime = WorkflowRuntime(
        tasks_file=args.tasks_file,
        workflow_plan=args.workflow_plan,
        run_id=args.run_id,
        max_total_workers=args.max_total_workers,
        global_max_concurrency=args.global_max_concurrency,
        provider_max_concurrency=parse_provider_caps(args.provider_cap),
    )
    report = runtime.run(instance_id=args.instance_id, dry_run=args.dry_run, force=args.force)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["status"] in {"planned", "completed"} else 1


if __name__ == "__main__":
    raise SystemExit(main())
