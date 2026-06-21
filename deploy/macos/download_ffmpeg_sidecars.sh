#!/bin/bash
# Downloads static ffmpeg and ffprobe binaries required by the pack-editor.
set -e

ARCH=$(uname -m)
if [ "$ARCH" = "x86_64" ]; then
  FFMPEG_URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/snapshot/ffmpeg.zip"
  FFPROBE_URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/snapshot/ffprobe.zip"
elif [ "$ARCH" = "arm64" ]; then
  FFMPEG_URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/snapshot/ffmpeg.zip"
  FFPROBE_URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/snapshot/ffprobe.zip"
else
  echo "Unsupported architecture: $ARCH"
  exit 1
fi

BINARIES_DIR="pack-editor/src-tauri/binaries"
mkdir -p "$BINARIES_DIR"

FFMPEG_SIDECAR="$BINARIES_DIR/lewdware-ffmpeg"
FFPROBE_SIDECAR="$BINARIES_DIR/lewdware-ffprobe"

if [ -f "$FFMPEG_SIDECAR" ] && [ -f "$FFPROBE_SIDECAR" ]; then
  echo "FFmpeg & ffprobe sidecars already present."
  exit 0
fi

echo "Downloading static FFmpeg/ffprobe binaries for macOS ($ARCH)..."
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

curl -L -s -o "$TEMP_DIR/ffmpeg.zip"  "$FFMPEG_URL"
curl -L -s -o "$TEMP_DIR/ffprobe.zip" "$FFPROBE_URL"

unzip -q "$TEMP_DIR/ffmpeg.zip"  -d "$TEMP_DIR/ffmpeg-dir"
unzip -q "$TEMP_DIR/ffprobe.zip" -d "$TEMP_DIR/ffprobe-dir"

cp "$(find "$TEMP_DIR/ffmpeg-dir"  -type f -name "ffmpeg"  | head -n 1)" "$FFMPEG_SIDECAR"
cp "$(find "$TEMP_DIR/ffprobe-dir" -type f -name "ffprobe" | head -n 1)" "$FFPROBE_SIDECAR"
chmod +x "$FFMPEG_SIDECAR" "$FFPROBE_SIDECAR"

echo "FFmpeg & ffprobe sidecars staged successfully."
