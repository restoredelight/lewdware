#!/bin/bash
# deploy/linux/build_flatpak.sh
# Flatpak packaging for Lewdware Pack Editor.
#
# The distro-independent Linux package. Unlike the AppImage it bundles no WebKitGTK of its own --
# it runs against the GNOME runtime's, which is built to work with the host's graphics drivers, so
# none of `shared/src/utils.rs`'s AppImage rendering workarounds apply and the editor's video
# preview works. See com.lewdware.pack-editor.yml.
#
# Reuses the .deb the Tauri bundler already produced (Tauri's own recommendation), so this stays in
# step with what the other Linux packages ship rather than restating the file layout.
set -e

MANIFEST_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$MANIFEST_DIR/../.." && pwd)"
cd "$REPO_ROOT"

APP_ID="com.lewdware.pack-editor"
VERSION=$(grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
case "$(uname -m)" in
  x86_64)        ARCH="x86_64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *)             ARCH="$(uname -m)" ;;
esac

command -v flatpak-builder >/dev/null || {
  echo "Error: flatpak-builder is not installed." >&2
  exit 1
}

# 1. Find the .deb. `build_installer_b.sh` stages one in dist/; a bare `pnpm tauri build` leaves it
#    under target/. Either will do -- but the manifest needs one fixed path, so stage a copy under
#    the name it names.
echo "🔎 Locating the pack editor .deb..."
STAGED_DEB="dist/lewdware-pack-editor.deb"
# Excluded from the search, or a staged copy left behind by an interrupted run becomes its own
# source on the next one.
trap 'rm -f "$STAGED_DEB"' EXIT
DEB=$(find dist target/release/bundle/deb -maxdepth 1 -type f \
  -iname "lewdware?pack?editor*.deb" ! -name "$(basename "$STAGED_DEB")" 2>/dev/null | head -1)
if [ -z "$DEB" ]; then
  echo "Error: no pack editor .deb found. Run deploy/linux/build_installer_b.sh (or" >&2
  echo "       'cd pack-editor && pnpm tauri build --bundles deb') first." >&2
  exit 1
fi
mkdir -p dist
cp "$DEB" "$STAGED_DEB"
echo "Using $DEB"

# 2. Build into a local repo, then export a single installable file. The bundle is the point: it
#    is what an AppImage user expects -- one download, `flatpak install ./that-file`, no remote to
#    add and no store account.
echo "🔨 Building the Flatpak..."
BUILD_DIR="build/flatpak"
rm -rf "$BUILD_DIR/build-dir" "$BUILD_DIR/repo"
mkdir -p "$BUILD_DIR"

flatpak-builder \
  --user \
  --force-clean \
  --install-deps-from=flathub \
  --repo="$BUILD_DIR/repo" \
  "$BUILD_DIR/build-dir" \
  "$MANIFEST_DIR/$APP_ID.yml"

echo "📦 Exporting the bundle..."
OUTPUT="dist/lewdware-pack-editor_${VERSION}_${ARCH}.flatpak"
flatpak build-bundle "$BUILD_DIR/repo" "$OUTPUT" "$APP_ID"

echo "SUCCESS: Staged $(basename "$OUTPUT") in dist/"
echo
echo "Install it with:  flatpak install --user $OUTPUT"
echo "Run it with:      flatpak run $APP_ID"
