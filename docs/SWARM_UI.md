# SWARMS Native Runtime UI

Estado: implementada y validada en Windows; binario Rust opcional `swarms-ui`.
Fecha de verificación: 2026-07-16.

La UI es una ventana nativa Rust. Observa `.agent/swarm/runs/<run_id>/`, nunca
reclama tareas ni ejecuta providers. Sólo escribe por acciones explícitas del
usuario: steering, o inicialización/sincronización local de Skillshare para el
proyecto. El runtime público sigue siendo `swarms-rs`; la dependencia gráfica
sólo se compila con `ui-egui`.

## Ejecutar

```powershell
cargo run --release --manifest-path rust/Cargo.toml --bin swarms-ui --features ui-egui -- --run-id <run-id>
```

Opciones:

- `--run-root <ruta>`: raíz de runs; por defecto `.agent/swarm/runs`.
- `--run-id <id>`: abre un run concreto.
- `--ready-file <ruta>`: escribe una señal JSON atómica cuando la ventana inicia.
- `--bench-duration <segundos>`: cierra automáticamente una medición controlada.

## Diseño de bajo consumo

- `eframe 0.32` con renderer Glow; WGPU, WebView, persistencia e imágenes están
  deshabilitados.
- Sondeo de metadatos con biblioteca estándar: 1 segundo durante un run activo
  y 5 segundos cuando está inactivo. No existe un loop fijo a 60 FPS.
- Los JSON completos sólo se releen si cambia la firma de `workflow.json`,
  `tasks/`, `claims/`, `results/`, `events.jsonl` o `report*.json`.
- El índice de runs se actualiza cada 10 segundos, no en cada frame.
- El árbol aplanado queda cacheado y usa `ScrollArea::show_rows`; sólo se
  renderizan las filas visibles.
- El buffer mantiene como máximo 500 eventos recientes.
- Sólo se carga el log de la tarea seleccionada, limitado a sus últimos 256 KiB y con líneas virtualizadas.
- Errores se truncan a 1000 caracteres y se sanea material parecido a tokens.

Se usa polling en lugar de un file watcher para evitar otra dependencia y más
hilos. Añadir un watcher sólo se justifica si una medición reproducible muestra
que el sondeo de metadatos domina el consumo.

## Estado observado

La ventana usa una navegación compacta inspirada en herramientas de agentes
como T3 Code: tema oscuro sobrio, barra contextual y jerarquía
`Proyecto → run → etapa → tarea`. Los proyectos salen de `workflow.json`; runs
históricos conservan fallback por workspace o `Legacy runs`. La agrupación es
de sólo lectura y no mueve directorios.

Cada run muestra su última actividad en formato compacto (`now`, `8m ago`,
`3d ago`). Se toma el timestamp más reciente entre workflow, eventos, reporte y
snapshots de tareas, por lo que una reanudación queda reflejada. Los runs
históricos sin evidencia temporal muestran `unknown`.

La ventana ofrece `Overview`, `Tasks`, `Activity` y `Resources`. `Overview` dibuja el DAG por
etapas sin una dependencia gráfica adicional; `Tasks` muestra runs, etapas,
tareas y subagentes; `Activity` presenta el stream reciente. También muestra estado, provider, modelo,
intentos, dependencias, artefactos, errores y heartbeat. El runtime Rust escribe
snapshots `pending` e `in_progress` antes de ejecutar, y refresca
`heartbeat_unix_ms` sin crear un hilo adicional por worker.

Una tarea activa se marca `stale` cuando supera un intervalo sin heartbeat.
La etiqueta es visual y nunca altera el snapshot.

## Cuotas de planes

La barra lateral lee el mismo snapshot saneado que usa el scheduler y muestra
cada plan y ventana (`5h`, `7d`, etc.), porcentaje restante y edad del snapshot.
Cada plan ocupa una fila compacta con nombre humano, barra proporcional y el
porcentaje más restrictivo como estado principal; no se fabrican resets ni
ventanas que el monitor no haya publicado.
No abre OAuth, no lee tokens y no invoca Python. La captura sigue siendo
responsabilidad de `ai-usage-monitor`; un snapshot ausente o antiguo aparece
como desconocido en vez de inventar disponibilidad.

## Steer prompts

Al seleccionar una tarea activa con adapter Codex, OpenCode o Kilo, el panel
`Steer agent` permite encolar una nueva dirección de hasta 4000 caracteres. Los
workers CLI son turnos no interactivos: el prompt no interrumpe tokens que ya se
están generando. El runtime lo reclama al terminar el turno actual, reanuda la
sesión reportada por el provider y lo aplica antes de verificar artefactos. El
historial persiste bajo `steering/<task_id>/history.jsonl` con estado
`applied`, `rejected` o `failed`; los prompts no aparecen en eventos globales.

Los subagentes declarados en el plan son direccionables mediante su propia
tarea. Los subagentes internos que un provider no identifica siguen siendo
visibles como opacos y no se presentan falsamente como steerables.

## Recursos compartidos de agentes

`Resources` descubre instrucciones AGENTS, skills y nombres de servidores MCP
sin mostrar comandos, headers, variables ni credenciales. Se puede alternar
entre el alcance global y el proyecto del run seleccionado; los recursos del
proyecto se resuelven desde `workspace_root`, no desde el repositorio SWARMS.

Las skills compartidas usan `.skillshare/skills/` como fuente canónica. La
acción `Sync project skills` ejecuta `skillshare sync -p --json` sólo para ese
workspace y distribuye a los targets configurados. Los junctions generados se
presentan como una sola skill con sus agentes consumidores. La sincronización
global nunca se ejecuta implícitamente desde la UI.

## Validación

```powershell
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-features
cargo build --release --manifest-path rust/Cargo.toml --all-features
cargo tree --manifest-path rust/Cargo.toml --features ui-egui
```

El modelo read-only, la retención de eventos, límites de log, señales de inicio,
firmas de cambio y frecuencias de polling tienen pruebas automáticas. CI compila
y prueba todas las features en Windows, Linux y macOS.

Medición local reproducible del release en Windows, observando un run terminado
y descartando 5 segundos de calentamiento:

- 10,22 s de muestra estable: 0,000 s de CPU acumulada según `Get-Process`;
- working set mediano: 159,22 MiB; máximo: 159,41 MiB;
- GPU: mediana 0,000%, máximo 0,051% en 125 muestras de GPU Engine;
- `swarms-ui.exe`: 4,10 MiB; `swarms-rs.exe`: 2,34 MiB.

Estas cifras son una medición de este equipo, no un presupuesto universal. El
driver gráfico domina la RAM de la ventana; CPU y GPU quedan prácticamente en
reposo gracias al repaint bajo demanda.

La auditoría histórica del linker permanece en
`docs/SWARM_UI_BUILD_ENVIRONMENT_AUDIT.md`; describe un bloqueo anterior y no
representa el estado actual del equipo.

## Límites actuales

- Iniciar, detener o reanudar workflows desde la UI.
- Inyectar texto en una generación CLI que ya está produciendo tokens.
- Steering de Hermes, Agy, OpenAI-compatible o agentes internos sin sesión
  direccionable.
- Series temporales, imágenes o ventanas flotantes.
- WGPU o un frontend web.
- Mantener logs completos en memoria.
