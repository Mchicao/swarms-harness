# Arquitectura Hospitalaria de Producción de Misión Crítica
**Agente Estratégico:** GPT-6-Astra-Medium (Ruta: `codex_astra` / SWARMS)  
**Institución:** Clínica Alemana de Osorno (CAO)  
**Sistema:** Plataforma de Navegación Oncológica y Gestión GES  
**Fecha:** 2026-09-09  

---

## 1. Visión Ejecutiva y Objetivos de Resiliencia
Para soportar la operación oncológica continua del Centro del Cáncer de CAO con cero tolerancia a pérdidas de datos clínicos y alta demanda de auditoría legal:
- **RTO (Recovery Time Objective):** < 5 minutos.
- **RPO (Recovery Point Objective):** 0 (cero pérdida de transacciones clínicas).
- **Cumplimiento Normativo:** Ley 21.719 (Protección de Datos Personales en Salud), Estándares MINSAL y Acreditación de Calidad en Salud.

---

## 2. Topología de Red y Seguridad Perimetral

### A. Integración con SAP PI / PO Hospitalario
1. **Túnel VPN IPSec Dedicado + mTLS:**
   - La comunicación bidireccional entre la app de Navegación Oncológica y SAP Process Integration (PI/PO) se establece sobre una DMZ hospitalaria privada con mutual TLS (mTLS) y certificados X.509 firmados por la PKI interna de CAO.
2. **Patrón Transaccional Outbox con Reintentos Exponenciales:**
   - Para mitigar caídas de SAP PI, todos los eventos salientes se persisten en una tabla particionada `sap_outbox`. Un worker asíncrono con backoff exponencial, jitter y circuit-breaker drena la cola garantizando entrega *at-least-once*.

### B. Identidad Federada y Control de Acceso (SSO Institucional)
1. **Federación OIDC / OAuth2 con Microsoft Entra ID:**
   - Reemplazo de credenciales locales por autenticación federada institucional de Clínica Alemana de Osorno vía Microsoft Entra ID (M365).
2. **RBAC y MFA Mandatorio:**
   - Roles hospitalarios estrictos: `ENFERMERA_NAVEGADORA`, `ENFERMERA_GES`, `MEDICO_TRATANTE`, `ADMINISTRADOR`.
   - Se exige MFA (Autenticación Multifactor) a través de Microsoft Authenticator para cualquier acceso a la red hospitalaria o remoto.
   - Eliminación total de endpoints con fallback sin autenticar identificados en la auditoría adversarial.

---

## 3. Plan de Migración de Base de Datos: SQLite a PostgreSQL 16 HA

### A. Estrategia de Almacenamiento y Concurrencia
- **PostgreSQL 16 en Clúster Activo-Pasivo (Patroni + etcd):**
  - Dos nodos PostgreSQL con replicación síncrona en streaming para garantizar RPO = 0.
  - Proxy de conexión vía HAProxy / PgBouncer con pool transaccional.
- **Particionamiento por Rango de Fechas y Rut:**
  - Particionamiento declarativo anual en tablas de alta volumetría: `auditoria_logs`, `eventos_clinicos`, `sap_outbox`.
- **Bloqueo Pesimista en Desempate de Eventos:**
  - Uso de `SELECT ... FOR UPDATE` en la asignación de prestaciones de SAP a casos oncológicos activos para eliminar condiciones de carrera concurrentes.

---

## 4. Gobernanza de Datos y Cumplimiento Ley 21.719 (Chile)
1. **Cifrado en Reposo y en Tránsito:**
   - TLS 1.3 mandatorio para toda conexión HTTP.
   - Cifrado transparente de datos (TDE) o LUKS en los volúmenes de disco de la base de datos.
2. **Enmascaramiento y Pseudoanonimización de RUT:**
   - El RUT de los pacientes se presenta enmascarado en la UI (`12.345.xxx-x`).
   - La acción de visualización de RUT completo dispara un evento de auditoría irrevocable (`AuditoriaLog`) con registro de IP, usuario y propósito asistencial.
