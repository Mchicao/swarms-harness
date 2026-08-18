"""Connection inventory and migration-risk analysis."""

from __future__ import annotations

import json
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any


def _load(value: Any) -> Any:
    if isinstance(value, (str, Path)):
        path = Path(value)
        if path.exists():
            return json.loads(path.read_text(encoding="utf-8"))
    return value


def _items(value: Any) -> list[Mapping[str, Any]]:
    if isinstance(value, Mapping):
        return [value]
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes)):
        return [item for item in value if isinstance(item, Mapping)]
    return []


def _first(item: Mapping[str, Any], *keys: str, default: Any = None) -> Any:
    for key in keys:
        value = item.get(key)
        if value not in (None, ""):
            return value
    return default


def _safe_source(item: Mapping[str, Any]) -> str | None:
    value = _first(item, "source", "server", "host", "database", "path", "url")
    if isinstance(value, Mapping):
        value = _first(value, "name", "host", "database", "path", "url")
    if value is None:
        return None
    text = str(value)
    for marker in ("password=", "pwd=", "token=", "secret=", "apikey="):
        if marker in text.lower():
            text = text[: text.lower().index(marker)].rstrip("; ,") + "[redacted]"
    return text


def analyze_connections(report: Any) -> dict[str, Any]:
    """Return a deterministic connection inventory without exposing credentials."""
    payload = _load(report)
    if not isinstance(payload, Mapping):
        payload = {}
    raw = _first(payload, "connections", "data_sources", "datasources", "sources", default=[])
    connections: list[dict[str, Any]] = []
    for index, item in enumerate(_items(raw), start=1):
        identifier = str(_first(item, "id", "name", "caption", default=f"connection-{index}"))
        kind = str(_first(item, "type", "class", "connector", "provider", default="unknown"))
        credential = str(_first(item, "credential_status", "auth", "authentication", default="unknown"))
        evidence = _first(item, "evidence", "source_ref", "path", default=f"$.connections[{index - 1}]")
        connections.append(
            {
                "id": identifier,
                "name": str(_first(item, "name", "caption", default=identifier)),
                "type": kind,
                "source": _safe_source(item),
                "credential_status": credential,
                "evidence": [str(evidence)],
            }
        )
    connections.sort(key=lambda item: item["id"])
    risks = [
        {
            "id": "connection-unknown-type",
            "message": f"{len([item for item in connections if item['type'] == 'unknown'])} connection(s) have no connector type.",
            "severity": "medium",
            "evidence": [item["evidence"][0] for item in connections if item["type"] == "unknown"],
        }
    ]
    risks = [risk for risk in risks if risk["evidence"]]
    return {
        "connections": connections,
        "summary": {
            "total": len(connections),
            "known_type": sum(item["type"] != "unknown" for item in connections),
            "credential_status_unknown": sum(item["credential_status"] == "unknown" for item in connections),
        },
        "risks": risks,
    }


class ConnectionAnalyzer:
    """Small object wrapper for callers that prefer an analyzer instance."""

    def analyze(self, report: Any) -> dict[str, Any]:
        return analyze_connections(report)


analyze = analyze_connections

