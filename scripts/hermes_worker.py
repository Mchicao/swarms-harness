#!/usr/bin/env python3
"""SWARMS worker for Hermes Agent (Nous Research) headless mode.

Hermes is a full coding-agent CLI with tool-calling, skills, Mixture-of-Agents
fallback, and multi-provider routing. Unlike a bare completion endpoint, a
Hermes worker can read files, run tools, and self-correct — so it is a real
subagent, not just a model call.

This worker invokes Hermes in non-interactive single-shot mode:

    hermes chat -q "<prompt>" -Q --max-turns N

``-q`` makes it one-shot (no TUI), ``-Q`` is the quiet flag that suppresses the
banner / spinner / tool previews so stdout carries ONLY the final response.
Verified working headless on Hermes v0.18.0 (unlike ``agy --print``, Hermes
prints the answer to stdout — no SQLite extraction trick needed).

Security:
    No credentials in this file. Hermes manages its own provider keys in its
    ``.env`` under the hermes project dir (user-local, gitignored). The model
    and provider are passed through verbatim; if unset, Hermes uses whatever
    it is configured to use by default.

Context:
    Minimal by design — Hermes loads its own skills/toolsets. We pass only the
    prompt text. ``--max-turns`` bounds the agent loop so a runaway worker
    cannot burn unbounded turns.

Usage:
    python scripts/hermes_worker.py --prompt /path/to/prompt.txt
    python scripts/hermes_worker.py --prompt @path --model glm-5.2 --provider zai
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

DEFAULT_MODEL = os.environ.get("HERMES_MODEL", "")  # empty = let Hermes pick
DEFAULT_PROVIDER = os.environ.get("HERMES_PROVIDER", "")  # empty = auto
DEFAULT_MAX_TURNS = int(os.environ.get("HERMES_MAX_TURNS", "8"))
DEFAULT_TIMEOUT = int(os.environ.get("HERMES_TIMEOUT", "300"))
HERMES_BIN = os.environ.get("HERMES_BIN", "hermes")


def hermes_complete(
    prompt: str,
    *,
    model: str = DEFAULT_MODEL,
    provider: str = DEFAULT_PROVIDER,
    max_turns: int = DEFAULT_MAX_TURNS,
    timeout: int = DEFAULT_TIMEOUT,
    yolo: bool = False,
) -> str:
    """Call Hermes headless and return the assistant response text.

    ``-q`` (query) + ``-Q`` (quiet) is the headless single-shot path.
    ``--max-turns`` bounds the internal agent loop. ``--yolo`` is used only
    when the caller explicitly grants the ``full`` tools policy.
    """
    cmd = [
        HERMES_BIN,
        "chat",
        "-q",
        prompt,
        "-Q",
        "--max-turns",
        str(max_turns),
    ]
    if model:
        cmd.extend(["-m", model])
    if provider:
        cmd.extend(["--provider", provider])
    if yolo:
        cmd.append("--yolo")

    proc = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        timeout=timeout,
        encoding="utf-8",
        errors="replace",
    )

    # Hermes emits a UnicodeDecodeError on stderr in some environments from an
    # internal subprocess reader; it does not corrupt stdout. We treat a
    # non-zero returncode with usable stdout as success, but surface real
    # failures (no stdout AND bad exit) as an error.
    output = (proc.stdout or "").strip()
    if proc.returncode != 0 and not output:
        stderr_tail = (proc.stderr or "")[-400:].strip()
        raise RuntimeError(f"Hermes exited {proc.returncode} with no stdout. stderr: {stderr_tail}")
    if not output:
        raise RuntimeError(
            f"Hermes produced no stdout. returncode={proc.returncode} stderr={(proc.stderr or '')[-200:]!r}"
        )
    return output


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Hermes Agent worker (headless).")
    parser.add_argument("--prompt", type=Path, required=True, help="Path to prompt file.")
    parser.add_argument("--status", type=Path, default=None, help="Optional status output path.")
    parser.add_argument(
        "--model", default=DEFAULT_MODEL, help="Model id (empty = Hermes default). e.g. glm-5.2, tencent/hy3."
    )
    parser.add_argument(
        "--provider", default=DEFAULT_PROVIDER, help="Provider name (empty = auto). e.g. zai, openrouter, google."
    )
    parser.add_argument("--max-turns", type=int, default=DEFAULT_MAX_TURNS, help="Max agent turns (bounds the loop).")
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    parser.add_argument(
        "--tools-policy",
        default="none",
        choices=["none", "full"],
        help="Contract compatibility. Hermes always has its own toolsets; "
        "--tools-policy=full is a no-op kept for the runtime contract.",
    )
    args = parser.parse_args(argv)

    prompt = args.prompt.read_text(encoding="utf-8", errors="replace")

    try:
        output = hermes_complete(
            prompt,
            model=args.model,
            provider=args.provider,
            max_turns=args.max_turns,
            timeout=args.timeout,
            yolo=args.tools_policy == "full",
        )
        print(output)
        if args.status:
            args.status.write_text(
                json.dumps({"success": True, "provider": "hermes", "model": args.model or "(hermes-default)"}),
                encoding="utf-8",
            )
        return 0
    except Exception as e:
        print(f"[hermes_worker] ERROR: {e}", file=sys.stderr)
        if args.status:
            args.status.write_text(json.dumps({"success": False, "error": str(e)}), encoding="utf-8")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
