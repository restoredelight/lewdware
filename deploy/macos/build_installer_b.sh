#!/bin/bash
# deploy/macos/build_installer_b.sh
# macOS Build & Packaging script for Lewdware Pack Editor (Installer B)
set -e

# Detect architecture
ARCH=$(uname -m)
if [ "$ARCH" = "x86_64" ]; then
  TRIPLE="x86_64-apple-darwin"
  FFMPEG_URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/snapshot/ffmpeg.zip"
  FFPROBE_URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/snapshot/ffprobe.zip"
elif [ "$ARCH" = "arm64" ]; then
  TRIPLE="aarch64-apple-darwin"
  FFMPEG_URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/snapshot/ffmpeg.zip"
  FFPROBE_URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/snapshot/ffprobe.zip"
else
  echo "Unsupported architecture: $ARCH"
  exit 1
fi

BINARIES_DIR="pack-editor/src-tauri/binaries"
mkdir -p "$BINARIES_DIR"

FFMPEG_SIDECAR="$BINARIES_DIR/lewdware-ffmpeg-$TRIPLE"
FFPROBE_SIDECAR="$BINARIES_DIR/lewdware-ffprobe-$TRIPLE"

# 1. stage FFmpeg and ffprobe if not already present
if [ ! -f "$FFMPEG_SIDECAR" ] || [ ! -f "$FFPROBE_SIDECAR" ]; then
  echo "Downloading static FFmpeg/ffprobe binaries for macOS ($ARCH)..."
  TEMP_DIR=$(mktemp -d)

  curl -L -s -o "$TEMP_DIR/ffmpeg.zip" "$FFMPEG_URL"
  curl -L -s -o "$TEMP_DIR/ffprobe.zip" "$FFPROBE_URL"

  unzip -q "$TEMP_DIR/ffmpeg.zip" -d "$TEMP_DIR/ffmpeg-dir"
  unzip -q "$TEMP_DIR/ffprobe.zip" -d "$TEMP_DIR/ffprobe-dir"

  cp "$(find "$TEMP_DIR/ffmpeg-dir" -type f -name "ffmpeg" | head -n 1)" "$FFMPEG_SIDECAR"
  cp "$(find "$TEMP_DIR/ffprobe-dir" -type f -name "ffprobe" | head -n 1)" "$FFPROBE_SIDECAR"

  chmod +x "$FFMPEG_SIDECAR" "$FFPROBE_SIDECAR"
  rm -rf "$TEMP_DIR"
  echo "FFmpeg & ffprobe sidecars staged successfully."
else
  echo "FFmpeg & ffprobe sidecars already present."
fi

# 2. Build the Tauri app
echo "Building pack-editor-tauri GUI..."
cd pack-editor
pnpm install
pnpm tauri build --bundles dmg
cd ..

# 3. Move output to dist
echo "Staging outputs..."
mkdir -p dist

# Look for generated dmg or app package
DMG_PATH=$(find target/release/bundle/dmg/ -name "lewdware-pack-editor*.dmg" 2>/dev/null | head -n 1)
if [ -n "$DMG_PATH" ]; then
  cp "$DMG_PATH" dist/
  echo "SUCCESS: Generated $(basename "$DMG_PATH") in dist/"
else
  echo "Error: Could not find generated .dmg package under target/release/bundle/dmg/" >&2
  exit 1
fi
