# SWARMS

![Portada del flujo SWARMS](images/swarms-cover.png)

Orquestacion local-first para coding agents.

SWARMS deja que cada persona decida que modelo planifica, que modelo programa, que modelo revisa y cuantos workers pueden correr al mismo tiempo. El repo funciona offline al clonar. Las llamadas a modelos ocurren solo cuando configuras tus propios planes, APIs, CLIs y politica de routing.

Sitio web: https://swarms-orchestrator.vercel.app/

Uso versiones de este flujo de forma personal desde enero-febrero de 2026. La idea original vino de los loops estilo Ralph: dejar un modelo fuerte en planificacion y revision, y usar workers baratos para implementacion, QA, lectura de issues y validacion repetida.

English: [README.md](README.md)

## Rutas de proveedores HY3

SWARMS incluye varias rutas HY3 configurables. Los precios, promociones y
requisitos de cuenta pueden cambiar; compruébalos con el proveedor antes de
una ejecución real.

| Ruta | Proveedor | Model ID | ¿Gratis? |
|---|---|---|---|
| `hy3_opencode` | OpenCode Zen | `opencode/hy3-free` | Tier gratis |
| `hy3_gitlawb` | GitLawb OpenGateway | `tencent/hy3` | Promo gratis |
| `hy3_openrouter` | OpenRouter | `tencent/hy3:free` | Variante gratis |
| `hy3_kilo` | Kilo CLI | `kilo/tencent/hy3:free` | Tier gratis |
| `hy3_hermes` | Hermes / Nous Portal | `tencent/hy3:free` | Tier gratis |
| `hy3_siliconflow` | SiliconFlow | `tencent/Hy3` | De pago |

El plan de ejemplo incluido usa solo `mock`. Para un plan privado cuyas tareas
usen explícitamente `hy3_gitlawb`, la forma de ejecución es:

```bash
python scripts/swarm.py run --plan path/to/hy3-plan.json --force \
  --global-max-concurrency 3 --provider-cap hy3_gitlawb=3
```

El adaptador **Hermes Agent** puede ejecutar un subagente con herramientas solo
cuando la configuración local fija un modelo explícito. Una ruta sin modelo se
rechaza.

Todas las rutas HY3 están **desactivadas por defecto** (mock sigue siendo el
default seguro de open-source). Habilitá las que quieras en
`config/swarm_router.local.json` y configurá las API keys correspondientes en
tu entorno.

## Flujos Estilo Ultra de Claude Code y GPT-5.6

Claude Fable 5 puede impulsar flujos multiagente de larga duración en Claude Code: planifica por etapas, delega en subagentes y revisa su propio trabajo. OpenAI también anunció un nuevo modo `ultra` de GPT-5.6 basado en subagentes, pero GPT-5.6 sigue en vista previa limitada y no tiene disponibilidad pública amplia. SWARMS apunta a este patrón operativo desde el lado local-first: tú eliges planner, critic, modelos worker, provider caps, metadatos de verificación y presupuesto de tokens.

Usa SWARMS cuando quieras un equipo de agentes estilo Ultra sin amarrar todo el flujo a un solo modo de un proveedor:

- corre todo local hasta que habilites proveedores reales;
- enruta planner, critic, programmer, verifier y QA a modelos distintos;
- mezcla APIs compatibles con OpenAI configuradas, GLM, Gemini, Codex CLI, Hermes o workers mock offline;
- mantiene provider caps y reportes visibles;
- corre Singularity cuando quieras un loop largo que siga proponiendo, implementando, testeando y resumiendo trabajo.

## Integraciones

SWARMS incluye rutas, wrappers, docs o telemetria para:

- APIs compatibles con OpenAI.
- Routing estilo LiteLLM.
- Rutas premium estilo Anthropic para planner/critic.
- GLM 5.2 mediante OpenCode o rutas estilo Z.AI.
- Gemini 3.5 Flash mediante Antigravity CLI.
- Codex CLI para orquestacion premium o escalamiento.
- Gateways compatibles con OpenAI configurados por el usuario.
- Workers offline `mock` para CI, demos y configuracion segura.
- Parsing de tokens/costos para logs de Codex, OpenCode, salidas CLI, cache reads, cache writes y reasoning tokens.
- Una skill SWARMS en `skills/swarms/` para que el agente de cada persona ayude a configurar planes, proveedores, limites y verificacion.

El router versionado solo habilita `mock`. Eso mantiene el clone local y gratis. Tu configuracion privada vive en archivos ignorados como `config/swarm_router.local.json` y en tus propias variables de entorno.

## Como Se Configura

Tu defines la politica:

- Los planes definen roles, tareas, dependencias, artefactos esperados, metadatos de verificación y permisos premium.
- `config/role_policy.json` define la intencion de planner, critic, programmer y verifier.
- `config/swarm_router.json` es el default local seguro.
- `config/swarm_router.local.example.json` muestra como habilitar tus proveedores.
- Los provider caps limitan concurrencia por ruta.
- La telemetria registra lo que reporta la CLI o API, y marca uso faltante en vez de fingir que fue gratis.

El repo incluye una skill para enseñar a agentes compatibles a usar SWARMS:

```powershell
Copy-Item -Recurse -Force .\skills\swarms "$env:USERPROFILE\.codex\skills\swarms"
```

Despues de eso, un agente puede revisar tu setup local, crear un plan, revisarlo y correr la validacion offline antes de que habilites rutas reales.

## Coordinador Rust

Los workflow plans pueden usar el coordinador Rust de menor consumo en Windows, macOS y Linux. La autenticación de proveedores sigue en los adaptadores CLI locales existentes.

```powershell
cargo run --release --manifest-path rust/Cargo.toml -- doctor
cargo run --release --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

El flujo completo está en `docs/RUST_RUNTIME.md`. Python sigue disponible para compatibilidad de benchmarks y telemetría heredados.

## Inicio Rapido

En un PC nuevo, inspecciona primero los agentes locales antes de habilitar o
ejecutar rutas reales:

```powershell
python scripts/swarm.py preflight --format json
```

Consulta [docs/AGENT_PREFLIGHT.md](docs/AGENT_PREFLIGHT.md). `doctor` ejecuta
este inventario primero y `run` rechaza agentes reales no verificados antes de
crear claims o workers.

Requiere Python 3.10+ y Git.

```powershell
python scripts/swarm.py doctor
python scripts/swarm.py review --plan docs/workflow_plan_example.json
python scripts/swarm.py dry-run --plan docs/workflow_plan_example.json --force
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Reanuda un run interrumpido con el mismo plan e identificador:

```powershell
# SWARMS-RESUME-004: conserva checkpoints de tareas terminadas.
cargo run --release --manifest-path rust/Cargo.toml -- run --plan docs/workflow_plan_example.json --run-id my-run --resume --provider-cap mock=3
```

Los archivos de estado son una frontera de integración de sólo lectura para
interfaces locales; consulta `docs/STATE_CONTRACT.md`.

Para coordinar un repositorio vecino, conserva SWARMS como harness y declara
el destino:

```powershell
# SWARMS-CLI-001: Ejecuta workers con herramientas en el repositorio objetivo.
python scripts/swarm.py dry-run --plan C:\proyecto\plan.json --workspace-root C:\proyecto --force
```

Instalacion editable opcional:

```powershell
python -m pip install -e ".[dev,yaml]"
swarms doctor
swarms run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

## Modelo De Runtime

![Mapa del runtime SWARMS](images/runtime-map.png)

```text
objetivo
  -> workflow plan
  -> revision estatica
  -> runtime deterministico
  -> pools de proveedores con limites
  -> salida de workers
  -> verificacion y report.json
```

El runtime guarda estado en `.agent/swarm/runs/<run_id>/`: prompts, logs, estado de tareas, eventos, resultados y reportes. El coordinador no tiene que cargar todo el ruido de workers en contexto.

## Modo Singularity

Singularity es el loop autonomo para personas dispuestas a gastar tokens.

La idea es correr un equipo local 24/7: proponer mejoras, leer issues, crear tareas, lanzar workers, hacer QA, validar funcionalidades, resumir cambios y empezar el siguiente ciclo. Es el modo mas cercano a un loop de ingenieria permanente dentro de SWARMS.

```powershell
pwsh scripts/start_singularity.ps1 -MaxCycles 5
```

Tu controlas el riesgo. Con `mock`, Singularity es una simulacion local. Con proveedores reales, muchos workers y muchos ciclos, puede consumir muchisimos tokens. Usa provider caps, `MaxCycles` y un archivo `STOP_SINGULARITY` cuando lo pruebes.

## Ideas Por Implementar

SWARMS deberia conectarse con las herramientas donde ya vive el trabajo de ingenieria:

- Trello: leer cards, crear planes de implementacion y mover cards despues de validar.
- Hermes Agent: usar Hermes como otra ruta local de agente o superficie de coordinacion.
- Discord: publicar resumenes de ciclo, pedir aprobaciones y aceptar comandos livianos.
- JIRA: leer tickets, planificar trabajo, actualizar estados y adjuntar reportes de verificacion.
- Microsoft Teams: enviar resumenes de QA, avisos de escalamiento y reportes de ciclos Singularity.

## Politica De Proveedores

Intencion por rol:

- Planner: Claude Fable puede configurarse como agente planificador premium. GPT-5.6 Sol se documenta como opción futura mientras su acceso siga limitado; GLM 5.2 permanece como valor seguro por defecto.
- Critic: GLM 5.2 primero, revision premium para planes riesgosos o caros.
- Programmer: GLM 5.2, Gemini Flash, OpenAI-compatible, LiteLLM, Kilo, Aider o cualquier ruta que configures.
- Verifier: ejecuta tests deterministas fuera del harness y luego usa revisión barata.
- Rutas premium: permiso explicito en el plan y config local.

Ver `docs/PROVIDER_STATUS.md`, `docs/CONFIG.md`, `docs/DYNAMIC_WORKFLOWS.md` y `AGENTS.md`.

## Origen

Construí las primeras versiones para uso personal alrededor de enero-febrero de 2026. Tenia restricciones de plan de estudiante y queria estirar los modelos disponibles: Gemini en Antigravity para workers, Opus para planes, y despues GLM 5.2 y Codex para planner/critic.

La forma no cambio: gastar modelos escasos en decisiones, no en trabajo repetitivo.

## Verificacion

```powershell
python -m ruff check .
python -m py_compile scripts\swarm.py scripts\plan_review.py scripts\workflow_runtime.py scripts\doctor.py scripts\mock_worker.py
python -m pytest tests -q
python scripts/swarm.py doctor
python scripts/swarm.py run --plan docs\workflow_plan_example.json --force --run-id verify-readme --global-max-concurrency 3 --provider-cap mock=3
```

## Licencia

MIT. Ver `LICENSE`.
