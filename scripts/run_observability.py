#!/usr/bin/env python3
"""Read-only, versioned observability contract for SWARMS runs.

Derives a stable, sanitized, read-only view of a run from existing on-disk
checkpoints (``workflow.json``, ``tasks/*.json``, ``results/*/result.json``,
``claims/*.lock``, ``events.jsonl``, ``report.json``). It never writes and has
no web/HTTP dependencies: standard library only.

The contract is a nested tree: run -> stages -> tasks -> owning agent and
nested subagents, carrying model, state, timestamps and the last heartbeat.
"""

from __future__ import annotations

import argparse
import json
import re
from collections.abc import Iterator
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

try:
    from .paths import PROJECT_ROOT, WORKSPACE_ROOT
except ImportError:  # pragma: no cover - direct script execution path.
    PROJECT_ROOT = Path(__file__).resolve().parents[1]
    WORKSPACE_ROOT = Path.cwd().resolve()

CONTRACT_SCHEMA_VERSION = 1
DEFAULT_RUNS_DIR = WORKSPACE_ROOT / ".agent" / "swarm" / "runs"

# Fields a task checkpoint is expected to carry. Unknown fields are ignored so
# the contract stays stable when the runtime adds new private task fields.
TASK_TERMINAL_STATUSES = {"completed", "failed", "blocked"}
TASK_RUNNING_STATUSES = {"in_progress", "queued"}

# Error/secret sanitization. Errors are worker log tails and may include env
# values, bearer tokens or API keys printed by a crashing CLI.
MAX_ERROR_CHARS = 1000
_SECRET_PATTERNS: list[tuple[re.Pattern[str], str]] = [
    (re.compile(r"(sk-[A-Za-z0-9_-]{6})[A-Za-z0-9_-]*"), r"\1***"),
    (re.compile(r"(Bearer\s+)[A-Za-z0-9._\-]+", re.IGNORECASE), r"\1***"),
    (
        re.compile(
            r"((?:api[_-]?key|token|password|passwd|secret|authorization)"
            r"[\"']?\s*[:=]\s*[\"']?)[^\"'\s,;]+",
            re.IGNORECASE,
        ),
        r"\1***",
    ),
    (re.compile(r"([A-Z][A-Z0-9_]{2,}(?:KEY|TOKEN|SECRET|PASS)=)[^\s&]+"), r"\1***"),
]


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def read_json_safe(path: Path) -> Any:
    """Return parsed JSON or ``None`` for missing/unreadable/corrupt files.

    A read-only observer must never raise on a half-written checkpoint.
    """
    try:
        if not path.exists():
            return None
        return json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError, UnicodeDecodeError):
        return None


def _redact_secrets(text: str) -> str:
    redacted = text
    for pattern, replacement in _SECRET_PATTERNS:
        redacted = pattern.sub(replacement, redacted)
    return redacted


def sanitize_error(value: Any, max_chars: int = MAX_ERROR_CHARS) -> str | None:
    """Cap length and redact secret-like substrings from an error string."""
    if value is None:
        return None
    text = str(value)
    if len(text) > max_chars:
        text = text[:max_chars] + "...[truncated]"
    text = text.replace("\\", "/")
    return _redact_secrets(text)


def sanitize_path(value: Any, roots: tuple[Path, ...]) -> str | None:
    """Relativize an absolute path against known roots; fall back to basename.

    Unknown absolute paths collapse to their basename so a foreign path
    structure (e.g. a worker's temp dir) is never leaked.
    """
    if value is None:
        return None
    text = str(value).strip()
    if not text:
        return None
    try:
        path = Path(text)
    except (TypeError, ValueError):
        return None
    for root in roots:
        try:
            return path.relative_to(root).as_posix()
        except (ValueError, TypeError):
            continue
    # Already-relative artifact path (e.g. "docs/bench_notes/x.md").
    if not path.is_absolute():
        return Path(text).as_posix()
    return path.name or None


def _status_counts(task_nodes: list[dict[str, Any]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for node in task_nodes:
        status = node.get("status", "pending")
        counts[status] = counts.get(status, 0) + 1
    return counts


def _derive_run_status(
    task_raw: list[dict[str, Any]],
    report: dict[str, Any] | None,
) -> str:
    if report and report.get("status"):
        return str(report["status"])
    if not task_raw:
        return "empty"
    statuses = {task.get("status", "pending") for task in task_raw}
    if statuses <= {"completed"}:
        return "completed"
    if statuses & TASK_RUNNING_STATUSES:
        return "running"
    if statuses & {"failed"}:
        return "failed"
    # No running tasks but not all completed: interrupted/partial.
    return "partial"


def _load_claims(claims_dir: Path) -> dict[str, dict[str, Any]]:
    """Index task_id -> claim payload, tolerating corrupt locks."""
    index: dict[str, dict[str, Any]] = {}
    if not claims_dir.exists():
        return index
    for path in sorted(claims_dir.glob("*.lock")):
        data = read_json_safe(path)
        if not isinstance(data, dict):
            continue
        task_id = data.get("task_id") or path.stem
        index[str(task_id)] = data
    return index


def _build_task_node(
    task: dict[str, Any],
    agent_index: dict[str, dict[str, Any]],
    claim_index: dict[str, dict[str, Any]],
    roots: tuple[Path, ...],
) -> dict[str, Any]:
    task_id = str(task.get("task_id", ""))
    claim = claim_index.get(task_id, {})
    subagents: list[dict[str, Any]] = []
    for agent_id in task.get("subagents") or []:
        agent_id = str(agent_id)
        child = agent_index.get(agent_id)
        subagents.append(
            {
                "agent_id": agent_id,
                "task_id": child.get("task_id") if child else None,
                "status": child.get("status", "unknown") if child else "unknown",
                "model": child.get("model") if child else None,
            }
        )
    heartbeat_unix_ms = task.get("heartbeat_unix_ms")
    return {
        "task_id": task_id,
        "index": task.get("index", 0),
        "role": task.get("role", "general"),
        "source_id": task.get("source_id"),
        "parent_task_id": task.get("parent_task_id"),
        "status": task.get("status", "pending"),
        "attempts": task.get("attempts", 0),
        "model": task.get("model"),
        "provider": task.get("provider"),
        "route": task.get("route"),
        "wrapper": task.get("wrapper"),
        "variant": task.get("variant"),
        "agent": {
            "agent_id": task.get("agent_id") or task.get("source_id") or task_id,
            "owner": claim.get("owner"),
            "claimed_at": claim.get("claimed_at"),
            "heartbeat_at": claim.get("heartbeat_at"),
        },
        "subagents": subagents,
        "provider_subagents": list(task.get("provider_subagents") or []),
        "provider_subagent_visibility": task.get("provider_subagent_visibility", "not_reported"),
        "timestamps": {
            "started_at": task.get("started_at"),
            "ended_at": task.get("ended_at"),
            "heartbeat_unix_ms": heartbeat_unix_ms,
        },
        "needs": list(task.get("needs") or []),
        "artifacts": [sanitize_path(artifact, roots) for artifact in (task.get("artifacts") or [])],
        "error": sanitize_error(task.get("error")),
    }


class RunObservability:
    """Read-only observer for a single SWARMS run directory."""

    def __init__(self, run_dir: Path, roots: tuple[Path, ...] | None = None):
        self.run_dir = Path(run_dir)
        self.tasks_dir = self.run_dir / "tasks"
        self.results_dir = self.run_dir / "results"
        self.claims_dir = self.run_dir / "claims"
        self._roots = tuple(roots) if roots else (WORKSPACE_ROOT, PROJECT_ROOT)

    @classmethod
    def from_run(
        cls,
        run_id: str,
        run_root: Path = DEFAULT_RUNS_DIR,
        roots: tuple[Path, ...] | None = None,
    ) -> RunObservability:
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", run_id or ""):
            raise ValueError(f"Unsafe run_id for observation: {run_id!r}")
        run_root = Path(run_root).resolve()
        run_dir = (run_root / run_id).resolve()
        if run_dir.parent != run_root:
            raise ValueError(f"run_id escapes run_root: {run_id!r}")
        return cls(run_dir, roots=roots)

    @property
    def exists(self) -> bool:
        return self.run_dir.is_dir()

    def _load_workflow(self) -> dict[str, Any]:
        data = read_json_safe(self.run_dir / "workflow.json")
        return data if isinstance(data, dict) else {}

    def _load_tasks_raw(self) -> list[dict[str, Any]]:
        if not self.tasks_dir.exists():
            return []
        tasks: list[dict[str, Any]] = []
        for path in sorted(self.tasks_dir.glob("*.json")):
            data = read_json_safe(path)
            if isinstance(data, dict):
                tasks.append(data)
        return tasks

    def _load_report(self) -> dict[str, Any] | None:
        data = read_json_safe(self.run_dir / "report.json")
        return data if isinstance(data, dict) else None

    def _count_results(self) -> int:
        if not self.results_dir.exists():
            return 0
        return sum(1 for _ in self.results_dir.glob("*/result.json"))

    def build_contract(self) -> dict[str, Any]:
        """Build the versioned, sanitized, read-only run contract."""
        workflow = self._load_workflow()
        tasks_raw = self._load_tasks_raw()
        report = self._load_report()

        workspace_root = workflow.get("workspace_root")
        roots = self._roots
        if workspace_root:
            roots = (Path(str(workspace_root)),) + self._roots

        claim_index = _load_claims(self.claims_dir)
        agent_index: dict[str, dict[str, Any]] = {}
        for task in tasks_raw:
            agent_id = task.get("agent_id") or task.get("source_id") or task.get("task_id")
            if agent_id:
                agent_index[str(agent_id)] = task

        task_nodes = [_build_task_node(task, agent_index, claim_index, roots) for task in tasks_raw]

        stages = self._group_stages_by_name(task_nodes, tasks_raw)
        status_counts = _status_counts(task_nodes)
        heartbeats = [
            node["timestamps"]["heartbeat_unix_ms"] for node in task_nodes if node["timestamps"]["heartbeat_unix_ms"]
        ]

        run_status = _derive_run_status(tasks_raw, report)
        has_real_provider = any(task.get("provider") not in (None, "mock") for task in tasks_raw)

        run_meta = {
            "run_id": workflow.get("run_id", self.run_dir.name),
            "runtime": workflow.get("runtime", "unknown"),
            "state_schema_version": workflow.get("state_schema_version"),
            "created_at": workflow.get("created_at"),
            "status": run_status,
            "workspace_root": sanitize_path(workspace_root, roots),
            "heartbeat_interval_seconds": workflow.get("heartbeat_interval_seconds"),
            "global_max_concurrency": workflow.get("global_max_concurrency"),
            "provider_max_concurrency": workflow.get("provider_max_concurrency"),
            "max_total_workers": workflow.get("max_total_workers"),
            "task_count": workflow.get("task_count", len(tasks_raw)),
            "tasks_file": sanitize_path(workflow.get("tasks_file"), roots),
            "workflow_plan": sanitize_path(workflow.get("workflow_plan"), roots),
            "observed_at": utc_now(),
        }

        return {
            "contract_schema_version": CONTRACT_SCHEMA_VERSION,
            "read_only": True,
            "run": run_meta,
            "summary": {
                "stage_count": len(stages),
                "task_status_counts": status_counts,
                "has_real_provider": has_real_provider,
                "last_heartbeat_unix_ms": max(heartbeats) if heartbeats else None,
                "result_count": self._count_results(),
                "report_status": report.get("status") if report else None,
            },
            "stages": stages,
        }

    @staticmethod
    def _group_stages_by_name(
        task_nodes: list[dict[str, Any]],
        tasks_raw: list[dict[str, Any]],
    ) -> list[dict[str, Any]]:
        """Group nodes by their source stage name, preserving plan order."""
        # Map task_id -> node for ordered joining.
        node_by_id = {node["task_id"]: node for node in task_nodes}
        ordered_pairs: list[tuple[str, dict[str, Any]]] = []
        for raw in sorted(tasks_raw, key=lambda item: (item.get("index", 0), str(item.get("task_id", "")))):
            task_id = str(raw.get("task_id", ""))
            node = node_by_id.get(task_id)
            if node is None:
                continue
            ordered_pairs.append((str(raw.get("stage") or "Unnamed"), node))

        stages: list[dict[str, Any]] = []
        current: dict[str, Any] | None = None
        for stage_name, node in ordered_pairs:
            if current is None or current["name"] != stage_name:
                current = {
                    "name": stage_name,
                    "tasks": [],
                    "status_counts": {},
                }
                stages.append(current)
            current["tasks"].append(node)
            status = node["status"]
            current["status_counts"][status] = current["status_counts"].get(status, 0) + 1
        return stages


def list_runs(run_root: Path = DEFAULT_RUNS_DIR) -> list[dict[str, Any]]:
    """Return a compact, sanitized index of every run under ``run_root``."""
    run_root = Path(run_root)
    if not run_root.exists():
        return []
    runs: list[dict[str, Any]] = []
    for entry in sorted(run_root.iterdir()):
        if not entry.is_dir():
            continue
        workflow = read_json_safe(entry / "workflow.json")
        workflow = workflow if isinstance(workflow, dict) else {}
        tasks_dir = entry / "tasks"
        task_count = len(list(tasks_dir.glob("*.json"))) if tasks_dir.exists() else 0
        runs.append(
            {
                "run_id": workflow.get("run_id", entry.name),
                "runtime": workflow.get("runtime", "unknown"),
                "created_at": workflow.get("created_at"),
                "task_count": task_count,
                "has_report": (entry / "report.json").exists(),
            }
        )
    return runs


def iter_events(run_dir: Path) -> Iterator[dict[str, Any]]:
    """Yield sanitized event rows from ``events.jsonl`` (best-effort)."""
    events_path = Path(run_dir) / "events.jsonl"
    if not events_path.exists():
        return
    with events_path.open("r", encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError:
                continue


def main() -> int:
    parser = argparse.ArgumentParser(description="Emit a read-only, versioned observability contract for a SWARMS run.")
    parser.add_argument("--run-id", help="Run id to inspect")
    parser.add_argument(
        "--run-root",
        type=Path,
        default=DEFAULT_RUNS_DIR,
        help="Root directory holding runs",
    )
    parser.add_argument("--list", action="store_true", help="List runs and exit")
    args = parser.parse_args()

    if args.list:
        print(json.dumps(list_runs(args.run_root), indent=2, sort_keys=True))
        return 0

    if not args.run_id:
        parser.error("--run-id is required unless --list is used")

    observer = RunObservability.from_run(args.run_id, args.run_root)
    if not observer.exists:
        print(json.dumps({"error": f"run not found: {args.run_id}"}, indent=2))
        return 1
    print(json.dumps(observer.build_contract(), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
