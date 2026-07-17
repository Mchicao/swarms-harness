from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

from scripts import codex_worker


def test_run_codex_forwards_requested_model_and_reasoning_effort(monkeypatch, tmp_path):
    captured: dict[str, object] = {}

    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("CODEX_REASONING_EFFORT", "high")
    monkeypatch.setattr(codex_worker, "find_codex_binary", lambda: "codex")

    def fake_run(command, **kwargs):
        captured["command"] = command
        captured["kwargs"] = kwargs
        return SimpleNamespace(returncode=0, stdout="ok", stderr="")

    monkeypatch.setattr(codex_worker.subprocess, "run", fake_run)

    returncode, stdout, stderr = codex_worker.run_codex(
        "Revisa el repositorio",
        "gpt-5.6-luna",
        "none",
        30,
    )

    command = captured["command"]
    assert returncode == 0
    assert stdout == "ok"
    assert stderr == ""
    # SWARMS-CODEX-001: La aprobación global debe preceder al subcomando.
    assert command[:4] == ["codex", "-a", "never", "exec"]
    assert "--no-alt-screen" not in command
    assert command[command.index("--model") + 1] == "gpt-5.6-luna"
    assert command[command.index("-c") + 1] == "model_reasoning_effort=high"


def test_legacy_parallel_swarm_uses_canonical_codex_cli_order():
    # SWARMS-CODEX-002: El adaptador legado conserva la misma sintaxis válida.
    source = Path("scripts/parallel_swarm.ps1").read_text(encoding="utf-8")

    assert "codex -a never exec" in source
    assert "codex.exe`\" exec --full-auto" not in source


def test_codex_resume_uses_exact_thread_id_and_persists_it(monkeypatch, tmp_path):
    captured = {}
    status = tmp_path / "status.json"
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(codex_worker, "find_codex_binary", lambda: "codex")

    def fake_run(command, **_kwargs):
        captured["command"] = command
        return SimpleNamespace(
            returncode=0,
            stdout=json.dumps({"type": "thread.started", "thread_id": "thread-123"}),
            stderr="",
        )

    monkeypatch.setattr(codex_worker.subprocess, "run", fake_run)
    codex_worker.run_codex("Continue", "gpt-5.6-luna", "none", 30, "thread-123", status)

    command = captured["command"]
    assert command[command.index("resume") + 1] == "thread-123"
    assert "--last" not in command
    assert json.loads(status.read_text())["provider_session_id"] == "thread-123"
