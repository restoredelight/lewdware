#!/bin/bash
# deploy/linux/build_installer_b.sh
# Linux Build & Packaging script for Lewdware Pack Editor (Installer B)
set -e

# Detect architecture
case "$(uname -m)" in
  x86_64)       ARCH="x86_64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *)             ARCH="$(uname -m)" ;;
esac
if [ "$ARCH" = "x86_64" ]; then
  FFMPEG_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
elif [ "$ARCH" = "arm64" ]; then
  FFMPEG_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linuxarm64-gpl.tar.xz"
else
  echo "Unsupported architecture: $ARCH"
  exit 1
fi

BINARIES_DIR="pack-editor/src-tauri/binaries"
mkdir -p "$BINARIES_DIR"

FFMPEG_SIDECAR="$BINARIES_DIR/lewdware-ffmpeg"
FFPROBE_SIDECAR="$BINARIES_DIR/lewdware-ffprobe"

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

VERSION=$(grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')

copied_count=0

DEB_PATH=$(find target/release/bundle/ -type f -name "lewdware-pack-editor*.deb" 2>/dev/null | head -1)
if [ -n "$DEB_PATH" ]; then
  cp "$DEB_PATH" "dist/lewdware-pack-editor_${VERSION}_${ARCH}.deb"
  echo "SUCCESS: Staged lewdware-pack-editor_${VERSION}_${ARCH}.deb in dist/"
  copied_count=$((copied_count + 1))
fi

RPM_PATH=$(find target/release/bundle/ -type f -name "lewdware-pack-editor*.rpm" 2>/dev/null | head -1)
if [ -n "$RPM_PATH" ]; then
  cp "$RPM_PATH" "dist/lewdware-pack-editor_${VERSION}_${ARCH}.rpm"
  echo "SUCCESS: Staged lewdware-pack-editor_${VERSION}_${ARCH}.rpm in dist/"
  copied_count=$((copied_count + 1))
fi

APPIMAGE_PATH=$(find target/release/bundle/ -type f -name "lewdware-pack-editor*.AppImage" 2>/dev/null | head -1)
if [ -n "$APPIMAGE_PATH" ]; then
  cp "$APPIMAGE_PATH" "dist/lewdware-pack-editor_${VERSION}_${ARCH}.AppImage"
  echo "SUCCESS: Staged lewdware-pack-editor_${VERSION}_${ARCH}.AppImage in dist/"
  copied_count=$((copied_count + 1))
fi

if [ "$copied_count" -eq 0 ]; then
  echo "Error: No generated packages (.deb, .rpm, .AppImage) found under target/release/bundle/!" >&2
  exit 1
fi
