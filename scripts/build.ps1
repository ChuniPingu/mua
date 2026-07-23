$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

. (Join-Path $PSScriptRoot "import-build-environment.ps1")
& (Join-Path $PSScriptRoot "prepare-vcpkg.ps1")

$env:VCPKGRS_TRIPLET = "x64-windows-static"
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

    foreach ($binary in @("mua_wav", "mua_img")) {
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

    $legalOutput = Join-Path $publishRoot "legal"
    New-Item -ItemType Directory -Path $legalOutput -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $root "legal\NOTICE.md") -Destination $legalOutput -Force
    Copy-Item -LiteralPath (Join-Path $root "legal\FFMPEG-SOURCE-OFFER.md") -Destination $legalOutput -Force

    $ffmpegCopyright = Join-Path $env:VCPKG_ROOT "installed\x64-windows-static\share\ffmpeg\copyright"
    if (-not (Test-Path -LiteralPath $ffmpegCopyright -PathType Leaf)) {
        throw "FFmpeg copyright notice is missing: $ffmpegCopyright"
    }
    Copy-Item -LiteralPath $ffmpegCopyright -Destination (Join-Path $legalOutput "FFMPEG-COPYRIGHT.txt") -Force
}
finally {
    Pop-Location
}
