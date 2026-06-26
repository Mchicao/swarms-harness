# Install The SWARMS Skill

`AGENTS.md` and `CLAUDE.md` help agents after they are inside this repository. The installable skill helps agents know how to use SWARMS from other projects too.

The skill lives at:

```text
skills/swarms/
```

## Codex

Copy or symlink the skill folder into your Codex skills directory:

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE\.codex\skills" | Out-Null
Copy-Item -Recurse -Force .\skills\swarms "$env:USERPROFILE\.codex\skills\swarms"
```

Or from macOS/Linux:

```bash
mkdir -p ~/.codex/skills
cp -R skills/swarms ~/.codex/skills/swarms
```

## Claude Code

Claude Code does not use the same skill loader everywhere, so keep both:

- `CLAUDE.md` in the SWARMS repo;
- `skills/swarms/SKILL.md` copied into any agent/skill system that supports Markdown skills.

If your tool supports custom instructions, paste or reference `skills/swarms/SKILL.md`.

## Skillshare

If you use Skillshare or another multi-agent skill sync tool, point it at `skills/swarms/` and sync to Codex, Claude, OpenCode, or other supported targets.

## Validation

After installing, start a fresh agent context and ask:

```text
Use the SWARMS skill to run the offline doctor and mock benchmark.
```

The agent should run:

```powershell
python scripts\swarm.py doctor
python scripts\swarm.py run --plan docs\workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

It should not run Gemini, Codex, Claude, OpenCode, or any paid provider unless you explicitly ask.
