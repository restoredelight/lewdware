#!/bin/bash
# deploy/macos/build_installer_a.sh
# macOS Build & Package script for Lewdware Main Suite (Installer A)
set -e

# Configuration
APP_NAME="Lewdware"
BUNDLE_ID="com.lewdware"
VERSION=$(grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
ARCH=$(uname -m)
BUILD_DIR="build/stage"
OUTPUT_DIR="dist"

echo "🧹 Preparing clean staging area..."
# Only the staging dir is wiped - dist/ is shared with build_installer_b.sh, so clearing it
# here would delete the pack editor's .dmg whenever the two scripts run in the other order.
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/root/Applications"
mkdir -p "$BUILD_DIR/scripts"
mkdir -p "$OUTPUT_DIR"

# 1. Compile all applications dynamically
echo "Compiling applications..."
cargo build -p lw --release

echo "Building default modes..."
for mode in sandbox experience; do
  (cd "default-modes/$mode" && ../../target/release/lw mode build)
done

# --features build-ffmpeg vendors and statically links FFmpeg from pristine
# upstream source (LGPL-only, no --enable-gpl/libx264/libx265) instead of
# linking Homebrew's ffmpeg, whose formula is declared GPL-3.0-or-later -
# see lewdware/Cargo.toml.
cargo build -p lewdware --release --features build-ffmpeg
cargo build -p lewdware-supervisor --release

# Compile Tauri GUI
echo "Building config GUI..."
cd config
pnpm install
pnpm tauri build --bundles app
cd ..

# 2. Copy config.app package to our staging area
echo "📦 Staging config.app bundle..."
# The bundle is named after productName in config/src-tauri/tauri.conf.json ("Lewdware");
# -iname keeps this working if that casing ever changes.
CONFIG_APP=$(find "target/release/bundle/macos" -maxdepth 1 -iname "lewdware.app" | head -n 1)
if [ -z "$CONFIG_APP" ]; then
  echo "Error: config .app bundle not found under target/release/bundle/macos/" >&2
  exit 1
fi
cp -R "$CONFIG_APP" "$BUILD_DIR/root/Applications/Lewdware.app"

# Ship the MIT license inside the app bundle - MIT's own terms require the
# copyright/permission notice to accompany copies of the software.
mkdir -p "$BUILD_DIR/root/Applications/Lewdware.app/Contents/Resources"
cp "LICENSE" "$BUILD_DIR/root/Applications/Lewdware.app/Contents/Resources/LICENSE"

# Rename internal binary to config-tauri if needed, ensuring the plist matches
# Tauri usually handles this. Let's make sure our CLI and Engine live in the same directory.
MAC_BIN_DIR="$BUILD_DIR/root/Applications/Lewdware.app/Contents/MacOS"
FRAMEWORKS_DIR="$BUILD_DIR/root/Applications/Lewdware.app/Contents/Frameworks"
mkdir -p "$FRAMEWORKS_DIR"

# Copy CLI, Supervisor, and Engine into the bundle
cp "target/release/lw" "$MAC_BIN_DIR/lw"
cp "target/release/lewdware-supervisor" "$MAC_BIN_DIR/lewdware-supervisor"
cp "target/release/lewdware-engine" "$MAC_BIN_DIR/lewdware-engine"
chmod +x "$MAC_BIN_DIR/lw" "$MAC_BIN_DIR/lewdware-supervisor" "$MAC_BIN_DIR/lewdware-engine"

# 3. Dynamic Library Bundling and Relinking (dylib)
# FFmpeg is statically linked into lewdware-engine (see the --features
# build-ffmpeg cargo invocation above), so it won't show up here. libdav1d
# will: the `image` crate's avif-native feature links Homebrew's, which is why
# the workflow installs it.
echo "Resolving dynamic library dependencies..."

# Recursively copy all non-system dylib deps of $1 into Frameworks/ and relink.
# Handles transitive deps (libvpx, libopus, libaom, etc.) automatically.
bundle_dylib() {
  local target="$1"
  while IFS= read -r dep; do
    case "$dep" in
      /usr/lib/* | /System/* | @* | "") continue ;;
    esac
    local lib_name
    lib_name=$(basename "$dep")
    local staged="$FRAMEWORKS_DIR/$lib_name"
    install_name_tool -change "$dep" "@executable_path/../Frameworks/$lib_name" "$target"
    if [ ! -f "$staged" ]; then
      echo "   Bundling $dep"
      cp "$dep" "$staged"
      chmod 755 "$staged"
      install_name_tool -id "@executable_path/../Frameworks/$lib_name" "$staged"
      bundle_dylib "$staged"
    fi
  done < <(otool -L "$target" | tail -n +2 | awk '{print $1}')
}

bundle_dylib "$MAC_BIN_DIR/lewdware-engine"
bundle_dylib "$MAC_BIN_DIR/lewdware-supervisor"
bundle_dylib "$MAC_BIN_DIR/lw"
bundle_dylib "$MAC_BIN_DIR/lewdware"

find "$FRAMEWORKS_DIR" -type f -name "*.dylib" -exec codesign --force --sign - {} \;

codesign --force --sign - "$MAC_BIN_DIR/lw"
codesign --force --sign - "$MAC_BIN_DIR/lewdware-supervisor"
codesign --force --sign - "$MAC_BIN_DIR/lewdware-engine"
codesign --force --sign - "$MAC_BIN_DIR/lewdware"
codesign --force --sign - "$BUILD_DIR/root/Applications/Lewdware.app"

# 4. Create the preinstall script, which shuts down a running install before it gets replaced
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
for image in lewdware-engine lewdware-supervisor lewdware; do
  pkill -x "$image" >/dev/null 2>&1 || true
done

# Never fail the install over this.
exit 0
EOF
chmod +x "$BUILD_DIR/scripts/preinstall"

# 4b. Create the postinstall script for PATH integration
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

# 5. Build the Component Package
echo "📦 Building component package..."
pkgbuild --root "$BUILD_DIR/root" \
         --scripts "$BUILD_DIR/scripts" \
         --identifier "$BUNDLE_ID" \
         --version "$VERSION" \
         --install-location / \
         "$BUILD_DIR/LewdwareComponents.pkg"

# 6. Build the Final Installer
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
