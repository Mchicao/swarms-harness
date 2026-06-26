"""Programmatic wrapper around `agy` (Antigravity CLI).

WHY THIS EXISTS
---------------
`agy -p` / `agy --print` does NOT print the model response to stdout in this
environment (confirmed on agy 1.0.10, headless / non-TTY). The process exits 0,
the HTTP stream completes, and the answer is persisted to the conversation's
SQLite store -- but stdout is empty. So `subprocess.run(..., capture_output=True)`
always sees an empty stdout.

This wrapper works around that by:
  1. Calling `agy --print --model <label> <prompt>` (which still generates and
     persists the response).
  2. Locating the newest conversation DB that agy just wrote.
  3. Extracting the assistant turn text from the `steps` table.

The result is a reliable, programmatic Gemini call with no TTY dependency.

USAGE
-----
    from scripts.agy_call import agy_complete
    answer = agy_complete("Say OK", model="Gemini 3.5 Flash (Low)")
    print(answer)

CLI:
    python scripts/agy_call.py "Say OK" --model "Gemini 3.5 Flash (Low)"
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

# agy stores conversations here regardless of --model.
AGY_CONV_DIR = Path(os.path.expandvars(r"%USERPROFILE%\.gemini\antigravity-cli\conversations"))
# Human-readable transcripts live under brain/<cascade_id>/.system_generated/logs.
AGY_BRAIN_DIR = Path(os.path.expandvars(r"%USERPROFILE%\.gemini\antigravity-cli\brain"))

# --print must point at an absolute path or be passed a literal string.
AGY_BIN = os.environ.get("AGY_BIN", "agy")
DEFAULT_TIMEOUT = int(os.environ.get("AGY_TIMEOUT", "180"))


def _list_conv_dbs() -> list[Path]:
    if not AGY_CONV_DIR.exists():
        return []
    return sorted(AGY_CONV_DIR.glob("*.db"), key=lambda p: p.stat().st_mtime)


def _extract_answer(db_path: Path) -> str:
    """Pull the last assistant text turn from a conversation.

    Primary source: the human-readable transcript JSONL that agy writes under
    brain/<cascade_id>/.system_generated/logs/transcript.jsonl. Each line is a
    JSON object; PLANNER_RESPONSE lines carry a clean ``content`` field with the
    final answer (separate from ``thinking``). This is far more reliable than
    parsing the protobuf step payloads.

    Fallback: if no transcript is found, parse the step_type-15 protobuf blob.
    """
    answer = _extract_from_transcript(db_path)
    if answer:
        return answer
    return _extract_from_protobuf(db_path)


def _extract_from_transcript(db_path: Path) -> str:
    """Read the assistant answer from the transcript.jsonl via cascade_id."""
    import json

    cascade_id = _cascade_id_for(db_path)
    if not cascade_id:
        return ""
    transcript = AGY_BRAIN_DIR / cascade_id / ".system_generated" / "logs" / "transcript.jsonl"
    if not transcript.exists():
        # Some builds use transcript_full.jsonl.
        transcript = AGY_BRAIN_DIR / cascade_id / ".system_generated" / "logs" / "transcript_full.jsonl"
    if not transcript.exists():
        return ""
    last_content = ""
    try:
        with open(transcript, "r", encoding="utf-8", errors="replace") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if obj.get("type") == "PLANNER_RESPONSE":
                    content = obj.get("content")
                    if isinstance(content, str) and content.strip():
                        last_content = content.strip()
    except OSError:
        return ""
    return last_content


def _cascade_id_for(db_path: Path) -> str | None:
    """Map a conversation DB to its brain cascade_id via trajectory_meta."""
    try:
        con = sqlite3.connect(str(db_path))
        try:
            row = con.execute("SELECT cascade_id FROM trajectory_meta LIMIT 1").fetchone()
            return row[0] if row else None
        finally:
            con.close()
    except sqlite3.Error:
        return None


def _extract_from_protobuf(db_path: Path) -> str:
    """Fallback: parse the assistant turn from the step_type-15 protobuf payload."""
    con = sqlite3.connect(str(db_path))
    try:
        cur = con.cursor()
        rows = cur.execute(
            "SELECT step_payload FROM steps WHERE step_type = 15 ORDER BY idx"
        ).fetchall()
        candidates: list[str] = []
        for (payload,) in rows:
            if not payload:
                continue
            raw = bytes(payload) if isinstance(payload, (bytes, bytearray)) else str(payload).encode()
            for cand, trailing in _decode_string_fields(raw):
                if not _looks_like_answer(cand):
                    continue
                is_turn = trailing.startswith(b"2(bot-") or trailing.startswith(b"(bot-")
                candidates.append(cand)
                if is_turn:
                    return cand.strip()
        return _pick_best_answer(candidates).strip()
    finally:
        con.close()


def _pick_best_answer(candidates: list[str]) -> str:
    """Choose the most likely assistant answer from decoded string fields.

    agy serializes the assistant turn with both the final answer and (sometimes)
    internal thinking fragments. Truncated fragments appear when our byte-level
    scan happens to land on a stray 0x0a inside binary data. We rank by a
    completeness score that rewards natural endings (whitespace, punctuation,
    or a full short token) and penalizes mid-word truncation.
    """
    if not candidates:
        return ""
    scored: list[tuple[float, str]] = []
    for c in candidates:
        score = float(len(c))
        if not c:
            continue
        last = c[-1]
        # Natural sentence/word ending -> likely a complete answer.
        if last in ".!?,;:)]}\"'" or last.isspace():
            score += 100
        elif len(c) <= 40 and c.strip() == c:
            # Short, clean token with no surrounding spaces (e.g. "LISTO", "56").
            score += 50
        else:
            # Ends mid-word -> likely a truncated fragment; penalize.
            score -= 50
        scored.append((score, c))
    scored.sort(key=lambda t: t[0], reverse=True)
    return scored[0][1]


def _decode_string_fields(raw: bytes) -> list[tuple[str, bytes]]:
    """Extract length-delimited UTF-8 string fields from a protobuf-like blob.

    The step payload is not pure protobuf from byte 0 (it has a binary header
    with conversation/step IDs), so a strict from-start parse desynchronizes.
    Instead we slide a window: wherever we see tag byte 0x0a (field 1,
    length-delimited), read the following varint length and try to decode that
    many bytes as clean UTF-8. Returns (text, trailing_bytes) so callers can
    inspect what follows the field (e.g. a ``2(bot-`` author-id marker).
    """
    out: list[tuple[str, bytes]] = []
    n = len(raw)
    i = 0
    while i < n:
        if raw[i] == 0x0A and i + 1 < n:
            length, j = _read_varint(raw, i + 1)
            if (
                length is not None
                and 1 <= length <= 8000
                and j + length <= n
            ):
                chunk = raw[j:j + length]
                trailing = raw[j + length:j + length + 16]
                try:
                    text = chunk.decode("utf-8")
                except UnicodeDecodeError:
                    text = ""
                if text and _is_clean_text(text):
                    out.append((text, trailing))
                    i = j + length
                    continue
        i += 1
    return out


def _is_clean_text(text: str) -> bool:
    """True if the string looks like real prose/code, not binary noise."""
    if not text:
        return False
    printable = sum(1 for c in text if c.isprintable() or c in "\t\n\r")
    if printable / len(text) < 0.9:
        return False
    # An answer can be prose (needs letters) OR a short numeric/code token.
    has_letters = bool(re.search(r"[A-Za-zÀ-ÿ]{2,}", text))
    is_numeric = bool(re.fullmatch(r"[\d.,\s+\-*/=()]{1,40}", text)) and any(
        c.isdigit() for c in text
    )
    return has_letters or is_numeric


def _read_varint(buf: bytes, pos: int) -> tuple[int | None, int]:
    result = 0
    shift = 0
    while pos < len(buf):
        b = buf[pos]
        pos += 1
        result |= (b & 0x7F) << shift
        if not (b & 0x80):
            return result, pos
        shift += 7
        if shift > 63:
            return None, pos
    return None, pos


def _looks_like_answer(frag: str) -> bool:
    """Heuristic to distinguish a model answer from protobuf/UUID scaffolding."""
    if not frag:
        return False
    if frag.startswith(("sessionID", "b$", "$", '"$', "JM", "B!")):
        return False
    # UUID-like blobs.
    if re.fullmatch(r"[0-9a-f-]{20,}", frag):
        return False
    if "file:///" in frag[:20]:
        return False
    # Accept prose (needs letters) OR a short numeric/code-only token.
    has_letters = bool(re.search(r"[A-Za-zÀ-ÿ]{2,}", frag))
    is_numeric = bool(re.fullmatch(r"[\d.,\s+\-*/=()]{1,40}", frag)) and any(
        c.isdigit() for c in frag
    )
    return has_letters or is_numeric


def agy_complete(
    prompt: str,
    *,
    model: str | None = None,
    timeout: int = DEFAULT_TIMEOUT,
    skip_permissions: bool = True,
) -> str:
    """Run agy in print mode and return the assistant's answer.

    Falls back to reading the persisted conversation because `--print` does not
    emit the answer on stdout in headless mode.

    The prompt may be a literal string or an `@file` reference (path to a text
    file whose contents become the prompt), matching agy's own `@file` syntax.
    """
    # Resolve @file references by inlining their contents. We pass the literal
    # text to --print rather than relying on agy's @file resolution, so the
    # prompt is identical regardless of working directory.
    if isinstance(prompt, str) and prompt.startswith("@"):
        path = Path(prompt[1:])
        prompt = path.read_text(encoding="utf-8", errors="replace")

    before = {p.name: p.stat().st_mtime for p in _list_conv_dbs()}

    cmd: list[str] = [AGY_BIN]
    if skip_permissions:
        cmd.append("--dangerously-skip-permissions")
    if model:
        cmd += ["--model", model]
    cmd += ["--print", prompt]

    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    # stdout is usually empty in headless mode; keep it as a first attempt.
    direct = (proc.stdout or "").strip()
    if direct:
        return direct

    # Otherwise: wait for a new/updated conversation DB and read from it.
    # agy needs ~10-20s for auth + model setup before it writes the answer.
    deadline = time.time() + 60
    answer = ""
    while time.time() < deadline:
        time.sleep(2.0)
        after = _list_conv_dbs()
        new_or_updated = [
            p for p in after
            if p.name not in before or p.stat().st_mtime > before.get(p.name, 0)
        ]
        if new_or_updated:
            latest = max(new_or_updated, key=lambda p: p.stat().st_mtime)
            answer = _extract_answer(latest)
            if answer:
                break
    if not answer:
        raise RuntimeError(
            f"agy produced no stdout and no parseable answer. "
            f"returncode={proc.returncode} stderr={(proc.stderr or '')[:300]!r}"
        )
    return answer


def main() -> int:
    ap = argparse.ArgumentParser(description="Programmatic agy (Gemini) call.")
    ap.add_argument("prompt", help="Prompt text, or @path to read prompt from a file.")
    ap.add_argument(
        "--model",
        default=os.environ.get("AGY_MODEL"),
        help="Model label, e.g. 'Gemini 3.5 Flash (Medium)'. "
             "Defaults to $AGY_MODEL.",
    )
    ap.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    args = ap.parse_args()
    try:
        out = agy_complete(args.prompt, model=args.model, timeout=args.timeout)
    except Exception as e:  # noqa: BLE001
        print(f"[agy_call] ERROR: {e}", file=sys.stderr)
        return 2
    print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
