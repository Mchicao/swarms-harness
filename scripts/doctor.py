#!/usr/bin/env python3
"""Offline health check for the open-source SWARMS distribution."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parent.parent


def ok(message: str) -> None:
    print(f"[OK] {message}")


def fail(message: str) -> None:
    print(f"[FAIL] {message}")


def warn(message: str) -> None:
    print(f"[WARN] {message}")


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as f:
        return json.load(f)


def run(cmd: list[str], cwd: Path = PROJECT_ROOT) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, cwd=cwd, text=True, capture_output=True, timeout=120)


def check_python() -> bool:
    if sys.version_info < (3, 10):
        fail(f"Python 3.10+ required, found {sys.version.split()[0]}")
        return False
    ok(f"Python {sys.version.split()[0]}")
    return True


def check_git() -> bool:
    if not shutil.which("git"):
        fail("git not found on PATH")
        return False
    result = run(["git", "--version"])
    if result.returncode != 0:
        fail("git is not available")
        return False
    ok("git available")
    return True


def check_legacy_powershell_parser() -> bool:
    ps = shutil.which("pwsh") or shutil.which("powershell")
    if not ps:
        warn("PowerShell not found; legacy parallel_swarm.ps1 adapter unavailable")
        return True
    script = PROJECT_ROOT / "scripts" / "parallel_swarm.ps1"
    command = (
        "$tokens=$null; $errors=$null; "
        f"[System.Management.Automation.Language.Parser]::ParseFile('{script}', [ref]$tokens, [ref]$errors) > $null; "
        "if ($errors.Count) { $errors | Format-List *; exit 1 }"
    )
    result = run([ps, "-NoProfile", "-Command", command])
    if result.returncode != 0:
        warn("legacy parallel_swarm.ps1 has PowerShell parse errors")
        print(result.stdout + result.stderr)
        return True
    ok("legacy PowerShell adapter parses")
    return True


def check_public_cli() -> bool:
    commands = [
        [sys.executable, "scripts/swarm.py", "review", "--plan", "docs/workflow_plan_example.json"],
        [
            sys.executable,
            "scripts/swarm.py",
            "dry-run",
            "--plan",
            "docs/workflow_plan_example.json",
            "--run-id",
            "doctor-dry-run",
            "--force",
        ],
    ]
    for command in commands:
        result = run(command)
        if result.returncode != 0:
            fail(f"public CLI failed: {' '.join(command)}")
            print(result.stdout + result.stderr)
            return False
    ok("public swarm CLI works")
    return True


def check_router_config() -> bool:
    path = PROJECT_ROOT / "config" / "swarm_router.json"
    try:
        config = load_json(path)
    except Exception as exc:
        fail(f"cannot read {path}: {exc}")
        return False
    providers = config.get("providers", {})
    mock = providers.get("mock", {})
    if not mock.get("enabled"):
        fail("default config must enable mock provider")
        return False
    enabled_real = [name for name, provider in providers.items() if name != "mock" and provider.get("enabled", True)]
    if enabled_real:
        fail(f"default config enables real providers: {', '.join(enabled_real)}")
        return False
    ok("default router is offline-safe")
    return True


def check_mock_route() -> bool:
    result = run(
        [
            sys.executable,
            "scripts/smart_router.py",
            "--task",
            "- [ ] [backend] implement offline demo",
            "--strategy",
            "auto",
            "--format",
            "json",
        ]
    )
    if result.returncode != 0:
        fail("smart_router.py failed")
        print(result.stdout + result.stderr)
        return False
    route = json.loads(result.stdout)
    if route.get("id") != "mock":
        fail(f"default route should be mock, got {route.get('id')}")
        return False
    ok("default router selects mock")
    return True


def check_secret_hygiene() -> bool:
    risky_files = [
        PROJECT_ROOT / ".env",
        PROJECT_ROOT / "config" / "swarm_router.local.json",
        PROJECT_ROOT / "config" / "model_pricing_catalog.local.json",
    ]
    for path in risky_files:
        if path.exists():
            warn(f"local-only file exists and must not be committed: {path.relative_to(PROJECT_ROOT)}")
    patterns = ["api_key", "oauthToken", "refresh_token", "BEGIN PRIVATE KEY"]
    tracked_candidates = [
        PROJECT_ROOT / "README.md",
        PROJECT_ROOT / "config" / "swarm_router.json",
        PROJECT_ROOT / "config" / "model_pricing_catalog.json",
    ]
    for path in tracked_candidates:
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8", errors="ignore").lower()
        for pattern in patterns:
            if pattern.lower() in text and "api_key_env" not in text:
                fail(f"possible secret marker in {path.relative_to(PROJECT_ROOT)}: {pattern}")
                return False
    ok("basic secret hygiene checks passed")
    return True


def main() -> int:
    checks = [
        check_python,
        check_git,
        check_public_cli,
        check_legacy_powershell_parser,
        check_router_config,
        check_mock_route,
        check_secret_hygiene,
    ]
    results = [check() for check in checks]
    if all(results):
        print("[OK] SWARMS doctor passed")
        return 0
    print("[FAIL] SWARMS doctor found issues")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
