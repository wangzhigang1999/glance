param(
    [string]$PythonHome = (Join-Path $env:USERPROFILE '.cache\codex-runtimes\codex-primary-runtime\dependencies\python'),
    [string]$IdfHome = 'D:\t\idf55',
    [string]$TargetDir = 'D:\t\rlcd',
    [switch]$Public
)
$ErrorActionPreference = 'Stop'
$pythonExe = Join-Path $PythonHome 'python.exe'
if (-not (Test-Path -LiteralPath $pythonExe)) { throw 'Pass -PythonHome pointing to Python 3.10+.' }
if (-not (Test-Path -LiteralPath (Join-Path $IdfHome 'CMakeLists.txt'))) { throw 'Pass -IdfHome pointing to ESP-IDF v5.5.3.' }
$savedBuildEnv = @{}
foreach ($key in @('Path', 'IDF_PATH', 'CARGO_TARGET_DIR', 'LIBCLANG_PATH')) {
    $savedBuildEnv[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
}
try {
    $env:Path = "$PythonHome;$env:Path"
    $env:IDF_PATH = $IdfHome
    $env:CARGO_TARGET_DIR = $TargetDir
    if (-not $env:LIBCLANG_PATH) {
        $sysroot = (& rustc +esp --print sysroot).Trim()
        if ($LASTEXITCODE -ne 0) { throw 'Cannot locate esp toolchain.' }
        $env:LIBCLANG_PATH = Join-Path $sysroot 'xtensa-esp32-elf-clang\esp-clang\bin\libclang.dll'
    }
    $buildArgs = @((Join-Path $PSScriptRoot 'build.py'))
    if ($Public) { $buildArgs += '--public' }
    & $pythonExe @buildArgs
    if ($LASTEXITCODE -ne 0) { throw "Firmware build failed: $LASTEXITCODE" }
} finally {
    foreach ($key in $savedBuildEnv.Keys) {
        [Environment]::SetEnvironmentVariable($key, $savedBuildEnv[$key], 'Process')
    }
}
