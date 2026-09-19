# Use an available base Python without changing the user's global PATH.
param(
    [string]$PythonHome = (Join-Path $env:USERPROFILE '.cache\codex-runtimes\codex-primary-runtime\dependencies\python'),
    [string]$IdfHome = 'D:\t\idf55'
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath (Join-Path $PythonHome 'python.exe'))) {
    throw 'Pass -PythonHome pointing to a Python 3.10+ base installation.'
}
$scaleBuildPath = $env:Path
$scaleBuildIdf = $env:IDF_PATH
try {
    $env:Path = "$PythonHome;$scaleBuildPath"
    # A global https->SSH Git rewrite confuses embuild's managed-repo URL check.
    # Reuse the SDK explicitly instead of letting it delete/reclone a valid checkout.
    if (-not (Test-Path -LiteralPath (Join-Path $IdfHome 'CMakeLists.txt'))) {
        throw 'Pass -IdfHome pointing to an ESP-IDF v5.5.3 checkout with its submodules.'
    }
    $env:IDF_PATH = $IdfHome
    Push-Location $PSScriptRoot
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "Firmware build failed: $LASTEXITCODE" }
    } finally { Pop-Location }
} finally { $env:Path = $scaleBuildPath; $env:IDF_PATH = $scaleBuildIdf }
