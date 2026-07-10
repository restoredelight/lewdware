#!/bin/bash
# deploy/linux/build_installer_a.sh
# Linux Build & Debian Packaging script for Lewdware Main Suite (Installer A)
set -e

VERSION=$(grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
DEB_ARCH=$(dpkg --print-architecture)
case "$(uname -m)" in
  x86_64)       ARCH="x86_64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *)             ARCH="$(uname -m)" ;;
esac
STAGE_DIR="build/deb-stage"
OUTPUT_DIR="dist"

echo "🧹 Preparing clean staging area..."
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/DEBIAN"
mkdir -p "$STAGE_DIR/usr/bin"
mkdir -p "$STAGE_DIR/usr/lib/lewdware"
mkdir -p "$STAGE_DIR/usr/share/applications"
mkdir -p "$STAGE_DIR/usr/share/icons/hicolor/128x128/apps"
mkdir -p "$STAGE_DIR/usr/share/doc/lewdware"
mkdir -p "$OUTPUT_DIR"

# Ship the MIT license alongside the binaries - MIT's own terms require the
# copyright/permission notice to accompany copies of the software.
cp "LICENSE" "$STAGE_DIR/usr/share/doc/lewdware/copyright"

# 1. Compile all applications
echo "🔨 Compiling applications..."
cargo build -p lw --release

echo "🔨 Building default mode..."
(cd default-modes && ../target/release/lw mode build)

# Compile lewdware with a relative rpath targeting the bundled libs.
# --features build-ffmpeg vendors and statically links FFmpeg from pristine
# upstream source (LGPL-only, no --enable-gpl/libx264) instead of linking
# Ubuntu's GPL-licensed libavcodec.so etc - see lewdware/Cargo.toml.
echo "   Compiling lewdware with relative rpath..."
cargo rustc -p lewdware --release --features build-ffmpeg -- -C link-args="-Wl,-rpath,\$ORIGIN/../lib/lewdware"

# Compile Tauri GUI
echo "🔨 Building config GUI..."
cd config
pnpm install
export APPIMAGE_EXTRACT_AND_RUN=1
export NO_STRIP=1
pnpm tauri build
cd ..

# 2. Stage binaries
echo "Staging binaries..."
cp "target/release/lewdware" "$STAGE_DIR/usr/bin/lewdware"
cp "target/release/lw" "$STAGE_DIR/usr/bin/lw"
cp "target/release/lewdware-engine" "$STAGE_DIR/usr/lib/lewdware/lewdware-engine"
chmod +x "$STAGE_DIR/usr/bin/"* "$STAGE_DIR/usr/lib/lewdware/lewdware-engine"

# 3. Dynamic Library Bundling (transitive deps of any dynamically-linked libs).
# FFmpeg and dav1d are statically linked into lewdware-engine (see the
# --features build-ffmpeg cargo invocation above), so they won't show up here.
echo "Bundling dynamic library dependencies..."

# System libraries that must remain as host deps (UI, audio, core runtime).
is_system_lib() {
  local lib="$1"
  case "$lib" in
    libc.so* | libm.so* | libdl.so* | libpthread.so* | librt.so* | \
    libgcc_s.so* | libstdc++.so* | ld-linux* | libz.so* | \
    libGL.so* | libGLX.so* | libEGL.so* | libvulkan.so* | \
    libX11.so* | libXext.so* | libXrender.so* | libXi.so* | libXtst.so* | \
    libXrandr.so* | libXcursor.so* | libXdamage.so* | libXfixes.so* | \
    libXcomposite.so* | libXau.so* | libXdmcp.so* | libxcb*.so* | \
    libwayland-*.so* | libxkbcommon*.so* | \
    libgtk-3.so* | libgdk-3.so* | libgtk-4.so* | libgdk-4.so* | \
    libglib-2.0.so* | libgobject-2.0.so* | libgio-2.0.so* | libgmodule-2.0.so* | \
    libpango*.so* | libcairo*.so* | libatk*.so* | libepoxy.so* | \
    libharfbuzz.so* | libfontconfig.so* | libfreetype.so* | libpixman-1.so* | \
    libasound.so* | libpulse*.so* | libpipewire*.so* | \
    libwebkit2gtk*.so* | libjavascriptcoregtk*.so* | libsoup*.so* | \
    libdbus-1.so* | libsystemd.so* | libudev.so* | \
    libmount.so* | libblkid.so* | libuuid.so* | libpcre2*.so* | libffi.so* | \
    libexpat.so* | libselinux.so* | libssl.so* | libcrypto.so*)
      return 0 ;;
    *) return 1 ;;
  esac
}

# Recursively copy all non-system deps of $1 into usr/lib/lewdware/.
bundle_lib() {
  local target="$1"
  while IFS= read -r dep_path; do
    [[ -z "$dep_path" || ! -f "$dep_path" ]] && continue
    local lib_name
    lib_name=$(basename "$dep_path")
    is_system_lib "$lib_name" && continue
    local staged="$STAGE_DIR/usr/lib/lewdware/$lib_name"
    [[ -f "$staged" ]] && continue
    echo "   Bundling: $dep_path"
    cp "$dep_path" "$staged"
    chmod 755 "$staged"
    bundle_lib "$staged"
  done < <(ldd "$target" 2>/dev/null | awk '/=>/ { print $3 }')
}

bundle_lib "target/release/lewdware-engine"
bundle_lib "target/release/lw"
bundle_lib "target/release/lewdware"

echo "Patching bundled library rpaths..."
for lib in "$STAGE_DIR/usr/lib/lewdware/"*; do
  [ -f "$lib" ] || continue
  patchelf --set-rpath '$ORIGIN' "$lib" 2>/dev/null || true
done

# 4. Create Desktop File and Icon
echo "Creating desktop entries..."
cat <<EOF > "$STAGE_DIR/usr/share/applications/lewdware.desktop"
[Desktop Entry]
Name=Lewdware
Comment=Configure and launch Lewdware
Exec=lewdware
Icon=lewdware
Terminal=false
Type=Application
Categories=Utility;Development;
EOF

# Copy app icon if exists (use a placeholder if not)
if [ -f "config/src-tauri/icons/128x128.png" ]; then
  cp "config/src-tauri/icons/128x128.png" "$STAGE_DIR/usr/share/icons/hicolor/128x128/apps/lewdware.png"
fi

# 5. Create Debian Package control file
echo "Creating Debian control file..."
cat <<EOF > "$STAGE_DIR/DEBIAN/control"
Package: lewdware
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${DEB_ARCH}
Depends: libasound2, libx11-6, libxi6, libxtst6, libxrandr2, libxcursor1
Maintainer: restoredelight <restoreddelight@proton.me>
Description: Lewdware (Main App, Config GUI, and lw CLI tool)
EOF

# 6. Build the Debian Package
echo "Building Debian package..."
dpkg-deb --build "$STAGE_DIR" "$OUTPUT_DIR/lewdware_${VERSION}_${ARCH}.deb"
echo "Debian package created!"

# 7. Build the RPM Package
if command -v rpmbuild &> /dev/null; then
  echo "Building RPM package..."
  RPM_STAGE_DIR="build/rpm-stage"
  rm -rf "$RPM_STAGE_DIR"
  mkdir -p "$RPM_STAGE_DIR"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}

  cat <<EOF > "$RPM_STAGE_DIR/SPECS/lewdware.spec"
# FFmpeg/dav1d are statically linked into lewdware-engine (build-ffmpeg
# feature), so they no longer appear as separate .so files here - these
# excludes just guard against auto-generated requires/provides for whatever
# ends up bundled in usr/lib/lewdware/.
%global __requires_exclude_from /usr/lib/lewdware/
%global __provides_exclude_from /usr/lib/lewdware/
%global debug_package %{nil}
%global __strip /bin/true

Name:           lewdware
Version:        ${VERSION}
Release:        1
Summary:        Lewdware (Main App, Config GUI, and lw CLI tool)
License:        MIT
Requires:       alsa-lib, libX11, libXi, libXtst, libXrandr, libXcursor

%description
Lewdware, containing the main app, config GUI, and lw CLI tool.

%install
mkdir -p %{buildroot}/usr/bin
mkdir -p %{buildroot}/usr/lib/lewdware
mkdir -p %{buildroot}/usr/share/applications
mkdir -p %{buildroot}/usr/share/icons/hicolor/128x128/apps
mkdir -p %{buildroot}/usr/share/licenses/lewdware

cp -p %{staged_dir}/usr/bin/* %{buildroot}/usr/bin/
cp -pr %{staged_dir}/usr/lib/lewdware/* %{buildroot}/usr/lib/lewdware/
cp -p %{staged_dir}/usr/share/applications/* %{buildroot}/usr/share/applications/
cp -p %{staged_dir}/usr/share/icons/hicolor/128x128/apps/* %{buildroot}/usr/share/icons/hicolor/128x128/apps/
cp -p %{staged_dir}/usr/share/doc/lewdware/copyright %{buildroot}/usr/share/licenses/lewdware/LICENSE

%files
/usr/bin/lewdware
/usr/bin/lw
/usr/lib/lewdware/*
/usr/share/applications/lewdware.desktop
/usr/share/icons/hicolor/128x128/apps/lewdware.png
%license /usr/share/licenses/lewdware/LICENSE
EOF

  rpmbuild -bb \
    --define "_topdir $(pwd)/$RPM_STAGE_DIR" \
    --define "staged_dir $(pwd)/$STAGE_DIR" \
    "$RPM_STAGE_DIR/SPECS/lewdware.spec"

  # Find generated RPM and copy to dist
  find "$RPM_STAGE_DIR/RPMS" -type f -name "*.rpm" -exec cp {} "$OUTPUT_DIR/lewdware_${VERSION}_${ARCH}.rpm" \;
  echo "RPM package created!"
else
  echo "Warning: rpmbuild not found, skipping RPM packaging."
fi

# 8. Build the portable tar.gz package
echo "Building portable tar.gz package..."
TAR_STAGE="build/tar-stage"
TAR_ROOT="$TAR_STAGE/lewdware-${VERSION}"
rm -rf "$TAR_STAGE"
mkdir -p "$TAR_ROOT/bin"
mkdir -p "$TAR_ROOT/lib/lewdware"

# Copy lw CLI
cp "$STAGE_DIR/usr/bin/lw" "$TAR_ROOT/bin/"

# Copy config AppImage as the user-facing lewdware binary
APPIMAGE_PATH=$(find "target/release/bundle/appimage/" -name "lewdware_${VERSION}_*.AppImage" 2>/dev/null | head -1)
if [ -f "$APPIMAGE_PATH" ]; then
  cp "$APPIMAGE_PATH" "$TAR_ROOT/bin/lewdware"
  chmod +x "$TAR_ROOT/bin/lewdware"
else
  echo "Warning: config AppImage not found! Skipping config GUI in tar.gz."
fi

# Copy dynamic libraries and the engine (internal, launched by config app)
cp "$STAGE_DIR/usr/lib/lewdware/"* "$TAR_ROOT/lib/lewdware/"

# Ship the MIT license alongside the binaries
cp "LICENSE" "$TAR_ROOT/LICENSE"

# Create a simple setup/run README
cat << 'EOF' > "$TAR_ROOT/README.md"
# Lewdware (and tools)

This portable distribution contains the Lewdware Config app, Engine, and lw CLI.

## Structure
* `bin/lewdware`: Lewdware Config app (AppImage) — start here
* `bin/lw`: Lewdware CLI
* `lib/lewdware/`: Engine and any bundled dynamic libraries (FFmpeg and dav1d are statically linked into the engine)
* `LICENSE`: MIT license covering this software

## Running
Ensure you have the basic client dependencies installed on your Linux distribution (X11, ALSA, etc.).
Simply run the config app from the `bin` directory:
```bash
./bin/lewdware
```
EOF

# Pack archive
tar -czf "$OUTPUT_DIR/lewdware_${VERSION}_${ARCH}.tar.gz" -C "$TAR_STAGE" "lewdware-${VERSION}"
echo "Portable tar.gz package created!"

echo "SUCCESS: All Linux target packages staged/created in $OUTPUT_DIR!"
