---
name: swarms-project-coordination
description: Shared project context for agents working on the native Rust SWARMS runtime and UI.
metadata:
  targets: [codex, gemini, antigravity, opencode]
---

# SWARMS project coordination

- Treat `AGENTS.md` as the project contract.
- Use the Rust runtime for workflow operations; Python is legacy tooling only.
- Keep provider credentials and local configuration out of commits and reports.
- Prefer project skills from `.skillshare/skills/` over similarly named global skills.
- Before completion, run the validation commands listed in `AGENTS.md`.
