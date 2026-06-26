# Swarm V10 Singularity V2 - The Infinite Loop
# Usage: ./start_singularity_v2.ps1 [-MaxCycles 10]

param(
    [int]$MaxCycles = 10
)

$ErrorActionPreference = "Stop"

Write-Host "============================================" -ForegroundColor Magenta
Write-Host "   ♾️  SINGULARITY V2 - OPUS & GEMINI LOOP   " -ForegroundColor Magenta
Write-Host "============================================" -ForegroundColor Magenta

# 0. PRE-FLIGHT CHECK
Write-Host "[LOOP] Verificando dependencias críticas..." -ForegroundColor Yellow
python scripts/utils/ensure_dependencies.py

$cycle = 1

while ($cycle -le $MaxCycles) {
    if (Test-Path "STOP_SINGULARITY") {
        Write-Host "[LOOP] Stop file detected. Halting." -ForegroundColor Red
        break
    }

    Write-Host ""
    Write-Host ">>> CICLO $cycle / $MaxCycles <<<" -ForegroundColor Cyan
    Write-Host "--------------------------------"
    
    # 1. ARCHITECT PHASE (Opus)
    Write-Host "[LOOP] Convocando al Arquitecto (Opus)..." -ForegroundColor Yellow
    python scripts/agents/architect_opus.py
    
    if (-not (Test-Path ".agent/tasks_singularity.md")) {
        Write-Host "[LOOP] El Arquitecto no generó tareas. Finalizando bucle." -ForegroundColor Green
        break
    }

    # 2. ROUTER PHASE (Métricas)
    Write-Host "[LOOP] Enrutando tareas inteligentemente..." -ForegroundColor Yellow
    python scripts/agents/smart_router.py

    # 3. BUILDER PHASE (Swarm)
    Write-Host "[LOOP] Lanzando Swarm de Constructores..." -ForegroundColor Yellow
    # Aseguramos que parallel_swarm use el archivo correcto
    pwsh scripts/agents/parallel_swarm.ps1 -TaskFile ".agent/tasks_singularity.md" -ModelProvider "hybrid" -WorkerCount 4

    # 4. SUMMARIZER PHASE (Flash)
    Write-Host "[LOOP] Generando resumen para el siguiente ciclo..." -ForegroundColor Yellow
    python scripts/agents/summarizer.py

    Write-Host "[LOOP] Ciclo $cycle Completado." -ForegroundColor Green
    $cycle++
    
    Write-Host "[LOOP] Esperando 30 segundos (Respetando Rate Limits)..." -ForegroundColor DarkGray
    Start-Sleep -Seconds 30
}

Write-Host "============================================" -ForegroundColor Magenta
Write-Host "   ♾️  SINGULARITY COMPLETO                 " -ForegroundColor Magenta
Write-Host "============================================" -ForegroundColor Magenta
