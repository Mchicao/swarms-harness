from types import SimpleNamespace

from scripts import kilo_worker


def test_kilo_worker_uses_pure_mode_without_tools(monkeypatch):
    captured = {}

    def fake_run(command, **kwargs):
        captured["command"] = command
        captured["timeout"] = kwargs["timeout"]
        captured["kwargs"] = kwargs
        return SimpleNamespace(returncode=0, stdout="KILO_OK", stderr="")

    monkeypatch.setattr(kilo_worker.subprocess, "run", fake_run)

    assert kilo_worker.kilo_complete("Review this.") == "KILO_OK"
    assert "--pure" in captured["command"]
    assert "--auto" not in captured["command"]
    assert captured["command"][captured["command"].index("-m") + 1] == "kilo/tencent/hy3:free"
    assert captured["kwargs"]["encoding"] == "utf-8"
    assert captured["kwargs"]["errors"] == "replace"
    assert captured["kwargs"]["env"]["XDG_DATA_HOME"].endswith("kilo-data")


def test_kilo_worker_auto_approves_only_with_full_tools(monkeypatch):
    captured = {}

    def fake_run(command, **_kwargs):
        captured["command"] = command
        return SimpleNamespace(returncode=0, stdout="KILO_OK", stderr="")

    monkeypatch.setattr(kilo_worker.subprocess, "run", fake_run)

    kilo_worker.kilo_complete("Implement this.", tools_policy="full")

    assert "--auto" in captured["command"]
    assert "--pure" not in captured["command"]
