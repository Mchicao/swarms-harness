# Platform Compatibility

The public SWARMS flow is the native Rust binary. Retained Python scripts are
legacy benchmark and telemetry tools; the workflow runtime and CLI never
invoke Python.

## Supported

- Windows with Git
- macOS with Git
- Linux with Git

Run the native runtime:

```powershell
cargo run --manifest-path rust/Cargo.toml -- doctor
```

If doctor passes, the default offline mock workflow can run without model credentials:

```powershell
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

## Legacy Compatibility

The old `scripts/parallel_swarm.ps1` and `scripts/swarm.py` runtimes are no
longer part of the public flow. Use the Rust runtime for workflow execution;
the installers build and install the native `swarms-rs` binary as `swarm`.