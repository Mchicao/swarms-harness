"""Tests for the OpenAI-compatible worker used by the HY3 routes.

No real network calls: ``urllib.request.urlopen`` is monkeypatched. These
tests cover the three paths that matter for correctness — missing key (clean
error, no call made), successful completion (content parsed), and HTTP error
(useful message surfaced).
"""

import io
import json
import urllib.error

import pytest

from scripts import openai_compat_worker as worker


def test_missing_api_key_returns_clean_error_no_network(tmp_path, monkeypatch, capsys):
    # No key in the environment, so no real provider call should be attempted.
    monkeypatch.delenv("OPENROUTER_API_KEY", raising=False)
    monkeypatch.delenv("MY_TEST_KEY", raising=False)
    prompt = tmp_path / "prompt.txt"
    prompt.write_text("do something", encoding="utf-8")
    status = tmp_path / "status.json"

    def _fail(*a, **k):  # pragma: no cover - should never run
        raise AssertionError("urlopen must not be called without an API key")

    monkeypatch.setattr(worker.urllib.request, "urlopen", _fail)
    rc = worker.main(["--prompt", str(prompt), "--status", str(status), "--key-env", "MY_TEST_KEY"])

    assert rc == 2
    status_data = json.loads(status.read_text(encoding="utf-8"))
    assert status_data["success"] is False
    assert "MY_TEST_KEY" in status_data["error"]


def test_successful_completion_parses_content(tmp_path, monkeypatch):
    monkeypatch.setenv("OPENROUTER_API_KEY", "sk-test")
    prompt = tmp_path / "prompt.txt"
    prompt.write_text("Write a function.", encoding="utf-8")
    status = tmp_path / "status.json"

    captured = {}

    class FakeResp(io.BytesIO):
        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

    def fake_urlopen(req, timeout=None):
        captured["url"] = req.full_url
        captured["auth"] = req.headers.get("Authorization")
        body = req.data.decode("utf-8") if req.data else ""
        captured["payload"] = json.loads(body)
        resp_payload = {"choices": [{"message": {"content": "def f():\n    return 42\n"}}]}
        return FakeResp(json.dumps(resp_payload).encode("utf-8"))

    monkeypatch.setattr(worker.urllib.request, "urlopen", fake_urlopen)
    rc = worker.main(
        [
            "--prompt",
            str(prompt),
            "--status",
            str(status),
            "--model",
            "tencent/hy3:free",
            "--key-env",
            "OPENROUTER_API_KEY",
            "--base-url",
            "https://openrouter.ai/api/v1",
        ]
    )

    assert rc == 0
    assert captured["url"] == "https://openrouter.ai/api/v1/chat/completions"
    assert captured["auth"] == "Bearer sk-test"
    assert captured["payload"]["model"] == "tencent/hy3:free"
    status_data = json.loads(status.read_text(encoding="utf-8"))
    assert status_data["success"] is True
    assert status_data["model"] == "tencent/hy3:free"


def test_explicit_base_url_takes_priority_over_environment(monkeypatch):
    monkeypatch.setenv("CUSTOM_BASE_URL", "https://stale.example/v1")

    resolved = worker._resolve_base_url("CUSTOM_BASE_URL", "https://safe.example/v1")

    assert resolved == "https://safe.example/v1"


def test_remote_http_base_url_is_rejected(monkeypatch):
    monkeypatch.delenv("CUSTOM_BASE_URL", raising=False)

    with pytest.raises(ValueError, match="HTTPS"):
        worker._resolve_base_url("CUSTOM_BASE_URL", "http://remote.example/v1")


def test_loopback_http_base_url_is_allowed():
    assert worker._resolve_base_url("", "http://127.0.0.1:8080/v1") == "http://127.0.0.1:8080/v1"


def test_http_error_surfaces_useful_message(tmp_path, monkeypatch):
    monkeypatch.setenv("OPENROUTER_API_KEY", "sk-test")
    prompt = tmp_path / "prompt.txt"
    prompt.write_text("Write a function.", encoding="utf-8")
    status = tmp_path / "status.json"

    def fake_urlopen(req, timeout=None):
        raise urllib.error.HTTPError(
            url=req.full_url,
            code=429,
            msg="Too Many Requests",
            hdrs=None,
            fp=io.BytesIO(b'{"error":"rate limited"}'),
        )

    monkeypatch.setattr(worker.urllib.request, "urlopen", fake_urlopen)
    rc = worker.main(
        [
            "--prompt",
            str(prompt),
            "--status",
            str(status),
            "--key-env",
            "OPENROUTER_API_KEY",
            "--base-url",
            "https://x.example/v1",
        ]
    )

    assert rc == 2
    status_data = json.loads(status.read_text(encoding="utf-8"))
    assert status_data["success"] is False
    assert "HTTP 429" in status_data["error"]
