#!/bin/bash
set -e

# Configuration
APP_NAME="Lewdware"
BUNDLE_ID="com.lewdware.suite"
VERSION="0.1.0"
DIST_DIR="dist" # Change this to your actual build output folder
BUILD_DIR="build/pkg"

echo "?? Preparing staging area..."
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/root/Applications"
mkdir -p "$BUILD_DIR/scripts"

# 1. Copy the GUI .app bundles
# Adjust these paths to match where your build system puts the .app folders
echo "?? Copying .app bundles..."
cp -R "$DIST_DIR/lewdware.app" "$BUILD_DIR/root/Applications/Lewdware.app"
cp -R "$DIST_DIR/config.app" "$BUILD_DIR/root/Applications/Lewdware Config.app"
cp -R "$DIST_DIR/pack-editor.app" "$BUILD_DIR/root/Applications/Lewdware Pack Editor.app"

# 2. Embed the 'lw' binary into the main app bundle
# This keeps the CLI tool inside the app's internal folder
echo "?? Embedding CLI binary..."
cp "$DIST_DIR/lw" "$BUILD_DIR/root/Applications/Lewdware.app/Contents/MacOS/lw"
chmod +x "$BUILD_DIR/root/Applications/Lewdware.app/Contents/MacOS/lw"

# 3. Create the postinstall script
echo "?? Creating postinstall script..."
cat << 'EOF' > "$BUILD_DIR/scripts/postinstall"
#!/bin/bash

# Path to the internal CLI binary
TARGET_BIN="/Applications/Lewdware.app/Contents/MacOS/lw"
# Path to the symlink we want to create
LINK_PATH="/usr/local/bin/lw"

echo "Setting up CLI symlink..."

# Ensure /usr/local/bin exists
mkdir -p /usr/local/bin

# Create/Overwrite the symlink
ln -sf "$TARGET_BIN" "$LINK_PATH"

# Ensure the binary remains executable
chmod +x "$TARGET_BIN"

exit 0
EOF

chmod +x "$BUILD_DIR/scripts/postinstall"

# 4. Build the Component Package
echo "?? Building component package..."
pkgbuild --root "$BUILD_DIR/root" \
         --scripts "$BUILD_DIR/scripts" \
         --identifier "$BUNDLE_ID" \
         --version "$VERSION" \
         --install-location / \
         "$BUILD_DIR/LewdwareComponents.pkg"

# 5. Build the Final Installer
echo "?? Wrapping into final installer..."
productbuild --package "$BUILD_DIR/LewdwareComponents.pkg" \
             "LewdwareInstaller.pkg"

echo "? SUCCESS: LewdwareInstaller.pkg created!"
echo "? Note: Since this is unsigned, users must Right-Click -> Open to install."
