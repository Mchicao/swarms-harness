# Preflight de agentes disponibles antes de ejecutar SWARMS

## Purpose / Big Picture

Después de clonar SWARMS y sincronizar su skill, el primer flujo debe mostrar
qué agentes locales existen, qué rutas están configuradas, si sus CLIs están
instaladas y si la autenticación sólo está presente o realmente puede
considerarse verificada. `mock` será la única ruta automáticamente lista sin
credenciales. Una ruta real no podrá iniciar workers si su disponibilidad es
`unverified` o `missing`.

## Progress

- [x] (2026-07-16 00:20 -04:00) Reconciliado el worktree y leído el flujo
  Python/Rust, router, doctor y pruebas existentes.
- [x] (2026-07-16 00:46 -04:00) Añadido inventario determinista de agentes y
  CLIs con estados `ready`, `unverified`, `missing_cli`, `missing_auth` y
  `disabled`.
- [x] (2026-07-16 00:46 -04:00) `run` ejecuta preflight antes de crear claims o
  workers y expone `preflight --format json|text`.
- [x] (2026-07-16 00:46 -04:00) Añadidas regresiones de rutas realistas y
  bloqueo sin dispatch; la suite completa queda en 104 tests verdes.
- [x] (2026-07-16 00:46 -04:00) `doctor`, README y guía documentan el primer
  paso. Python/Ruff pasan; Rust quedó instalado después de recuperar el
  toolchain interrumpido.
- [x] (2026-07-16 01:20 -04:00) Probes headless reales confirmaron `READY` en
  OpenCode/Z.AI y agy/Gemini; el worker OpenCode se corrigió para usar un cwd
  estable del workspace, porque el cwd temporal producía sólo `step_start`.
- [x] (2026-07-16 01:39 -04:00) El DAG real `dataviz-implementation-20260716`
  completó 4/4 tareas con dos raíces en paralelo y checkpoints reanudables.
- [x] (2026-07-16 01:42 -04:00) Regresión final SWARMS: 105 tests, Ruff dirigido
  y `git diff --check` pasan; no quedan workers OpenCode/agy activos.

## Surprises & Discoveries

- El runtime Python comprueba `enabled` y wrapper, pero no verifica que la CLI
  pueda autenticarse; esto permitió que OpenCode fallara dentro del worker.
- `agy` puede existir en PATH y aun así quedar sin respuesta headless; la
  presencia del binario no equivale a disponibilidad operativa.
- El coordinador Rust es el flujo público y debe compartir la misma regla de
  no dispatch; no basta con reparar sólo la compatibilidad Python.

## Decision Log

- Decision: separar `installed`, `auth_present` y `status`; no llamar al modelo
  automáticamente durante el preflight para no gastar cuota ni ejecutar código.
  `ready` queda reservado para mock y para rutas con evidencia local suficiente;
  las rutas reales se clasifican como `unverified` hasta un probe explícito.
- Decision: fallar antes de crear run directory o claims cuando una tarea usa
  una ruta real `unverified`, salvo `--allow-unverified-agents` explícito.
- Decision: inventariar también CLIs conocidas aunque no tengan ruta configurada,
  para que el usuario sepa qué instalar/configurar primero.

## Outcomes & Retrospective

 Python queda verificado. Rust tiene `cargo/rustc 1.97.0` y rustfmt válido,
 pero test/check/clippy requieren `link.exe` de Visual C++ Build Tools, que no
 está instalado en este PC; no se instaló un cambio de sistema no solicitado.

## Context and Orientation

El router seguro vive en `config/swarm_router.json`; la configuración privada
se fusiona desde `config/swarm_router.local.json`. El punto Python es
`scripts/swarm.py` y el coordinador público es `rust/src/main.rs`. Los workers
existentes no deben decidir disponibilidad por separado.

## Plan of Work

1. Crear `scripts/agent_preflight.py` con sólo biblioteca estándar.
2. Conectar `preflight` y el guard de `run` en Python.
3. Conectar el guard equivalente en Rust y devolver diagnóstico JSON/textual.
4. Añadir pruebas unitarias, CLI y regresión de no-dispatch.
5. Actualizar configuración/documentación y ejecutar validación completa.

## Concrete Steps

- `python -m scripts.swarm preflight --format json` debe listar agentes,
  rutas, CLIs, modelos, estado y razón sin iniciar workers.
- `python -m scripts.swarm run ...` debe ejecutar preflight antes de crear
  `.agent/swarm/runs/<id>`; las rutas reales `unverified` deben fallar con
  código 1 y findings accionables.
- `cargo run --manifest-path rust/Cargo.toml -- preflight` debe producir el
  mismo inventario básico y `run` debe rechazar rutas no verificadas.

## Validation and Acceptance

- `python -m pytest tests -q` pasa.
- `python scripts/swarm.py doctor` pasa en un clone sin credenciales.
- El plan mock ejecuta 4 tareas.
- Una prueba con OpenCode/agy configurado pero no verificado no crea claims,
  workers ni run directory.
- `cargo fmt --manifest-path rust/Cargo.toml -- --check` pasa. `cargo test`,
  `cargo check` y `cargo clippy` quedan pendientes por `link.exe` ausente.

## Idempotence and Recovery

El preflight es read-only y repetible. No modifica auth, router local ni
workspaces. Si una ruta está `unverified`, el usuario puede ejecutar un probe
externo explícito y después reintentar el mismo plan/run id.

## Artifacts and Notes

- `scripts/agent_preflight.py`
- `tests/test_agent_preflight.py`
- `docs/AGENT_PREFLIGHT.md`
- `rust/src/main.rs`

## Interfaces and Dependencies

```text
discover_agents(router_config, env, platform) -> PreflightReport
PreflightReport.agents[] = {
  id, route, provider, model, wrapper, command,
  installed, auth_present, enabled, status, reason
}
```

## Plan Revision Notes

- 2026-07-16: creado tras observar workers OpenCode/agy iniciados sin
  disponibilidad autenticada verificable.
- 2026-07-16: implementado y verificado en Python; Rust actualizado en paralelo
  con validación pendiente por falta de toolchain local.
- 2026-07-16: probes reales de OpenCode/agy y completaron el DAG; se usó
  `--resume` tras limpiar un lock obsoleto, sin sobrescribir checkpoints.
