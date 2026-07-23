# AGENTS.md

## What Lewdware is

A scriptable pseudo-malware desktop app: it spawns popup windows with images,
videos, prompts and choices, plays audio, opens links, and sets the wallpaper.
Behaviour is driven by **packs** (the media: images/videos/audio, `.lwpack`) and
**modes** (Lua scripts that decide what happens). See the root
[`README.md`](README.md) for the full overview, goals, and getting-started steps.

## Layout

A Cargo workspace of Rust crates plus two Tauri + Svelte desktop apps and an Astro
docs site.

| Path | What it is | README |
| --- | --- | --- |
| `lewdware/` | The engine — spawns/renders windows, plays media, runs the mode. Rust (`winit`, `wgpu`/`softbuffer`, ffmpeg). Never launched directly. | [link](lewdware/README.md) |
| `config/` | The user-facing app: configure the engine, pick pack/mode, launch/stop, updates. Tauri + Svelte. | [link](config/README.md) |
| `pack-editor/` | GUI for creating/editing packs; compresses media via the ffmpeg CLI. Tauri + Svelte. Distributed separately. | [link](pack-editor/README.md) |
| `lw/` | CLI for creating/editing/building modes. | [link](lw/README.md) |
| `supervisor/` | Background daemon owning session lifecycle, scheduling, tray, panic key, wallpaper. Config talks to it; it spawns the engine. | [link](supervisor/README.md) |
| `shared/` | Library crate shared by all of the above: pack reading, `behaviour.json`, the Lua API (`lua/api.lua`), config, IPC, scheduling, wallpaper. | [link](shared/README.md) |
| `converter/` | Converts Edgeware/Edgeware++ packs into `.lwpack` pieces. Used by the pack editor. | [link](converter/README.md) |
| `docs/` | The website (<https://lewdware.net>), Astro + Starlight. Hosts the Lua API reference and version manifests. | [link](docs/README.md) |
| `default-modes/` | The modes bundled with Lewdware by default. | — |
| `shared-ui/` | Shared Svelte components + design tokens for `config/` and `pack-editor/`. | see below |
| `deploy/` | Bundling/distribution scripts (driven by `.github/workflows/build.yml`). | — |

## How it fits together

`config` (or `lw`) asks the **supervisor** daemon to run a session; the supervisor
spawns the **engine**, which loads the selected **pack** and **mode** and does the
actual window-spawning. `config`, `pack-editor`, `lw`, and the supervisor all share
types and logic through the **shared** crate.

## Tooling & commands

- **Rust:** one Cargo workspace (`Cargo.toml` at the root). Build/run crates with
  `cargo run -p <crate>` (e.g. `cargo run -p lewdware`, `cargo run -p lw -- <cmd>`).
- **JS/Svelte:** use **pnpm**, not npm. Run the Tauri apps with `pnpm tauri dev`
  from `config/` or `pack-editor/`; the docs site with `pnpm dev` from `docs/`.
- First-time setup (ffmpeg sidecars, building the default modes) is in the root
  README's "Getting started".

## Conventions

- **Doing UI work in `config/` or `pack-editor/`?** Read
  [`shared-ui/DESIGN.md`](shared-ui/DESIGN.md) first.
