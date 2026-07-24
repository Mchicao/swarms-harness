# CLAUDE.md

> [!IMPORTANT]
> **READ `AGENTS.md` FIRST!**
> Do not make any code edits or run commands without reading [AGENTS.md](file:///c:/Proyectos/SWARMS/AGENTS.md) in full. It contains the prime directives, project architecture, validation checklists, and system rules.

## Build and Tests
All builds and tests are run via Cargo:
```bash
cargo build --release --manifest-path rust/Cargo.toml --features ui-egui
cargo test --manifest-path rust/Cargo.toml
```
