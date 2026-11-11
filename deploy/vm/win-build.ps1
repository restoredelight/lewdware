# Dev build inside the Windows test VM (run from a `vmctl sync`ed checkout).
#
#   vmctl ssh -- 'powershell -ExecutionPolicy Bypass -File C:\lw\lewdware\deploy\vm\win-build.ps1'                      # engine, supervisor, config
#   vmctl ssh -- 'powershell -ExecutionPolicy Bypass -File C:\lw\lewdware\deploy\vm\win-build.ps1 -Targets engine,pack-editor'
#
# Targets: engine, supervisor, config, pack-editor, lw.
# Everything lands in target\debug (or target\release with -Release), next to
# each other, which is where shared::binaries looks for sibling binaries.
# Toolchain (installed once): MSVC Build Tools + clang component, rustup,
# vcpkg at C:\vcpkg with pkgconf/dav1d/ffmpeg (x64-windows-release), Git.

param(
    [string[]]$Targets = @("engine", "supervisor", "config"),
    [switch]$Release,
    [switch]$SkipModes
)

$ErrorActionPreference = "Stop"
# `-File` passes "a,b" as one string rather than an array.
$Targets = $Targets -split "," | ForEach-Object { $_.Trim() } | Where-Object { $_ }
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

$vcpkg = "C:\vcpkg\installed\x64-windows-release"
$env:FFMPEG_DIR = $vcpkg
$env:PKG_CONFIG_PATH = "$vcpkg\lib\pkgconfig"
$env:PKG_CONFIG = "$vcpkg\tools\pkgconf\pkgconf.exe"
$env:LIBCLANG_PATH = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\Llvm\x64\bin"
$env:Path = @(
    "$vcpkg\bin", "C:\Program Files\Git\cmd", "$env:USERPROFILE\.cargo\bin",
    [Environment]::GetEnvironmentVariable("Path", "Machine"),
    [Environment]::GetEnvironmentVariable("Path", "User")
) -join ";"
$env:CARGO_TERM_COLOR = "never"

function Run([string]$exe, [string[]]$argv) {
    Write-Host ">> $exe $($argv -join ' ')"
    & $exe @argv
    if ($LASTEXITCODE) { throw "$exe failed with exit code $LASTEXITCODE" }
}

$profileArgs = if ($Release) { @("--release") } else { @() }
$outDir = Join-Path $root ("target\" + $(if ($Release) { "release" } else { "debug" }))

# The engine, config and shared embed the built default modes.
if (-not $SkipModes) {
    foreach ($m in "sandbox", "experience") {
        Push-Location "default-modes\$m"
        try { Run cargo @("run", "-q", "-p", "lw", "--", "mode", "build") } finally { Pop-Location }
    }
}

foreach ($t in $Targets) {
    switch ($t) {
        "engine"      { Run cargo (@("build", "-p", "lewdware") + $profileArgs) }
        "supervisor"  { Run cargo (@("build", "-p", "lewdware-supervisor") + $profileArgs) }
        "lw"          { Run cargo (@("build", "-p", "lw") + $profileArgs) }
        "config"      { Run cargo (@("build", "-p", "lewdware-config") + $profileArgs) }
        "pack-editor" { Run cargo (@("build", "-p", "lewdware-pack-editor") + $profileArgs) }
        default       { throw "unknown target '$t'" }
    }
}

# Runtime DLLs (ffmpeg, dav1d, ...) next to the binaries, so apps launched from
# the desktop work without PATH changes.
Copy-Item "$vcpkg\bin\*.dll" $outDir -Force
Write-Host "built: $($Targets -join ', ') -> $outDir"
exit 0
