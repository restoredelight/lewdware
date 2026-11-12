# Lewdware

Lewdware is a scriptable pseudo-malware desktop app. It is able to spawn popup
windows containing images, videos, prompts and choices; play audio; open links;
and set the user's wallpaper.

Developers can create experiences for users using packs and modes. Packs change
the content (images, videos and audio) that the main app shows. Modes change
the behaviour of lewdware using Lua scripts. Modes can be embedded inside
packs, to provide a full experience, or distributed separately. Similarly,
packs don't necessarily require modes either, and lewdware provides a default
set of modes.

## Structure

Lewdware is split into several crates:

- `lewdware/` is the main "engine" - it reads the users config, loads the
  current pack and mode, and does all the window spawning, audio playing, etc.
- `config-gpui/` is how users configure and launch the Lewdware engine. When a
  user launches "Lewdware" the application, this is what they launch. It is
  written in Rust with [gpui](https://gpui.rs).
- `pack-editor-gpui/` is a GUI tool for editing packs, also written with gpui.
  It uses the ffmpeg CLI to encode and compress images, videos and audio.
- `lw/` is a CLI tool, mostly used for creating, editing and building modes.
- `supervisor/` is a background daemon that owns session lifecycle, scheduling,
  the tray icon and the stop shortcut.

`deploy/` contains scripts for bundling and distributing the tools (done in
`.github/workflows/build.yml`). Everything is distributed together, one installer
per platform. In releases the engine, supervisor, config app and pack editor are
built into one shared library (`core/`), which a small executable per app loads
(`launcher/`), so their code is on disk once; the pack editor is still an app of
its own. `lw` stays a separate program.

`docs/` contains the code for the [website](https://lewdware.net), written in
Astro and Starlight, and hosted on Cloudflare. The Lua mode API can be found
at `docs/src/data/config.schema.json`.

`default-modes/` contains the code for the modes included by default in
Lewdware.

## Goals

Lewdware aims, as much as possible, to be:

- Fast - the lewdware engine is written in Rust, using winit and
  wgpu/softbuffer rather than a GUI framework, making it very fast, not too
  memory intensive, and able to handle spawning hundreds of windows at once.
  Video/audio decoding is done using ffmpeg - we use hardware decoding when
  available, and use zero-copy APIs when we can (see
  `lewdware/src/zero_copy/`).
- Small - we make an effort to reduce file size, especially of pack files.
  The pack editor compresses all media files, significantly reducing file size.
- Easy to use - we compile and distribute Lewdware as a platform-native
  installer for Windows, macOS and Linux. We notify users if an update is
  available.

## Getting started

You will need to install Rust and Cargo, and have dav1d and ffmpeg libraries
installed. (This is just for local development - official release builds of
the engine vendor their own FFmpeg instead; see [License](#license) below.)

On Linux, the gpui apps also need the xkbcommon-x11 development headers
(`libxkbcommon-x11-devel` on Fedora, `libxkbcommon-x11-dev` on Debian/Ubuntu).

The pack editor encodes video and audio through the linked FFmpeg libraries,
which for local development need the SVT-AV1, Opus and libwebp encoders (most
distro FFmpeg builds have all three). Images still go through `avifenc`
(`libavif-bin` on Debian/Ubuntu, `libavif-tools` on Fedora), found on `PATH` or
as the `lewdware-avifenc` sidecar in `pack-editor-gpui/binaries/`.

The engine, the config app and `shared`'s test suite all embed the built default
modes, so they have to be built first -- once per mode project:

```bash
for mode in sandbox experience; do
  (cd "default-modes/$mode" && cargo run -p lw -- mode build)
done
```

Once those are built, you should be able to run the four apps.

To run the lewdware engine:

```bash
cargo run -p lewdware
```

To run the config app:

```bash
cargo run -p lewdware-config
```

To run the pack editor:

```bash
cargo run -p lewdware-pack-editor
```

To run `lw`:

```bash
cargo run -p lw -- <subcommand>
```

Development builds use `line-tables-only` debug information to reduce build
storage while keeping file names and line numbers in backtraces. Incremental
compilation stays enabled for faster rebuilds. For debugging that needs local
variables and types, use `CARGO_PROFILE_DEV_DEBUG=2 cargo build -p <crate>`
(or set `CARGO_PROFILE_DEV_DEBUG=2` in your shell before building).

## More information

- [`lewdware/README.md`](lewdware/README.md)
- [`config-gpui/README.md`](config-gpui/README.md)
- [`pack-editor-gpui/README.md`](pack-editor-gpui/README.md)
- [`lw/README.md`](lw/README.md)
- [`supervisor/README.md`](supervisor/README.md)
- [`shared/README.md`](shared/README.md)
- [`converter/README.md`](converter/README.md)
- [`docs/README.md`](docs/README.md)

## License

All of Lewdware's own source code is licensed under the MIT License (see
[`LICENSE`](LICENSE)). That covers everything you write or change in this
repository. Additionally, all the binaries distributed by Lewdware are
MIT-licensed, _except_ [`lewdware-pack-editor`](./pack-editor-gpui/), which
is GPL-licensed since it bundles a GPL-licensed ffmpeg binary (see
[`pack-editor-gpui/THIRD_PARTY_LICENSES.md`](pack-editor-gpui/THIRD_PARTY_LICENSES.md)).
