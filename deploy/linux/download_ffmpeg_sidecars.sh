#!/bin/bash
# Downloads static ffmpeg and ffprobe binaries required by the pack-editor.
set -e

case "$(uname -m)" in
  x86_64)        ARCH="x86_64" ; FFMPEG_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz" ;;
  aarch64|arm64) ARCH="arm64"  ; FFMPEG_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linuxarm64-gpl.tar.xz" ;;
  *) echo "Unsupported architecture: $(uname -m)" ; exit 1 ;;
esac

BINARIES_DIR="pack-editor/src-tauri/binaries"
mkdir -p "$BINARIES_DIR"

FFMPEG_SIDECAR="$BINARIES_DIR/lewdware-ffmpeg"
FFPROBE_SIDECAR="$BINARIES_DIR/lewdware-ffprobe"

if [ -f "$FFMPEG_SIDECAR" ] && [ -f "$FFPROBE_SIDECAR" ]; then
  echo "FFmpeg & ffprobe sidecars already present."
  exit 0
fi

echo "Downloading static FFmpeg/ffprobe binaries for Linux ($ARCH)..."
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

wget -qO- "$FFMPEG_URL" | tar -xJ -C "$TEMP_DIR"

cp "$(find "$TEMP_DIR" -type f -name "ffmpeg"  | head -n 1)" "$FFMPEG_SIDECAR"
cp "$(find "$TEMP_DIR" -type f -name "ffprobe" | head -n 1)" "$FFPROBE_SIDECAR"
chmod +x "$FFMPEG_SIDECAR" "$FFPROBE_SIDECAR"

echo "FFmpeg & ffprobe sidecars staged successfully."
