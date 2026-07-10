# AGY (Antigravity CLI) — Uso Programático

## Resumen ejecutivo

`agy` (CLI de Google Antigravity, v1.0.10+) **sí se puede usar de forma
programática**, pero **`agy --print` NO imprime la respuesta del modelo a
stdout en entornos sin TTY** (headless / CI / `subprocess.run`). El proceso
termina con código 0, la petición HTTP al modelo se completa, y la respuesta
**se persiste** en la conversación SQLite, pero stdout queda vacío.

Por eso los agentes que hacían:

```python
result = subprocess.run(["agy", "--print", prompt], capture_output=True)
answer = result.stdout  # SIEMPRE VACÍO en headless
```

...creían que "la CLI no responde". Sí responde: solo que no por stdout.

## Solución: `scripts/agy_call.py`

Wrapper que (1) lanza `agy --print` (que genera y persiste la respuesta) y
(2) lee la respuesta de la conversación SQLite recién escrita, parseando el
payload protobuf del turno del asistente.

### Uso como librería

```python
from scripts.agy_call import agy_complete

answer = agy_complete(
    "¿Cuánto es 7 por 8? Responde solo el número.",
    model="Gemini 3.5 Flash (Medium)",
)
print(answer)  # -> "56"
```

### Uso desde CLI

```powershell
python scripts/agy_call.py "Say OK" --model "Gemini 3.5 Flash (Medium)"
```

### Pruebas realizadas (2026-06-24)

| Prompt | Modelo | Salida | OK |
|---|---|---|---|
| "Responde solo: HOLA" | Gemini 3.5 Flash (Medium) | `HOLA` | ✓ |
| "¿Cuánto es 7×8? Solo el número." | Gemini 3.5 Flash (Medium) | `56` | ✓ |
| "¿Cuánto es 9×9? Solo el número." | Gemini 3.5 Flash (Medium) | `81` | ✓ |
| "Traduce 'buenos dias' al inglés." | Gemini 3.5 Flash (Medium) | `Good morning` | ✓ |

## Labels de modelo válidos

El flag `--model` **exige el label exacto**; si no coincide, agy ignora
silenciosamente el flag y usa el modelo por defecto (`Gemini 3.1 Pro (Low)`),
o la petición falla sin respuesta. Labels confirmados en uso histórico:

- `Gemini 3.5 Flash (Low)`
- `Gemini 3.5 Flash (Medium)` ← recomendado (más usado y estable)
- `Gemini 3.5 Flash (High)`
- `Gemini 3.1 Pro (Low)` / `(Medium)` / `(High)`
- `Gemini 3 Flash`, `Gemini 3.1 Flash Lite`

> Nota: `agy models` también sale vacío en headless, por el mismo bug de
> stdout. Para descubrir labels, inspeccionar transcripts en
> `%USERPROFILE%\.gemini\antigravity-cli\brain\*\transcript*.jsonl`.

## Autenticación

`agy` autentica vía keyring OAuth (silent auth) con la cuenta Google
configurada. No requiere `GOOGLE_API_KEY`. El wrapper usa `--sandbox` por
defecto y no omite permisos. Solo `tools_policy=full` agrega
`--dangerously-skip-permissions` de forma explícita.

Para entornos sin keyring, se puede usar ADC: `USE_ADC=1 agy ...`.

## Qué NO hacer

- **No** usar `agy -p "..."` directo y leer stdout: siempre vacío en headless.
- **No** usar `winpty`: falla con `ASSERT_CONDITION` en este entorno.
- **No** usar timeout corto (<120s): agy tarda ~15-20s solo en auth+setup.
- **No** pasar `--model gemini-2.5-flash` (nombre de API): se ignora. Usar el
  label completo `Gemini 3.5 Flash (Medium)`.

## Dónde se persisten las respuestas

- Conversaciones: `%USERPROFILE%\.gemini\antigravity-cli\conversations\<uuid>.db`
  - Tabla `steps`, columna `step_payload` (protobuf), `step_type=15` = turno del asistente.
- Transcripts legibles: `%USERPROFILE%\.gemini\antigravity-cli\brain\<uuid>\.system_generated\logs\transcript.jsonl`
- Log de diagnóstico: `agy --log-file <path> ...`

## Integración con SWARMS

`scripts/goal_evaluator.py` y `scripts/parallel_swarm.ps1` invocaban `agy
--print` directamente y recibían stdout vacío. Migrarlos a `agy_complete()`
de `scripts/agy_call.py` para que el planner/critic/verifier Gemini funcionen
realmente.
