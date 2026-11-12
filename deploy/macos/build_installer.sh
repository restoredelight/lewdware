#!/bin/bash
# deploy/macos/build_installer.sh
# Builds and packages Lewdware for macOS: one .pkg that installs two apps into /Applications.
#
#   Lewdware.app/Contents/
#     MacOS/lewdware                     the config app's launcher (the bundle's executable)
#     MacOS/lewdware-engine              the engine's launcher, started by the supervisor
#     MacOS/lewdware-supervisor          the supervisor's launcher
#     MacOS/lw                           the CLI, its own program
#     Frameworks/liblewdware_core.dylib  every app but lw, FFmpeg statically linked in
#   Lewdware Pack Editor.app/Contents/
#     MacOS/lewdware-pack-editor         the pack editor's launcher, which loads the core above
#     Resources/binaries/lewdware-avifenc
#
# The pack editor is its own app (its own Dock icon, menu bar name and bundle id) because its
# launcher is its bundle's executable; the code is in Lewdware.app once. Its launcher finds the
# core in the Lewdware.app beside it, or wherever LaunchServices says Lewdware.app is (launcher/).
set -e
source "$(dirname "$0")/bundle.sh"

# Configuration
APP_NAME="Lewdware"
BUNDLE_ID="com.lewdware"
EDITOR_NAME="Lewdware Pack Editor"
EDITOR_BUNDLE_ID="com.lewdware.pack-editor"
VERSION=$(grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
ARCH=$(uname -m)
BUILD_DIR="build/stage"
OUTPUT_DIR="dist"

echo "🧹 Preparing clean staging area..."
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/root/Applications"
mkdir -p "$BUILD_DIR/scripts"
mkdir -p "$OUTPUT_DIR"

# The pack editor's image encoder, the one thing it still runs as a separate program. A no-op when
# it is already built.
"$(dirname "$0")/build_avifenc_sidecar.sh"

# Static, LGPL-only FFmpeg with SVT-AV1, Opus, dav1d and libwebp (deploy/build_ffmpeg.sh; a no-op
# when build/ffmpeg-static is already up to date). Its pkg-config files go first on the path so
# that FFmpeg is what `--features static-ffmpeg` links, along with the libraries it needs, and so
# the `image` crate's AVIF decoder links the same static dav1d.
"$(dirname "$0")/../build_ffmpeg.sh" build/ffmpeg-static
export PKG_CONFIG_PATH="$PWD/build/ffmpeg-static/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

# 1. Compile all applications
echo "Compiling applications..."
cargo build -p lw --release

echo "Building default modes..."
for mode in sandbox experience; do
  (cd "default-modes/$mode" && ../../target/release/lw mode build)
done

# The shared core, linking the static FFmpeg built above rather than Homebrew's, whose formula is
# declared GPL-3.0-or-later (see lewdware/THIRD_PARTY_LICENSES.md), and the launchers. The config
# app, in the core, embeds the default modes built above.
echo "Building the shared core and the launchers..."
cargo build -p lewdware-core -p lewdware-launcher --release --features lewdware-core/static-ffmpeg

# 2. Assemble Lewdware.app. The launchers are built as launcher-<app> (launcher/Cargo.toml).
echo "📦 Assembling Lewdware.app..."
APP_DIR="$BUILD_DIR/root/Applications/$APP_NAME.app"
make_app_bundle "$APP_DIR" "target/release/launcher-config" \
  lewdware "$APP_NAME" "$BUNDLE_ID" "$VERSION"

MAC_BIN_DIR="$APP_DIR/Contents/MacOS"
cp "target/release/launcher-engine" "$MAC_BIN_DIR/lewdware-engine"
cp "target/release/launcher-supervisor" "$MAC_BIN_DIR/lewdware-supervisor"
cp "target/release/lw" "$MAC_BIN_DIR/lw"
chmod +x "$MAC_BIN_DIR/"*
cp "target/release/liblewdware_core.dylib" "$APP_DIR/Contents/Frameworks/liblewdware_core.dylib"

# Licences. MIT's own terms require its notice to accompany copies of the software; the LGPL
# requires its text to accompany the FFmpeg statically linked into the core; the OFL requires the
# same of the fonts the apps embed.
RESOURCES="$APP_DIR/Contents/Resources"
cp "LICENSE" "$RESOURCES/LICENSE"
cp "lewdware/THIRD_PARTY_LICENSES.md" "$RESOURCES/THIRD_PARTY_LICENSES.md"
cp "pack-editor-gpui/COPYING.LGPLv2.1" "$RESOURCES/COPYING.LGPLv2.1"
cp -r "assets/fonts/licenses" "$RESOURCES/fonts"

# 3. Assemble the pack editor's app
echo "📦 Assembling $EDITOR_NAME.app..."
EDITOR_DIR="$BUILD_DIR/root/Applications/$EDITOR_NAME.app"
make_app_bundle "$EDITOR_DIR" "target/release/launcher-pack-editor" \
  lewdware-pack-editor "$EDITOR_NAME" "$EDITOR_BUNDLE_ID" "$VERSION"

# The editor looks for the avifenc sidecar in its Contents/Resources/binaries
# (pack-editor-gpui/src/lib.rs, `sidecar_dirs`).
EDITOR_RESOURCES="$EDITOR_DIR/Contents/Resources"
AVIFENC="$EDITOR_RESOURCES/binaries/lewdware-avifenc"
mkdir -p "$EDITOR_RESOURCES/binaries"
cp "pack-editor-gpui/binaries/lewdware-avifenc" "$AVIFENC"
chmod 755 "$AVIFENC"
# Apple Silicon won't run unsigned code. Keep the sidecar's own signature if it has one.
codesign -v "$AVIFENC" 2>/dev/null || codesign --force --sign - "$AVIFENC"
cp "LICENSE" "$EDITOR_RESOURCES/LICENSE"
cp "pack-editor-gpui/THIRD_PARTY_LICENSES.md" "$EDITOR_RESOURCES/THIRD_PARTY_LICENSES.md"
cp "pack-editor-gpui/COPYING.LGPLv2.1" "$EDITOR_RESOURCES/COPYING.LGPLv2.1"

# 4. Dynamic Library Bundling and Relinking (dylib)
# FFmpeg and its codec libraries, dav1d included, are statically linked into the core (see the
# static-ffmpeg feature above), so they won't show up here.
echo "Resolving dynamic library dependencies..."
bundle_dylibs "$APP_DIR" "$APP_DIR/Contents/Frameworks/liblewdware_core.dylib"
for bin in lewdware lewdware-engine lewdware-supervisor lw; do
  bundle_dylibs "$APP_DIR" "$MAC_BIN_DIR/$bin"
done
bundle_dylibs "$EDITOR_DIR" "$EDITOR_DIR/Contents/MacOS/lewdware-pack-editor"

sign_app "$APP_DIR"
sign_app "$EDITOR_DIR"

# 5. Create the preinstall script, which shuts down a running install before it gets replaced
echo "📝 Creating installer preinstall script..."
cat << 'EOF' > "$BUILD_DIR/scripts/preinstall"
#!/bin/bash
# Stop a running Lewdware before the installer replaces its binaries underneath it.
SUPERVISOR="/Applications/Lewdware.app/Contents/MacOS/lewdware-supervisor"

# Installer scripts run as root, but on macOS the supervisor's control socket is a filesystem
# socket under env::temp_dir() (shared/src/ipc.rs -- the abstract namespace it uses on Linux
# isn't available here), which resolves to the *console user's* per-user $TMPDIR. Issuing the
# stop as root with root's TMPDIR would quietly connect to nothing, so point it at the console
# user's directory instead; root may open a socket it doesn't own.
CONSOLE_USER=$(stat -f "%Su" /dev/console 2>/dev/null)

if [ -x "$SUPERVISOR" ] && [ -n "$CONSOLE_USER" ] && [ "$CONSOLE_USER" != "root" ]; then
  USER_TMPDIR=$(sudo -u "$CONSOLE_USER" getconf DARWIN_USER_TEMP_DIR 2>/dev/null)
  if [ -n "$USER_TMPDIR" ]; then
    # Ends any live session cleanly: the engine tears its windows down and puts the desktop
    # wallpaper back on the way out.
    TMPDIR="$USER_TMPDIR" "$SUPERVISOR" stop >/dev/null 2>&1 || true
    sleep 2
  fi
fi

# Then make sure nothing is left holding the old bundle open. There is no IPC request that shuts
# the daemon itself down (only StopSession), so the supervisor has to be killed outright. If the
# graceful stop above didn't land, the wallpaper snapshot persisted in
# supervisor/src/wallpaper.rs means the next start restores the desktop anyway.
#
# A running pack editor is left alone: it may have unsaved work, and it keeps the old core it
# already loaded until it is closed.
for image in lewdware-engine lewdware-supervisor lewdware; do
  pkill -x "$image" >/dev/null 2>&1 || true
done

# Never fail the install over this.
exit 0
EOF
chmod +x "$BUILD_DIR/scripts/preinstall"

# 5b. Create the postinstall script for PATH integration
echo "📝 Creating installer postinstall script..."
cat << 'EOF' > "$BUILD_DIR/scripts/postinstall"
#!/bin/bash
# Path to the internal CLI binary
TARGET_BIN="/Applications/Lewdware.app/Contents/MacOS/lw"
# Path to the symlink we want to create
LINK_PATH="/usr/local/bin/lw"

echo "Setting up CLI symlink in /usr/local/bin..."
mkdir -p /usr/local/bin
ln -sf "$TARGET_BIN" "$LINK_PATH"
chmod +x "$TARGET_BIN"

echo "Resetting TCC permissions..."
tccutil reset All com.lewdware || true

exit 0
EOF
chmod +x "$BUILD_DIR/scripts/postinstall"

# 6. Build the Component Package
#
# Neither app is relocatable. By default Installer updates a bundle wherever it finds one with the
# same id, so a pack editor from the old .dmg left in ~/Downloads would be updated there, away
# from Lewdware.app; and the scripts above name /Applications/Lewdware.app.
echo "📦 Building component package..."
COMPONENTS="$BUILD_DIR/components.plist"
pkgbuild --analyze --root "$BUILD_DIR/root" "$COMPONENTS"
# An array with one entry per app.
i=0
while /usr/libexec/PlistBuddy -c "Print :$i" "$COMPONENTS" >/dev/null 2>&1; do
  /usr/libexec/PlistBuddy -c "Set :$i:BundleIsRelocatable false" "$COMPONENTS"
  i=$((i + 1))
done
if [[ $i -lt 2 ]] || plutil -p "$COMPONENTS" | grep -q '"BundleIsRelocatable" => 1'; then
  echo "Error: couldn't make the apps non-relocatable in $COMPONENTS" >&2
  exit 1
fi

pkgbuild --root "$BUILD_DIR/root" \
         --component-plist "$COMPONENTS" \
         --scripts "$BUILD_DIR/scripts" \
         --identifier "$BUNDLE_ID" \
         --version "$VERSION" \
         --install-location / \
         "$BUILD_DIR/LewdwareComponents.pkg"

# 7. Build the Final Installer
#
# This goes through a distribution file rather than `productbuild --package` directly. The
# distribution productbuild synthesizes for a bare --package has no <title>, and Installer.app
# uses that title both for its window and for the "Do you want to move the ... Installer to the
# Trash?" prompt shown after installing - with no title, that prompt names an empty string.
echo "📝 Synthesizing installer distribution..."
DIST_FILE="$BUILD_DIR/distribution.xml"
productbuild --synthesize --package "$BUILD_DIR/LewdwareComponents.pkg" "$DIST_FILE"

awk -v title="$APP_NAME" '
  { print }
  /<installer-gui-script/ && !done { print "    <title>" title "</title>"; done = 1 }
' "$DIST_FILE" > "$DIST_FILE.tmp"
mv "$DIST_FILE.tmp" "$DIST_FILE"

grep -q "<title>$APP_NAME</title>" "$DIST_FILE" || {
  echo "Error: failed to inject <title> into $DIST_FILE" >&2
  exit 1
}

echo "📦 Wrapping into final installer..."
productbuild --distribution "$DIST_FILE" \
             --package-path "$BUILD_DIR" \
             "$OUTPUT_DIR/lewdware_${VERSION}_${ARCH}.pkg"

echo "SUCCESS: $OUTPUT_DIR/lewdware_${VERSION}_${ARCH}.pkg created!"
