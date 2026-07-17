from __future__ import annotations

import pytest

from scripts.context_sync import ContextSyncError, sync_agent_context


def test_context_sync_runs_allowlisted_noninteractive_commands(monkeypatch, tmp_path):
    source = tmp_path / ".rulesync"
    (source / "rules").mkdir(parents=True)
    (source / "rules" / "policy.md").write_text("# Policy\n", encoding="utf-8")
    (source / "mcp.json").write_text('{"mcpServers": {}}\n', encoding="utf-8")
    commands = []

    def fake_run(command, **_kwargs):
        commands.append(command)
        return {"returncode": 0, "stdout": "{}", "stderr": ""}

    monkeypatch.setattr("scripts.context_sync._run", fake_run)
    report = sync_agent_context(tmp_path, ["claudecode", "codexcli", "opencode"])

    assert commands[0] == ["skillshare", "sync", "--all", "--json", "--dry-run"]
    assert commands[1][:4] == ["rulesync", "generate", "--targets", "claudecode,codexcli,opencode"]
    assert "--dry-run" in commands[1]
    assert commands[2] == ["skillshare", "sync", "--all", "--json"]
    assert commands[3][:4] == ["rulesync", "generate", "--targets", "claudecode,codexcli,opencode"]
    assert "--output-roots" in commands[3]
    assert str(tmp_path.resolve()) in commands[3]
    assert "mcp" in commands[3][commands[3].index("--features") + 1]
    assert report["targets"] == ["claudecode", "codexcli", "opencode"]


def test_context_sync_requires_canonical_rulesync_source(tmp_path):
    with pytest.raises(ContextSyncError, match=".rulesync"):
        sync_agent_context(tmp_path, ["codexcli"])


def test_context_sync_rejects_unknown_target(tmp_path):
    (tmp_path / ".rulesync").mkdir()
    with pytest.raises(ContextSyncError, match="Unsupported"):
        sync_agent_context(tmp_path, ["shell-injection"])


def test_context_sync_expands_human_aliases(monkeypatch, tmp_path):
    source = tmp_path / ".rulesync"
    (source / "rules").mkdir(parents=True)
    (source / "rules" / "policy.md").write_text("# Policy\n", encoding="utf-8")
    (source / "mcp.json").write_text('{"mcpServers": {}}\n', encoding="utf-8")
    commands = []
    monkeypatch.setattr(
        "scripts.context_sync._run",
        lambda command, **_kwargs: commands.append(command) or {"returncode": 0, "stdout": "", "stderr": ""},
    )

    report = sync_agent_context(tmp_path, ["claude", "codex", "opencode", "agy"])

    assert report["targets"] == ["claudecode", "codexcli", "opencode", "agentsmd", "agentsskills"]
    assert "claudecode,codexcli,opencode,agentsmd,agentsskills" in commands[-1]


def test_context_sync_rejects_literal_mcp_secrets(tmp_path):
    source = tmp_path / ".rulesync"
    (source / "rules").mkdir(parents=True)
    (source / "rules" / "policy.md").write_text("# Policy\n", encoding="utf-8")
    (source / "mcp.json").write_text(
        '{"mcpServers":{"unsafe":{"env":{"API_KEY":"literal-secret"}}}}',
        encoding="utf-8",
    )

    with pytest.raises(ContextSyncError, match="literal credential"):
        sync_agent_context(tmp_path, ["codex"])
