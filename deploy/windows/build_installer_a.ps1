# deploy/windows/build_installer_a.ps1
# Windows Build & Packaging script for Lewdware Main Suite (Installer A)

$ErrorActionPreference = "Stop"

$VERSION = (Select-String -Path "Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$STAGE_DIR = "build/win-stage"
$OUTPUT_DIR = "dist"

Write-Host "Preparing staging area..."
if (Test-Path $STAGE_DIR) { Remove-Item -Recurse -Force $STAGE_DIR }
New-Item -ItemType Directory -Force -Path $STAGE_DIR
New-Item -ItemType Directory -Force -Path $OUTPUT_DIR

# Download Visual C++ Redistributable for bundling into the installer (cached to avoid downloading on every build)
$vcRedistCacheDir = "build"
$vcRedistCache = "$vcRedistCacheDir\vc_redist.x64.exe"
$vcRedistDest = "$STAGE_DIR\vc_redist.x64.exe"

if (-not (Test-Path $vcRedistCacheDir)) {
    New-Item -ItemType Directory -Force -Path $vcRedistCacheDir
}

if (-not (Test-Path $vcRedistCache)) {
    # aka.ms redirects to a CDN that intermittently cuts the response short ("The response ended
    # prematurely"), which used to fail the whole build before a line of it had been compiled.
    # Retry rather than let a hiccup on someone else's server decide whether a release ships.
    $attempts = 3
    for ($attempt = 1; $attempt -le $attempts; $attempt++) {
        Write-Host "Downloading Visual C++ Redistributable (attempt $attempt/$attempts)..."
        try {
            Invoke-WebRequest -Uri "https://aka.ms/vs/17/release/vc_redist.x64.exe" -OutFile $vcRedistCache
            break
        } catch {
            # A part-written file would otherwise be taken for a good download by the check above.
            Remove-Item -Path $vcRedistCache -Force -ErrorAction SilentlyContinue
            if ($attempt -eq $attempts) {
                throw "Could not download the Visual C++ Redistributable: $_"
            }
            Start-Sleep -Seconds (5 * $attempt)
        }
    }
}

Write-Host "Staging Visual C++ Redistributable..."
Copy-Item $vcRedistCache -Destination $vcRedistDest

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

Write-Host "Building default modes..."
foreach ($mode in @("sandbox", "experience")) {
    Push-Location "default-modes\$mode"
    & "..\..\target\release\lw.exe" mode build
    Check-LastExitCode
    Pop-Location
}

cargo build -p lewdware --release
Check-LastExitCode

cargo build -p lewdware-supervisor --release
Check-LastExitCode

# Compile Tauri GUI. Only the raw target\release\lewdware.exe is used here - installer_a.iss
# packages it directly - so --no-bundle skips producing MSI/NSIS installers nobody consumes.
Write-Host "Building config GUI..."
Push-Location config
pnpm install
Check-LastExitCode
pnpm tauri build --no-bundle
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
    Copy-Item "$VCPKG_BIN_PATH\avdevice-*.dll" -Destination "target/release/"
    Copy-Item "$VCPKG_BIN_PATH\avfilter-*.dll" -Destination "target/release/"
    Copy-Item "$VCPKG_BIN_PATH\dav1d.dll" -Destination "target/release/"
} else {
    Write-Error "Vcpkg bin directory not found. Please make sure FFmpeg and dav1d DLLs are present."
}

# Verify that all DLL dependencies are present in target/release
$requiredDlls = @("avcodec", "avformat", "avutil", "swscale", "swresample", "avdevice", "avfilter", "dav1d")
foreach ($dllName in $requiredDlls) {
    $found = Get-ChildItem "target/release/" -Filter "*$dllName*.dll"
    if (-not $found) {
        Write-Error "Required DLL dependency '$dllName' is missing from target/release! Aborting build."
    }
}

# 3. Build the Installer using Inno Setup
Write-Host "Compiling Inno Setup installer..."
$isccCmd = Get-Command "iscc" -ErrorAction SilentlyContinue
if ($isccCmd) {
    $ISCC = $isccCmd.Source
} elseif (Test-Path "C:\Program Files (x86)\Inno Setup 6\ISCC.exe") {
    $ISCC = "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
} else {
    Write-Error "Inno Setup compiler (iscc) not found! Installer package could not be built."
    exit 1
}

& $ISCC "/DAppVersion=$VERSION" deploy\windows\installer_a.iss
Check-LastExitCode
Write-Host "SUCCESS: Installer created in dist/"
