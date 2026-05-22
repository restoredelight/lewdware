#!/bin/bash
# deploy/linux/build_installer_a.sh
# Linux Build & Debian Packaging script for Lewdware Main Suite (Installer A)
set -e

VERSION="0.1.0"
DEB_ARCH="amd64" # We can parameterize this or use `dpkg --print-architecture`
STAGE_DIR="build/deb-stage"
OUTPUT_DIR="dist"

echo "🧹 Preparing clean staging area..."
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/DEBIAN"
mkdir -p "$STAGE_DIR/usr/bin"
mkdir -p "$STAGE_DIR/usr/lib/lewdware"
mkdir -p "$STAGE_DIR/usr/share/applications"
mkdir -p "$STAGE_DIR/usr/share/icons/hicolor/128x128/apps"
mkdir -p "$OUTPUT_DIR"

# 1. Compile all applications
echo "🔨 Compiling applications..."
cargo build -p lw --release

echo "🔨 Building default mode..."
(cd default-modes && ../target/release/lw mode build)

# Compile lewdware with a relative rpath targeting the bundled libs
echo "   Compiling lewdware with relative rpath..."
cargo rustc -p lewdware --release -- -C link-args="-Wl,-rpath,\$ORIGIN/../lib/lewdware"

# Compile Tauri GUI
echo "🔨 Building config-tauri GUI..."
cd config-tauri
pnpm install
export APPIMAGE_EXTRACT_AND_RUN=1
export NO_STRIP=1
pnpm tauri build
cd ..

# 2. Stage binaries
echo "📦 Staging binaries..."
cp "target/release/config-tauri" "$STAGE_DIR/usr/bin/lewdware-config"
cp "target/release/lw" "$STAGE_DIR/usr/bin/lw"
cp "target/release/lewdware" "$STAGE_DIR/usr/bin/lewdware"
chmod +x "$STAGE_DIR/usr/bin/"*

# 3. Dynamic Library Copying (avoiding cross-distro package mismatches)
echo "🔗 Copying and staging dynamic library dependencies (FFmpeg & dav1d)..."

# Locate and copy a shared library from the system; aborts if not found.
copy_so() {
  local lib_name="$1"
  local lib_path
  lib_path=$(/sbin/ldconfig -p | grep -m 1 "$lib_name" | awk '{print $NF}')

  if [ -z "$lib_path" ]; then
    echo "❌ Required shared library $lib_name not found via ldconfig!" >&2
    exit 1
  fi
  echo "   Copying $lib_path -> usr/lib/lewdware/"
  cp "$lib_path" "$STAGE_DIR/usr/lib/lewdware/"
}

copy_so "libavcodec.so"
copy_so "libavformat.so"
copy_so "libavutil.so"
copy_so "libswscale.so"
copy_so "libswresample.so"
copy_so "libdav1d.so"
copy_so "libavfilter.so"
copy_so "libavdevice.so"

# 4. Create Desktop File and Icon
echo "📝 Creating desktop entries..."
cat <<EOF > "$STAGE_DIR/usr/share/applications/lewdware-config.desktop"
[Desktop Entry]
Name=Lewdware Configurator
Comment=Configure your Lewdware malware experience
Exec=lewdware-config
Icon=lewdware-config
Terminal=false
Type=Application
Categories=Utility;Development;
EOF

# Copy app icon if exists (use a placeholder if not)
if [ -f "config-tauri/src-tauri/icons/128x128.png" ]; then
  cp "config-tauri/src-tauri/icons/128x128.png" "$STAGE_DIR/usr/share/icons/hicolor/128x128/apps/lewdware-config.png"
fi

# 5. Create Debian Package control file
echo "📝 Creating Debian control file..."
cat <<EOF > "$STAGE_DIR/DEBIAN/control"
Package: lewdware-suite
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${DEB_ARCH}
Depends: libasound2, libx11-6, libxi6, libxtst6, libxrandr2, libxcursor1
Maintainer: restoredelight <restoreddelight@proton.me>
Description: Lewdware Main Suite (Client Engine, CLI tool and Config GUI)
EOF

# 6. Build the Debian Package
echo "📦 Building Debian package..."
dpkg-deb --build "$STAGE_DIR" "$OUTPUT_DIR/lewdware-suite_${VERSION}_${DEB_ARCH}.deb"
echo "✓ Debian package created!"

# 7. Build the RPM Package
if command -v rpmbuild &> /dev/null; then
  echo "📦 Building RPM package..."
  RPM_STAGE_DIR="build/rpm-stage"
  rm -rf "$RPM_STAGE_DIR"
  mkdir -p "$RPM_STAGE_DIR"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}

  cat <<EOF > "$RPM_STAGE_DIR/SPECS/lewdware-suite.spec"
%global __requires_exclude ^lib(avcodec|avformat|avutil|swscale|swresample|dav1d|avfilter|avdevice)\\.so
%global __provides_exclude ^lib(avcodec|avformat|avutil|swscale|swresample|dav1d|avfilter|avdevice)\\.so

Name:           lewdware-suite
Version:        ${VERSION}
Release:        1
Summary:        Lewdware Main Suite (Client Engine, CLI tool and Config GUI)
License:        MIT
Requires:       alsa-lib, libX11, libXi, libXtst, libXrandr, libXcursor

%description
Lewdware Main Suite containing the client engine, config GUI, and CLI tool.

%install
mkdir -p %{buildroot}/usr/bin
mkdir -p %{buildroot}/usr/lib/lewdware
mkdir -p %{buildroot}/usr/share/applications
mkdir -p %{buildroot}/usr/share/icons/hicolor/128x128/apps

cp -p %{staged_dir}/usr/bin/* %{buildroot}/usr/bin/
cp -pr %{staged_dir}/usr/lib/lewdware/* %{buildroot}/usr/lib/lewdware/
cp -p %{staged_dir}/usr/share/applications/* %{buildroot}/usr/share/applications/
cp -p %{staged_dir}/usr/share/icons/hicolor/128x128/apps/* %{buildroot}/usr/share/icons/hicolor/128x128/apps/

%files
/usr/bin/lewdware-config
/usr/bin/lw
/usr/bin/lewdware
/usr/lib/lewdware/*
/usr/share/applications/lewdware-config.desktop
/usr/share/icons/hicolor/128x128/apps/lewdware-config.png
EOF

  rpmbuild -bb \
    --define "_topdir $(pwd)/$RPM_STAGE_DIR" \
    --define "staged_dir $(pwd)/$STAGE_DIR" \
    "$RPM_STAGE_DIR/SPECS/lewdware-suite.spec"

  # Find generated RPM and copy to dist
  find "$RPM_STAGE_DIR/RPMS" -type f -name "*.rpm" -exec cp {} "$OUTPUT_DIR/" \;
  echo "✓ RPM package created!"
else
  echo "⚠️ Warning: rpmbuild not found, skipping RPM packaging."
fi

# 8. Build the portable tar.gz package
echo "📦 Building portable tar.gz package..."
TAR_STAGE="build/tar-stage"
TAR_ROOT="$TAR_STAGE/lewdware-suite-${VERSION}"
rm -rf "$TAR_STAGE"
mkdir -p "$TAR_ROOT/bin"
mkdir -p "$TAR_ROOT/lib/lewdware"

# Copy lewdware & lw binaries
cp "$STAGE_DIR/usr/bin/lewdware" "$TAR_ROOT/bin/"
cp "$STAGE_DIR/usr/bin/lw" "$TAR_ROOT/bin/"

# Copy config-tauri AppImage as lewdware-config binary
APPIMAGE_PATH="target/release/bundle/appimage/config-tauri_${VERSION}_amd64.AppImage"
if [ -f "$APPIMAGE_PATH" ]; then
  cp "$APPIMAGE_PATH" "$TAR_ROOT/bin/lewdware-config"
  chmod +x "$TAR_ROOT/bin/lewdware-config"
else
  echo "⚠️ Warning: config-tauri AppImage not found! Skipping config GUI in tar.gz."
fi

# Copy dynamic libraries
cp "$STAGE_DIR/usr/lib/lewdware/"* "$TAR_ROOT/lib/lewdware/"

# Create a simple setup/run README
cat << 'EOF' > "$TAR_ROOT/README.md"
# Lewdware Main Suite (Portable)

This portable distribution contains the Lewdware Engine, Config GUI, and helper CLI.

## Structure
* `bin/lewdware`: Lewdware Engine
* `bin/lw`: Lewdware CLI
* `bin/lewdware-config`: Config GUI (AppImage)
* `lib/lewdware/`: Bundled dynamic libraries (FFmpeg and dav1d)

## Running
Ensure you have the basic client dependencies installed on your Linux distribution (X11, ALSA, etc.).
Simply run the binaries from the `bin` directory:
```bash
./bin/lewdware-config
```
EOF

# Pack archive
tar -czf "$OUTPUT_DIR/lewdware-suite_${VERSION}_amd64.tar.gz" -C "$TAR_STAGE" "lewdware-suite-${VERSION}"
echo "✓ Portable tar.gz package created!"

echo "🎉 SUCCESS: All Linux target packages staged/created in $OUTPUT_DIR!"
