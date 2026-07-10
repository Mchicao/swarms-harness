#!/usr/bin/env python
"""
SWARMS - Worker para Codex CLI (OpenAI Codex / gpt-5.5-codex).

Invoca el binario `codex` (o `codex.exe`) en modo `exec` con sandbox
workspace-write, de forma no interactiva. Sigue la misma interfaz CLI que los
demas workers (--prompt, --status, --model) para que workflow_runtime pueda
usarlo via WRAPPER_SCRIPTS.

El prompt se ejecuta en el directorio de trabajo actual (un worktree aislado
creado por el runtime). Codex escribe su salida a stdout y a un archivo -o.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

DEFAULT_TIMEOUT = int(os.environ.get("CODEX_WORKER_TIMEOUT", "600"))


def find_codex_binary() -> str:
    """Localiza el binario codex en PATH o en rutas conocidas."""
    for name in ("codex", "codex.exe"):
        found = shutil.which(name)
        if found:
            return found
    raise FileNotFoundError("No se encontro el binario 'codex' en PATH")


def run_codex(prompt: str, model: str, tools_policy: str, timeout: int) -> tuple[int, str, str]:
    """Ejecuta codex exec y retorna (returncode, stdout, stderr)."""
    binary = find_codex_binary()
    sandbox = "workspace-write" if tools_policy != "none" else "read-only"
    reasoning_effort = os.environ.get("CODEX_REASONING_EFFORT", "medium")
    cwd = Path.cwd()
    out_file = cwd / "codex_output.md"
    cmd = [
        binary,
        "exec",
        "--model",
        model,
        "-s",
        sandbox,
        "-c",
        f"model_reasoning_effort={reasoning_effort}",
        "-o",
        str(out_file),
        "--json",
        prompt,
    ]
    try:
        result = subprocess.run(
            cmd,
            cwd=str(cwd),
            capture_output=True,
            text=True,
            timeout=timeout,
            encoding="utf-8",
            errors="replace",
        )
        # Codex escribe el resultado en out_file; adjuntarlo a stdout para el runtime
        if out_file.exists():
            artifact = out_file.read_text(encoding="utf-8", errors="replace")
            sys.stdout.write(artifact)
        return result.returncode, result.stdout or "", result.stderr or ""
    except subprocess.TimeoutExpired:
        return 124, "", f"codex timed out after {timeout}s"
    except FileNotFoundError as exc:
        return 127, "", str(exc)


def main() -> int:
    parser = argparse.ArgumentParser(description="SWARMS codex worker")
    parser.add_argument("--prompt", required=True, help="Task prompt for codex")
    parser.add_argument("--status", default=None, help="Optional status JSON path (compat)")
    parser.add_argument("--model", default="gpt-5.5-codex", help="Model label (informational)")
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT, help="Max seconds")
    parser.add_argument("--tools-policy", default="full", choices=["none", "full"])
    args = parser.parse_args()

    rc, out, err = run_codex(args.prompt, args.model, args.tools_policy, args.timeout)
    if err and rc != 0:
        sys.stderr.write(err + "\n")
    return rc


if __name__ == "__main__":
    sys.exit(main())
