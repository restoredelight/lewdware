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

# 1. Stage FFmpeg & ffprobe if not already present
& "$PSScriptRoot\download_ffmpeg_sidecars.ps1"

# 1b. Stage the avifenc sidecar if not already present
& "$PSScriptRoot\download_avifenc_sidecar.ps1"

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

# The .msi name comes from tauri.conf.json's productName ("Lewdware Pack Editor"), which the
# bundler does not sanitise for file names - match on a pattern that tolerates the spaces.
$INSTALLER = Get-ChildItem -Path "target\release\bundle" -Filter "*.msi" -Recurse |
    Where-Object { $_.Name -match '(?i)lewdware.pack.editor' } |
    Select-Object -First 1
if ($INSTALLER) {
    $DEST = "dist\lewdware-pack-editor_${VERSION}_x86_64.msi"
    Copy-Item $INSTALLER.FullName -Destination $DEST -Force
    Write-Host "SUCCESS: Staged lewdware-pack-editor_${VERSION}_x86_64.msi in dist/"
} else {
    Write-Error "Could not find generated MSI package under target/release/bundle/"
}
