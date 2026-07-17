# Investigar retención de sesiones y prompt cache por proveedor

Este ExecPlan es un documento vivo. Las secciones `Progress`, `Surprises &
Discoveries`, `Decision Log` y `Outcomes & Retrospective` deben mantenerse
actualizadas mientras avance la investigación.

## Purpose / Big Picture

El usuario necesita saber durante cuánto tiempo cada plan o proveedor que puede
usar SWARMS conserva dos recursos distintos: una sesión conversacional que se
puede reabrir y un prompt cache que puede reducir tokens o latencia. El resultado
observable será una matriz con duración publicada, alcance, condiciones de
invalidación, nivel de certeza y fuente para cada ruta configurada.

La implementación posterior debe además recuperar automáticamente una sesión
de proveedor cuando el subagente falla o el coordinador se reanuda. La sesión
sólo será elegible durante los 300 segundos posteriores a su última actualización;
después se ejecutará una conversación nueva para evitar contexto obsoleto.

## Progress

- [x] (2026-07-17 00:39Z) Reconciliados el router público, el router local y el
  inventario de proveedores de SWARMS.
- [x] (2026-07-17 00:39Z) Separados los conceptos sesión persistida, checkpoint
  de SWARMS y prompt cache del proveedor.
- [x] (2026-07-17 01:02Z) Investigadas documentación oficial y ayuda local de
  Codex, Claude Code, agy/Gemini, OpenCode/Z.AI y OpenRouter; Kilo y Hermes no
  están instalados.
- [x] (2026-07-17 01:02Z) Registrados resultados, incertidumbres y
  recomendaciones en `docs/technical/PROVIDER_SESSION_CACHE_RETENTION.md`.
- [x] (2026-07-17 01:07Z) Validados secciones obligatorias, nueve URLs de
  fuentes, ausencia de secretos y `git diff --check`.
- [x] (2026-07-17 01:18Z) Reconciliada la rama fusionada y creada
  `codex/auto-resume-provider-sessions` desde `origin/main`.
- [x] (2026-07-17 01:14Z) Añadidas pruebas rojas de captura, persistencia,
  vencimiento exacto a 300 segundos y límite de un reintento.
- [x] (2026-07-17 01:16Z) Implementada reanudación por ID exacto para Codex,
  OpenCode y agy, sin selectores globales `--last` o `--continue`.
- [x] (2026-07-17 01:18Z) Integrados reintento automático y recuperación tras
  reinicio en los coordinadores Python y Rust.
- [ ] Validar suites, documentar el contrato y publicar un PR draft.

## Surprises & Discoveries

- Observation: Rust está instalado, pero el host no tiene `link.exe` ni las
  bibliotecas del Windows SDK. `cargo fmt --check` pasa; `cargo check/test`
  quedan bloqueados antes de compilar el crate por `kernel32.lib` ausente.

- Observation: `config/swarm_router.local.json` habilita actualmente `mock`,
  OpenCode/Z.AI y agy/Antigravity, mientras el router público declara además
  Codex, varias rutas HY3, Kilo y Hermes.
  Evidence: `config/swarm_router.json` y `config/swarm_router.local.json`.
- Observation: Codex, agy y OpenCode conservan sesiones aunque termine el
  proceso one-shot, pero los wrappers actuales descartan sus identificadores.
  Evidence: ayuda instalada y `scripts/codex_worker.py`, `scripts/agy_call.py`
  y `scripts/opencode_worker.py`.
- Observation: los TTL publicados varían desde cinco minutos para prompt cache
  hasta treinta días para una transcripción Claude; no son la misma capa.
  Evidence: documentación oficial enlazada en el informe técnico.

## Decision Log

- Decision: informar por separado retención de conversación, retención de prompt
  cache y checkpoint local de SWARMS.
  Rationale: una sesión reanudable puede sobrevivir meses aunque el descuento de
  prompt cache expire en minutos; combinarlos produciría una respuesta falsa.
  Date/Author: 2026-07-17 / Codex.
- Decision: no inferir una duración cuando el proveedor no la publica.
  Rationale: los archivos locales demuestran persistencia actual, pero no una
  garantía contractual de retención.
  Date/Author: 2026-07-17 / Codex.
- Decision: usar un plazo local fijo de 300 segundos para decidir si SWARMS
  reanuda una sesión, independientemente del TTL comercial del prompt cache.
  Rationale: el ID de sesión preserva contexto; el límite corto reduce el riesgo
  de continuar estado obsoleto y coincide con la ventana mínima solicitada.
  Date/Author: 2026-07-17 / Codex.
- Decision: reanudar por ID exacto y nunca mediante `--last` o `--continue`.
  Rationale: un swarm paralelo puede crear varias sesiones simultáneas y los
  selectores implícitos pueden dirigir un task al chat equivocado.
  Date/Author: 2026-07-17 / Codex.

## Outcomes & Retrospective

La investigación produjo una matriz por ruta. Los únicos plazos contractuales
verificados son los publicados por OpenAI API, Anthropic/Claude, Gemini API y
OpenRouter response cache. Z.AI Coding Plan, agy, OpenCode Zen, HY3, Kilo y
Hermes no publican un TTL aplicable a esta integración. El checkpoint local de
SWARMS no expira automáticamente, pero `--force` lo elimina y una definición de
tarea modificada lo invalida.

La implementación agregó un sidecar atómico compartido, captura de sesiones en
Codex/OpenCode/agy, comandos de continuación por ID exacto y un único reintento
en los coordinadores Python y Rust. La ventana local es exactamente 300
segundos y puede configurarse con `SWARMS_SESSION_RESUME_WINDOW_SECONDS`.

## Context and Orientation

Las rutas se definen en `config/swarm_router.json` y se habilitan localmente en
`config/swarm_router.local.json`. Los wrappers viven bajo `scripts/`. Un
checkpoint de SWARMS es un `result.json` reutilizable por `--resume`; una sesión
es el historial persistido por la CLI; un prompt cache es una optimización del
proveedor que reutiliza prefijos de entrada y normalmente tiene una ventana
mucho menor.

## Plan of Work

Primero se inventariarán las rutas actuales y la CLI real que las ejecuta.
Después se consultarán fuentes oficiales para sesión y prompt cache. Cuando una
ruta agregadora, como OpenRouter, OpenCode o Hermes, dependa del proveedor
subyacente, se distinguirá lo garantizado por el agregador de lo delegado al
modelo final. Finalmente se consolidará una tabla operativa y se indicará qué
metadatos debe conservar SWARMS para aprovechar cada modalidad.

La fase de implementación añadirá un contrato mínimo en `status.json` para el
ID y la hora de actualización de la sesión. Los coordinadores Python y Rust
pasarán `--resume-session <id>` sólo cuando el estado tenga como máximo 300
segundos. Tras una salida fallida harán un único reintento automático; el mismo
estado permitirá que `--resume` continúe una tarea interrumpida sin elegir la
sesión más reciente de otro worker.

## Concrete Steps

Desde `C:\Proyectos\SWARMS`, inspeccionar router y wrappers con `rg`; consultar
la ayuda instalada de Codex, agy, Claude Code, OpenCode, Kilo y Hermes; buscar
documentación oficial de OpenAI, Anthropic, Google, Z.AI, OpenRouter y los demás
backends declarados. Guardar sólo conclusiones respaldadas por evidencia.

## Validation and Acceptance

La investigación se acepta cuando cada ruta declarada tiene una clasificación:
duración exacta publicada, persistencia local sin plazo garantizado, dependiente
del backend o no soportada. La tabla debe impedir confundir sesión reanudable con
prompt cache y debe identificar explícitamente cualquier dato desconocido.

La implementación se acepta cuando pruebas deterministas demuestran que una
sesión de 299 segundos se reanuda, una de más de 300 segundos se descarta, el
comando usa el ID exacto y no hay más de un reintento automático. Los flujos
mock existentes deben seguir pasando y Rust debe conservar el mismo contrato.

## Idempotence and Recovery

Las consultas son de sólo lectura. La actualización del plan es aditiva y puede
repetirse. Si una fuente deja de estar disponible, se conserva la incertidumbre
y no se reemplaza por una estimación.

Los reintentos serán idempotentes respecto del ID almacenado y estarán limitados
a uno. Un `status.json` corrupto, incompleto o vencido se ignora de forma segura.
`--force` elimina el run y su vínculo de sesión; `--resume` conserva el estado.

## Artifacts and Notes

- `docs/codex/plans/PLAN_PROVIDER_SESSION_CACHE_RETENTION.md`
- `docs/technical/PROVIDER_SESSION_CACHE_RETENTION.md`
- `config/swarm_router.json`
- `config/swarm_router.local.json`
- `scripts/workflow_runtime.py`

## Interfaces and Dependencies

La investigación depende de las interfaces de reanudación de cada CLI y de las
políticas de prompt caching publicadas por cada proveedor. No cambia todavía el
contrato del runtime. Una implementación posterior necesitaría persistir
`provider_session_id`, `provider_session_kind`, `created_at` y, cuando exista,
la política de expiración relevante.

## Plan Revision Notes

- 2026-07-17: creado para separar y auditar retención de sesiones, checkpoints y
  prompt cache en todas las rutas declaradas por SWARMS.
- 2026-07-17: añadida la matriz investigada y marcada explícitamente la ausencia
  de TTL contractual en las rutas agregadas o no instaladas.
- 2026-07-17: validación final completada; no se ejecutaron proveedores ni se
  consumió cuota durante la investigación.
- 2026-07-17: ampliado a implementación de reanudación automática con ventana
  fija de cinco minutos y un solo reintento por caída.
- 2026-07-17: implementación y pruebas dirigidas Python completadas; validación
  integral y publicación del PR pendientes.
