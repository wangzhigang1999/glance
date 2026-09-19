$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
Push-Location $repoRoot
try {
    $hostTarget = ((rustc +stable -vV | Select-String '^host: ').Line -replace '^host: ', '')
    cargo +stable test --locked --manifest-path tests/host/Cargo.toml --target $hostTarget --target-dir (Join-Path $repoRoot 'target/host-tests')
    if ($LASTEXITCODE -ne 0) { throw 'Host tests failed' }
} finally { Pop-Location }
