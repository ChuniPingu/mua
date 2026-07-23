$ErrorActionPreference = "Stop"
$selectedVcpkgRoot = $env:VCPKG_ROOT

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
    throw "vswhere.exe was not found; install Visual Studio 2022 C++ build tools"
}

$installation = (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath).Trim()
if ([string]::IsNullOrWhiteSpace($installation)) {
    throw "Visual Studio C++ x64 build tools were not found"
}

$developerCommand = Join-Path $installation "Common7\Tools\VsDevCmd.bat"
$environmentLines = & cmd.exe /s /c "`"$developerCommand`" -no_logo -arch=x64 -host_arch=x64 >nul && set"
if ($LASTEXITCODE -ne 0) {
    throw "failed to import the Visual Studio x64 build environment"
}
foreach ($line in $environmentLines) {
    $separator = $line.IndexOf('=')
    if ($separator -gt 0) {
        [Environment]::SetEnvironmentVariable($line.Substring(0, $separator), $line.Substring($separator + 1), "Process")
    }
}
if (-not [string]::IsNullOrWhiteSpace($selectedVcpkgRoot)) {
    $env:VCPKG_ROOT = $selectedVcpkgRoot
}

if (-not [string]::IsNullOrWhiteSpace($env:LIBCLANG_PATH) -and (Test-Path -LiteralPath (Join-Path $env:LIBCLANG_PATH "libclang.dll") -PathType Leaf)) {
    return
}

$libclangCandidates = @(
    (Join-Path $env:ProgramFiles "LLVM\bin"),
    (Join-Path $installation "VC\Tools\Llvm\x64\bin"),
    (Join-Path $installation "VC\Tools\Llvm\bin")
)
foreach ($candidate in $libclangCandidates) {
    if (Test-Path -LiteralPath (Join-Path $candidate "libclang.dll") -PathType Leaf) {
        $env:LIBCLANG_PATH = $candidate
        return
    }
}

throw "libclang.dll was not found; install LLVM and set LIBCLANG_PATH to its bin directory"
