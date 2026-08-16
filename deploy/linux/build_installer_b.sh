#!/bin/bash
# deploy/linux/build_installer_b.sh
# Linux Build & Packaging script for Lewdware Pack Editor (Installer B)
set -e

# Detect architecture
case "$(uname -m)" in
  x86_64)       ARCH="x86_64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *)             ARCH="$(uname -m)" ;;
esac

# 1. Fetch static FFmpeg and ffprobe if not already present
"$(dirname "$0")/download_ffmpeg_sidecars.sh"

# 1b. Build the avifenc sidecar if not already present
"$(dirname "$0")/build_avifenc_sidecar.sh"

# 2. Build the Tauri app
echo "🔨 Building pack-editor-tauri GUI..."
cd pack-editor
pnpm install
export NO_STRIP=1
# `tauri.conf.json` sets targets to "all" for the Windows and macOS scripts; on Linux that would
# also build an AppImage, which the pack editor no longer ships. See the staging comments below.
pnpm tauri build --bundles deb,rpm
cd ..

# 3. Move output to dist
echo "Staging outputs..."
mkdir -p dist

VERSION=$(grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')

# The bundle file names come straight from tauri.conf.json's productName ("Lewdware Pack
# Editor"), which the bundler only sanitises for *package* names, not file names - so match
# case-insensitively and tolerate spaces/hyphens between the words.
stage() {
  local ext="$1"
  local src
  src=$(find target/release/bundle/ -type f -iname "lewdware?pack?editor*.$ext" 2>/dev/null | head -1)
  if [ -z "$src" ]; then
    echo "Error: no .$ext package found under target/release/bundle/!" >&2
    return 1
  fi
  cp "$src" "dist/lewdware-pack-editor_${VERSION}_${ARCH}.$ext"
  echo "SUCCESS: Staged lewdware-pack-editor_${VERSION}_${ARCH}.$ext in dist/"
}

# The distro-agnostic option for anyone who isn't on deb/rpm and doesn't want Flatpak -- and the
# form a downstream packager (an AUR PKGBUILD, a Nix derivation) can actually consume, which a
# .deb and a .flatpak are both awkward for.
#
# Repacked from the .deb rather than built separately: the bundler has already staged the binary,
# the sidecars, the icons and the desktop entry there, and its `usr/{bin,lib,share}` tree is
# relocatable as-is, because Tauri resolves resources relative to the executable
# (`<exe>/../lib/<productName>`) rather than from an absolute path. So the unpacked directory runs
# in place from anywhere, and `install.sh` is a convenience rather than a requirement.
stage_tarball() {
  local deb root staging
  deb=$(find target/release/bundle/deb -type f -iname "lewdware?pack?editor*.deb" 2>/dev/null | head -1)
  if [ -z "$deb" ]; then
    echo "Error: no .deb found to repack into a tarball!" >&2
    return 1
  fi

  root="lewdware-pack-editor_${VERSION}_${ARCH}"
  staging=$(mktemp -d)

  # `ar`/`tar` rather than `dpkg-deb`, which is not installed on non-Debian build hosts. `tar -xf`
  # detects the payload's compression itself, so this survives the bundler switching it.
  (cd "$staging" && ar -x "$OLDPWD/$deb")
  tar -C "$staging" -xf "$staging"/data.tar.*
  mkdir -p "$staging/$root"
  mv "$staging/usr/"* "$staging/$root/"

  # The bundler's Exec is a bare command name, which only resolves once the binary is on PATH.
  # Rewritten at install time to wherever the user actually put it.
  cat > "$staging/$root/install.sh" <<'INSTALL'
#!/bin/sh
# Installs the pack editor for the current user (or to $PREFIX, if set).
set -e
PREFIX="${PREFIX:-$HOME/.local}"
HERE=$(cd "$(dirname "$0")" && pwd)

mkdir -p "$PREFIX/bin" "$PREFIX/lib" "$PREFIX/share"
cp -a "$HERE/bin/." "$PREFIX/bin/"
cp -a "$HERE/lib/." "$PREFIX/lib/"
cp -a "$HERE/share/." "$PREFIX/share/"

desktop="$PREFIX/share/applications/lewdware-pack-editor.desktop"
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
  chmod +x "$staging/$root/install.sh"

  cat > "$staging/$root/README" <<README
Lewdware Pack Editor ${VERSION} (${ARCH})

Run it straight out of this directory:

    ./bin/lewdware-pack-editor

Or install it for your user (override the location with PREFIX=...):

    ./install.sh

Requires WebKitGTK 4.1 and GTK 3, which this archive does not bundle -- deliberately, so the
editor renders with the same WebKit as the rest of your desktop. Install them with your package
manager if the binary reports a missing library:

    Arch      sudo pacman -S webkit2gtk-4.1 gtk3
    Debian    sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0
    Fedora    sudo dnf install webkit2gtk4.1 gtk3
    openSUSE  sudo zypper install libwebkit2gtk-4_1-0 gtk3

If you would rather not manage those, the Flatpak build carries its own and needs nothing from
the host. See https://lewdware.net/download/pack-editor
README

  tar -C "$staging" -czf "dist/${root}.tar.gz" "$root"
  rm -rf "$staging"
  echo "SUCCESS: Staged ${root}.tar.gz in dist/"
}

# These are advertised in docs/src/data/pack-editor-latest.json, so a missing one means a dead
# download link on the website - fail the build rather than publish a partial release.
#
# No AppImage: it was retired in August 2026, replaced by the Flatpak above and the tarball below
# between them. An AppImage bundles its own WebKitGTK, which then disagrees with the host's GL
# stack; the workarounds that follows (`WEBKIT_DISABLE_DMABUF_RENDERER` and friends, in
# `shared/src/utils.rs`) break `<video>` outright, which is what froze the editor's media preview.
# The Flatpak runs against a WebKit built to match host drivers, and the tarball uses the host's
# own -- neither needs the workaround.
#
# Note `shared/src/utils.rs` still carries its AppImage branches: the *config* app is still
# shipped as an AppImage inside the main suite's tarball (see build_installer_a.sh), and it calls
# the same safeguards.
stage deb
stage rpm
stage_tarball
