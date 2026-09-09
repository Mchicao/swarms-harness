# Install SWARMS skills

SWARMS publishes exactly two agent skills with intentionally separate responsibilities:

1. `.skillshare/skills/multi-provider-agent-orchestration/` is **direct agent delegation**.
   It teaches an agent to ask Codex, Gemini, Claude, OpenCode/GLM or another agent to do bounded
   work directly, using the host's native subagent/delegation tool when available or the provider
   CLI otherwise. It must not create or run a SWARMS workflow.
2. `.skillshare/skills/swarms/` is **SWARMS runtime operation**. It teaches an agent to create,
   validate, run, resume and inspect durable workflows through the native Rust coordinator.

The decision boundary is simple: use direct delegation for one or a few agent calls that do not
need durable orchestration. Use the SWARMS runtime when the workflow needs dependencies, persisted
state/resume, concurrency/provider caps, observer/telemetry, retries or runtime-managed scaling.

`AGENTS.md` is the repository contract for agents editing SWARMS itself. It is not a replacement for
either installable skill.

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

Install the same two skill folders in the harness's Markdown-skill location. If a harness only
supports custom instructions, reference:

- `.skillshare/skills/multi-provider-agent-orchestration/SKILL.md` for direct delegation;
- `.skillshare/skills/swarms/SKILL.md` for SWARMS runtime operation.

Within this repository, Skillshare can distribute those two canonical skills to supported agent
harnesses. Generated/local agent directories are not source of truth and should not be committed.

## Validation

Test each responsibility independently in a fresh agent context.

Direct delegation test:

```text
Use the multi-provider-agent-orchestration skill to ask another available coding agent to review
this repository read-only. Do not use the SWARMS runtime.
```

Expected behavior: the agent uses a native subagent/delegation capability or directly invokes an
installed provider CLI. It does not create a workflow plan or call the SWARMS Rust binary.

Runtime test:

```text
Use the SWARMS skill to create a mock workflow contract, validate it and run it through the Rust
runtime.
```

Expected lifecycle begins with the Rust coordinator:

```powershell
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- review --plan <plan.json>
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan <plan.json> --force
```

The runtime test must use `mock` unless a real configured provider was explicitly authorized.