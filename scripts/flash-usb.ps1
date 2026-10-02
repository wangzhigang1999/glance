# Use the build runtime instead of the Windows Store alias.
$ErrorActionPreference = 'Stop'
$flashPython = Join-Path $env:USERPROFILE '.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe'
if (-not (Test-Path -LiteralPath $flashPython)) {
    $flashPython = (Get-Command python -ErrorAction Stop).Source
}
& $flashPython (Join-Path $PSScriptRoot 'flash-usb.py') @args
if ($LASTEXITCODE -ne 0) { throw "USB flash failed: $LASTEXITCODE" }
