# Revisión adversarial de tareas largas

Estado: aceptada por verificación local del coordinador, 2026-07-16.

La primera salida del reviewer Gemini no fue aceptada como evidencia porque su
log inspeccionó un proyecto ajeno y no produjo este archivo. Esta revisión usa
el workspace SWARMS real y deja esa discrepancia registrada.

## Evidencia ejecutada

- `uv run pytest -q --basetemp .cache/pytest-recovery-review tests/test_workflow_runtime.py`
  — **46 passed**.
- `uv run ruff check scripts/workflow_runtime.py tests/test_workflow_runtime.py`
  — **All checks passed**.
- El runtime conserva checkpoints idempotentes, leases con heartbeat,
  recuperación de claims vencidos y reanudación sin repetir tareas completadas.
- `scripts/run_observability.py` y `rust/src/ui_main.rs` sólo leen snapshots,
  resultados, claims y eventos; no escriben estado ni lanzan workers.

## Criterios adversariales

La suite cubre interrupción/reinicio, heartbeat, claim único, expiración de
lease y estados parciales. El contrato observability añade sanitización de
rutas/errores, tolerancia a JSON corrupto y límite de logs; su suite dirigida
pasó **16 tests** y Ruff dirigido pasó.

## Brechas explícitas

- Rust no puede enlazar en este PC porque falta `link.exe` y el SDK MSVC;
  `cargo fmt --manifest-path rust/Cargo.toml -- --check` sí pasa después del
  spike UI.
- El spike UI queda implementado a nivel de fuente, pero `cargo test`/Clippy
  con sus dependencias no pueden completarse sin el linker/SDK.
- No se afirma fidelidad visual, multi-tenancy productivo ni ejecución DAX.
