# AGENTS.md

## What Lewdware is

A scriptable pseudo-malware desktop app: it spawns popup windows with images,
videos, prompts and choices, plays audio, opens links, and sets the wallpaper.
Behaviour is driven by **packs** (the media: images/videos/audio, `.lwpack`) and
**modes** (Lua scripts that decide what happens). See the root
[`README.md`](README.md) for the full overview, goals, and getting-started steps.

## Layout

A Cargo workspace of Rust crates, including two gpui desktop apps, plus an Astro
docs site.

| Path | What it is | README |
| --- | --- | --- |
| `lewdware/` | The engine — spawns/renders windows, plays media, runs the mode. Rust (`winit`, `wgpu`/`softbuffer`, ffmpeg). Never launched directly. | [link](lewdware/README.md) |
| `config-gpui/` | The user-facing app (package `lewdware-config`, binary `lewdware`): configure the engine, pick pack/mode, launch/stop, updates. gpui. | [link](config-gpui/README.md) |
| `pack-editor-gpui/` | GUI for creating/editing packs (package/binary `lewdware-pack-editor`); encodes media with the linked ffmpeg. gpui. Its own app, shipped in the same installers. | [link](pack-editor-gpui/README.md) |
| `pack-editor-core/` | The pack editor's model: editing sessions, undo, import pipelines. No UI. | [link](pack-editor-core/README.md) |
| `shared-gpui/` | Design tokens, fonts and components shared by the two gpui apps. | — |
| `media-gpui/` | The pack editor's media previews (ffmpeg decode, gpui images). | [link](media-gpui/README.md) |
| `lw/` | CLI for creating/editing/building modes. | [link](lw/README.md) |
| `supervisor/` | Background daemon owning session lifecycle, scheduling, tray, stop shortcut, wallpaper. Config talks to it; it spawns the engine. | [link](supervisor/README.md) |
| `core/` | Release builds only: the engine, supervisor, config app and pack editor in one shared library (`lewdware_core`), so the code and the static ffmpeg are on disk once. | — |
| `launcher/` | Release builds only: the small per-app executables that load `core/` and run their app in it. Installers rename `launcher-<app>` to the app's name. | — |
| `shared/` | Library crate shared by all of the above: pack reading, `behaviour.json`, the Lua API (`lua/api.lua`), config, IPC, scheduling, wallpaper. | [link](shared/README.md) |
| `converter/` | Converts Edgeware/Edgeware++ packs into `.lwpack` pieces. Used by the pack editor. | [link](converter/README.md) |
| `docs/` | The website (<https://lewdware.net>), Astro + Starlight. Hosts the Lua API reference and version manifests. | [link](docs/README.md) |
| `default-modes/` | The modes bundled with Lewdware by default. | — |
| `dev-modes/` | Modes used to exercise the engine during development; not shipped. | [link](dev-modes/README.md) |
| `shared-ui/` | Just `DESIGN.md`, the UI style guide for both gpui apps. | see below |
| `deploy/` | Bundling/distribution scripts (driven by `.github/workflows/build.yml`). | — |

## How it fits together

`config` (or `lw`) asks the **supervisor** daemon to run a session; the supervisor
spawns the **engine**, which loads the selected **pack** and **mode** and does the
actual window-spawning. `config`, `pack-editor`, `lw`, and the supervisor all share
types and logic through the **shared** crate.

## Tooling & commands

- **Rust:** one Cargo workspace (`Cargo.toml` at the root). Build/run crates with
  `cargo run -p <crate>` (e.g. `cargo run -p lewdware`, `cargo run -p lw -- <cmd>`).
- Run the apps with `cargo run -p lewdware-config` and `cargo run -p lewdware-pack-editor`.
  On Linux they need the xkbcommon-x11 dev headers.
- **JS:** use **pnpm**, not npm. Run the docs site with `pnpm dev` from `docs/`.
- First-time setup (FFmpeg libraries and avifenc, building the default modes) is in the root
  README's "Getting started".

## Conventions

- **Doing UI work in `config-gpui/` or `pack-editor-gpui/`?** Read
  [`shared-ui/DESIGN.md`](shared-ui/DESIGN.md) first.
