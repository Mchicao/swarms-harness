# Auditoría read-only del entorno de compilación — `rust/src/ui_bin.rs`

- **Rol SWARMS:** `critic` (auditoría de entorno, sin cambios de código).
- **Alcance:** determinar por qué el binario `ui_bin.rs` (que reutiliza la lib `swarms_ui` y la UI feature-gated `ui-egui`) **no enlaza** en este equipo, con evidencia de comandos y `exit code` reales.
- **Fecha de la auditoría:** 2026-07-16.
- **Equipo/host:** Windows (`x86_64-pc-windows-msvc`), repo `C:\Proyectos\SWARMS`.

> Convención de evidencia: **[VERIFICADO]** = comando ejecutado durante esta auditoría, con `exit code` observado. **[FUENTE]** = afirmación leída de un fichero con formato (no reproducida en binario). No se declara que `cargo test`/`cargo clippy` pasan salvo con `exit code` 0 explícito.

---

## 1. Veredicto (TL;DR)

**El bloqueo NO es específico de la UI.** Todo comando de `cargo` que requiera enlazar (incluido el del coordinador y de la librería serde-only, **con o sin** la feature `ui-egui`) falla con `exit code 101`:

```text
error: linker `link.exe` not found
note: the msvc targets depend on the msvc linker but `link.exe` was not found
note: please ensure that Visual Studio 2017 or later, or Build Tools for Visual Studio
      were installed with the Visual C++ option
```

**Causa raíz exacta:** el toolchain **host por defecto es `stable-x86_64-pc-windows-msvc`** y faltan por completo el MSVC linker (`link.exe`), las bibliotecas del Windows SDK y Visual Studio / Build Tools. Los **proc-macros y build scripts** (`proc-macro2`, `quote`, `serde`/`serde_core` derive, `serde_json`, `zmij`) **siempre se compilan y enlazan para el host**, por lo que **cualquier** `cargo build|test|clippy` cae en el enlace del host antes de llegar a tocar `eframe`. Añadir `--target x86_64-pc-windows-gnullvm` **no lo resuelve**: el lado host sigue siendo msvc y sigue necesitando `link.exe` (verificado, §3).

**Único gate que pasa:** `cargo fmt --check` (`exit 0`), porque no invoca enlazador.

---

## 2. Entorno verificado

| Ítem | Estado | Evidencia |
| --- | --- | --- |
| `rustc` | 1.97.0 (2d8144b78 2026-07-07), **host `x86_64-pc-windows-msvc`**, LLVM 22.1.6 | **[VERIFICADO]** `~/.cargo/bin/rustc.exe -vV`, exit 0 |
| `cargo` | 1.97.0 (c980f4866 2026-06-30) | **[VERIFICADO]** exit 0 |
| `rustup` | 1.29.0; `default_toolchain = stable-x86_64-pc-windows-msvc`; profile `default` | **[VERIFICADO]** `settings.toml` + `rustup show` |
| Targets instalados | `x86_64-pc-windows-msvc` **y** `x86_64-pc-windows-gnullvm` | **[VERIFICADO]** `rustup target list --installed`, exit 0 |
| Toolchains instalados | sólo `stable-x86_64-pc-windows-msvc` (el target gnullvm se añadió como `rustup target add`, **no** como toolchain propio) | **[VERIFICADO]** `~/.rustup/toolchains/` |
| `link.exe`, `cl.exe`, `lld-link.exe`, `rust-lld.exe` (PATH), `clang.exe`, `vswhere.exe`, `gcc.exe`, `dumpbin.exe` | **todos AUSENTES del PATH** | **[VERIFICADO]** `Get-Command` por nombre |
| `vswhere.exe` (rutas estándar `Program Files (x86)\Microsoft Visual Studio\Installer\…`) | **AUSENTE** | **[VERIFICADO]** |
| `Windows Kits`, `Microsoft Visual Studio`, `Microsoft SDK`, `LLVM`, `clang` (en `Program Files*`) | **AUSENTES** (sin SDK ni VS ni LLVM) | **[VERIFICADO]** `Test-Path` |
| `rust-lld.exe` (dentro del toolchain) | **EXISTE**: `…\toolchains\stable-x86_64-pc-windows-msvc\lib\rustlib\x86_64-pc-windows-msvc\bin\rust-lld.exe` | **[VERIFICADO]** (no está en PATH; rustc lo resuelve internamente) |
| `eframe-0.35.0.crate` | **cacheado** en `~/.cargo/registry/cache` → la resolución offline de la dependencia funciona | **[VERIFICADO]** |
| `rust/Cargo.lock` | existe (100 284 bytes, mtime 2026-07-16) — **no modificado** en esta auditoría | **[VERIFICADO]** |
| `rust/target/debug/swarms-rs.exe` | existía (binario msvc de 2026-07-15, previo a la desaparición de `link.exe`); se eliminó el artefacto stale del caché **sólo** para forzar un relink real | **[VERIFICADO]** |
| `rust/target/x86_64-pc-windows-gnullvm/` | sólo rlibs del stack serde (`itoa`, `memchr`, `serde`, `serde_json`); **sin** exe final → el intento gnullvm de hoy no produjo binario | **[VERIFICADO]** |
| Docker | `docker.exe` instalado (`C:\Program Files\Docker\Docker\resources\bin\docker.exe`) pero **daemon CAÍDO**: `failed to connect to the docker API … DockerDesktopLinuxEngine` | **[VERIFICADO]** exit 1 |

### 2.1 `self-contained` del target gnullvm — inspección directa

`…\lib\rustlib\x86_64-pc-windows-gnullvm\lib\self-contained` contiene **sólo 2 ficheros `.o`** y **ningún** `.lib`. Verificado ausentes: `kernel32.lib`, `ntdll.lib`, `userenv.lib`, `ws2_32.lib`, `dbghelp.lib`, `libcmt.lib`, `libvcruntime.lib`, `msvcrt.lib`. **[VERIFICADO]**

→ Implicación: aunque el host fuera gnullvm, `rust-lld` no encontraría las import libs del sistema y fallaría con `could not open 'kernel32.lib' …` (el segundo error documentado en `SWARM_UI.md`). Esa rama **no es alcanzable hoy** porque el build muere antes en el enlace del host proc-macro.

---

## 3. Evidencia de comandos (con `exit code`)

Cada comando se ejecutó con `cargo` por ruta absoluta (`~/.cargo/bin/cargo.exe`) y flags `--offline --locked` (no se tocaron `Cargo.lock`/`Cargo.toml`, ni PATH, ni procesos; sólo se borró el exe stale del caché para forzar relink).

| Comando | Exit | Resultado | Tipo |
| --- | :--: | --- | --- |
| `cargo build --bin swarms-rs --offline --locked` (default msvc) | **101** | `linker 'link.exe' not found` | **[VERIFICADO]** |
| `cargo build --lib --offline --locked` (modelo serde-only, sin feature) | **101** | `link.exe not found` (proc-macro host: `serde_json`/`zmij`/`quote`/`serde`/`serde_core`/`proc-macro2`) | **[VERIFICADO]** |
| `cargo test --lib --offline --locked` (sin feature) | **101** | `link.exe not found` | **[VERIFICADO]** |
| `cargo clippy --offline --locked -- -D warnings` (sin feature, check-only) | **101** | `link.exe not found` (los check también compilan/enlazan proc-macros y build scripts del host) | **[VERIFICADO]** |
| `cargo build --bin swarms-rs --target x86_64-pc-windows-gnullvm --offline --locked` | **101** | `link.exe not found` (host proc-macro; **no** llegó a `rust-lld`) | **[VERIFICADO]** |
| `cargo fmt --manifest-path rust/Cargo.toml -- --check` | **0** | pasa (no usa enlazador) | **[VERIFICADO]** |
| `cargo metadata --no-deps` | 0 | paquete único `swarms-runtime 0.1.0` | **[VERIFICADO]** |

**Conclusión de evidencia:** ni `cargo test` ni `cargo clippy` pasan (ambos `exit 101`). No puede afirmarse compilación/enlace correcto de `ui_main.rs` en este entorno con ningún target instalado.

### 3.1 Comportamiento observado, no declarado

- El fallo ocurre al enlazar **build scripts y proc-macros del host msvc**; nunca se alcanza el enlace final del binario. Por eso el error es idéntico con/sin `--features ui-egui` y con `--target gnullvm`.
- `eframe 0.35` no llegó a compilarse en esta sesión: el grafo muere en `proc-macro2`/`quote`/`serde_core` antes de descender a la UI.

---

## 4. Discrepancias detectadas en la documentación (sólo contexto)

| Documento | Afirmación | Estado real verificado |
| --- | --- | --- |
| `docs/SWARM_UI.md` (§Compilación) | "`cargo build`/`cargo test` … sin la feature … se construyen exactamente igual" | **Falso en este entorno:** ambos `exit 101` (§3). |
| `docs/SWARM_UI.md` (§Bloqueo) | "no tiene el target `x86_64-pc-windows-gnullvm` instalado" | **Falso:** ambos targets están instalados (§2). |
| `docs/SWARM_UI.md` (§Bloqueo) | error `rust-lld: could not open 'kernel32.lib' …` | **No reproducible hoy** vía `cargo build`/`--target gnullvm`: el build muere antes en el host proc-macro. Sólo aparecería si el host fuera un toolchain gnullvm completo. |
| `docs/UI_RUNTIME_EVALUATION.md` | "El repositorio ya compila el runtime con … `x86_64-pc-windows-gnullvm` y `rust-lld`, sin Visual Studio Build Tools" | **Engañoso:** el host por defecto es msvc; el único enlace exitoso documentado es un `swarms-rs.exe` msvc stale de 2026-07-15. El intento gnullvm de hoy no produjo binario. |
| `docs/SWARM_UI.md` (tabla Artefactos) | `rust/src/app.rs` contiene la ventana egui | **Corregido:** `Cargo.toml` define `[[bin]] swarms-ui` con `path = "src/ui_bin.rs"`; `ui_main.rs` contiene la lib y el módulo inline `ui_egui` (`ObservabilityApp`). |

> Nota: `rust/src/{main.rs,app.rs,ui_bin.rs}` y `rust/tests/ui_state.rs` sí existen en disco. La observación es de coherencia doc↔`Cargo.toml`, no de presencia de ficheros.

---

## 5. Alternativas soportadas y pasos manuales (requieren confirmación explícita)

> Esta auditoría es **read-only**. Ninguno de los pasos siguientes fue ejecutado. **Todos requieren confirmación del usuario** y, antes de instalar, verificar espacio libre (`Get-PSDrive C`) y mantener ≥20 % de reserva en el volumen (política de disco).

### Opción A — Restaurar el toolchain MSVC (recomendada; desbloquea todo)
Restaura `link.exe`, las import libs y la CRT del SDK, lo que habilita `cargo build|test|clippy` **y** el binario `swarms-ui` con `--features ui-egui`, sin tocar `Cargo.toml`.

1. Instalar **"Build Tools for Visual Studio 2022"** con la carga **"Desktop development with C++"** (componentes: `VC.Tools.x86.x64`, `Microsoft.VisualStudio.Component.Windows11SDK.*`).
2. Abrir **nuevo** shell (para que `vsdevcmd`/PATH se aplique) y confirmar:
   ```powershell
   Get-Command link.exe, cl.exe          # deben aparecer
   vswhere -latest -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property displayName
   ```
3. Validación requerida (deben dar **exit 0**):
   ```powershell
   cargo build --manifest-path rust/Cargo.toml --offline --locked
   cargo test  --manifest-path rust/Cargo.toml --offline --locked
   cargo build --manifest-path rust/Cargo.toml --features ui-egui --offline --locked
   cargo clippy --manifest-path rust/Cargo.toml --features ui-egui --offline --locked -- -D warnings
   ```
- **Coste:** varios GB en disco. **Confirmación requerida antes de instalar.**

### Opción B — Host gnullvm completo + `rust-lld` (coherente con `UI_RUNTIME_EVALUATION.md`)
Para que el host enlace con `rust-lld` (y así los proc-macros/build scripts no necesiten `link.exe`).

1. `rustup toolchain install stable-x86_64-pc-windows-gnullvm`
2. `rustup default stable-x86_64-pc-windows-gnullvm`
3. Validar que el `self-contained` del toolchain **completo** traiga las import libs (`kernel32.lib`…) — **no verificado** en esta auditoría (el `rustup target add` actual sólo aporta 2 `.o`, §2.1).
4. Validación requerida (exit 0):
   ```powershell
   cargo build --manifest-path rust/Cargo.toml --features ui-egui --target x86_64-pc-windows-gnullvm --offline --locked
   ```
- **Riesgo:** si el toolchain completo no incluye `self-contained` con import libs, aparecerá `rust-lld: could not open 'kernel32.lib'` (error ya documentado). **Confirmación requerida.**

### Opción C — Compilación aislada en contenedor Linux
El `Cargo.toml` ya declara dependencias Linux de `eframe` (`glow`, `wayland`, `x11`). Útil para reproducir el benchmark de `UI_RUNTIME_EVALUATION.md` sin tocar el host Windows.

1. Iniciar **Docker Desktop** (hoy el daemon está caído: `failed to connect to the docker API`).
2. Construir en imagen `rust:1.97` con un servidor gráfico/Xvfb o build `--no-run` sólo para verificar enlace.
- **Confirmación requerida** para levantar Docker y para cualquier volumen/imagen (espacio en disco).

### Opción D — LLVM/clang + `lld-link` + CRT (no recomendada)
Equivalente a configurar `rustflags = ["-C", "linker-flavor=lld-link", "-C", "linker=rust-lld"]`, pero **sigue requiriendo** una CRT y las import libs del SDK; aislamiento peor que A/B. Descartada salvo restricción explícita.

---

## 6. Restricciones respetadas (qué NO se hizo)

- No se instalaron paquetes, ni toolchains, ni componentes del SDK.
- No se modificó `PATH`, `Cargo.lock`, ni procesos/sistemas externos. El manifiesto fue actualizado después para separar el entry point y eliminar la advertencia de target duplicado; la compilación dirigida posterior siguió fallando sólo por `link.exe` ausente.
- La única mutación fue **borrar el artefacto stale** `rust/target/debug/swarms-rs.exe` (+`.d`) del caché de build, estrictamente para forzar un relink y obtener un `exit code` real; es reproducible (cargo lo regenera) y no toca fuentes ni lockfile.
- No se ejecutaron providers ni workers; lectura exclusiva del entorno.

---

## 7. Riesgos / preguntas abiertas

- **Bloqueo pendiente:** restaurar un linker/SDK Windows y repetir build/test/clippy con la feature `ui-egui`; la separación del entry point ya está aplicada.
- **Origen de la regresión:** `swarms-rs.exe` (msvc, 2026-07-15) prueba que `link.exe` existió y fue retirado después. Conviene confirmar si Visual Studio/Build Tools fue desinstalado o si el PATH perdió las entradas del SDK.
- **Target duplicado:** resuelto separando `[[bin]] swarms-ui` en `ui_bin.rs`; la compilación posterior ya no muestra la advertencia, pero sigue sin enlazar por el entorno MSVC incompleto.
