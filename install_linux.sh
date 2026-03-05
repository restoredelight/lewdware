#!/bin/bash
# Install the suite to ~/.local (standard for user-space installs)

INSTALL_DIR="$HOME/.local/share/lewdware"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"

echo "?? Installing Lewdware Suite..."

# 1. Create directories
mkdir -p "$INSTALL_DIR"
mkdir -p "$BIN_DIR"
mkdir -p "$APP_DIR"

# 2. Copy binaries
cp -r ./bin/* "$INSTALL_DIR/"

# 3. Create symlinks for the CLI tool
ln -sf "$INSTALL_DIR/lw" "$BIN_DIR/lw"

# 4. Create Desktop entries for the GUI apps
cat <<EOF > "$APP_DIR/lewdware.desktop"
[Desktop Entry]
Name=Lewdware
Exec=$INSTALL_DIR/lewdware
Type=Application
Icon=video-display
EOF

# Repeat for Config and Pack Editor...

echo "? Installation complete!"
echo "? 'lw' is now available in your terminal (ensure $BIN_DIR is in your PATH)."
