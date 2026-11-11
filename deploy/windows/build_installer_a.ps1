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

# Links FFmpeg statically from vcpkg's x64-windows-static-md triplet (the workflow installs it, with
# SVT-AV1, Opus, dav1d and libwebp), so there are no FFmpeg DLLs to ship.
cargo build -p lewdware --release --features static-ffmpeg
Check-LastExitCode

cargo build -p lewdware-supervisor --release
Check-LastExitCode

# The config app (config-gpui\), whose binary is lewdware.exe. It embeds the default modes built
# above, and its icon (build.rs).
Write-Host "Building config app..."
cargo build -p lewdware-config --release
Check-LastExitCode

# 2. Build the Installer using Inno Setup
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
