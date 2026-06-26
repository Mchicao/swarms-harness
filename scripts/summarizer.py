import json
import os
from pathlib import Path


def summarize_logs():
    worker_dir = Path(".agent/worker_scripts")
    summary_file = Path(".agent/cycle_summary.md")
    metrics_file = Path(".agent/swarm_metrics.json")

    if not worker_dir.exists():
        return "No worker logs found."

    outputs = list(worker_dir.glob("output_*.txt"))
    if not outputs:
        return "No outputs recorded in this cycle."

    summary = ["# 🏁 Resumen del Ciclo Anterior\n"]

    # Éxitos y Fallos rápidos
    successes = []
    failures = []

    for log_path in outputs:
        worker_id = log_path.stem.replace("output_", "")
        status_path = worker_dir / f"status_{worker_id}.txt"

        # Leer tarea del prompt (un poco hacky pero efectivo)
        prompt_path = worker_dir / f"prompt_{worker_id}.txt"
        task_name = "Tarea desconocida"
        if prompt_path.exists():
            with open(prompt_path, "r", encoding="utf-8") as f:
                content = f.read()
                if "TASK:" in content:
                    task_name = content.split("TASK:")[1].split("Target:")[0].strip()

        if status_path.exists():
            with open(status_path, "r") as f:
                exit_code = f.read().strip()
                if exit_code == "0":
                    successes.append(task_name)
                else:
                    # Extraer última línea de error del log
                    last_lines = []
                    with open(log_path, "r", encoding="utf-8", errors="replace") as f:
                        last_lines = f.readlines()[-5:]
                    error_msg = "".join(last_lines).strip()
                    failures.append(f"- **{task_name}**: {error_msg}")

    summary.append("## ✅ Éxitos")
    if successes:
        summary.extend([f"- {s}" for s in successes])
    else:
        summary.append("- Ninguno")

    summary.append("\n## ❌ Fallos")
    if failures:
        summary.extend(failures)
    else:
        summary.append("- Ninguno")

    # Añadir métricas breves
    if metrics_file.exists():
        with open(metrics_file, "r", encoding="utf-8") as f:
            metrics = json.load(f)
            stats = metrics.get("summary", {})
            summary.append(f"\n## 📊 Métricas Globales")
            summary.append(
                f"- Éxito: {stats.get('success_rate', 0)}% ({stats.get('total_success')}/{stats.get('total_tasks')})"
            )

    with open(summary_file, "w", encoding="utf-8") as f:
        f.write("\n".join(summary))

    print(f"✅ Resumen generado en {summary_file}")


if __name__ == "__main__":
    summarize_logs()
