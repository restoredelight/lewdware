# deploy/windows/build_installer_b.ps1
# Windows Build & Packaging script for Lewdware Pack Editor (Installer B): an Inno Setup installer,
# like the main suite's (installer_b.iss).

$ErrorActionPreference = "Stop"

# Helper to check exit code of native commands
function Check-LastExitCode {
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Command failed with exit code $LASTEXITCODE"
        exit $LASTEXITCODE
    }
}

$VERSION = (Select-String -Path "Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value

# 1. Stage the avifenc sidecar if not already present. Images are the one thing still encoded by a
# separate program; video, audio and probing go through the linked FFmpeg.
& "$PSScriptRoot\download_avifenc_sidecar.ps1"

# 1b. The Visual C++ Redistributable. build_installer_a.ps1 caches the same file; fetch it here
# only when this runs on its own.
$vcRedist = "build\vc_redist.x64.exe"
if (-not (Test-Path $vcRedist)) {
    New-Item -ItemType Directory -Force -Path "build" | Out-Null
    $attempts = 3
    for ($attempt = 1; $attempt -le $attempts; $attempt++) {
        Write-Host "Downloading Visual C++ Redistributable (attempt $attempt/$attempts)..."
        try {
            Invoke-WebRequest -Uri "https://aka.ms/vs/17/release/vc_redist.x64.exe" -OutFile $vcRedist
            break
        } catch {
            Remove-Item -Path $vcRedist -Force -ErrorAction SilentlyContinue
            if ($attempt -eq $attempts) {
                throw "Could not download the Visual C++ Redistributable: $_"
            }
            Start-Sleep -Seconds (5 * $attempt)
        }
    }
}

# 2. Build the editor, linking vcpkg's static FFmpeg (LGPL) as the engine does; the icon is
# embedded by build.rs.
Write-Host "Building the pack editor..."
cargo build -p lewdware-pack-editor --release --features static-ffmpeg
Check-LastExitCode

# 3. Build the installer
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

& $ISCC "/DAppVersion=$VERSION" deploy\windows\installer_b.iss
Check-LastExitCode
Write-Host "SUCCESS: Staged lewdware-pack-editor_${VERSION}_x86_64.exe in dist/"
