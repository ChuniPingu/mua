param(
    [string]$VcpkgRoot = $env:VCPKG_ROOT
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$target = Join-Path $root "vcpkg\ffmpeg"
$expectedTarget = [System.IO.Path]::GetFullPath((Join-Path $root "vcpkg\ffmpeg"))

if ([string]::IsNullOrWhiteSpace($VcpkgRoot)) {
    throw "VCPKG_ROOT must point to a Microsoft vcpkg checkout"
}

$vcpkgRootPath = [System.IO.Path]::GetFullPath($VcpkgRoot)
if (-not (Test-Path -LiteralPath (Join-Path $vcpkgRootPath ".git") -PathType Container)) {
    throw "vcpkg Git checkout not found: $vcpkgRootPath"
}

$configuration = Get-Content -LiteralPath (Join-Path $root "vcpkg-configuration.json") -Raw | ConvertFrom-Json
$baseline = $configuration."default-registry".baseline
& git -C $vcpkgRootPath cat-file -e "$baseline^{commit}"
if ($LASTEXITCODE -ne 0) {
    throw "pinned vcpkg baseline $baseline is absent from $vcpkgRootPath; fetch it before refreshing"
}

# This script intentionally replaces only the workspace-owned overlay directory.
if ([System.IO.Path]::GetFullPath($target) -ne $expectedTarget) {
    throw "refusing to replace unexpected path: $target"
}

$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$scratch = Join-Path $tempRoot ("mua-vcpkg-port-" + [Guid]::NewGuid().ToString("N"))
$archive = "$scratch.zip"
if (-not ([System.IO.Path]::GetFullPath($scratch)).StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "refusing to use unexpected temporary path: $scratch"
}
try {
    & git -C $vcpkgRootPath archive --format=zip --output=$archive $baseline ports/ffmpeg
    if ($LASTEXITCODE -ne 0) { throw "failed to export FFmpeg port at $baseline" }
    Expand-Archive -LiteralPath $archive -DestinationPath $scratch
    $upstream = Join-Path $scratch "ports\ffmpeg"

    if (Test-Path -LiteralPath $target) {
        Remove-Item -LiteralPath $target -Recurse -Force
    }
    Copy-Item -LiteralPath $upstream -Destination $target -Recurse -Force
}
finally {
    if (Test-Path -LiteralPath $scratch) {
        Remove-Item -LiteralPath $scratch -Recurse -Force
    }
    if (Test-Path -LiteralPath $archive) {
        Remove-Item -LiteralPath $archive -Force
    }
}

$patches = Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot "ffmpeg-port-overlay") -Filter "*.patch" -File | Sort-Object Name
if ($patches.Count -eq 0) {
    throw "no FFmpeg overlay patches were found"
}
foreach ($patch in $patches) {
    & git -C $root apply $patch.FullName
    if ($LASTEXITCODE -ne 0) {
        throw "failed to apply $($patch.Name)"
    }
}

Write-Host "Refreshed $target from pinned vcpkg baseline $baseline"
