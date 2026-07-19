#!/bin/bash
# Builds a static `avifenc` binary (from libavif + a locally-built libaom) required by the
# pack-editor's image encode pipeline. No prebuilt static macOS binary is published upstream, so
# this builds one from source, encode-only and AOM-only (no dav1d/rav1e/SVT-AV1 decoder/encoder
# backends we don't use), which keeps both the build and the licensing surface minimal -- see
# pack-editor/THIRD_PARTY_LICENSES.md. Builds natively for whatever architecture this runs on.
set -e

LIBAVIF_TAG="v1.4.2"

BINARIES_DIR="pack-editor/src-tauri/binaries"
mkdir -p "$BINARIES_DIR"

AVIFENC_SIDECAR="$BINARIES_DIR/lewdware-avifenc"

if [ -f "$AVIFENC_SIDECAR" ]; then
  echo "avifenc sidecar already present."
  exit 0
fi

echo "Building avifenc from libavif $LIBAVIF_TAG..."
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

git clone --branch "$LIBAVIF_TAG" --depth 1 https://github.com/AOMediaCodec/libavif.git "$TEMP_DIR/libavif"

cmake -S "$TEMP_DIR/libavif" -B "$TEMP_DIR/libavif/build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DAVIF_CODEC_AOM=LOCAL \
  -DAVIF_CODEC_AOM_ENCODE=ON \
  -DAVIF_CODEC_AOM_DECODE=OFF \
  -DAVIF_LIBYUV=LOCAL \
  -DAVIF_LIBSHARPYUV=OFF \
  -DAVIF_JPEG=LOCAL \
  -DAVIF_ZLIBPNG=LOCAL \
  -DAVIF_BUILD_APPS=ON \
  -DAVIF_BUILD_TESTS=OFF \
  -DAVIF_BUILD_EXAMPLES=OFF

cmake --build "$TEMP_DIR/libavif/build" --config Release --parallel "$(sysctl -n hw.ncpu)"

cp "$TEMP_DIR/libavif/build/avifenc" "$AVIFENC_SIDECAR"
chmod +x "$AVIFENC_SIDECAR"

echo "avifenc sidecar staged successfully."
