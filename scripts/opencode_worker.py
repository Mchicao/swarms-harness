#!/usr/bin/env python3
"""SWARMS worker for GLM-5.2 via OpenCode CLI (Z.AI Coding Plan backend).

OpenCode is already authenticated with the Z.AI Coding Plan (auth stored at
``~/.local/share/opencode/auth.json``). This worker calls ``opencode run``
in one-shot mode, which is the simplest and most reliable way to invoke it
programmatically.

Security:
    No credentials in this file. OpenCode manages its own auth store
    (``~/.local/share/opencode/auth.json``, user-local, gitignored).

Context:
    OpenCode loads AGENTS.md from cwd by default. To keep context lean,
    this worker sets ``--cwd`` to a temp directory and passes the prompt
    via ``-f`` (file attachment), so no project rules leak into the worker
    context unless explicitly desired.

Usage:
    python scripts/opencode_worker.py --prompt /path/to/prompt.txt
    python scripts/opencode_worker.py --prompt @path --model zai-coding-plan/glm-5.2
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

DEFAULT_MODEL = os.environ.get("OPENCODE_MODEL", "zai-coding-plan/glm-5.2")
DEFAULT_TIMEOUT = int(os.environ.get("OPENCODE_TIMEOUT", "300"))
OPENCODE_BIN = os.environ.get("OPENCODE_BIN", "opencode")


PROJECT_ROOT = Path(__file__).resolve().parent.parent


def opencode_complete(
    prompt: str,
    *,
    model: str = DEFAULT_MODEL,
    timeout: int = DEFAULT_TIMEOUT,
    cwd: str | Path | None = None,
    tools_policy: str = "none",
) -> str:
    """Call GLM-5.2 via OpenCode one-shot and return the assistant text.

    If tools_policy is 'full', sets the cwd to PROJECT_ROOT so that OpenCode
    can load the workspace-level context (like AGENTS.md).
    Otherwise, uses a clean temp directory to keep the context window lean.
    """
    # Write prompt to temp file so we can pass it via -f
    tmp_dir = Path(tempfile.mkdtemp(prefix="swarms_opencode_"))
    prompt_file = tmp_dir / "prompt.md"
    prompt_file.write_text(prompt, encoding="utf-8")

    cmd = [
        OPENCODE_BIN,
        "run",
        "-m",
        model,
        "-f",
        str(prompt_file),
        "--auto",
        "Complete the task described in the attached file. Write only the required code changes.",
    ]

    target_cwd = cwd
    if tools_policy == "full" and not target_cwd:
        target_cwd = PROJECT_ROOT

    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=str(target_cwd) if target_cwd else str(tmp_dir),
        )
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)

    if proc.returncode != 0:
        raise RuntimeError(f"OpenCode exited {proc.returncode}. stderr={proc.stderr[:500]!r}")

    output = (proc.stdout or "").strip()
    if not output:
        raise RuntimeError(f"OpenCode produced no stdout. returncode={proc.returncode} stderr={proc.stderr[:300]!r}")
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description="GLM-5.2 worker via OpenCode CLI.")
    parser.add_argument("--prompt", type=Path, required=True, help="Path to prompt file.")
    parser.add_argument("--status", type=Path, default=None, help="Optional status output path.")
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    parser.add_argument("--tools-policy", default="none", choices=["none", "full"], help="Tools policy: none or full")
    args = parser.parse_args()

    prompt = args.prompt.read_text(encoding="utf-8", errors="replace")

    try:
        output = opencode_complete(prompt, model=args.model, timeout=args.timeout, tools_policy=args.tools_policy)
        print(output)
        if args.status:
            args.status.write_text(
                json.dumps({"success": True, "provider": "opencode", "model": args.model}),
                encoding="utf-8",
            )
        return 0
    except Exception as e:
        print(f"[opencode_worker] ERROR: {e}", file=sys.stderr)
        if args.status:
            args.status.write_text(json.dumps({"success": False, "error": str(e)}), encoding="utf-8")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
