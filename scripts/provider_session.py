"""Persist provider session identifiers used for bounded crash recovery."""

from __future__ import annotations

import json
import os
import time
from pathlib import Path
from typing import Any

RESUME_WINDOW_SECONDS = int(os.environ.get("SWARMS_SESSION_RESUME_WINDOW_SECONDS", "300"))


def write_provider_status(path: Path | None, **fields: Any) -> None:
    """Merge and atomically persist non-secret provider session state."""
    if path is None:
        return
    current: dict[str, Any] = {}
    try:
        current = json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        pass
    current.update(fields)
    current["provider_session_updated_unix_ms"] = int(time.time() * 1000)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(current, indent=2), encoding="utf-8")
    os.replace(temporary, path)


def load_fresh_provider_session(
    path: Path, *, now_ms: int | None = None, window_seconds: int = RESUME_WINDOW_SECONDS
) -> str | None:
    """Return an exact session ID only while its bounded recovery window is open."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        session_id = data["provider_session_id"]
        updated = int(data["provider_session_updated_unix_ms"])
    except (FileNotFoundError, KeyError, TypeError, ValueError, json.JSONDecodeError, OSError):
        return None
    now = int(time.time() * 1000) if now_ms is None else now_ms
    age = now - updated
    return session_id if isinstance(session_id, str) and session_id and 0 <= age <= window_seconds * 1000 else None
