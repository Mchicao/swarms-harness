# Evaluación del runtime para la UI de administración

Estado: spike read-only implementado; enlace pendiente del toolchain Windows
Fecha de verificación: 2026-07-16

## Decisión

El primer *spike* se implementó con **Rust + egui/eframe**, como un segundo
binario opcional y de sólo lectura. La UI debe ser una única ventana nativa, sobria y
densa, que observe el contrato de estado en `.agent/swarm/runs/`. No debe
embeber ni reemplazar al coordinador, y el binario CLI actual debe conservar
exactamente sus dependencias y comportamiento cuando no se active la feature.

Esta es una recomendación de corte, no una afirmación de que egui consume menos
RAM que Slint o Tauri: no hay un benchmark oficial comparable para esta carga.
La elección final se confirma con el protocolo reproducible de este documento.

Razones para empezar por egui/eframe:

- Es Rust nativo y su licencia `MIT OR Apache-2.0` encaja limpiamente con el
  repositorio MIT. `eframe` soporta Windows y Linux y permite desactivar sus
  features por defecto, que actualmente incluyen WGPU, X11, Wayland y soporte
  web que no deben entrar todos sin medir.
- No incorpora HTML, JavaScript, un WebView ni un puente IPC. Esto minimiza las
  capas que habrá que perfilar y mantener para una UI local de observación.
- Puede dormir entre eventos. `request_repaint_after` existe explícitamente
  para evitar repintados innecesarios, y `ScrollArea::show_rows` permite
  virtualizar listados grandes. El modo inmediato sigue siendo un riesgo de CPU
  si se solicita repintado continuo; el diseño prohíbe hacerlo.
- El repositorio tiene instalados Rust 1.97.0 y los targets
  `x86_64-pc-windows-msvc`/`x86_64-pc-windows-gnullvm`, pero la auditoría del
  2026-07-16 no pudo reproducir el enlace: faltan `link.exe`, el Windows SDK y
  Visual Studio Build Tools. `rust-lld` tampoco basta porque el host MSVC y
  las import libs/CRT requeridas no están disponibles. El soporte de eframe
  con gnullvm sigue sin estar verificado.

## Alcance funcional del primer corte

Una sola ventana, sin diálogos o ventanas por agente:

| Zona | Contenido | Política de memoria |
|---|---|---|
| Izquierda | Enjambres detectados, activos primero, estado y antigüedad | Sólo metadatos de `workflow.json` y reportes |
| Centro | Árbol virtualizado de tareas y subagentes, filtros y búsqueda | Índice compacto; filas visibles únicamente |
| Derecha | Tarea seleccionada, proveedor/modelo, heartbeat, error y log | Sólo el log seleccionado, lectura incremental y tope de 2 MiB |
| Inferior | Contadores, límites de concurrencia y estado del lector | Series temporales fuera del primer corte |

"Ir a ver" un subagente significa seleccionar su fila y actualizar el panel
derecho dentro de la misma ventana. No se crea ni enfoca otra ventana del SO.
Los workers continúan como procesos en segundo plano, con stdout/stderr en
`worker.log`. El primer corte de la UI no inicia, detiene ni reanuda procesos:
observa exclusivamente `docs/STATE_CONTRACT.md`.

Para una fase posterior que permita iniciar ejecuciones desde la UI, la prueba
de aceptación en Windows debe verificar que el coordinador y sus adaptadores se
lancen sin consola visible ni cambio de foco. Eso requiere una decisión
explícita sobre `CREATE_NO_WINDOW`; redirigir stdout/stderr por sí solo no prueba
que nunca aparezca una consola.

## Encaje con la arquitectura actual

El coordinador Rust ya escribe:

- `workflow.json`, con identidad, workspace y límites;
- `tasks/*.json`, con snapshots atómicos, jerarquía y heartbeat;
- `events.jsonl`, como flujo append-only;
- `results/<task_id>/worker.log`, cargado sólo al seleccionar una tarea;
- `report-rs.json`, al terminar.

El lector debe mantener el offset de `events.jsonl` y procesar sólo líneas
completas nuevas. Un snapshot que coincida con un reemplazo atómico se reintenta
una vez. Los campos desconocidos se ignoran. La jerarquía visual no altera el
DAG de `needs`.

Cadencia propuesta:

- ventana enfocada y run activo: comprobar tamaño/fecha cada 500 ms;
- ventana sin foco o sin cambios: cada 2 s;
- cambio detectado: leer sólo el delta y pedir un repaint;
- interacción del usuario: repaint inmediato;
- nunca ejecutar un loop fijo a 60 FPS.

No se añade inicialmente un *file watcher*: el sondeo por metadatos usa la
biblioteca estándar, es portable y permite medir primero si una dependencia
adicional realmente ahorra CPU.

## Corte feature-gated implementado

Esta es la configuración efectiva del segundo binario; no se construye en el
flujo normal porque requiere `ui-egui`:

```toml
# SWARMS-UI-CUT-001: Aísla la UI de las dependencias del coordinador CLI.
[features]
default = []
ui-egui = ["dep:eframe"]

[[bin]]
name = "swarms-ui"
path = "src/ui_bin.rs"
required-features = ["ui-egui"]

[target.'cfg(windows)'.dependencies]
# SWARMS-UI-CUT-002: Empieza con Glow; WGPU queda fuera hasta medirlo.
eframe = { version = "0.35", optional = true, default-features = false, features = ["accesskit", "default_fonts", "glow"] }

[target.'cfg(target_os = "linux")'.dependencies]
# SWARMS-UI-CUT-003: Habilita ambos window systems para el artefacto Linux.
eframe = { version = "0.35", optional = true, default-features = false, features = ["accesskit", "default_fonts", "glow", "wayland", "x11"] }
```

Antes de modificar `Cargo.toml`, verificar que la versión elegida siga siendo
la publicada y revisar transitivamente licencias con una herramienta de SBOM o
`cargo-deny`. No activar `persistence`, `wgpu`, imágenes ni gráficos hasta que
una historia funcional los necesite.

## Comparación

| Criterio | egui/eframe | Slint | Tauri |
|---|---|---|---|
| Modelo | Rust inmediato, render nativo | Rust declarativo/reactivo | Core Rust + HTML/CSS/JS en WebView |
| Procesos UI | Diseñable como uno | Diseñable como uno | La arquitectura oficial usa core + proceso(s) WebView |
| Licencia | MIT o Apache-2.0 | GPLv3 o licencia royalty-free/comercial propia | MIT o Apache-2.0 |
| Toolchain actual | Spike implementado; enlace bloqueado por MSVC ausente | Spike necesario; docs recomiendan ajustes de linker MSVC en Windows | No: la guía oficial exige C++ Build Tools, MSVC y WebView2 en Windows |
| Linux | X11/Wayland por features | Winit X11/Wayland; varios renderers | WebKitGTK y dependencias de distribución |
| Riesgo de CPU | Repaint continuo accidental | Renderer y animaciones sin medir | DOM/WebView y proceso(s) sin medir |
| Riesgo de RAM | Backend gráfico y fuentes sin medir | Renderer elegido y runtime desktop sin medir | Suma del core y árbol de procesos WebView sin medir |
| Densidad tipo IDE | Alta, pero requiere estilo propio | Alta, con markup declarativo | Alta y rápida con CSS |
| Decisión | **Spike principal** | Comparador de rendimiento condicionado a licencia | Comparador sólo si la ergonomía web justifica su costo |

### egui/eframe

La documentación oficial describe a egui como inmediato y a eframe como la
integración nativa/web sobre winit y Glow o WGPU. También advierte que eframe
tiene muchas más dependencias que el núcleo egui. Para esta UI, `show_rows`,
repaints solicitados y un único viewport son suficientes. No usar viewports
inmediatos: la documentación indica que hacen repintar padre e hijo y duplican
o triplican trabajo.

Riesgos abiertos: apariencia no nativa, regresiones entre versiones según el
propio README, costo del contexto gráfico en idle y compilación del backend
Glow con `windows-gnullvm`.

### Slint

Slint es el candidato técnico que merece compararse si la prioridad absoluta
es memoria. Su documentación ofrece backend Winit con renderer software,
FemtoVG o Skia; el software renderer se presenta como ligero y soporta render
parcial. La cifra comercial de “menos de 300 KiB de RAM” describe el runtime y
no el working set completo de una aplicación desktop, así que no se usará como
benchmark contra los otros candidatos.

El bloqueo principal es de gobernanza: Slint no se publica simplemente bajo
MIT/Apache. Para una app desktop permite GPLv3 o una licencia royalty-free que
exige atribución mediante `AboutSlint` o un badge público. Incluirlo en este
repositorio MIT requiere aceptar y documentar esas condiciones, además de las
licencias de terceros. No se selecciona por defecto sin esa decisión.

### Tauri

Tauri es compatible en licencia y reutiliza el WebView del sistema, por lo que
su binario distribuido puede ser pequeño. Tamaño en disco no equivale a RAM: su
documentación describe un proceso core y uno o más procesos WebView. El proceso
completo, y no sólo el `.exe`, debe entrar al benchmark.

Además, la guía oficial requiere Microsoft C++ Build Tools, el toolchain MSVC y
WebView2 para desarrollar en Windows; en Linux requiere WebKitGTK y varias
bibliotecas del sistema. Eso rompe la ventaja del toolchain aislado actual. Se
mantiene como alternativa si en el futuro pesan más la accesibilidad web, CSS o
la velocidad de iteración que el costo mínimo medido.

## Benchmark reproducible

### Fixture y casos

Los tres prototipos deben renderizar exactamente el mismo estado sintético,
sin ejecutar providers:

- 10 runs, 1.000 tareas y hasta 4 niveles de subagentes;
- nombres, estados, providers y heartbeats deterministas;
- un `worker.log` de 10 MiB para la tarea seleccionada;
- caso A: arranque hasta archivo `ready`;
- caso B: 60 s idle, ventana visible y enfocada, sin cambios;
- caso C: 60 s con 10 eventos/s y 10 snapshots/s;
- caso D: expandir/colapsar el árbol, filtrar y hacer scroll durante 60 s;
- caso E: ventana minimizada durante 60 s.

Cada prototipo acepta `--fixture`, `--ready-file` y `--bench-duration`; termina
solo al acabar el caso. Ejecutar cinco repeticiones después de una corrida de
calentamiento, en release, mismo equipo, resolución, escala DPI y plan de
energía. Reportar mediana, mínimo, máximo y p95; conservar datos crudos. No
comparar una build debug con otra release.

### Métricas

1. **RAM idle/activa:** suma del private working set en Windows o RSS/PSS en
   Linux para todo el árbol de procesos. Informar por separado GPU dedicated y
   shared si la herramienta del sistema lo permite.
2. **CPU idle/activa:** suma de delta de tiempo CPU del árbol dividida por
   tiempo de pared; 100% equivale a un core completamente ocupado.
3. **Arranque:** tiempo monotónico desde `CreateProcess`/`exec` hasta que la
   primera ventana funcional escribe atómicamente `ready-file`.
4. **Distribución:** bytes del binario más assets propios. Para Tauri, informar
   por separado que WebView2/WebKitGTK es dependencia del sistema.
5. **Capacidad:** latencia de selección y scroll, pérdida de eventos, y bytes de
   log retenidos por el proceso.

En Windows se debe sumar todo el árbol; medir sólo el PID padre favorece
artificialmente a Tauri. En Linux, PSS desde `smem` es preferible a sumar RSS
compartido; si `smem` no está disponible, declarar que RSS puede contar páginas
compartidas más de una vez.

### Comandos de referencia

Los paths son contractuales para futuros prototipos, no existen todavía:

```powershell
# SWARMS-UI-BENCH-001: Compila sólo el candidato egui en release y con lockfile.
cargo build --release --locked --manifest-path rust/Cargo.toml --features ui-egui --bin swarms-ui

# SWARMS-UI-BENCH-002: Ejecuta un caso controlado y deja una señal de primera pintura.
rust\target\release\swarms-ui.exe --fixture .cache\ui-bench\fixture --ready-file .cache\ui-bench\egui-ready.json --bench-duration 60

# SWARMS-UI-BENCH-003: Captura memoria privada y CPU acumulada del PID observado.
Get-Process -Id $PID_OBSERVADO | Select-Object Id, ProcessName, CPU, WorkingSet64, PrivateMemorySize64

# SWARMS-UI-BENCH-004: Enumera descendientes para sumar el árbol completo.
Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name

# SWARMS-UI-BENCH-005: Registra los bytes del artefacto propio.
Get-Item rust\target\release\swarms-ui.exe | Select-Object FullName, Length
```

```bash
# SWARMS-UI-BENCH-006: Mide tiempo y máximo RSS del caso Linux controlado.
/usr/bin/time -v ./target/release/swarms-ui --fixture .cache/ui-bench/fixture --ready-file .cache/ui-bench/egui-ready.json --bench-duration 60

# SWARMS-UI-BENCH-007: Captura CPU y RSS del proceso; sumar descendientes por PPID.
ps -o pid,ppid,pcpu,rss,etime,comm -p "$PID_OBSERVADO" --ppid "$PID_OBSERVADO"

# SWARMS-UI-BENCH-008: Captura PSS cuando smem está disponible.
smem -P 'swarms-ui|WebView2|WebKitWebProcess' -c 'pid pss rss command'
```

El harness definitivo debe automatizar muestreo a 250 ms y escribir CSV en
`.cache/ui-bench/results/`; no se deben transcribir resultados a mano.

### Regla de selección

- Descartar cualquier candidato que no compile y funcione en Windows y Linux o
  no pueda cumplir las licencias del proyecto.
- Elegir el menor consumo mediano de RAM y CPU idle del árbol completo.
- Tratar diferencias menores a 5% como ruido hasta repetir en otro equipo. Si
  egui y otro candidato quedan dentro de ese margen, preferir egui por licencia,
  una sola tecnología y encaje con el toolchain existente.
- Si Slint supera a egui por más de 5% en RAM **y** CPU idle, detener la elección
  y resolver primero la aceptación de su licencia; no cambiar silenciosamente.
- Tauri sólo gana si sus métricas del árbol completo vencen a los nativos o si
  se aprueba explícitamente priorizar desarrollo web/accesibilidad sobre
  consumo y nuevas dependencias del sistema.

## Riesgos y pruebas de aceptación

| Riesgo | Prueba antes de avanzar |
|---|---|
| Repaint loop eleva CPU | Caso B y E con causa de repaint instrumentada |
| Árbol grande eleva RAM | 1.000 y 10.000 filas con virtualización |
| Logs crecen sin límite | Abrir 10 MiB y comprobar tope residente de 2 MiB |
| Lectura parcial de JSONL | Inyectar última línea truncada y completarla después |
| Snapshot cambia durante lectura | Reemplazo atómico concurrente sin caída de UI |
| Worker roba foco | Ejecución desde UI futura sin consola/ventana adicional |
| Toolchain no enlaza GUI | Build limpia con `windows-gnullvm` y `rust-lld` aislados |
| Dependencias afectan CLI | `cargo tree` y tamaño del binario CLI iguales sin feature |

## Fuentes oficiales

- [egui/eframe README, plataformas, dependencias y licencia](https://github.com/emilk/egui)
- [Features de eframe 0.35.0](https://docs.rs/crate/eframe/0.35.0/features)
- [Repaint bajo demanda en egui](https://docs.rs/egui/0.35.0/egui/struct.Context.html#method.request_repaint_after)
- [Virtualización con ScrollArea::show_rows](https://docs.rs/egui/0.35.0/egui/containers/scroll_area/struct.ScrollArea.html#method.show_rows)
- [Backends y renderers de Slint](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/)
- [Licencias oficiales de Slint](https://slint.dev/terms-and-conditions)
- [Plataformas desktop de Slint](https://docs.slint.dev/latest/docs/slint/guide/platforms/desktop/)
- [Arquitectura de Tauri](https://v2.tauri.app/concept/architecture/)
- [Modelo multiproceso de Tauri](https://v2.tauri.app/concept/process-model/)
- [Prerequisitos Windows/Linux de Tauri](https://v2.tauri.app/start/prerequisites/)
- [Licencia de Tauri](https://github.com/tauri-apps/tauri)
