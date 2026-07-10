from types import SimpleNamespace

from scripts import opencode_worker


def test_opencode_worker_does_not_auto_approve_without_tools(monkeypatch):
    captured = {}

    def fake_run(command, **kwargs):
        captured["command"] = command
        captured["timeout"] = kwargs["timeout"]
        return SimpleNamespace(returncode=0, stdout="review complete", stderr="")

    monkeypatch.setattr(opencode_worker.subprocess, "run", fake_run)

    result = opencode_worker.opencode_complete("Review this bounded input.")

    assert result == "review complete"
    assert "--auto" not in captured["command"]
    assert "--pure" in captured["command"]
    assert "--dangerously-skip-permissions" not in captured["command"]
    assert captured["timeout"] == 600


def test_opencode_worker_auto_approves_only_with_full_tools(monkeypatch):
    captured = {}

    def fake_run(command, **_kwargs):
        captured["command"] = command
        return SimpleNamespace(returncode=0, stdout="done", stderr="")

    monkeypatch.setattr(opencode_worker.subprocess, "run", fake_run)

    opencode_worker.opencode_complete("Implement it.", tools_policy="full")

    assert "--auto" in captured["command"]
    assert "--pure" not in captured["command"]
