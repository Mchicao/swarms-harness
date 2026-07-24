# Contributing

Thanks for considering a contribution. SWARMS is alpha software, so small, well-scoped changes are preferred.

## Ground Rules

- Keep the default path offline and free.
- Do not add secrets, tokens, local auth files, generated traces, or provider logs.
- Do not enable paid providers in committed config.
- Treat the native Rust binary as the public workflow runtime and CLI.
- Treat Python modules as legacy support, telemetry, benchmark, migration, or compatibility tooling unless a change explicitly narrows that surface further.
- Keep docs honest about experimental behavior and trust boundaries.

## Local Setup

Install the Rust toolchain with `rustfmt` and `clippy`, then install Python development dependencies for the retained support tools:

```powershell
rustup component add rustfmt clippy
python -m pip install -e ".[dev,yaml]"
```

Run the native CLI from the repository root:

```powershell
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json
```

## Required Checks

Run these before opening a pull request that affects the Rust runtime:

```powershell
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --release --manifest-path rust/Cargo.toml --all-features
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json
cargo run --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_example.json --force --run-id verify-contrib-dry
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --run-id verify-contrib --global-max-concurrency 3 --provider-cap mock=3
```

Run these checks when changing retained Python tooling:

```powershell
python -m py_compile scripts/swarm.py scripts/plan_review.py scripts/workflow_runtime.py scripts/doctor.py scripts/mock_worker.py
python -m ruff check .
python -m ruff format --check .
python -m pytest tests -q
```

## Provider Changes

Provider adapters should include tests that use fake or mock providers. Do not add tests that require paid credentials in CI. Document dangerous capability flags, cancellation behavior, retry classification, and provider-specific protocol assumptions.
