#!/usr/bin/env bash
set -euxo pipefail

PREFIX="$1"
TARGET="${CARGO_BUILD_TARGET:-$(uname -m)-$(uname -s | tr '[:upper:]' '[:lower:]')}"

# Create a lock file to prevent race conditions
LOCK_FILE="$PREFIX/.ffmpeg-build.lock"
mkdir -p "$(dirname "$LOCK_FILE")"

# Function to cleanup on exit
cleanup() {
    if [ -d "$LOCK_FILE" ]; then
        rmdir "$LOCK_FILE" 2>/dev/null || true
    fi
    if [ -n "${BUILD_DIR:-}" ] && [ -d "$BUILD_DIR" ]; then
        rm -rf "$BUILD_DIR"
    fi
}
trap cleanup EXIT
trap cleanup EXIT

# Check if already built (with lock)
if [ -f "$PREFIX/lib/pkgconfig/libavcodec.pc" ]; then
    echo "FFmpeg already built, skipping..."
    exit 0
fi

# Try to acquire lock (simple file-based locking)
if ! mkdir "$LOCK_FILE" 2>/dev/null; then
    echo "Another FFmpeg build is in progress, waiting..."
    while [ -d "$LOCK_FILE" ]; do
        sleep 5
    done
    # Check again after waiting
    if [ -f "$PREFIX/lib/pkgconfig/libavcodec.pc" ]; then
        echo "FFmpeg was built by another process, skipping..."
        exit 0
    fi
fi

echo "Building FFmpeg for target: $TARGET"

# Create unique build directory
BUILD_DIR="ffmpeg-build-$$"
rm -rf "$BUILD_DIR"
git clone --depth=1 --branch "release/8.0" https://github.com/FFmpeg/FFmpeg.git "$BUILD_DIR"
cd "$BUILD_DIR"

# Base configure arguments for decoding-only
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

    # Core libraries required by ffmpeg-sys-next
    --enable-avcodec
    --enable-avformat
    --enable-avutil
    --enable-swscale
    --enable-swresample
    --enable-avfilter
    --enable-avdevice

    # Enable built-in decoders (no external libs needed)
    --enable-decoder=opus           # Built-in Opus decoder
    --enable-decoder=vp8            # Built-in VP8 decoder  
    --enable-decoder=vp9            # Built-in VP9 decoder
    --enable-decoder=h264           # H.264 decoder (common in WebM containers)
    
    # Enable parsers
    --enable-parser=opus
    --enable-parser=vp8
    --enable-parser=vp9
    --enable-parser=h264
    
    # Enable demuxers for container formats
    --enable-demuxer=matroska       # For .webm files (WebM uses Matroska container)
    --enable-demuxer=ogg            # For .ogg files with Opus
    --enable-demuxer=mov            # For .mp4 files (if needed)
    
    # Enable protocols
    --enable-protocol=file
)

# Platform-specific configuration
case "$(uname -s)" in
    "Linux")
        echo "Configuring for Linux..."
        # No special configuration needed for Linux
        ;;
    "Darwin")
        echo "Configuring for macOS..."
        # Handle macOS universal builds
        if [[ "$TARGET" == *"x86_64"* ]]; then
            CONFIGURE_ARGS+=(--arch=x86_64)
            if [[ "$(uname -m)" == "arm64" ]]; then
                # Cross-compiling to x86_64 on Apple Silicon
                CONFIGURE_ARGS+=(
                    --extra-cflags="-target x86_64-apple-macos10.12"
                    --extra-ldflags="-target x86_64-apple-macos10.12"
                )
            fi
        elif [[ "$TARGET" == *"aarch64"* ]] || [[ "$TARGET" == *"arm64"* ]]; then
            CONFIGURE_ARGS+=(--arch=aarch64)
            if [[ "$(uname -m)" == "x86_64" ]]; then
                # Cross-compiling to ARM64 on Intel
                CONFIGURE_ARGS+=(
                    --extra-cflags="-target arm64-apple-macos11"
                    --extra-ldflags="-target arm64-apple-macos11"
                )
            fi
        fi
        ;;
    "MINGW"*|"MSYS"*|"CYGWIN"*)
        echo "Configuring for Windows (MSYS2/MinGW)..."
        CONFIGURE_ARGS+=(--target-os=mingw32)
        ;;
    *)
        echo "Configuring for generic Unix-like system..."
        ;;
esac

echo "Configure arguments: ${CONFIGURE_ARGS[*]}"

# Configure FFmpeg
./configure "${CONFIGURE_ARGS[@]}"

# Determine number of parallel jobs
if command -v nproc >/dev/null 2>&1; then
    JOBS=$(nproc)
elif command -v sysctl >/dev/null 2>&1; then
    JOBS=$(sysctl -n hw.ncpu 2>/dev/null || echo "2")
elif [[ -n "${NUMBER_OF_PROCESSORS:-}" ]]; then
    JOBS="$NUMBER_OF_PROCESSORS"
else
    JOBS=2
fi

echo "Building with $JOBS parallel jobs..."

# Build and install
make -j"$JOBS"
make install

echo "FFmpeg successfully built and installed to $PREFIX"

# Verify installation
if [ -f "$PREFIX/lib/pkgconfig/libavcodec.pc" ]; then
    echo "✓ FFmpeg installation verified"
else
    echo "✗ FFmpeg installation failed - missing pkg-config file"
    exit 1
fi
