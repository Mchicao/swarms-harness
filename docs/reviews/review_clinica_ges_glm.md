# Informe Adversarial de FSM Clínica (EST-01 a EST-13) y Garantías GES
**Agente Auditor:** GLM 5.3 Max (Ruta: `glm53` / OpenCode)  
**Sistema Evaluado:** Navegación Oncológica - Clínica Alemana de Osorno  
**Archivos Auditados:** `state_engine.py`, `ges_tracker.py`  
**Fecha:** 2026-09-09  

---

## 1. Hallazgos en Máquina de Estados (EST-01 a EST-13)

### 🔴 FSM-CLIN-01: Transiciones Ilegales desde Estados Terminales (Regla RN-013)
- **Hallazgo:** En `evaluar_transicion(estado_actual, codigo_evento)` de `state_engine.py`, no se valida si `estado_actual` es un estado terminal (`EST-09` Fallecido, `EST-10` Descarte, `EST-12` Alta Médica Definitiva) antes de procesar un nuevo evento secuencial (ej. `EVG-006` TAC o `EVG-001` Consulta).
- **Riesgo:** Un informe tardío de laboratorio o imagen clínica podría reactivar un caso cerrado de un paciente fallecido a `EST-03` (Etapificación), alterando estadísticas clínicas y auditorías forenses de mortalidad.
- **Remediación:** Bloqueo incondicional: si `estado_actual in ["EST-09", "EST-10", "EST-12"]`, rechazar cualquier transición excepto revocación expresa de descarte o auditoría médica supervisada.

### 🟡 FSM-CLIN-02: Cálculo de Plazos Legales GES sin Días Inhábiles
- **Hallazgo:** En `ges_tracker.py`, el cálculo `(now - fecha_inicio).days` computa días corridos naturales.
- **Riesgo:** Para ciertas garantías y normativas específicas del decreto GES, se requiere control exacto respecto a la fecha límite legal fijada por Fonasa/Isapre (que descuenta feriados en plazos administrativos de notificación).
- **Remediación:** Incorporar calendario de feriados nacionales chilenos en el cómputo de días hábiles administrativos.
