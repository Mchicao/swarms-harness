#!/usr/bin/env python3
"""Synchronize the repo-canonical multi-provider orchestration skill."""

from __future__ import annotations

import argparse
import os
import shutil
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SOURCE = PROJECT_ROOT / "skills" / "multi-provider-agent-orchestration"
DEFAULT_TARGET = (
    Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local"))
    / "hermes"
    / "skills"
    / "autonomous-ai-agents"
    / "multi-provider-agent-orchestration"
)


def _files(root: Path) -> dict[Path, bytes]:
    if not root.exists():
        return {}
    return {
        path.relative_to(root): path.read_bytes()
        for path in root.rglob("*")
        if path.is_file()
    }


def sync_skill(source: Path = DEFAULT_SOURCE, target: Path = DEFAULT_TARGET, *, check: bool = False) -> bool:
    """Copy the canonical skill tree or report whether the target is identical."""
    source_files = _files(source)
    if not source_files or Path("SKILL.md") not in source_files:
        raise FileNotFoundError(f"Canonical skill is missing: {source}")

    target_files = _files(target)
    in_sync = source_files == target_files
    if check:
        return in_sync
    if in_sync:
        return True

    target.mkdir(parents=True, exist_ok=True)
    for relative in sorted(set(target_files) - set(source_files)):
        (target / relative).unlink()
    for relative, content in source_files.items():
        destination = target / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(content)

    for directory in sorted((path for path in target.rglob("*") if path.is_dir()), reverse=True):
        try:
            directory.rmdir()
        except OSError:
            pass
    return _files(source) == _files(target)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--target", type=Path, default=DEFAULT_TARGET)
    parser.add_argument("--check", action="store_true", help="Exit non-zero when the installed skill has drifted")
    args = parser.parse_args()

    synchronized = sync_skill(args.source, args.target, check=args.check)
    if synchronized:
        print(f"Skill synchronized: {args.target}")
        return 0
    print(f"Skill drift detected: {args.target}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
