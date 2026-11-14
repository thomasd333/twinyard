# Builds the portable twinyard: dist-portable\twinyard\twinyard.exe (+ data\ on first start).
#   powershell -ExecutionPolicy Bypass -File scripts\build-portable.ps1            # program only
#   powershell -ExecutionPolicy Bypass -File scripts\build-portable.ps1 -WithData  # + copy of the dev data\
# -WithData copies config, favorites, connections and passwords (DPAPI: they decrypt only
# for the same Windows user). Logs and the WebView cache are not copied.
param([switch]$WithData)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location (Join-Path $root "app")
try {
    npx tauri build --no-bundle
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
} finally { Pop-Location }

$out = Join-Path $root "dist-portable\twinyard"
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item (Join-Path $root "target\release\twinyard.exe") $out -Force
Copy-Item (Join-Path $root "THIRD-PARTY-NOTICES.md") $out -Force  # UnRAR licence requires it
Copy-Item (Join-Path $root "LICENSE") (Join-Path $out "LICENSE.txt") -Force

if ($WithData) {
    $src = Join-Path $root "data"
    $dst = Join-Path $out "data"
    New-Item -ItemType Directory -Force $dst | Out-Null
    foreach ($f in "config.toml", "favorites.json", "connections.json", "passwords.json") {
        $p = Join-Path $src $f
        if (Test-Path $p) { Copy-Item $p $dst -Force }
    }
}
$exe = Get-Item (Join-Path $out "twinyard.exe")
"Gotowe: $($exe.FullName) ($([math]::Round($exe.Length / 1MB, 1)) MB)"
