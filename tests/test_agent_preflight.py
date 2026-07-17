import json

from scripts.agent_preflight import discover_agents, route_findings


def test_preflight_reports_mock_ready_and_real_route_unverified(tmp_path, monkeypatch):
    config = {
        "providers": {
            "mock": {"enabled": True, "provider": "mock", "model": "mock", "wrapper": "mock"},
            "glm52": {"enabled": True, "provider": "opencode", "model": "glm", "wrapper": "opencode"},
        }
    }
    path = tmp_path / "router.json"
    path.write_text(json.dumps(config), encoding="utf-8")
    monkeypatch.setattr("scripts.agent_preflight.shutil.which", lambda command: command)
    monkeypatch.setattr("scripts.agent_preflight._auth_present", lambda _command: True)

    report = discover_agents(path)

    statuses = {record["id"]: record["status"] for record in report["routes"]}
    assert statuses == {"mock": "ready", "glm52": "unverified"}
    assert route_findings(report, {"glm52"}) == [{"code": "agent_unverified", "route": "glm52"}]


def test_preflight_disabled_route_is_not_a_dispatch_candidate(tmp_path):
    path = tmp_path / "router.json"
    path.write_text(
        json.dumps({"providers": {"glm52": {"enabled": False, "wrapper": "opencode"}}}),
        encoding="utf-8",
    )

    report = discover_agents(path)

    assert report["routes"][0]["status"] == "disabled"
    assert route_findings(report, set()) == []
