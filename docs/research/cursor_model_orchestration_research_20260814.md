# Cursor: modelo caro para planificar o modelo barato que escala

Fecha de corte: 2026-08-14.

## Respuesta corta

Cursor no publicó una comparación directa entre estas dos políticas:

1. un modelo caro planifica y modelos baratos implementan;
2. un modelo barato implementa y pide ayuda a un modelo caro cuando lo considera necesario.

La evidencia más cercana favorece la primera política para trabajos grandes. En el experimento SQLite de Cursor, un modelo *frontier* planificó y Composer 2.5 ejecutó. Esa mezcla alcanzó una calidad final similar a las configuraciones caras y redujo mucho el costo. Cursor no probó allí una política de escalamiento iniciada por el worker. [Cursor, “Agent swarms and the new model economics”](https://cursor.com/blog/agent-swarm-model-economics)

Para tráfico variado, Cursor obtuvo buenos resultados con una tercera política: el *harness* clasifica cada turno y decide si usa el modelo económico o uno *frontier*. El modelo barato no decide la escalada. [Cursor, “How Cursor Router chooses the right model for the task”](https://cursor.com/blog/how-cursor-router-works)

Mi conclusión para SWARMS es:

- Use un planner fuerte para tareas largas, ambiguas o arquitectónicas.
- Use workers económicos para subtareas estrechas con criterios verificables.
- Permita que el worker solicite ayuda, pero deje la decisión al runtime.
- Escale por señales observables: pruebas fallidas, ambigüedad, reintentos, riesgo y costo restante.

Si hay que elegir una de las dos opciones sin construir un router, conviene **modelo caro planificando y barato implementando**. La alternativa “barato pide ayuda” necesita un juez o política externa para ser confiable.

## 1. Evidencia directa de Cursor sobre planner caro y worker barato

Cursor publicó el 20 de julio de 2026 un experimento de swarm sobre una tarea larga. El sistema debía implementar en Rust el manual completo de SQLite, de 835 páginas. Cursor ocultó el código fuente, las pruebas, el binario de SQLite e internet. Midió el resultado con `sqllogictest`, que contiene millones de consultas con respuestas conocidas. También revisó cada ejecución para detectar atajos. [Metodología del experimento](https://cursor.com/blog/agent-swarm-model-economics#the-sqlite-experiment)

Cursor comparó cuatro configuraciones:

1. GPT-5.5 como planner y worker.
2. Grok 4.5 como planner y worker.
3. Opus 4.8 como planner y Composer 2.5 como worker.
4. Fable 5 como planner y Composer 2.5 como worker.

Todas las configuraciones del harness nuevo llegaron al 100% de la suite. Al corte de cuatro horas, estaban entre 73% y 85%. Cursor advierte que no ejecutó la matriz completa de combinaciones y que el objetivo principal era comparar versiones del harness. [Configuraciones y resultados](https://cursor.com/blog/agent-swarm-model-economics#results-across-model-mixes)

El costo varió mucho con una calidad final similar. La mezcla Opus 4.8 más Composer 2.5 costó USD 1.339. GPT-5.5 para ambos roles costó USD 10.565. Los workers GPT-5.5 costaron USD 9.373; los workers Composer 2.5 del híbrido costaron USD 411. Los workers consumieron al menos 69% de los tokens y más de 90% en la mayoría de las ejecuciones. [Economía por rol](https://cursor.com/blog/agent-swarm-model-economics#model-economics)

Cursor atribuye el resultado a una separación estricta. Los planners toman decisiones, descomponen el objetivo y delegan. Los workers ejecutan piezas estrechas. El planner no llena su contexto con detalles de implementación, y cada worker evita cargar el objetivo completo. [Árbol de tareas y contexto](https://cursor.com/blog/agent-swarm-model-economics#trees-and-leaves)

### Qué demuestra

- Un planner *frontier* puede comprimir la ambigüedad antes de gastar la mayoría de los tokens.
- Los workers baratos pueden mantener la calidad cuando reciben tareas estrechas y explícitas.
- El modelo que usa más tokens debe ser económico para controlar el costo total.

### Qué no demuestra

- Cursor evaluó una sola tarea monumental, no una muestra diversa de repositorios.
- Cursor no informa repeticiones o intervalos de confianza para cada mezcla.
- Cursor no publicó el harness ni las ejecuciones híbridas completas para reproducirlas.
- El experimento no compara contra un worker barato que decide cuándo escalar.
- Cursor afirma que la matriz completa planner-worker queda pendiente.

Por eso, los números respaldan una dirección. No establecen una ley general ni un ganador universal.

## 2. Evidencia directa de Cursor sobre routing y escalamiento

Cursor Router sigue una política distinta a “el modelo barato pide ayuda”. El harness usa señales del turno actual y del estado reciente. Primero, `Compass` estima la complejidad. Después, un clasificador selecciona un modelo *frontier* según dominio, tarea y modificadores. [Arquitectura de Cursor Router](https://cursor.com/blog/how-cursor-router-works#a-data-driven-approach-to-routing)

El sistema mantiene el turno en un modelo económico cuando la puntuación queda bajo el umbral. Si supera el umbral, el router elige un modelo *frontier*. Un candidato solo entra cuando su mejora observada supera un umbral unilateral del 75% frente al modelo económico. Después, un optimizador elige la mezcla con mayor ganancia esperada dentro del presupuesto. [Algoritmo de routing](https://cursor.com/blog/how-cursor-router-works#combining-into-an-algorithm)

Cursor entrenó el router con cientos de miles de turnos de tráfico real. Cada muestra contiene señales de conversación, costo y una señal de rendimiento inferida de la siguiente acción del usuario. Cursor ajustó los umbrales con validación cruzada, evaluó en un conjunto reservado y después ejecutó pruebas en tráfico real. [Dataset](https://cursor.com/blog/how-cursor-router-works#building-a-dataset) y [evaluación](https://cursor.com/blog/how-cursor-router-works#evaluating-performance-in-production)

Al 6 de agosto de 2026, Cursor reportó estos resultados internos:

- Auto Intelligence superó la satisfacción de Fable con 68% menos costo.
- Auto Balance superó la satisfacción de Opus 4.8 con 41% menos costo.
- Las solicitudes con mayor probabilidad de éxito según Compass obtuvieron señal positiva en 96% de los casos. Las de menor probabilidad obtuvieron 71%.

Cursor calcula el costo real por turno e incluye fallos de caché causados por cambios de modelo. [Resultados y predicción de Compass](https://cursor.com/blog/how-cursor-router-works)

### Límite de esta evidencia

Cursor no publicó el dataset, el clasificador, las asignaciones ni intervalos de confianza. La satisfacción inferida y la permanencia del código son métricas internas. Un tercero no puede reproducir el resultado. Además, el router decide antes de ejecutar el modelo. No evalúa si un worker barato sabe reconocer su propio límite.

## 3. Qué permite Cursor hoy

### Planificar con un modelo y construir con otro

Cursor 2.0 permite crear un plan con un modelo y construirlo con otro. También permite producir varios planes en paralelo. El anuncio no incluye una evaluación de combinaciones de modelos. [Cursor 2.0, “Plan Mode in Background”](https://cursor.com/changelog/2-0)

Plan Mode investiga el repositorio, pregunta por requisitos y crea un plan editable antes de escribir código. Cursor recomienda usarlo para cambios complejos, tareas que afectan varios sistemas y decisiones arquitectónicas. [Documentación de Plan Mode](https://cursor.com/docs/agent/plan-mode)

### Dar un modelo propio a cada subagente

Los subagentes aceptan `model: inherit` o un identificador específico. Cursor documenta ejemplos de planner y agente de razonamiento con modelos *frontier*. El agente padre puede delegar según complejidad, alcance, descripción del subagente, contexto y herramientas. [Configuración y delegación de subagentes](https://cursor.com/docs/subagents)

Esto permite construir un padre barato con un subagente caro. La documentación no demuestra que el padre barato detecte con fiabilidad cuándo necesita ayuda. Las restricciones del equipo o del plan también pueden sustituir el modelo configurado. [Condiciones de fallback de subagentes](https://cursor.com/docs/subagents#when-the-configured-model-wont-be-used)

### Cambiar de modelo durante una conversación

Cursor permite el cambio, pero advierte dos costos. El modelo nuevo recibe un historial producido por otro harness y pierde la caché específica del proveedor. Cursor recomienda mantener un modelo durante una conversación y usar un subagente con contexto nuevo cuando se necesite otro. [Cursor, “Facilitating mid-chat model switching”](https://cursor.com/blog/continually-improving-agent-harness#facilitating-mid-chat-model-switching)

### Usar Cursor Router

Cursor Router está disponible en Teams y Enterprise. Clasifica cada solicitud y no permite elegir el modelo subyacente para un turno. El usuario solo controla el objetivo `Cost`, `Balance` o `Intelligence`, además de listas permitidas en Enterprise. [Documentación de Cursor Router](https://cursor.com/docs/cursor-router)

## 4. Evidencia reproducible fuera de Cursor

Esta evidencia no mide Cursor ni tareas de programación equivalentes. Sirve para comprobar si las dos intuiciones aparecen en experimentos controlados.

### PEAR: la calidad del planner pesa más que la del executor

PEAR evaluó 84 tareas de banca, Slack, viajes y archivos. Probó pares planner-executor de cuatro familias y repitió cada experimento cinco veces. Usó funciones deterministas de utilidad y herramientas sintéticas. [PEAR, metodología oficial en ACL Anthology](https://aclanthology.org/2026.findings-eacl.237.pdf)

Un planner Gemini 2.0 Flash obtuvo cerca de 30% de utilidad incluso con Gemini 2.5 Pro como executor. Un planner Gemini 2.5 Pro con Gemini 2.0 Flash como executor quedó cerca de 50%. Los autores concluyen que un planner débil limita más el sistema y que un executor fuerte no compensa ese daño. [PEAR, sección 4.3](https://aclanthology.org/2026.findings-eacl.237.pdf)

El artículo declara disponibles el código y los datos. Sus tareas no son cambios reales de software, por lo que la transferencia a Cursor o SWARMS necesita validación propia. [Página oficial de PEAR](https://aclanthology.org/2026.findings-eacl.237/)

### AutoMix: un modelo barato puede escalar, pero necesita verificación externa

AutoMix pide primero una respuesta al modelo pequeño. El mismo modelo produce una autoevaluación, y un meta-verificador decide si dirige la consulta al modelo grande. En cinco modelos y cinco datasets de comprensión y preguntas, los autores reportaron más de 50% de reducción de costo con rendimiento comparable. [Artículo de AutoMix](https://arxiv.org/abs/2310.12963)

Los autores publicaron código, entradas, salidas y un comando para reproducir sus resultados. [Repositorio oficial de AutoMix](https://github.com/automix-llm/automix)

AutoMix no deja la decisión final al modelo barato. Un POMDP o un umbral corrige la autoevaluación ruidosa. Tampoco estudia edición de repositorios, herramientas persistentes o conversaciones largas. Por eso, respalda el escalamiento controlado por el harness, no la confianza ciega en que el worker pedirá ayuda a tiempo.

## 5. Decisión recomendada para SWARMS

### Tareas largas y ambiguas

Use esta secuencia:

`planner fuerte -> tareas estrechas -> workers baratos -> verificación local -> escalamiento controlado`

La evidencia de Cursor SQLite y PEAR respalda concentrar la capacidad en planificación. El costo se controla porque los workers consumen la mayoría de los tokens.

### Tareas pequeñas y tráfico mixto

No pague un planner caro para cada solicitud. Use un clasificador determinista o aprendido antes del despacho. Cursor Router muestra que esa política puede mejorar la relación costo-calidad en producción, aunque sus resultados no se pueden reproducir fuera de Cursor.

### Escalamiento desde un worker

Permita que el worker emita `needs_help` con una causa estructurada. El runtime debe decidir la escalada. Use criterios como:

- prueba o verificación fallida;
- dos intentos sin progreso;
- decisión arquitectónica fuera del alcance delegado;
- conflicto entre requisitos;
- cambio con alto riesgo o baja reversibilidad.

El worker no debe cambiar de modelo dentro de su contexto. Inicie un subagente caro con una especificación corta, evidencia del fallo y estado verificable. Esto conserva la separación de roles y evita cargar un historial incompatible.

## Veredicto

La evidencia disponible favorece **caro para planificar y barato para implementar** en trabajos grandes. Para trabajos variados, favorece **routing del harness antes de ejecutar**. Cursor no demuestra que un modelo barato, por sí solo, sepa cuándo pedir ayuda a uno caro.

La arquitectura recomendada combina ambos hallazgos: planner fuerte cuando la complejidad lo exige, workers baratos y escalamiento autorizado por el runtime después de señales objetivas.

## Fuentes primarias

- [Cursor: Agent swarms and the new model economics](https://cursor.com/blog/agent-swarm-model-economics)
- [Cursor: How Cursor Router chooses the right model for the task](https://cursor.com/blog/how-cursor-router-works)
- [Cursor: Cursor Router documentation](https://cursor.com/docs/cursor-router)
- [Cursor: Subagents documentation](https://cursor.com/docs/subagents)
- [Cursor: Plan Mode documentation](https://cursor.com/docs/agent/plan-mode)
- [Cursor: version 2.0 changelog](https://cursor.com/changelog/2-0)
- [Cursor: Continually improving our agent harness](https://cursor.com/blog/continually-improving-agent-harness)
- [PEAR, ACL Anthology](https://aclanthology.org/2026.findings-eacl.237/)
- [AutoMix paper](https://arxiv.org/abs/2310.12963)
- [AutoMix source and replication assets](https://github.com/automix-llm/automix)

## Alcance de esta investigación

No ejecuté Cursor, modelos externos ni APIs de pago. Revisé documentación, publicaciones de Cursor y fuentes académicas primarias disponibles hasta la fecha de corte.
