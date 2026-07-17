from pathlib import Path

from scripts import gemini_worker


def test_gemini_none_policy_is_sandboxed_without_permission_bypass(monkeypatch):
    captured = {}

    def fake_complete(_prompt, **kwargs):
        captured.update(kwargs)
        assert kwargs["cwd"]
        return "SAFE"

    monkeypatch.setattr(gemini_worker, "agy_complete", fake_complete)

    assert gemini_worker.gemini_complete("Review", tools_policy="none") == "SAFE"
    assert captured["skip_permissions"] is False
    assert captured["sandbox"] is True


def test_gemini_full_policy_explicitly_allows_permission_bypass(monkeypatch):
    captured = {}

    def fake_complete(_prompt, **kwargs):
        captured.update(kwargs)
        return "DONE"

    monkeypatch.setattr(gemini_worker, "agy_complete", fake_complete)

    # SWARMS-004: Gemini debe operar en el repositorio objetivo declarado.
    workspace = Path("C:/Proyectos/Migrador")
    assert gemini_worker.gemini_complete("Implement", tools_policy="full", cwd=workspace) == "DONE"
    assert captured["skip_permissions"] is True
    assert captured["sandbox"] is False
    assert captured["cwd"] == workspace


def test_gemini_retries_one_transient_recursion_error(monkeypatch):
    calls = 0

    def fake_complete(_prompt, **_kwargs):
        nonlocal calls
        calls += 1
        if calls == 1:
            raise RecursionError("transient")
        return "RECOVERED"

    monkeypatch.setattr(gemini_worker, "agy_complete", fake_complete)
    monkeypatch.setattr(gemini_worker.time, "sleep", lambda _seconds: None)

    assert gemini_worker.gemini_complete("Review", tools_policy="none") == "RECOVERED"
    assert calls == 2


def test_gemini_forwards_exact_resume_session(monkeypatch):
    captured = {}

    def fake_complete(_prompt, **kwargs):
        captured.update(kwargs)
        return "RESUMED"

    monkeypatch.setattr(gemini_worker, "agy_complete", fake_complete)
    assert gemini_worker.gemini_complete(
        "Continue", tools_policy="full", resume_session="conversation-123"
    ) == "RESUMED"
    assert captured["conversation_id"] == "conversation-123"
