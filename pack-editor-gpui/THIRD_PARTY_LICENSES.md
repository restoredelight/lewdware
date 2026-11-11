# Third-party licenses

The pack editor's own source code is licensed under the MIT License (see the
repository root [`LICENSE`](../LICENSE)).

## Linked FFmpeg (LGPL v2.1+)

The pack editor probes, previews and encodes media through FFmpeg's
libraries (`libavcodec`, `libavformat`, `libavfilter`, etc.), linked into
the editor itself through the `ffmpeg-next` crate (see
`../shared/src/encode/` and `../media-gpui/`). It runs no FFmpeg programs.

Release builds link FFmpeg statically, built from pristine upstream source
*without* `--enable-gpl`, `--enable-version3` or `--enable-nonfree`, so the
linked FFmpeg is under the **GNU Lesser General Public License, version 2.1
or later**:

- Linux and macOS: built by
  [`deploy/build_ffmpeg.sh`](../deploy/build_ffmpeg.sh), which pins every
  version and lists the exact configure flags.
- Windows: `vcpkg`'s `ffmpeg` port, `x64-windows-static-md` triplet, with the
  features listed in `.github/workflows/build.yml` (`VCPKG_FFMPEG`) and no
  `gpl` feature.

Because FFmpeg is statically linked, LGPLv2.1 §6 applies: you may obtain
FFmpeg's source and relink the editor against a modified FFmpeg. FFmpeg's
source is at <https://github.com/FFmpeg/FFmpeg>; the editor's own source and
the build configuration above are in this repository.

- License text: [`COPYING.LGPLv2.1`](./COPYING.LGPLv2.1) in this directory, or
  <https://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt>.
- Copyright: FFmpeg is Copyright (C) the FFmpeg developers.

The codec libraries linked in with FFmpeg are permissively licensed:

| Library | Used for | License |
| --- | --- | --- |
| [SVT-AV1](https://gitlab.com/AOMediaCodec/SVT-AV1) | AV1 video encoding | BSD-3-Clause-Clear, plus the Alliance for Open Media patent license |
| [dav1d](https://code.videolan.org/videolan/dav1d) | AV1 decoding, AVIF decoding | BSD-2-Clause |
| [Opus](https://opus-codec.org) | audio encoding and decoding | BSD-3-Clause |
| [libwebp](https://chromium.googlesource.com/webm/libwebp) | thumbnails | BSD-3-Clause |
| [nv-codec-headers](https://github.com/FFmpeg/nv-codec-headers) | NVENC (Linux; headers only) | MIT |

On Windows, vcpkg also links AMD's AMF headers and Intel's libvpl, both
MIT-licensed, for the hardware encoders.

## Bundled avifenc binary (BSD-2-Clause)

Images are encoded to AVIF by a separate program, `avifenc`, which the
installers ship as `lewdware-avifenc` in a `binaries/` directory beside the
editor (see `src/main.rs`'s `sidecar_dirs`,
`deploy/linux/build_avifenc_sidecar.sh`,
`deploy/macos/build_avifenc_sidecar.sh` and
`deploy/windows/download_avifenc_sidecar.ps1`).

It's built from [libavif](https://github.com/AOMediaCodec/libavif)
(BSD-2-Clause) with only the `libaom` AV1 codec enabled, encode-only, plus
the small helper libraries (`libpng`, `zlib`, `libjpeg`) libavif's own build
pulls in for its command-line tools. All of these are permissively licensed
(BSD/zlib/IJG-style); `libaom` itself is BSD-2-Clause with an accompanying
royalty-free patent grant from the Alliance for Open Media.

- Linux and macOS: built from source at a pinned libavif release tag (see
  the two `build_avifenc_sidecar.sh` scripts for the exact CMake
  configuration and version), since libavif does not publish a prebuilt
  static binary for either platform.
- Windows: libavif's own official prebuilt static release artifact for the
  same pinned tag (which may additionally bundle `dav1d`/`rav1e` -- still all
  permissively licensed, just unused by the editor's `-c aom` invocation).
- Corresponding source: <https://github.com/AOMediaCodec/libavif>,
  <https://aomedia.googlesource.com/aom/> (or the CMake-vendored copy libavif
  itself fetches for `AVIF_CODEC_AOM=LOCAL` builds).
- Copyright: libavif is Copyright 2019 Joe Drago; libaom is Copyright the
  Alliance for Open Media.

This file, [`COPYING.LGPLv2.1`](./COPYING.LGPLv2.1), and a copy of the root
MIT `LICENSE` are installed with the editor by every installer, so they
travel with the app for anyone who only has the compiled binary and not this
repository.
