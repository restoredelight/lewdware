#!/usr/bin/env bash
set -euxo pipefail

PREFIX="$1"
TARGET="${CARGO_BUILD_TARGET:-$(uname -m)-$(uname -s | tr '[:upper:]' '[:lower:]')}"
WORK_DIR="$(pwd)"

if [ -f "$PREFIX/lib/pkgconfig/dav1d.pc" ]; then
  echo "dav1d already built, skipping..."
  exit 0
fi

# Create a temporary directory for dav1d build
TEMP_DIR=$(mktemp -d)
cd "$TEMP_DIR"

git clone --depth=1 https://code.videolan.org/videolan/dav1d.git
cd dav1d

# Detect number of cores for different platforms
if command -v nproc >/dev/null 2>&1; then
  CORES=$(nproc)
elif command -v sysctl >/dev/null 2>&1; then
  CORES=$(sysctl -n hw.ncpu)
else
  CORES=2
fi

# Configure cross-compilation if needed
MESON_ARGS=()
if [[ "$TARGET" == *"pc-windows-gnu"* ]]; then
  # Create a cross-compilation file for Windows
  cat > cross_file.txt << EOF
[binaries]
c = 'x86_64-w64-mingw32-gcc'
ar = 'x86_64-w64-mingw32-ar'
strip = 'x86_64-w64-mingw32-strip'
pkgconfig = 'pkg-config'

[host_machine]
system = 'windows'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
EOF
  MESON_ARGS+=(--cross-file cross_file.txt)
fi

if [ ${#MESON_ARGS[@]} -eq 0 ]; then
  meson setup build \
    --prefix="$PREFIX" \
    --default-library=static \
    --buildtype=release \
    -Denable_tools=false \
    -Denable_tests=false
else
  meson setup build \
    --prefix="$PREFIX" \
    --default-library=static \
    --buildtype=release \
    -Denable_tools=false \
    -Denable_tests=false \
    "${MESON_ARGS[@]}"
fi

ninja -C build -j"$CORES"
ninja -C build install

if [ -f "$PREFIX/lib/pkgconfig/dav1d.pc" ]; then
  export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH:-}"
elif [ -f "$PREFIX/lib/x86_64-linux-gnu/pkgconfig/dav1d.pc" ]; then
  export PKG_CONFIG_PATH="$PREFIX/lib/x86_64-linux-gnu/pkgconfig:${PKG_CONFIG_PATH:-}"
else
  echo "Could not find dav1d.pc after install!"
  cd "$WORK_DIR"
  rm -rf "$TEMP_DIR"
  exit 1
fi

# Clean up
cd "$WORK_DIR"
rm -rf "$TEMP_DIR"
