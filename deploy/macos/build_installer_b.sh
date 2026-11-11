#!/bin/bash
# deploy/macos/build_installer_b.sh
# macOS Build & Packaging script for Lewdware Pack Editor (Installer B): a .dmg holding
# "Lewdware Pack Editor.app" and an Applications link to drag it onto.
set -e
source "$(dirname "$0")/bundle.sh"

ARCH=$(uname -m)
VERSION=$(grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
APP_NAME="Lewdware Pack Editor"
BUILD_DIR="build/pack-editor-stage"
APP_DIR="$BUILD_DIR/dmg/$APP_NAME.app"

# 1. Stage FFmpeg and ffprobe if not already present
"$(dirname "$0")/download_ffmpeg_sidecars.sh"

# 1b. Build the avifenc sidecar if not already present
"$(dirname "$0")/build_avifenc_sidecar.sh"

# Static, LGPL-only FFmpeg with SVT-AV1, Opus, dav1d and libwebp (deploy/build_ffmpeg.sh; a no-op
# when build/ffmpeg-static is already up to date). Its pkg-config files go first on the path so
# that FFmpeg is what `--features static-ffmpeg` links, along with the libraries it needs, and so
# the `image` crate's AVIF decoder links the same static dav1d.
"$(dirname "$0")/../build_ffmpeg.sh" build/ffmpeg-static
export PKG_CONFIG_PATH="$PWD/build/ffmpeg-static/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

# 2. Build the editor, linking that FFmpeg (it previews and encodes with it) rather than Homebrew's
# GPL build. The sidecars above are separate GPL executables; see THIRD_PARTY_LICENSES.md.
echo "Building the pack editor..."
cargo build -p lewdware-pack-editor --release --features static-ffmpeg

# 3. Assemble the .app
echo "📦 Assembling $APP_NAME.app..."
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/dmg"
make_app_bundle "$APP_DIR" "target/release/lewdware-pack-editor" \
  lewdware-pack-editor "$APP_NAME" "com.lewdware.pack-editor" "$VERSION"

# The editor looks for its sidecars in Contents/Resources/binaries (pack-editor-gpui/src/main.rs,
# `sidecar_dirs`).
RESOURCES="$APP_DIR/Contents/Resources"
mkdir -p "$RESOURCES/binaries"
for bin in lewdware-ffmpeg lewdware-ffprobe lewdware-avifenc; do
  cp "pack-editor-gpui/binaries/$bin" "$RESOURCES/binaries/$bin"
  chmod 755 "$RESOURCES/binaries/$bin"
  # Apple Silicon won't run unsigned code. Keep a sidecar's own signature if it has one.
  codesign -v "$RESOURCES/binaries/$bin" 2>/dev/null || codesign --force --sign - "$RESOURCES/binaries/$bin"
done
cp "LICENSE" "$RESOURCES/LICENSE"
cp "pack-editor-gpui/THIRD_PARTY_LICENSES.md" "$RESOURCES/THIRD_PARTY_LICENSES.md"
cp "pack-editor-gpui/COPYING.GPLv3" "$RESOURCES/COPYING.GPLv3"

# FFmpeg and its codec libraries, dav1d included, are static (static-ffmpeg).
echo "Resolving dynamic library dependencies..."
bundle_dylibs "$APP_DIR" "$APP_DIR/Contents/MacOS/lewdware-pack-editor"
sign_app "$APP_DIR"

# 4. The .dmg
echo "📦 Building the disk image..."
ln -s /Applications "$BUILD_DIR/dmg/Applications"
mkdir -p dist
OUTPUT="dist/lewdware-pack-editor_${VERSION}_${ARCH}.dmg"
rm -f "$OUTPUT"
hdiutil create -volname "$APP_NAME" -srcfolder "$BUILD_DIR/dmg" -ov -format UDZO "$OUTPUT"
echo "SUCCESS: Staged $(basename "$OUTPUT") in dist/"
