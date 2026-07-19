# Third-party licenses

pack-editor's own source code is licensed under the MIT License (see the
repository root [`LICENSE`](../LICENSE)).

## Bundled FFmpeg / ffprobe binaries (GPLv3)

The pack-editor binary invokes `ffmpeg` and `ffprobe` as separate subprocesses
to encode and inspect media (see `src-tauri/src/encode.rs` and
`src-tauri/src/thumbnail.rs`), and the distributed pack-editor installer
bundles prebuilt copies of these executables alongside its own binary
(`src-tauri/binaries/lewdware-ffmpeg`, `lewdware-ffprobe`; see
`src-tauri/tauri.conf.json`'s `bundle.resources` and
`deploy/*/download_ffmpeg_sidecars.sh`).

These binaries are built with GPL-only components (notably `libx264` for
H.264 encoding), which makes them licensed under the **GNU General Public
License, version 3 or later**, not MIT:

- Linux and Windows: [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds)
  "gpl" release variant.
- macOS: [ffmpeg.martin-riedl.de](https://ffmpeg.martin-riedl.de) snapshot
  build (also compiled with `libx264`/`libx265`).

Because pack-editor invokes these as separate processes through their
ordinary command-line interface, rather than linking against FFmpeg's
libraries, pack-editor's own source is not itself a derivative work of
FFmpeg and can remain MIT-licensed. However, **the distributed pack-editor
installer as a whole includes a verbatim copy of GPLv3-licensed software**,
and redistributing it carries the usual GPL obligations for those files:
a copy of the GPLv3 license text, FFmpeg's copyright notices, and access to
matching source.

- License text: see [`COPYING.GPLv3`](./COPYING.GPLv3) in this directory, or
  <https://www.gnu.org/licenses/gpl-3.0.txt>.
- Corresponding source: FFmpeg's source is available at
  <https://github.com/FFmpeg/FFmpeg>; the exact build configuration used for
  the Linux/Windows binaries is published by the BtbN/FFmpeg-Builds project
  linked above, and for macOS by ffmpeg.martin-riedl.de.
- Copyright: FFmpeg is Copyright (C) the FFmpeg developers.

If you redistribute pack-editor (or a modified version of it), you must
continue to satisfy these GPLv3 obligations for the bundled `ffmpeg`/`ffprobe`
binaries, in addition to the MIT terms covering pack-editor's own source.

## Bundled avifenc binary (BSD-2-Clause, permissive)

The pack-editor binary also invokes `avifenc` as a subprocess to encode
images to AVIF (see `shared/src/encode.rs`'s `encode_image`), and bundles a
prebuilt/self-built copy alongside its own binary
(`src-tauri/binaries/lewdware-avifenc`; see
`src-tauri/tauri.conf.json`'s `bundle.resources`,
`deploy/linux/build_avifenc_sidecar.sh`,
`deploy/macos/build_avifenc_sidecar.sh`, and
`deploy/windows/download_avifenc_sidecar.ps1`).

Unlike FFmpeg, this binary carries no GPL obligations. It's built from
[libavif](https://github.com/AOMediaCodec/libavif) (BSD-2-Clause) with only
the `libaom` AV1 codec enabled, encode-only -- no `dav1d`, `rav1e`, or
SVT-AV1 -- plus the small helper libraries (`libpng`, `zlib`, `libjpeg`)
libavif's own build pulls in for its command-line tools. All of these are
permissively licensed (BSD/zlib/IJG-style); `libaom` itself is BSD-2-Clause
with an accompanying royalty-free patent grant from the Alliance for Open
Media. None of this requires the GPLv3 source-availability/license-text
obligations that the bundled FFmpeg does.

- Linux and macOS: built from source at a pinned libavif release tag (see
  `deploy/linux/build_avifenc_sidecar.sh` /
  `deploy/macos/build_avifenc_sidecar.sh` for the exact CMake configuration
  and version), since libavif does not publish a prebuilt static binary for
  either platform.
- Windows: libavif's own official prebuilt static release artifact for the
  same pinned tag (which may additionally bundle `dav1d`/`rav1e` -- still all
  permissively licensed, just unused by pack-editor's `-c aom` invocation).
- Corresponding source: <https://github.com/AOMediaCodec/libavif>,
  <https://aomedia.googlesource.com/aom/> (or the CMake-vendored copy libavif
  itself fetches for `AVIF_CODEC_AOM=LOCAL` builds).
- Copyright: libavif is Copyright 2019 Joe Drago; libaom is Copyright the
  Alliance for Open Media.

This file, [`COPYING.GPLv3`](./COPYING.GPLv3), and a copy of the root MIT
`LICENSE` are themselves bundled into the distributed pack-editor installer
(see `src-tauri/tauri.conf.json`'s `bundle.resources`), so they travel with
the app for anyone who only has the compiled binary and not this repository.
