$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

. (Join-Path $PSScriptRoot "import-build-environment.ps1")
& (Join-Path $PSScriptRoot "prepare-vcpkg.ps1")
$env:VCPKGRS_TRIPLET = "x64-windows-static"

Push-Location $root
try {
    cargo fmt --all --check
    if ($LASTEXITCODE -ne 0) { throw "cargo fmt failed" }

    cargo clippy --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "cargo clippy failed" }

    cargo test --workspace
    if ($LASTEXITCODE -ne 0) { throw "cargo test failed" }
}
finally {
    Pop-Location
}
