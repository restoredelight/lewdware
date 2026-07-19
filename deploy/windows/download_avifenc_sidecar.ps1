# Downloads the prebuilt static avifenc.exe required by the pack-editor's image encode pipeline.
# libavif publishes an official static Windows build per release, unlike Linux/macOS which have no
# prebuilt static binary and are built from source instead (see build_avifenc_sidecar.sh).

$ErrorActionPreference = "Stop"

$LIBAVIF_TAG = "v1.4.2"

$BINARIES_DIR = "pack-editor\src-tauri\binaries"
if (!(Test-Path $BINARIES_DIR)) {
    New-Item -ItemType Directory -Force -Path $BINARIES_DIR | Out-Null
}

$AVIFENC_SIDECAR = Join-Path $BINARIES_DIR "lewdware-avifenc.exe"

if (Test-Path $AVIFENC_SIDECAR) {
    Write-Host "avifenc sidecar already present."
    exit 0
}

Write-Host "Downloading static avifenc for Windows (libavif $LIBAVIF_TAG)..."

$TEMP_DIR = Join-Path ([System.IO.Path]::GetTempPath()) "avifenc-sidecar-download"
$ZIP_PATH = Join-Path $TEMP_DIR "windows-artifacts.zip"
$EXTRACT_DIR = Join-Path $TEMP_DIR "extract"

if (Test-Path $TEMP_DIR) { Remove-Item -Recurse -Force $TEMP_DIR }
New-Item -ItemType Directory -Force -Path $TEMP_DIR | Out-Null

try {
    $URL = "https://github.com/AOMediaCodec/libavif/releases/download/$LIBAVIF_TAG/windows-artifacts.zip"
    Invoke-WebRequest -Uri $URL -OutFile $ZIP_PATH -UseBasicParsing

    Write-Host "Extracting libavif Windows artifacts..."
    Expand-Archive -Path $ZIP_PATH -DestinationPath $EXTRACT_DIR

    $AVIFENC_EXE = Get-ChildItem -Path $EXTRACT_DIR -Filter "avifenc.exe" -Recurse | Select-Object -First 1
    if (!$AVIFENC_EXE) {
        Write-Error "Could not find avifenc.exe in the downloaded archive."
    }

    Copy-Item $AVIFENC_EXE.FullName -Destination $AVIFENC_SIDECAR -Force
    Write-Host "avifenc sidecar staged successfully."
} finally {
    Remove-Item -Recurse -Force $TEMP_DIR -ErrorAction SilentlyContinue
}
