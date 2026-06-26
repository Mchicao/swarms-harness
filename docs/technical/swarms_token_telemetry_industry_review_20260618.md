# Investigacion y recomendaciones: telemetria de tokens en SWARMS

Fecha: 2026-06-18

## Resumen ejecutivo

Los proyectos lideres no tratan el consumo LLM como un contador simple de `prompt_tokens` y `completion_tokens`. El patron dominante es un modelo de uso normalizado, append-only o consultable por API, donde cada generacion conserva:

1. Identidad de ejecucion: request/generation id, trace id, user/key/team/project, proveedor real y modelo real.
2. Tipos de uso: input, output, cache read, cache write, reasoning, audio, image, tool/server-side usage cuando exista.
3. Costos separados por tipo: costo input normal, input cacheado, cache write, output, razonamiento si el proveedor lo factura distinto.
4. Fuente de verdad: `api_reported` primero; despues `gateway_reported`; despues `tokenizer_estimated`; finalmente `missing`.
5. Politicas de control: presupuesto por key/user/team/provider/model, ventanas de reset, limites RPM/TPM/concurrencia y alertas antes del corte.
6. Auditoria historica: posibilidad de consultar stats post-request por id, recalcular con catalogos versionados de precios y explicar por que una llamada se bloqueo.

SWARMS ya tiene una base correcta: un JSONL central, campos para `input_tokens`, `cache_read_tokens`, `cache_write_tokens`, `output_tokens`, `reasoning_tokens`, `usage_source`, `cost_usd`, fases y roles. La brecha principal es que la implementacion todavia mezcla estimaciones fragiles con costos hardcodeados, no versiona pricing ni cuotas, no distingue con precision cache read vs cache write en el costo, no tiene presupuestos ejecutables por provider/model/plan, y no captura suficiente contexto para auditar rutas, retries, workers y proveedores reales.

## Como lo resuelven proyectos lideres

### LiteLLM

LiteLLM opera como gateway/proxy y por eso su unidad de control es la virtual key. La documentacion de Virtual Keys indica que permite trackear gasto y controlar acceso por modelo; el gasto puede consultarse por key, user y team, y se actualiza automaticamente cuando pasan llamadas por endpoints como completions/chat/embeddings. Tambien declara que el costo por modelo se guarda en una tabla de modelos/precios y se calcula con `completion_cost()`.

Fuentes:

1. https://docs.litellm.ai/docs/proxy/virtual_keys
2. https://docs.litellm.ai/docs/proxy/users

Patrones relevantes:

1. El gasto no vive solo en logs: se materializa en tablas por key, user y team.
2. El calculo de costo se desacopla de la llamada mediante una tabla de precios por modelo.
3. Los presupuestos no son globales solamente. LiteLLM permite presupuestos por usuario, virtual key y equipo.
4. El presupuesto puede tener duracion de reset y multiples ventanas concurrentes, por ejemplo diaria y mensual.
5. Al cruzar `max_budget`, el gateway bloquea nuevas requests con un error explicito.
6. Tiene controles complementarios: limites RPM, TPM, max parallel requests, acceso a modelos y aliases para upgrade/downgrade.

Implicacion para SWARMS: el archivo `.agent/usage_stats.json` y el `daily_limit` de requests son insuficientes. SWARMS necesita cuotas en terminos de dinero y tokens, por provider/model/role, con ventanas diarias y mensuales.

### Langfuse

Langfuse modela el uso en observaciones de tipo `generation` y `embedding`. Su documentacion recomienda ingerir `usage_details` y `cost_details` desde la respuesta del proveedor cuando existan, y permite tipos de uso arbitrarios. En sus ejemplos separa `input`, `output` y `cache_read_input_tokens`; tambien permite `cached_tokens`, `audio_tokens`, `image_tokens` u otros tipos. Si no se ingiere `total`, Langfuse lo deriva. Si no se ingiere costo, puede inferirlo segun el modelo y definiciones de precio.

Fuente:

1. https://langfuse.com/docs/observability/features/token-and-cost-tracking

Patrones relevantes:

1. Uso y costo son dos mapas extensibles, no columnas cerradas.
2. Los tipos de uso pueden variar por proveedor.
3. Los datos reportados por la API tienen prioridad sobre inferencias.
4. El modelo y el precio usados al momento de ingestion importan, porque el pricing cambia.
5. La metrica debe ser consultable agregada por usuario, tags, aplicacion o filtros.

Implicacion para SWARMS: conservar columnas canonicas esta bien, pero conviene agregar `usage_details` y `cost_details` como objetos extensibles. Esto evita perder tokens especificos de proveedores nuevos.

### OpenRouter

OpenRouter normaliza la respuesta al esquema de Chat Completions y declara que siempre retorna informacion detallada de uso. Para streaming, el usage llega una vez en el chunk final. El esquema incluye `prompt_tokens`, `completion_tokens`, `total_tokens`, `prompt_tokens_details.cached_tokens`, `prompt_tokens_details.cache_write_tokens`, `completion_tokens_details.reasoning_tokens`, `cost`, `is_byok`, `cost_details` y `server_tool_use`. Tambien permite consultar estadisticas historicas por generation id via `/api/v1/generation`.

Fuente:

1. https://openrouter.ai/docs/api/reference/overview

Patrones relevantes:

1. El proveedor/gateway normaliza pero conserva detalles nativos.
2. Streaming debe manejar el usage final, no solo chunks de contenido.
3. Se registra el costo devuelto por el gateway cuando existe.
4. Se conserva el id de generacion para auditoria posterior.
5. Cache read y cache write son campos distintos.
6. Herramientas server-side, como web search, se registran fuera de tokens de texto.

Implicacion para SWARMS: los parsers deben soportar final streaming usage, `cost`, `cost_details`, `cache_write_tokens`, `is_byok`, `server_tool_use` y `generation_id`.

### Portkey

Portkey se posiciona como gateway, catalogo de modelos y capa de gobierno. Su documentacion de Model Catalog describe proveedores con credenciales seguras, presupuestos, rate limits, allow-lists y modelos con limites input/output y pricing cuando disponible. La documentacion de Budget Limits permite limites por costo o por tokens, alertas por umbral, reset semanal/mensual y monitoreo por proveedor en Analytics. Tambien advierte que si una request queda con costo `0 cents` por falta de pricing, no cuenta para presupuesto de costo; para presupuestos por tokens, rastrea input y output.

Fuentes:

1. https://portkey.ai/docs/product/model-catalog
2. https://portkey.ai/docs/product/ai-gateway/virtual-keys/budget-limits

Patrones relevantes:

1. El catalogo de modelos no es solo descriptivo: gobierna acceso, pricing, limites y credenciales.
2. Hay budgets por proveedor/integracion y workspace.
3. Existen budgets en costo y budgets en tokens; no son equivalentes.
4. Las alertas ocurren antes del corte.
5. El sistema reconoce limitaciones de pricing y evita cobrar lo que no sabe estimar.

Implicacion para SWARMS: separar `token_budget` y `cost_budget` por plan, provider y role. Si el precio es desconocido, no fingir costo cero como si fuera gratis: marcar `pricing_status=unknown` y excluir o alertar.

### Hallazgo academico reciente sobre agentes de codigo

Un estudio de abril de 2026 sobre tareas agenticas de codigo en SWE-bench Verified reporta que los agentes pueden consumir muchisimos mas tokens que chat/coding simple; que los input tokens dominan el costo; que ejecuciones repetidas de la misma tarea pueden variar hasta 30x; y que mayor gasto no implica necesariamente mejor exactitud. Tambien encuentra que los modelos suelen subestimar su propio costo.

Fuente:

1. https://arxiv.org/abs/2604.22750

Implicacion para SWARMS: no basta con medir al final. El orquestador necesita guardrails antes y durante la ejecucion: estimacion previa, presupuesto por intento, stop-loss de retries y metricas de costo por tarea resuelta.

## Evaluacion de la implementacion actual de SWARMS

Archivos revisados:

1. @scripts/utils/token_telemetry.py
2. @scripts/parallel_swarm.ps1
3. @scripts/run_swarm_benchmark.py
4. @scripts/scout_limits.py
5. @scripts/goal_evaluator.py
6. @scripts/smart_router.py
7. @docs/technical/swarm_token_savings_benchmark_plan.md
8. @docs/TOKEN_OPTIMIZATION.md

### Lo que esta bien encaminado

1. SWARMS ya escribe telemetria append-only en `.agent/traces/telemetry.jsonl`.
2. El esquema ya contiene los campos criticos basicos: `run_id`, `benchmark_id`, `phase`, `provider`, `model`, `role`, `task_id`, `input_tokens`, `cache_read_tokens`, `cache_write_tokens`, `output_tokens`, `reasoning_tokens`, `usage_source`, `success`, timestamps y `cost_usd`.
3. El plan de benchmark ya define metricas utiles: CTR, TCR, TTA, pass rate y costo por tarea resuelta.
4. El orquestador intenta registrar workers, overhead de watchers, Kilo, Antigravity y goal evaluation.
5. `scout_limits.py` ya mide disponibilidad, latencia y concurrencia aproximada antes del despacho.
6. La arquitectura reconoce phases y roles, lo cual permite separar coordinador, worker y overhead.

### Riesgos y brechas

1. `PRICING_TABLE` esta hardcodeada en codigo. Esto hace dificil auditar cambios de precios, fecha de vigencia, moneda, fuente y diferencias por proveedor/gateway.
2. `calculate_cost()` solo descuenta `cache_read_tokens`; ignora `cache_write_tokens` aunque varios proveedores/gateways lo reportan y lo facturan distinto.
3. `reasoning_tokens` se registra pero no se calcula como costo independiente ni como subtipo de output. En algunos proveedores puede tener reglas de billing propias o al menos debe sumarse con claridad al output total.
4. `usage_source="estimated"` se usa aunque no haya estimacion real. En muchos paths, si el parser no encuentra tokens, quedan ceros y se marca estimated. Eso confunde "estimado" con "missing".
5. `parse_stdout_text()` depende de regex sobre salida humana. Es fragil y puede capturar texto de logs que no sea usage real.
6. `parse_codex_log()` devuelve el primer bloque `usage`, no necesariamente el ultimo o acumulado. En ejecuciones con multiples eventos puede subcontar.
7. `run_swarm_benchmark.py` calcula `swarm_cost` asumiendo que todo el swarm es GLM-5.2, aunque la telemetria real puede mezclar Codex, Z.AI, Gemini, Kilo, watchers y retries.
8. El archivo `.agent/usage_stats.json` solo cuenta requests y un `daily_limit` global. No hay cuotas por proveedor, modelo, rol, costo, tokens, ventana ni plan.
9. `scout_limits.py` mide disponibilidad y concurrencia, pero no captura headers de rate limit, remaining quota, reset time, credit balance ni costo esperado.
10. No hay catalogo versionado de modelos/proveedores. Los nombres como `gpt-5.5-codex`, `gemini-3.5-flash` y `glm-5.2` viven duplicados entre scripts.
11. No se registra `request_id`, `generation_id`, `provider_request_id`, `parent_event_id` ni `attempt`. Esto limita auditoria de retries y overhead.
12. No se registra `streaming=true/false`, `is_byok`, `server_tool_use`, `finish_reason`, `error_type`, `http_status`, ni `rate_limit_reset`.
13. No existe bloqueo presupuestario real antes de lanzar workers. La telemetria mide, pero no gobierna.
14. No hay agregador robusto de reportes por run que derive costo desde eventos heterogeneos; el benchmark reimplementa calculos parciales.
15. Los eventos JSONL no incluyen version de pricing ni fuente del precio, por lo que un reporte historico puede cambiar si se recalcula con precios nuevos.

## Recomendacion de arquitectura para SWARMS

### 1. Separar "usage events" de "model catalog"

Mantener `.agent/traces/telemetry.jsonl`, pero mover precios y limites a un catalogo versionado, por ejemplo:

```yaml
schema_version: 1
currency: USD
effective_at: "2026-06-18"
providers:
  openrouter:
    plans:
      free:
        daily_requests: 50
        monthly_cost_usd: 0
      paid:
        monthly_cost_usd: 20
    models:
      openai/gpt-5.2:
        input_per_1m: 0
        cache_read_input_per_1m: 0
        cache_write_input_per_1m: 0
        output_per_1m: 0
        pricing_status: unknown
  zai_coding:
    models:
      glm-5.2:
        input_per_1m: 0.10
        cache_read_input_per_1m: 0.05
        cache_write_input_per_1m: 0.10
        output_per_1m: 0.20
        pricing_status: configured
```

El evento debe guardar `pricing_catalog_version` y `pricing_source`. Si `pricing_status=unknown`, el costo debe quedar `null` o `estimated=false`, no `0.0` salvo que sea realmente gratis por contrato.

### 2. Adoptar un esquema canonico mas extensible

Propuesta V2 para cada evento:

```json
{
  "schema_version": "2.0",
  "event_id": "uuid",
  "parent_event_id": "uuid|null",
  "run_id": "uuid",
  "benchmark_id": "uuid",
  "task_id": "string",
  "attempt": 1,
  "phase": "baseline|swarm|watcher|retry|goal_eval|scout",
  "role": "coordinator|worker|overhead|router",
  "provider": "openrouter|litellm|zai_coding|antigravity_cli|codex_cli|kilo",
  "provider_route": "string|null",
  "model_requested": "string",
  "model_resolved": "string",
  "usage_details": {
    "input": 0,
    "cache_read_input_tokens": 0,
    "cache_write_input_tokens": 0,
    "output": 0,
    "reasoning_output_tokens": 0,
    "audio_input_tokens": 0,
    "image_input_tokens": 0
  },
  "cost_details": {
    "input": 0.0,
    "cache_read_input_tokens": 0.0,
    "cache_write_input_tokens": 0.0,
    "output": 0.0,
    "reasoning_output_tokens": 0.0,
    "total": 0.0
  },
  "usage_source": "api_reported|gateway_reported|cli_reported|tokenizer_estimated|missing",
  "cost_source": "api_reported|gateway_reported|catalog_estimated|missing",
  "pricing_catalog_version": "2026-06-18",
  "request_id": "string|null",
  "generation_id": "string|null",
  "streaming": false,
  "finish_reason": "stop|length|tool_calls|error|null",
  "server_tool_use": {},
  "success": true,
  "http_status": 200,
  "error_type": null,
  "started_at": "ISO",
  "ended_at": "ISO",
  "duration_ms": 0
}
```

Conservar campos legacy (`input_tokens`, `output_tokens`, etc.) durante una transicion puede ser util, pero los reportes nuevos deberian leer `usage_details`.

### 3. Crear adaptadores por proveedor

En vez de un parser generico con regex, implementar adaptadores:

1. `parse_openai_like_usage(response_or_json)`: soporta `prompt_tokens`, `completion_tokens`, `prompt_tokens_details.cached_tokens`, `prompt_tokens_details.cache_write_tokens`, `completion_tokens_details.reasoning_tokens`, `cost`, `cost_details`.
2. `parse_openrouter_usage(response_or_stream_final_chunk)`: conserva `generation_id`, `cost`, `is_byok`, `server_tool_use`.
3. `parse_anthropic_usage(response)`: mapear `input_tokens`, `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`.
4. `parse_zai_usage(response)`: manejar `dict` y object-like como ya propone el plan.
5. `parse_cli_usage(log)`: solo para CLI, con `usage_source=cli_reported` si existe bloque estructurado; si no, `missing` o `tokenizer_estimated` con tokenizer explicito.

Regla: no mezclar stdout humano con uso confiable salvo que el CLI emita JSON estructurado.

### 4. Hacer budgets ejecutables, no solo informes

Agregar un `budget_guard.py` llamado antes de despachar workers y antes de retries. Debe leer:

1. Catalogo de precios.
2. Eventos JSONL del dia/mes/run.
3. Plan de ejecucion: provider/model/worker count/attempts.
4. Presupuesto configurado.

Presupuestos minimos:

1. Por run: `max_run_cost_usd`, `max_run_tokens`.
2. Por dia: `max_daily_cost_usd`, `max_daily_tokens`.
3. Por provider/model: `max_daily_cost_usd`, `max_concurrent_workers`, `rpm`, `tpm`.
4. Por role: coordinador caro vs workers baratos vs overhead.
5. Por retries: `max_retry_cost_usd` y `max_retry_attempts`.

Comportamiento recomendado:

1. `warn`: sobre 70% del presupuesto.
2. `degrade`: sobre 85%, mover tareas a modelos baratos o bajar worker count.
3. `stop`: sobre 95% o si la proyeccion del siguiente batch cruza el limite.

### 5. Corregir las metricas del benchmark

El benchmark debe calcular costos desde eventos reales:

1. `baseline_cost = sum(cost_details.total where phase=baseline)`.
2. `swarm_cost = sum(cost_details.total where run_id=swarm_run_id)`.
3. `coordinator_expensive_tokens = sum(input+output+reasoning for role=coordinator and provider in expensive_set)`.
4. `worker_tokens = sum(...) by role=worker`.
5. `overhead_tokens = sum(...) by role=overhead`.

No asumir que todo el swarm es GLM-5.2.

Metricas nuevas:

1. `cache_hit_ratio = cache_read_input_tokens / (input + cache_read_input_tokens)`.
2. `cache_write_ratio = cache_write_input_tokens / total_input_like_tokens`.
3. `overhead_cost_ratio = overhead_cost / total_swarm_cost`.
4. `retry_cost_ratio = retry_cost / total_swarm_cost`.
5. `cost_per_successful_task`.
6. `tokens_per_merged_diff_line`.
7. `estimated_vs_reported_ratio`, para medir calidad de estimadores.

### 6. Mejorar `scout_limits.py`

El scout deberia producir algo mas parecido a una foto de capacidad:

```yaml
providers:
  openrouter:
    status: ok
    max_safe_concurrency: 2
    models:
      qwen/qwen3-coder:free:
        status: ok
        latency_ms_p50: 1200
        rpm_remaining: null
        tpm_remaining: null
        reset_at: null
        pricing_status: provider_reported
        usage_probe:
          input_tokens: 2
          output_tokens: 1
          cost_usd: 0.0
```

Para APIs HTTP, capturar headers de rate limit cuando existan. Para CLI, registrar explicitamente `quota_visibility=none` si no hay forma de saber remaining quota.

### 7. Tratar prompt cache como producto medible

SWARMS ya tiene una guia de optimizacion de tokens en @docs/TOKEN_OPTIMIZATION.md. Para hacerla verificable:

1. Registrar hash del prefijo estable: `prompt_prefix_hash`.
2. Registrar tamano del prefijo cacheable: `cacheable_prefix_tokens`.
3. Registrar `cache_read_input_tokens` y `cache_write_input_tokens`.
4. Medir hit ratio por worker, provider y fase.
5. Alertar si una edicion accidental del prompt estable destruye el hit ratio.

### 8. Prioridad de implementacion

Orden recomendado:

1. Crear `config/model_catalog.yaml` con precios, cuotas y `effective_at`.
2. Refactorizar @scripts/utils/token_telemetry.py para V2: `usage_details`, `cost_details`, `pricing_catalog_version`, `usage_source=missing` cuando corresponda.
3. Reemplazar calculos hardcodeados de @scripts/run_swarm_benchmark.py por agregacion desde eventos.
4. Agregar `budget_guard.py` y llamarlo en @scripts/parallel_swarm.ps1 antes de lanzar un batch y antes de cada retry.
5. Mejorar parsers por proveedor y soportar OpenRouter/LiteLLM style usage.
6. Extender @scripts/scout_limits.py para capturar capacidad, rate limits y usage probe.
7. Crear un reporte `summarize_telemetry.py` que agrupe por run, task, provider, model, role y phase.
8. Agregar tests unitarios para parsers y calculo de costo, especialmente cache read/write, reasoning y missing usage.

## Decision recomendada

SWARMS no deberia convertirse inmediatamente en un gateway completo como LiteLLM o Portkey. Su rol natural es orquestador local de agentes. Pero si debe adoptar tres ideas de gateway:

1. Catalogo gobernado de modelos, precios y limites.
2. Telemetria normalizada y auditable por request/generation.
3. Presupuestos ejecutables antes de gastar, no solo reportes despues de gastar.

La implementacion actual esta cerca para un V1 publico, pero no es suficiente para tomar decisiones economicas reales. El siguiente salto de calidad es reemplazar costos hardcodeados y regex heuristics por un pipeline de ingestion normalizado, con `api_reported` como fuente de verdad, `missing` como estado honesto, pricing versionado y guardrails de presupuesto.
