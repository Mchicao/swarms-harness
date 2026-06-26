"""
Swarm Metrics Recorder - Registra resultados de tareas para análisis de rendimiento.

Uso:
    python scripts/record_metric.py --model gemini --task "Crear componente" --difficulty medium --result success --task-type frontend --language python
    python scripts/record_metric.py --model glm --task "Migrar página" --difficulty high --result failure --error-type syntax_error --task-type backend
    python scripts/record_metric.py --summary
"""

import argparse
import json
from datetime import datetime
from pathlib import Path

METRICS_FILE = Path(".agent/swarm_metrics.json")

TASK_TYPES = ["backend", "frontend", "qa", "docs", "general"]
LANGUAGES = ["python", "javascript", "typescript", "powershell", "markdown", "yaml", "other"]
DIFFICULTIES = ["low", "medium", "high"]
ERROR_TYPES = ["rate_limit", "syntax_error", "logic_error", "timeout", "scope_violation", "other"]


def get_metrics_file(project_name=None) -> Path:
    """Retorna la ruta del archivo de métricas."""
    if project_name:
        return Path(".agent") / project_name / "swarm_metrics.json"
    return Path(".agent/swarm_metrics.json")


def load_metrics(metrics_file: Path) -> dict:
    """Carga el archivo de métricas existente o crea uno nuevo."""
    if metrics_file.exists():
        with open(metrics_file, encoding="utf-8") as f:
            data = json.load(f)
            return migrate_metrics(data)
    return create_empty_metrics()


def migrate_metrics(data: dict) -> dict:
    """Migra métricas de versiones anteriores al formato actual."""
    if data.get("version") == "1.1":
        return data

    # Ensure new fields exist
    for model in ["gemini", "glm"]:
        if model not in data.get("by_model", {}):
            data["by_model"][model] = create_model_stats()
        else:
            m = data["by_model"][model]
            if "by_task_type" not in m:
                m["by_task_type"] = {t: {"success": 0, "failures": 0} for t in TASK_TYPES}
            if "by_language" not in m:
                m["by_language"] = {lang: {"success": 0, "failures": 0} for lang in LANGUAGES}

    data["version"] = "1.1"
    return data


def create_empty_metrics() -> dict:
    """Crea estructura de métricas vacía."""
    return {
        "version": "1.1",
        "project": Path.cwd().name,
        "last_updated": datetime.now().isoformat(),
        "summary": {
            "total_tasks": 0,
            "total_success": 0,
            "total_failures": 0,
            "success_rate": 0.0,
        },
        "by_model": {
            "gemini": create_model_stats(),
            "glm": create_model_stats(),
        },
        "task_history": [],
    }


def create_model_stats() -> dict:
    """Crea estadísticas vacías para un modelo."""
    return {
        "total": 0,
        "success": 0,
        "failures": 0,
        "success_rate": 0.0,
        "by_difficulty": {d: {"success": 0, "failures": 0} for d in DIFFICULTIES},
        "by_task_type": {t: {"success": 0, "failures": 0} for t in TASK_TYPES},
        "by_language": {lang: {"success": 0, "failures": 0} for lang in LANGUAGES},
        "by_error_type": {e: 0 for e in ERROR_TYPES},
    }


def record_task(
    model: str,
    task_name: str,
    difficulty: str,
    result: str,
    task_type: str = "general",
    language: str = "other",
    error_type: str | None = None,
    duration_seconds: float | None = None,
    project_name: str | None = None,
) -> None:
    metrics_file = get_metrics_file(project_name)
    metrics = load_metrics(metrics_file)

    # Ensure directory exists
    metrics_file.parent.mkdir(parents=True, exist_ok=True)

    # Normalizar modelo
    model_key = "glm" if model.lower() in ("glm", "zai", "aider") else "gemini"

    # Normalizar task_type y language
    task_type = task_type.lower() if task_type.lower() in TASK_TYPES else "general"
    language = language.lower() if language.lower() in LANGUAGES else "other"

    # Actualizar contadores
    metrics["summary"]["total_tasks"] += 1
    metrics["by_model"][model_key]["total"] += 1

    if result == "success":
        metrics["summary"]["total_success"] += 1
        metrics["by_model"][model_key]["success"] += 1
        metrics["by_model"][model_key]["by_difficulty"][difficulty]["success"] += 1
        metrics["by_model"][model_key]["by_task_type"][task_type]["success"] += 1
        metrics["by_model"][model_key]["by_language"][language]["success"] += 1
    else:
        metrics["summary"]["total_failures"] += 1
        metrics["by_model"][model_key]["failures"] += 1
        metrics["by_model"][model_key]["by_difficulty"][difficulty]["failures"] += 1
        metrics["by_model"][model_key]["by_task_type"][task_type]["failures"] += 1
        metrics["by_model"][model_key]["by_language"][language]["failures"] += 1

        # Clasificar tipo de error
        if error_type and error_type in metrics["by_model"][model_key]["by_error_type"]:
            metrics["by_model"][model_key]["by_error_type"][error_type] += 1
        else:
            metrics["by_model"][model_key]["by_error_type"]["other"] += 1

    # Calcular tasas de éxito
    for m in ["gemini", "glm"]:
        total = metrics["by_model"][m]["total"]
        if total > 0:
            metrics["by_model"][m]["success_rate"] = round(
                metrics["by_model"][m]["success"] / total * 100, 1
            )

    total = metrics["summary"]["total_tasks"]
    if total > 0:
        metrics["summary"]["success_rate"] = round(
            metrics["summary"]["total_success"] / total * 100, 1
        )

    # Registrar en historial
    metrics["task_history"].append(
        {
            "timestamp": datetime.now().isoformat(),
            "model": model_key,
            "task": task_name[:100],
            "difficulty": difficulty,
            "task_type": task_type,
            "language": language,
            "result": result,
            "error_type": error_type,
            "duration_seconds": duration_seconds,
        }
    )

    # Limitar historial a últimas 500 tareas
    if len(metrics["task_history"]) > 500:
        metrics["task_history"] = metrics["task_history"][-500:]

    metrics["last_updated"] = datetime.now().isoformat()

    # Guardar
    with open(metrics_file, "w", encoding="utf-8") as f:
        json.dump(metrics, f, indent=2, ensure_ascii=False)

    print(f"✅ {model_key} | {result} | {task_type} | {language} | {task_name[:40]}...")


def print_summary(project_name=None) -> None:
    """Imprime resumen de métricas."""
    metrics_file = get_metrics_file(project_name)
    metrics = load_metrics(metrics_file)

    print(f"\n📊 RESUMEN DE MÉTRICAS (Proyecto: {project_name or 'Global'})")
    print("=" * 60)
    print(f"Total tareas: {metrics['summary']['total_tasks']}")
    print(f"Éxitos: {metrics['summary']['total_success']}")
    print(f"Fallos: {metrics['summary']['total_failures']}")
    print(f"Tasa de éxito global: {metrics['summary']['success_rate']}%")

    print("\n🤖 POR MODELO:")
    for model in ["gemini", "glm"]:
        m = metrics["by_model"][model]
        print(f"\n  {model.upper()}:")
        print(f"    Total: {m['total']} | Éxitos: {m['success']} | Fallos: {m['failures']}")
        print(f"    Tasa de éxito: {m['success_rate']}%")

        # Por dificultad
        print("    📈 Por dificultad:")
        for diff in DIFFICULTIES:
            d = m["by_difficulty"][diff]
            total_diff = d["success"] + d["failures"]
            if total_diff > 0:
                rate = round(d["success"] / total_diff * 100, 1)
                print(f"      {diff}: {d['success']}/{total_diff} ({rate}%)")

        # Por tipo de tarea
        print("    🏷️  Por tipo de tarea:")
        for tt in TASK_TYPES:
            t = m["by_task_type"][tt]
            total_tt = t["success"] + t["failures"]
            if total_tt > 0:
                rate = round(t["success"] / total_tt * 100, 1)
                print(f"      {tt}: {t['success']}/{total_tt} ({rate}%)")

        # Por lenguaje
        print("    💻 Por lenguaje:")
        for lang in LANGUAGES:
            lang_stats = m["by_language"][lang]
            total_lang = lang_stats["success"] + lang_stats["failures"]
            if total_lang > 0:
                rate = round(lang_stats["success"] / total_lang * 100, 1)
                print(f"      {lang}: {lang_stats['success']}/{total_lang} ({rate}%)")

        # Errores
        errors = m["by_error_type"]
        total_errors = sum(errors.values())
        if total_errors > 0:
            print("    ❌ Errores:")
            for err_type, count in errors.items():
                if count > 0:
                    pct = round(count / total_errors * 100, 1)
                    print(f"      {err_type}: {count} ({pct}%)")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Registrar métricas del Swarm")
    parser.add_argument("--model", choices=["gemini", "glm", "zai"], help="Modelo usado")
    parser.add_argument("--task", type=str, help="Nombre de la tarea")
    parser.add_argument("--difficulty", choices=DIFFICULTIES, help="Dificultad de la tarea")
    parser.add_argument("--result", choices=["success", "failure"], help="Resultado")
    parser.add_argument("--task-type", choices=TASK_TYPES, default="general", help="Tipo de tarea")
    parser.add_argument("--language", choices=LANGUAGES, default="other", help="Lenguaje principal")
    parser.add_argument("--error-type", choices=ERROR_TYPES, help="Tipo de error (si falló)")
    parser.add_argument("--duration", type=float, help="Duración en segundos")
    parser.add_argument("--summary", action="store_true", help="Mostrar resumen")
    parser.add_argument("--project", help="Nombre del proyecto")

    args = parser.parse_args()

    if args.summary:
        print_summary(args.project)
    elif args.model and args.task and args.difficulty and args.result:
        record_task(
            model=args.model,
            task_name=args.task,
            difficulty=args.difficulty,
            result=args.result,
            task_type=args.task_type,
            language=args.language,
            error_type=args.error_type,
            duration_seconds=args.duration,
            project_name=args.project,
        )
    else:
        parser.print_help()
