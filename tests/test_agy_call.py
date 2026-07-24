from types import SimpleNamespace

import pytest

from scripts import agy_call


def test_agy_safe_mode_uses_sandbox_without_permission_bypass(monkeypatch, tmp_path):
    captured = {}

    def fake_run(command, **kwargs):
        captured["command"] = command
        captured["cwd"] = kwargs["cwd"]
        return SimpleNamespace(returncode=0, stdout="SAFE", stderr="")

    monkeypatch.setattr(agy_call.subprocess, "run", fake_run)

    answer = agy_call.agy_complete(
        "Review only",
        skip_permissions=False,
        sandbox=True,
        cwd=tmp_path,
    )

    assert answer == "SAFE"
    assert "--sandbox" in captured["command"]
    assert "--dangerously-skip-permissions" not in captured["command"]
    assert "--add-dir" in captured["command"]
    assert str(tmp_path.resolve()) in captured["command"]
    assert captured["cwd"] == tmp_path


def test_agy_failure_does_not_poll_conversation_store(monkeypatch):
    monkeypatch.setattr(
        agy_call.subprocess,
        "run",
        lambda *_args, **_kwargs: SimpleNamespace(returncode=2, stdout="", stderr="auth failed"),
    )
    monkeypatch.setattr(agy_call, "_list_conv_dbs", lambda: [])
    monkeypatch.setattr(
        agy_call.time, "sleep", lambda _seconds: (_ for _ in ()).throw(AssertionError("must not sleep"))
    )

    with pytest.raises(RuntimeError, match="agy exited 2"):
        agy_call.agy_complete("Review only")
