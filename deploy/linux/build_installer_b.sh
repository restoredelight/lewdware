#!/bin/bash
# deploy/linux/build_installer_b.sh
# Linux Build & Packaging script for Lewdware Pack Editor (Installer B)
set -e

# Detect architecture
ARCH=$(uname -m)
if [ "$ARCH" = "x86_64" ]; then
  TRIPLE="x86_64-unknown-linux-gnu"
  FFMPEG_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
elif [ "$ARCH" = "aarch64" ] || [ "$ARCH" = "arm64" ]; then
  TRIPLE="aarch64-unknown-linux-gnu"
  FFMPEG_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linuxarm64-gpl.tar.xz"
else
  echo "Unsupported architecture: $ARCH"
  exit 1
fi

BINARIES_DIR="pack-editor/src-tauri/binaries"
mkdir -p "$BINARIES_DIR"

FFMPEG_SIDECAR="$BINARIES_DIR/lewdware-ffmpeg-$TRIPLE"
FFPROBE_SIDECAR="$BINARIES_DIR/lewdware-ffprobe-$TRIPLE"

# 1. Fetch static FFmpeg and ffprobe if not already present
if [ ! -f "$FFMPEG_SIDECAR" ] || [ ! -f "$FFPROBE_SIDECAR" ]; then
  echo "Downloading static FFmpeg/ffprobe binaries for $TRIPLE..."
  TEMP_DIR=$(mktemp -d)

  wget -qO- "$FFMPEG_URL" | tar -xJ -C "$TEMP_DIR"

  cp "$(find "$TEMP_DIR" -type f -name "ffmpeg" | head -n 1)" "$FFMPEG_SIDECAR"
  cp "$(find "$TEMP_DIR" -type f -name "ffprobe" | head -n 1)" "$FFPROBE_SIDECAR"

  chmod +x "$FFMPEG_SIDECAR" "$FFPROBE_SIDECAR"
  rm -rf "$TEMP_DIR"
  echo "FFmpeg & ffprobe sidecars staged successfully."
else
  echo "FFmpeg & ffprobe sidecars already present."
fi

# 2. Build the Tauri app
echo "🔨 Building pack-editor-tauri GUI..."
cd pack-editor
pnpm install
export APPIMAGE_EXTRACT_AND_RUN=1
export NO_STRIP=1
pnpm tauri build
cd ..

# 3. Move output to dist
echo "Staging outputs..."
mkdir -p dist

# Look for generated Linux packages
copied_count=0
while read -r pkg; do
  if [ -n "$pkg" ]; then
    cp "$pkg" dist/
    echo "SUCCESS: Staged $(basename "$pkg") in dist/"
    copied_count=$((copied_count + 1))
  fi
done < <(find target/release/bundle/ -type f \( -name "lewdware-pack-editor*.deb" -o -name "lewdware-pack-editor*.rpm" -o -name "lewdware-pack-editor*.AppImage" \) 2>/dev/null)

if [ "$copied_count" -eq 0 ]; then
  echo "Error: No generated packages (.deb, .rpm, .AppImage) found under target/release/bundle/!" >&2
  exit 1
fi
