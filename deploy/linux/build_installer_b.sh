#!/bin/bash
# deploy/linux/build_installer_b.sh
# Linux Build & Packaging script for Lewdware Pack Editor (Installer B): .deb, .rpm and a portable
# tarball, all staged from one tree. build_flatpak.sh repacks the .deb afterwards.
set -e

VERSION=$(grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
DEB_ARCH=$(dpkg --print-architecture 2>/dev/null || true)
case "$(uname -m)" in
  x86_64)        ARCH="x86_64" ; DEB_ARCH="${DEB_ARCH:-amd64}" ;;
  aarch64|arm64) ARCH="arm64"  ; DEB_ARCH="${DEB_ARCH:-arm64}" ;;
  *)             ARCH="$(uname -m)" ;;
esac
STAGE_DIR="build/pack-editor-stage"
OUTPUT_DIR="dist"
# Where the sidecars and licences go. The editor looks for `../lib/lewdware-pack-editor/binaries`
# relative to its executable (pack-editor-gpui/src/main.rs, `sidecar_dirs`), so the tree is
# relocatable: the same layout works under /usr, /app (Flatpak) and an unpacked tarball.
LIB_DIR="$STAGE_DIR/usr/lib/lewdware-pack-editor"
DOC_DIR="$STAGE_DIR/usr/share/doc/lewdware-pack-editor"

# 1. Build the avifenc sidecar if not already present. Images are the one thing still encoded by
# a separate program; video, audio and probing go through the linked FFmpeg.
"$(dirname "$0")/build_avifenc_sidecar.sh"

# Static, LGPL-only FFmpeg with SVT-AV1, Opus, dav1d and libwebp (deploy/build_ffmpeg.sh; a no-op
# when build/ffmpeg-static is already up to date). Its pkg-config files go first on the path so
# that FFmpeg is what `--features static-ffmpeg` links, along with the libraries it needs, and so
# the `image` crate's AVIF decoder links the same static dav1d.
"$(dirname "$0")/../build_ffmpeg.sh" build/ffmpeg-static
export PKG_CONFIG_PATH="$PWD/build/ffmpeg-static/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

# 2. Build the editor, linking that FFmpeg (it previews and encodes with it) rather than the
# distro's GPL libavcodec. See pack-editor-gpui/THIRD_PARTY_LICENSES.md.
echo "🔨 Building the pack editor..."
cargo build -p lewdware-pack-editor --release --features static-ffmpeg

# 3. Stage
echo "Staging..."
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/usr/bin" "$LIB_DIR/binaries" "$DOC_DIR" "$STAGE_DIR/usr/share/applications"
mkdir -p "$OUTPUT_DIR"

cp "target/release/lewdware-pack-editor" "$STAGE_DIR/usr/bin/lewdware-pack-editor"
cp "pack-editor-gpui/binaries/lewdware-avifenc" "$LIB_DIR/binaries/lewdware-avifenc"
chmod 755 "$STAGE_DIR/usr/bin/lewdware-pack-editor" "$LIB_DIR/binaries/"*

cp "LICENSE" "$DOC_DIR/copyright"
cp "pack-editor-gpui/THIRD_PARTY_LICENSES.md" "$DOC_DIR/THIRD_PARTY_LICENSES.md"
cp "pack-editor-gpui/COPYING.LGPLv2.1" "$DOC_DIR/COPYING.LGPLv2.1"

# The desktop file is named after the window's app id (pack-editor-gpui/src/main.rs), and so is the
# icon, which lets the Flatpak use both unchanged.
cp "deploy/linux/com.lewdware.pack-editor.desktop" "$STAGE_DIR/usr/share/applications/"
for size in 32 64 128 256 512; do
  mkdir -p "$STAGE_DIR/usr/share/icons/hicolor/${size}x${size}/apps"
  cp "assets/icons/${size}x${size}.png" \
    "$STAGE_DIR/usr/share/icons/hicolor/${size}x${size}/apps/com.lewdware.pack-editor.png"
done

# 4. .deb
#
# FFmpeg and its codec libraries are linked statically (static-ffmpeg), so what's left are the
# windowing, font, GPU, video-acceleration and sound libraries every desktop has.
echo "Building Debian package..."
mkdir -p "$STAGE_DIR/DEBIAN"
cat <<EOF > "$STAGE_DIR/DEBIAN/control"
Package: lewdware-pack-editor
Version: ${VERSION}
Section: graphics
Priority: optional
Architecture: ${DEB_ARCH}
Depends: libasound2, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libfreetype6, libvulkan1, libva2, libva-drm2, libdrm2
Recommends: mesa-vulkan-drivers
Maintainer: restoredelight <restoreddelight@proton.me>
Homepage: https://lewdware.net
Description: Create and edit Lewdware packs
EOF
dpkg-deb --root-owner-group --build "$STAGE_DIR" "$OUTPUT_DIR/lewdware-pack-editor_${VERSION}_${ARCH}.deb"
rm -rf "$STAGE_DIR/DEBIAN"
echo "SUCCESS: Staged lewdware-pack-editor_${VERSION}_${ARCH}.deb in dist/"

# 5. .rpm
#
# These are advertised in docs/src/data/pack-editor-latest.json, so a missing one means a dead
# download link on the website: fail the build rather than publish a partial release.
command -v rpmbuild >/dev/null || {
  echo "Error: rpmbuild not found." >&2
  exit 1
}
echo "Building RPM package..."
RPM_DIR="build/pack-editor-rpm"
rm -rf "$RPM_DIR"
mkdir -p "$RPM_DIR"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
cat <<EOF > "$RPM_DIR/SPECS/lewdware-pack-editor.spec"
# The sidecars are prebuilt static executables; don't scan them for requires/provides.
%global __requires_exclude_from /usr/lib/lewdware-pack-editor/
%global __provides_exclude_from /usr/lib/lewdware-pack-editor/
%global debug_package %{nil}
%global __strip /bin/true
%define _binary_payload w3.zstdio

Name:           lewdware-pack-editor
Version:        ${VERSION}
Release:        1
Summary:        Create and edit Lewdware packs
License:        MIT and LGPL-2.1-or-later and BSD-2-Clause and BSD-3-Clause
URL:            https://lewdware.net
Requires:       alsa-lib, libxcb, libxkbcommon, libxkbcommon-x11, freetype, vulkan-loader, libva, libdrm
Recommends:     mesa-vulkan-drivers

%description
The Lewdware Pack Editor builds and edits packs: the images, videos, audio, captions, prompts and
behaviour that drive a Lewdware session.

%install
mkdir -p %{buildroot}
cp -pr %{staged_dir}/usr %{buildroot}/

%files
/usr/bin/lewdware-pack-editor
/usr/lib/lewdware-pack-editor
/usr/share/applications/com.lewdware.pack-editor.desktop
/usr/share/icons/hicolor/*/apps/com.lewdware.pack-editor.png
%doc /usr/share/doc/lewdware-pack-editor
EOF
rpmbuild -bb \
  --define "_topdir $(pwd)/$RPM_DIR" \
  --define "staged_dir $(pwd)/$STAGE_DIR" \
  "$RPM_DIR/SPECS/lewdware-pack-editor.spec"
find "$RPM_DIR/RPMS" -type f -name "*.rpm" \
  -exec cp {} "$OUTPUT_DIR/lewdware-pack-editor_${VERSION}_${ARCH}.rpm" \;
echo "SUCCESS: Staged lewdware-pack-editor_${VERSION}_${ARCH}.rpm in dist/"

# 6. Tarball
#
# The distro-agnostic option for anyone who isn't on deb/rpm and doesn't want Flatpak, and the
# form a downstream packager (an AUR PKGBUILD, a Nix derivation) can actually consume. The staged
# `usr/` tree is relocatable (see LIB_DIR above), so the unpacked directory runs in place and
# `install.sh` is a convenience rather than a requirement.
ROOT="lewdware-pack-editor_${VERSION}_${ARCH}"
TAR_STAGE=$(mktemp -d)
trap 'rm -rf "$TAR_STAGE"' EXIT
mkdir -p "$TAR_STAGE/$ROOT"
cp -a "$STAGE_DIR/usr/." "$TAR_STAGE/$ROOT/"

# The desktop file's Exec is a bare command name, which only resolves once the binary is on PATH.
# Rewritten at install time to wherever the user actually put it.
cat > "$TAR_STAGE/$ROOT/install.sh" <<'INSTALL'
#!/bin/sh
# Installs the pack editor for the current user (or to $PREFIX, if set).
set -e
PREFIX="${PREFIX:-$HOME/.local}"
HERE=$(cd "$(dirname "$0")" && pwd)

mkdir -p "$PREFIX/bin" "$PREFIX/lib" "$PREFIX/share"
cp -a "$HERE/bin/." "$PREFIX/bin/"
cp -a "$HERE/lib/." "$PREFIX/lib/"
cp -a "$HERE/share/." "$PREFIX/share/"

desktop="$PREFIX/share/applications/com.lewdware.pack-editor.desktop"
if [ -f "$desktop" ]; then
  sed -i "s|^Exec=.*|Exec=$PREFIX/bin/lewdware-pack-editor|" "$desktop"
fi
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" 2>/dev/null || true

echo "Installed to $PREFIX. Run: $PREFIX/bin/lewdware-pack-editor"
case ":$PATH:" in
  *":$PREFIX/bin:"*) ;;
  *) echo "Note: $PREFIX/bin is not on your PATH." ;;
esac
INSTALL
chmod +x "$TAR_STAGE/$ROOT/install.sh"

cat > "$TAR_STAGE/$ROOT/README" <<README
Lewdware Pack Editor ${VERSION} (${ARCH})

Run it straight out of this directory:

    ./bin/lewdware-pack-editor

Or install it for your user (override the location with PREFIX=...):

    ./install.sh

It needs the libraries any desktop has: xkbcommon, FreeType, ALSA, libva, and a Vulkan driver.
If the binary reports a missing library, install it with your package manager, for example:

    Arch      sudo pacman -S libxkbcommon-x11 freetype2 alsa-lib libva vulkan-icd-loader
    Debian    sudo apt install libxkbcommon-x11-0 libfreetype6 libasound2 libva2 libva-drm2 libvulkan1
    Fedora    sudo dnf install libxkbcommon-x11 freetype alsa-lib libva vulkan-loader

The Flatpak build carries its own and needs nothing from the host. See
https://lewdware.net/download/pack-editor
README

tar -C "$TAR_STAGE" -czf "$OUTPUT_DIR/${ROOT}.tar.gz" "$ROOT"
echo "SUCCESS: Staged ${ROOT}.tar.gz in dist/"
