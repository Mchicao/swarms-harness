# Routing de modelos para agentes: de la clasificación inicial al escalamiento temporal

**Fecha de corte:** 15 de agosto de 2026.  
**Alcance:** ampliación de @docs/research/cursor_model_orchestration_research_20260814.md y @docs/research/model_routing_strategies_modern_20260614_20260814.md.  
**Pregunta:** qué política debería usar SWARMS para decidir modelo, esfuerzo, contexto y verificación durante una tarea, especialmente cuando un worker económico descubre que el trabajo era más difícil de lo esperado.

## Veredicto

El hallazgo nuevo más importante es **temporal**: para routing agentic no basta con clasificar el prompt. Conviene dejar que un worker económico explore durante unos pocos pasos de bajo riesgo y hacer que el harness decida, usando la trayectoria observada, si continúa o escala. Ese diseño combina las dos intuiciones originales, pero no entrega la autoridad al worker:

`gate inicial -> exploración económica acotada -> gate externo -> continuar o escalar -> verificación`

[SWE-Router](https://arxiv.org/abs/2607.00053) presenta la evidencia más directa para ingeniería de software. Su router observa una trayectoria parcial —estructura de archivos, búsquedas, tests, errores y progreso— y supera a routers que sólo ven el prompt. En su evaluación, usar tres pasos exploratorios mejoró `Route-AUC` en 15,3 puntos porcentuales para el par con DeepSeek y en 12 puntos para el par con GPT-5-mini, respecto de las comparaciones declaradas por los autores. El costo de explorar se incluye en el total.

Esto modifica, pero no invalida, la recomendación anterior:

- Para una tarea larga que ya sabemos ambigua o arquitectónica, sigue siendo mejor un planner fuerte que entregue hojas estrechas a workers eficientes.
- Para tráfico mixto y dificultad desconocida, un worker barato puede ser un buen **sensor** de dificultad después de explorar.
- El worker no debe ser el único juez de su propia competencia. El runtime conserva presupuesto, seguridad y autoridad de escalamiento.
- La escalada debe abrir un contexto limpio para el modelo fuerte. Se transfiere evidencia verificable, no la especulación completa del worker débil.

## 1. Tres mecanismos distintos que suelen confundirse

### Cursor Router decide por turno

Cursor Router clasifica la solicitud antes de ejecutar el modelo. `Compass` estima complejidad y, si el turno no queda en la ruta eficiente, otro clasificador selecciona un modelo frontier según dominio, tarea y modificadores. La decisión pertenece al harness y opera sobre el turno, el estado reciente, el presupuesto y el costo de caché. No es un modelo barato trabajando hasta que espontáneamente pide ayuda. [Descripción oficial de Cursor Router](https://cursor.com/blog/how-cursor-router-works).

### El swarm de Cursor separa planner y workers

El experimento SQLite usa otra granularidad. Un planner frontier descompone un objetivo largo y workers económicos ejecutan hojas estrechas. La ruta se decide por **rol y nodo del árbol**, no re-clasificando cada turno de una misma conversación. Cursor encontró que los workers consumían la mayoría de los tokens y que una mezcla de planner caro con workers baratos podía llegar a una calidad final similar a configuraciones totalmente frontier con mucho menor costo. La matriz completa de combinaciones no fue evaluada. [Experimento de economía de swarms de Cursor](https://cursor.com/blog/agent-swarm-model-economics).

### SWE-Router decide después de observar trabajo real

SWE-Router introduce un tercer nivel. El modelo económico recibe la tarea y explora durante `K` pasos. Un value head externo lee esa trayectoria parcial y decide si el modelo económico continúa o si se reinicia el trabajo con el modelo fuerte. Esto ataca una limitación estructural del router por prompt: la descripción puede ocultar dificultad que sólo aparece al inspeccionar el repositorio o ejecutar tests. [Paper y materiales de SWE-Router](https://arxiv.org/html/2607.00053).

Los tres mecanismos son complementarios:

1. El gate inicial evita discovery barato cuando el riesgo o la complejidad ya son evidentes.
2. La separación planner/worker reduce contexto y costo en objetivos divisibles.
3. El gate temporal captura dificultad latente dentro de una hoja aparentemente simple.

### Scrouting: el handoff puede valer más que el router

[Scrouting/SuperScout](https://arxiv.org/abs/2608.04804), publicado el 5 de agosto, agrega una ablación especialmente útil. Un scout de 7B explora el repositorio, verifica en sandbox sus afirmaciones y entrega un handoff estructurado antes de elegir entre cuatro fixers. En las 266 tareas Python de SWE-bench Pro, el sistema resolvió 159 frente a 158 del mejor modelo fijo con aproximadamente una quinta parte del costo por solución. Sin embargo, **usar siempre el fixer más barato con el mismo handoff empató al router aprendido**. En ese benchmark, el scouting y la transferencia verificada explicaron el beneficio; la selección aprendida de modelo no añadió valor observable. Es un único benchmark y un preprint, pero fija una ablación obligatoria para SWARMS: comparar cualquier router contra `scout + handoff verificado + ruta barata fija`.

## 2. Por qué la trayectoria parcial cambia la decisión

Un prompt como «corrige el login» no revela si el fallo es una condición local, un contrato distribuido o una migración de seguridad. Tras tres pasos, el harness puede conocer datos mucho más discriminantes:

- cantidad y dispersión de archivos relevantes;
- profundidad de búsqueda antes de localizar el punto de cambio;
- existencia de un oracle local y resultado de tests;
- stack traces y clases de error;
- ediciones revertidas o repetidas;
- contradicciones entre requisitos y código;
- progreso verificable frente al presupuesto consumido.

SWE-Router formaliza que una observación más informativa no empeora el routing Bayes-óptimo y puede mejorarlo cuando la exploración contiene señal. En la práctica, su resultado importante no es sólo la mejora de AUROC del clasificador. Los autores muestran que el AUROC del value head no se correlaciona necesariamente con la calidad económica del router. Por eso hay que medir la frontera costo-resolución completa, no celebrar la precisión del gate de forma aislada.

El paper también observa que los modelos débil y fuerte resuelven subconjuntos no idénticos de tareas. En algunos puntos, un router puede superar brevemente a `always-strong` porque el modelo caro no domina cada caso. Esto desaconseja interpretar «escalar» como «corregir siempre con un modelo universalmente superior».

### Reinicio limpio al escalar

SWE-Router reinicia el modelo fuerte desde el prompt original. Los autores reportan que transferir el razonamiento del worker débil puede anclar al modelo fuerte en sus errores. Para SWARMS, la adaptación razonable es crear un subagente nuevo con:

- objetivo y criterios de término originales;
- archivos encontrados y símbolos relevantes;
- comandos ejecutados y salida sanitizada;
- tests que pasan y fallan;
- parche actual sólo si es necesario para reproducir el estado;
- ninguna cadena especulativa presentada como hecho.

La inclusión de un paquete neutral de evidencia es una propuesta de diseño para SWARMS, no un resultado probado directamente por SWE-Router. Debe compararse contra el reinicio sólo con prompt original.

## 3. Routing multi-eje, no sólo selección de modelo

[X-Router](https://aclanthology.org/2026.findings-acl.994/) separa dos decisiones: si la consulta necesita recuperación de evidencia y si necesita razonamiento adicional. Sus cuatro perfiles —directo, RAG, CoT y RAG+CoT— muestran que routing también significa decidir **qué cómputo y qué contexto habilitar**, no únicamente qué nombre de modelo usar. Los autores reportan hasta 86% menos tokens y 84% menos latencia en seis benchmarks de QA, con las limitaciones de transferencia propias de un dominio que no es edición agentic de código.

Para SWARMS, la acción de routing debería representarse como un vector:

`acción = {modelo, thinking, contexto/herramientas, estrategia de verificación, stay|switch|spawn}`

- **Modelo:** eficiente, frontier o especialista autorizado.
- **Thinking:** subir esfuerzo dentro de una sesión caliente puede ser más barato que cambiar de familia.
- **Contexto y herramientas:** lectura local, búsqueda, retrieval externo o herramienta especializada sólo cuando agrega señal.
- **Verificación:** tests deterministas, review económico, review cross-family o portfolio selectivo.
- **Sesión:** permanecer, cambiar de ruta o crear un subagente fresco.

La guía vigente de GPT-5.6 también trata modelo y esfuerzo como controles distintos y recomienda evaluar éxito, completitud, evidencia, tokens, latencia y costo. `reasoning.mode: "pro"` se reserva para casos donde la ganancia de calidad justifica mayor latencia y consumo. [Guía oficial de OpenAI](https://developers.openai.com/api/docs/guides/latest-model).

## 4. Gate externo y abstención del worker

Permitir que un worker emita `needs_help` es útil, pero debe considerarse una feature, no una autorización. La política externa dispone de información que el modelo no controla:

- hard caps y aprobación de rutas premium;
- cuota y salud del proveedor;
- intentos y costo acumulados en todo el run;
- resultados deterministas de tests y verificadores;
- afinidad de sesión y penalización de cold switch;
- riesgo del cambio y reversibilidad;
- historial de calibración de ese modelo para esa clase de tarea.

[Conformal LLM Routing](https://aclanthology.org/2026.acl-srw.70/) muestra una manera de calibrar un gate barato con una cota explícita sobre la tasa de violaciones. Aunque usa pares de modelos y benchmarks que no representan el stack moderno de coding agents, su principio sí es transferible: una política no debe depender de un umbral ajustado una sola vez y asumido como estable. Debe declarar una tolerancia, medir cobertura y recalibrarse.

Una política robusta combina tres fuentes:

1. **Gates duros:** riesgo, permiso premium, presupuesto, seguridad y disponibilidad.
2. **Evidencia determinista:** tests, artefactos, timeouts, diffs repetidos, intentos sin progreso.
3. **Score probabilístico:** dificultad estimada desde prompt, trayectoria y outcome histórico.

La auto-abstención del worker entra en la tercera categoría. Nunca debe poder saltarse las dos primeras.

[AgentAbstain](https://arxiv.org/abs/2607.10059) refuerza esta cautela con 263 pares controlados en 42 sandboxes: entre 17 modelos y cuatro harnesses, el mejor sistema obtuvo 59,5% de exactitud emparejada al decidir actuar o abstenerse, y 13 modelos quedaron bajo 50%. La evidencia no dice que una señal `needs_help` sea inútil; dice que no puede ser el único gate.

## 5. Riesgos que un benchmark limpio suele ocultar

### Drift

[LENS](https://aclanthology.org/2026.acl-long.1508/) estudia 192 escenarios reales de cambio de distribución con 81 modelos. Bajo cambios naturales moderados de prompts, reporta una pérdida media de rendimiento de 73%. El número no debe trasladarse mecánicamente a SWARMS, pero sí invalida una evaluación basada sólo en un split aleatorio estable.

El router debe monitorear resultados por tiempo, repositorio, clase de tarea, usuario, idioma y versión de modelo. Una actualización silenciosa del proveedor, una nueva base de código o una campaña distinta pueden romper el gate aunque su métrica agregada parezca estable.

### Ataques de costo y bypass de seguridad

[Route to Rome](https://aclanthology.org/2026.acl-long.2051/) demuestra que sufijos adversariales pueden empujar un router black-box hacia modelos costosos mediante un ensemble sustituto. La defensa no es sólo mejorar el clasificador:

- una ruta premium necesita allowlist, presupuesto y autorización independiente del texto;
- hay que medir `expensive_route_rate` por origen y detectar saltos anómalos;
- el score semántico nunca modifica hard caps;
- una ruta barata no puede omitir controles de seguridad presentes en la ruta frontier.

SWE-Router señala el riesgo inverso: enrutar tráfico sensible a modelos baratos podría eludir salvaguardas más fuertes. Seguridad debe ser un objetivo o restricción del router, no una métrica posterior.

### Privacidad

El routing temporal aumenta la superficie de telemetría porque observa trayectorias, archivos, errores y resultados. Para SWARMS, el dataset del router debe guardar features derivadas y outcomes, no diffs, prompts completos, secretos ni logs crudos por defecto. Las rutas aprendidas deberían entrenarse localmente sobre identificadores sanitizados, con retención explícita y separación por tenant/proyecto cuando corresponda.

Esta es una restricción de diseño de SWARMS, no una afirmación de que los papers citados resuelvan privacidad. Ninguna mejora de routing justifica exportar código o trazas sin autorización.

### Cold-start y feedback sesgado

Al inicio no existen suficientes outcomes locales para un router aprendido. [Agentic Routing: Harness-Native Data Flywheel](https://arxiv.org/abs/2607.11399) propone registrar estado del harness, acción, trayectoria, resultado y costo, y partir con un ranker simple antes de políticas más sofisticadas. Es un preprint reciente; su arquitectura es más útil aquí que sus afirmaciones empíricas.

Además, los logs de una política existente sólo muestran el resultado de la ruta elegida. No revelan qué habría hecho otro modelo con la misma tarea. Entrenar directamente con esos datos puede reforzar decisiones históricas y confundir selección con capacidad. La fase inicial debe usar reglas explícitas, shadow routing y una matriz pequeña donde varias rutas sí ejecuten los mismos fixtures con autorización.

## 6. Encaje exacto con el SWARMS actual

SWARMS ya contiene los controles estructurales correctos:

- @config/role_policy.json fija una política estática por rol, con rutas premium deshabilitadas salvo configuración explícita.
- @config/swarm_router.json contiene `canonical_model`, clase de costo, cuotas, fallbacks y descripciones de fortalezas/debilidades.
- @rust/src/model.rs modela proveedor, fallbacks, cuota, `thinking` y sesión, pero no convierte las fortalezas/debilidades descriptivas en una decisión semántica.
- @rust/src/runtime.rs aplica DAG, capacidad, reintentos, límites de proveedor y fallback cuando una ruta no está disponible o no tiene cuota. No re-rutea dinámicamente por dificultad, trayectoria o progreso semántico.
- @rust/src/adapter.rs sólo traduce niveles de thinking verificados para cada adapter.
- @rust/src/session.rs conserva afinidad de sesión, una base necesaria para contabilizar el costo de switching.
- La verificación local y los reportes persistidos ya permiten separar outcome de la opinión del worker.

La brecha no es el scheduler. Es una función de decisión observable entre el plan y la selección de ruta:

`RouteDecision = f(policy, task, trajectory, outcome, session, quota, budget)`

### Política incremental recomendada

**P0 — Determinista y en shadow.** Sin llamar otro LLM, producir una decisión hipotética y persistir `policy_version`, features, candidatos, acción, razón y restricciones aplicadas.

**P1 — Gate inicial.** Enviar directamente a planner/frontier sólo riesgo alto, arquitectura, requisitos conflictivos, ausencia de oracle o alcance transversal conocido. El resto comienza económico.

**P1 — Exploración temporal.** Permitir al worker económico hasta tres pasos de discovery de bajo riesgo: lectura, búsqueda, inspección y tests no mutantes. Antes de ediciones materiales, evaluar continuar, subir thinking o escalar.

**P1 — Escalada limpia.** Crear un subagente nuevo en vez de cambiar de familia dentro de una conversación caliente. Pasar objetivo y evidencia neutral; medir por separado la variante con razonamiento del worker para comprobar si realmente perjudica.

**P2 — Score aprendido.** Sólo después de acumular outcomes suficientes, entrenar un value head o ranker local. Los hard gates permanecen fuera del modelo.

**P3 — Bandit con presupuesto.** Si hay volumen y cobertura contrafactual, optimizar costo global por workload. [WISERouter](https://arxiv.org/abs/2607.23765) formaliza este problema como contextual bandit con restricción de presupuesto y reporta mejor adherencia presupuestaria en RouterBench y SWE-Bench. No es el primer paso para SWARMS.

## 7. Benchmark mínimo para SWARMS

El benchmark debe responder una pregunta operacional: **qué política entrega más tareas verificadas por unidad de costo y tiempo sin violar seguridad ni presupuesto**.

### Conjunto

- 60 a 100 fixtures sanitizados y reproducibles.
- Mezcla de cambios mecánicos, bugs localizados, cambios multiarchivo, arquitectura, seguridad y tareas sin oracle completo.
- Criterios deterministas de éxito cuando existan: tests, compilación, artefactos y reglas de alcance.
- Particiones por tiempo y por repositorio. Mantener al menos un repo y una versión de modelo fuera del entrenamiento.
- Repeticiones suficientes para no confundir varianza de sampling con routing.

### Políticas comparadas

1. `all-strong`: techo de costo y referencia de calidad, no supuesto oracle.
2. política estática actual por roles.
3. gate determinista sólo con prompt/metadatos (`K=0`).
4. gate determinista temporal con hasta tres pasos (`K=3`).
5. value head temporal, sólo cuando exista dataset suficiente.
6. portfolio `race+judge` únicamente en el estrato de alto riesgo.
7. ablación `scout + handoff verificado + ruta barata fija`, para comprobar si el router aporta algo más que discovery y transferencia.

Para cada fixture, la primera campaña autorizada debe ejecutar al menos las rutas 1 a 4. Reproducir sólo logs históricos no permite estimar el outcome contrafactual de una ruta que nunca se ejecutó.

### Métricas de promoción

- `verified_success_rate` y éxito por clase de tarea;
- costo total por éxito verificado, incluyendo exploración, reintentos, cache misses y review;
- tiempo p50/p95 hasta éxito verificado;
- `Route-AUC`: área normalizada bajo la curva costo frente a tareas resueltas, usando `all-weak` y `all-strong` como extremos de referencia;
- precisión y recall de escalamiento;
- `false-cheap-rate`: tareas enviadas barato que terminan escalando tarde o fallando;
- `false-premium-rate`: gasto frontier sin mejora atribuible;
- cantidad de switches, costo de cold switch y reúso de sesión/caché;
- tasa de violación de presupuesto y de bypass premium, ambas obligatoriamente cero;
- calibración del score por bucket y degradación bajo drift;
- tasa de rutas caras bajo perturbaciones adversariales.

### Criterios de promoción

La política temporal sólo reemplaza la estática si:

1. no reduce la tasa de éxito verificado fuera del margen predefinido;
2. mejora costo por éxito y `Route-AUC` en repositorios retenidos;
3. no genera oscilación de rutas;
4. mantiene cero bypass de autorización y cero exceso de hard caps;
5. conserva explicación reproducible para cada decisión;
6. resiste una prueba temporal posterior a cambios de modelo.

Primero debe ejecutarse todo con rutas `mock` y decisiones shadow para validar contrato, persistencia y gates. Cualquier benchmark con modelos reales o APIs pagadas requiere autorización explícita.

## 8. Decisión recomendada

La política objetivo para SWARMS no es «caro planifica» **o** «barato pide ayuda». Es una jerarquía:

1. **Gate del harness:** aplica permisos, riesgo, presupuesto y señales iniciales.
2. **Planner fuerte cuando la ambigüedad ya es visible:** crea especificaciones estrechas.
3. **Worker eficiente:** ejecuta o explora durante un horizonte corto.
4. **Gate temporal externo:** usa trayectoria, tests y progreso para continuar, subir thinking o escalar.
5. **Subagente fuerte limpio:** recibe evidencia neutral cuando la escalada está justificada.
6. **Verifier determinista primero:** review por modelo sólo donde agrega cobertura.

SWE-Router aporta la pieza que faltaba en la comparación original: el worker barato puede producir la evidencia que hace posible una buena escalada, pero no necesita ni debería tener la última palabra sobre ella. La metacognición útil vive en el harness porque allí están el presupuesto, la seguridad, la sesión, los tests y el resultado real.

## Fuentes primarias principales

- [SWE-Router: Routing in Multi-turn Agentic Software Engineering Tasks](https://arxiv.org/abs/2607.00053) — 30-jun-2026.
- [Scrouting: Cost-Aware Routing of Coding Agents by Scouting the Repository First](https://arxiv.org/abs/2608.04804) — 5-ago-2026; preprint.
- [AgentAbstain: Do LLM Agents Know When Not to Act?](https://arxiv.org/abs/2607.10059) — 11-jul-2026; preprint.
- [X-Router: Decoupling Knowledge and Reasoning for Cost-Effective LLM Inference](https://aclanthology.org/2026.findings-acl.994/) — ACL Findings 2026.
- [Agentic Routing: The Harness-Native Data Flywheel](https://arxiv.org/abs/2607.11399) — 13-jul-2026; preprint.
- [Measuring Distribution Shift in User Prompts and Its Effects on LLM Performance](https://aclanthology.org/2026.acl-long.1508/) — ACL 2026.
- [Route to Rome Attack: Directing LLM Routers to Expensive Models via Adversarial Suffix Optimization](https://aclanthology.org/2026.acl-long.2051/) — ACL 2026.
- [Conformal LLM Routing](https://aclanthology.org/2026.acl-srw.70/) — ACL 2026.
- [WISERouter](https://arxiv.org/abs/2607.23765) — 26-jul-2026; preprint.
- [LLMRouterBench](https://aclanthology.org/2026.findings-acl.1881/) — ACL Findings 2026.
- [MTRouter](https://aclanthology.org/2026.acl-long.2045/) — ACL 2026.
- [Cursor: How Cursor Router chooses the right model](https://cursor.com/blog/how-cursor-router-works) — 6-ago-2026.
- [Cursor: Agent swarms and the new model economics](https://cursor.com/blog/agent-swarm-model-economics) — 20-jul-2026.
- [OpenAI: guía de modelos GPT-5.6](https://developers.openai.com/api/docs/guides/latest-model) — documentación vigente al corte.

## Límites

- No se ejecutaron modelos, benchmarks ni APIs pagadas.
- Las cifras citadas pertenecen a sus autores; no son reproducciones propias.
- SWE-Router es un preprint de workshop y Agentic Routing/WISERouter son preprints recientes. Sus resultados justifican un benchmark local, no un despliegue automático.
- Parte de la evidencia académica usa modelos o dominios distintos de SWARMS. Se separaron hallazgos publicados de propuestas de diseño locales.
