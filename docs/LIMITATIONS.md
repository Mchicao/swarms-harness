# Limitations

SWARMS is a personal workflow released publicly. The offline path is the safest first run; real-provider routes depend on local CLIs, local credentials, and provider limits.

## What Works Today

- The public CLI supports `doctor`, `review`, `dry-run`, and `run`.
- Static plan review catches missing goals, missing task ids, duplicate ids, unsafe artifact paths, missing dependencies, blocked premium routes, and zero provider capacity.
- The deterministic runtime can execute dependency-aware task waves.
- Provider caps and global concurrency are enforced for the runtime scheduler.
- Real routes must exist and be enabled in the selected router config.
- `tools_policy=none` does not add permission-bypass flags; AGY also uses its sandbox flag.
- The offline `mock` worker supports tests, demos, and CI without credentials.
- Run reports are written under
  `<workspace-root>/.agent/swarm/runs/<run-id>/`, which is ignored by Git in a
  correctly configured target workspace.

## What Needs Local Setup

- Real provider routes such as `glm52`, `gemini_flash`, `codex`, and `claude`.
- Token and cost telemetry from external CLIs.
- Multi-worktree execution and merge coordination.
- Automatic conflict resolution between parallel coding workers.
- Security boundaries for untrusted model-generated code.

## What SWARMS Does Not Guarantee

- It does not sandbox real providers.
- It does not prevent a configured coding CLI from editing files unless the adapter enforces that behavior.
- It executes each task's bounded `verify` commands, records their evidence and
  prevents feature-producing roles from completing without a passing verify
  command.
- Declared artifacts must exist, remain inside the workspace and be fresh; any
  declared `protected` paths must remain unchanged. This is not a full final
  diff allowlist: a real provider can still modify additional unprotected
  workspace files unless its own sandbox or worktree boundary prevents it.
- It does not guarantee token savings for every task.
- It does not guarantee that cheap workers produce acceptable code.
- It does not replace human review for security-sensitive changes.

## Safe Public Demo Boundary

The committed demo must remain offline:

```powershell
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Any demo that requires external credentials should live in private local config or future optional docs clearly marked as provider-specific.
