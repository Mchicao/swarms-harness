"""Coordinate optional Skillshare and Rulesync synchronization."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

TARGET_ALIASES = {
    "claude": ["claudecode"],
    "codex": ["codexcli"],
    "opencode": ["opencode"],
    "agy": ["agentsmd", "agentsskills"],
    "gemini": ["geminicli"],
    "antigravity": ["antigravity"],
}
SUPPORTED_TARGETS = {
    "agentsmd",
    "agentsskills",
    "claudecode",
    "codexcli",
    "geminicli",
    "opencode",
    "antigravity",
}
DEFAULT_TARGETS = ["claude", "codex", "opencode", "agy", "gemini", "antigravity"]
SECRET_KEY = re.compile(r"(?:token|secret|password|api[_-]?key|authorization|header)", re.IGNORECASE)
ENV_REFERENCE = re.compile(r"^\$\{[A-Za-z_][A-Za-z0-9_]*\}$")


class ContextSyncError(RuntimeError):
    """Raised when coordinated context synchronization cannot complete safely."""


def _run(command: list[str], *, cwd: Path) -> dict[str, Any]:
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    return {"returncode": result.returncode, "stdout": result.stdout or "", "stderr": result.stderr or ""}


def _selected_targets(targets: list[str] | None) -> list[str]:
    requested = targets or DEFAULT_TARGETS
    expanded = [mapped for target in requested for mapped in TARGET_ALIASES.get(target, [target])]
    selected = list(dict.fromkeys(expanded))
    unsupported = [target for target in selected if target not in SUPPORTED_TARGETS]
    if unsupported:
        raise ContextSyncError(f"Unsupported context sync targets: {', '.join(unsupported)}")
    return selected


def _validate_mcp_source(path: Path) -> None:
    """SWARMS-CONTEXT-002: Reject credentials that Rulesync would copy literally."""
    try:
        source = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ContextSyncError(f"Invalid canonical MCP source {path}: {exc}") from exc

    def inspect(value: Any, key: str = "") -> None:
        if isinstance(value, dict):
            for child_key, child in value.items():
                inspect(child, str(child_key))
        elif isinstance(value, list):
            for child in value:
                inspect(child, key)
        elif isinstance(value, str):
            if "://" in value and re.search(r"://[^/@:]+:[^/@]+@", value):
                raise ContextSyncError("MCP source contains a literal credential in a URL")
            if SECRET_KEY.search(key) and value and not ENV_REFERENCE.fullmatch(value):
                raise ContextSyncError(f"MCP source contains a literal credential in {key!r}; use ${{ENV_VAR}}")

    inspect(source)


def sync_agent_context(workspace: Path, targets: list[str] | None = None) -> dict[str, Any]:
    """Synchronize skills plus canonical rules/MCP without exposing environment values."""
    workspace = workspace.resolve()
    selected = _selected_targets(targets)
    rulesync = workspace / ".rulesync"
    mcp_source = rulesync / "mcp.json"
    if not (rulesync / "rules").is_dir() or not mcp_source.is_file():
        raise ContextSyncError(f"Missing canonical Rulesync source under: {rulesync}")
    _validate_mcp_source(mcp_source)

    rulesync_command = [
        "rulesync",
        "generate",
        "--targets",
        ",".join(selected),
        "--features",
        "rules,mcp,subagents,skills",
        "--input-root",
        str(workspace),
        "--output-roots",
        str(workspace),
        "--silent",
    ]
    commands = [
        ["skillshare", "sync", "--all", "--json", "--dry-run"],
        [*rulesync_command, "--dry-run"],
        ["skillshare", "sync", "--all", "--json"],
        rulesync_command,
    ]
    results = []
    for command in commands:
        outcome = _run(command, cwd=workspace)
        results.append({"tool": command[0], "returncode": outcome["returncode"]})
        if outcome["returncode"] != 0:
            detail = (outcome["stderr"] or outcome["stdout"])[-1000:]
            raise ContextSyncError(f"{command[0]} failed with exit {outcome['returncode']}: {detail}")
    return {
        "success": True,
        "targets": selected,
        "features": ["skills", "rules", "agentsmd", "subagents", "mcp"],
        "scope": "rulesync-project; skillshare-configured-targets",
        "previewed": True,
        "results": results,
    }


def main(argv: list[str] | None = None) -> int:
    """SWARMS-CONTEXT-CLI-001: Bridge the public Rust CLI to context sync."""
    args = list(sys.argv[1:] if argv is None else argv)
    if len(args) != 2:
        print("usage: python -m scripts.context_sync WORKSPACE TARGETS", file=sys.stderr)
        return 2
    try:
        report = sync_agent_context(Path(args[0]), [target for target in args[1].split(",") if target])
    except ContextSyncError as exc:
        print(str(exc), file=sys.stderr)
        return 2
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
