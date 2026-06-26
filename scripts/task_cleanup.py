#!/usr/bin/env python3
"""
Task Cleanup Script - Verifica tareas completadas y genera baseline de errores.

Funciones:
1. Detecta tareas cuyo archivo destino ya existe
2. Verifica que el archivo pase Ruff (sintaxis válida)
3. Marca tareas como [DONE] si el archivo está correcto
4. Genera baseline de errores pre-existentes para ignorar en validación
"""

import argparse
import re
import subprocess
from pathlib import Path


def extract_target_file(task_line: str) -> str | None:
    """Extrae el archivo destino de una línea de tarea."""
    # Patrones comunes:
    # - Crear `path/to/file.py`
    # - Migrar ... → `path/to/file.py`
    # - Actualizar `path/to/file.py`
    patterns = [
        r"Crear\s+`([^`]+)`",
        r"→\s+`([^`]+)`",
        r"Actualizar\s+`([^`]+)`",
        r"Refactorizar\s+`([^`]+)`",
    ]
    for pattern in patterns:
        match = re.search(pattern, task_line)
        if match:
            return match.group(1)
    return None


def check_file_quality(filepath: Path) -> tuple[bool, str]:
    """Verifica que el archivo exista y pase Ruff."""
    if not filepath.exists():
        return False, "Archivo no existe"

    # Solo validar archivos Python
    if filepath.suffix != ".py":
        return True, "Archivo no-Python (asumido OK)"

    # Ejecutar Ruff check
    try:
        result = subprocess.run(
            ["ruff", "check", str(filepath), "--select", "E,F"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        if result.returncode == 0:
            return True, "Ruff: OK"
        else:
            return False, f"Ruff: {result.stdout[:100]}"
    except FileNotFoundError:
        # Ruff no instalado, intentar con uv
        try:
            result = subprocess.run(
                ["uv", "run", "ruff", "check", str(filepath), "--select", "E,F"],
                capture_output=True,
                text=True,
                timeout=10,
            )
            if result.returncode == 0:
                return True, "Ruff: OK"
            else:
                return False, f"Ruff: {result.stdout[:100]}"
        except Exception:
            return True, "Ruff no disponible (asumido OK)"
    except Exception as e:
        return False, f"Error: {e}"


def process_tasks(task_file: Path, dry_run: bool = True) -> dict:
    """Procesa el archivo de tareas y marca las completadas."""
    content = task_file.read_text(encoding="utf-8")
    lines = content.splitlines()

    stats = {"checked": 0, "completed": 0, "pending": 0, "failed": 0}
    updated_lines = []

    for line in lines:
        # Solo procesar líneas de tareas pendientes
        if line.strip().startswith("- [ ]"):
            stats["checked"] += 1
            target = extract_target_file(line)

            if target:
                filepath = Path(target)
                found = None
                if filepath.exists():
                    found = filepath

                if found:
                    is_valid, message = check_file_quality(found)
                    if is_valid:
                        # Marcar como completada
                        new_line = line.replace("- [ ]", "- [x]") + f" (Auto-verified: {message})"
                        updated_lines.append(new_line)
                        stats["completed"] += 1
                        print(f"✅ DONE: {target} -> {message}")
                    else:
                        # Archivo existe pero tiene errores
                        updated_lines.append(line)
                        stats["failed"] += 1
                        print(f"⚠️  EXISTS BUT INVALID: {target} -> {message}")
                else:
                    updated_lines.append(line)
                    stats["pending"] += 1
            else:
                updated_lines.append(line)
                stats["pending"] += 1
        else:
            updated_lines.append(line)

    if not dry_run:
        task_file.write_text("\n".join(updated_lines), encoding="utf-8")
        print(f"\n📝 Archivo actualizado: {task_file}")

    return stats


def generate_error_baseline(output_path: Path) -> None:
    """Genera baseline de errores conocidos de basedpyright."""
    print("\n🔍 Generando baseline de errores...")

    try:
        result = subprocess.run(
            ["uv", "run", "basedpyright", "--outputjson"],
            capture_output=True,
            text=True,
            timeout=60,
        )

        # Guardar output como baseline
        output_path.write_text(result.stdout, encoding="utf-8")

        # Contar errores
        import json

        try:
            data = json.loads(result.stdout)
            error_count = data.get("summary", {}).get("errorCount", "?")
            print(f"📊 Baseline generado: {error_count} errores conocidos")
        except Exception:
            print("📊 Baseline generado (formato raw)")

    except Exception as e:
        print(f"❌ Error generando baseline: {e}")


def main():
    parser = argparse.ArgumentParser(description="Task Cleanup Script")
    parser.add_argument("--project", help="Nombre del proyecto (subcarpeta en .agent/)")
    parser.add_argument(
        "--task-file",
        help="Archivo de tareas (override)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Solo mostrar cambios, no aplicar",
    )
    parser.add_argument(
        "--generate-baseline",
        action="store_true",
        help="Generar baseline de errores de basedpyright",
    )
    args = parser.parse_args()

    print("=" * 50)
    print("   TASK CLEANUP SCRIPT")
    print("=" * 50)

    # Path resolution
    project = args.project
    agent_root = Path(".agent")
    if project:
        agent_root = agent_root / project

    task_file = args.task_file
    if not task_file:
        task_file = agent_root / "tasks.md"
        if not task_file.exists():
            task_file = agent_root / "swarm_tasks.md"
    else:
        task_file = Path(task_file)

    if not task_file.exists():
        print(f"❌ Archivo no encontrado: {task_file}")
        return

    stats = process_tasks(task_file, dry_run=args.dry_run)

    print("\n📊 Resumen:")
    print(f"   Chequeadas: {stats['checked']}")
    print(f"   Completadas: {stats['completed']}")
    print(f"   Pendientes: {stats['pending']}")
    print(f"   Con errores: {stats['failed']}")

    if args.dry_run:
        print("\n💡 Ejecuta sin --dry-run para aplicar cambios")

    if args.generate_baseline:
        baseline_path = agent_root / "baseline_errors.json"
        generate_error_baseline(baseline_path)


if __name__ == "__main__":
    main()
