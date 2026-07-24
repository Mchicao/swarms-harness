#!/usr/bin/env python3
"""SWARMS worker for Gemini 3.5 Flash via agy (Antigravity CLI).

Wraps the existing ``agy_call.py`` module so the SWARMS runtime can invoke
Gemini as just another provider.  agy handles authentication (OAuth) and
model selection internally; this worker just passes the prompt through and
returns the assistant response.

Security:
    No credentials in this file. agy uses OAuth tokens stored under
    ``~/.gemini/antigravity-cli/`` (gitignored, user-local).

Context:
    Minimal system prompt — no skills, no AGENTS.md, no project rules.

Usage:
    python scripts/gemini_worker.py --prompt /path/to/prompt.txt
    python scripts/gemini_worker.py --prompt @path --model "Gemini 3.5 Flash (Low)"
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

# Import the existing agy_call wrapper from the same scripts/ directory.
SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

from agy_call import agy_complete  # noqa: E402

DEFAULT_MODEL = os.environ.get("AGY_MODEL", "Gemini 3.5 Flash (Low)")
DEFAULT_TIMEOUT = int(os.environ.get("AGY_TIMEOUT", "600"))


def gemini_complete(
    prompt: str,
    *,
    model: str | None = None,
    timeout: int = DEFAULT_TIMEOUT,
    cwd: str | Path | None = None,
    tools_policy: str = "none",
) -> str:
    """Call Gemini via agy and return the assistant text.

    If tools_policy is 'none', runs the agy call in a temporary clean directory
    to prevent it from loading workspace-level AGENTS.md rules.
    """
    kwargs = {"model": model, "timeout": timeout}
    if tools_policy == "full":
        # SWARMS-004: La edición ocurre en el workspace objetivo explícito.
        kwargs.update(skip_permissions=True, sandbox=False, cwd=cwd)
        return agy_complete(prompt, **kwargs)

    import tempfile

    with tempfile.TemporaryDirectory(prefix="swarms_gemini_") as tmp_dir:
        kwargs.update(skip_permissions=False, sandbox=True, cwd=tmp_dir)
        for attempt in range(2):
            try:
                return agy_complete(prompt, **kwargs)
            except RecursionError:
                if attempt:
                    raise
                time.sleep(2)
    raise AssertionError("unreachable")


def main() -> int:
    parser = argparse.ArgumentParser(description="Gemini worker via agy CLI.")
    parser.add_argument("--prompt", type=Path, required=True, help="Path to prompt file.")
    parser.add_argument("--status", type=Path, default=None, help="Optional status output path.")
    parser.add_argument(
        "--model",
        default=DEFAULT_MODEL,
        help="agy model label, e.g. 'Gemini 3.5 Flash (Low)'",
    )
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    parser.add_argument("--cwd", type=Path, default=None, help="Workspace used for full-tools execution")
    parser.add_argument("--tools-policy", default="none", choices=["none", "full"], help="Tools policy: none or full")
    args = parser.parse_args()

    prompt = args.prompt.read_text(encoding="utf-8", errors="replace")

    try:
        output = gemini_complete(
            prompt,
            model=args.model,
            timeout=args.timeout,
            cwd=args.cwd,
            tools_policy=args.tools_policy,
        )
        if hasattr(sys.stdout, "reconfigure"):
            sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        print(output)
        if args.status:
            args.status.write_text(
                json.dumps({"success": True, "provider": "gemini", "model": args.model}),
                encoding="utf-8",
            )
        return 0
    except Exception as e:
        print(f"[gemini_worker] ERROR: {e}", file=sys.stderr)
        if args.status:
            args.status.write_text(json.dumps({"success": False, "error": str(e)}), encoding="utf-8")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
