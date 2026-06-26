# SWARMS Singularity V2 - Extended Loop
# Usage: ./scripts/start_singularity_v2.ps1 [-MaxCycles 10]

param(
    [int]$MaxCycles = 10
)

$ErrorActionPreference = "Stop"

Write-Host "============================================" -ForegroundColor Magenta
Write-Host "      SINGULARITY V2 - PLANNER & WORKER LOOP " -ForegroundColor Magenta
Write-Host "============================================" -ForegroundColor Magenta

# 0. PRE-FLIGHT CHECK
Write-Host "[LOOP] Checking public SWARMS entrypoint..." -ForegroundColor Yellow
python scripts/swarm.py doctor

$cycle = 1

while ($cycle -le $MaxCycles) {
    if (Test-Path "STOP_SINGULARITY") {
        Write-Host "[LOOP] Stop file detected. Halting." -ForegroundColor Red
        break
    }

    Write-Host ""
    Write-Host ">>> CICLO $cycle / $MaxCycles <<<" -ForegroundColor Cyan
    Write-Host "--------------------------------"
    
    # 1. ARCHITECT PHASE
    Write-Host "[LOOP] Running architect phase..." -ForegroundColor Yellow
    python scripts/architect.py
    
    if (-not (Test-Path ".agent/tasks_singularity.md")) {
        Write-Host "[LOOP] El Arquitecto no generó tareas. Finalizando bucle." -ForegroundColor Green
        break
    }

    # 2. ROUTER PHASE
    Write-Host "[LOOP] Routing tasks with current provider policy..." -ForegroundColor Yellow
    python scripts/smart_router.py --task-file ".agent/tasks_singularity.md" --format text

    # 3. BUILDER PHASE
    Write-Host "[LOOP] Launching worker swarm..." -ForegroundColor Yellow
    pwsh scripts/parallel_swarm.ps1 -TaskFile ".agent/tasks_singularity.md" -ProviderStrategy "auto" -WorkerCount 4

    # 4. SUMMARIZER PHASE
    Write-Host "[LOOP] Summarizing cycle state..." -ForegroundColor Yellow
    python scripts/summarizer.py

    Write-Host "[LOOP] Ciclo $cycle Completado." -ForegroundColor Green
    $cycle++
    
    Write-Host "[LOOP] Esperando 30 segundos (Respetando Rate Limits)..." -ForegroundColor DarkGray
    Start-Sleep -Seconds 30
}

Write-Host "============================================" -ForegroundColor Magenta
Write-Host "      SINGULARITY COMPLETE                 " -ForegroundColor Magenta
Write-Host "============================================" -ForegroundColor Magenta
