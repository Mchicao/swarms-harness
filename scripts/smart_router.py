#!/usr/bin/env python3
"""Configurable SWARMS provider router.

The router keeps provider choice outside the coordinator model:
- explicit prompt directives win;
- user preferences shape cost/quality tradeoffs;
- capability cards and role rules provide deterministic routing;
- health data from scout_limits.yaml can suppress unhealthy providers.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any

try:
    import yaml
except Exception:  # pragma: no cover - PyYAML is expected in the repo env.
    yaml = None


PROJECT_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CONFIG = PROJECT_ROOT / "config" / "swarm_router.json"
LOCAL_CONFIG = PROJECT_ROOT / "config" / "swarm_router.local.json"
DEFAULT_LIMITS = PROJECT_ROOT / "config" / "swarm_limits.yaml"
DEFAULT_METRICS = PROJECT_ROOT / "config" / "swarm_metrics.json"
LEGACY_TASK_FILE = PROJECT_ROOT / ".agent" / "tasks_singularity.md"

DIRECTIVE_RE = re.compile(r"\[\[route:([a-zA-Z0-9_.:/-]+)\]\]|(?:^|\s)@([a-zA-Z0-9_.:/-]+)")
TAG_RE = re.compile(r"\[([a-zA-Z0-9_.:/-]+)\]")


def _load_json(path: Path, default: Any) -> Any:
    if not path.exists():
        return default
    with path.open(encoding="utf-8") as f:
        return json.load(f)


def _strip_comment_keys(obj: Any) -> Any:
    if isinstance(obj, dict):
        return {k: _strip_comment_keys(v) for k, v in obj.items() if not str(k).startswith("_")}
    if isinstance(obj, list):
        return [_strip_comment_keys(v) for v in obj]
    return obj


def load_config(path: Path | None = None) -> dict[str, Any]:
    if path is None:
        path = LOCAL_CONFIG if LOCAL_CONFIG.exists() else DEFAULT_CONFIG
    return _strip_comment_keys(_load_json(path, {}))


def load_limits(path: Path = DEFAULT_LIMITS) -> dict[str, Any]:
    if not path.exists() or yaml is None:
        return {}
    with path.open(encoding="utf-8") as f:
        return yaml.safe_load(f) or {}


def load_metrics(path: Path = DEFAULT_METRICS) -> dict[str, Any]:
    return _load_json(path, {})


def normalize_name(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", "", value.lower())


def extract_role(task_raw: str) -> str:
    for tag in TAG_RE.findall(task_raw):
        n = normalize_name(tag)
        if n not in {"x", "done", "route"}:
            return n
    return "general"


def extract_directive(task_raw: str, aliases: dict[str, str]) -> tuple[str | None, str | None]:
    matches = []
    for match in DIRECTIVE_RE.finditer(task_raw):
        name = match.group(1) or match.group(2)
        if name:
            matches.append(name)
    resolved = []
    for name in matches:
        key = normalize_name(name)
        route = aliases.get(key) or aliases.get(name.lower()) or name
        if route not in resolved:
            resolved.append(route)
    if len(resolved) == 1:
        return resolved[0], "directive"
    if len(resolved) > 1:
        return None, "ambiguous_directive"
    return None, None


def strip_directives(task_raw: str) -> str:
    return DIRECTIVE_RE.sub(" ", task_raw).strip()


def task_signature(task_raw: str, role: str) -> str:
    clean = re.sub(r"\s+", " ", strip_directives(task_raw).lower()).strip()
    return hashlib.sha256(f"{role}|{clean}".encode()).hexdigest()


def provider_is_healthy(provider: dict[str, Any], limits: dict[str, Any]) -> bool:
    health_key = provider.get("health_key")
    if not health_key:
        return True
    status = limits.get(health_key, {})
    if isinstance(status, dict):
        return status.get("status", "ok") == "ok"
    return True


def role_bonus(provider: dict[str, Any], role: str, config: dict[str, Any]) -> float:
    role_routes = config.get("role_routes", {})
    route = role_routes.get(role) or role_routes.get("general")
    if not route:
        return 0.0
    return 0.35 if provider.get("id") == route else 0.0


def text_match_score(provider: dict[str, Any], task_raw: str) -> float:
    text = task_raw.lower()
    strengths = provider.get("strengths", [])
    weaknesses = provider.get("weaknesses", [])
    score = 0.0
    for token in strengths:
        if str(token).lower() in text:
            score += 0.08
    for token in weaknesses:
        if str(token).lower() in text:
            score -= 0.12
    return score


def historical_score(provider: dict[str, Any], role: str, metrics: dict[str, Any]) -> float:
    metric_key = provider.get("metric_key")
    if not metric_key:
        return 0.0
    by_model = metrics.get("by_model", {})
    model_stats = by_model.get(metric_key, {})
    if not model_stats:
        return 0.0
    success_rate = float(model_stats.get("success_rate", 50)) / 100.0
    role_stats = model_stats.get("by_task_type", {}).get(role, {})
    role_total = role_stats.get("success", 0) + role_stats.get("failures", 0)
    if role_total:
        success_rate = (role_stats.get("success", 0) / role_total + success_rate) / 2
    return (success_rate - 0.5) * 0.4


def candidate_score(
    provider: dict[str, Any],
    task_raw: str,
    role: str,
    config: dict[str, Any],
    metrics: dict[str, Any],
) -> float:
    prefs = config.get("preferences", {})
    quality_weight = float(prefs.get("quality_weight", 0.55))
    cost_weight = float(prefs.get("cost_weight", 0.35))
    quota_weight = float(prefs.get("quota_saving_weight", 0.10))

    quality = float(provider.get("quality", 0.5))
    cost = float(provider.get("relative_cost", 1.0))
    plan_pressure = float(provider.get("scarcity", 0.5))
    max_cost = max(float(p.get("relative_cost", 1.0)) for p in config.get("providers", {}).values()) or 1.0

    score = quality * quality_weight
    score -= (cost / max_cost) * cost_weight
    score -= plan_pressure * quota_weight
    score += role_bonus(provider, role, config)
    score += text_match_score(provider, task_raw)
    score += historical_score(provider, role, metrics)
    return score


def choose_route(
    task_raw: str,
    strategy: str = "auto",
    config_path: Path | None = None,
    limits_path: Path = DEFAULT_LIMITS,
    metrics_path: Path = DEFAULT_METRICS,
) -> dict[str, Any]:
    config = load_config(config_path)
    limits = load_limits(limits_path)
    metrics = load_metrics(metrics_path)
    providers = config.get("providers", {})
    aliases = {normalize_name(k): v for k, v in config.get("aliases", {}).items()}

    role = extract_role(task_raw)
    directive_route, directive_reason = extract_directive(task_raw, aliases)
    if directive_route and directive_route in providers:
        provider = dict(providers[directive_route])
        provider.update(
            {
                "id": directive_route,
                "routing_method": "directive",
                "routing_reason": f"explicit route directive selected {directive_route}",
                "task_role": role,
                "task_signature": task_signature(task_raw, role),
            }
        )
        return provider

    if strategy in {"mock-only", "glm-only", "gemini-only", "codex-only"}:
        route = {
            "mock-only": "mock",
            "glm-only": "glm52",
            "gemini-only": "gemini_flash",
            "codex-only": "codex",
        }[strategy]
        if route not in providers:
            route = config.get("fallback_route", "mock")
        provider = dict(providers[route])
        provider.update({"id": route, "routing_method": strategy, "routing_reason": strategy, "task_role": role})
        return provider

    role_route = config.get("role_routes", {}).get(role)
    if strategy == "role-based" and role_route in providers:
        provider = dict(providers[role_route])
        provider.update(
            {"id": role_route, "routing_method": "role-based", "routing_reason": f"role {role}", "task_role": role}
        )
        return provider

    viable = []
    for route_id, provider in providers.items():
        if not provider.get("enabled", True):
            continue
        if not provider_is_healthy(provider, limits):
            continue
        p = dict(provider)
        p["id"] = route_id
        p["score"] = round(candidate_score(p, task_raw, role, config, metrics), 4)
        viable.append(p)

    if not viable:
        fallback = config.get("fallback_route", "glm52")
        provider = dict(providers.get(fallback) or next(iter(providers.values())))
        provider.update(
            {
                "id": fallback,
                "routing_method": "fallback",
                "routing_reason": "no viable healthy provider",
                "task_role": role,
            }
        )
        return provider

    chosen = max(viable, key=lambda p: (p["score"], -float(p.get("relative_cost", 1.0))))
    chosen["routing_method"] = "auto"
    if directive_reason == "ambiguous_directive":
        chosen["routing_reason"] = "ambiguous directive ignored; auto selected best configured score"
    else:
        chosen["routing_reason"] = "best configured score after preferences, role, health and history"
    chosen["task_role"] = role
    chosen["task_signature"] = task_signature(task_raw, role)
    chosen["routing_scores"] = {p["id"]: p["score"] for p in viable}
    return chosen


def provider_for_powershell(route: dict[str, Any]) -> dict[str, Any]:
    return {
        "Provider": route["provider"],
        "Model": route["model"],
        "CanonicalModel": route.get("canonical_model") or route["model"],
        "Wrapper": route["wrapper"],
        "ApiKeyEnv": route.get("api_key_env"),
        "RouteId": route["id"],
        "RoutingMethod": route.get("routing_method"),
        "RoutingReason": route.get("routing_reason"),
        "TaskRole": route.get("task_role"),
        "Score": route.get("score"),
    }


def route_tasks(task_file: Path = LEGACY_TASK_FILE, config_path: Path | None = None) -> None:
    if not task_file.exists():
        print(f"{task_file} not found.")
        return
    lines = task_file.read_text(encoding="utf-8").splitlines(keepends=True)
    new_lines = []
    for line in lines:
        if re.search(r"-\s*\[\s*\]", line) and "[ROUTE:" not in line:
            route = choose_route(line, config_path=config_path)
            new_lines.append(line.replace("- [ ]", f"- [ ] [ROUTE:{route['id']}]", 1))
        else:
            new_lines.append(line)
    task_file.write_text("".join(new_lines), encoding="utf-8")
    print(f"tasks routed in {task_file}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Route a SWARMS task to a provider")
    parser.add_argument("--task", help="Raw task text")
    parser.add_argument("--task-file", type=Path, help="Legacy task file to annotate")
    parser.add_argument("--strategy", default="auto")
    parser.add_argument("--format", choices=["json", "powershell"], default="json")
    parser.add_argument("--config", type=Path, help="Router configuration JSON")
    args = parser.parse_args()

    if args.task_file:
        route_tasks(args.task_file, config_path=args.config)
        return 0
    if not args.task:
        parser.error("--task is required unless --task-file is used")

    route = choose_route(args.task, strategy=args.strategy, config_path=args.config)
    payload = provider_for_powershell(route) if args.format == "powershell" else route
    print(json.dumps(payload, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
