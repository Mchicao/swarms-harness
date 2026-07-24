# Parallel Swarm V11 - Manager Edition (Git Worktrees & Observability)
# Launches fully automated workers in ISOLATED Git Worktrees.
# Closes the loop: Updates swarm_tasks.md real-time.

param(
    [string]$TaskFile = "tasks.md",
    [string]$ContractFile = "CONTRACT.yaml",
    [string]$ProjectRoot = (Get-Location).Path,
    [string]$ProjectName = "", # Subfolder inside .agent/ (e.g. "mockup"). If empty, uses .agent/ root.
    [int]$WorkerCount = 4,
    [ValidateSet("auto", "mock-only", "glm-only", "gemini-only", "codex-only", "round-robin", "role-based", "scout-driven")]
    [string]$ProviderStrategy = "auto",
    [string]$AiderModel = "qwen/qwen3-32b",
    [switch]$Background,
    [switch]$DryRun,
    [switch]$NoRetry,
    [switch]$KillStaleWorkers,
    [int]$WorkerTimeoutMinutes = 10
)

$ErrorActionPreference = "Stop"

# --- VALIDATION: TaskFile is REQUIRED ---
if (-not $TaskFile -or $TaskFile -eq "tasks.md") {
    Write-Host ""
    Write-Host "============================================" -ForegroundColor Red
    Write-Host "   ERROR: -TaskFile es OBLIGATORIO          " -ForegroundColor Red
    Write-Host "============================================" -ForegroundColor Red
    Write-Host ""
    Write-Host "El parametro -TaskFile es requerido para evitar colisiones entre flujos de trabajo." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "USO CORRECTO:" -ForegroundColor Cyan
    Write-Host "  .\scripts\swarm\parallel_swarm.ps1 -TaskFile .agent/tasks_mi_feature.md" -ForegroundColor White
    Write-Host ""
    exit 1
}

# --- VALIDATION: TaskFile must exist ---
if (-not (Test-Path $TaskFile)) {
    # Try resolving relative to agent root first
    $possiblePath = Join-Path ".agent" $TaskFile
    if (-not (Test-Path $possiblePath)) {
        Write-Host ""
        Write-Host "============================================" -ForegroundColor Red
        Write-Host "   ERROR: Archivo de tareas no existe       " -ForegroundColor Red
        Write-Host "============================================" -ForegroundColor Red
        Write-Host ""
        Write-Host "El archivo especificado no existe: $TaskFile" -ForegroundColor Yellow
        exit 1
    }
}


# --- Path Resolution ---
$AgentRoot = ".agent"
if ($ProjectName -ne "") {
    $AgentRoot = Join-Path ".agent" $ProjectName
}

# Resolve defaults if relative paths given
if (-not $TaskFile.Contains("\") -and -not $TaskFile.Contains("/")) {
    $TaskFile = Join-Path $AgentRoot $TaskFile
}
if (-not $ContractFile.Contains("\") -and -not $ContractFile.Contains("/")) {
    $ContractFile = Join-Path $AgentRoot $ContractFile
}

# --- Load Environment Variables ---
if (Test-Path "$ProjectRoot\.env") {
    Write-Host "Loading .env file..." -ForegroundColor DarkGray
    Get-Content "$ProjectRoot\.env" | ForEach-Object {
        if ($_ -match "^\s*([^#=]+)\s*=\s*(.*)") {
            $key = $Matches[1].Trim()
            $value = $Matches[2].Trim()
            if ($value -match "^['`"](.*)['`"]$") { $value = $Matches[1] }
            [System.Environment]::SetEnvironmentVariable($key, $value, "Process")
        }
    }
}

# --- Configuration ---
$LimitsFile = Join-Path $AgentRoot "swarm_limits.yaml"
$UsageFile = Join-Path $AgentRoot "usage_stats.json"
$DelayMs = 2000

# --- 0. SWARM LOAD LIMITS ---
$script:SwarmLimits = @{}
if (Test-Path $LimitsFile) {
    Write-Host "[SWARM] Loading limits from $LimitsFile..." -ForegroundColor DarkGray
    # Usar python para convertir YAML a JSON y leer en PS
    $jsonLimits = python -c "import yaml, json, sys; print(json.dumps(yaml.safe_load(open('$LimitsFile'))))" 2>$null
    if ($LASTEXITCODE -eq 0 -and $jsonLimits) {
        $script:SwarmLimits = $jsonLimits | ConvertFrom-Json
        Write-Host "[SWARM] Limits loaded successfully." -ForegroundColor Gray
        
        # Auto-config based on _meta
        if ($script:SwarmLimits."_meta") {
            if (-not $PSBoundParameters.ContainsKey("WorkerCount")) {
                $WorkerCount = $script:SwarmLimits."_meta".recommended_workers
            }
            if (-not $PSBoundParameters.ContainsKey("ProviderStrategy")) {
                $ProviderStrategy = $script:SwarmLimits."_meta".recommended_strategy
            }
        }
    }
}

# --- Select Provider Routing ---
function Select-Provider {
    param([string]$TaskRaw, [int]$WorkerIndex)

    if ($ProviderStrategy -in @("auto", "mock-only", "glm-only", "role-based", "scout-driven", "gemini-only", "codex-only")) {
        try {
            $routerScript = Join-Path $PSScriptRoot "smart_router.py"
            if (Test-Path $routerScript) {
                $routerJson = python $routerScript --task $TaskRaw --strategy $ProviderStrategy --format powershell 2>$null
                if ($LASTEXITCODE -eq 0 -and $routerJson) {
                    $route = $routerJson | ConvertFrom-Json
                    $selected = @{
                        Provider = $route.Provider
                        Model = $route.Model
                        CanonicalModel = $route.CanonicalModel
                        Wrapper = $route.Wrapper
                        RouteId = $route.RouteId
                        RoutingMethod = $route.RoutingMethod
                        RoutingReason = $route.RoutingReason
                        TaskRole = $route.TaskRole
                        Score = $route.Score
                    }
                    if ($route.ApiKeyEnv -and [System.Environment]::GetEnvironmentVariable($route.ApiKeyEnv, "Process")) {
                        $selected.ApiKey = [System.Environment]::GetEnvironmentVariable($route.ApiKeyEnv, "Process")
                    }
                    Write-Host "[ROUTER] $($selected.RouteId) -> $($selected.Provider)/$($selected.Model) ($($selected.RoutingMethod): $($selected.RoutingReason))" -ForegroundColor DarkGray
                    return $selected
                }
            }
        } catch {
            Write-Host "[ROUTER] smart_router failed, falling back to built-in routing: $_" -ForegroundColor Yellow
        }
    }
    
    # Extraer rol del tag: "- [ ] [backend] Implementar API" -> "backend"
    $role = "general"
    if ($TaskRaw -match '\[(\w+)\]') { $role = $Matches[1].ToLower() }
    
    switch ($ProviderStrategy) {
        "mock-only" { return @{ Provider="mock"; Model="mock-worker"; CanonicalModel="mock-worker"; Wrapper="mock"; RouteId="mock"; RoutingMethod="mock-only"; RoutingReason="offline mock provider" } }
        "gemini-only" { return @{ Provider="antigravity_cli"; Model="gemini-3.5-flash"; Wrapper="gemini" } }
        "codex-only"  { return @{ Provider="codex_cli"; Model="gpt-5.5-codex"; Wrapper="codex" } }
        
        "round-robin" {
            $isEven = ($WorkerIndex % 2) -eq 0
            if ($isEven) {
                return @{ Provider="antigravity_cli"; Model="gemini-3.5-flash"; Wrapper="gemini" }
            } else {
                return @{ Provider="antigravity_api"; Model="gemini-3.5-flash"; Wrapper="gemini"; ApiKey=$env:GOOGLE_API_KEY }
            }
        }
        
        "role-based" {
            $roleMap = @{
                "backend"  = @{ Provider="zai_coding"; Model="glm-5.2"; Wrapper="zai_clean" }
                "codex"    = @{ Provider="codex_cli"; Model="gpt-5.5-codex"; Wrapper="codex" }
                "lite"     = @{ Provider="antigravity_cli"; Model="gemini-3.5-flash"; Wrapper="gemini" }
                "frontend" = @{ Provider="zai_coding"; Model="glm-5.2"; Wrapper="zai_clean" }
                "debug"    = @{ Provider="zai_coding"; Model="glm-5.2"; Wrapper="zai_clean" }
                "docs"     = @{ Provider="antigravity_cli"; Model="gemini-3.5-flash"; Wrapper="gemini" }
                "qa"       = @{ Provider="antigravity_cli"; Model="gemini-3.5-flash"; Wrapper="gemini" }
                "general"  = @{ Provider="zai_coding"; Model="glm-5.2"; Wrapper="zai_clean" }
            }
            if ($null -ne $roleMap.$role) { return $roleMap.$role }
            return $roleMap["general"]
        }
        
        "scout-driven" {
            # Fallback chain: Kilo Auto Free > Laguna Free > Nex-N2 Free > Step 3.7 Free
            $chain = @("kilo_auto_free", "kilo_laguna", "kilo_nex", "kilo_step_flash")
            foreach ($p in $chain) {
                if ($null -ne $script:SwarmLimits.$p -and $script:SwarmLimits.$p.status -eq "ok") {
                    if ($p -match "kilo_auto_free") { return @{ Provider="kilo"; Model="kilo/kilo-auto/free"; Wrapper="kilo" } }
                    if ($p -match "kilo_laguna") { return @{ Provider="kilo"; Model="kilo/poolside/laguna-m.1:free"; Wrapper="kilo" } }
                    if ($p -match "kilo_nex") { return @{ Provider="kilo"; Model="kilo/nex-agi/nex-n2-pro:free"; Wrapper="kilo" } }
                    if ($p -match "kilo_step_flash") { return @{ Provider="kilo"; Model="kilo/stepfun/step-3.7-flash:free"; Wrapper="kilo" } }
                }
            }
            # Ultimate fallback
            return @{ Provider="kilo"; Model="kilo/kilo-auto/free"; Wrapper="kilo" }
        }
    }
}


# --- Worktrees & Traces Setup ---
# Mover worktrees fuera del root para no ensuciar el repo
$ParentDir = Split-Path $ProjectRoot -Parent
$ExternalRoot = Join-Path $ParentDir ".swarm_worktrees"
$ProjectFolderName = Split-Path $ProjectRoot -Leaf
$WorktreesDir = Join-Path $ExternalRoot $ProjectFolderName

$TracesDir = Join-Path $AgentRoot "traces"
$LogsDir = Join-Path $AgentRoot "logs"
$TelemetryUtilsDir = Join-Path $PSScriptRoot "utils"
$TelemetryFile = Join-Path $ProjectRoot ".agent\traces\telemetry.jsonl"
if (-not [System.Environment]::GetEnvironmentVariable("SWARM_TELEMETRY_FILE", "Process")) {
    [System.Environment]::SetEnvironmentVariable("SWARM_TELEMETRY_FILE", $TelemetryFile, "Process")
}
$BaseBranch = "HEAD"
$oldErrorActionPreference = $ErrorActionPreference
$ErrorActionPreference = "Continue"
git -C $ProjectRoot rev-parse --verify main *> $null
if ($LASTEXITCODE -eq 0) {
    $BaseBranch = "main"
} else {
    git -C $ProjectRoot rev-parse --verify master *> $null
    if ($LASTEXITCODE -eq 0) { $BaseBranch = "master" }
}
$ErrorActionPreference = $oldErrorActionPreference

Write-Host "[GIT] Worktrees isolated in: $WorktreesDir" -ForegroundColor DarkGray
New-Item -ItemType Directory -Path $WorktreesDir -Force | Out-Null
New-Item -ItemType Directory -Path $TracesDir -Force | Out-Null
New-Item -ItemType Directory -Path $LogsDir -Force | Out-Null

# --- Usage Stats Helper ---
function Add-Usage {
    if (-not (Test-Path $UsageFile)) {
        @{ date = (Get-Date -Format "yyyy-MM-dd"); total_requests = 0; daily_limit = 1500 } | ConvertTo-Json | Set-Content $UsageFile
    }
    $stats = Get-Content $UsageFile | ConvertFrom-Json
    $stats.total_requests++
    $stats | ConvertTo-Json | Set-Content $UsageFile
    return $stats
}

function Invoke-GitQuiet {
    param([string[]]$ArgsList)
    $oldErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    & git @ArgsList *> $null
    $code = $LASTEXITCODE
    $ErrorActionPreference = $oldErrorActionPreference
    return $code
}

function Remove-SwarmWorktree {
    param([string]$Root, [string]$Path, [string]$Branch)
    if ($Path) {
        [void](Invoke-GitQuiet -ArgsList @("-C", $Root, "worktree", "remove", "--force", $Path))
        if (Test-Path $Path) {
            try { Microsoft.PowerShell.Management\Remove-Item -Path $Path -Recurse -Force -ErrorAction SilentlyContinue } catch {}
        }
    }
    if ($Branch) {
        [void](Invoke-GitQuiet -ArgsList @("-C", $Root, "branch", "-D", $Branch))
    }
}

function Stop-ProcessTree {
    param([int]$RootProcessId)
    if (-not $RootProcessId) { return }
    $children = Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $RootProcessId }
    foreach ($child in $children) {
        Stop-ProcessTree -RootProcessId $child.ProcessId
    }
    try { Stop-Process -Id $RootProcessId -Force -ErrorAction SilentlyContinue } catch {}
}

function Get-DisallowedBenchmarkChanges {
    param([string[]]$ChangedFiles)
    $disallowed = @()
    foreach ($file in $ChangedFiles) {
        $normalized = ([string]$file).Replace("\", "/")
        if (-not (
            $normalized.StartsWith("bench_apps/") -or
            $normalized.StartsWith("bench_tests/") -or
            $normalized.StartsWith("docs/bench_notes/")
        )) {
            $disallowed += $normalized
        }
    }
    return $disallowed
}

function Get-DisallowedBenchmarkTaskChanges {
    param([string]$TaskName, [string[]]$ChangedFiles)
    $disallowed = @()
    $isTestCreationTask = $TaskName -match "\[qa\]" -and $TaskName -match "Create\s+bench_tests/"
    foreach ($file in $ChangedFiles) {
        $normalized = ([string]$file).Replace("\", "/")
        if ($isTestCreationTask -and $normalized.StartsWith("bench_apps/")) {
            $disallowed += $normalized
        }
    }
    return $disallowed
}

function Clear-WorkerArtifact {
    param([string]$WorktreePath, [string]$RelativePath)
    $oldErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    git -C $WorktreePath ls-files --error-unmatch -- $RelativePath *> $null
    $isTracked = ($LASTEXITCODE -eq 0)
    if ($isTracked) {
        git -C $WorktreePath restore -- $RelativePath *> $null
    } else {
        $artifactPath = Join-Path $WorktreePath $RelativePath
        if (Test-Path $artifactPath) {
            Remove-Item -LiteralPath $artifactPath -Force -ErrorAction SilentlyContinue
        }
    }
    $ErrorActionPreference = $oldErrorActionPreference
}

function Invoke-AgyOverhead {
    param([string]$Prompt, [string]$FileArg = $null, [string]$Phase = "watcher", [string]$TaskId = "")

    $benchmark_id = [System.Environment]::GetEnvironmentVariable("SWARM_BENCHMARK_ID", "Process")
    if (-not $benchmark_id) { $benchmark_id = "default-run" }
    $run_id = [System.Environment]::GetEnvironmentVariable("SWARM_RUN_ID", "Process")
    if (-not $run_id) { $run_id = [System.Guid]::NewGuid().ToString() }

    $startedTime = (Get-Date).ToString("o")

    # NOTE: `agy --print` does not emit the model response on stdout in headless
    # mode (see docs/AGY_PROGRAMMATIC.md). We delegate to scripts/agy_call.py,
    # which runs agy and then reads the answer from the persisted conversation.
    $agyCallScript = Join-Path $PSScriptRoot "agy_call.py"
    $agyModel = [System.Environment]::GetEnvironmentVariable("AGY_MODEL", "Process")
    if (-not $agyModel) { $agyModel = "Gemini 3.5 Flash (Medium)" }

    # If a file arg is given, read it inline so the wrapper gets the full prompt.
    $effectivePrompt = $Prompt
    if ($FileArg) { $effectivePrompt = "@" + $FileArg }

    Write-Host "[SWARM-TELEMETRY] Invoking agy overhead for $Phase..." -ForegroundColor DarkGray
    $output = python $agyCallScript --model $agyModel $effectivePrompt 2>$null
    $endedTime = (Get-Date).ToString("o")

    $pythonCmd = "import sys; sys.path.insert(0, r'$TelemetryUtilsDir'); from token_telemetry import parse_stdout_text, record_event; import json;"
    $escapedText = ($output -join "`n").Replace("'", "''").Replace("`n", "\n").Replace("`r", "\r")
    $jsonUsage = python -c "$pythonCmd print(json.dumps(parse_stdout_text('''$escapedText''')))" 2>$null
    
    $input_tokens = 0
    $output_tokens = 0
    $cached_tokens = 0
    $reasoning_tokens = 0
    $usage_source = "missing"
    if ($jsonUsage) {
        $usageObj = $jsonUsage | ConvertFrom-Json
        $input_tokens = $usageObj.input
        $output_tokens = $usageObj.output
        $cached_tokens = $usageObj.cached
        $reasoning_tokens = $usageObj.reasoning
        if ($input_tokens -gt 0) { $usage_source = "cli_reported" }
    }
    
    python -c "$pythonCmd record_event('$run_id', '$benchmark_id', '$Phase', 'antigravity_cli', 'gemini-3.5-flash', 'overhead', '$TaskId', $input_tokens, $cached_tokens, 0, $output_tokens, $reasoning_tokens, '$usage_source', True, '$startedTime', '$endedTime')" 2>$null
    
    return ($output -join "`n")
}

function Invoke-KiloOverhead {
    param([string]$Prompt, [string]$Phase = "watcher", [string]$TaskId = "")
    
    $benchmark_id = [System.Environment]::GetEnvironmentVariable("SWARM_BENCHMARK_ID", "Process")
    if (-not $benchmark_id) { $benchmark_id = "default-run" }
    $run_id = [System.Environment]::GetEnvironmentVariable("SWARM_RUN_ID", "Process")
    if (-not $run_id) { $run_id = [System.Guid]::NewGuid().ToString() }
    
    $startedTime = (Get-Date).ToString("o")
    
    $cmd = "kilo run -m `"kilo/z-ai/glm-5:free`" --auto `"$Prompt`""
    Write-Host "[SWARM-TELEMETRY] Invoking kilo overhead for $Phase..." -ForegroundColor DarkGray
    $output = Invoke-Expression "$cmd 2>$null"
    $endedTime = (Get-Date).ToString("o")
    
    $pythonCmd = "import sys; sys.path.insert(0, r'$TelemetryUtilsDir'); from token_telemetry import parse_stdout_text, record_event; import json;"
    $escapedText = ($output -join "`n").Replace("'", "''").Replace("`n", "\n").Replace("`r", "\r")
    $jsonUsage = python -c "$pythonCmd print(json.dumps(parse_stdout_text('''$escapedText''')))" 2>$null
    
    $input_tokens = 0
    $output_tokens = 0
    $cached_tokens = 0
    $reasoning_tokens = 0
    $usage_source = "missing"
    if ($jsonUsage) {
        $usageObj = $jsonUsage | ConvertFrom-Json
        $input_tokens = $usageObj.input
        $output_tokens = $usageObj.output
        $cached_tokens = $usageObj.cached
        $reasoning_tokens = $usageObj.reasoning
        if ($input_tokens -gt 0) { $usage_source = "cli_reported" }
    }
    
    python -c "$pythonCmd record_event('$run_id', '$benchmark_id', '$Phase', 'kilo', 'kilo/z-ai/glm-5:free', 'overhead', '$TaskId', $input_tokens, $cached_tokens, 0, $output_tokens, $reasoning_tokens, '$usage_source', True, '$startedTime', '$endedTime')" 2>$null
    
    return ($output -join "`n")
}

Write-Host "============================================" -ForegroundColor Cyan
Write-Host "   PARALLEL SWARM V11 - WORKTREES EDITION   " -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host "[SWARM] Workers: $WorkerCount"
Write-Host "[SWARM] Provider Strategy: $ProviderStrategy" -ForegroundColor Magenta
$UseAiWatcher = $ProviderStrategy -ne "mock-only"
if (-not $UseAiWatcher) {
    Write-Host "[SWARM] AI watcher disabled for mock-only strategy." -ForegroundColor DarkGray
}

# --- 0. SWARM LIFECYCLE MANAGER ---
Write-Host "1. Checking worktrees..." -ForegroundColor Yellow
if ($KillStaleWorkers) {
    Write-Host "[SWARM] KillStaleWorkers enabled. Stopping known worker CLIs except Codex Desktop." -ForegroundColor Yellow
    $zombies = Get-Process -Name "aider", "gemini", "kilo" -ErrorAction SilentlyContinue
    if ($zombies) { $zombies | Stop-Process -Force -ErrorAction SilentlyContinue }
} else {
    Write-Host "[SWARM] Skipping global process cleanup. Use -KillStaleWorkers to stop stale aider/gemini/kilo workers." -ForegroundColor DarkGray
}
git -C $ProjectRoot worktree prune

$script:SuccessCount = 0
$script:FailCount = 0
$script:TasksProcessed = @()

$activeWorkers = @{} 
$roleKeys = @("backend", "frontend", "qa", "general")

while ($true) {
    # 1. CHECK COMPLETED WORKERS
    $completedIds = @()
    $workerKeys = @($activeWorkers.Keys)
    foreach ($id in $workerKeys) {
        $worker = $activeWorkers[$id]
        if ($null -eq $worker -or $null -eq $worker.StatusFile) { continue }

        if (-not (Test-Path $worker.StatusFile) -and $WorkerTimeoutMinutes -gt 0) {
            $elapsedMinutes = ((Get-Date) - $worker.StartTime).TotalMinutes
            if ($elapsedMinutes -ge $WorkerTimeoutMinutes) {
                Write-Host "[SWARM] Worker timeout after $([math]::Round($elapsedMinutes, 1)) minutes: $($worker.TaskName)" -ForegroundColor Red
                if ($worker.ProcessId) {
                    Stop-ProcessTree -RootProcessId $worker.ProcessId
                }
                Set-Content -Path $worker.StatusFile -Value 124
            }
        }
        
        if (Test-Path $worker.StatusFile) {
            $statusContent = Get-Content $worker.StatusFile -ErrorAction SilentlyContinue
            if ($null -eq $statusContent) { continue }
            $exitCode = $statusContent.Trim()
            $completedIds += $id
            
            # --- TOKEN TELEMETRY EXTRACTION ---
            $wtPath = $worker.WorktreePath
            $pythonCmd = "import sys; sys.path.insert(0, r'$TelemetryUtilsDir'); from token_telemetry import parse_codex_log, parse_opencode_log, parse_stdout_text, record_event; import json, pathlib, os;"
            $input_tokens = 0
            $output_tokens = 0
            $cached_tokens = 0
            $cache_write_tokens = 0
            $reasoning_tokens = 0
            $usage_source = "missing"
            
            try {
                if ($worker.Provider -eq "codex_cli") {
                    $logFile = Join-Path $wtPath ".agent_codex_out.txt"
                    if (Test-Path $logFile) {
                        $jsonUsage = python -c "$pythonCmd log_path = pathlib.Path('$($logFile.Replace('\', '/'))'); print(json.dumps(parse_codex_log(log_path)))" 2>$null
                        if ($jsonUsage) {
                            $usageObj = $jsonUsage | ConvertFrom-Json
                            $input_tokens = $usageObj.input
                            $output_tokens = $usageObj.output
                            $cached_tokens = $usageObj.cached
                            $reasoning_tokens = $usageObj.reasoning
                            $usage_source = "cli_reported"
                        }
                    }
                } elseif ($worker.Provider -eq "zai_coding") {
                    $usageJsonFile = Join-Path $wtPath "token_usage.json"
                    if (Test-Path $usageJsonFile) {
                        $usageObj = Get-Content $usageJsonFile -Raw | ConvertFrom-Json
                        $input_tokens = $usageObj.input
                        $output_tokens = $usageObj.output
                        $cached_tokens = $usageObj.cached
                        if ($null -ne $usageObj.cache_write) { $cache_write_tokens = $usageObj.cache_write }
                        if ($null -ne $usageObj.cache_write_input_tokens) { $cache_write_tokens = $usageObj.cache_write_input_tokens }
                        if ($null -ne $usageObj.reasoning) { $reasoning_tokens = $usageObj.reasoning }
                        if ($null -ne $usageObj.reasoning_output_tokens) { $reasoning_tokens = $usageObj.reasoning_output_tokens }
                        $usage_source = "api_reported"
                    }
                } elseif ($worker.Provider -eq "opencode") {
                    $logFile = Join-Path $wtPath "agent_log.txt"
                    if (Test-Path $logFile) {
                        $jsonUsage = python -c "$pythonCmd log_path = pathlib.Path('$($logFile.Replace('\', '/'))'); print(json.dumps(parse_opencode_log(log_path)))" 2>$null
                        if ($jsonUsage) {
                            $usageObj = $jsonUsage | ConvertFrom-Json
                            $input_tokens = $usageObj.input
                            $output_tokens = $usageObj.output
                            $cached_tokens = $usageObj.cached
                            if ($null -ne $usageObj.cache_write) { $cache_write_tokens = $usageObj.cache_write }
                            if ($null -ne $usageObj.reasoning) { $reasoning_tokens = $usageObj.reasoning }
                            if ($input_tokens -gt 0) { $usage_source = "cli_reported" }
                        }
                    }
                } else {
                    $logFile = Join-Path $wtPath "agent_log.txt"
                    if (Test-Path $logFile) {
                        $logText = Get-Content $logFile -Raw
                        $escapedText = $logText.Replace("'", "''").Replace("`n", "\n").Replace("`r", "\r")
                        $jsonUsage = python -c "$pythonCmd print(json.dumps(parse_stdout_text('''$escapedText''')))" 2>$null
                        if ($jsonUsage) {
                            $usageObj = $jsonUsage | ConvertFrom-Json
                            $input_tokens = $usageObj.input
                            $output_tokens = $usageObj.output
                            $cached_tokens = $usageObj.cached
                            $reasoning_tokens = $usageObj.reasoning
                            if ($input_tokens -gt 0) { $usage_source = "cli_reported" }
                        }
                    }
                }
                
                # Fetch Run/Benchmark ID from Env
                $benchmark_id = [System.Environment]::GetEnvironmentVariable("SWARM_BENCHMARK_ID", "Process")
                if (-not $benchmark_id) { $benchmark_id = "default-run" }
                $run_id = [System.Environment]::GetEnvironmentVariable("SWARM_RUN_ID", "Process")
                if (-not $run_id) { $run_id = [System.Guid]::NewGuid().ToString() }
                
                $modelName = $worker.Provider
                if ($worker.CanonicalModel) { $modelName = $worker.CanonicalModel }
                elseif ($worker.Provider -eq "codex_cli") { $modelName = "gpt-5.5-codex" }
                elseif ($worker.Provider -eq "zai_coding") { $modelName = "glm-5.2" }
                elseif ($worker.Provider -eq "antigravity_cli") { $modelName = "gemini-3.5-flash" }
                elseif ($worker.Provider -eq "mock") { $modelName = "mock-worker" }
                
                $endedTime = (Get-Date).ToString("o")
                $startedTime = $worker.StartTime.ToString("o")
                if ($exitCode -eq "0") { $successBool = "True" } else { $successBool = "False" }
                $routeId = [string]$worker.RouteId
                $routingMethod = [string]$worker.RoutingMethod
                $routingReason = ([string]$worker.RoutingReason).Replace("'", "''")
                
                python -c "$pythonCmd record_event('$run_id', '$benchmark_id', 'swarm', '$($worker.Provider)', '$modelName', 'worker', '$($worker.TaskIndex)', $input_tokens, $cached_tokens, $cache_write_tokens, $output_tokens, $reasoning_tokens, '$usage_source', $successBool, '$startedTime', '$endedTime', route_id='$routeId', routing_method='$routingMethod', routing_reason='$routingReason')" 2>$null
            } catch {
                Write-Host "[SWARM-TELEMETRY] Error writing worker telemetry: $_" -ForegroundColor Yellow
            }
            
            $lines = Get-Content $TaskFile
            
            # --- WORKTREE MERGE LOGIC ---
            $wtPath = $worker.WorktreePath
            $branch = $worker.Branch
            
            try {
                if ($exitCode -ne "0") {
                    throw "Task returned non-zero exit code: $exitCode"
                }
                
                Write-Host "[SWARM] Task DONE: $($worker.TaskName)" -ForegroundColor Green
                
                foreach ($internalArtifact in @("prompt.txt", "run.ps1", "agent_log.txt", "status.txt", "changes.diff", "token_usage.json", ".agent_codex_out.txt", ".agent/memory/PROJECT_MEMORY.md")) {
                    Clear-WorkerArtifact -WorktreePath $wtPath -RelativePath $internalArtifact
                }

                Push-Location $wtPath
                git add -A
                $stagedChanges = git diff --cached --name-only
                if (-not $stagedChanges) {
                    Pop-Location
                    if ($worker.TaskName -match "Run\s+pytest|verify|verification") {
                        Write-Host "[SWARM] Verification task completed without code changes." -ForegroundColor Green
                        $lines[$worker.TaskIndex] = "- [x] $($worker.TaskName)"
                        Remove-SwarmWorktree -Root $ProjectRoot -Path $wtPath -Branch $branch
                        $script:SuccessCount++
                        $script:TasksProcessed += "OK $($worker.TaskName)"
                        Set-Content -Path $TaskFile -Value $lines
                        continue
                    }
                    throw "Task completed without repository changes"
                }
                if ([System.Environment]::GetEnvironmentVariable("SWARM_BENCHMARK_ID", "Process")) {
                    $disallowedChanges = Get-DisallowedBenchmarkChanges -ChangedFiles $stagedChanges
                    $disallowedChanges += Get-DisallowedBenchmarkTaskChanges -TaskName $worker.TaskName -ChangedFiles $stagedChanges
                    if ($disallowedChanges.Count -gt 0) {
                        git reset -q
                        Pop-Location
                        throw "Benchmark task modified disallowed paths: $($disallowedChanges -join ', ')"
                    }
                }
                git commit -m "Swarm Task: $($worker.TaskName)" --allow-empty
                if ($LASTEXITCODE -ne 0) {
                    Pop-Location
                    throw "Failed to commit worker changes"
                }
                Pop-Location
                
                # --- WATCHER v2: Local-First Validation ---
                $diffFile = Join-Path $wtPath "changes.diff"
                git -C $ProjectRoot diff "$BaseBranch..$branch" > $diffFile
                $diffLines = (Get-Content $diffFile | Measure-Object -Line).Lines

                # FASE 1: Validación Local (Ahorra API Keys)
                Write-Host "[WATCHER-LOCAL] Ejecutando Ruff & Pytest..." -ForegroundColor Yellow
                $ruffCmd = Get-Command ruff -ErrorAction SilentlyContinue
                if ($ruffCmd) {
                    Push-Location $wtPath
                    $lintResult = ruff check . --select E,F --quiet 2>&1
                    Pop-Location

                    if ($LASTEXITCODE -ne 0) {
                        Write-Host "[WATCHER-LOCAL] REJECTED: Lint failed" -ForegroundColor Red
                        if ($worker.TaskIndex -ge 0 -and $worker.TaskIndex -lt $lines.Count) {
                            $lines[$worker.TaskIndex] = "- [FAILED] $($worker.TaskName) (Lint Error)"
                        }
                        Set-Content -Path $TaskFile -Value $lines
                        Remove-SwarmWorktree -Root $ProjectRoot -Path $wtPath -Branch $branch
                        continue 
                    }
                } else {
                    Write-Host "[WATCHER-LOCAL] Ruff not found. Skipping lint phase." -ForegroundColor DarkGray
                }

                # FASE 2: AI Review (Solo si el diff es grande > 50 líneas)
                if ($diffLines -gt 50 -and $UseAiWatcher) {
                    Write-Host "[WATCHER-AI] Large diff ($diffLines lines), requesting AI review..." -ForegroundColor Yellow
                    try {
                        $watchCheck = Invoke-AgyOverhead -Prompt "Act as a Supervisor. Review this DIFF against CONTRACT.yaml. Are there any breaking changes? Respond with 'OK' or describe the error." -FileArg $diffFile -Phase "watcher" -TaskId "$($worker.TaskIndex)"
                    } catch {
                        Write-Host "[WATCHER-AI] Antigravity CLI failed. Trying Kilo (GLM-5) fallback..." -ForegroundColor Yellow
                    }

                    if (-not $watchCheck -or $LASTEXITCODE -ne 0) {
                        try {
                            $diffContent = Get-Content $diffFile -Raw
                            if ($diffContent.Length -gt 15000) { $diffContent = $diffContent.Substring(0, 15000) }
                            $watchCheck = Invoke-KiloOverhead -Prompt "Act as a Supervisor. Review this DIFF against CONTRACT.yaml. Are there any breaking changes? Respond with 'OK' or describe the error. DIFF:`n$diffContent" -Phase "watcher" -TaskId "$($worker.TaskIndex)"
                        } catch {
                            Write-Host "[WATCHER-AI] Fallback also failed. Approving by default." -ForegroundColor Yellow
                            $watchCheck = "OK"
                        }
                    }

                    if ($watchCheck -and $watchCheck -notmatch "OK") {
                        Write-Host "[WATCHER-AI REJECTED] $watchCheck" -ForegroundColor Red
                        $lines[$worker.TaskIndex] = "- [FAILED] $($worker.TaskName) (AI Rejected)"
                        Set-Content -Path $TaskFile -Value $lines
                        Remove-SwarmWorktree -Root $ProjectRoot -Path $wtPath -Branch $branch
                        continue 
                    }
                } elseif ($diffLines -gt 50) {
                    Write-Host "[WATCHER-LOCAL] Large diff ($diffLines lines), AI review disabled by strategy." -ForegroundColor DarkGray
                } else {
                    Write-Host "[WATCHER-LOCAL] Small diff ($diffLines lines), skipping AI review." -ForegroundColor Green
                }

                # FASE 2.5: GOAL EVALUATOR (Modo /goal de Claude Code/Codex)
                if ($worker.TaskName -match '@goal\("([^"]+)"\)') {
                    $goalText = $Matches[1]
                    Write-Host "[GOAL-EVALUATOR] Verificando objetivo: $goalText..." -ForegroundColor Yellow
                    $evalLog = Join-Path $wtPath "agent_log.txt"
                    
                    $evalOutput = python SWARMS/scripts/goal_evaluator.py --goal "$goalText" --log "$evalLog" --diff "$diffFile" 2>$null
                    if ($LASTEXITCODE -eq 0 -and $evalOutput) {
                        $evalJson = $evalOutput | ConvertFrom-Json
                        if (-not $evalJson.done) {
                            Write-Host "[GOAL-EVALUATOR] META NO CUMPLIDA: $($evalJson.reason)" -ForegroundColor Red
                            # Forzar salida como código de error (1) para gatillar reintento/paramédico
                            $exitCode = "1"
                        } else {
                            Write-Host "[GOAL-EVALUATOR] META ALCANZADA: $($evalJson.reason)" -ForegroundColor Green
                        }
                    }
                }
                
                if ($exitCode -eq "0") {
                    Write-Host "[GIT] Merging worktree changes from $branch..." -ForegroundColor Cyan
                    git -C $ProjectRoot merge --no-ff $branch -m "Merge Swarm Task: $($worker.TaskName)"
                    if ($LASTEXITCODE -eq 0) {
                        Write-Host "[GIT] Merge Successful." -ForegroundColor Green
                        $lines[$worker.TaskIndex] = "- [x] $($worker.TaskName)"
                    } else {
                        Write-Host "[GIT] Merge CONFLICT detected. Worktree preserved for manual resolution." -ForegroundColor Red
                        $lines[$worker.TaskIndex] = "- [FAILED] $($worker.TaskName) (Merge Conflict)"
                        [void](Invoke-GitQuiet -ArgsList @("-C", $ProjectRoot, "merge", "--abort"))
                        throw "Merge conflict"
                    }
                    
                    # Cleanup
                    Remove-SwarmWorktree -Root $ProjectRoot -Path $wtPath -Branch $branch

                    # Metrics & Reports (Success)
                    $script:SuccessCount++
                    $script:TasksProcessed += "OK $($worker.TaskName)"
                } else {
                    # Force jump to failure block by throwing to catch or by bypassing
                    throw "Task returned non-zero exit code: $exitCode"
                }
            } catch {
                Write-Host "[GIT/EXEC] Error during worker execution or merge: $_" -ForegroundColor Red
                
                # RETRY / PARAMEDIC LOGIC (Gemini 3 Flash)
                $currentTask = $worker.TaskName
                if ($currentTask -match "\(Retry: (\d+)\)") {
                     $retryCount = [int]$Matches[1]
                     $cleanTask = $currentTask -replace "\s*\(Retry: \d+\)", ""
                } else { $retryCount = 0; $cleanTask = $currentTask }
                
                if ($retryCount -lt 2 -and -not $NoRetry) {
                     $newCount = $retryCount + 1
                     if ($UseAiWatcher) {
                         Write-Host "[PARAMEDIC] Analizando fallo con Antigravity..." -ForegroundColor Magenta
                     } else {
                         Write-Host "[PARAMEDIC] AI diagnostics disabled by strategy." -ForegroundColor DarkGray
                     }
                     $failLogPath = Join-Path $wtPath "agent_log.txt"
                     $diagnostics = ""
                     if (Test-Path $failLogPath -and $UseAiWatcher) {
                         Write-Host "[SWARM] === LOG OUTPUT DE FAILURE ===" -ForegroundColor DarkRed
                         Get-Content $failLogPath -Tail 15 | Write-Host -ForegroundColor DarkRed
                         $diagnostics = Invoke-AgyOverhead -Prompt "You are an expert paramedic. Analyze this LOG of a failed agent. Summarize the ERROR and provide a short RECOMMENDATION for the next agent." -FileArg $failLogPath -Phase "retry" -TaskId "$($worker.TaskIndex)"
                         Write-Host "[PARAMEDIC] Diagnostics: $diagnostics" -ForegroundColor Gray
                     }
                     $lines[$worker.TaskIndex] = "- [ ] $cleanTask (Retry: $newCount) [DIAG: $diagnostics]"
                } else {
                     $lines[$worker.TaskIndex] = "- [FAILED] $cleanTask (Max Retries)"
                     $script:FailCount++
                     $script:TasksProcessed += "FAILED $cleanTask"
                }
                
                # Cleanup failed worktree
                Remove-SwarmWorktree -Root $ProjectRoot -Path $wtPath -Branch $branch
            }
            Set-Content -Path $TaskFile -Value $lines
        }
    }
    foreach ($id in $completedIds) { $activeWorkers.Remove($id) }

    # 2. TASK SELECTION (Dependency-Aware V11.6)
    $lines = Get-Content $TaskFile
    $pendingTasks = 0
    $launchedThisCycle = 0
    $blockedThisCycle = 0
    
    # ... (Stage Logic as in V10) ...
    $targetStage = $null
    $currentStage = "Uncategorized"
    
    # Find active stage
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        if ($line -match "^##\s+(.+)") { $currentStage = $Matches[1].Trim() }
        if ($line -match "^\s*-\s*\[\s*\]\s*(.+)$") { 
            if ($null -eq $targetStage) { $targetStage = $currentStage; break }
        }
    }
    
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        if ($line -match "^##\s+(.+)") { $currentStage = $Matches[1].Trim() }
        
        if ($line -match "^\s*-\s*\[\s*\]\s*(.+)$") {
            $pendingTasks++
            $taskRaw = $Matches[1].Trim()
            
            if ($targetStage -ne $null -and $currentStage -ne $targetStage) { continue }

            # Dependency verification: check for `@needs(...)` pattern
            $dependenciesSatisfied = $true
            $dependencyFailed = $false
            if ($taskRaw -match '@needs\(([^)]+)\)') {
                $depTargets = $Matches[1].Split(',')
                foreach ($depTarget in $depTargets) {
                    $depTarget = $depTarget.Trim()
                    $foundDep = $false
                    $depCompleted = $false
                    
                    # If dependency is a 1-based line/index or a partial text match
                    if ($depTarget -as [int]) {
                        $depLineIdx = ($depTarget -as [int]) - 1
                        if ($depLineIdx -ge 0 -and $depLineIdx -lt $lines.Count) {
                            $foundDep = $true
                            if ($lines[$depLineIdx] -match "^\s*-\s*\[x\]") { $depCompleted = $true }
                            if ($lines[$depLineIdx] -match "^\s*-\s*\[(FAILED|BLOCKED)\]") { $dependencyFailed = $true }
                        }
                    } else {
                        # Search by text matching in the backlog
                        for ($j = 0; $j -lt $lines.Count; $j++) {
                            if ($j -eq $i) { continue }
                            if ($lines[$j] -like "*$depTarget*") {
                                $foundDep = $true
                                if ($lines[$j] -match "^\s*-\s*\[x\]") { $depCompleted = $true; break }
                                if ($lines[$j] -match "^\s*-\s*\[(FAILED|BLOCKED)\]") { $dependencyFailed = $true; break }
                            }
                        }
                        if (-not $foundDep) {
                            $dependenciesSatisfied = $false
                            break
                        }
                    }
                    
                    if ($dependencyFailed) {
                        $dependenciesSatisfied = $false
                        break
                    }
                    if ($foundDep -and -not $depCompleted) {
                        $dependenciesSatisfied = $false
                        break
                    }
                }
            }

            if (-not $dependenciesSatisfied) {
                if ($dependencyFailed) {
                    $lines[$i] = "- [BLOCKED] $taskRaw (Dependency failed)"
                    $blockedThisCycle++
                    $script:FailCount++
                    $script:TasksProcessed += "BLOCKED $taskRaw"
                    Set-Content -Path $TaskFile -Value $lines
                }
                continue
            }
            
            if ($activeWorkers.Count -lt $WorkerCount) {
                # --- NEW WORKTREE SETUP ---
                $workerInstanceId = [System.Guid]::NewGuid()
                $branchName = "swarm/task-$($workerInstanceId.ToString().Substring(0,8))"
                $wtPath = Join-Path $WorktreesDir $workerInstanceId
                
                Write-Host "[GIT] Creating Worktree for: $taskRaw" -ForegroundColor Cyan
                # Create branch and worktree
                $oldErrorActionPreference = $ErrorActionPreference
                $ErrorActionPreference = "Continue"
                git -C $ProjectRoot worktree add -b $branchName "$wtPath" HEAD *> $null
                $ErrorActionPreference = $oldErrorActionPreference
                
                if (-not (Test-Path $wtPath)) {
                    # Retry with master if origin/master or master checkout fails
                    $oldErrorActionPreference = $ErrorActionPreference
                    $ErrorActionPreference = "Continue"
                    git -C $ProjectRoot worktree add -b $branchName "$wtPath" HEAD *> $null
                    $ErrorActionPreference = $oldErrorActionPreference
                }
                
                if (-not (Test-Path $wtPath)) {
                    Write-Host "[ERROR] Failed to create worktree at $wtPath" -ForegroundColor Red
                    continue
                }

                $excludePath = git -C $wtPath rev-parse --git-path info/exclude
                Add-Content -Path $excludePath -Value @(
                    "",
                    "# SWARMS internal worker artifacts",
                    "prompt.txt",
                    "run.ps1",
                    "agent_log.txt",
                    "status.txt",
                    "changes.diff",
                    "token_usage.json",
                    ".agent_codex_out.txt"
                )
                
                # Copy Environment
                if (Test-Path "$ProjectRoot\.env") {
                    Copy-Item "$ProjectRoot\.env" "$wtPath\.env"
                }
                
                # Mark In Progress
                $lines[$i] = "- [W:GIT] $taskRaw"
                Set-Content -Path $TaskFile -Value $lines
                
                # Prompt Setup
                $promptPath = Join-Path $wtPath "prompt.txt"
                $statusPath = Join-Path $wtPath "status.txt"
                $wrapperScript = Join-Path $wtPath "run.ps1"
                $outputLog = Join-Path $wtPath "agent_log.txt"
                
                # Context Isolation & Prompt Setup
                $contractContent = ""
                if (Test-Path $ContractFile) { $contractContent = Get-Content $ContractFile -Raw }
                
                $promptPath = Join-Path $wtPath "prompt.txt"
                $statusPath = Join-Path $wtPath "status.txt"
                $wrapperScript = Join-Path $wtPath "run.ps1"
                $outputLog = Join-Path $wtPath "agent_log.txt"
                
                $finalPrompt = @"
You are a Swarm Worker in an isolated Git Worktree.

## SYSTEM RULES & CONTRACT (Quality Rules)
$contractContent

## CONSTRAINTS
- MODIFY files directly. DO NOT run git commands.
- For benchmark tasks, only modify files under bench_apps/, bench_tests/, and docs/bench_notes/.
- Do not modify .agent/, core/, scripts/, scratch/, tests/, docs/technical/, prompt.txt, run.ps1, or unrelated project files.
- If you need notes or handoff text for a benchmark task, write it only under docs/bench_notes/.
- If the target role is [backend], do not create or edit bench_tests/ unless that exact target task asks for tests.
- If the target names a specific file, keep edits focused on that file and directly required package init files.
- Run the verification command named in the target task before finishing. For benchmark tasks, this is usually a focused `pytest bench_tests/... -q` command.
- If tests fail, fix them. Max 2 self-repair attempts.

## EXECUTION CONTEXT
- Worktree: $wtPath

## TARGET TASK
$taskRaw
"@
                Set-Content $promptPath $finalPrompt -Encoding UTF8
                
                # Provider Selection
                $provider = Select-Provider -TaskRaw $taskRaw -WorkerIndex $activeWorkers.Count
                
                # Wrapper Generation (Agy / Kilo / Aider / ZAI Clean)
                $apiKeyLine = ""
                $execLine = ""
                if ($provider.Wrapper -eq "gemini") {
                    # Delegate to scripts/agy_call.py: `agy --print` does not emit
                    # the response on stdout in headless mode, so the wrapper reads
                    # the persisted conversation answer (see docs/AGY_PROGRAMMATIC.md).
                    $agyCallScript = Join-Path $PSScriptRoot "agy_call.py"
                    $execLine = "python `"$agyCallScript`" --model `"$($provider.Model)`" --timeout $($WorkerTimeoutMinutes * 60) @prompt.txt 2>&1 | Tee-Object -FilePath '$outputLog'"
                } elseif ($provider.Wrapper -eq "kilo") {
                    $execLine = "kilo run -m $($provider.Model) --auto -f prompt.txt '$(Get-Content $promptPath -Raw)' 2>&1 | Tee-Object -FilePath '$outputLog'"
                } elseif ($provider.Wrapper -eq "codex") {
                    # SWARMS-CODEX-002: -a es global y debe preceder a exec.
                    $execLine = "`$promptText = Get-Content prompt.txt -Raw; & codex -a never exec -o '.agent_codex_out.txt' --json `$promptText 2>&1 | Tee-Object -FilePath '$outputLog'"
                } elseif ($provider.Wrapper -eq "zai_clean") {
                    # Execute clean endpoint using a quick Python script helper with official thinking parameters
                    $apiKeyLine = "if (-not `$env:ZAI_API_KEY) { throw 'ZAI_API_KEY must be set in the environment before running this worker.' }"
                    $execLine = "python -c `"import os, openai, json; client=openai.OpenAI(api_key=os.environ.get('ZAI_API_KEY'), base_url='https://api.z.ai/api/coding/paas/v4'); resp=client.chat.completions.create(model='glm-5.2', messages=[{'role':'system','content':'You are a coding assistant. Complete the target task.'},{'role':'user','content':open('prompt.txt','r',encoding='utf-8').read()}], extra_body={'thinking': {'type': 'enabled'}, 'reasoning_effort': 'max'}); msg=resp.choices[0].message; print(getattr(msg, 'content', None) or getattr(msg, 'reasoning_content', '') or ''); usage=resp.usage; d=getattr(usage, 'prompt_tokens_details', None); cd=getattr(usage, 'completion_tokens_details', None); get=lambda o,k: (o.get(k,0) if isinstance(o,dict) else (getattr(o,k,0) if o else 0)); cached=get(d,'cached_tokens'); cache_write=get(d,'cache_write_tokens') or get(d,'cache_creation_input_tokens'); reasoning=get(cd,'reasoning_tokens'); open('token_usage.json','w',encoding='utf-8').write(json.dumps({'input':usage.prompt_tokens,'output':usage.completion_tokens,'cached':cached,'reasoning':reasoning,'cache_write':cache_write,'cache_read_input_tokens':cached,'cache_write_input_tokens':cache_write,'reasoning_output_tokens':reasoning}))`" 2>&1 | Tee-Object -FilePath '$outputLog'"
                } elseif ($provider.Wrapper -eq "opencode") {
                    $execLine = "opencode run -m $($provider.Model) --format json --dangerously-skip-permissions `"Complete the task described in prompt.txt. Modify files directly and run the requested verification commands.`" --file prompt.txt 2>&1 | Tee-Object -FilePath '$outputLog'"
                } elseif ($provider.Wrapper -eq "mock") {
                    $mockScript = Join-Path $PSScriptRoot "mock_worker.py"
                    $execLine = "python `"$mockScript`" --prompt prompt.txt --status status.txt 2>&1 | Tee-Object -FilePath '$outputLog'"
                } else {
                    $execLine = "aider --model $($provider.Model) --message-file prompt.txt --yes --no-auto-commits 2>&1 | Tee-Object -FilePath '$outputLog'"
                }

                $wrapperContent = @"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
`$env:PYTHONUTF8 = "1"
$apiKeyLine
Set-Location '$wtPath'
`$Host.UI.RawUI.WindowTitle = 'Worker ($branchName) - $($provider.Provider)'
Write-Host '=== Worker ($($provider.Provider)) ===' -ForegroundColor Cyan

# Execute Agent
Write-Host "[INFO] Starting $($provider.Model)..." -ForegroundColor Cyan
$execLine

`$exitCode = `$LASTEXITCODE
Set-Content -Path 'status.txt' -Value `$exitCode
exit `$exitCode
"@
                Set-Content $wrapperScript -Value $wrapperContent -Encoding UTF8
                
                # Launch
                if ($Background) { $windowStyle = "Hidden" } else { $windowStyle = "Minimized" }
                $workerProcess = Start-Process "pwsh" -ArgumentList "-File `"$wrapperScript`"" -WindowStyle $windowStyle -PassThru
                
                Add-Usage | Out-Null
                
                $activeWorkers[$workerInstanceId] = @{
                    TaskIndex = $i; 
                    TaskName = $taskRaw; 
                    StatusFile = $statusPath; 
                    WorktreePath = $wtPath;
                    Branch = $branchName;
                    StartTime = (Get-Date); 
                    ProcessId = $workerProcess.Id
                    Provider = $provider.Provider
                    CanonicalModel = $provider.CanonicalModel
                    RouteId = $provider.RouteId
                    RoutingMethod = $provider.RoutingMethod
                    RoutingReason = $provider.RoutingReason
                    TaskRole = $provider.TaskRole
                }
                $launchedThisCycle++
                
                # Jitter Delay
                $jitter = Get-Random -Minimum 1500 -Maximum 6000
                Write-Host "[JITTER] Delay: ${jitter}ms before next worker" -ForegroundColor DarkGray
                Start-Sleep -Milliseconds $jitter
                break
            }
        }
    }
    
    if ($pendingTasks -eq 0 -and $activeWorkers.Count -eq 0) {
        Write-Host "All tasks completed." -ForegroundColor Green
        break
    }
    if ($pendingTasks -gt 0 -and $activeWorkers.Count -eq 0 -and $launchedThisCycle -eq 0) {
        Write-Host "[SWARM] No runnable tasks remain. Pending tasks are blocked by failed or missing dependencies." -ForegroundColor Red
        exit 1
    }
    
    Start-Sleep -Seconds 2
}

if ($script:FailCount -gt 0) {
    Write-Host "[SWARM] Completed with $script:FailCount failed or blocked task(s)." -ForegroundColor Red
    exit 1
}
exit 0
