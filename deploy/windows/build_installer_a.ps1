# deploy/windows/build_installer_a.ps1
# Windows Build & Packaging script for Lewdware Main Suite (Installer A)

$ErrorActionPreference = "Stop"

$VERSION = "0.1.0"
$STAGE_DIR = "build/win-stage"
$OUTPUT_DIR = "dist"

Write-Host "Preparing staging area..."
if (Test-Path $STAGE_DIR) { Remove-Item -Recurse -Force $STAGE_DIR }
New-Item -ItemType Directory -Force -Path $STAGE_DIR
New-Item -ItemType Directory -Force -Path $OUTPUT_DIR

# Helper to check exit code of native commands
function Check-LastExitCode {
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Command failed with exit code $LASTEXITCODE"
        exit $LASTEXITCODE
    }
}

# 1. Compile all applications
Write-Host "Compiling applications..."
cargo build -p lw --release
Check-LastExitCode

Write-Host "Building default mode..."
Push-Location default-modes
& "..\target\release\lw.exe" mode build
Check-LastExitCode
Pop-Location

cargo build -p lewdware --release
Check-LastExitCode

# Compile Tauri GUI
Write-Host "Building config-tauri GUI..."
Push-Location config-tauri
pnpm install
Check-LastExitCode
pnpm tauri build
Check-LastExitCode
Pop-Location

# 2. Dynamic Library Copying (vcpkg integration)
Write-Host "Locating and copying dynamic library dependencies (FFmpeg and dav1d)..."
$VCPKG_BIN_PATH = ""
if ($env:VCPKG_ROOT) {
    $VCPKG_BIN_PATH = Join-Path $env:VCPKG_ROOT "installed\x64-windows-release\bin"
} elseif (Test-Path "vcpkg") {
    $VCPKG_BIN_PATH = "vcpkg\installed\x64-windows-release\bin"
}

if ($VCPKG_BIN_PATH -and (Test-Path $VCPKG_BIN_PATH)) {
    Write-Host "   Copying DLLs from $VCPKG_BIN_PATH to target/release..."
    Copy-Item "$VCPKG_BIN_PATH\avcodec-*.dll" -Destination "target/release/"
    Copy-Item "$VCPKG_BIN_PATH\avformat-*.dll" -Destination "target/release/"
    Copy-Item "$VCPKG_BIN_PATH\avutil-*.dll" -Destination "target/release/"
    Copy-Item "$VCPKG_BIN_PATH\swscale-*.dll" -Destination "target/release/"
    Copy-Item "$VCPKG_BIN_PATH\swresample-*.dll" -Destination "target/release/"
    Copy-Item "$VCPKG_BIN_PATH\dav1d.dll" -Destination "target/release/"
} else {
    Write-Error "Vcpkg bin directory not found. Please make sure FFmpeg and dav1d DLLs are present."
}

# Verify that all DLL dependencies are present in target/release
$requiredDlls = @("avcodec", "avformat", "avutil", "swscale", "swresample", "dav1d")
foreach ($dllName in $requiredDlls) {
    $found = Get-ChildItem "target/release/" -Filter "*$dllName*.dll"
    if (-not $found) {
        Write-Error "Required DLL dependency '$dllName' is missing from target/release! Aborting build."
    }
}

# 3. Build the Installer using Inno Setup
Write-Host "Compiling Inno Setup installer..."
$ISCC = "iscc"
if (!(Get-Command $ISCC -ErrorAction SilentlyContinue)) {
    # Try default Inno Setup path
    $ISCC = "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
}

if (Test-Path $ISCC) {
    & $ISCC deploy\windows\installer_a.iss
    Check-LastExitCode
    Write-Host "SUCCESS: Installer created in dist/"
} else {
    Write-Error "Inno Setup compiler (iscc) not found! Installer package could not be built."
}
