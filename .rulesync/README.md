# Coordinated agent context

Rulesync treats this directory as the canonical project source for shared
rules and MCP declarations. Keep credentials out of `mcp.json`; reference
environment-variable names in tool-specific fields instead.

Run the SWARMS CLI with `--sync-agent-context` to synchronize Skillshare first
and then generate Rulesync targets for AGENTS.md, Claude Code, Codex CLI,
Gemini CLI, OpenCode, and Antigravity. The flag is intentionally opt-in because
generation updates agent configuration files.
