#!/usr/bin/env python3
"""SWARMS worker for Kilo CLI models such as Tencent HY3 free."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

DEFAULT_MODEL = os.environ.get("KILO_MODEL", "kilo/tencent/hy3:free")
DEFAULT_TIMEOUT = int(os.environ.get("KILO_TIMEOUT", "600"))
KILO_BIN = os.environ.get("KILO_BIN", "kilo")


def kilo_complete(
    prompt: str,
    *,
    model: str = DEFAULT_MODEL,
    timeout: int = DEFAULT_TIMEOUT,
    tools_policy: str = "none",
) -> str:
    """Run Kilo once, avoiding automatic permission approval by default."""
    tmp_dir = Path(tempfile.mkdtemp(prefix="swarms_kilo_"))
    prompt_file = tmp_dir / "prompt.md"
    prompt_file.write_text(prompt, encoding="utf-8")
    # Kilo uses a SQLite database below XDG_DATA_HOME. Isolate concurrent tasks.
    environment = os.environ.copy()
    environment["XDG_DATA_HOME"] = str(tmp_dir / "kilo-data")
    command = [KILO_BIN, "run", "-m", model, "-f", str(prompt_file)]
    command.append("--auto" if tools_policy == "full" else "--pure")
    command.append("Complete the task described in the attached file. Return only the required result.")
    try:
        result = subprocess.run(
            command,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=tmp_dir,
            encoding="utf-8",
            errors="replace",
            env=environment,
        )
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)
    if result.returncode != 0:
        raise RuntimeError(f"Kilo exited {result.returncode}. stderr={result.stderr[:500]!r}")
    output = (result.stdout or "").strip()
    if not output:
        raise RuntimeError(f"Kilo produced no stdout. stderr={result.stderr[:300]!r}")
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description="Kilo CLI worker for SWARMS.")
    parser.add_argument("--prompt", type=Path, required=True)
    parser.add_argument("--status", type=Path)
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    parser.add_argument("--tools-policy", default="none", choices=["none", "full"])
    args = parser.parse_args()
    try:
        output = kilo_complete(
            args.prompt.read_text(encoding="utf-8", errors="replace"),
            model=args.model,
            timeout=args.timeout,
            tools_policy=args.tools_policy,
        )
        sys.stdout.buffer.write(output.encode("utf-8", errors="replace") + b"\n")
        if args.status:
            args.status.write_text(
                json.dumps({"success": True, "provider": "kilo", "model": args.model}), encoding="utf-8"
            )
        return 0
    except Exception as exc:
        print(f"[kilo_worker] ERROR: {exc}", file=sys.stderr)
        if args.status:
            args.status.write_text(json.dumps({"success": False, "error": str(exc)}), encoding="utf-8")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
