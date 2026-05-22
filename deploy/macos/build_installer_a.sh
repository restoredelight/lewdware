#!/bin/bash
# deploy/macos/build_installer_a.sh
# macOS Build & Package script for Lewdware Main Suite (Installer A)
set -e

# Configuration
APP_NAME="Lewdware"
BUNDLE_ID="com.lewdware.suite"
VERSION="0.1.0"
BUILD_DIR="build/stage"
OUTPUT_DIR="dist"

echo "🧹 Preparing clean staging area..."
rm -rf "$BUILD_DIR" "$OUTPUT_DIR"
mkdir -p "$BUILD_DIR/root/Applications"
mkdir -p "$BUILD_DIR/scripts"
mkdir -p "$OUTPUT_DIR"

# 1. Compile all applications dynamically
echo "🔨 Compiling applications..."
cargo build -p lw --release

echo "🔨 Building default mode..."
(cd default-modes && ../target/release/lw mode build)

cargo build -p lewdware --release

# Compile Tauri GUI
echo "🔨 Building config-tauri GUI..."
cd config-tauri
pnpm install
pnpm tauri build
cd ..

# 2. Copy config.app package to our staging area
echo "📦 Staging config.app bundle..."
cp -R "target/release/bundle/macos/config-tauri.app" "$BUILD_DIR/root/Applications/Lewdware.app"

# Fix the bundle display name — productName in tauri.conf.json is still "config-tauri"
/usr/libexec/PlistBuddy -c "Set :CFBundleName Lewdware" \
  "$BUILD_DIR/root/Applications/Lewdware.app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName Lewdware" \
  "$BUILD_DIR/root/Applications/Lewdware.app/Contents/Info.plist"

# Rename internal binary to config-tauri if needed, ensuring the plist matches
# Tauri usually handles this. Let's make sure our CLI and Engine live in the same directory.
MAC_BIN_DIR="$BUILD_DIR/root/Applications/Lewdware.app/Contents/MacOS"
FRAMEWORKS_DIR="$BUILD_DIR/root/Applications/Lewdware.app/Contents/Frameworks"
mkdir -p "$FRAMEWORKS_DIR"

# Copy CLI and Engine into the bundle
cp "target/release/lw" "$MAC_BIN_DIR/lw"
cp "target/release/lewdware" "$MAC_BIN_DIR/lewdware"
chmod +x "$MAC_BIN_DIR/lw" "$MAC_BIN_DIR/lewdware"

# 3. Dynamic Library Bundling and Relinking (dylib)
echo "🔗 Resolving dynamic library dependencies (FFmpeg & dav1d)..."

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

bundle_dylib "$MAC_BIN_DIR/lewdware"
bundle_dylib "$MAC_BIN_DIR/lw"
bundle_dylib "$MAC_BIN_DIR/config-tauri"

# 4. Create the postinstall script for PATH integration
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
echo "📦 Wrapping into final installer..."
productbuild --package "$BUILD_DIR/LewdwareComponents.pkg" \
             "$OUTPUT_DIR/Lewdware-Installer-macOS.pkg"

echo "🎉 SUCCESS: $OUTPUT_DIR/Lewdware-Installer-macOS.pkg created!"
