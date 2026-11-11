# Third-party licenses

lewdware's own source code (the engine, `config`, and `lw`) is licensed under
the MIT License (see the repository root [`LICENSE`](../LICENSE)).

## Bundled FFmpeg (LGPL v2.1+)

The engine (`lewdware/`) links against FFmpeg's libraries (`libavcodec`,
`libavformat`, etc.) through the `ffmpeg-next` crate to decode video/audio
from packs. Linking against FFmpeg makes the resulting binary a combined work
governed by whatever license that copy of FFmpeg carries. The pack editor
links the same build (see `pack-editor-gpui/THIRD_PARTY_LICENSES.md`).

**Official release builds** (produced by `.github/workflows/build.yml`) link
FFmpeg statically with the `static-ffmpeg` cargo feature, against a build that
is configured *without* `--enable-gpl`/`--enable-version3`/`--enable-nonfree`
and without `libx264`/`libx265`. This keeps the linked FFmpeg under its default
**LGPL version 2.1 or later** license, so the released binaries can remain
genuinely MIT (with the lighter LGPL obligations below), rather than becoming
a GPL combined work.

- Linux and macOS: [`deploy/build_ffmpeg.sh`](../deploy/build_ffmpeg.sh)
  builds pristine upstream FFmpeg and its codec libraries from source, at the
  versions it pins, with the configure flags it lists. It refuses to finish if
  the configured FFmpeg isn't LGPL-2.1.
- Windows: `vcpkg`'s `ffmpeg` port in the `x64-windows-static-md` triplet,
  with the features listed in the workflow's `VCPKG_FFMPEG` and no `gpl`
  feature, which is also LGPL-only.
- Deliberately avoided: Ubuntu's `apt` `libavcodec-dev` and Homebrew's
  `ffmpeg` formula, both of which are built `--enable-gpl` (Homebrew's
  formula is explicitly declared `GPL-3.0-or-later`) and would make a linked
  binary a GPL combined work if distributed.

The codec libraries linked in with it are under permissive licenses, which
don't affect FFmpeg's:

| Library | Used for | License |
| --- | --- | --- |
| [dav1d](https://code.videolan.org/videolan/dav1d) | AV1 decoding (and the `image` crate's AVIF decoding) | BSD-2-Clause |
| [Opus](https://opus-codec.org) | audio | BSD-3-Clause |
| [libwebp](https://chromium.googlesource.com/webm/libwebp) | WebP thumbnails | BSD-3-Clause |
| [SVT-AV1](https://gitlab.com/AOMediaCodec/SVT-AV1) | AV1 encoding (pack editor) | BSD-3-Clause-Clear, plus the AOMedia patent license |
| [nv-codec-headers](https://github.com/FFmpeg/nv-codec-headers) | NVENC (Linux; headers only) | MIT |

On Linux, libva, libdrm and zlib stay dynamic host libraries.

Because FFmpeg is statically linked, LGPLv2.1 §6 applies: recipients must be
able to obtain the means to relink the engine against a modified version of
FFmpeg. In practice this just means the exact FFmpeg source and build
configuration must stay available - it's pristine, unmodified upstream
FFmpeg source (<https://ffmpeg.org>, <https://github.com/FFmpeg/FFmpeg>),
built by the script and vcpkg configuration named above. FFmpeg's LGPL license
text is available at <https://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt>.

### Local development builds

Without `--features static-ffmpeg`, `cargo build` links dynamically against
whatever FFmpeg is installed on your system (per the root README's "Getting
started" section) for faster iteration. That system FFmpeg may well be a
GPL build (e.g. from `apt`/Homebrew) - that's fine for local development,
but if *you* redistribute a binary built this way, you take on the GPL's
obligations. Official releases always use the LGPL-only build described above.

## Bundled fonts

The engine embeds the typefaces below (the repo-level [`assets/fonts/`](../assets/fonts))
so that text popups and window themes render identically everywhere, without
depending on what is installed on the user's machine. Each is used unmodified.

They sit outside this crate because they have a second consumer: `config/`
embeds the UI faces to draw a theme's widgets in its picker, so the same
obligations below apply to that binary too. Neither reads the files at run
time — each embeds what it needs at build time.

Most are under the SIL Open Font License 1.1. The OFL permits bundling and redistribution — including inside a commercial or
proprietary application — and, unlike a code license, does not extend to the
program that embeds the font. It does require that the license text travel with
the font, which is why a copy of each sits in
[`assets/fonts/licenses/`](../assets/fonts/licenses). Its one real constraint is on
*modifying* a font: a derivative must stay under the OFL and must not use a
Reserved Font Name. We do neither.

Two of them are not OFL and carry their own terms, both permitting redistribution
and embedding: the **Ubuntu Font Licence 1.0** for Ubuntu, and the **MIT License**
for Hack (whose glyphs derive from the public-domain DejaVu project). These two
were previously used *via egui*, which bundles them itself; they are embedded
directly now so that the engine and `config/` draw the same text with the same
file rather than each reaching for its own default.

| Font | Used for | Copyright | License |
| --- | --- | --- | --- |
| Ubuntu | The neutral UI face — the `plain` theme, and the default text font | Canonical Ltd. | [UFL 1.0](../assets/fonts/licenses/Ubuntu-UFL.txt) |
| Hack | The `mono` text font | Source Foundry Authors; DejaVu (public domain) | [MIT](../assets/fonts/licenses/Hack-LICENSE.md) |
| Anton | The `display` text font | The Anton Project Authors | OFL 1.1 |
| W95FA | The `pixel` text font, and the `redmond` theme | Alina Sava | OFL 1.1 |
| Selawik | The `fluent` theme — Microsoft's own metrically-compatible substitute for Segoe UI, which is not redistributable | Microsoft Corporation (Reserved Font Name "Selawik") | [OFL 1.1](../assets/fonts/licenses/Selawik-OFL.txt) |
| Inter | The `aqua` theme — the closest freely-licensed stand-in for San Francisco | The Inter Project Authors | [OFL 1.1](../assets/fonts/licenses/Inter-OFL.txt) |
| Cantarell | The `adwaita` theme — GNOME's own UI font | The Cantarell Project Authors | [OFL 1.1](../assets/fonts/licenses/Cantarell-OFL.txt) |
| Noto Sans | The `breeze` theme — KDE Plasma's default UI face | The Noto Project Authors | [OFL 1.1](../assets/fonts/licenses/NotoSans-OFL.txt) |
| Liberation Sans | The `cde` theme — a metrically compatible substitute for its Helvetica-family UI typography | Red Hat | [SIL OFL 1.1](../assets/fonts/licenses/LiberationSans-LICENSE.txt) |
| Source Sans 3 | The `platinum` theme — a compact UI-oriented stand-in for Charcoal | Adobe | [OFL 1.1](../assets/fonts/licenses/SourceSans3-OFL.md) |

Anton and W95FA predate this file. Both are OFL 1.1, but unlike the others their
license texts are not yet checked in beside them — worth doing before release,
since the OFL asks that the license travel with the font.

None of the platform UI fonts these imitate — Segoe UI, San Francisco,
Charcoal — are redistributable, which is why each theme uses a free substitute
rather than the real thing.
