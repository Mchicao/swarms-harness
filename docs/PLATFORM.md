# Platform Compatibility

The public SWARMS flow is Rust-first. Python remains available for legacy
compatibility, benchmarks, and telemetry tools.

## Supported

- Windows with Python 3.10+ and Git
- macOS with Python 3.10+ and Git
- Linux with Python 3.10+ and Git

Run the native runtime:

```powershell
cargo run --manifest-path rust/Cargo.toml -- doctor
```

If doctor passes, the default offline mock workflow can run without model credentials:

```powershell
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

## Legacy Compatibility

The old `scripts/parallel_swarm.ps1` adapter is no longer part of the public
flow. Use the Rust runtime for workflow execution.
