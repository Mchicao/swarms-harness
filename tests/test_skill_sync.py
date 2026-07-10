from pathlib import Path

import pytest

from scripts.sync_multi_provider_skill import sync_skill


def test_sync_skill_copies_canonical_tree_and_detects_drift(tmp_path: Path):
    source = tmp_path / "source"
    target = tmp_path / "target"
    (source / "references").mkdir(parents=True)
    (source / "SKILL.md").write_text("canonical skill\n", encoding="utf-8")
    (source / "references" / "guide.md").write_text("canonical guide\n", encoding="utf-8")

    assert sync_skill(source, target) is True
    assert (target / "SKILL.md").read_text(encoding="utf-8") == "canonical skill\n"
    assert (target / "references" / "guide.md").read_text(encoding="utf-8") == "canonical guide\n"
    assert sync_skill(source, target, check=True) is True

    (target / "SKILL.md").write_text("drifted\n", encoding="utf-8")

    assert sync_skill(source, target, check=True) is False


def test_sync_skill_refuses_non_skill_target(tmp_path: Path):
    source = tmp_path / "source"
    target = tmp_path / "unrelated"
    source.mkdir()
    target.mkdir()
    (source / "SKILL.md").write_text("canonical\n", encoding="utf-8")
    sentinel = target / "keep.txt"
    sentinel.write_text("keep\n", encoding="utf-8")

    with pytest.raises(ValueError, match="non-skill directory"):
        sync_skill(source, target)

    assert sentinel.read_text(encoding="utf-8") == "keep\n"


def test_sync_skill_refuses_overlapping_trees(tmp_path: Path):
    source = tmp_path / "source"
    source.mkdir()
    (source / "SKILL.md").write_text("canonical\n", encoding="utf-8")

    with pytest.raises(ValueError, match="must not overlap"):
        sync_skill(source, source / "nested")
