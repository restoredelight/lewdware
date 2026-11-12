#!/bin/bash
# deploy/linux/build_installer.sh
# Builds and packages Lewdware for Linux as a .deb, an .rpm and a portable tarball: the config app,
# the pack editor, the supervisor, the engine and the lw CLI.
#
# Every app but lw is in one shared library, liblewdware_core.so, which each app's small launcher
# loads (core/, launcher/). The layout, the same under /usr and in the unpacked tarball:
#
#   bin/lewdware                         the config app's launcher
#   bin/lewdware-pack-editor             the pack editor's launcher
#   bin/lw                               the CLI, its own program
#   lib/lewdware/liblewdware_core.so     everything else, FFmpeg statically linked in
#   lib/lewdware/lewdware-engine         the engine's launcher, started by the supervisor
#   lib/lewdware/lewdware-supervisor     the supervisor's launcher
#   lib/lewdware/binaries/               the pack editor's avifenc sidecar
#
# The launchers find the core relative to themselves, and the core finds everything else relative
# to itself (shared::binaries::core_dir), so the tree is relocatable.
set -e

VERSION=$(grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
DEB_ARCH=$(dpkg --print-architecture)
case "$(uname -m)" in
  x86_64)       ARCH="x86_64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *)             ARCH="$(uname -m)" ;;
esac
STAGE_DIR="build/deb-stage"
LIB_DIR="$STAGE_DIR/usr/lib/lewdware"
DOC_DIR="$STAGE_DIR/usr/share/doc/lewdware"
OUTPUT_DIR="dist"

echo "🧹 Preparing clean staging area..."
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/DEBIAN"
mkdir -p "$STAGE_DIR/usr/bin"
mkdir -p "$LIB_DIR/binaries"
mkdir -p "$STAGE_DIR/usr/share/applications"
for size in 32 64 128 256 512; do
  mkdir -p "$STAGE_DIR/usr/share/icons/hicolor/${size}x${size}/apps"
done
mkdir -p "$DOC_DIR"
mkdir -p "$OUTPUT_DIR"

# Licences. MIT's own terms require its notice to accompany copies of the software; the LGPL
# requires its text to accompany the FFmpeg statically linked into the core; the OFL requires the
# same of the fonts the apps embed.
cp "LICENSE" "$DOC_DIR/copyright"
cp "lewdware/THIRD_PARTY_LICENSES.md" "$DOC_DIR/THIRD_PARTY_LICENSES.md"
cp "pack-editor-gpui/THIRD_PARTY_LICENSES.md" "$DOC_DIR/THIRD_PARTY_LICENSES-pack-editor.md"
cp "pack-editor-gpui/COPYING.LGPLv2.1" "$DOC_DIR/COPYING.LGPLv2.1"
cp -r "assets/fonts/licenses" "$DOC_DIR/fonts"

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
echo "🔨 Compiling applications..."
cargo build -p lw --release

echo "🔨 Building default modes..."
for mode in sandbox experience; do
  (cd "default-modes/$mode" && ../../target/release/lw mode build)
done

# The shared core, linking the static FFmpeg built above rather than Ubuntu's GPL-licensed
# libavcodec.so (see lewdware/THIRD_PARTY_LICENSES.md), and the launchers. The config app, in the
# core, embeds the default modes built above.
echo "🔨 Building the shared core and the launchers..."
cargo build -p lewdware-core -p lewdware-launcher --release --features lewdware-core/static-ffmpeg

# 2. Stage binaries. The launchers are built as launcher-<app> (launcher/Cargo.toml).
echo "Staging binaries..."
cp "target/release/launcher-config" "$STAGE_DIR/usr/bin/lewdware"
cp "target/release/launcher-pack-editor" "$STAGE_DIR/usr/bin/lewdware-pack-editor"
cp "target/release/lw" "$STAGE_DIR/usr/bin/lw"
cp "target/release/liblewdware_core.so" "$LIB_DIR/liblewdware_core.so"
cp "target/release/launcher-engine" "$LIB_DIR/lewdware-engine"
cp "target/release/launcher-supervisor" "$LIB_DIR/lewdware-supervisor"
cp "pack-editor-gpui/binaries/lewdware-avifenc" "$LIB_DIR/binaries/lewdware-avifenc"
chmod 755 "$STAGE_DIR/usr/bin/"* "$LIB_DIR/"* "$LIB_DIR/binaries/"*

# 3. Dynamic Library Bundling (transitive deps of any dynamically-linked libs).
# FFmpeg and its codec libraries, dav1d included, are statically linked into the core (see the
# static-ffmpeg feature above), so they won't show up here. libva and libdrm will, and stay host
# dependencies.
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
    libexpat.so* | libselinux.so* | libssl.so* | libcrypto.so* | \
    libva.so* | libva-drm.so* | libdrm.so*)
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
    local staged="$LIB_DIR/$lib_name"
    [[ -f "$staged" ]] && continue
    echo "   Bundling: $dep_path"
    cp "$dep_path" "$staged"
    chmod 755 "$staged"
    bundle_lib "$staged"
  done < <(ldd "$target" 2>/dev/null | awk '/=>/ { print $3 }')
}

bundle_lib "target/release/liblewdware_core.so"
bundle_lib "target/release/lw"
for launcher in config pack-editor engine supervisor; do
  bundle_lib "target/release/launcher-$launcher"
done

# Everything directly in usr/lib/lewdware/ (the core, any bundled .so files, and the engine and
# supervisor launchers) resolves its siblings via $ORIGIN. The avifenc sidecar is static.
echo "Patching bundled library rpaths..."
for lib in "$LIB_DIR/"*; do
  [ -f "$lib" ] || continue
  patchelf --set-rpath '$ORIGIN' "$lib" 2>/dev/null || true
done

# usr/bin/* live a directory up, so they need an rpath pointing into the bundle - without this,
# any non-system dependency of theirs that bundle_lib staged above is invisible at runtime and the
# dynamic loader falls back to whatever the host happens to have.
echo "Patching /usr/bin rpaths..."
for bin in "$STAGE_DIR/usr/bin/"*; do
  [ -f "$bin" ] || continue
  patchelf --set-rpath '$ORIGIN/../lib/lewdware' "$bin" 2>/dev/null || true
done

# 4. Desktop files and icons
#
# The two apps' desktop files are named after their windows' app ids (config-gpui/src/lib.rs,
# pack-editor-gpui/src/lib.rs): that is how Wayland and X11 desktops find a window's icon, since
# gpui can't set one itself on Linux. The supervisor's is its portal identity (it has no window),
# also used in compositor permission dialogs.
echo "Installing desktop entries and icons..."
for app in config pack-editor supervisor; do
  cp "deploy/linux/com.lewdware.$app.desktop" "$STAGE_DIR/usr/share/applications/"
done
for size in 32 64 128 256 512; do
  icons="$STAGE_DIR/usr/share/icons/hicolor/${size}x${size}/apps"
  cp "assets/icons/${size}x${size}.png" "$icons/lewdware.png"
  cp "assets/icons/${size}x${size}.png" "$icons/com.lewdware.pack-editor.png"
done

# 5. Create Debian Package control file
#
# The pack editor used to be a package of its own. It is part of this one now: Conflicts makes apt
# remove the old one, Replaces lets this one take over its files, and Provides keeps anything that
# depended on it satisfied.
echo "Creating Debian control file..."
cat <<EOF > "$STAGE_DIR/DEBIAN/control"
Package: lewdware
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${DEB_ARCH}
Depends: libasound2, libdbus-1-3, libx11-6, libxi6, libxtst6, libxrandr2, libxcursor1, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libfreetype6, libvulkan1, libva2, libva-drm2, libdrm2
Recommends: mesa-vulkan-drivers
Conflicts: lewdware-pack-editor
Replaces: lewdware-pack-editor
Provides: lewdware-pack-editor
Maintainer: restoredelight <restoreddelight@proton.me>
Homepage: https://lewdware.net
Description: Lewdware: the config app, the pack editor, the supervisor, the engine and the lw CLI
EOF

# 5b. Maintainer scripts that stop a running install before its files are touched.
#
# Unlike Windows, a running binary here doesn't block the upgrade (dpkg/rpm swap the inode and
# the old process keeps the old one), so this is about not leaving a stale supervisor running
# old code against a new install - and about ending any live session cleanly on the way.
#
# A running pack editor is left alone: it may have unsaved work, and it keeps the old core it
# already loaded until it is closed.
#
# Shared by preinst (runs before unpack, on both install and upgrade) and prerm (runs before
# removal).
cat <<'EOF' > "$STAGE_DIR/DEBIAN/preinst"
#!/bin/sh
SUPERVISOR=/usr/lib/lewdware/lewdware-supervisor

# On Linux the control socket lives in the abstract namespace (shared/src/ipc.rs), which is
# per-netns rather than per-user, so root can reach a session started by any user.
if [ -x "$SUPERVISOR" ]; then
  # Ends any live session cleanly - the engine restores the desktop wallpaper on its way out.
  "$SUPERVISOR" stop >/dev/null 2>&1 || true
  sleep 2
fi

# There is no IPC request that shuts the daemon itself down (only StopSession), so the
# supervisor has to be killed outright. If pkill isn't installed this degrades to leaving it
# running until the next reboot, which is why it isn't a hard failure.
for image in lewdware-engine lewdware-supervisor lewdware; do
  pkill -x "$image" >/dev/null 2>&1 || true
done

# Never fail the package operation over this.
exit 0
EOF

cp "$STAGE_DIR/DEBIAN/preinst" "$STAGE_DIR/DEBIAN/prerm"
chmod 755 "$STAGE_DIR/DEBIAN/preinst" "$STAGE_DIR/DEBIAN/prerm"

# 6. Build the Debian Package
echo "Building Debian package..."
dpkg-deb --root-owner-group --build "$STAGE_DIR" "$OUTPUT_DIR/lewdware_${VERSION}_${ARCH}.deb"
echo "Debian package created!"

# 7. Build the RPM Package
#
# Advertised in docs/src/data/latest.json, so a missing one means a dead download link on the
# website: fail the build rather than publish a partial release.
command -v rpmbuild >/dev/null || {
  echo "Error: rpmbuild not found." >&2
  exit 1
}
echo "Building RPM package..."
RPM_STAGE_DIR="build/rpm-stage"
rm -rf "$RPM_STAGE_DIR"
mkdir -p "$RPM_STAGE_DIR"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}

cat <<EOF > "$RPM_STAGE_DIR/SPECS/lewdware.spec"
# FFmpeg/dav1d are statically linked into liblewdware_core.so (static-ffmpeg feature), so they
# don't appear as separate .so files here - these excludes just guard against auto-generated
# requires/provides for whatever ends up bundled in usr/lib/lewdware/.
%global __requires_exclude_from /usr/lib/lewdware/
%global __provides_exclude_from /usr/lib/lewdware/
%global debug_package %{nil}
%global __strip /bin/true

Name:           lewdware
Version:        ${VERSION}
Release:        1
Summary:        Lewdware: the config app, the pack editor, the supervisor, the engine and lw
License:        MIT and LGPL-2.1-or-later and BSD-2-Clause and BSD-3-Clause and OFL-1.1
URL:            https://lewdware.net
Requires:       alsa-lib, dbus-libs, libX11, libXi, libXtst, libXrandr, libXcursor, libxcb, libxkbcommon, libxkbcommon-x11, freetype, vulkan-loader, libva, libdrm
Recommends:     mesa-vulkan-drivers
# The pack editor used to be a package of its own.
Provides:       lewdware-pack-editor = %{version}-%{release}
Obsoletes:      lewdware-pack-editor < %{version}-%{release}

%description
Lewdware, containing the config app, the pack editor, the supervisor, the engine and the lw CLI.

%install
mkdir -p %{buildroot}/usr/share/licenses
cp -pr %{staged_dir}/usr/bin %{buildroot}/usr/
mkdir -p %{buildroot}/usr/lib
cp -pr %{staged_dir}/usr/lib/lewdware %{buildroot}/usr/lib/
mkdir -p %{buildroot}/usr/share
cp -pr %{staged_dir}/usr/share/applications %{buildroot}/usr/share/
mkdir -p %{buildroot}/usr/share/icons
cp -pr %{staged_dir}/usr/share/icons/hicolor %{buildroot}/usr/share/icons/
cp -pr %{staged_dir}/usr/share/doc/lewdware %{buildroot}/usr/share/licenses/lewdware
mv %{buildroot}/usr/share/licenses/lewdware/copyright %{buildroot}/usr/share/licenses/lewdware/LICENSE

# The rpm counterpart of the deb preinst/prerm above: stop a running install before its files
# are touched, so no stale supervisor is left running old code. Note the escaped \$ - this spec
# is written through an unquoted heredoc, so unescaped variables would expand at build time.
%pre
SUPERVISOR=/usr/lib/lewdware/lewdware-supervisor
if [ -x "\$SUPERVISOR" ]; then
  "\$SUPERVISOR" stop >/dev/null 2>&1 || true
  sleep 2
fi
for image in lewdware-engine lewdware-supervisor lewdware; do
  pkill -x "\$image" >/dev/null 2>&1 || true
done
exit 0

%preun
SUPERVISOR=/usr/lib/lewdware/lewdware-supervisor
if [ -x "\$SUPERVISOR" ]; then
  "\$SUPERVISOR" stop >/dev/null 2>&1 || true
  sleep 2
fi
for image in lewdware-engine lewdware-supervisor lewdware; do
  pkill -x "\$image" >/dev/null 2>&1 || true
done
exit 0

%files
/usr/bin/lewdware
/usr/bin/lewdware-pack-editor
/usr/bin/lw
/usr/lib/lewdware
/usr/share/applications/com.lewdware.config.desktop
/usr/share/applications/com.lewdware.pack-editor.desktop
/usr/share/applications/com.lewdware.supervisor.desktop
/usr/share/icons/hicolor/*/apps/lewdware.png
/usr/share/icons/hicolor/*/apps/com.lewdware.pack-editor.png
%license /usr/share/licenses/lewdware
EOF

rpmbuild -bb \
  --define "_topdir $(pwd)/$RPM_STAGE_DIR" \
  --define "staged_dir $(pwd)/$STAGE_DIR" \
  "$RPM_STAGE_DIR/SPECS/lewdware.spec"

find "$RPM_STAGE_DIR/RPMS" -type f -name "*.rpm" -exec cp {} "$OUTPUT_DIR/lewdware_${VERSION}_${ARCH}.rpm" \;
echo "RPM package created!"

# 8. Build the portable tarball
#
# For anyone not on deb or rpm, and the form a downstream packager (an AUR PKGBUILD, a Nix
# derivation) can consume. It is the staged usr/ tree, which is relocatable, so it runs in place;
# install.sh is a convenience, not a requirement.
echo "Building portable tar.gz package..."
TAR_STAGE="build/tar-stage"
TAR_ROOT="$TAR_STAGE/lewdware-${VERSION}"
rm -rf "$TAR_STAGE"
mkdir -p "$TAR_ROOT"
cp -a "$STAGE_DIR/usr/bin" "$STAGE_DIR/usr/lib" "$STAGE_DIR/usr/share" "$TAR_ROOT/"
cp "LICENSE" "$TAR_ROOT/LICENSE"

# The desktop files name bare commands (and the supervisor's, /usr/lib/lewdware/), which only
# resolve for a /usr install. Rewritten at install time to wherever the user put it.
cat > "$TAR_ROOT/install.sh" <<'INSTALL'
#!/bin/sh
# Installs Lewdware for the current user (or to $PREFIX, if set).
set -e
PREFIX="${PREFIX:-$HOME/.local}"
HERE=$(cd "$(dirname "$0")" && pwd)

mkdir -p "$PREFIX/bin" "$PREFIX/lib" "$PREFIX/share"
cp -a "$HERE/bin/." "$PREFIX/bin/"
cp -a "$HERE/lib/." "$PREFIX/lib/"
cp -a "$HERE/share/." "$PREFIX/share/"

apps="$PREFIX/share/applications"
sed -i "s|^Exec=.*|Exec=$PREFIX/bin/lewdware|" "$apps/com.lewdware.config.desktop"
sed -i "s|^Exec=.*|Exec=$PREFIX/bin/lewdware-pack-editor|" "$apps/com.lewdware.pack-editor.desktop"
sed -i "s|^Exec=.*|Exec=$PREFIX/lib/lewdware/lewdware-supervisor|" "$apps/com.lewdware.supervisor.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$apps" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" 2>/dev/null || true

echo "Installed to $PREFIX. Run: $PREFIX/bin/lewdware"
case ":$PATH:" in
  *":$PREFIX/bin:"*) ;;
  *) echo "Note: $PREFIX/bin is not on your PATH." ;;
esac
INSTALL
chmod +x "$TAR_ROOT/install.sh"

cat > "$TAR_ROOT/README" <<README
Lewdware ${VERSION} (${ARCH})

Run it straight out of this directory:

    ./bin/lewdware                  the config app: start here
    ./bin/lewdware-pack-editor      the pack editor
    ./bin/lw                        the command-line tool for modes

Or install it for your user (override the location with PREFIX=...):

    ./install.sh

lib/lewdware/ holds the code both apps share, the supervisor and the engine they start, and the
pack editor's image encoder. share/doc/lewdware/ has the licences.

It needs the libraries any desktop has: X11 or Wayland, xkbcommon, FreeType, ALSA, D-Bus, libva
and a Vulkan driver. If an app reports a missing library, install it with your package manager,
for example:

    Arch      sudo pacman -S libxkbcommon-x11 libxtst libxi freetype2 alsa-lib libva vulkan-icd-loader
    Debian    sudo apt install libxkbcommon-x11-0 libxtst6 libxi6 libfreetype6 libasound2 libva2 libva-drm2 libvulkan1
    Fedora    sudo dnf install libxkbcommon-x11 libXtst libXi freetype alsa-lib libva vulkan-loader
README

tar -czf "$OUTPUT_DIR/lewdware_${VERSION}_${ARCH}.tar.gz" -C "$TAR_STAGE" "lewdware-${VERSION}"
echo "Portable tar.gz package created!"

echo "SUCCESS: All Linux target packages staged/created in $OUTPUT_DIR!"
