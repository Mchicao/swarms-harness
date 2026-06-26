#!/usr/bin/env python3
"""
SWARM Benchmark Runner - Executes baseline vs swarm runs on SWE-bench tasks
and calculates CTR (Coordinator Token Reduction), TCR (Total Cost Ratio),
and TTA (Total Token Amplification).
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path

# Add project root to sys.path
PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.append(str(PROJECT_ROOT))

from scripts.utils.token_telemetry import iter_events, record_event, summarize_events

class SwarmBenchmark:
    def __init__(self, tasks_file: str, task_id: str = None, limit: int = 1, strategy: str = "role-based"):
        self.tasks_file = Path(tasks_file)
        self.task_id = task_id
        self.limit = limit
        self.strategy = strategy
        self.benchmark_id = str(uuid.uuid4())
        self.run_dir = PROJECT_ROOT / ".agent" / "benchmark"
        self.telemetry_file = PROJECT_ROOT / ".agent" / "traces" / "telemetry.jsonl"
        
    def load_tasks(self) -> list:
        """Loads SWE-bench tasks and returns a list of paginated task capsules."""
        if not self.tasks_file.exists():
            print(f"❌ Tasks file not found: {self.tasks_file}")
            sys.exit(1)
            
        with open(self.tasks_file, encoding="utf-8") as f:
            tasks = json.load(f)
            
        if self.task_id:
            tasks = [t for t in tasks if t["instance_id"] == self.task_id]
            
        return tasks[:self.limit]

    def setup_worktree(self, task: dict, suffix: str) -> Path:
        """Creates an isolated git worktree checked out at the task's base_commit."""
        worktree_path = self.run_dir / f"{task['instance_id']}_{suffix}"
        if worktree_path.exists():
            shutil.rmtree(worktree_path, ignore_errors=True)
            
        # Run git worktree prune first
        subprocess.run(["git", "worktree", "prune"], cwd=PROJECT_ROOT, capture_output=True)
        
        branch_name = f"bench/{task['instance_id']}-{suffix}"
        # Delete branch if exists
        subprocess.run(["git", "branch", "-D", branch_name], cwd=PROJECT_ROOT, capture_output=True)
        
        print(f"🔧 Creating worktree at {worktree_path} for commit {task['base_commit']}...")
        result = subprocess.run(
            ["git", "worktree", "add", "-b", branch_name, str(worktree_path), task["base_commit"]],
            cwd=PROJECT_ROOT,
            capture_output=True,
            text=True
        )
        if result.returncode != 0:
            # Fallback to HEAD if commit checkout fails
            subprocess.run(
                ["git", "worktree", "add", "-b", branch_name, str(worktree_path), "HEAD"],
                cwd=PROJECT_ROOT,
                capture_output=True
            )
            
        # Copy env file if present
        if (PROJECT_ROOT / ".env").exists():
            shutil.copy(PROJECT_ROOT / ".env", worktree_path / ".env")
            
        return worktree_path

    def run_baseline(self, task: dict, wt_path: Path) -> dict:
        """Runs the baseline coordinator on the task statement and returns tokens consumed."""
        print(f"🚀 Running baseline coordinator for {task['instance_id']}...")
        run_id = str(uuid.uuid4())
        started_at = datetime.now(timezone.utc).isoformat()
        
        # Build prompt capsule
        prompt = f"""
You are resolving the following issue in the repository:
Instance ID: {task['instance_id']}
Problem Statement:
{task['problem_statement']}

Please modify the codebase to resolve this issue. Run pytest tests/ to verify your changes.
"""
        prompt_file = wt_path / "prompt_baseline.txt"
        prompt_file.write_text(prompt, encoding="utf-8")
        
        log_file = wt_path / "baseline_out.jsonl"
        
        # Invoke Codex CLI
        codex_path = r"C:\Users\matia\.bun\bin\codex.exe"
        cmd = [
            codex_path, "exec",
            "-s", "workspace-write",
            "-c", "reasoning_effort=high",
            "-o", str(wt_path / "baseline_summary.md"),
            "--json",
            prompt
        ]
        
        # Run process
        try:
            # Set environment variables for benchmark tracking
            env = os.environ.copy()
            env["SWARM_BENCHMARK_ID"] = self.benchmark_id
            env["SWARM_RUN_ID"] = run_id
            env["SWARM_TELEMETRY_FILE"] = str(self.telemetry_file)
            
            with open(log_file, "w") as out:
                result = subprocess.run(
                    cmd,
                    cwd=wt_path,
                    stdout=out,
                    stderr=subprocess.PIPE,
                    env=env,
                    timeout=300
                )
            success = result.returncode == 0
        except subprocess.TimeoutExpired:
            print("⏳ Baseline run timed out.")
            success = False
        except Exception as e:
            print(f"❌ Failed to execute baseline: {e}")
            success = False
            
        ended_at = datetime.now(timezone.utc).isoformat()
        
        # Read final usage from the log file
        input_tokens = 0
        output_tokens = 0
        cached_tokens = 0
        reasoning_tokens = 0
        
        if log_file.exists():
            try:
                with open(log_file, encoding="utf-8") as f:
                    for line in f:
                        if not line.strip():
                            continue
                        data = json.loads(line)
                        if "usage" in data:
                            usage = data["usage"]
                            input_tokens = usage.get("prompt_tokens", 0)
                            output_tokens = usage.get("completion_tokens", 0)
                            details = usage.get("prompt_tokens_details", {})
                            cached_tokens = details.get("cached_tokens", 0) if isinstance(details, dict) else getattr(details, "cached_tokens", 0)
                            r_details = usage.get("completion_tokens_details", {})
                            reasoning_tokens = r_details.get("reasoning_tokens", 0) if isinstance(r_details, dict) else getattr(r_details, "reasoning_tokens", 0)
            except Exception as e:
                print(f"⚠️ Warning: could not parse baseline token usage: {e}")
                
        # Record event
        record_event(
            run_id=run_id,
            benchmark_id=self.benchmark_id,
            phase="baseline",
            provider="codex_cli",
            model="gpt-5.5-codex",
            role="coordinator",
            task_id=task["instance_id"],
            input_tokens=input_tokens,
            cache_read_tokens=cached_tokens,
            output_tokens=output_tokens,
            reasoning_tokens=reasoning_tokens,
            usage_source="cli_reported" if input_tokens > 0 else "missing",
            success=success,
            started_at=started_at,
            ended_at=ended_at
        )
        
        return {
            "success": success,
            "run_id": run_id,
            "input": input_tokens,
            "output": output_tokens,
            "cached": cached_tokens,
            "reasoning": reasoning_tokens,
            "events": [event for event in iter_events(self.telemetry_file) if event.get("run_id") == run_id],
        }

    def run_swarm(self, task: dict, wt_path: Path) -> dict:
        """Runs the Swarm engine on the task and captures total token usage."""
        print(f"🚀 Running Swarm on {task['instance_id']}...")
        run_id = str(uuid.uuid4())
        started_at = datetime.now(timezone.utc).isoformat()
        
        # Prepare task backlog tasks.md file in the worktree
        task_md = wt_path / "tasks.md"
        task_md_content = f"""# Swarm Task: {task['instance_id']}
## Issue Resolution
- [ ] [backend] Resolve problem: {task['problem_statement']}
"""
        task_md.write_text(task_md_content, encoding="utf-8")
        
        # Launch parallel_swarm.ps1
        # Set environment variables for the subprocesses
        env = os.environ.copy()
        env["SWARM_BENCHMARK_ID"] = self.benchmark_id
        env["SWARM_RUN_ID"] = run_id
        env["SWARM_TELEMETRY_FILE"] = str(self.telemetry_file)
        
        cmd = [
            "powershell", "-ExecutionPolicy", "Bypass", "-File",
            str(PROJECT_ROOT / "scripts" / "parallel_swarm.ps1"),
            "-TaskFile", str(task_md),
            "-ProviderStrategy", self.strategy,
            "-WorkerCount", "2"
        ]
        
        try:
            result = subprocess.run(
                cmd,
                cwd=wt_path,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=env,
                timeout=600
            )
            success = result.returncode == 0
        except subprocess.TimeoutExpired:
            print("⏳ Swarm execution timed out.")
            success = False
        except Exception as e:
            print(f"❌ Failed to run Swarm: {e}")
            success = False
            
        ended_at = datetime.now(timezone.utc).isoformat()
        
        # Read the telemetry events generated by Swarm in this run
        events = [event for event in iter_events(self.telemetry_file) if event.get("run_id") == run_id]
        summary = summarize_events(events)["totals"]
                        
        return {
            "success": success,
            "run_id": run_id,
            "input": summary["input_tokens"],
            "output": summary["output_tokens"],
            "cached": summary["cache_read_tokens"],
            "reasoning": summary["reasoning_tokens"],
            "events": events,
            "summary": summary,
        }

    def cleanup(self, wt_path: Path, task: dict, suffix: str):
        """Cleans up the git worktree and branch."""
        shutil.rmtree(wt_path, ignore_errors=True)
        subprocess.run(["git", "worktree", "prune"], cwd=PROJECT_ROOT, capture_output=True)
        branch_name = f"bench/{task['instance_id']}-{suffix}"
        subprocess.run(["git", "branch", "-D", branch_name], cwd=PROJECT_ROOT, capture_output=True)

    def calculate_results(self, baseline: dict, swarm: dict) -> dict:
        """Computes benchmark metrics CTR, TCR, TTA."""
        baseline_events = baseline.get("events", [])
        swarm_events = swarm.get("events", [])

        baseline_summary = summarize_events(baseline_events)["totals"]
        swarm_summary = summarize_events(swarm_events)["totals"]

        expensive_baseline = baseline["input"] + baseline["output"] + baseline.get("reasoning", 0)
        coordinator_swarm = sum(
            event.get("input_tokens", 0) + event.get("output_tokens", 0) + event.get("reasoning_tokens", 0)
            for event in swarm_events
            if event.get("role") == "coordinator"
        )
        total_swarm_tokens = swarm["input"] + swarm["output"] + swarm.get("reasoning", 0)

        baseline_cost = baseline_summary["known_cost_usd"]
        swarm_cost = swarm_summary["known_cost_usd"]
        known_cost_events = baseline_summary["events"] - baseline_summary["unknown_cost_events"] + swarm_summary["events"] - swarm_summary["unknown_cost_events"]
        all_events = baseline_summary["events"] + swarm_summary["events"]

        ctr = (coordinator_swarm / expensive_baseline) if expensive_baseline > 0 else 0.0
        tcr = (swarm_cost / baseline_cost) if baseline_cost > 0 else 0.0
        tta = (total_swarm_tokens / expensive_baseline) if expensive_baseline > 0 else 0.0
        cost_coverage = (known_cost_events / all_events) if all_events > 0 else 0.0
        missing_usage_events = baseline_summary["missing_usage_events"] + swarm_summary["missing_usage_events"]
        
        return {
            "CTR": round(ctr, 4),
            "TCR": round(tcr, 4),
            "TTA": round(tta, 4),
            "baseline_cost_usd": round(baseline_cost, 6),
            "swarm_cost_usd": round(swarm_cost, 6),
            "cost_coverage": round(cost_coverage, 4),
            "missing_usage_events": missing_usage_events,
            "baseline_events": baseline_summary["events"],
            "swarm_events": swarm_summary["events"],
        }

    def execute(self):
        tasks = self.load_tasks()
        print(f"📊 Running benchmark with {len(tasks)} task(s)...")
        
        results = []
        for task in tasks:
            print(f"\n--- TASK: {task['instance_id']} ---")
            
            # Setup & Run Baseline
            wt_baseline = self.setup_worktree(task, "baseline")
            baseline_stats = self.run_baseline(task, wt_baseline)
            self.cleanup(wt_baseline, task, "baseline")
            
            # Setup & Run Swarm
            wt_swarm = self.setup_worktree(task, "swarm")
            swarm_stats = self.run_swarm(task, wt_swarm)
            self.cleanup(wt_swarm, task, "swarm")
            
            # Evaluate Metrics
            metrics = self.calculate_results(baseline_stats, swarm_stats)
            
            task_result = {
                "instance_id": task["instance_id"],
                "baseline": baseline_stats,
                "swarm": swarm_stats,
                "metrics": metrics
            }
            results.append(task_result)
            
            print(f"✨ Task Results for {task['instance_id']}:")
            print(f"  CTR (Coordinator Token Reduction): {metrics['CTR']}")
            print(f"  TCR (Total Cost Ratio):            {metrics['TCR']}")
            print(f"  TTA (Total Token Amplification):   {metrics['TTA']}")
            print(f"  Baseline Cost: ${metrics['baseline_cost_usd']} USD")
            print(f"  Swarm Cost:    ${metrics['swarm_cost_usd']} USD")
            
        # Save final report
        report_file = PROJECT_ROOT / "config" / "benchmark_report.json"
        with open(report_file, "w", encoding="utf-8") as f:
            json.dump(results, f, indent=2)
        print(f"\n📊 Benchmark complete! Report saved to {report_file}")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Run SWARM token usage benchmark")
    parser.add_argument("--tasks-file", default="docs/swebench_pallets_flask_tasks.json", help="Path to SWE-bench tasks JSON file")
    parser.add_argument("--task-id", help="Filter by a specific instance_id")
    parser.add_argument("--limit", type=int, default=1, help="Max tasks to execute")
    parser.add_argument("--strategy", default="role-based", help="Provider routing strategy")
    
    args = parser.parse_args()
    
    bench = SwarmBenchmark(
        tasks_file=args.tasks_file,
        task_id=args.task_id,
        limit=args.limit,
        strategy=args.strategy
    )
    bench.execute()
