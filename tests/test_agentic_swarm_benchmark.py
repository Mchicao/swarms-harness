import json

from scripts.run_agentic_swarm_benchmark import PROJECT_ROOT, VARIANTS, AgenticSwarmBenchmark


def test_agentic_micro_tasks_are_multistage_and_verifiable():
    tasks = json.loads((PROJECT_ROOT / "docs" / "agentic_swarm_micro_tasks.json").read_text(encoding="utf-8"))
    assert len(tasks) >= 3
    for task in tasks:
        assert task["verify_command"].startswith("python -m pytest")
        assert len(task["stages"]) >= 3
        rendered = AgenticSwarmBenchmark(
            PROJECT_ROOT / "docs" / "agentic_swarm_micro_tasks.json",
            ["swarm_auto"],
            1,
        ).render_task_file(task)
        assert "@needs(" in rendered
        assert "- [ ]" in rendered


def test_agentic_variants_do_not_include_claude():
    assert set(VARIANTS) == {
        "mock_swarm",
        "glm52_only",
        "gemini_flash_only",
        "swarm_auto",
        "gpt55_medium_orchestrate_glm52",
    }
    for cfg in VARIANTS.values():
        assert "claude" not in cfg["strategy"]


def test_codex_medium_orchestrator_variant_uses_glm_workers():
    cfg = VARIANTS["gpt55_medium_orchestrate_glm52"]
    assert cfg["orchestrator"] == "codex_medium"
    assert cfg["strategy"] == "glm-only"
    assert cfg["workers"] == 3


def test_default_mock_swarm_is_offline():
    cfg = VARIANTS["mock_swarm"]
    assert cfg["strategy"] == "mock-only"
    assert cfg["workers"] == 3


def test_benchmark_scope_allows_only_micro_task_paths():
    changed = [
        "bench_apps/reshard/compress.py",
        "bench_tests/test_bench_reshard.py",
        "docs/bench_notes/reshard_plan.md",
        "core/onbase_powerbi/pipeline.py",
        "tests/conftest.py",
    ]

    assert AgenticSwarmBenchmark.disallowed_benchmark_changes(changed) == [
        "core/onbase_powerbi/pipeline.py",
        "tests/conftest.py",
    ]
