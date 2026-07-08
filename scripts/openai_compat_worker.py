#!/usr/bin/env python3
"""SWARMS worker for any OpenAI-compatible chat-completions endpoint.

This is the adapter used for routes whose provider speaks the OpenAI
``/v1/chat/completions`` protocol. It keeps credentials and endpoints in
environment variables (never in the file, never in the repo), so the same
code serves OpenRouter, Novita, SiliconFlow, and any compatible gateway.

Security:
    No credentials in this file. The API key and base URL come from
    environment variables named by the caller via ``--key-env`` and
    ``--base-url-env`` (default ``OPENROUTER_API_KEY`` /
    ``OPENROUTER_BASE_URL``). Nothing is written to disk beyond the optional
    status JSON, which carries only success/error and the model name.

Context:
    Minimal system prompt — no skills, no AGENTS.md, no project rules leak.
    The prompt file is read and sent as the single user message.

Usage:
    python scripts/openai_compat_worker.py --prompt /path/to/prompt.txt
    python scripts/openai_compat_worker.py --prompt @path \
        --model tencent/hy3:free \
        --base-url-env OPENROUTER_BASE_URL --key-env OPENROUTER_API_KEY

    Default base URL is https://openrouter.ai/api/v1 (overridable via the
    named env var, or directly with --base-url for local testing).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

DEFAULT_MODEL = os.environ.get("OPENAI_COMPAT_MODEL", "tencent/hy3:free")
DEFAULT_BASE_URL_ENV = "OPENROUTER_BASE_URL"
DEFAULT_BASE_URL_FALLBACK = "https://openrouter.ai/api/v1"
DEFAULT_KEY_ENV = "OPENROUTER_API_KEY"
DEFAULT_TIMEOUT = int(os.environ.get("OPENAI_COMPAT_TIMEOUT", "300"))


def _resolve_base_url(base_url_env: str, base_url: str | None) -> str:
    """Env var wins, then explicit flag, then the OpenRouter fallback."""
    if base_url_env and os.environ.get(base_url_env):
        return os.environ[base_url_env].rstrip("/")
    if base_url:
        return base_url.rstrip("/")
    return DEFAULT_BASE_URL_FALLBACK


def openai_compat_complete(
    prompt: str,
    *,
    model: str,
    base_url: str,
    api_key: str,
    timeout: int = DEFAULT_TIMEOUT,
    extra_headers: dict[str, str] | None = None,
) -> str:
    """Call an OpenAI-compatible chat-completions endpoint, return text.

    Raises RuntimeError on network/HTTP/parse errors so the caller can mark
    the task failed with a useful message.
    """
    url = f"{base_url}/chat/completions"
    payload = {
        "model": model,
        "messages": [
            {"role": "system", "content": "You are a focused coding worker. Implement exactly what the user asks. Output only the required code or a concise result."},
            {"role": "user", "content": prompt},
        ],
        "temperature": 0.2,
    }
    body = json.dumps(payload).encode("utf-8")
    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {api_key}",
    }
    if extra_headers:
        headers.update(extra_headers)

    req = urllib.request.Request(url, data=body, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as exc:
        detail = ""
        try:
            detail = exc.read().decode("utf-8", errors="replace")[:500]
        except Exception:
            pass
        raise RuntimeError(
            f"HTTP {exc.code} from {url}: {detail or exc.reason}"
        ) from exc
    except urllib.error.URLError as exc:
        raise RuntimeError(f"Network error calling {url}: {exc.reason}") from exc

    try:
        data = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"Non-JSON response from {url}: {raw[:300]!r}") from exc

    # OpenAI-compatible shape: choices[0].message.content
    choices = data.get("choices") or []
    if not choices:
        raise RuntimeError(f"No choices in response: {raw[:300]!r}")
    content = choices[0].get("message", {}).get("content")
    if not content:
        raise RuntimeError(f"Empty content in response: {raw[:300]!r}")
    return content.strip()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="SWARMS worker for any OpenAI-compatible chat-completions endpoint."
    )
    parser.add_argument("--prompt", type=Path, required=True, help="Path to prompt file.")
    parser.add_argument("--status", type=Path, default=None, help="Optional status output path.")
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    parser.add_argument("--tools-policy", default="none", choices=["none", "full"],
                        help="Accepted for contract compatibility; this worker never grants tools.")
    parser.add_argument("--base-url-env", default=DEFAULT_BASE_URL_ENV,
                        help="Env var name holding the base URL (e.g. OPENROUTER_BASE_URL).")
    parser.add_argument("--key-env", default=DEFAULT_KEY_ENV,
                        help="Env var name holding the API key (e.g. OPENROUTER_API_KEY).")
    parser.add_argument("--base-url", default=None,
                        help="Base URL override (local testing); env var and fallback are ignored when set.")
    args = parser.parse_args(argv)

    prompt = args.prompt.read_text(encoding="utf-8", errors="replace")

    api_key = os.environ.get(args.key_env, "")
    if not api_key:
        msg = (f"Missing API key: set ${args.key_env}. No real provider call was made.")
        print(f"[openai_compat_worker] ERROR: {msg}", file=sys.stderr)
        if args.status:
            args.status.write_text(json.dumps({"success": False, "error": msg}), encoding="utf-8")
        return 2

    base_url = _resolve_base_url(args.base_url_env, args.base_url)

    try:
        output = openai_compat_complete(
            prompt,
            model=args.model,
            base_url=base_url,
            api_key=api_key,
            timeout=args.timeout,
        )
        print(output)
        if args.status:
            args.status.write_text(
                json.dumps({"success": True, "provider": "openai_compat", "model": args.model,
                            "base_url": base_url, "key_env": args.key_env}),
                encoding="utf-8",
            )
        return 0
    except Exception as e:
        print(f"[openai_compat_worker] ERROR: {e}", file=sys.stderr)
        if args.status:
            args.status.write_text(json.dumps({"success": False, "error": str(e)}), encoding="utf-8")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
