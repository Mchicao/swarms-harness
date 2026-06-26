import json
from pathlib import Path

from scripts.utils import token_telemetry as tt


def test_parse_openai_like_usage_reads_cache_and_reasoning():
    usage = tt.parse_openai_like_usage(
        {
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 20,
                "prompt_tokens_details": {"cached_tokens": 30, "cache_write_tokens": 10},
                "completion_tokens_details": {"reasoning_tokens": 5},
            }
        }
    )
    assert usage["input"] == 100
    assert usage["cache_read_input_tokens"] == 30
    assert usage["cache_write_input_tokens"] == 10
    assert usage["output"] == 20
    assert usage["reasoning_output_tokens"] == 5


def test_missing_usage_source_when_no_tokens(tmp_path, monkeypatch):
    telemetry_file = tmp_path / "telemetry.jsonl"
    monkeypatch.setattr(tt, "TELEMETRY_FILE", telemetry_file)
    event = tt.record_event(
        run_id="run-1",
        benchmark_id="bench-1",
        phase="swarm",
        provider="antigravity_cli",
        model="gemini-3.5-flash",
        role="overhead",
        task_id="task-1",
        usage_source="estimated",
    )
    assert event["usage_source"] == "missing"
    assert json.loads(telemetry_file.read_text())["usage_source"] == "missing"


def test_record_event_persists_routing_metadata(tmp_path, monkeypatch):
    telemetry_file = tmp_path / "telemetry.jsonl"
    monkeypatch.setattr(tt, "TELEMETRY_FILE", telemetry_file)
    event = tt.record_event(
        run_id="run-1",
        benchmark_id="bench-1",
        phase="swarm",
        provider="zai_coding",
        model="glm-5.2",
        role="worker",
        task_id="task-1",
        route_id="glm52",
        routing_method="auto",
        routing_reason="best configured score",
    )
    persisted = json.loads(telemetry_file.read_text())
    assert event["route_id"] == "glm52"
    assert persisted["routing_method"] == "auto"


def test_cost_details_include_cache_write_and_reasoning(monkeypatch):
    monkeypatch.setattr(
        tt,
        "DEFAULT_CATALOG",
        {
            "schema_version": 1,
            "catalog_version": "test",
            "currency": "USD",
            "models": {
                "glm-5.2": {
                    "provider": "opencode",
                    "plan": "test",
                    "input_per_1m": 0.10,
                    "cache_read_input_per_1m": 0.05,
                    "cache_write_input_per_1m": 0.10,
                    "output_per_1m": 0.20,
                    "reasoning_output_per_1m": 0.20,
                    "pricing_status": "test",
                }
            },
        },
    )
    monkeypatch.setattr(tt, "CATALOG_FILE", Path("missing-catalog.json"))
    monkeypatch.setattr(tt, "LOCAL_CATALOG_FILE", Path("missing-local-catalog.json"))
    usage = tt.normalize_usage(
        input_tokens=1_000_000,
        cache_read_tokens=100_000,
        cache_write_tokens=50_000,
        output_tokens=20_000,
        reasoning_tokens=10_000,
    )
    details = tt.calculate_cost_details("glm-5.2", usage)
    assert details["total"] is not None
    assert details["cache_write_input_tokens"] > 0
    assert details["reasoning_output_tokens"] > 0


def test_parse_codex_log_uses_latest_usage(tmp_path: Path):
    log = tmp_path / "codex.jsonl"
    log.write_text(
        "\n".join(
            [
                json.dumps({"usage": {"prompt_tokens": 10, "completion_tokens": 1}}),
                json.dumps({"usage": {"prompt_tokens": 50, "completion_tokens": 5}}),
            ]
        ),
        encoding="utf-8",
    )
    usage = tt.parse_codex_log(log)
    assert usage["input"] == 50
    assert usage["output"] == 5
