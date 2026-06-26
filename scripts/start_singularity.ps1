# SWARMS Singularity Loop
# Usage: ./scripts/start_singularity.ps1 [-MaxCycles 5] [-ProjectName my-project]

param(
    [int]$MaxCycles = 5,
    [string]$ProjectName = ""
)

$ErrorActionPreference = "Stop"

Write-Host "============================================" -ForegroundColor Magenta
Write-Host "      SWARMS SINGULARITY STARTED           " -ForegroundColor Magenta
Write-Host "============================================" -ForegroundColor Magenta

$cycle = 1

while ($cycle -le $MaxCycles) {
    if (Test-Path "STOP_SINGULARITY") {
        Write-Host "[LOOP] Stop file detected. Halting." -ForegroundColor Red
        break
    }

    Write-Host ""
    Write-Host ">>> CYCLE $cycle / $MaxCycles <<<" -ForegroundColor Cyan
    Write-Host "--------------------------------"
    
    # 1. ARCHITECT PHASE
    Write-Host "[LOOP] Summoning Architect..." -ForegroundColor Yellow
    $archCmd = "python scripts/architect.py"
    if ($ProjectName -ne "") { $archCmd += " --project `"$ProjectName`"" }
    Invoke-Expression $archCmd
    
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[LOOP] Architect failed. Aborting." -ForegroundColor Red
        break
    }
    
    # 2. SWARM PHASE
    Write-Host "[LOOP] Unleashing Swarm..." -ForegroundColor Yellow
    $swarmCmd = "pwsh scripts/parallel_swarm.ps1"
    if ($ProjectName -ne "") { $swarmCmd += " -ProjectName `"$ProjectName`"" }
    Invoke-Expression $swarmCmd
    
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[LOOP] Swarm reported issues. Continuing to next cycle to fix..." -ForegroundColor DarkGray
    }
    
    # 3. VERIFY
    # (Implicitly done by Validation Hooks in Swarm) //
    
    Write-Host "[LOOP] Cycle $cycle Complete." -ForegroundColor Green
    $cycle++
    
    # Pause between cycles
    Start-Sleep -Seconds 5
}

Write-Host "============================================" -ForegroundColor Magenta
Write-Host "      SINGULARITY COMPLETE                 " -ForegroundColor Magenta
Write-Host "============================================" -ForegroundColor Magenta
