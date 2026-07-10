"""Tests for the Hermes Agent worker.

No real hermes invocation: ``subprocess.run`` is monkeypatched so we assert
the exact command shape without depending on the CLI being configured.
"""

import json

import pytest

from scripts import hermes_worker as worker


class _FakeProc:
    def __init__(self, stdout="", stderr="", returncode=0):
        self.stdout = stdout
        self.stderr = stderr
        self.returncode = returncode


def test_hermes_complete_uses_headless_flags(monkeypatch):
    captured = {}

    def fake_run(cmd, **kwargs):
        captured["cmd"] = cmd
        return _FakeProc(stdout="HERMES_DONE")

    monkeypatch.setattr(worker.subprocess, "run", fake_run)
    out = worker.hermes_complete("do the thing", model="", provider="")

    assert out == "HERMES_DONE"
    cmd = captured["cmd"]
    assert cmd[0] == "hermes"
    assert cmd[1] == "chat"
    assert "-q" in cmd and cmd[cmd.index("-q") + 1] == "do the thing"
    assert "-Q" in cmd
    assert "--yolo" not in cmd
    # max-turns must be present and integer-string to bound the agent loop.
    assert cmd[cmd.index("--max-turns") + 1] == str(worker.DEFAULT_MAX_TURNS)
    # Empty model/provider must NOT add -m / --provider (let Hermes default).
    assert "-m" not in cmd
    assert "--provider" not in cmd


def test_hermes_complete_passes_model_and_provider_when_set(monkeypatch):
    captured = {}

    def fake_run(cmd, **kwargs):
        captured["cmd"] = cmd
        return _FakeProc(stdout="ok")

    monkeypatch.setattr(worker.subprocess, "run", fake_run)
    worker.hermes_complete("prompt", model="glm-5.2", provider="zai")

    cmd = captured["cmd"]
    assert cmd[cmd.index("-m") + 1] == "glm-5.2"
    assert cmd[cmd.index("--provider") + 1] == "zai"


def test_hermes_complete_adds_yolo_only_when_explicit(monkeypatch):
    captured = {}

    def fake_run(cmd, **kwargs):
        captured["cmd"] = cmd
        return _FakeProc(stdout="ok")

    monkeypatch.setattr(worker.subprocess, "run", fake_run)
    worker.hermes_complete("prompt", yolo=True)

    assert "--yolo" in captured["cmd"]


def test_hermes_complete_raises_on_no_stdout_with_bad_exit(monkeypatch):
    def fake_run(cmd, **kwargs):
        return _FakeProc(stdout="", stderr="boom", returncode=1)

    monkeypatch.setattr(worker.subprocess, "run", fake_run)
    with pytest.raises(RuntimeError, match="exited 1"):
        worker.hermes_complete("prompt")


def test_hermes_complete_recovers_when_stderr_noisy_but_stdout_present(monkeypatch):
    # Hermes emits a UnicodeDecodeError on stderr in some envs; stdout is fine.
    def fake_run(cmd, **kwargs):
        return _FakeProc(stdout="REAL_ANSWER", stderr="UnicodeDecodeError...", returncode=0)

    monkeypatch.setattr(worker.subprocess, "run", fake_run)
    assert worker.hermes_complete("prompt") == "REAL_ANSWER"


def test_main_writes_status_success(tmp_path, monkeypatch):
    monkeypatch.setattr(worker.subprocess, "run", lambda cmd, **k: _FakeProc(stdout="THE_OUTPUT"))
    prompt = tmp_path / "prompt.txt"
    prompt.write_text("task", encoding="utf-8")
    status = tmp_path / "status.json"

    rc = worker.main(["--prompt", str(prompt), "--status", str(status)])
    assert rc == 0
    data = json.loads(status.read_text(encoding="utf-8"))
    assert data["success"] is True
    assert data["provider"] == "hermes"


def test_main_writes_unicode_output_as_utf8(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(worker.subprocess, "run", lambda cmd, **k: _FakeProc(stdout="resultado →"))
    prompt = tmp_path / "prompt.txt"
    prompt.write_text("task", encoding="utf-8")

    assert worker.main(["--prompt", str(prompt)]) == 0
    assert "resultado →" in capsys.readouterr().out


def test_main_writes_status_failure_on_error(tmp_path, monkeypatch):
    def boom(cmd, **k):
        return _FakeProc(stdout="", stderr="err", returncode=2)

    monkeypatch.setattr(worker.subprocess, "run", boom)
    prompt = tmp_path / "prompt.txt"
    prompt.write_text("task", encoding="utf-8")
    status = tmp_path / "status.json"

    rc = worker.main(["--prompt", str(prompt), "--status", str(status)])
    assert rc == 2
    data = json.loads(status.read_text(encoding="utf-8"))
    assert data["success"] is False
