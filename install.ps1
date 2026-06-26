param(
    [string]$BinDir = "$env:USERPROFILE\.local\bin",
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $MyInvocation.MyCommand.Path
$launcher = Join-Path $BinDir "swarm.ps1"

if ($Uninstall) {
    if (Test-Path $launcher) {
        Remove-Item -Path $launcher -Force
        Write-Host "Removed $launcher"
    }
    exit 0
}

New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
@"
param(
    [Parameter(ValueFromRemainingArguments=`$true)]
    [string[]]`$Args
)

`$repo = "$repo"
if (-not `$Args) {
    `$Args = @("doctor")
}
python "`$repo\scripts\swarm.py" @Args
exit `$LASTEXITCODE
"@ | Set-Content -Path $launcher -Encoding UTF8

Write-Host "Installed $launcher"
Write-Host "Add $BinDir to PATH if needed."
