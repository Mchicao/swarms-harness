# Informe Adversarial de Integración SAP PI, Contratos y Concurrencia
**Agente Auditor:** GLM 5.3 Max (Ruta: `glm53` / OpenCode)  
**Sistema Evaluado:** Navegación Oncológica - Clínica Alemana de Osorno  
**Archivos Auditados:** `main.py`, `state_engine.py`, `models.py` (Outbox, Inbound, ColaAsignacion)  
**Fecha:** 2026-09-09  

---

## 1. Hallazgos en Inbound y Cola de Asignación (RN-004, RN-005)

### 🔴 SAP-INT-01: Condición de Carrera en Desempate de Casos Activos
- **Hallazgo:** Cuando ingresa un evento de SAP PI para un paciente con múltiples casos oncológicos activos (ej. Cáncer de Mama sincrónico con Cáncer Tiroideo), la consulta `db.query(CasoOncologico).filter(CasoOncologico.rut == ...)` se evalúa sin un `SELECT FOR UPDATE` ni bloqueo a nivel de fila.
- **Riesgo:** Si dos webhooks de SAP PI llegan en milisegundos idénticos para el mismo paciente, ambos eventos pueden derivarse a casos distintos o duplicar la entrada a la `ColaAsignacion`.
- **Remediación:** En PostgreSQL 16, usar transacciones serializables o bloqueo pesimista `with_for_update()` sobre el registro del paciente/caso.

### 🟡 SAP-INT-02: Manejo de Caídas y Reintentos en `SapOutbox`
- **Hallazgo:** El sistema persiste eventos salientes en `SapOutbox` con estado `PENDIENTE`. Sin embargo, el endpoint `/api/sap/procesar-outbox` carece de backoff exponencial automático y corte por `Circuit Breaker`.
- **Riesgo:** Ante una indisponibilidad prolongada de SAP PI/PO en la clínica, el Outbox acumulará miles de registros, generando una tormenta de peticiones HTTP al reanudar el enlace.
- **Remediación:** Implementar reintentos con jitter y límite de 5 intentos antes de marcar `FALLIDO_PERMANENTE` con notificación a monitoreo Dynatrace.

### 🟢 SAP-INT-03: Idempotencia en Inbound
- **Fortaleza:** La validación de unicidad sobre `codigo_prestacion_sap` y payload hash evita la duplicación de facturaciones clínicas ya procesadas.
