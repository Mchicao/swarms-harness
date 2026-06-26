# Limitations

SWARMS is alpha software. The project is publicable as an offline MVP, not as a complete multi-provider coding platform.

## What Works Today

- The public CLI supports `doctor`, `review`, `dry-run`, and `run`.
- Static plan review catches missing goals, missing task ids, duplicate ids, unsafe artifact paths, missing dependencies, blocked premium routes, and zero provider capacity.
- The deterministic runtime can execute dependency-aware task waves.
- Provider caps and global concurrency are enforced for the runtime scheduler.
- The offline `mock` worker supports tests, demos, and CI without credentials.
- Reports are written under `.agent/`, which is ignored by Git.

## What Is Experimental

- Real provider routes such as `glm52`, `gemini_flash`, `codex`, and `claude`.
- Token and cost telemetry from external CLIs.
- Multi-worktree execution and merge coordination.
- Automatic conflict resolution between parallel coding workers.
- Security boundaries for untrusted model-generated code.

## What SWARMS Does Not Guarantee

- It does not sandbox real providers.
- It does not prevent a configured coding CLI from editing files unless the adapter enforces that behavior.
- It does not guarantee token savings for every task.
- It does not guarantee that cheap workers produce acceptable code.
- It does not replace human review for security-sensitive changes.

## Safe Public Demo Boundary

The committed demo must remain offline:

```powershell
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Any demo that requires external credentials should live in private local config or future optional docs clearly marked as provider-specific.
