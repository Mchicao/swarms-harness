import json
import subprocess
from pathlib import Path

import pytest

from scripts import opencode_worker


class _FakeOpenCodeProcess:
    def __init__(self, outcome):
        self.pid = 4182
        self.returncode = 0
        self._outcome = outcome
        self.timeout = None
        self.wait_timeout = None

    def communicate(self, *, timeout):
        self.timeout = timeout
        if isinstance(self._outcome, BaseException):
            raise self._outcome
        return self._outcome

    def poll(self):
        return self.returncode

    def wait(self, *, timeout=None):
        self.wait_timeout = timeout


def test_opencode_worker_does_not_auto_approve_without_tools(monkeypatch):
    captured = {}

    def fake_popen(command, **kwargs):
        captured["command"] = command
        process = _FakeOpenCodeProcess(("review complete", ""))
        captured["process"] = process
        return process

    monkeypatch.setattr(opencode_worker.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(opencode_worker, "_terminate_process_tree", lambda _proc: None)

    result = opencode_worker.opencode_complete("Review this bounded input.")

    assert result == "review complete"
    assert "--auto" not in captured["command"]
    assert "--pure" in captured["command"]
    assert "--dangerously-skip-permissions" not in captured["command"]
    assert captured["process"].timeout == 600


def test_opencode_worker_auto_approves_only_with_full_tools(monkeypatch):
    captured = {}

    def fake_popen(command, **_kwargs):
        captured["command"] = command
        return _FakeOpenCodeProcess(("done", ""))

    monkeypatch.setattr(opencode_worker.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(opencode_worker, "_terminate_process_tree", lambda _proc: None)

    # SWARMS-001: La ejecución real debe fijar razonamiento y workspace.
    workspace = Path("C:/Proyectos/Migrador")
    opencode_worker.opencode_complete(
        "Implement it.",
        tools_policy="full",
        variant="high",
        cwd=workspace,
    )

    assert "--dangerously-skip-permissions" in captured["command"]
    assert "--auto" not in captured["command"]
    assert "--pure" not in captured["command"]
    assert captured["command"][captured["command"].index("--variant") + 1] == "high"


def test_opencode_worker_rejects_json_error_even_when_cli_exits_zero(monkeypatch):
    # SWARMS-002: OpenCode 1.14 puede devolver exit 0 para errores API JSON.
    payload = json.dumps({"type": "error", "error": {"data": {"message": "Authentication Failed"}}})

    def fake_popen(_command, **_kwargs):
        return _FakeOpenCodeProcess((payload, ""))

    monkeypatch.setattr(opencode_worker.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(opencode_worker, "_terminate_process_tree", lambda _proc: None)

    with pytest.raises(RuntimeError, match="Authentication Failed"):
        opencode_worker.opencode_complete("Implement it.")


def test_opencode_worker_extracts_json_text_events(monkeypatch):
    payload = "\n".join(
        [
            json.dumps({"type": "step_start"}),
            json.dumps({"type": "text", "part": {"text": "READY"}}),
            json.dumps({"type": "step_finish"}),
        ]
    )

    def fake_popen(command, **_kwargs):
        assert "--format" in command
        assert command[command.index("--format") + 1] == "json"
        return _FakeOpenCodeProcess((payload, ""))

    monkeypatch.setattr(opencode_worker.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(opencode_worker, "_terminate_process_tree", lambda _proc: None)

    assert opencode_worker.opencode_complete("Respond exactly READY.") == "READY"


def test_opencode_wrapper_cleans_up_after_normal_completion(monkeypatch):
    """SWARMS-003: normal completion still requests bounded tree cleanup."""
    process = _FakeOpenCodeProcess(("READY", ""))
    cleanup_pids = []

    monkeypatch.setattr(opencode_worker.subprocess, "Popen", lambda *args, **kwargs: process)
    monkeypatch.setattr(opencode_worker, "_terminate_process_tree", lambda proc: cleanup_pids.append(proc.pid))

    assert opencode_worker.opencode_complete("READY", timeout=7) == "READY"
    assert cleanup_pids == [process.pid]


@pytest.mark.parametrize(
    "failure",
    [subprocess.TimeoutExpired(["opencode", "run"], timeout=7), ValueError("launch failed")],
)
def test_opencode_wrapper_cleans_up_after_timeout_or_exception(monkeypatch, failure):
    """SWARMS-004: every failure path requests cleanup for the launched PID."""
    process = _FakeOpenCodeProcess(failure)
    cleanup_pids = []

    monkeypatch.setattr(opencode_worker.subprocess, "Popen", lambda *args, **kwargs: process)
    monkeypatch.setattr(opencode_worker, "_terminate_process_tree", lambda proc: cleanup_pids.append(proc.pid))

    with pytest.raises(type(failure)):
        opencode_worker.opencode_complete("READY", timeout=7)
    assert cleanup_pids == [process.pid]


def test_opencode_wrapper_requests_cleanup_only_for_launched_process_tree(monkeypatch):
    """SWARMS-005: cleanup is scoped to the wrapper's process tree, never global."""
    process = _FakeOpenCodeProcess(("READY", ""))
    cleanup_pids = []
    popen_calls = []

    def fake_popen(*args, **kwargs):
        popen_calls.append((args, kwargs))
        return process

    monkeypatch.setattr(opencode_worker.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(opencode_worker, "_terminate_process_tree", lambda proc: cleanup_pids.append(proc.pid))

    opencode_worker.opencode_complete("READY", timeout=7)

    assert len(popen_calls) == 1
    assert cleanup_pids == [process.pid]


def test_opencode_cleanup_attempts_group_cleanup_after_root_exits(monkeypatch):
    """SWARMS-006: an exited root may leave descendants requiring group cleanup."""
    process = _FakeOpenCodeProcess(("READY", ""))
    cleanup_commands = []

    monkeypatch.setattr(opencode_worker.os, "name", "nt")
    monkeypatch.setattr(
        opencode_worker.subprocess,
        "run",
        lambda command, **_kwargs: cleanup_commands.append(command),
    )

    opencode_worker._terminate_process_tree(process)

    assert cleanup_commands == [["taskkill", "/PID", str(process.pid), "/T", "/F"]]
    assert process.wait_timeout == 2


def test_opencode_resume_uses_exact_session_and_persists_event_id(monkeypatch, tmp_path):
    captured = {}
    status = tmp_path / "status.json"
    payload = json.dumps({"type": "text", "sessionID": "session-123", "part": {"text": "OK"}})

    def fake_popen(command, **_kwargs):
        captured["command"] = command
        return _FakeOpenCodeProcess((payload, ""))

    monkeypatch.setattr(opencode_worker.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(opencode_worker, "_terminate_process_tree", lambda _proc: None)
    assert opencode_worker.opencode_complete("Continue", resume_session="session-123", status_path=status) == "OK"
    assert captured["command"][captured["command"].index("--session") + 1] == "session-123"
    assert json.loads(status.read_text())["provider_session_id"] == "session-123"


def test_opencode_persists_session_before_reporting_failed_process(monkeypatch, tmp_path):
    status = tmp_path / "status.json"
    payload = json.dumps({"type": "step_start", "sessionID": "session-failed"})
    process = _FakeOpenCodeProcess((payload, "boom"))
    process.returncode = 2
    monkeypatch.setattr(opencode_worker.subprocess, "Popen", lambda *_args, **_kwargs: process)
    monkeypatch.setattr(opencode_worker, "_terminate_process_tree", lambda _proc: None)

    with pytest.raises(RuntimeError, match="exited 2"):
        opencode_worker.opencode_complete("Continue", status_path=status)

    assert json.loads(status.read_text())["provider_session_id"] == "session-failed"
