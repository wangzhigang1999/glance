$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    foreach ($pageName in @('live', 'settings', 'logs', 'system')) {
        npx --yes --package tailwindcss@3.4.17 tailwindcss -c web/tailwind.config.cjs -i "web/styles/$pageName.css" -o "web/$pageName.css" --content './web/*.html' --minify
        if ($LASTEXITCODE -ne 0) { throw "CSS build failed: $pageName" }
    }
} finally { Pop-Location }
