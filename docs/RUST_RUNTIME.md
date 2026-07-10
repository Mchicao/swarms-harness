# Rust runtime

`rust/` contains the low-overhead, cross-platform coordinator for workflow-plan runs. It runs on Windows, macOS, and Linux using `std::process`; set `SWARMS_PYTHON` only when the platform's Python command is not discoverable.

The Rust coordinator owns plan parsing, local-router overlay, enabled-route checks, dependency waves, global and per-route caps, result files, and process dispatch. Python workers remain narrow adapters for provider CLIs, so their user-local OAuth/key stores are not copied into Rust, plans, or the repository.

```powershell
cargo run --release --manifest-path rust/Cargo.toml -- doctor
cargo run --release --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json
cargo run --release --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_example.json --force
cargo run --release --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

For CI or an offline workstation, use `--offline` after Cargo has built the lockfile dependencies once. The legacy Python CLI remains available for benchmark and telemetry compatibility while those non-plan surfaces are ported.
