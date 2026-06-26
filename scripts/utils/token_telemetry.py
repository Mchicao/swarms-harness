"""
SWARM token telemetry.

This module records append-only usage events and prices them from a versioned
catalog. It keeps the original flat fields for compatibility while adding
structured usage/cost details for benchmark reporting.
"""

from __future__ import annotations

import json
import os
import re
import uuid
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

TELEMETRY_FILE = Path(os.environ.get("SWARM_TELEMETRY_FILE", ".agent/traces/telemetry.jsonl"))
CATALOG_FILE = Path("config/model_pricing_catalog.json")
LOCAL_CATALOG_FILE = Path("config/model_pricing_catalog.local.json")
CATALOG_VERSION = "2026-06-19.oss-safe-default"

DEFAULT_CATALOG: dict[str, Any] = {
    "schema_version": 1,
    "catalog_version": CATALOG_VERSION,
    "currency": "USD",
    "effective_at": "2026-06-19",
    "models": {
        "mock-worker": {
            "provider": "mock",
            "plan": "offline",
            "input_per_1m": 0.0,
            "cache_read_input_per_1m": 0.0,
            "cache_write_input_per_1m": 0.0,
            "output_per_1m": 0.0,
            "reasoning_output_per_1m": 0.0,
            "pricing_status": "offline_mock",
        },
        "gpt-5.5-codex": {
            "provider": "codex_cli",
            "plan": "scarce_expensive_plan",
            "input_per_1m": 0.0,
            "cache_read_input_per_1m": 0.0,
            "cache_write_input_per_1m": 0.0,
            "output_per_1m": 0.0,
            "reasoning_output_per_1m": 0.0,
            "pricing_status": "disabled_by_default",
        },
        "gemini-3.5-flash": {
            "provider": "antigravity_cli",
            "plan": "user_plan_quota",
            "input_per_1m": 0.0,
            "cache_read_input_per_1m": 0.0,
            "cache_write_input_per_1m": 0.0,
            "output_per_1m": 0.0,
            "reasoning_output_per_1m": 0.0,
            "pricing_status": "opaque_plan_quota",
        },
        "glm-5.2": {
            "provider": "opencode",
            "plan": "user_configured",
            "input_per_1m": 0.0,
            "cache_read_input_per_1m": 0.0,
            "cache_write_input_per_1m": 0.0,
            "output_per_1m": 0.0,
            "reasoning_output_per_1m": 0.0,
            "pricing_status": "user_must_configure",
        },
    },
}


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _as_int(value: Any) -> int:
    try:
        return max(0, int(value or 0))
    except (TypeError, ValueError):
        return 0


def _load_catalog() -> dict[str, Any]:
    if LOCAL_CATALOG_FILE.exists():
        try:
            with open(LOCAL_CATALOG_FILE, encoding="utf-8") as f:
                return json.load(f)
        except (OSError, json.JSONDecodeError):
            pass
    if CATALOG_FILE.exists():
        try:
            with open(CATALOG_FILE, encoding="utf-8") as f:
                return json.load(f)
        except (OSError, json.JSONDecodeError):
            return DEFAULT_CATALOG
    return DEFAULT_CATALOG


def get_catalog_version() -> str:
    return str(_load_catalog().get("catalog_version", CATALOG_VERSION))


def get_pricing(model: str) -> dict[str, Any] | None:
    model_l = (model or "").lower()
    models = _load_catalog().get("models", {})
    for key, val in models.items():
        if key.lower() in model_l:
            return dict(val)
    return None


def calculate_cost_details(model: str, usage: dict[str, int]) -> dict[str, Any]:
    pricing = get_pricing(model)
    if pricing is None:
        return {
            "input": None,
            "cache_read_input_tokens": None,
            "cache_write_input_tokens": None,
            "output": None,
            "reasoning_output_tokens": None,
            "total": None,
            "currency": "USD",
            "pricing_status": "unknown",
            "pricing_source": "missing",
        }

    input_tokens = max(0, usage["input"] - usage["cache_read_input_tokens"])
    parts = {
        "input": input_tokens * float(pricing.get("input_per_1m", 0.0)) / 1_000_000.0,
        "cache_read_input_tokens": usage["cache_read_input_tokens"]
        * float(pricing.get("cache_read_input_per_1m", 0.0))
        / 1_000_000.0,
        "cache_write_input_tokens": usage["cache_write_input_tokens"]
        * float(pricing.get("cache_write_input_per_1m", 0.0))
        / 1_000_000.0,
        "output": usage["output"] * float(pricing.get("output_per_1m", 0.0)) / 1_000_000.0,
        "reasoning_output_tokens": usage["reasoning_output_tokens"]
        * float(pricing.get("reasoning_output_per_1m", pricing.get("output_per_1m", 0.0)))
        / 1_000_000.0,
    }
    return {
        **{k: round(v, 8) for k, v in parts.items()},
        "total": round(sum(parts.values()), 8),
        "currency": _load_catalog().get("currency", "USD"),
        "pricing_status": pricing.get("pricing_status", "configured"),
        "pricing_source": "catalog",
    }


def calculate_cost(model: str, input_t: int, output_t: int, cached_t: int = 0) -> float | None:
    usage = normalize_usage(input_t, cached_t, 0, output_t, 0)
    return calculate_cost_details(model, usage)["total"]


def normalize_usage(
    input_tokens: int = 0,
    cache_read_tokens: int = 0,
    cache_write_tokens: int = 0,
    output_tokens: int = 0,
    reasoning_tokens: int = 0,
) -> dict[str, int]:
    usage = {
        "input": _as_int(input_tokens),
        "cache_read_input_tokens": _as_int(cache_read_tokens),
        "cache_write_input_tokens": _as_int(cache_write_tokens),
        "output": _as_int(output_tokens),
        "reasoning_output_tokens": _as_int(reasoning_tokens),
    }
    usage["cached"] = usage["cache_read_input_tokens"]
    usage["reasoning"] = usage["reasoning_output_tokens"]
    return usage


def usage_has_tokens(usage: dict[str, int]) -> bool:
    return any(int(v) > 0 for v in usage.values())


def normalize_usage_source(source: str, usage: dict[str, int]) -> str:
    if usage_has_tokens(usage):
        return source if source and source != "estimated" else "tokenizer_estimated"
    return "missing"


def record_event(
    run_id: str,
    benchmark_id: str,
    phase: str,
    provider: str,
    model: str,
    role: str,
    task_id: str,
    input_tokens: int = 0,
    cache_read_tokens: int = 0,
    cache_write_tokens: int = 0,
    output_tokens: int = 0,
    reasoning_tokens: int = 0,
    usage_source: str = "missing",
    success: bool = True,
    started_at: str | None = None,
    ended_at: str | None = None,
    attempt: int = 1,
    error_type: str | None = None,
    http_status: int | None = None,
    request_id: str | None = None,
    parent_event_id: str | None = None,
    route_id: str | None = None,
    routing_method: str | None = None,
    routing_reason: str | None = None,
) -> dict[str, Any]:
    TELEMETRY_FILE.parent.mkdir(parents=True, exist_ok=True)
    usage = normalize_usage(input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, reasoning_tokens)
    source = normalize_usage_source(usage_source, usage)
    cost_details = calculate_cost_details(model, usage)
    cost_usd = cost_details["total"]
    event = {
        "schema_version": "2.0",
        "event_id": str(uuid.uuid4()),
        "parent_event_id": parent_event_id,
        "run_id": run_id,
        "benchmark_id": benchmark_id,
        "phase": phase,
        "provider": provider,
        "model": model,
        "role": role,
        "task_id": task_id,
        "route_id": route_id,
        "routing_method": routing_method,
        "routing_reason": routing_reason,
        "attempt": attempt,
        "input_tokens": usage["input"],
        "cache_read_tokens": usage["cache_read_input_tokens"],
        "cache_write_tokens": usage["cache_write_input_tokens"],
        "output_tokens": usage["output"],
        "reasoning_tokens": usage["reasoning_output_tokens"],
        "usage_details": usage,
        "cost_usd": cost_usd,
        "cost_details": cost_details,
        "usage_source": source,
        "cost_source": cost_details["pricing_source"],
        "pricing_catalog_version": get_catalog_version(),
        "success": bool(success),
        "error_type": error_type,
        "http_status": http_status,
        "request_id": request_id,
        "started_at": started_at or _now(),
        "ended_at": ended_at or _now(),
    }
    with open(TELEMETRY_FILE, "a", encoding="utf-8") as f:
        f.write(json.dumps(event, ensure_ascii=True) + "\n")
    return event


def _get_nested(obj: dict[str, Any], *keys: str) -> Any:
    cur: Any = obj
    for key in keys:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key)
    return cur


def parse_openai_like_usage(data: dict[str, Any]) -> dict[str, int]:
    usage = data.get("usage", data)
    details = usage.get("prompt_tokens_details") or {}
    completion_details = usage.get("completion_tokens_details") or {}
    return normalize_usage(
        input_tokens=usage.get("prompt_tokens") or usage.get("input_tokens"),
        cache_read_tokens=details.get("cached_tokens") or usage.get("cached_input_tokens"),
        cache_write_tokens=details.get("cache_write_tokens") or details.get("cache_creation_input_tokens"),
        output_tokens=usage.get("completion_tokens") or usage.get("output_tokens"),
        reasoning_tokens=completion_details.get("reasoning_tokens") or usage.get("reasoning_tokens"),
    )


def parse_zai_usage_object(usage: Any) -> dict[str, int]:
    if usage is None:
        return normalize_usage()
    if not isinstance(usage, dict):
        usage = {
            "prompt_tokens": getattr(usage, "prompt_tokens", 0),
            "completion_tokens": getattr(usage, "completion_tokens", 0),
            "prompt_tokens_details": getattr(usage, "prompt_tokens_details", None),
            "completion_tokens_details": getattr(usage, "completion_tokens_details", None),
        }
    details = usage.get("prompt_tokens_details") or {}
    if not isinstance(details, dict):
        details = {
            "cached_tokens": getattr(details, "cached_tokens", 0),
            "cache_write_tokens": getattr(details, "cache_write_tokens", 0),
        }
    c_details = usage.get("completion_tokens_details") or {}
    if not isinstance(c_details, dict):
        c_details = {"reasoning_tokens": getattr(c_details, "reasoning_tokens", 0)}
    return normalize_usage(
        input_tokens=usage.get("prompt_tokens"),
        cache_read_tokens=details.get("cached_tokens"),
        cache_write_tokens=details.get("cache_write_tokens"),
        output_tokens=usage.get("completion_tokens"),
        reasoning_tokens=c_details.get("reasoning_tokens"),
    )


def parse_codex_log(log_path: Path) -> dict[str, int]:
    if not log_path.exists():
        return normalize_usage()
    latest = normalize_usage()
    try:
        with open(log_path, encoding="utf-8", errors="replace") as f:
            for line in f:
                if not line.strip():
                    continue
                try:
                    data = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if "usage" in data:
                    latest = parse_openai_like_usage(data)
                elif data.get("event") == "tokens":
                    latest = normalize_usage(
                        input_tokens=data.get("input_tokens", latest["input"]),
                        cache_read_tokens=data.get("cached_input_tokens", latest["cache_read_input_tokens"]),
                        output_tokens=data.get("output_tokens", latest["output"]),
                        reasoning_tokens=data.get("reasoning_tokens", latest["reasoning_output_tokens"]),
                    )
    except OSError:
        return normalize_usage()
    return latest


def parse_stdout_text(text: str) -> dict[str, int]:
    # Best-effort fallback only. Structured API usage should be preferred.
    prompt_match = re.search(r"(?:Prompt|Input|Context)\s*(?:Tokens|tokens)?\s*[:=]\s*(\d+)", text, re.I)
    completion_match = re.search(r"(?:Completion|Output|Generated)\s*(?:Tokens|tokens)?\s*[:=]\s*(\d+)", text, re.I)
    cached_match = re.search(r"(?:Cached|Cache Read)\s*(?:Tokens|tokens)?\s*[:=]\s*(\d+)", text, re.I)
    cache_write_match = re.search(r"(?:Cache Write|Cache Creation)\s*(?:Tokens|tokens)?\s*[:=]\s*(\d+)", text, re.I)
    reasoning_match = re.search(r"(?:Reasoning)\s*(?:Tokens|tokens)?\s*[:=]\s*(\d+)", text, re.I)
    return normalize_usage(
        input_tokens=prompt_match.group(1) if prompt_match else 0,
        cache_read_tokens=cached_match.group(1) if cached_match else 0,
        cache_write_tokens=cache_write_match.group(1) if cache_write_match else 0,
        output_tokens=completion_match.group(1) if completion_match else 0,
        reasoning_tokens=reasoning_match.group(1) if reasoning_match else 0,
    )


def parse_opencode_log(log_path: Path) -> dict[str, int]:
    usage = normalize_usage()
    if not log_path.exists():
        return usage
    for line in log_path.read_text(encoding="utf-8", errors="ignore").splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            data = json.loads(line)
        except json.JSONDecodeError:
            continue
        candidates = [data.get("usage"), data.get("tokens")]
        if isinstance(data.get("part"), dict):
            candidates.append(data["part"].get("tokens"))
        if isinstance(data.get("cost"), dict):
            candidates.append(data["cost"].get("usage"))
        if isinstance(data.get("message"), dict):
            candidates.append(data["message"].get("usage"))
        for candidate in candidates:
            if not isinstance(candidate, dict):
                continue
            parsed = parse_openai_like_usage(candidate)
            if not usage_has_tokens(parsed):
                cache = candidate.get("cache", {}) if isinstance(candidate.get("cache"), dict) else {}
                parsed = normalize_usage(
                    input_tokens=candidate.get("input", candidate.get("input_tokens", 0)),
                    cache_read_tokens=candidate.get(
                        "cached", candidate.get("cache_read_input_tokens", cache.get("read", 0))
                    ),
                    cache_write_tokens=candidate.get(
                        "cache_write", candidate.get("cache_write_input_tokens", cache.get("write", 0))
                    ),
                    output_tokens=candidate.get("output", candidate.get("output_tokens", 0)),
                    reasoning_tokens=candidate.get("reasoning", candidate.get("reasoning_output_tokens", 0)),
                )
            if usage_has_tokens(parsed):
                usage = parsed
    return usage


def iter_events(path: Path = TELEMETRY_FILE) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    events = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            if not line.strip():
                continue
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return events


def summarize_events(events: list[dict[str, Any]]) -> dict[str, Any]:
    def empty_bucket() -> dict[str, Any]:
        return {
            "events": 0,
            "success_events": 0,
            "missing_usage_events": 0,
            "input_tokens": 0,
            "cache_read_tokens": 0,
            "cache_write_tokens": 0,
            "output_tokens": 0,
            "reasoning_tokens": 0,
            "known_cost_usd": 0.0,
            "unknown_cost_events": 0,
        }

    totals = empty_bucket()
    grouped: dict[str, dict[str, Any]] = defaultdict(empty_bucket)
    for event in events:
        key = "|".join(
            [
                str(event.get("phase", "")),
                str(event.get("provider", "")),
                str(event.get("model", "")),
                str(event.get("role", "")),
            ]
        )
        for bucket in (totals, grouped[key]):
            bucket["events"] += 1
            bucket["success_events"] += 1 if event.get("success") else 0
            bucket["missing_usage_events"] += 1 if event.get("usage_source") == "missing" else 0
            bucket["input_tokens"] += _as_int(event.get("input_tokens"))
            bucket["cache_read_tokens"] += _as_int(event.get("cache_read_tokens"))
            bucket["cache_write_tokens"] += _as_int(event.get("cache_write_tokens"))
            bucket["output_tokens"] += _as_int(event.get("output_tokens"))
            bucket["reasoning_tokens"] += _as_int(event.get("reasoning_tokens"))
            if event.get("cost_usd") is None:
                bucket["unknown_cost_events"] += 1
            else:
                bucket["known_cost_usd"] += float(event.get("cost_usd", 0.0))
    return {"totals": totals, "by_phase_provider_model_role": dict(grouped)}


def summarize_run(run_id: str, path: Path = TELEMETRY_FILE) -> dict[str, Any]:
    events = [event for event in iter_events(path) if event.get("run_id") == run_id]
    return summarize_events(events)
