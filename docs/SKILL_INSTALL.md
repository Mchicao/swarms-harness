# Install SWARMS skills

Use the two skills together without duplicating their responsibilities:

- `.skillshare/skills/swarms/` teaches an agent to create a SWARMS contract and operate the
  native Rust runtime and its observer UI.
- `.skillshare/skills/multi-provider-agent-orchestration/` teaches an agent how to split,
  delegate, observe and integrate work across coding agents. It is independent
  of this repository's runtime.

`AGENTS.md` is the repository contract once an agent is already working in
SWARMS. The installable skills make the two capabilities discoverable from
other projects.

## Codex

Copy or symlink both folders into the Codex skills directory:

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE\.codex\skills" | Out-Null
Copy-Item -Recurse -Force .\.skillshare\skills\swarms "$env:USERPROFILE\.codex\skills\swarms"
Copy-Item -Recurse -Force .\.skillshare\skills\multi-provider-agent-orchestration "$env:USERPROFILE\.codex\skills\multi-provider-agent-orchestration"
```

On macOS/Linux:

```bash
mkdir -p ~/.codex/skills
cp -R .skillshare/skills/swarms ~/.codex/skills/swarms
cp -R .skillshare/skills/multi-provider-agent-orchestration ~/.codex/skills/multi-provider-agent-orchestration
```

## Other agent harnesses

Copy each required folder into the harness's Markdown-skill location. If it
only supports custom instructions, reference `.skillshare/skills/swarms/SKILL.md` for
runtime operation and `.skillshare/skills/multi-provider-agent-orchestration/SKILL.md` for
delegation policy.

Within this repository, `skillshare sync -p` distributes exactly these two
canonical skills to Codex, Gemini, Antigravity and OpenCode.

## Validation

After installing, start a fresh agent context and ask:

```text
Use the SWARMS skill to create a mock workflow contract, run it, and open the observer UI.
```

It should run the Rust lifecycle, beginning with:

```powershell
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- review --plan <plan.json>
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan <plan.json> --force
```

It must use `mock` unless real configured providers were explicitly authorized.
It must not treat legacy timeout fields as worker-kill deadlines.
