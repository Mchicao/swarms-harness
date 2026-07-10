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
import shutil
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

try:
    from .paths import PROJECT_ROOT, WORKSPACE_ROOT
    from .smart_router import load_config
except ImportError:  # pragma: no cover - direct script execution path.
    from paths import PROJECT_ROOT, WORKSPACE_ROOT
    from smart_router import load_config

DEFAULT_TASKS_FILE = PROJECT_ROOT / "docs" / "agentic_swarm_micro_tasks.json"
DEFAULT_RUNS_DIR = WORKSPACE_ROOT / ".agent" / "swarm" / "runs"
WORKER_SCRIPTS = {
    "mock": "mock_worker.py",
    "gemini": "gemini_worker.py",
    "opencode": "opencode_worker.py",
    "kilo": "kilo_worker.py",
    "codex": "codex_worker.py",
    "openai_compat": "openai_compat_worker.py",
    "hermes": "hermes_worker.py",
}
# For openai_compat workers: which env var holds the API key per provider.
# Base URL may also live in an env var (e.g. OPENROUTER_BASE_URL); the worker
# falls back to a sane default (openrouter.ai/api/v1) when unset.
OPENAI_COMPAT_KEY_ENV = {
    "openrouter": "OPENROUTER_API_KEY",
    "novita": "NOVITA_API_KEY",
    "gitlawb": "GITLAWB_API_KEY",
    "siliconflow": "SILICONFLOW_API_KEY",
    "kilo": "KILO_API_KEY",
}
MAX_DEPENDENCY_CONTEXT_CHARS = 12_000
DEFAULT_WORKER_TIMEOUT = int(os.environ.get("SWARMS_WORKER_TIMEOUT", "600"))
SAFE_RUN_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$")
SERIAL_PROVIDERS = {"antigravity_cli"}

ROLE_RE = re.compile(r"^\[(?P<role>[A-Za-z0-9_-]+)\]\s*(?P<body>.*)$")
NEEDS_RE = re.compile(r"@needs\((?P<deps>[^)]+)\)")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def slugify(value: str) -> str:
    clean = re.sub(r"[^A-Za-z0-9_.-]+", "-", value.strip()).strip("-")
    return clean[:80] or "task"


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def read_text_tail(path: Path, max_bytes: int = 8_000) -> str:
    with path.open("rb") as handle:
        handle.seek(0, os.SEEK_END)
        handle.seek(max(0, handle.tell() - max_bytes))
        return handle.read().decode("utf-8", errors="replace")


def write_json_atomic(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(data, indent=2, sort_keys=True) + "\n"
    for attempt in range(5):
        tmp = path.with_name(f"{path.name}.{uuid.uuid4().hex}.tmp")
        try:
            tmp.write_text(payload, encoding="utf-8")
            tmp.replace(path)
            return
        except PermissionError:
            tmp.unlink(missing_ok=True)
            if attempt == 4:
                raise
            time.sleep(0.05 * (attempt + 1))


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
    if dependency in {task.task_id, task.source_id}:
        return True
    if task.source_id is not None:
        return False
    key = slugify(dependency).lower()
    _, separator, task_suffix = task.task_id.partition("-")
    if separator and key == task_suffix.lower():
        return True
    artifact_keys = {
        candidate
        for artifact in task.artifacts
        for candidate in (slugify(Path(artifact).name).lower(), slugify(Path(artifact).stem).lower())
    }
    return key in artifact_keys


@dataclass
class WorkflowTask:
    task_id: str
    stage: str
    index: int
    text: str
    role: str
    needs: list[str] = field(default_factory=list)
    source_id: str | None = None
    route: str = "mock"
    provider: str = "mock"
    model: str = "mock-worker"
    wrapper: str = "mock"
    tools_policy: str = "none"
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
            "source_id": self.source_id,
            "route": self.route,
            "provider": self.provider,
            "model": self.model,
            "wrapper": self.wrapper,
            "tools_policy": self.tools_policy,
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
        router_config: Path | None = None,
    ):
        self.tasks_file = tasks_file
        self.workflow_plan = workflow_plan
        self.router_config = load_config(router_config)
        self.run_id = run_id or str(uuid.uuid4())
        if not SAFE_RUN_ID_RE.fullmatch(self.run_id):
            raise ValueError(f"Unsafe run_id {self.run_id!r}; use letters, numbers, dot, underscore, or dash")
        self.run_root = run_root.resolve()
        self.run_dir = (self.run_root / self.run_id).resolve()
        if self.run_dir.parent != self.run_root:
            raise ValueError(f"run_id escapes run_root: {self.run_id!r}")
        self.tasks_dir = self.run_dir / "tasks"
        self.results_dir = self.run_dir / "results"
        self.claim_store = ClaimStore(self.run_dir / "claims")
        self.max_total_workers = max_total_workers
        if global_max_concurrency < 1 or max_total_workers < 1:
            raise ValueError("Worker and concurrency limits must be positive")
        self.global_max_concurrency = global_max_concurrency
        self.provider_max_concurrency = dict(provider_max_concurrency or {"mock": 64})
        providers = self.router_config.get("providers", {})
        unknown_caps = sorted(set(self.provider_max_concurrency) - set(providers))
        if unknown_caps:
            raise ValueError(f"Provider caps use unknown route ids: {', '.join(unknown_caps)}")
        for route in self.provider_max_concurrency:
            if providers[route].get("provider") in SERIAL_PROVIDERS:
                self.provider_max_concurrency[route] = min(1, self.provider_max_concurrency[route])
        self.route_active: dict[str, int] = {route: 0 for route in self.provider_max_concurrency}
        self.state_lock = threading.RLock()

    def event(self, event_type: str, **payload: Any) -> None:
        item = {"time": utc_now(), "event": event_type, **payload}
        with self.state_lock:
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
        providers = self.router_config.get("providers", {})
        tasks: list[WorkflowTask] = []
        index = 0
        for stage in plan.get("stages", []):
            for spec in stage.get("tasks", []):
                route = spec.get("route", "mock")
                route_config = providers.get(route)
                if not route_config:
                    raise ValueError(f"Unknown route {route!r}")
                provider = route_config.get("provider")
                model = route_config.get("model")
                wrapper = route_config.get("wrapper")
                if not all(isinstance(value, str) for value in (provider, model, wrapper)):
                    raise ValueError(f"Route {route!r} has an invalid provider definition")
                if wrapper not in WORKER_SCRIPTS:
                    raise ValueError(f"Route {route!r} uses unsupported wrapper {wrapper!r}")
                if route != "mock" and not model:
                    raise ValueError(f"Route {route!r} must pin an explicit model")
                if wrapper == "gemini" and "(" not in model:
                    model = "Gemini 3.5 Flash (Low)"
                task_text = f"[{spec.get('role', 'general')}] {spec.get('task', '')}"
                tasks.append(
                    WorkflowTask(
                        task_id=f"{index:04d}-{slugify(spec.get('id', task_text))}",
                        stage=stage.get("name", "Unnamed"),
                        index=index,
                        text=task_text,
                        role=spec.get("role", "general"),
                        needs=list(spec.get("needs", [])),
                        source_id=str(spec.get("id", "")) or None,
                        route=route,
                        provider=provider,
                        model=model,
                        wrapper=wrapper,
                        tools_policy=spec.get("tools_policy", "none"),
                        artifacts=list(spec.get("artifacts", [])),
                    )
                )
                index += 1
        if len(tasks) > self.max_total_workers:
            raise ValueError(f"Workflow has {len(tasks)} tasks, above max_total_workers={self.max_total_workers}")
        return tasks

    def ensure_routes_enabled(self, tasks: list[WorkflowTask]) -> None:
        providers = self.router_config.get("providers", {})
        disabled = sorted({task.route for task in tasks if not providers.get(task.route, {}).get("enabled", False)})
        if disabled:
            raise ValueError(f"Routes are disabled in router config: {', '.join(disabled)}")

    def _expected_artifacts(self, text: str) -> list[str]:
        artifacts = []
        for token in re.findall(r"[\w./-]+\.(?:py|md|json|toml|yaml|yml)", text):
            if token.startswith(("bench_apps/", "bench_tests/", "docs/bench_notes/")):
                artifacts.append(token)
        return artifacts

    def initialize(self, instance_id: str | None = None, force: bool = False) -> list[WorkflowTask]:
        workflow_state = self.run_dir / "workflow.json"
        if workflow_state.exists() and not force:
            tasks = self.load_tasks()
            for task in tasks:
                if task.status in {"queued", "in_progress"}:
                    task.status = "blocked"
                    task.error = "run was interrupted; restart with --force to retry safely"
                    task.ended_at = utc_now()
                    self.save_task(task)
                    self.event("task_blocked", task_id=task.task_id, error=task.error)
            return tasks
        if force and self.run_dir.exists():
            shutil.rmtree(self.run_dir)
            self.claim_store = ClaimStore(self.run_dir / "claims")
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
        ready: list[WorkflowTask] = []
        reserved = dict(self.route_active)
        for task in sorted(tasks, key=lambda item: (item.index, item.task_id)):
            if task.status != "pending" or not self.dependency_satisfied(task, tasks):
                continue
            active = reserved.get(task.route, 0)
            if active >= self.provider_max_concurrency.get(task.route, 0):
                continue
            ready.append(task)
            reserved[task.route] = active + 1
        return ready

    def dependency_outputs(self, task: WorkflowTask, tasks: list[WorkflowTask]) -> str:
        sections: list[str] = []
        remaining = MAX_DEPENDENCY_CONTEXT_CHARS
        for dependency in task.needs:
            matches = [
                candidate
                for candidate in tasks
                if candidate.task_id != task.task_id
                and candidate.status == "completed"
                and task_matches_dependency(candidate, dependency)
            ]
            for candidate in matches:
                log_path = self.results_dir / candidate.task_id / "worker.log"
                if not log_path.exists() or remaining <= 0:
                    continue
                output = log_path.read_text(encoding="utf-8", errors="replace")
                excerpt = output[-remaining:]
                sections.append(f"Dependency {candidate.task_id} output:\n{excerpt}")
                remaining -= len(excerpt)
        return "\n\n".join(sections)

    def write_prompt(self, task: WorkflowTask, work_dir: Path, tasks: list[WorkflowTask] | None = None) -> Path:
        prompt = work_dir / f"{task.task_id}.prompt.txt"
        dependency_context = self.dependency_outputs(task, tasks or [])
        lines = [
            "You are a SWARMS worker with a narrow task.",
            f"Role: {task.role}",
            f"Task: {task.text}",
            f"Allowed artifacts: {', '.join(task.artifacts) or '(task-defined)'}",
        ]
        if dependency_context:
            lines.extend(
                [
                    "Use these completed dependency outputs as input:",
                    dependency_context,
                ]
            )
        lines.append("Return only the required result and keep output concise.")
        prompt.write_text(
            "\n".join(lines) + "\n",
            encoding="utf-8",
        )
        return prompt

    def worker_command(self, task: WorkflowTask, prompt: Path, status_path: Path) -> list[str]:
        script_name = WORKER_SCRIPTS.get(task.wrapper)
        if not script_name:
            raise ValueError(f"Unsupported worker wrapper: {task.wrapper!r}")
        command = [sys.executable, "-m", f"scripts.{Path(script_name).stem}", "--prompt", str(prompt)]
        if task.wrapper != "mock":
            command.extend(
                [
                    "--status",
                    str(status_path),
                    "--model",
                    task.model,
                    "--tools-policy",
                    task.tools_policy,
                ]
            )
        if task.wrapper == "openai_compat":
            key_env = OPENAI_COMPAT_KEY_ENV.get(task.provider, "OPENAI_COMPAT_API_KEY")
            base_url_env = f"{task.provider.upper()}_BASE_URL"
            command.extend(["--key-env", key_env, "--base-url-env", base_url_env])
            if task.provider == "gitlawb":
                command.extend(["--base-url", "https://opengateway.gitlawb.com/v1"])
        # For hermes routes that carry a HY3 model, force --provider nous so
        # Hermes uses the Nous Portal free tier (tencent/hy3:free is $0 there),
        # not its paid glm-5.2 Z.AI default.
        if task.wrapper == "hermes" and task.model.startswith("tencent/hy3"):
            command.extend(["--provider", "nous"])
        return command

    def run_task(self, task: WorkflowTask, tasks: list[WorkflowTask]) -> dict[str, Any]:
        owner = f"{task.provider}-{task.task_id}-{uuid.uuid4().hex[:8]}"
        if not self.claim_store.try_claim(task.task_id, owner):
            task.status = "blocked"
            task.error = "task already claimed"
            task.ended_at = utc_now()
            self.save_task(task)
            self.event("task_blocked", task_id=task.task_id, error=task.error)
            return {"task_id": task.task_id, "success": False, "returncode": 75, "error": task.error}
        try:
            task.status = "in_progress"
            task.attempts += 1
            task.started_at = utc_now()
            self.save_task(task)
            self.event("task_started", task_id=task.task_id, provider=task.provider, model=task.model)
            work_dir = self.results_dir / task.task_id
            work_dir.mkdir(parents=True, exist_ok=True)
            prompt = self.write_prompt(task, work_dir, tasks)
            output_log = work_dir / "worker.log"
            status_path = work_dir / "status.json"
            started = time.time()
            try:
                command = self.worker_command(task, prompt, status_path)
                with output_log.open("w", encoding="utf-8") as log:
                    proc = subprocess.run(
                        command,
                        cwd=WORKSPACE_ROOT,
                        text=True,
                        stdout=log,
                        stderr=subprocess.STDOUT,
                        timeout=DEFAULT_WORKER_TIMEOUT,
                        encoding="utf-8",
                        errors="replace",
                    )
                returncode = proc.returncode
            except (OSError, ValueError) as exc:
                returncode = 127
                output_log.write_text(f"{type(exc).__name__}: {exc}\n", encoding="utf-8")
            except subprocess.TimeoutExpired:
                returncode = 124
                with output_log.open("a", encoding="utf-8") as log:
                    log.write(f"\nWorker timed out after {DEFAULT_WORKER_TIMEOUT}s\n")
            success = returncode == 0
            task.status = "completed" if success else "failed"
            task.error = None if success else read_text_tail(output_log)[-2000:]
            task.ended_at = utc_now()
            self.save_task(task)
            result = {
                "task_id": task.task_id,
                "success": success,
                "returncode": returncode,
                "duration_seconds": round(time.time() - started, 3),
                "provider": task.provider,
                "model": task.model,
                "output_log": str(output_log),
            }
            write_json_atomic(work_dir / "result.json", result)
            self.event("task_finished", **result)
            return result
        finally:
            self.claim_store.release(task.task_id, owner)

    def run(self, instance_id: str | None = None, dry_run: bool = False, force: bool = False) -> dict[str, Any]:
        tasks = self.initialize(instance_id, force=force)
        if dry_run:
            report = self.report(tasks, status="planned")
            write_json_atomic(self.run_dir / "report.json", report)
            return report
        self.ensure_routes_enabled(tasks)

        active: dict[Future[dict[str, Any]], WorkflowTask] = {}
        results: list[dict[str, Any]] = []
        with ThreadPoolExecutor(max_workers=self.global_max_concurrency) as executor:
            while True:
                if all(task.status in {"completed", "failed", "blocked"} for task in tasks) and not active:
                    break

                launched = False
                capacity = self.global_max_concurrency - len(active)
                for task in self.ready_tasks(tasks)[:capacity]:
                    task.status = "queued"
                    self.save_task(task)
                    self.route_active[task.route] = self.route_active.get(task.route, 0) + 1
                    active[executor.submit(self.run_task, task, tasks)] = task
                    launched = True

                if not active and not launched:
                    for task in tasks:
                        if task.status not in {"completed", "failed", "blocked"}:
                            task.status = "blocked"
                            task.error = "dependencies were not satisfied or provider pool has zero capacity"
                            self.save_task(task)
                            self.event("task_blocked", task_id=task.task_id, error=task.error)
                    continue

                done, _ = wait(active.keys(), return_when=FIRST_COMPLETED)
                for future in done:
                    task = active.pop(future)
                    self.route_active[task.route] = max(0, self.route_active.get(task.route, 0) - 1)
                    try:
                        results.append(future.result())
                    except Exception as exc:
                        task.status = "failed"
                        task.error = f"{type(exc).__name__}: {exc}"
                        task.ended_at = utc_now()
                        self.save_task(task)
                        result = {
                            "task_id": task.task_id,
                            "success": False,
                            "returncode": 70,
                            "error": task.error,
                            "provider": task.provider,
                            "model": task.model,
                        }
                        results.append(result)
                        self.event("task_finished", **result)

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
        has_real_provider = any(task.provider != "mock" for task in tasks)
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
                "known_cost_usd": None if has_real_provider else 0.0,
                "usage_source": "missing" if has_real_provider else "offline_mock",
            },
        }


def parse_provider_caps(values: list[str]) -> dict[str, int]:
    caps: dict[str, int] = {}
    for value in values:
        provider, _, raw_count = value.partition("=")
        if not provider or not raw_count:
            raise ValueError(f"Provider cap must be provider=count, got {value!r}")
        count = int(raw_count)
        if count < 0:
            raise ValueError(f"Provider cap cannot be negative, got {value!r}")
        caps[provider] = count
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
    parser.add_argument("--provider-cap", action="append", default=[], help="Route cap as route=count")
    parser.add_argument("--router-config", type=Path)
    args = parser.parse_args()

    runtime = WorkflowRuntime(
        tasks_file=args.tasks_file,
        workflow_plan=args.workflow_plan,
        run_id=args.run_id,
        max_total_workers=args.max_total_workers,
        global_max_concurrency=args.global_max_concurrency,
        provider_max_concurrency=parse_provider_caps(args.provider_cap),
        router_config=args.router_config,
    )
    report = runtime.run(instance_id=args.instance_id, dry_run=args.dry_run, force=args.force)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["status"] in {"planned", "completed"} else 1


if __name__ == "__main__":
    raise SystemExit(main())
