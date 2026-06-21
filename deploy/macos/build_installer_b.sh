#!/bin/bash
# deploy/macos/build_installer_b.sh
# macOS Build & Packaging script for Lewdware Pack Editor (Installer B)
set -e

# Detect architecture
ARCH=$(uname -m)

# 1. stage FFmpeg and ffprobe if not already present
"$(dirname "$0")/download_ffmpeg_sidecars.sh"

# 2. Build the Tauri app
echo "Building pack-editor-tauri GUI..."
cd pack-editor
pnpm install
pnpm tauri build --bundles dmg
cd ..

# 3. Move output to dist
echo "Staging outputs..."
mkdir -p dist

VERSION=$(grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')

DMG_PATH=$(find target/release/bundle/dmg/ -name "lewdware-pack-editor*.dmg" 2>/dev/null | head -n 1)
if [ -n "$DMG_PATH" ]; then
  cp "$DMG_PATH" "dist/lewdware-pack-editor_${VERSION}_${ARCH}.dmg"
  echo "SUCCESS: Staged lewdware-pack-editor_${VERSION}_${ARCH}.dmg in dist/"
else
  echo "Error: Could not find generated .dmg package under target/release/bundle/dmg/" >&2
  exit 1
fi
