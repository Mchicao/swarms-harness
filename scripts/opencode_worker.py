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
    OpenCode requires a stable project cwd for headless runs. The worker
    therefore uses the workspace cwd and relies on ``--pure`` to prevent
    tool execution for read-only calls.

Usage:
    python scripts/opencode_worker.py --prompt /path/to/prompt.txt
    python scripts/opencode_worker.py --prompt @path --model zai-coding-plan/glm-5.2
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
from pathlib import Path

try:
    from .paths import WORKSPACE_ROOT
    from .provider_session import write_provider_status
except ImportError:  # pragma: no cover - direct script execution path.
    from paths import WORKSPACE_ROOT
    from provider_session import write_provider_status

DEFAULT_MODEL = os.environ.get("OPENCODE_MODEL", "zai-coding-plan/glm-5.2")
DEFAULT_TIMEOUT = int(os.environ.get("OPENCODE_TIMEOUT", "600"))
OPENCODE_BIN = os.environ.get("OPENCODE_BIN", "opencode")


def _terminate_process_tree(proc: subprocess.Popen[str]) -> None:
    """Termina el grupo del worker sin afectar procesos fuera de él."""
    root_running = proc.poll() is None
    if root_running:
        try:
            if os.name == "nt":
                # CREATE_NEW_PROCESS_GROUP permite una señal cooperativa al árbol.
                proc.send_signal(signal.CTRL_BREAK_EVENT)
            else:
                os.killpg(proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            root_running = False

    try:
        proc.wait(timeout=2)
    except subprocess.TimeoutExpired:
        pass

    if os.name == "nt":
        # /T limita la terminación al árbol cuyo PID pertenece al worker,
        # incluso si la raíz ya terminó pero dejó descendientes.
        subprocess.run(
            ["taskkill", "/PID", str(proc.pid), "/T", "/F"],
            capture_output=True,
            check=False,
        )
    else:
        try:
            # El grupo aislado sigue siendo propiedad de este worker aunque
            # su raíz haya terminado; sólo escalamos si aún existe.
            os.killpg(proc.pid, 0)
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    if root_running or proc.poll() is None:
        proc.wait()


def opencode_complete(
    prompt: str,
    *,
    model: str = DEFAULT_MODEL,
    variant: str | None = None,
    timeout: int = DEFAULT_TIMEOUT,
    cwd: str | Path | None = None,
    tools_policy: str = "none",
    resume_session: str | None = None,
    status_path: Path | None = None,
) -> str:
    """Call GLM-5.2 via OpenCode one-shot and return the assistant text.

    If tools_policy is 'full', sets the cwd to PROJECT_ROOT so that OpenCode
    can load the workspace-level context (like AGENTS.md).
    Otherwise, uses the workspace cwd with ``--pure`` to keep the call
    read-only while preserving OpenCode's project/session initialization.
    """
    # Inline the bounded prompt: OpenCode 1.14 can hang when a headless run
    # receives the task as a positional message plus a `-f` attachment.
    message = "Complete the task described below. Write only the required code changes.\n\n" + prompt
    cmd = [
        OPENCODE_BIN,
        "run",
        "--format",
        "json",
        "-m",
        model,
    ]
    if variant:
        cmd.extend(["--variant", variant])
    if resume_session:
        cmd.extend(["--session", resume_session])
    cmd.extend(
        [
            message,
        ]
    )
    message_index = cmd.index(message)
    if tools_policy == "full":
        # SWARMS-001: OpenCode 1.14 reemplazó --auto por esta bandera explícita.
        cmd.insert(message_index, "--dangerously-skip-permissions")
    else:
        cmd.insert(message_index, "--pure")

    target_cwd = cwd or WORKSPACE_ROOT
    popen_kwargs: dict[str, object] = {
        "stdout": subprocess.PIPE,
        "stderr": subprocess.PIPE,
        # OpenCode emits UTF-8 even when the parent PowerShell locale is
        # cp1252; replacement keeps a completed response parseable.
        "text": True,
        "encoding": "utf-8",
        "errors": "replace",
        "cwd": str(target_cwd),
    }
    if os.name == "nt":
        popen_kwargs["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
    else:
        popen_kwargs["start_new_session"] = True

    proc = subprocess.Popen(cmd, **popen_kwargs)
    try:
        stdout, stderr = proc.communicate(timeout=timeout)
        output = (stdout or "").strip()
        if not output:
            raise RuntimeError(f"OpenCode produced no stdout. returncode={proc.returncode} stderr={stderr[:300]!r}")
        text_events: list[str] = []
        session_id = resume_session
        for line in output.splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            event_session = event.get("sessionID") or event.get("session_id")
            if isinstance(event_session, str) and event_session:
                session_id = event_session
                write_provider_status(
                    status_path, provider="opencode", model=model,
                    provider_session_id=session_id, success=False,
                )
            if event.get("type") == "error":
                # SWARMS-002: Algunas fallas API terminan con exit code 0.
                error = event.get("error", {})
                message = error.get("data", {}).get("message") or error.get("message") or "OpenCode API error"
                raise RuntimeError(str(message))
            if event.get("type") == "text":
                text = event.get("part", {}).get("text")
                if isinstance(text, str):
                    text_events.append(text)
        if proc.returncode != 0:
            raise RuntimeError(f"OpenCode exited {proc.returncode}. stderr={stderr[:500]!r}")
        if session_id:
            write_provider_status(
                status_path, provider="opencode", model=model, provider_session_id=session_id, success=True
            )
        return "".join(text_events) if text_events else output
    finally:
        _terminate_process_tree(proc)


def main() -> int:
    parser = argparse.ArgumentParser(description="GLM-5.2 worker via OpenCode CLI.")
    parser.add_argument("--prompt", type=Path, required=True, help="Path to prompt file.")
    parser.add_argument("--status", type=Path, default=None, help="Optional status output path.")
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--variant", default=None, help="Model variant, e.g. high")
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    parser.add_argument("--cwd", type=Path, default=None, help="Workspace used for full-tools execution")
    parser.add_argument("--tools-policy", default="none", choices=["none", "full"], help="Tools policy: none or full")
    parser.add_argument("--resume-session", default=None, help="Exact OpenCode session ID to resume")
    args = parser.parse_args()

    prompt = args.prompt.read_text(encoding="utf-8", errors="replace")

    try:
        output = opencode_complete(
            prompt,
            model=args.model,
            variant=args.variant,
            timeout=args.timeout,
            cwd=args.cwd,
            tools_policy=args.tools_policy,
            resume_session=args.resume_session,
            status_path=args.status,
        )
        if hasattr(sys.stdout, "reconfigure"):
            sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        print(output)
        if args.status:
            write_provider_status(args.status, success=True, provider="opencode", model=args.model)
        return 0
    except Exception as e:
        print(f"[opencode_worker] ERROR: {e}", file=sys.stderr)
        if args.status:
            write_provider_status(args.status, success=False, error=str(e))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
