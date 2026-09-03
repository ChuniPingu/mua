$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Push-Location $root
try {
    cargo build --workspace --release
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }

    $publishRoot = Join-Path $root "target\release\mua"
    if (Test-Path -LiteralPath $publishRoot) {
        Remove-Item -LiteralPath $publishRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $publishRoot -Force | Out-Null

    foreach ($binary in @("mua_img")) {
        $source = Join-Path $root "target\release\$binary.exe"
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            $source = Join-Path $root "target\release\$binary"
        }
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Release binary is missing: $binary"
        }
        Copy-Item -LiteralPath $source -Destination $publishRoot -Force
    }

    Copy-Item -LiteralPath (Join-Path $root "LICENSE-MIT") -Destination $publishRoot -Force
    Copy-Item -LiteralPath (Join-Path $root "LICENSE-APACHE") -Destination $publishRoot -Force
}
finally {
    Pop-Location
}
