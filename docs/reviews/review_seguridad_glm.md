# Informe Adversarial de Seguridad, Privacidad PHI y Cumplimiento Ley 21.719
**Agente Auditor:** GLM 5.3 Max (Ruta: `glm53` / OpenCode)  
**Sistema Evaluado:** Navegación Oncológica - Clínica Alemana de Osorno  
**Archivos Auditados:** `auth.py`, `main.py`, `models.py`  
**Fecha:** 2026-09-09  

---

## 1. Vectores Críticos Identificados (Severidad Alta)

### 🔴 VULN-SEC-01: Fallback permisivo de autenticación por diseño (`main.py` líneas 108-115)
- **Hallazgo:** En la función `get_current_user()`, si no se proporciona cabecera `Authorization` ni cookie `cao_session`, o si el token es inválido, el sistema retorna un usuario default: `{"email": "enfermera@clinicaalemanaosorno.cl", "rol": "ENFERMERA_NAVEGADORA", "autenticado": False}`.
- **Riesgo:** Endpoints críticos de negocio que únicamente leen `current_user.get("nombre")` sin verificar explícitamente `current_user.get("autenticado") == True` permiten ejecución no autorizada y bypass total de autenticación.
- **Remediación Inmediata:** Modificar `get_current_user()` para lanzar de forma estricta `HTTPException(status_code=401, detail="Credenciales requeridas")` ante la ausencia o invalidez del token JWT.

### 🔴 VULN-SEC-02: Secreto JWT estático en código (`auth.py` línea 12)
- **Hallazgo:** Se utiliza un valor por defecto en texto plano `"cao-oncologia-secret-key-2026-secure"` si `JWT_SECRET` no está en las variables de entorno.
- **Riesgo:** Si un contenedor o nodo en producción se despliega sin inyectar la variable, un atacante puede firmar tokens arbitrarios con rol de Administrador o Director Médico.
- **Remediación:** Lanzar excepción fatal (`RuntimeError`) al iniciar la aplicación si `JWT_SECRET` no está configurado en el entorno de producción.

### 🟡 VULN-SEC-03: Exposición de PHI y Ley 21.719 en respuestas sin anonimización
- **Hallazgo:** Endpoints de consultas (`/api/casos`, `/api/alertas`, `/api/cola-asignacion`) devuelven el RUT completo, nombres y diagnósticos oncológicos a cualquier cliente autenticado con rol general.
- **Riesgo:** Infracción a los principios de proporcionalidad y finalidad de la Ley 21.719 sobre Protección de Datos Personales en Chile.
- **Remediación:** Implementar enmascaramiento de RUT (ej. `12.345.xxx-x`) para vistas operativas y requerir elevación de privilegios / registro estricto en `AuditoriaLog` para desenmascarar el RUT completo.

---

## 2. Robustez Criptográfica y Cookies
- **Bcrypt:** Implementación correcta con `passlib.context.CryptContext(schemes=["bcrypt"])`.
- **Cookies:** Las cookies se configuran con `HttpOnly=True` y `SameSite="lax"`. En producción sobre HTTPS hospitalario se debe forzar `Secure=True` de forma mandatoria.
