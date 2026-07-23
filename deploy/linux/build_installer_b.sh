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

# 1. Fetch static FFmpeg and ffprobe if not already present
"$(dirname "$0")/download_ffmpeg_sidecars.sh"

# 1b. Build the avifenc sidecar if not already present
"$(dirname "$0")/build_avifenc_sidecar.sh"

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

# The bundle file names come straight from tauri.conf.json's productName ("Lewdware Pack
# Editor"), which the bundler only sanitises for *package* names, not file names - so match
# case-insensitively and tolerate spaces/hyphens between the words.
stage() {
  local ext="$1"
  local src
  src=$(find target/release/bundle/ -type f -iname "lewdware?pack?editor*.$ext" 2>/dev/null | head -1)
  if [ -z "$src" ]; then
    echo "Error: no .$ext package found under target/release/bundle/!" >&2
    return 1
  fi
  cp "$src" "dist/lewdware-pack-editor_${VERSION}_${ARCH}.$ext"
  echo "SUCCESS: Staged lewdware-pack-editor_${VERSION}_${ARCH}.$ext in dist/"
}

# All three are advertised in docs/src/data/pack-editor-latest.json, so a missing one means a
# dead download link on the website - fail the build rather than publish a partial release.
stage deb
stage rpm
stage AppImage
