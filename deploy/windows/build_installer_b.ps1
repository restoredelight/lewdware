# deploy/windows/build_installer_b.ps1
# Windows Build & Packaging script for Lewdware Pack Editor (Installer B)

$ErrorActionPreference = "Stop"

# Helper to check exit code of native commands
function Check-LastExitCode {
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Command failed with exit code $LASTEXITCODE"
        exit $LASTEXITCODE
    }
}

$BINARIES_DIR = "pack-editor\src-tauri\binaries"

if (!(Test-Path $BINARIES_DIR)) {
    New-Item -ItemType Directory -Force -Path $BINARIES_DIR
}

$FFMPEG_SIDECAR = Join-Path $BINARIES_DIR "lewdware-ffmpeg.exe"
$FFPROBE_SIDECAR = Join-Path $BINARIES_DIR "lewdware-ffprobe.exe"

# 1. Stage FFmpeg & ffprobe if not already present
if (!(Test-Path $FFMPEG_SIDECAR) -or !(Test-Path $FFPROBE_SIDECAR)) {
    Write-Host "Downloading static FFmpeg and ffprobe for Windows..."

    $TEMP_DIR = [System.IO.Path]::GetTempPath()
    $ZIP_PATH = Join-Path $TEMP_DIR "ffmpeg-release-essentials.zip"
    $EXTRACT_DIR = Join-Path $TEMP_DIR "ffmpeg-extract"

    if (Test-Path $EXTRACT_DIR) { Remove-Item -Recurse -Force $EXTRACT_DIR }

    # Download BtbN static build
    $BTBN_URL = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
    Invoke-WebRequest -Uri $BTBN_URL -OutFile $ZIP_PATH -UseBasicParsing

    Write-Host "Extracting FFmpeg archive..."
    Expand-Archive -Path $ZIP_PATH -DestinationPath $EXTRACT_DIR

    $FFMPEG_EXE = Get-ChildItem -Path $EXTRACT_DIR -Filter "ffmpeg.exe" -Recurse | Select-Object -First 1
    $FFPROBE_EXE = Get-ChildItem -Path $EXTRACT_DIR -Filter "ffprobe.exe" -Recurse | Select-Object -First 1

    if ($FFMPEG_EXE -and $FFPROBE_EXE) {
        Copy-Item $FFMPEG_EXE.FullName -Destination $FFMPEG_SIDECAR -Force
        Copy-Item $FFPROBE_EXE.FullName -Destination $FFPROBE_SIDECAR -Force
        Write-Host "Stage successful: FFmpeg and ffprobe sidecars created."
    } else {
        Write-Error "Could not find ffmpeg.exe/ffprobe.exe in the extracted BtbN package."
    }

    # Clean up
    Remove-Item $ZIP_PATH -Force -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $EXTRACT_DIR -ErrorAction SilentlyContinue
} else {
    Write-Host "FFmpeg and ffprobe sidecars already present."
}

# 2. Build the Tauri app
Write-Host "Building pack-editor GUI..."
Push-Location pack-editor
pnpm install
Check-LastExitCode
pnpm tauri build
Check-LastExitCode
Pop-Location

# 3. Stage outputs
Write-Host "Staging outputs..."
if (!(Test-Path "dist")) { New-Item -ItemType Directory -Path "dist" }

$VERSION = (Select-String -Path "Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value

$INSTALLER = Get-ChildItem -Path "target\release\bundle" -Filter "lewdware-pack-editor*.msi" -Recurse | Select-Object -First 1
if ($INSTALLER) {
    $DEST = "dist\lewdware-pack-editor_${VERSION}_x86_64.msi"
    Copy-Item $INSTALLER.FullName -Destination $DEST -Force
    Write-Host "SUCCESS: Staged lewdware-pack-editor_${VERSION}_x86_64.msi in dist/"
} else {
    Write-Error "Could not find generated MSI package under target/release/bundle/"
}
