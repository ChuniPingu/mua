param(
    [string]$Triplet = "x64-windows-static"
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

if ([string]::IsNullOrWhiteSpace($env:VCPKG_ROOT)) {
    throw "VCPKG_ROOT must point to a Microsoft vcpkg checkout"
}

$vcpkg = Join-Path $env:VCPKG_ROOT "vcpkg.exe"
if (-not (Test-Path -LiteralPath $vcpkg -PathType Leaf)) {
    throw "vcpkg executable not found: $vcpkg"
}

& $vcpkg install `
    "--x-manifest-root=$root" `
    "--x-install-root=$(Join-Path $env:VCPKG_ROOT 'installed')" `
    "--triplet=$Triplet"
if ($LASTEXITCODE -ne 0) {
    throw "vcpkg install failed with exit code $LASTEXITCODE"
}

