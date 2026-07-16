# SWARMS Run Observer UI

Estado: spike read-only, feature-gated (`ui-egui`); enlace pendiente del
toolchain Windows local.
Fecha: 2026-07-16

Una única ventana nativa egui/eframe que observa un run de SWARMS leyendo el
contrato descrito en `docs/STATE_CONTRACT.md` y `docs/SWARM_UI_CONTRACT.md`. La
UI **nunca** escribe estado de run, **nunca** reclama tareas y **nunca** lanza
workers: es un observador puro del directorio
`.agent/swarm/runs/<run_id>/` (o el `--run-root` indicado).

## Artefactos

| Archivo | Rol |
| --- | --- |
| `rust/src/ui_main.rs` | Biblioteca `swarms_ui` (siempre compilada, solo `serde`) con el modelo read-only y `RunReader`; la ventana egui/eframe está inline bajo `ui_egui`. |
| `rust/src/ui_bin.rs` | Entry point mínimo del binario feature-gated `swarms-ui`; reutiliza `swarms_ui::ui_egui::run()`. |
| `rust/src/app.rs` | Archivo auxiliar presente en el checkout, pero no referenciado por un target de `rust/Cargo.toml` en el estado actual. |
| `rust/Cargo.toml` | Define la feature `ui-egui` (dependencia opcional `eframe` 0.35, Glow) y el binario `swarms-ui` con `required-features`. |
| `rust/tests/ui_state.rs` | Pruebas de integración del modelo (sin abrir ventana, sin eframe). |

## Compilación

Los siguientes son los comandos contractuales para el flujo sin feature. En el
entorno auditado actualmente fallan durante el enlace del host por falta de
`link.exe` (ver `docs/SWARM_UI_BUILD_ENVIRONMENT_AUDIT.md`); no se presentan
como comandos verdes hasta ejecutarlos con `exit 0`:

```powershell
cargo build --manifest-path rust/Cargo.toml
cargo test --manifest-path rust/Cargo.toml
```

Con la feature se compilan la parte egui/eframe inline de `ui_main.rs` y el
entry point `ui_bin.rs`; si el enlazado está disponible, también el binario
`swarms-ui`:

```powershell
cargo build --manifest-path rust/Cargo.toml --features ui-egui
cargo test --manifest-path rust/Cargo.toml --features ui-egui
cargo clippy --manifest-path rust/Cargo.toml --features ui-egui -- -D warnings
```

## Bloqueo de toolchain (Windows, `link.exe`)

El spike **no pudo enlazarse** en este entorno. El toolchain host activo es
`stable-x86_64-pc-windows-msvc` (rustc 1.97.0), y están instalados los targets
`x86_64-pc-windows-msvc` y `x86_64-pc-windows-gnullvm`, pero faltan el linker
MSVC, las bibliotecas del SDK y Visual Studio/Build Tools. Los proc-macros y
build scripts se enlazan para el host MSVC, por lo que añadir `--target
x86_64-pc-windows-gnullvm` tampoco evita el error. Error exacto:

```text
error: linker `link.exe` not found
  = note: program not found
note: the msvc targets depend on the msvc linker but `link.exe` was not found
note: please ensure that Visual Studio 2017 or later, or Build Tools for
      Visual Studio were installed with the Visual C++ option
```

Se intentó redirigir a `rust-lld` (`-C linker-flavor=lld-link -C linker=rust-lld`),
pero el SDK MSVC tampoco está disponible:

```text
rust-lld: error: could not open 'kernel32.lib': no such file or directory
rust-lld: error: could not open 'ntdll.lib': no such file or directory
rust-lld: error: could not open 'userenv.lib': no such file or directory
rust-lld: error: could not open 'ws2_32.lib': no such file or directory
rust-lld: error: could not open 'dbghelp.lib': no such file or directory
```

`cargo fmt --manifest-path rust/Cargo.toml -- --check` pasa. `cargo build`,
`cargo test` y `cargo clippy` fallan con `exit 101`, incluso en el intento
gnullvm, porque el lado host sigue necesitando `link.exe`. La verificación de
enlazado queda pendiente de un entorno con Build Tools para Visual Studio o
de un toolchain gnullvm completo con import libs/CRT verificadas; el target
instalado por sí solo no es suficiente (ver
`docs/SWARM_UI_BUILD_ENVIRONMENT_AUDIT.md`).

## Ventana

Una sola ventana, sin diálogos por agente:

| Zona | Contenido |
| --- | --- |
| Inferior | Badge de estado (`empty`/`loading`/`error`/`ready`), run, status, stages, tasks, results, concurrencia global y por provider, edad del último heartbeat, proveedor real activo, `log_cap`, eventos bufferizados. |
| Izquierda (si no se fijó `--run-id`) | Runs descubiertos bajo `--run-root`, activos primero, con task count y antigüedad. |
| Centro | Filtro por subcadena (case-insensitive), árbol virtualizado `run → stage → task → subagent`. Filas con color por estado y marca `⚠ stale`. |
| Derecha | Detalle de la fila seleccionada en la misma ventana: campos del snapshot, agente/claim, heartbeat, needs, artifacts, subagents, error y cola del `worker.log`. |

Seleccionar una fila de tarea o subagente actualiza el panel derecho. **No** se
abre ni enfoca ninguna ventana del SO adicional.

## Estados

- `empty`: el directorio del run no existe o no tiene checkpoints de tareas.
- `loading`: hay run seleccionado pero el primer `read()` aún no produjo contrato.
- `error`: `RunReader::open` rechazó el `run_id` (caracteres inseguros o escape
  de `run_root`); el mensaje saneado se muestra y la UI no cae.
- `stale`: por tarea, cuando su `heartbeat_unix_ms` es más antiguo que
  `workflow.json.heartbeat_interval_seconds`. Es una etiqueta visual: **nunca**
  muta el `status` del snapshot.

## Repintado bajo demanda

La UI **no** corre un loop fijo a 60 FPS. En cada frame se agenda el siguiente
repintado con `Context::request_repaint_after`:

- `500 ms` cuando el run está `running`;
- `2000 ms` en caso contrario.

La interacción del usuario (selección, filtro, scroll) dispara repintados
inmediatos por defecto en egui. La detección de cambios en disco es por sondeo
de metadatos (mtime de `tasks/`, tamaño y mtime de `events.jsonl`, presencia de
`report*.json`); **no** se añade un file watcher para mantener la superficie de
dependencias mínima y medible.

## Virtualización y límites de memoria

- El árbol central usa `ScrollArea::show_rows` con altura fija: sólo se renderizan
  las filas visibles (cumple `docs/UI_RUNTIME_EVALUATION.md`, caso 1.000/10.000
  filas).
- El `worker.log` se carga **solo** al seleccionar una tarea, y solo los últimos
  `MAX_LOG_BYTES` = 2 MiB (`read_worker_log_tail`). Cambiar de selección libera
  el anterior.
- `events.jsonl` se tail-ea por offset (solo líneas nuevas y completas); el
  buffer residente se acota a `4 * MAX_EVENT_ROWS`.
- Paths absolutos se relativizan al workspace/raíz; los foráneos colapsan a su
  basename. Errores se truncan a 1000 chars y se depuran de `Bearer …`, `sk-…`
  y patrones tipo `KEY=…` (`sanitize_error`, `sanitize_path` en `ui_main.rs`).

## Fuera de alcance (no implementado)

- Escribir estado, reclamar tareas, lanzar/detener workers o mutar planes.
- Colapsar/expandir nodos del árbol (la virtualización ya sostiene listas grandes).
- `file watcher` nativo, series temporales, export, multipaneles flotantes.
- WGPU, persistence, imágenes o gráficos (Glow-only hasta que se justifique medir).
- Iniciar ejecuciones desde la UI (`CREATE_NO_WINDOW`, redirección de stdout): eso
  requiere una decisión explícita posterior según `docs/UI_RUNTIME_EVALUATION.md`.

## Línea de comandos prevista (cuando se cablee `main`)

```text
swarms-ui [--run-root .agent/swarm/runs] [--run-id <id>]
```

Con `--run-id` se fija un único run y se omite el panel izquierdo. Sin él, la UI
lista los runs descubiertos bajo `--run-root` (por defecto
`.agent/swarm/runs`).
