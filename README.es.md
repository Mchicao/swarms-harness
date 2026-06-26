# SWARMS

![Portada del flujo SWARMS](assets/swarms-cover.svg)

Orquestacion local-first para coding agents.

SWARMS deja que cada persona decida que modelo planifica, que modelo programa, que modelo revisa y cuantos workers pueden correr al mismo tiempo. El repo funciona offline al clonar. Las llamadas a modelos ocurren solo cuando configuras tus propios planes, APIs, CLIs y politica de routing.

Uso versiones de este flujo de forma personal desde enero-febrero de 2026. La idea original vino de los loops estilo Ralph: dejar un modelo fuerte en planificacion y revision, y usar workers baratos para implementacion, QA, lectura de issues y validacion repetida.

English: [README.md](README.md)

## Integraciones

SWARMS incluye rutas, wrappers, docs o telemetria para:

- APIs compatibles con OpenAI.
- Routing estilo LiteLLM.
- Rutas premium estilo Anthropic para planner/critic.
- GLM 5.2 mediante OpenCode o rutas estilo Z.AI.
- Gemini 3.5 Flash mediante Antigravity CLI.
- Codex CLI para orquestacion premium o escalamiento.
- Kilo y Aider en el runner con worktrees.
- Verificacion local por shell/tests.
- Workers offline `mock` para CI, demos y configuracion segura.
- Parsing de tokens/costos para logs de Codex, OpenCode, salidas CLI, cache reads, cache writes y reasoning tokens.
- Una skill SWARMS en `skills/swarms/` para que el agente de cada persona ayude a configurar planes, proveedores, limites y verificacion.

El router versionado solo habilita `mock`. Eso mantiene el clone local y gratis. Tu configuracion privada vive en archivos ignorados como `config/swarm_router.local.json` y en tus propias variables de entorno.

## Como Se Configura

Tu defines la politica:

- Los planes definen roles, tareas, dependencias, artefactos, comandos de verificacion y permisos premium.
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

Singularity es el loop autonomo para personas dispuestas a gastar tokens.

La idea es correr un equipo local 24/7: proponer mejoras, leer issues, crear tareas, lanzar workers, hacer QA, validar funcionalidades, resumir cambios y empezar el siguiente ciclo. Es el modo mas cercano a un loop de ingenieria permanente dentro de SWARMS.

```powershell
pwsh scripts/start_singularity.ps1 -MaxCycles 5
```

Tu controlas el riesgo. Con `mock`, Singularity es una simulacion local. Con proveedores reales, muchos workers y muchos ciclos, puede consumir muchisimos tokens. Usa provider caps, `MaxCycles` y un archivo `STOP_SINGULARITY` cuando lo pruebes.

## Politica De Proveedores

Intencion por rol:

- Planner: GLM 5.2 por defecto, Codex o rutas premium estilo Anthropic cuando el plan lo justifica.
- Critic: GLM 5.2 primero, revision premium para planes riesgosos o caros.
- Programmer: GLM 5.2, Gemini Flash, OpenAI-compatible, LiteLLM, Kilo, Aider o cualquier ruta que configures.
- Verifier: tests locales primero, modelo barato despues.
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
