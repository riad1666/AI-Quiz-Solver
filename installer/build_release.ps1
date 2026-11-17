$ErrorActionPreference = "Stop"

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) {
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
}

$env:CARGO_INCREMENTAL = "0"
$env:RUSTFLAGS = "-C codegen-units=1"

Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    cargo test -j 1
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    cargo build --release -j 2
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Push-Location "$PSScriptRoot\setup_app"
    try {
        cargo build --release --offline -j 2
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
        Pop-Location
    }

    $setupOut = "target\release\KNUDiagnosis-Setup-0.1.0.exe"
    Copy-Item "$PSScriptRoot\setup_app\target\release\knu_diagnosis_setup.exe" $setupOut -Force
    Write-Host "Standalone installable EXE built at $setupOut"
} finally {
    Pop-Location
}
