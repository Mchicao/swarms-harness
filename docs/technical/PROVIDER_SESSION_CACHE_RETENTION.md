# Retención de sesiones, checkpoints y prompt cache por proveedor

Fecha de verificación: 2026-07-16 (America/Santiago).

## Resumen ejecutivo

SWARMS mezcla actualmente tres capas que deben tratarse por separado:

1. El **checkpoint de SWARMS** es un resultado local terminado que permite
   saltarse por completo un worker durante `--resume`.
2. La **sesión de una CLI** es el chat persistido por Codex, Claude Code, agy u
   OpenCode y puede sobrevivir al cierre de la consola.
3. El **prompt cache del proveedor** reutiliza prefijos de tokens durante un
   plazo mucho menor para reducir coste o latencia. No reabre por sí solo un
   chat.

No existe un único plazo por plan de suscripción. Cada capa y backend tiene su
propia política.

## Matriz de retención

| Ruta o plan | Sesión conversacional | Prompt cache | Aplicación real en SWARMS hoy |
|---|---|---|---|
| Checkpoint SWARMS Python/Rust | No tiene TTL automático. Permanece mientras exista el directorio del `run_id`. `--force` lo elimina; un cambio en la definición invalida su clave. | No aplica. | Sí. Es lo único que `--resume` reutiliza actualmente. Los checkpoints Python y Rust no son intercambiables. |
| Codex CLI con cuenta ChatGPT/Codex | La CLI persiste sesiones salvo que se use `--ephemeral`. La documentación consultada no publica un vencimiento automático; se conservan hasta borrado o limpieza externa. En esta máquina existen sesiones con casi cinco meses de antigüedad, lo cual es evidencia local, no garantía contractual. | La CLI no expone una garantía de TTL para el plan ChatGPT. En API, GPT-5.6 o posterior conserva prefijos al menos 30 minutos; modelos anteriores usan 5–10 minutos de inactividad, máximo 1 hora en memoria, o hasta 24 horas con retención extendida. GPT-5.5 API usa política `24h`. | El worker captura `thread.started`, persiste el ID y reanuda con `codex exec resume <ID>` durante un máximo local de 300 segundos. |
| Claude Code Pro/Max o API | Las transcripciones bajo `~/.claude/projects` se eliminan al arrancar cuando superan `cleanupPeriodDays`; el valor predeterminado es 30 días y es configurable. `--no-session-persistence` evita guardarlas. | 5 minutos por defecto; 1 hora si se solicita el TTL extendido. | La CLI instalada soporta `--resume`, pero el runtime público de SWARMS no tiene `claude_worker`; por tanto no captura sesiones Claude todavía. |
| agy / Google AI Pro / Antigravity | agy guarda conversaciones en bases SQLite locales y permite `--continue` o `--conversation`. No se encontró un TTL oficial de limpieza para la CLI. | El TTL del backend usado por el plan Google AI Pro no está publicado para agy. No debe confundirse con Gemini API: allí la caché explícita dura 1 hora por defecto y puede configurarse; la caché implícita no publica una duración garantizada. Vertex AI documenta caché en memoria con TTL de hasta 24 horas, pero esa política tampoco demuestra el comportamiento de agy. | El wrapper persiste el ID detectado y reanuda exclusivamente con `agy --conversation <ID>` dentro de la ventana local de 300 segundos. |
| OpenCode con Z.AI Coding Plan / GLM | OpenCode conserva sesiones en su base local, permite listarlas, exportarlas y borrarlas. No publica un vencimiento automático. En esta máquina hay datos OpenCode de varios meses, sin que eso constituya una garantía. | Z.AI documenta caché implícita automática y reporta `cached_tokens`, pero no publica el TTL. | El worker captura `sessionID` de JSONL y reanuda con `opencode run --session <ID>` durante un máximo local de 300 segundos. |
| OpenRouter / HY3 | La API no mantiene por sí sola un chat recuperable; el cliente debe reenviar mensajes. `session_id` fija modelo/proveedor para routing pegajoso, no almacena el historial como una CLI. | El prompt cache depende del proveedor final. El caché de **respuesta** de OpenRouter es otra función: está apagado salvo cabecera/preset, dura 5 minutos por defecto y admite de 1 segundo a 24 horas. | El worker OpenAI-compatible no envía `session_id`, `X-OpenRouter-Cache` ni TTL; por tanto el response cache de OpenRouter está apagado hoy. |
| Novita, GitLawb y SiliconFlow / HY3 | No hay sesión CLI en el wrapper; cada llamada es una petición independiente. | No se encontró un TTL oficial verificable para las rutas HY3 declaradas. Puede depender del gateway y backend efectivo. | Las rutas están declaradas, pero no deben anunciarse como caché reutilizable. Novita y SiliconFlow están deshabilitadas; GitLawb no está habilitado en el router local actual. |
| OpenCode Zen / HY3 | Hereda la persistencia local de sesiones de OpenCode. | Depende del backend que Zen seleccione; no hay TTL contractual publicado para esta ruta. | Declarada pero no habilitada localmente. El worker tampoco guarda el ID de sesión. |
| Kilo / HY3 | Depende de la persistencia implementada por Kilo. No se verificó localmente porque la CLI no está instalada. | Depende del proveedor final. | Ruta declarada y deshabilitada; no hay evidencia local para asignarle duración. |
| Hermes / Nous / HY3 | Depende del almacén de sesiones de Hermes. No se verificó localmente porque la CLI no está instalada. | Depende del proveedor que Hermes use; con HY3 se fuerza Nous, pero no se encontró TTL publicado. | Rutas declaradas y deshabilitadas; no hay evidencia para asignar duración. |
| Mock | No mantiene conversación. | No mantiene prompt cache. | Los resultados terminados sí quedan cubiertos por el checkpoint local de SWARMS. |

## Qué significa para una reanudación

Una tarea terminada puede reutilizar su checkpoint sin gastar tokens durante
todo el tiempo que permanezca el directorio del run. Una tarea interrumpida no
recibe ese beneficio: SWARMS la vuelve a despachar como conversación nueva,
aunque la CLI haya dejado una sesión recuperable. Desde esta implementación,
Codex, OpenCode y agy conservan el ID exacto en `status.json`; Python y Rust
hacen como máximo una continuación automática si el estado tiene 300 segundos
o menos. Un estado vencido, futuro, corrupto o sin ID inicia una sesión nueva.

El mayor valor inmediato está en guardar el ID exacto de sesión por tarea. No
se debe usar `--last` o `--continue` en un swarm paralelo porque puede seleccionar
la sesión de otro worker. El estado mínimo debería registrar
`provider_session_id`, `provider_session_kind`, `provider_session_created_at`,
`provider`, `model`, `workspace_root` y la clave del checkpoint.

Para prompt caching, SWARMS debe registrar los contadores reales de cache read y
cache write. Un TTL publicado sólo indica elegibilidad; no garantiza un hit si
cambia el prefijo, el modelo, la región, la organización, el API key o el
proveedor seleccionado por un gateway.

## Fuentes principales

- OpenAI, [Prompt caching](https://developers.openai.com/api/docs/guides/prompt-caching): GPT-5.6+ mínimo 30 minutos; memoria 5–10 minutos de inactividad y máximo 1 hora; retención extendida hasta 24 horas.
- Claude Code, [Explore the `.claude` directory](https://code.claude.com/docs/en/claude-directory): limpieza de transcripciones con `cleanupPeriodDays`, 30 días por defecto.
- Anthropic, [Pricing and prompt caching](https://docs.anthropic.com/en/docs/about-claude/pricing): TTL de 5 minutos y 1 hora.
- Google AI, [Context caching](https://ai.google.dev/gemini-api/docs/generate-content/caching): caché explícita de Gemini API, 1 hora por defecto y TTL configurable.
- Google Cloud, [Vertex AI zero data retention](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/vertex-ai-zero-data-retention): caché en memoria con TTL de 24 horas en Vertex AI.
- Z.AI, [Context caching](https://docs.z.ai/guides/capabilities/cache): caché implícita automática y telemetría `cached_tokens`, sin TTL publicado.
- OpenCode, [CLI session commands](https://dev.opencode.ai/docs/cli/): listado, exportación y borrado de sesiones locales.
- OpenRouter, [Prompt caching](https://openrouter.ai/docs/guides/best-practices/prompt-caching) y [Response caching](https://openrouter.ai/docs/guides/features/response-caching): routing pegajoso, TTL dependiente del proveedor y response cache de 5 minutos por defecto, configurable hasta 24 horas.

## Nivel de certeza

Los plazos exactos de OpenAI API, Claude, Gemini API y OpenRouter response cache
son contractuales según la documentación enlazada. La permanencia observada en
los discos locales sólo prueba el estado de esta máquina. Para agy, Z.AI Coding
Plan, OpenCode Zen, HY3, Kilo y Hermes no hay un TTL contractual verificado; el
valor correcto es **desconocido**, no infinito.
