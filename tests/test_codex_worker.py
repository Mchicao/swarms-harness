from __future__ import annotations

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
    assert command[command.index("--model") + 1] == "gpt-5.6-luna"
    assert command[command.index("-c") + 1] == "model_reasoning_effort=high"
