#!/usr/bin/env bash
set -euxo pipefail

PREFIX="$1"
TARGET="${CARGO_BUILD_TARGET:-$(uname -m)-$(uname -s | tr '[:upper:]' '[:lower:]')}"

if [ -f "$PREFIX/lib/pkgconfig/libavcodec.pc" ]; then
  echo "FFmpeg already built, skipping..."
  exit 0
fi

rm -rf ffmpeg
git clone --depth=1 https://github.com/FFmpeg/FFmpeg.git ffmpeg
cd ffmpeg

CONFIGURE_ARGS=(
  --prefix="$PREFIX"
  --disable-shared
  --enable-static
  --disable-debug
  --disable-doc
  --disable-programs
  --disable-network
  --disable-hwaccels
  --disable-autodetect
  --disable-everything

  # Minimal libraries required by ffmpeg-sys-next
  --enable-avcodec
  --enable-avformat
  --enable-avutil
  --enable-swscale
  --enable-swresample
  --enable-avfilter
  --enable-avdevice

  # Enable necessary decoders, parsers and demuxers
  --enable-decoder=h264
  --enable-decoder=vp9
  --enable-parser=h264
  --enable-parser=vp9
  --enable-demuxer=mov
  --enable-demuxer=matroska
  --enable-protocol=file
)

if [[ "$TARGET" == *"pc-windows-gnu"* ]]; then
  CONFIGURE_ARGS+=(
    --target-os=mingw32
    --arch=x86_64
    --cross-prefix=x86_64-w64-mingw32-
  )
fi

./configure "${CONFIGURE_ARGS[@]}"
make -j$(nproc)
make install
