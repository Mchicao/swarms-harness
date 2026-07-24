"""Inventario local de agentes y rutas antes de despachar workers."""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path
from typing import Any

try:
    from .smart_router import load_config
except ImportError:  # pragma: no cover
    from smart_router import load_config

KNOWN_COMMANDS = {
    "agy": "antigravity CLI",
    "opencode": "OpenCode",
    "gemini": "Gemini CLI",
    "zcode": "Zcode",
    "codex": "Codex CLI",
    "claude": "Claude CLI",
    "kilo": "Kilo CLI",
    "hermes": "Hermes CLI",
}


def _command_for(provider: dict[str, Any], route: str) -> str | None:
    wrapper = provider.get("wrapper")
    if route == "mock" or wrapper == "mock":
        return None
    if wrapper == "opencode":
        return "opencode"
    if wrapper == "gemini":
        return "agy" if shutil.which("agy") else "gemini"
    if wrapper == "codex":
        return "codex"
    if wrapper == "kilo":
        return "kilo"
    if wrapper == "hermes":
        return "hermes"
    return None


def _auth_present(command: str | None) -> bool | None:
    """Return local auth evidence without making a model/API request."""
    if command == "opencode":
        try:
            result = subprocess.run([command, "auth", "list"], capture_output=True, text=True, timeout=10, check=False)
        except (OSError, subprocess.TimeoutExpired):
            return False
        output = f"{result.stdout}\n{result.stderr}".lower()
        return result.returncode == 0 and "credential" in output
    if command in {"agy", "gemini"}:
        return bool(os.environ.get("GOOGLE_API_KEY") or Path.home().joinpath(".gemini", "antigravity-cli").exists())
    if command:
        return None
    return True


def _status(*, enabled: bool, installed: bool, auth_present: bool | None, route: str) -> str:
    if not enabled:
        return "disabled"
    if route == "mock":
        return "ready"
    if not installed:
        return "missing_cli"
    if auth_present is False:
        return "missing_auth"
    return "unverified"


def _route_record(route: str, provider: dict[str, Any]) -> dict[str, Any]:
    command = _command_for(provider, route)
    installed = command is None or shutil.which(command) is not None
    auth_present = _auth_present(command) if installed else False
    enabled = bool(provider.get("enabled", False))
    status = _status(enabled=enabled, installed=installed, auth_present=auth_present, route=route)
    reasons = {
        "ready": "offline mock is executable without external credentials",
        "disabled": "route is disabled by local router policy",
        "missing_cli": f"required CLI is not on PATH: {command}",
        "missing_auth": f"no local authentication evidence for {command}",
        "unverified": "CLI and local auth evidence found; model probe still required",
    }
    return {
        "id": route,
        "provider": provider.get("provider", route),
        "model": provider.get("model", ""),
        "wrapper": provider.get("wrapper", ""),
        "command": command,
        "installed": installed,
        "auth_present": auth_present,
        "enabled": enabled,
        "status": status,
        "reason": reasons[status],
    }


def discover_agents(config_path: Path | None = None) -> dict[str, Any]:
    """Describe configured routes and detected local agent CLIs read-only."""
    config = load_config(config_path)
    routes = [_route_record(route, provider) for route, provider in config.get("providers", {}).items()]
    configured_commands = {record["command"] for record in routes if record["command"]}
    detected = [
        {
            "id": command,
            "name": name,
            "command": command,
            "path": shutil.which(command),
            "configured": command in configured_commands,
        }
        for command, name in KNOWN_COMMANDS.items()
        if shutil.which(command)
    ]
    return {
        "router": str(config_path) if config_path else "default+local",
        "routes": routes,
        "detected_clis": detected,
        "ready_routes": [record["id"] for record in routes if record["status"] == "ready"],
        "unverified_routes": [record["id"] for record in routes if record["status"] == "unverified"],
    }


def route_findings(report: dict[str, Any], routes: set[str]) -> list[dict[str, str]]:
    """Return actionable findings for routes used by a plan."""
    records = {record["id"]: record for record in report["routes"]}
    findings = []
    for route in sorted(routes):
        record = records.get(route)
        if record is None:
            findings.append({"code": "unknown_route", "route": route})
        elif record["status"] not in {"ready", "disabled"}:
            findings.append({"code": f"agent_{record['status']}", "route": route})
    return findings


def format_text(report: dict[str, Any]) -> str:
    lines = ["SWARMS agent preflight", f"router: {report['router']}", "routes:"]
    for record in report["routes"]:
        lines.append(
            f"- {record['id']}: {record['status']} | model={record['model']} | "
            f"command={record['command'] or 'builtin'} | {record['reason']}"
        )
    lines.append("detected CLIs:")
    lines.extend(f"- {item['command']}: {item['path']}" for item in report["detected_clis"])
    return "\n".join(lines)
