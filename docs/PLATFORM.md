# Platform Compatibility

The public SWARMS flow is Python-first.

## Supported

- Windows with Python 3.10+ and Git
- macOS with Python 3.10+ and Git
- Linux with Python 3.10+ and Git

Run:

```powershell
python scripts/swarm.py doctor
```

If doctor passes, the default offline mock workflow can run without model credentials:

```powershell
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

## Legacy Compatibility

`scripts/parallel_swarm.ps1` remains in the repository as a legacy/internal adapter for older worktree experiments. It requires PowerShell 7. It is not the public flow and agents should not call it directly unless a user asks for legacy compatibility.
