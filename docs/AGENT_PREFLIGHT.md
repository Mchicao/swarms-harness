# Agent preflight

SWARMS identifica los agentes disponibles antes de ejecutar un plan.

## Primer comando en un PC nuevo

```powershell
# SWARMS-PREFLIGHT-001: Inventario read-only de CLIs y rutas configuradas.
python scripts/swarm.py preflight --format json
```

El informe separa `ready`, `unverified`, `missing_cli`, `missing_auth` y
`disabled`. La presencia de una CLI o credencial no se presenta como prueba de
que una llamada al modelo funcionará. También lista CLIs detectadas aunque no
tengan una ruta configurada.

`doctor` imprime el inventario como su primera salida. `run` ejecuta el mismo
preflight antes de crear el directorio del run, claims o workers. Si el plan
usa una ruta real no verificada, termina con un diagnóstico y no despacha.

Después de hacer un probe externo autorizado, el bypass explícito es:

```powershell
# SWARMS-PREFLIGHT-002: Sólo tras validar manualmente autenticación y cuota.
python scripts/swarm.py run --plan path\plan.json --allow-unverified-agents
```

El coordinador Rust expone el mismo inventario con:

```powershell
# SWARMS-PREFLIGHT-003: Inventario del coordinador público Rust.
cargo run --manifest-path rust/Cargo.toml -- preflight
```
