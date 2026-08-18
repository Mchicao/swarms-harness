# Estrategias modernas de routing de modelos para SWARMS

**Ventana investigada:** 14 de junio a 14 de agosto de 2026, inclusive.  
**Fecha de corte:** 14 de agosto de 2026.  
**Pregunta:** que estrategias recientes superan el routing estatico por rol y como aplicarlas a modelos y agentes modernos sin degradar calidad.

## Resumen ejecutivo

La evidencia reciente no favorece una eleccion binaria entre "modelo caro que planifica y barato que implementa" y "modelo barato que pide ayuda al caro". Favorece un hibrido con la autoridad de routing en el harness:

1. Un modelo fuerte resuelve la ambiguedad inicial y crea trabajos acotados.
2. Modelos eficientes ejecutan la mayor parte del volumen.
3. El harness decide escalamiento con riesgo, historial, tests, repeticion de errores, presupuesto y estado de cache. El worker puede pedir ayuda, pero esa peticion es una senal, no el unico gate.
4. La verificacion determinista va primero; el review por modelo se usa selectivamente y, cuando importa, con una familia distinta a la del implementador.
5. Antes de cambiar de modelo, conviene ajustar el nivel de razonamiento dentro del mismo modelo. Cambiar de familia puede perder cache, razonamiento preservado y compatibilidad de herramientas.

Para SWARMS, la mejor siguiente etapa no es entrenar inmediatamente un clasificador. Es implementar primero una politica determinista, observable y cache-aware en los limites de tareas/subagentes; registrar decisiones y resultados; y entrenar o calibrar un router aprendido solo cuando exista suficiente telemetria local.

## Metodo y limites

- Se usaron fuentes primarias: publicaciones oficiales de Cursor, Factory, OpenAI, Anthropic y Google; ACL Anthology; arXiv; documentacion y codigo local de SWARMS.
- Se exigio publicacion o actualizacion verificable dentro de la ventana. Los trabajos anteriores se mencionan solo como fundamentos, no como novedades.
- No se ejecutaron modelos, benchmarks ni APIs pagadas. Las cifras externas no son reproducciones propias.
- Las cifras de Cursor y Factory son mediciones internas del vendor. Son valiosas por su escala de produccion, pero no incluyen datasets o trazas completas para reproduccion independiente.
- Los papers de julio/agosto son evidencia de estrategias recientes; varios evaluan pools de modelos anteriores o tareas no identicas al coding agent de SWARMS.

## Las estrategias que si cambiaron el panorama

### 1. Clasificador de complejidad seguido por routing de capacidades

Cursor no usa un modelo barato para que decida libremente si necesita ayuda. Su router clasifica cada turno antes de ejecutar el modelo. La version publicada el 6 de agosto separa dos decisiones:

- **Compass** estima si un turno es lo bastante simple para el modelo eficiente.
- Si no lo es, una taxonomia de dominio, tarea y modificadores elige entre modelos frontier segun rendimiento observado.

Compass asigna un score continuo y un umbral mueve el sistema por la frontera costo-calidad. Cursor declara que los turnos con mayor probabilidad predicha de exito obtuvieron senal positiva 96% de las veces y los de menor probabilidad, 71%. Un candidato frontier solo entra si su mejora estimada supera un umbral unilateral de 75%; luego un optimizador selecciona la mezcla que maximiza ganancia dentro del presupuesto. La evaluacion combina cross-validation, conjunto retenido y A/B en trafico real, incluyendo cache misses. [Cursor Router, 22-jul-2026](https://cursor.com/blog/router); [como funciona, 6-ago-2026](https://cursor.com/blog/how-cursor-router-works).

**Lectura para SWARMS:** empezar con features explicitas (`role`, `task_kind`, `risk`, `verify`, tamano de cambio, intentos previos) y un umbral auditable. No comenzar con un prompt opaco que "adivine" el modelo.

### 2. Routing cache-aware con stickiness e histeresis

Factory sostiene que el selector de modelo debe vivir donde se construyen system prompt, herramientas, contexto, reasoning settings y subagentes: el harness. Su articulo del 14 de agosto reporta 58% de ahorro agregado frente a usar frontier en todas las llamadas, mediana de 76%, y latencia mediana por turno de 81 a 49 segundos, manteniendo sus ocho medidas internas de resultado. Tambien muestra el costo de ignorar cache: el costo modelado totalmente uncached llega a 2,12 veces el baseline all-frontier en turnos 61-150 y 2,37 veces en 151-200; su routing cache-aware queda entre 0,19 y 0,28 veces el baseline. [Abhay Singhal/Factory, 14-ago-2026](https://x.com/_AbhaySinghal/status/2088361241928732705).

Cursor llega a la misma conclusion desde otra implementacion: entrena y evalua considerando el costo de cache misses, y senala que el routing real decide tanto que modelo elegir como cuando cambiar durante una conversacion. [Cursor Router](https://cursor.com/blog/router).

MTRouter aporta evidencia academica multi-turn: aprende utilidad por turno desde embeddings conjuntos de historial-modelo; en sus evaluaciones cambia menos de modelo, tolera mejor errores transitorios y reduce costo 58,7% en ScienceWorld y 43,4% en HLE respecto de GPT-5. [MTRouter, ACL 2026](https://aclanthology.org/2026.acl-long.2045/).

**Lectura para SWARMS:** `stay` debe ser una decision de routing de primera clase. Cambiar solo cuando la ganancia esperada supere costo de cold context, incompatibilidad de adapter y riesgo de perder afinidad de sesion. Usar histeresis para no oscilar por una falla aislada.

### 3. Planner fuerte, workers eficientes y reviewer independiente

En el experimento SQLite de Cursor, planners inteligentes descomponen un arbol de tareas y workers mas rapidos/baratos ejecutan las hojas. Los workers consumieron al menos 69% de los tokens y mas de 90% en la mayoria de las corridas. La mezcla Opus 4.8 planner + Composer 2.5 workers costo USD 1.339; GPT-5.5 en ambos roles, USD 10.565. Cursor advierte que todas las mezclas terminaron con calidad similar en ese experimento, pero no hizo una matriz N por N completa. [Cursor, 20-jul-2026](https://cursor.com/blog/agent-swarm-model-economics).

Factory describe el mismo patron con un padre fuerte que crea especificaciones breves para workers eficientes y conserva su propia cache caliente. El reviewer es un trabajo separado y su validator por defecto pertenece a otra familia que el implementador, buscando errores no correlacionados. En Missions, el vendor reporta mediana de 423 turnos, diez sesiones y unas doce horas, con 37,8% de ahorro frente a tarifar todo como frontier. [Factory, 14-ago-2026](https://x.com/_AbhaySinghal/status/2088361241928732705).

**Lectura para SWARMS:** esta debe ser la politica base. El modelo fuerte se gasta en desambiguar, partir, definir interfaces y criterios de termino; el modelo eficiente produce volumen. Un implementador barato no debe ser el unico responsable de detectar que no entiende el problema.

### 4. Gate barato sobre resultados y escalamiento de workflow

LLM-as-Scheduler ejecuta primero un agente, aplica un gate ligero a su salida y solo entonces usa un scheduler LLM para decidir verificacion o reparacion adicional. Frente a un workflow fuerte fijo, reporta 43% menos tokens, mas de 36% menos latencia y como maximo 1,4 puntos porcentuales de perdida de accuracy. [LLM-as-Scheduler, ACL 2026](https://aclanthology.org/2026.acl-long.581/).

Factory usa senales todavia mas cercanas al coding real: fallos de comandos/tests, ediciones repetidas, artefactos entregados y aceptacion o reinicio por el usuario. Un resultado fallido puede cambiar la siguiente eleccion dentro de la sesion; resultados finales refinan la politica futura. [Factory, 14-ago-2026](https://x.com/_AbhaySinghal/status/2088361241928732705).

**Lectura para SWARMS:** el gate principal debe ser determinista cuando exista: `verify` fallido, mismo parche repetido, artefacto ausente, timeout, review de seguridad o intento agotado. La autoevaluacion del worker complementa esas pruebas, no las reemplaza.

### 5. Routing como loop Contexto -> Accion -> Feedback

Agent-as-a-Router critica el clasificador one-shot por deficit de informacion y propone un loop C-A-F con Orchestrator, Verifier y Memory. Solo agregar estadisticas de rendimiento por dimension a un router base produjo una mejora relativa de 15,3%; CodeRouterBench contiene unas 10.000 tareas verificadas sobre ocho LLMs. [Agent-as-a-Router, 22-jun-2026](https://arxiv.org/abs/2606.22902).

EvoRoute selecciona backbones Pareto-optimos por paso desde una base de experiencias y actualiza la politica con feedback del entorno; reporta hasta 80% menos costo y mas de 70% menos latencia en GAIA y BrowseComp+. [EvoRoute, ACL 2026](https://aclanthology.org/2026.acl-long.1771/).

**Lectura para SWARMS:** cada decision debe persistir `policy_version`, features, candidatos, eleccion, razon, costo esperado, estado de cache y outcome. Sin ese registro no existe flywheel, solo heuristicas imposibles de auditar.

### 6. Routing por thinking/effort antes de cambiar de modelo

Los modelos modernos exponen una segunda dimension de routing: profundidad de razonamiento.

- Anthropic presenta Sonnet 5 con curvas costo-rendimiento por `effort`: `medium` mejora eficiencia y niveles altos pueden alcanzar Opus 4.8 en algunas tareas. [Anthropic, 30-jun-2026](https://www.anthropic.com/news/claude-sonnet-5).
- OpenAI separa la seleccion de tier (`gpt-5.6-sol`, `terra` o `luna`) de `reasoning.effort` (`none` a `max`). Recomienda comparar configuraciones en tareas representativas, reservar `max` para trabajo quality-first y activar `reasoning.mode: "pro"` solo cuando la ganancia marginal justifique mayor latencia y tokens. GPT-5.6 tambien puede preservar razonamiento entre turnos y usar cache explicita, por lo que cambiar de familia no es gratuito. [OpenAI, guia GPT-5.6](https://developers.openai.com/api/docs/guides/latest-model).
- Anthropic extiende el mismo eje a Sonnet 5 y Opus 5: `low` sirve para tareas acotadas/subagentes, `medium` para balance y `xhigh|max` para trabajo agentico largo o de maxima dificultad. `effort` tambien afecta la cantidad de tool calls. [Anthropic, control de effort vigente al corte](https://platform.claude.com/docs/en/build-with-claude/effort).
- Google documenta para Gemini 3.6 Flash niveles `minimal`, `low`, `medium` y `high`, con pensamiento dinamico y `medium` por defecto. Tambien exige preservar las thought signatures para mantener correctamente el contexto de razonamiento multi-turn cuando se gestiona el historial. [Google, Gemini thinking, actualizado el 21-jul-2026](https://ai.google.dev/gemini-api/docs/generate-content/thinking).

**Lectura para SWARMS:** para una familia ya caliente, subir `thinking` puede ser mas barato y seguro que saltar a otra familia. El runtime ya tiene `thinking` por plan/tarea y afinidad de sesion; la politica debe poder modificar el esfuerzo sin falsificar soporte ni niveles del adapter.

### 7. Presupuesto global, no presupuesto fijo por request

Cursor ofrece modos Cost, Balance e Intelligence como puntos distintos sobre una frontera Pareto. Su optimizador asigna el presupuesto promedio a la mezcla de trafico, no obliga a que cada turno cueste lo mismo. [Cursor, 6-ago-2026](https://cursor.com/blog/how-cursor-router-works).

WISERouter formaliza la misma intuicion como contextual multi-armed bandit con restriccion de presupuesto del workload. Aprende offline desde interacciones historicas u online con exploracion y reporta mejor adherencia al presupuesto que baselines en RouterBench y SWE-Bench. [WISERouter, 26-jul-2026](https://arxiv.org/abs/2607.23765).

**Lectura para SWARMS:** usar un envelope por run/stage y permitir que tareas faciles financien escalamiento en la cola dificil. Mantener hard caps por ruta, proveedor y run; el score nunca debe poder sobrepasarlos.

### 8. Capacidades intrinsecas e historial, no palabras superficiales

DecoR descompone cada consulta en requisitos de capacidad y recupera casos similares del historial para evitar que el router memorice la redaccion superficial; su evaluacion incluye generalizacion OOD. [DecoR, ACL 2026](https://aclanthology.org/2026.acl-long.1852/).

LLMRouter modela routing como decision secuencial con cinco piezas separables: encoder de contexto, encoder de modelo, score, regla de decision y learning signal. En xRouteBench, los routers aprendidos superaron relativamente 14,6% al mejor modelo fijo y los routers ligeros fueron mas competitivos bajo presupuesto estricto. [LLMRouter, 7-ago-2026](https://arxiv.org/abs/2608.06867).

**Lectura para SWARMS:** mantener `task_kind`, `capabilities_required`, `risk` y `completion_conditions` como campos estructurados. Los nombres de rol solos (`programmer`, `verifier`) son demasiado gruesos para aprender especializacion.

### 9. Gates de seguridad y defensa contra routing caro adversarial

Conformal LLM Routing calibra un gate logistico para limitar la tasa de violacion del modelo barato por debajo de una tolerancia `alpha` con confianza `1-delta`. En GSM8K/MMLU mantuvo el objetivo en su sweep, mientras un umbral ajustado solo en validation lo cruzo en GSM8K. [Conformal LLM Routing, ACL 2026](https://aclanthology.org/2026.acl-srw.70/).

Route-to-Rome muestra el riesgo inverso: sufijos adversariales pueden empujar routers black-box hacia modelos caros. [Route-to-Rome, ACL 2026](https://aclanthology.org/2026.acl-long.2051/).

**Lectura para SWARMS:** la ruta premium nunca debe depender solo del texto del usuario. Debe exigir politica, presupuesto y, cuando corresponda, aprobacion. Registrar y alertar subidas anormales de `expensive_route_rate` por tenant, repo o clase de tarea.

### 10. Provider failover separado de model escalation

Factory distingue dos decisiones:

- el **harness** elige el modelo porque conoce job, herramientas, cache y outcomes;
- el **gateway** aplica modelos permitidos, salud/capacidad y failover de proveedor.

Su pagina de producto describe failover al mismo modelo por un provider sano y declara 99,9%+ de confiabilidad de requests. Esa cifra es del vendor y no demuestra mejora de calidad, pero la separacion arquitectonica es correcta. [Factory Router](https://factory.ai/product/router); [anuncio del 1-jun-2026, anterior a la ventana](https://factory.ai/news/factory-router).

**Lectura para SWARMS:** primero intentar otra ruta del mismo `canonical_model` ante rate limit o indisponibilidad; cambiar de modelo solo como una decision semantica explicita del harness. Nunca confundir disponibilidad con dificultad.

## Estrategias que no conviene adoptar como default

### Portfolio/race + judge en todo

Ejecutar varios modelos y pedir a un juez que elija puede aumentar cobertura, pero multiplica costo y hereda la fragilidad del juez. CodeJudgeBench encuentra sensibilidad a orden, nombres y comentarios; el resultado aconseja tests deterministas y juez independiente antes que un unico LLM evaluator. [CodeJudgeBench, ACL 2026](https://aclanthology.org/2026.acl-long.888/).

Usarlo solo en decisiones de alto riesgo, planes irreversibles o bugs sin oracle local. No en tareas rutinarias ni como sustituto de tests.

### Pools cada vez mas grandes

LLMRouterBench reevalua 400.000 instancias, 21 datasets, 33 modelos y diez routers. Confirma complementariedad, pero varios routers recientes —incluidos comerciales— no superan consistentemente un baseline simple; pools grandes muestran retornos decrecientes frente a una curacion cuidadosa. [LLMRouterBench, ACL Findings 2026](https://aclanthology.org/2026.findings-acl.1881/).

Para SWARMS, un pool corto y diverso es preferible: eficiente generalista, frontier generalista y, como maximo, uno o dos especialistas medidos.

### Switching por cada turno

Es incompatible con la evidencia de cache, thought preservation y tooling por familia. El selector debe operar en limites naturales: inicio de task, creacion de subagente, compaction, fallo verificado, milestone/review o perdida de salud del provider.

## Comparacion de las dos hipotesis originales

| Estrategia | Donde gana | Falla principal | Veredicto |
| --- | --- | --- | --- |
| Modelo fuerte planifica; barato implementa | Ambiguedad alta, specs divisibles, workers dominan tokens | Un plan incorrecto propaga errores; algunas hojas siguen siendo dificiles | **Mejor baseline** |
| Modelo barato implementa y pide ayuda al caro | Tareas faciles, discovery barato, problemas con senales claras | El modelo barato puede no reconocer su error o pedir ayuda demasiado tarde | **Escape hatch, no control principal** |
| Harness hibrido | Usa rol, riesgo, outcomes, cache, budget y salud | Requiere telemetria y politica explicita | **Objetivo recomendado** |

La regla operacional propuesta es: **planner fuerte para colapsar ambiguedad; worker eficiente para producir volumen; harness para escalar; verifier independiente para cerrar**.

## Encaje con el SWARMS actual

SWARMS ya tiene varias piezas correctas:

- politica estatica por roles y premium bloqueado por defecto en @config/role_policy.json;
- `fallback_routes`, `canonical_model`, clases de costo y quota guard en @config/swarm_router.json;
- metadatos descriptivos `strengths`/`weaknesses` en @config/swarm_router.json, aunque hoy el `Provider` tipado de @rust/src/model.rs no los consume para decidir rutas;
- `thinking` por plan/tarea y afinidad de sesion en @rust/src/model.rs, @rust/src/adapter.rs y @rust/src/session.rs;
- scheduler determinista, reintentos, limites por provider y outcomes persistidos en @rust/src/runtime.rs;
- tests/verificaciones como gates de finalizacion.

La brecha es que la ruta efectiva sigue viniendo principalmente de la tarea/alias y los fallbacks responden a capacidad/cuota, no a una politica dinamica que use riesgo, progreso, outcomes o costo de switching.

## Politica recomendada para SWARMS

### P0 — Router determinista y explicable

Agregar una decision de routing en el limite de cada task, sin invocar otro LLM:

**Entradas:** rol, `task_kind`, riesgo, dependencias, completion conditions, comandos `verify`, intentos, ultimo outcome, artefactos, route/model actual, afinidad de sesion, salud/cuota, costo acumulado y presupuesto restante.

**Salida persistida:** route/model, thinking, `stay|switch|spawn`, razon, policy version, confianza, costo/cache estimados y candidatos descartados.

Orden de decision:

1. Aplicar allowlist, premium approval y hard caps.
2. Si falla el provider, hacer failover al mismo `canonical_model` cuando exista.
3. Si hay sesion caliente y progreso, permanecer.
4. Si la tarea es mecanica y acotada, usar modelo eficiente con thinking bajo/medio.
5. Si hay ambiguedad arquitectonica, riesgo alto o ausencia de oracle, usar planner fuerte.
6. Si fallan tests, se repite la misma edicion, se agotan intentos o el verifier rechaza, escalar modelo o thinking.
7. Tras plan/diagnostico, crear subagente fresco con spec breve para bajar de modelo sin mover todo el historial.

### P1 — Tres modos Pareto y presupuesto por run

- **Economy:** maximo uso de eficiente, premium solo por gates duros.
- **Balanced:** planner fuerte selectivo, workers eficientes, reviewer diferente en hitos.
- **Quality:** frontier en plan/diagnostico y review de alto riesgo; eficiente en trabajo mecanico.

Cada modo define umbrales, no nombres fijos. Los modelos cambian mas rapido que la politica.

### P1 — Telemetria minima para aprender

Medir por clase de tarea y policy version:

- tasa de tareas verificadas y aceptadas;
- costo por tarea exitosa, no solo costo por request;
- latencia a primer resultado y a resultado verificado;
- reintentos, rollback/rework y cambios de modelo;
- cache/session reuse y costo estimado de cold switch;
- precision del escalamiento: cuantos premium calls resolvieron un fallo real;
- tasa de falsos baratos: tareas que debieron escalar luego.

### P2 — Shadow routing y calibracion

Antes de habilitar cambios automaticos, correr la politica en shadow sobre runs normales y comparar su eleccion con el resultado real. Luego A/B contra tres baselines:

1. un solo modelo fuerte;
2. roles estaticos actuales;
3. reglas deterministas cache-aware.

Entrenar un classifier, bandit o retrieval router solo si supera al baseline simple en tareas retenidas, costo por exito y calidad, no solo en accuracy offline.

### P3 — Portfolio selectivo

Habilitar `race+judge` unicamente para seguridad, arquitectura dificil, migraciones irreversibles o diagnosticos sin tests. Exigir presupuesto explicito, tests locales y reviewer independiente.

## Experimento minimo recomendado

Sin gastar APIs durante la implementacion:

1. Construir 30-50 fixtures historicos/sinteticos de metadatos de tareas y outcomes, sin contenido sensible.
2. Implementar el router de reglas como funcion pura y probarlo con `mock`.
3. Registrar decisiones shadow en el reporte de run.
4. Definir criterios de promocion: ningun bypass de premium approval, presupuesto nunca excedido, cero oscilacion de ruta, explicacion completa y mejora prospectiva de costo por exito.
5. Solo despues pedir autorizacion para un benchmark pequeno con modelos reales y matriz parcial que incluya planner, worker y verifier.

## Conclusion

La mejora moderna no consiste en encontrar "el modelo barato suficientemente listo para llamar al caro". Consiste en mover la metacognicion al sistema: el harness conoce el trabajo, la cache, las pruebas, el presupuesto y el resultado. El modelo fuerte debe concentrarse donde reduce ambiguedad o corrige un bloqueo; el modelo eficiente debe llevar el volumen; y el cambio debe ocurrir en fronteras de trabajo que preserven contexto y hagan el outcome atribuible.

En SWARMS eso aprovecha la arquitectura actual: Rust conserva scheduling, locks, quotas, session affinity, verification, telemetry y reports; los modelos solo ejecutan roles autorizados. La siguiente inversion con mayor retorno es un router determinista, outcome-aware y cache-aware, no un meta-modelo adicional.

## Fuentes recientes principales

- [Cursor Router: lanzamiento](https://cursor.com/blog/router) — 22-jul-2026.
- [How Cursor Router chooses the right model](https://cursor.com/blog/how-cursor-router-works) — 6-ago-2026.
- [Agent swarms and the new model economics](https://cursor.com/blog/agent-swarm-model-economics) — 20-jul-2026.
- [Why model routing must be in the harness](https://x.com/_AbhaySinghal/status/2088361241928732705) — 14-ago-2026.
- [OpenAI: guia de modelos GPT-5.6](https://developers.openai.com/api/docs/guides/latest-model) — documentacion vigente al corte.
- [Claude Sonnet 5](https://www.anthropic.com/news/claude-sonnet-5) — 30-jun-2026.
- [Anthropic: control de effort](https://platform.claude.com/docs/en/build-with-claude/effort) — documentacion vigente al corte.
- [Google: Gemini thinking](https://ai.google.dev/gemini-api/docs/generate-content/thinking) — actualizado el 21-jul-2026.
- [Agent-as-a-Router](https://arxiv.org/abs/2606.22902) — 22-jun-2026.
- [LLM-as-Scheduler](https://aclanthology.org/2026.acl-long.581/) — ACL, jul-2026.
- [MTRouter](https://aclanthology.org/2026.acl-long.2045/) — ACL, jul-2026.
- [EvoRoute](https://aclanthology.org/2026.acl-long.1771/) — ACL, jul-2026.
- [DecoR](https://aclanthology.org/2026.acl-long.1852/) — ACL, jul-2026.
- [Conformal LLM Routing](https://aclanthology.org/2026.acl-srw.70/) — ACL, jul-2026.
- [WISERouter](https://arxiv.org/abs/2607.23765) — 26-jul-2026.
- [LLMRouter](https://arxiv.org/abs/2608.06867) — 7-ago-2026.
- [LLMRouterBench](https://aclanthology.org/2026.findings-acl.1881/) — ACL Findings, jul-2026.
- [Route-to-Rome](https://aclanthology.org/2026.acl-long.2051/) — ACL, jul-2026.

## Fundamentos anteriores a la ventana

- [Factory Router](https://factory.ai/news/factory-router), 1-jun-2026: benchmark inicial, escalamiento y provider failover. Se usa solo para contexto historico.
- AutoMix, RouteLLM y PEAR siguen siendo antecedentes utiles sobre cascadas, routing costo-calidad y fragilidad planner/worker, pero no se presentan aqui como evidencia nueva de los ultimos dos meses.
