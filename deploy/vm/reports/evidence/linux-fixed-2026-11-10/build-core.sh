#!/bin/bash
set -euo pipefail
cd /source
export CARGO_HOME=/build/cargo CARGO_TARGET_DIR=/build/ubuntu-target CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
cargo build --locked -p lw -p lewdware-supervisor
for mode in sandbox experience; do
 (cd default-modes/$mode && /build/ubuntu-target/debug/lw mode build)
done
(cd dev-modes/wallpaper-test && /build/ubuntu-target/debug/lw mode build)
cargo build --locked -p lewdware --features build-ffmpeg
mkdir -p /bundle/bin /bundle/lib
for name in lw lewdware-engine lewdware-supervisor; do
 cp /build/ubuntu-target/debug/$name /bundle/bin/$name
 strip /bundle/bin/$name
 patchelf --set-rpath '$ORIGIN/../lib' /bundle/bin/$name
done
# Bundle non-core dependencies; leave the guest's libc, X11 and graphics stack in place.
for binary in /bundle/bin/*; do
 while read -r lib; do
  case "$lib" in
   */libc.so.*|*/libm.so.*|*/libdl.so.*|*/libpthread.so.*|*/librt.so.*|*/libgcc_s.so.*|*/libstdc++.so.*|*/ld-linux*|*/libX*.so.*|*/libxcb*.so.*|*/libwayland*.so.*|*/libEGL.so.*|*/libGL*.so.*|*/libvulkan.so.*) continue;;
  esac
  [ -f "$lib" ] && cp -L "$lib" /bundle/lib/
 done < <(ldd "$binary" | awk '/=> \/.* / {print $3}')
done
for library in /bundle/lib/*; do patchelf --set-rpath '$ORIGIN' "$library"; done
cp 'dev-modes/wallpaper-test/build/Wallpaper test.lwmode' /bundle/wallpaper-test.lwmode
cp vm-source-manifest.json /bundle/source-manifest.json
rustc --version > /bundle/toolchain.txt
(cd /bundle && sha256sum bin/* lib/* wallpaper-test.lwmode > SHA256SUMS)
