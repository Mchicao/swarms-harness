param(
    [string]$BinDir = "$env:USERPROFILE\.local\bin",
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $MyInvocation.MyCommand.Path
$binaryName = "swarms-rs.exe"
$launcher = Join-Path $BinDir "swarm.ps1"
$binary = Join-Path $BinDir $binaryName

if ($Uninstall) {
    foreach ($file in @($launcher, $binary)) {
        if (Test-Path $file) {
            Remove-Item -Path $file -Force
            Write-Host "Removed $file"
        }
    }
    exit 0
}

# Build the native Rust runtime; no Python is involved anymore.
cargo build --release --manifest-path (Join-Path $repo "rust\Cargo.toml")
$built = Join-Path $repo "rust\target\release\$binaryName"
if (-not (Test-Path $built)) {
    throw "Release build did not produce $built"
}

New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
Copy-Item -Path $built -Destination $binary -Force

@"
param(
    [Parameter(ValueFromRemainingArguments=`$true)]
    [string[]]`$Args
)

`$binary = "$binary"
if (-not `$Args) {
    `$Args = @("doctor")
}
& `$binary @Args
exit `$LASTEXITCODE
"@ | Set-Content -Path $launcher -Encoding UTF8

Write-Host "Installed $launcher (native swarms-rs binary)"
Write-Host "Add $BinDir to PATH if needed."
