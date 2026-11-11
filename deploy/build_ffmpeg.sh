#!/usr/bin/env bash
# Builds the FFmpeg that Linux and macOS releases link: static, LGPL-only, with the codec libraries
# the pack editor encodes with and the hardware APIs the engine decodes with.
#
#   deploy/build_ffmpeg.sh [prefix]          (default: build/ffmpeg-static)
#
# Then build with the prefix's pkg-config files first on the path and `--features static-ffmpeg`:
#
#   export PKG_CONFIG_PATH="$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
#   cargo build -p lewdware --release --features static-ffmpeg
#
# ffmpeg-sys-next's own `build` feature can't do this: it has no switch for SVT-AV1 and no way to
# pass configure flags, and it configures with --disable-autodetect, which also leaves out VA-API
# and VideoToolbox. Linking through pkg-config (FFMPEG_DIR unset) is what pulls in each library's
# own dependencies from the .pc files; FFMPEG_DIR would link the libav* archives alone.
#
# Everything is static except what has to come from the host: zlib, and on Linux libva and libdrm
# (libva loads the GPU's driver from the distro's own path, so it can't be bundled).
#
# What gets built, and its license (all compatible with shipping under the LGPL):
#   dav1d     BSD-2           AV1 decoding (also the `image` crate's AVIF decoder)
#   opus      BSD-3           audio in packs
#   libwebp   BSD-3           thumbnails
#   SVT-AV1   BSD-3-Clear     AV1 encoding
#   ffnvcodec MIT, headers    NVENC (Linux only; the driver is loaded at runtime)
#   FFmpeg    LGPL-2.1+       no --enable-gpl, --enable-version3 or --enable-nonfree
#
# The prefix records what it was built from in .build-id, and a later run with the same versions
# and the same script does nothing, so CI can cache the prefix.
#
# Needs: a C compiler, make, cmake, ninja, meson, nasm, pkg-config, git; on Linux also libva-dev
# and libdrm-dev.

set -euo pipefail

FFMPEG_VERSION=n9.0.2
DAV1D_VERSION=1.5.4
OPUS_VERSION=v1.6.1
WEBP_VERSION=v1.6.0
SVTAV1_VERSION=v3.1.2
# 12.1 is the newest API NVIDIA's 530 drivers have; newer headers would raise the driver floor for
# no gain, as AV1 NVENC only needs RTX 40-series support, which 530 has.
NVCODEC_VERSION=n12.1.14.0

PREFIX="${1:-build/ffmpeg-static}"
mkdir -p "$PREFIX"
PREFIX="$(cd "$PREFIX" && pwd)"
SRC="$PREFIX.src"

OS="$(uname -s)"
if [[ "$OS" == Darwin ]]; then
  JOBS="$(sysctl -n hw.ncpu)"
  # The apps' LSMinimumSystemVersion (deploy/macos/bundle.sh); otherwise the libraries target the
  # build machine's macOS.
  export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-10.15}"
else
  JOBS="$(nproc)"
fi

if [[ "$OS" == Darwin ]]; then
  script_hash="$(shasum -a 256 "$0" | cut -d' ' -f1)"
else
  script_hash="$(sha256sum "$0" | cut -d' ' -f1)"
fi
BUILD_ID="$OS-$(uname -m) ffmpeg=$FFMPEG_VERSION dav1d=$DAV1D_VERSION opus=$OPUS_VERSION webp=$WEBP_VERSION svtav1=$SVTAV1_VERSION nvcodec=$NVCODEC_VERSION script=$script_hash"

if [[ -f "$PREFIX/.build-id" && "$(cat "$PREFIX/.build-id")" == "$BUILD_ID" ]]; then
  echo "FFmpeg is already built in $PREFIX"
  exit 0
fi

if [[ "$OS" == Linux ]] && ! pkg-config --exists libva libva-drm libdrm; then
  echo "error: libva and libdrm development files are missing (Debian/Ubuntu: libva-dev" \
    "libdrm-dev; Fedora: libva-devel libdrm-devel)" >&2
  exit 1
fi

echo "Building static FFmpeg into $PREFIX"
rm -rf "$PREFIX" "$SRC"
mkdir -p "$PREFIX" "$SRC"

export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"
# Release builds of every library, position-independent so they can go into a shared library too.
export CFLAGS="${CFLAGS:-} -O2 -fPIC"
export CXXFLAGS="${CXXFLAGS:-} -O2 -fPIC"

fetch() { # url tag dir
  git -c advice.detachedHead=false clone --depth 1 --branch "$2" "$1" "$SRC/$3"
}

cmake_install() { # dir [cmake args...]
  local dir="$SRC/$1"
  shift
  cmake -S "$dir" -B "$dir/build" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DCMAKE_INSTALL_LIBDIR=lib \
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
    -DBUILD_SHARED_LIBS=OFF \
    "$@"
  cmake --build "$dir/build" --parallel "$JOBS"
  cmake --install "$dir/build"
}

echo "== dav1d $DAV1D_VERSION"
fetch https://code.videolan.org/videolan/dav1d.git "$DAV1D_VERSION" dav1d
meson setup "$SRC/dav1d/build" "$SRC/dav1d" \
  --prefix="$PREFIX" --libdir=lib --buildtype=release --default-library=static \
  -Denable_tools=false -Denable_tests=false -Denable_examples=false
ninja -C "$SRC/dav1d/build" install

echo "== opus $OPUS_VERSION"
fetch https://github.com/xiph/opus.git "$OPUS_VERSION" opus
cmake_install opus -DOPUS_BUILD_PROGRAMS=OFF -DOPUS_BUILD_TESTING=OFF -DOPUS_INSTALL_PKG_CONFIG_MODULE=ON

echo "== libwebp $WEBP_VERSION"
fetch https://chromium.googlesource.com/webm/libwebp "$WEBP_VERSION" libwebp
cmake_install libwebp \
  -DWEBP_BUILD_ANIM_UTILS=OFF -DWEBP_BUILD_CWEBP=OFF -DWEBP_BUILD_DWEBP=OFF \
  -DWEBP_BUILD_GIF2WEBP=OFF -DWEBP_BUILD_IMG2WEBP=OFF -DWEBP_BUILD_VWEBP=OFF \
  -DWEBP_BUILD_WEBPINFO=OFF -DWEBP_BUILD_WEBPMUX=OFF -DWEBP_BUILD_EXTRAS=OFF \
  -DWEBP_BUILD_LIBWEBPMUX=ON

echo "== SVT-AV1 $SVTAV1_VERSION"
fetch https://gitlab.com/AOMediaCodec/SVT-AV1.git "$SVTAV1_VERSION" svtav1
cmake_install svtav1 -DBUILD_APPS=OFF -DBUILD_DEC=OFF -DBUILD_TESTING=OFF

platform_flags=()
if [[ "$OS" == Linux ]]; then
  echo "== nv-codec-headers $NVCODEC_VERSION"
  fetch https://github.com/FFmpeg/nv-codec-headers.git "$NVCODEC_VERSION" nv-codec-headers
  make -C "$SRC/nv-codec-headers" install PREFIX="$PREFIX"
  platform_flags+=(--enable-vaapi --enable-libdrm --enable-ffnvcodec --enable-nvenc)
  # libva and libdrm come from the host; let pkg-config find their .pc files too.
  PKG_CONFIG_PATH="$PKG_CONFIG_PATH:$(pkg-config --variable pc_path pkg-config)"
elif [[ "$OS" == Darwin ]]; then
  platform_flags+=(--enable-videotoolbox)
fi

echo "== FFmpeg $FFMPEG_VERSION"
fetch https://github.com/FFmpeg/FFmpeg.git "$FFMPEG_VERSION" ffmpeg
(
  cd "$SRC/ffmpeg"
  # --disable-autodetect, so that what's in the build is exactly what's listed here and nothing
  # GPL or host-specific gets picked up from whatever the build machine has installed.
  ./configure \
    --prefix="$PREFIX" \
    --pkg-config-flags=--static \
    --enable-static --disable-shared --enable-pic \
    --disable-programs --disable-doc --disable-debug \
    --disable-autodetect --enable-pthreads --enable-zlib \
    --enable-libdav1d --enable-libopus --enable-libwebp --enable-libsvtav1 \
    "${platform_flags[@]}" \
    --extra-cflags="-I$PREFIX/include" --extra-ldflags="-L$PREFIX/lib"
  make -j"$JOBS"
  make install
)

# Check that nothing GPL or nonfree slipped in, and that what the apps rely on did.
if grep -Eq '^CONFIG_(GPL|NONFREE|VERSION3)=yes' "$SRC/ffmpeg/ffbuild/config.mak"; then
  echo "error: this FFmpeg build is not LGPL-2.1" >&2
  exit 1
fi
for want in LIBSVTAV1_ENCODER LIBDAV1D_DECODER LIBOPUS_ENCODER LIBWEBP_ENCODER PNG_DECODER; do
  if ! grep -q "^#define CONFIG_$want 1" "$SRC/ffmpeg/config_components.h"; then
    echo "error: FFmpeg was built without CONFIG_$want" >&2
    exit 1
  fi
done

rm -rf "$SRC"
echo "$BUILD_ID" > "$PREFIX/.build-id"
echo "FFmpeg built into $PREFIX"
