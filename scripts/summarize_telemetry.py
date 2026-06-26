#!/usr/bin/env python3
"""Summarize SWARMS telemetry events."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.append(str(PROJECT_ROOT))

from scripts.utils.token_telemetry import TELEMETRY_FILE, iter_events, summarize_events


def _round_cost(value: float) -> float:
    return round(float(value), 6)


def main() -> int:
    parser = argparse.ArgumentParser(description="Summarize SWARMS token telemetry")
    parser.add_argument("--run-id", help="Filter to one run_id")
    parser.add_argument("--benchmark-id", help="Filter to one benchmark_id")
    parser.add_argument("--file", default=str(TELEMETRY_FILE), help="Telemetry JSONL path")
    parser.add_argument("--format", choices=["json", "table"], default="table")
    args = parser.parse_args()

    events = iter_events(Path(args.file))
    if args.run_id:
        events = [event for event in events if event.get("run_id") == args.run_id]
    if args.benchmark_id:
        events = [event for event in events if event.get("benchmark_id") == args.benchmark_id]

    summary = summarize_events(events)
    if args.format == "json":
        print(json.dumps(summary, indent=2))
        return 0

    totals = summary["totals"]
    print("SWARMS telemetry summary")
    print(f"events: {totals['events']} success: {totals['success_events']} missing_usage: {totals['missing_usage_events']}")
    print(
        "tokens: "
        f"input={totals['input_tokens']} "
        f"cache_read={totals['cache_read_tokens']} "
        f"cache_write={totals['cache_write_tokens']} "
        f"output={totals['output_tokens']} "
        f"reasoning={totals['reasoning_tokens']}"
    )
    print(f"known_cost_usd: {_round_cost(totals['known_cost_usd'])} unknown_cost_events: {totals['unknown_cost_events']}")
    print("")
    print("phase | provider | model | role | events | missing | cost")
    for key, bucket in sorted(
        summary["by_phase_provider_model_role"].items(),
        key=lambda item: item[1]["known_cost_usd"],
        reverse=True,
    ):
        phase, provider, model, role = key.split("|")
        print(
            f"{phase} | {provider} | {model} | {role} | "
            f"{bucket['events']} | {bucket['missing_usage_events']} | {_round_cost(bucket['known_cost_usd'])}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
