# Rust runtime

`rust/` contains the low-overhead, cross-platform coordinator for workflow-plan runs. It runs on Windows, macOS, and Linux using `std::process`; set `SWARMS_PYTHON` only when the platform's Python command is not discoverable.

The Rust coordinator owns plan parsing, local-router overlay, enabled-route checks, dependency waves, global and per-route caps, result files, and process dispatch. Python workers remain narrow adapters for provider CLIs, so their user-local OAuth/key stores are not copied into Rust, plans, or the repository.

```powershell
cargo run --release --manifest-path rust/Cargo.toml -- doctor
cargo run --release --manifest-path rust/Cargo.toml -- review --plan docs/workflow_plan_example.json
cargo run --release --manifest-path rust/Cargo.toml -- dry-run --plan docs/workflow_plan_example.json --force
cargo run --release --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Para coordinar otro repositorio sin copiar el harness, fija su raíz de trabajo:

```powershell
# SWARMS-RS-001: Ejecuta workers en el repositorio objetivo.
cargo run --release --manifest-path rust/Cargo.toml -- run --plan C:\ruta\plan.json --workspace-root C:\ruta\proyecto --force
```

### Windows sin Visual Studio Build Tools

El target `x86_64-pc-windows-gnullvm` puede compilar con `rust-lld.exe` incluido
en el propio toolchain, sin instalar un linker global. Si Rust se instaló de
forma aislada bajo `.cache/`, usa:

```powershell
# SWARMS-RS-002: selecciona el toolchain aislado y su linker LLVM incluido.
$env:CARGO_HOME = (Resolve-Path '.cache/cargo').Path
$env:RUSTUP_HOME = (Resolve-Path '.cache/rustup').Path
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = (Resolve-Path '.cache/rustup/toolchains/stable-x86_64-pc-windows-gnullvm/lib/rustlib/x86_64-pc-windows-gnullvm/bin/rust-lld.exe').Path

# SWARMS-RS-003: ejecuta pruebas y Clippy sin modificar PATH global.
& .cache/cargo/bin/cargo.exe +stable-x86_64-pc-windows-gnullvm test --manifest-path rust/Cargo.toml
& .cache/cargo/bin/cargo.exe +stable-x86_64-pc-windows-gnullvm clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
```

Esta configuración pasó 4 pruebas Rust y Clippy el 2026-07-15. `.cache/` está
ignorado y no contiene artefactos distribuibles del proyecto.

For CI or an offline workstation, use `--offline` after Cargo has built the lockfile dependencies once. The legacy Python CLI remains available for benchmark and telemetry compatibility while those non-plan surfaces are ported.
