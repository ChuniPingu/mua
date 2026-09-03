$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Push-Location $root
try {
    cargo test --workspace
    if ($LASTEXITCODE -ne 0) { throw "cargo test failed" }
}
finally {
    Pop-Location
}
