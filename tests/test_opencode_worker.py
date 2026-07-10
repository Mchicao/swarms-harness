from types import SimpleNamespace

from scripts import opencode_worker


def test_opencode_worker_uses_supported_auto_approval_flag(monkeypatch):
    captured = {}

    def fake_run(command, **kwargs):
        captured["command"] = command
        captured["timeout"] = kwargs["timeout"]
        return SimpleNamespace(returncode=0, stdout="review complete", stderr="")

    monkeypatch.setattr(opencode_worker.subprocess, "run", fake_run)

    result = opencode_worker.opencode_complete("Review this bounded input.")

    assert result == "review complete"
    assert "--auto" in captured["command"]
    assert "--dangerously-skip-permissions" not in captured["command"]
    assert captured["timeout"] == 600
