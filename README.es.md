# SWARMS

Arnes experimental para ahorrar cuota de modelos en flujos de coding agents.

SWARMS existe para ejecutar flujos de programacion agentica y paralela sin gastar modelos premium en cada paso. La idea es usar modelos caros solo donde realmente aportan: planificacion, critica, revision de alto riesgo y escalamiento. La ejecucion rutinaria queda en scripts deterministas, workers baratos, tests locales o el worker offline `mock`.

El estado publico por defecto es seguro: despues de clonar, SWARMS solo usa `mock`. Proveedores reales como GLM 5.2, Gemini Flash, Codex, Claude u Opus quedan deshabilitados hasta que el usuario cree configuracion local y los pida explicitamente.

English docs: [README.md](README.md)

## Estado

SWARMS esta en alpha. El MVP publicable incluye el flujo offline, revision estatica de planes, runtime deterministico, worker mock, pruebas y CI. Los adaptadores reales de proveedores estan documentados como experimentales o reservados hasta que tengan setup local estable, telemetria y controles de seguridad.

## Origen

SWARMS comenzo alrededor de enero-febrero de 2026, cuando se hicieron populares los loops estilo Ralph. La idea original era paralelizar esos loops usando los modelos baratos que tenia disponibles como estudiante, especialmente Gemini en Antigravity, y reservar Opus para crear planes.

Hoy la idea es usar GLM 5.2 y Gemini Flash para subagentes de programacion o verificacion rutinaria, y reservar Codex u Opus para planificacion, critica, trabajo sensible o escalamiento explicito.

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

## Arquitectura Publica

```text
Objetivo del usuario
  -> planner escribe un workflow_plan.json estructurado
  -> reviewer estatico valida alcance, dependencias, presupuesto y uso premium
  -> runtime deterministico agenda tareas con limites por proveedor y locks
  -> workers ejecutan tareas acotadas
  -> verificadores y comandos locales validan resultados
  -> report.json registra estado, resultados y campos de costo/tokens
```

Archivos centrales:

- `scripts/swarm.py`: CLI publico unico.
- `scripts/plan_review.py`: revision estatica de planes.
- `scripts/workflow_runtime.py`: runtime deterministico.
- `scripts/mock_worker.py`: worker offline para pruebas y demos.
- `scripts/doctor.py`: chequeo local de salud.
- `config/role_policy.json`: politica por rol.
- `config/swarm_router.json`: configuracion segura offline.
- `docs/workflow_plan_example.json`: plan de ejemplo funcional.

## Politica De Proveedores

La configuracion versionada solo habilita `mock`. La configuracion local de proveedores reales debe vivir en `config/swarm_router.local.json`, que esta ignorado por Git.

Intencion por rol:

- Planner: GLM 5.2 por defecto, Codex solo con justificacion explicita.
- Critic: GLM 5.2 primero, Codex solo para planes riesgosos o caros.
- Programmer: GLM 5.2 o Gemini Flash cuando esten configurados y solicitados.
- Verifier: tests locales primero, modelo barato despues, premium solo por politica.
- Claude, Codex, Opus y otros modelos premium: deshabilitados por defecto.

Ver `docs/PROVIDER_STATUS.md` y `docs/CONFIG.md`.

## Seguridad

No subir:

- `.env`
- `config/*.local.json`
- API keys, OAuth tokens, auth files o credenciales privadas
- `.agent/`
- worktrees
- prompts, logs, traces, reportes o telemetria generada

El flujo por defecto no llama APIs pagadas ni proveedores externos.

## Verificacion

```powershell
python -m py_compile scripts\swarm.py scripts\plan_review.py scripts\workflow_runtime.py scripts\doctor.py scripts\mock_worker.py
python -m pytest tests -q
python scripts/swarm.py doctor
python scripts/swarm.py run --plan docs/workflow_plan_example.json --force --run-id verify-readme --global-max-concurrency 3 --provider-cap mock=3
```

## Licencia

MIT. Ver `LICENSE`.
