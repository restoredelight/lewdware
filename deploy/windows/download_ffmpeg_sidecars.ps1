# Downloads static ffmpeg and ffprobe binaries required by the pack-editor.

$ErrorActionPreference = "Stop"

$BINARIES_DIR = "pack-editor\src-tauri\binaries"
if (!(Test-Path $BINARIES_DIR)) {
    New-Item -ItemType Directory -Force -Path $BINARIES_DIR | Out-Null
}

$FFMPEG_SIDECAR = Join-Path $BINARIES_DIR "lewdware-ffmpeg.exe"
$FFPROBE_SIDECAR = Join-Path $BINARIES_DIR "lewdware-ffprobe.exe"

if ((Test-Path $FFMPEG_SIDECAR) -and (Test-Path $FFPROBE_SIDECAR)) {
    Write-Host "FFmpeg and ffprobe sidecars already present."
    exit 0
}

Write-Host "Downloading static FFmpeg and ffprobe for Windows..."

$TEMP_DIR   = Join-Path ([System.IO.Path]::GetTempPath()) "ffmpeg-sidecar-download"
$ZIP_PATH   = Join-Path $TEMP_DIR "ffmpeg.zip"
$EXTRACT_DIR = Join-Path $TEMP_DIR "extract"

if (Test-Path $TEMP_DIR) { Remove-Item -Recurse -Force $TEMP_DIR }
New-Item -ItemType Directory -Force -Path $TEMP_DIR | Out-Null

try {
    $BTBN_URL = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
    Invoke-WebRequest -Uri $BTBN_URL -OutFile $ZIP_PATH -UseBasicParsing

    Write-Host "Extracting FFmpeg archive..."
    Expand-Archive -Path $ZIP_PATH -DestinationPath $EXTRACT_DIR

    $FFMPEG_EXE  = Get-ChildItem -Path $EXTRACT_DIR -Filter "ffmpeg.exe"  -Recurse | Select-Object -First 1
    $FFPROBE_EXE = Get-ChildItem -Path $EXTRACT_DIR -Filter "ffprobe.exe" -Recurse | Select-Object -First 1

    if (!$FFMPEG_EXE -or !$FFPROBE_EXE) {
        Write-Error "Could not find ffmpeg.exe/ffprobe.exe in the downloaded archive."
    }

    Copy-Item $FFMPEG_EXE.FullName  -Destination $FFMPEG_SIDECAR  -Force
    Copy-Item $FFPROBE_EXE.FullName -Destination $FFPROBE_SIDECAR -Force
    Write-Host "FFmpeg and ffprobe sidecars staged successfully."
} finally {
    Remove-Item -Recurse -Force $TEMP_DIR -ErrorAction SilentlyContinue
}
