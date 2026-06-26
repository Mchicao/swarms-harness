# SWARMS

![Portada del flujo SWARMS](assets/swarms-cover.svg)

Orquestacion para ahorrar cuota en flujos de coding agents.

SWARMS crea un plan, lo revisa y ejecuta workers acotados con modelos baratos, checks locales o el proveedor offline `mock`. Uso versiones de este flujo de forma personal desde enero-febrero de 2026, cuando los loops estilo Ralph dejaron clara la idea: usar modelos fuertes para planificar y revisar, y dejar la implementacion rutinaria a workers baratos.

El repo publico arranca en modo offline. Puedes correr demo, tests y CI sin llaves. Los proveedores reales se activan solo con configuracion local.

English: [README.md](README.md)

## Que Ejecuta

- `scripts/swarm.py`: CLI publico para revisar, simular y ejecutar planes.
- `scripts/workflow_runtime.py`: estado deterministico, dependencias, locks, limites por proveedor, eventos, resultados y reportes.
- `scripts/plan_review.py`: revision estatica del plan antes de lanzar workers.
- `scripts/parallel_swarm.ps1`: runner historico con worktrees, routing, watcher, retry, telemetria y merge.
- `scripts/start_singularity.ps1`: loop que crea tareas con architect, lanza swarm, resume y repite.
- `scripts/agy_call.py`: wrapper programatico para Antigravity/Gemini cuando la CLI no devuelve respuesta limpia por stdout.
- `scripts/smart_router.py`: routing por rol, directiva, salud y politica local.
- `scripts/utils/token_telemetry.py`: normalizacion de eventos de tokens y costo.

## Integraciones Ya Implementadas

SWARMS tiene rutas, wrappers, docs o telemetria para:

- GLM 5.2 mediante OpenCode o rutas estilo Z.AI.
- Gemini 3.5 Flash mediante Antigravity CLI.
- Codex CLI para orquestacion premium o escalamiento.
- Kilo y Aider en el runner legacy con worktrees.
- Verificacion local por shell/tests.
- Workers offline `mock` para CI, demos y clones seguros.
- Distribucion de skills para Codex, Claude/OpenCode desde `skills/swarms/`.
- Parsing de tokens/costos para logs de Codex, OpenCode, salidas CLI, cache reads, cache writes y reasoning tokens.

El router versionado solo habilita `mock`. Eso protege a usuarios nuevos y mantiene CI gratis. Tus rutas privadas viven en `config/swarm_router.local.json`.

## Inicio Rapido

Requiere Python 3.10+ y Git.

```powershell
python scripts/swarm.py doctor
python scripts/swarm.py review --plan docs/workflow_plan_example.json
python scripts/swarm.py dry-run --plan docs/workflow_plan_example.json --force
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

Instalacion editable opcional:

```powershell
python -m pip install -e ".[dev,yaml]"
swarms doctor
swarms run --plan docs/workflow_plan_example.json --force --global-max-concurrency 3 --provider-cap mock=3
```

## Modelo De Runtime

![Mapa del runtime SWARMS](assets/runtime-map.svg)

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

Singularity es el loop autoalimentado:

```powershell
pwsh scripts/start_singularity.ps1 -MaxCycles 5
```

Cada ciclo corre architect, lanza el swarm, registra estado, resume y pasa al siguiente ciclo. Sirve cuando quieres que el sistema siga descomponiendo y reparando un proyecto sin escribir cada tarea a mano.

Usalo con limites. Singularity puede gastar muchisimos tokens si activas proveedores reales, subes el numero de workers o dejas correr muchos ciclos. Ten listo `STOP_SINGULARITY`, empieza con `mock-only` y usa `MaxCycles` bajo antes de habilitar rutas pagadas.

## Politica De Proveedores

Intencion por rol:

- Planner: GLM 5.2 por defecto, Codex solo cuando el plan justifica cuota premium.
- Critic: GLM 5.2 primero, Codex para planes riesgosos o caros.
- Programmer: GLM 5.2 o Gemini Flash cuando esten configurados.
- Verifier: tests locales primero, modelo barato despues.
- Rutas premium: permiso explicito en el plan y config local.

Ver `docs/PROVIDER_STATUS.md`, `docs/CONFIG.md` y `docs/DYNAMIC_WORKFLOWS.md`.

## Origen

Construí las primeras versiones para uso personal alrededor de enero-febrero de 2026. Tenia restricciones de plan de estudiante y queria estirar los modelos disponibles: Gemini en Antigravity para workers, Opus para planes, y despues GLM 5.2 y Codex para planner/critic.

La forma del producto no cambio: gastar modelos escasos en decisiones, no en trabajo repetitivo.

## Seguridad

No subas:

- `.env`
- `config/*.local.json`
- API keys, OAuth tokens, auth files o credenciales privadas
- `.agent/`
- worktrees
- prompts, logs, traces, reportes o telemetria generada

El flujo por defecto no llama APIs pagadas ni proveedores externos.

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
