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
- `config/` is how users configure and launch the Lewdware engine. When a user
  launches "Lewdware" the application, this is what they launch. It is written
  and built using Svelte and Tauri.
- `pack-editor/` is a GUI tool for editing packs, written in Svelte and Tauri.
  It uses the ffmpeg CLI to encode and compress images, videos and audio.
- `lw/` is a CLI tool, mostly used for creating, editing and building modes.

`deploy/` contains scripts for bundling and distributing the four tools (done
in `.github/workflows/build.yml`). `lewdware`, `config` and `lw` are distributed
together, while `pack-editor` is distributed separately (since it's quite a
large application).

`docs/` contains the code for the [website](https://lewdware.net), written in
Astro and Starlight, and hosted on Cloudflare.

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
installed.

<!-- TODO: detail how to download ffmpeg sidecar binaries for pack-editor -->

The engine and config apps require the default modes to be built:

``` bash
cd default-modes
cargo run -p lw -- mode build
```

Once those are built, you should be able to run the four apps.

To run the lewdware engine:

``` bash
cargo run -p lewdware
```

To run the config app:

``` bash
cd config
pnpm tauri dev
```

To run the pack-editor:

``` bash
cd pack-editor
pnpm tauri dev
```

To run `lw`:

```bash
cargo run -p lw -- <subcommand>
```

## More information

- [`lewdware/README.md`](lewdware/README.md)
- [`config/README.md`](config/README.md)
- [`pack-editor/README.md`](pack-editor/README.md)
- [`lw/README.md`](lw/README.md)
