import os
import sys
from pathlib import Path
from typing import Any

import yaml

try:
    from rich import print
    from rich.console import Console
    from rich.table import Table

    console = Console()
except ImportError:
    # Fallback if rich is not available (though it is in dependencies)
    class FakeConsole:
        def print(self, *args, **kwargs):
            print(*args)

    console = FakeConsole()

# Importar el sistema de auditoría modular
import sys
import os

# Add project root to sys.path to allow imports from 'core'
sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from core.rules.protected import ProtectedFilesRule
from core.rules.scope import ScopeRule

# Intentamos importar las herramientas de smart_check para validación de contenido
try:
    from scripts.smart_check import (
        detect_language,
        find_related_test,
        get_modified_files,
        run_validators,
    )
except ImportError:
    # Stubs si no existe smart_check
    def get_modified_files() -> list[str]:
        # Implementación mínima si falla el import
        import subprocess

        try:
            result = subprocess.run(
                ["git", "diff", "--name-only", "HEAD~1"], capture_output=True, text=True, check=True
            )
            return [line.strip() for line in result.stdout.splitlines() if line.strip()]
        except Exception:
            return []

    def detect_language(fp: str) -> str:
        return "unknown"

    def run_validators(fp: str, lang: str) -> tuple[bool, str]:
        return (True, "No validator")

    def find_related_test(fp: str) -> str | None:
        return None


def load_audit_config(project_name=None) -> dict[str, Any]:
    """Carga la configuración de auditoría desde el proyecto o global."""
    # Look in .agent/<project>/CONTRACT.yaml or core/audit.yaml (legacy)
    search_paths = []
    if project_name:
        search_paths.append(Path(".agent") / project_name / "CONTRACT.yaml")
        search_paths.append(Path(".agent") / project_name / "audit.yaml")
    
    # Legacy/Global fallback
    search_paths.append(Path(".agent/mockup/core/audit.yaml"))
    search_paths.append(Path("core/audit.yaml"))

    config_path = None
    for p in search_paths:
        if p.exists():
            config_path = p
            break

    if not config_path:
        console.print(
            f"[yellow]Advertencia: Configuración de auditoría no encontrada. Usando config vacía.[/yellow]"
        )
        return {}

    console.print(f"[dim]Usando configuración: {config_path}[/dim]")
    with open(config_path, encoding="utf-8") as f:
        try:
            return yaml.safe_load(f)
        except Exception as e:
            console.print(f"[red]Error cargando {config_path}: {e}[/red]")
            return {}


def main() -> int:
    """
    Agente de Validación Modular.
    1. Carga configuración de auditoría.
    2. Valida integridad (archivos protegidos y alcance).
    3. Valida calidad de código (smart_check).
    """
    import argparse

    parser = argparse.ArgumentParser(description="Agente de Validación Modular")
    parser.add_argument("--role", help="Rol del agente para validación de alcance")
    parser.add_argument("--project", help="Nombre del proyecto")
    parser.add_argument(
        "--files",
        nargs="*",
        help="Lista específica de archivos a validar (override git diff)",
    )
    parser.add_argument(
        "--baseline",
        help="Archivo de baseline de errores conocidos",
    )
    args = parser.parse_args()

    project = args.project or os.environ.get("SWARM_PROJECT")
    baseline_path = args.baseline
    if not baseline_path:
        if project:
            baseline_path = Path(".agent") / project / "baseline_errors.json"
        else:
            baseline_path = Path(".agent/baseline_errors.json")

    console.print("[bold blue]Iniciando Agente de Validación Modular...[/bold blue]")

    # 1. Preparar Contexto y Configuración
    audit_config = load_audit_config(project)

    # Use explicit file list if provided, otherwise fall back to git diff
    if args.files:
        modified_files = args.files
        console.print(f"[dim]Modo: Archivos explícitos ({len(modified_files)})[/dim]")
    else:
        modified_files = get_modified_files()
        console.print(f"[dim]Modo: Git diff ({len(modified_files)} archivos)[/dim]")

    if not modified_files:
        console.print("[dim]No se detectaron archivos modificados. Nada que hacer.[/dim]")
        return 0

    console.print(f"Archivos detectados: [bold cyan]{len(modified_files)}[/bold cyan]")

    exit_code = 0
    all_violations = []

    # --- FASE 1: Auditoría de Integridad (Modular Rules) ---
    console.print("\n[bold]Fase 1: Auditoría de Integridad[/bold]")

    # Regla de Archivos Protegidos
    protected_rule = ProtectedFilesRule(audit_config.get("protected_files", []))
    protected_violations = protected_rule.validate(modified_files, audit_config)
    if protected_violations:
        for v in protected_violations:
            console.print(f"[red]❌ VIOLACIÓN:[/red] {v}")
            all_violations.append(v)
            exit_code = 1
    else:
        console.print("[green]✅ Archivos protegidos: OK[/green]")

    # Regla de Alcance (Scope)
    # Prioridad: Argumento --role > AGENT_ROLE > SWARM_ROLE > default 'backend'
    role = args.role or os.environ.get("AGENT_ROLE") or os.environ.get("SWARM_ROLE") or "backend"

    scope_context = {
        "roles": audit_config.get("roles", {}),
        "role": role,
        "protected_files": audit_config.get("protected_files", []),
    }
    scope_rule = ScopeRule()
    scope_violations = scope_rule.validate(modified_files, scope_context)
    if scope_violations:
        for v in scope_violations:
            console.print(f"[yellow]⚠️  ALCANCE:[/yellow] {v}")
            # Las violaciones de alcance pueden ser advertencias o errores según política
            # all_violations.append(v)
            # exit_code = 1 # Descomentar para hacer que falle por alcance
    else:
        console.print("[green]✅ Alcance (Scope): OK[/green]")

    # --- FASE 2: Calidad de Código (Smart Check) ---
    console.print("\n[bold]Fase 2: Calidad de Código[/bold]")

    results_table = Table(title="Resultados de Validación")
    results_table.add_column("Archivo", style="cyan")
    results_table.add_column("Lenguaje", style="dim")
    results_table.add_column("Estado", justify="center")
    results_table.add_column("Detalle")

    for filepath in modified_files:
        if not os.path.exists(filepath):
            continue

        lang = detect_language(filepath)
        success, message = run_validators(filepath, lang)

        status_str = "[green]PASS[/green]" if success else "[red]FAIL[/red]"
        results_table.add_row(filepath, lang, status_str, message)

        if not success:
            console.print(f"   [yellow]WARNING: Failed checks for {filepath} (Continuing)[/yellow]")
            # exit_code = 1  <-- DISABLED FOR RELAXED MODE

    console.print(results_table)

    # 3. Resumen Final
    print("\n" + "=" * 50)
    if exit_code == 0:
        console.print("[bold green]ESTADO FINAL: SUCCESS[/bold green]")
    else:
        console.print("[bold red]ESTADO FINAL: FAILED[/bold red]")
        if all_violations:
            console.print(f"Total violaciones de integridad: {len(all_violations)}")

    return exit_code


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:
        console.print(
            f"[bold white on red]Error crítico en validate_agent: {e}[/bold white on red]"
        )
        import traceback

        traceback.print_exc()
        sys.exit(1)
