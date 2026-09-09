# Publishing Checklist

This workspace may live inside a larger Git repository. Confirm the Git root before publishing:

```powershell
git rev-parse --show-toplevel
```

If the output is not the SWARMS directory itself, publish from a clean copy or initialize a dedicated repository inside the SWARMS folder.

## Recommended First Publication

```powershell
cd C:\Proyectos\SWARMS
git init
git add .
git status --short
git commit -m "Initial public release"
git branch -M main
git remote add origin https://github.com/Mchicao/swarms-harness.git
git push -u origin main
```

Before pushing, run:

```powershell
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --release --manifest-path rust/Cargo.toml --all-features
cargo run --manifest-path rust/Cargo.toml -- doctor
cargo run --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json
cargo run --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --run-id verify-publish --global-max-concurrency 3 --provider-cap mock=3
python -m pytest tests -q
```

## Do Not Publish

- `.env`
- `config/*.local.json`
- `.agent/`
- `.cache/`
- `.swarm_worktrees/`
- generated prompts, logs, traces, reports, telemetry, and worktrees
- personal provider auth files

## Suggested Repository Metadata

- Description: `Quota-saving workflow harness for coding agents.`
- Topics: `coding-agents`, `llm`, `workflow`, `orchestration`, `rust`, `developer-tools`
- Website: `https://github.com/Mchicao`
