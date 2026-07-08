import json

from scripts import smart_router


def write_config(tmp_path, overrides=None):
    config = smart_router.load_config(smart_router.DEFAULT_CONFIG)
    if overrides:
        config.update(overrides)
    path = tmp_path / "router.json"
    path.write_text(json.dumps(config), encoding="utf-8")
    return path


def test_directive_wins_over_strategy(tmp_path):
    config_path = write_config(tmp_path)
    route = smart_router.choose_route(
        "- [ ] [backend] [[route:codex]] fix critical bug",
        strategy="auto",
        config_path=config_path,
        limits_path=tmp_path / "missing.yaml",
        metrics_path=tmp_path / "missing.json",
    )
    assert route["id"] == "codex"
    assert route["routing_method"] == "directive"


def test_cost_preference_routes_simple_work_to_free(tmp_path):
    config_path = write_config(tmp_path)
    route = smart_router.choose_route(
        "- [ ] [lite] add simple docstring and format comments",
        strategy="auto",
        config_path=config_path,
        limits_path=tmp_path / "missing.yaml",
        metrics_path=tmp_path / "missing.json",
    )
    assert route["id"] == "mock"


def test_role_based_keeps_configured_role_route(tmp_path):
    config_path = write_config(tmp_path)
    route = smart_router.choose_route(
        "- [ ] [qa] review tests",
        strategy="role-based",
        config_path=config_path,
        limits_path=tmp_path / "missing.yaml",
        metrics_path=tmp_path / "missing.json",
    )
    assert route["id"] == "mock"
    assert route["routing_method"] == "role-based"


def test_default_auto_routes_to_mock(tmp_path):
    config_path = write_config(tmp_path)
    route = smart_router.choose_route(
        "- [ ] [backend] implement API",
        strategy="auto",
        config_path=config_path,
        limits_path=tmp_path / "missing.yaml",
        metrics_path=tmp_path / "missing.json",
    )
    assert route["id"] == "mock"
    assert route["provider"] == "mock"


def test_glm_only_uses_opencode_glm52(tmp_path):
    config_path = write_config(tmp_path)
    route = smart_router.choose_route(
        "- [ ] [backend] implement API",
        strategy="glm-only",
        config_path=config_path,
        limits_path=tmp_path / "missing.yaml",
        metrics_path=tmp_path / "missing.json",
    )
    assert route["provider"] == "opencode"
    assert route["wrapper"] == "opencode"
    assert route["model"] == "zai-coding-plan/glm-5.2"
    assert route["canonical_model"] == "glm-5.2"


def test_ambiguous_directive_falls_back_to_auto(tmp_path):
    config_path = write_config(tmp_path)
    route = smart_router.choose_route(
        "- [ ] [backend] [[route:codex]] [[route:glm52]] implement API",
        strategy="auto",
        config_path=config_path,
        limits_path=tmp_path / "missing.yaml",
        metrics_path=tmp_path / "missing.json",
    )
    assert route["routing_method"] == "auto"
    assert "ambiguous" in route["routing_reason"]
