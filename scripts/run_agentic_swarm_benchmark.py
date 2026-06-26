#!/usr/bin/env python3
"""Run a cheap agentic benchmark matrix for SWARMS.

This runner intentionally does not call Claude Code. It compares:
- glm52_only: one GLM 5.2 worker, serial
- gemini_flash_only: one Gemini 3.5 Flash worker, serial
- swarm_auto: configurable SWARMS routing, parallel workers

The tasks are small Terminal-Bench/DeepSWE-inspired workflows with stages,
@needs dependencies, and local verification commands.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.append(str(PROJECT_ROOT))

from scripts.utils.token_telemetry import iter_events, summarize_events
from scripts.utils.token_telemetry import parse_codex_log, record_event


VARIANTS = {
    "mock_swarm": {"strategy": "mock-only", "workers": 3},
    "glm52_only": {"strategy": "glm-only", "workers": 1},
    "gemini_flash_only": {"strategy": "gemini-only", "workers": 1},
    "swarm_auto": {"strategy": "auto", "workers": 3},
    "gpt55_medium_orchestrate_glm52": {
        "strategy": "glm-only",
        "workers": 3,
        "orchestrator": "codex_medium",
    },
}


class AgenticSwarmBenchmark:
    def __init__(
        self,
        tasks_file: Path,
        variants: list[str],
        limit: int,
        keep_worktrees: bool = False,
        worker_timeout_minutes: int = 12,
        runner_timeout_seconds: int = 1800,
    ):
        self.tasks_file = tasks_file
        self.variants = variants
        self.limit = limit
        self.keep_worktrees = keep_worktrees
        self.worker_timeout_minutes = worker_timeout_minutes
        self.runner_timeout_seconds = runner_timeout_seconds
        self.benchmark_id = str(uuid.uuid4())
        self.run_dir = PROJECT_ROOT / ".agent" / "agentic_benchmark"
        self.telemetry_file = PROJECT_ROOT / ".agent" / "traces" / "telemetry.jsonl"
        self.base_refs: dict[str, str] = {}

    def load_tasks(self) -> list[dict[str, Any]]:
        tasks = json.loads(self.tasks_file.read_text(encoding="utf-8"))
        return tasks[: self.limit]

    def render_task_file(self, task: dict[str, Any]) -> str:
        lines = [f"# Agentic benchmark task: {task['instance_id']}", ""]
        lines.append(task.get("description", "").strip())
        lines.append("")
        for stage in task["stages"]:
            lines.append(f"## {stage['name']}")
            for item in stage["tasks"]:
                lines.append(f"- [ ] {item}")
            lines.append("")
        return "\n".join(lines)

    def render_orchestrator_prompt(self, task: dict[str, Any]) -> str:
        return f"""You are the planning coordinator for a cheap GLM-5.2 worker swarm.

Goal: rewrite this benchmark task into a concise staged task backlog for parallel workers.

Rules:
- Output ONLY markdown taskfile content.
- Do not solve the task.
- Do not include prose outside the taskfile.
- Use stages with markdown headings.
- Use unchecked tasks in the form "- [ ] [role] task".
- Use @needs(...) when a task must wait for another.
- Prefer roles: [docs], [backend], [qa], [lite], [debug].
- Assume workers are GLM-5.2 only, so make tasks explicit and verifiable.

Benchmark task:
{json.dumps(task, indent=2)}
"""

    def orchestrate_with_codex_medium(self, task: dict[str, Any], wt_path: Path, run_id: str) -> str:
        started_at = datetime.now(timezone.utc).isoformat()
        prompt = self.render_orchestrator_prompt(task)
        log_file = wt_path / "orchestrator_codex_medium.jsonl"
        out_file = wt_path / "orchestrated_tasks.md"
        codex_path = r"C:\Users\matia\.bun\bin\codex.exe"
        cmd = [
            codex_path,
            "exec",
            "-s",
            "workspace-write",
            "-c",
            "reasoning_effort=medium",
            "-o",
            str(out_file),
            "--json",
            prompt,
        ]
        env = os.environ.copy()
        env["SWARM_BENCHMARK_ID"] = self.benchmark_id
        env["SWARM_RUN_ID"] = run_id
        env["SWARM_TELEMETRY_FILE"] = str(self.telemetry_file)
        success = False
        try:
            with log_file.open("w", encoding="utf-8") as out:
                result = subprocess.run(
                    cmd,
                    cwd=wt_path,
                    stdout=out,
                    stderr=subprocess.PIPE,
                    text=True,
                    env=env,
                    timeout=300,
                )
            success = result.returncode == 0 and out_file.exists()
        except Exception:
            success = False
        ended_at = datetime.now(timezone.utc).isoformat()
        usage = parse_codex_log(log_file)
        record_event(
            run_id=run_id,
            benchmark_id=self.benchmark_id,
            phase="swarm",
            provider="codex_cli",
            model="gpt-5.5-codex",
            role="coordinator",
            task_id=task["instance_id"],
            input_tokens=usage.get("input", 0),
            cache_read_tokens=usage.get("cache_read_input_tokens", usage.get("cached", 0)),
            cache_write_tokens=usage.get("cache_write_input_tokens", 0),
            output_tokens=usage.get("output", 0),
            reasoning_tokens=usage.get("reasoning_output_tokens", usage.get("reasoning", 0)),
            usage_source="cli_reported" if usage.get("input", 0) else "missing",
            success=success,
            started_at=started_at,
            ended_at=ended_at,
            route_id="codex_medium_orchestrator",
            routing_method="fixed_orchestrator",
            routing_reason="GPT-5.5 medium decomposes task; GLM-5.2 workers execute",
        )
        if success:
            return out_file.read_text(encoding="utf-8")
        return self.render_task_file(task)

    def setup_worktree(self, task: dict[str, Any], variant: str) -> Path:
        self.run_dir.mkdir(parents=True, exist_ok=True)
        safe_id = task["instance_id"].replace("/", "_")
        wt_path = self.run_dir / f"{safe_id}_{variant}"
        if wt_path.exists():
            shutil.rmtree(wt_path, ignore_errors=True)
        subprocess.run(["git", "worktree", "prune"], cwd=PROJECT_ROOT, capture_output=True)
        branch = f"agentic-bench/{safe_id}-{variant}"
        subprocess.run(["git", "branch", "-D", branch], cwd=PROJECT_ROOT, capture_output=True)
        result = subprocess.run(
            ["git", "worktree", "add", "-b", branch, str(wt_path), "HEAD"],
            cwd=PROJECT_ROOT,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise RuntimeError(result.stderr or result.stdout)
        self.seed_benchmark_scaffold(wt_path)
        self.base_refs[str(wt_path)] = subprocess.check_output(
            ["git", "rev-parse", "HEAD"],
            cwd=wt_path,
            text=True,
        ).strip()
        for env_path in (PROJECT_ROOT / ".env", PROJECT_ROOT.parent / ".env"):
            if env_path.exists():
                shutil.copy(env_path, wt_path / ".env")
                break
        (wt_path / ".agent").mkdir(exist_ok=True)
        (wt_path / ".agent" / "tasks_agentic_bench.md").write_text(self.render_task_file(task), encoding="utf-8")
        return wt_path

    def seed_benchmark_scaffold(self, wt_path: Path) -> None:
        seed_files = {
            "bench_apps/__init__.py": "",
            "bench_apps/reshard/__init__.py": "",
            "bench_tests/.gitkeep": "",
            "docs/bench_notes/.gitkeep": "",
        }
        for rel_path, content in seed_files.items():
            path = wt_path / rel_path
            path.parent.mkdir(parents=True, exist_ok=True)
            if not path.exists():
                path.write_text(content, encoding="utf-8")
        subprocess.run(["git", "add", *seed_files.keys()], cwd=wt_path, check=True)
        subprocess.run(
            ["git", "commit", "-m", "Seed agentic benchmark scaffold"],
            cwd=wt_path,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

    def cleanup_worktree(self, wt_path: Path, task: dict[str, Any], variant: str) -> None:
        if self.keep_worktrees:
            return
        shutil.rmtree(wt_path, ignore_errors=True)
        subprocess.run(["git", "worktree", "prune"], cwd=PROJECT_ROOT, capture_output=True)
        safe_id = task["instance_id"].replace("/", "_")
        subprocess.run(["git", "branch", "-D", f"agentic-bench/{safe_id}-{variant}"], cwd=PROJECT_ROOT, capture_output=True)

    def run_variant(self, task: dict[str, Any], variant: str) -> dict[str, Any]:
        cfg = VARIANTS[variant]
        run_id = str(uuid.uuid4())
        wt_path = self.setup_worktree(task, variant)
        started_at = datetime.now(timezone.utc)
        env = os.environ.copy()
        env["SWARM_BENCHMARK_ID"] = self.benchmark_id
        env["SWARM_RUN_ID"] = run_id
        env["SWARM_TELEMETRY_FILE"] = str(self.telemetry_file)
        if cfg.get("orchestrator") == "codex_medium":
            task_file_content = self.orchestrate_with_codex_medium(task, wt_path, run_id)
            (wt_path / ".agent" / "tasks_agentic_bench.md").write_text(task_file_content, encoding="utf-8")
        cmd = [
            "powershell",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(PROJECT_ROOT / "scripts" / "parallel_swarm.ps1"),
            "-TaskFile",
            str(wt_path / ".agent" / "tasks_agentic_bench.md"),
            "-ProjectRoot",
            str(wt_path),
            "-ProviderStrategy",
            cfg["strategy"],
            "-WorkerCount",
            str(cfg["workers"]),
            "-Background",
            "-NoRetry",
            "-WorkerTimeoutMinutes",
            str(self.worker_timeout_minutes),
        ]
        try:
            result = subprocess.run(
                cmd,
                cwd=wt_path,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=self.runner_timeout_seconds,
            )
            runner_success = result.returncode == 0
        except subprocess.TimeoutExpired as exc:
            result = exc
            runner_success = False

        verify = self.verify_task(task, wt_path)
        ended_at = datetime.now(timezone.utc)
        events = [event for event in iter_events(self.telemetry_file) if event.get("run_id") == run_id]
        summary = summarize_events(events)
        self.cleanup_worktree(wt_path, task, variant)
        return {
            "variant": variant,
            "strategy": cfg["strategy"],
            "workers": cfg["workers"],
            "run_id": run_id,
            "runner_success": runner_success,
            "verified": verify["success"],
            "verify": verify,
            "started_at": started_at.isoformat(),
            "ended_at": ended_at.isoformat(),
            "duration_seconds": round((ended_at - started_at).total_seconds(), 2),
            "telemetry": summary["totals"],
            "by_phase_provider_model_role": summary["by_phase_provider_model_role"],
            "stdout_tail": getattr(result, "stdout", "")[-4000:],
            "stderr_tail": getattr(result, "stderr", "")[-4000:],
        }

    def verify_task(self, task: dict[str, Any], wt_path: Path) -> dict[str, Any]:
        cmd = task.get("verify_command")
        if not cmd:
            return {"success": False, "reason": "missing verify_command"}
        result = subprocess.run(
            ["powershell", "-NoProfile", "-Command", cmd],
            cwd=wt_path,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=300,
        )
        missing = [p for p in task.get("success_artifacts", []) if not (wt_path / p).exists()]
        changed_files = self.changed_files_since_base(wt_path)
        disallowed_changes = self.disallowed_benchmark_changes(changed_files)
        return {
            "success": result.returncode == 0 and not missing and not disallowed_changes,
            "returncode": result.returncode,
            "missing_artifacts": missing,
            "changed_files": changed_files,
            "disallowed_changes": disallowed_changes,
            "stdout_tail": result.stdout[-2000:],
            "stderr_tail": result.stderr[-2000:],
        }

    def changed_files_since_base(self, wt_path: Path) -> list[str]:
        base_ref = self.base_refs.get(str(wt_path))
        if not base_ref:
            return []
        result = subprocess.run(
            ["git", "diff", "--name-only", f"{base_ref}..HEAD"],
            cwd=wt_path,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=60,
        )
        if result.returncode != 0:
            return [f"<git diff failed: {result.stderr.strip()}>"]
        return [line.strip().replace("\\", "/") for line in result.stdout.splitlines() if line.strip()]

    @staticmethod
    def disallowed_benchmark_changes(changed_files: list[str]) -> list[str]:
        allowed_prefixes = ("bench_apps/", "bench_tests/", "docs/bench_notes/")
        return [path for path in changed_files if not path.startswith(allowed_prefixes)]

    def execute(self) -> list[dict[str, Any]]:
        tasks = self.load_tasks()
        report = {
            "benchmark_id": self.benchmark_id,
            "tasks_file": str(self.tasks_file),
            "variants": self.variants,
            "started_at": datetime.now(timezone.utc).isoformat(),
            "results": [],
        }
        for task in tasks:
            task_result = {"instance_id": task["instance_id"], "runs": []}
            for variant in self.variants:
                print(f"[agentic-bench] {task['instance_id']} :: {variant}")
                task_result["runs"].append(self.run_variant(task, variant))
            report["results"].append(task_result)
        report["ended_at"] = datetime.now(timezone.utc).isoformat()
        out = PROJECT_ROOT / "config" / "agentic_benchmark_report.json"
        out.write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(f"[agentic-bench] report saved to {out}")
        return report["results"]


def main() -> int:
    parser = argparse.ArgumentParser(description="Run cheap SWARMS agentic benchmark matrix")
    parser.add_argument("--tasks-file", type=Path, default=PROJECT_ROOT / "docs" / "agentic_swarm_micro_tasks.json")
    parser.add_argument("--limit", type=int, default=1)
    parser.add_argument(
        "--variants",
        default="mock_swarm",
        help="Comma-separated variants: mock_swarm, glm52_only, gemini_flash_only, swarm_auto, gpt55_medium_orchestrate_glm52",
    )
    parser.add_argument("--keep-worktrees", action="store_true")
    parser.add_argument("--worker-timeout-minutes", type=int, default=12)
    parser.add_argument("--runner-timeout-seconds", type=int, default=1800)
    args = parser.parse_args()
    variants = [v.strip() for v in args.variants.split(",") if v.strip()]
    unknown = [v for v in variants if v not in VARIANTS]
    if unknown:
        raise SystemExit(f"unknown variants: {', '.join(unknown)}")
    AgenticSwarmBenchmark(
        args.tasks_file,
        variants,
        args.limit,
        args.keep_worktrees,
        worker_timeout_minutes=args.worker_timeout_minutes,
        runner_timeout_seconds=args.runner_timeout_seconds,
    ).execute()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
