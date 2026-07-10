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

This file, [`COPYING.GPLv3`](./COPYING.GPLv3), and a copy of the root MIT
`LICENSE` are themselves bundled into the distributed pack-editor installer
(see `src-tauri/tauri.conf.json`'s `bundle.resources`), so they travel with
the app for anyone who only has the compiled binary and not this repository.
